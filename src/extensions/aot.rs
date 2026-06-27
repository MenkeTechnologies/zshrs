//! Ahead-of-time build: bake one or more shell scripts into a copy of the
//! running `zshrs` binary as a compressed trailer, producing a self-contained
//! executable. At startup, zshrs detects the trailer and runs every embedded
//! script IN INPUT ORDER as a single concatenated zsh program.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C zsh
//! has the `zcompile` builtin (Src/parse.c → `bin_zcompile()`) which
//! writes a parsed-AST `.zwc` file alongside a script for faster
//! re-parse. That's a separate file, not a self-contained binary.
//! AOT-ing scripts into the shell executable itself is a zshrs
//! addition — it lets you ship a single binary that bundles its own
//! script (think `zsh -c '...'` but compiled in).
//!
//! Layout (little-endian, appended to the end of a copy of the `zshrs` binary):
//!
//! ```text
//!   [elf/mach-o bytes of zshrs ...]   (unchanged, still runs as `zshrs`)
//!   [zstd-compressed payload ...]
//!   [u64 compressed_len]
//!   [u64 uncompressed_len]
//!   [u32 version]
//!   [u32 reserved (0)]
//!   [8 bytes magic  b"ZSHRSAOT"]
//! ```
//!
//! Payload v1 (single script, BACKWARD-COMPAT decoder only — new builds
//! always emit v2 even for one input):
//!
//! ```text
//!   [u32 script_name_len]
//!   [script_name utf8]
//!   [source bytes utf8]
//! ```
//!
//! Payload v2 (ordered file list — current `zbuild` output):
//!
//! ```text
//!   [u32 file_count]
//!   for each file (file_count times, in input order):
//!     [u32 name_len][name utf8]
//!     [u32 source_len][source utf8]
//! ```
//!
//! Files run sequentially in iteration order. zsh has no "project" concept;
//! ordering matches `--in` argv order. Globals/functions defined by file N
//! are visible to file N+1 (single ShellExecutor across all files).
//!
//! ELF (Linux) and Mach-O (macOS) loaders ignore bytes past the program-
//! header-listed segments, so appending leaves the original `zshrs` fully
//! runnable. macOS resulting binary is unsigned; signed-build distributors
//! must re-codesign.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// 8-byte trailer magic.
pub const AOT_MAGIC: &[u8; 8] = b"ZSHRSAOT";
/// Trailer format version 1: single script (legacy decode-only).
pub const AOT_VERSION_V1: u32 = 1;
/// Trailer format version 2: ordered file list (current build output).
pub const AOT_VERSION_V2: u32 = 2;
/// Fixed trailer length: `8 (cl) + 8 (ul) + 4 (ver) + 4 (rsv) + 8 (magic)`.
pub const TRAILER_LEN: u64 = 32;
/// `EmbeddedFile` — see fields for layout.
#[derive(Debug, Clone)]
pub struct EmbeddedFile {
    /// `__FILE__` / error-reporting name (e.g. `hello.zsh`).
    pub name: String,
    /// UTF-8 zsh source.
    pub source: String,
}

/// One or more embedded files, in build-order. v1 binaries decode to a
/// 1-element vec; v2 to N elements preserving input order.
#[derive(Debug, Clone)]
pub struct EmbeddedFiles(pub Vec<EmbeddedFile>);

fn encode_payload_v2(files: &[EmbeddedFile]) -> Vec<u8> {
    let mut out = Vec::with_capacity(
        64 + files
            .iter()
            .map(|f| f.name.len() + f.source.len() + 8)
            .sum::<usize>(),
    );
    let count = u32::try_from(files.len()).expect("file count fits in u32");
    out.extend_from_slice(&count.to_le_bytes());
    for f in files {
        let name_len = u32::try_from(f.name.len()).expect("name length fits in u32");
        let src_len = u32::try_from(f.source.len()).expect("source length fits in u32");
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(f.name.as_bytes());
        out.extend_from_slice(&src_len.to_le_bytes());
        out.extend_from_slice(f.source.as_bytes());
    }
    out
}

fn decode_payload_v2(bytes: &[u8]) -> Option<EmbeddedFiles> {
    let mut pos = 0usize;
    if bytes.len() < 4 {
        return None;
    }
    let count = u32::from_le_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
    pos += 4;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        if pos + 4 > bytes.len() {
            return None;
        }
        let name_len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        pos += 4;
        if pos + name_len > bytes.len() {
            return None;
        }
        let name = std::str::from_utf8(&bytes[pos..pos + name_len])
            .ok()?
            .to_string();
        pos += name_len;
        if pos + 4 > bytes.len() {
            return None;
        }
        let src_len = u32::from_le_bytes(bytes[pos..pos + 4].try_into().ok()?) as usize;
        pos += 4;
        if pos + src_len > bytes.len() {
            return None;
        }
        let source = std::str::from_utf8(&bytes[pos..pos + src_len])
            .ok()?
            .to_string();
        pos += src_len;
        out.push(EmbeddedFile { name, source });
    }
    Some(EmbeddedFiles(out))
}

/// v1 decoder kept for backward compat: one-script payload promoted into a
/// single-element EmbeddedFiles. Old binaries built with the previous zbuild
/// still load.
fn decode_payload_v1(bytes: &[u8]) -> Option<EmbeddedFiles> {
    if bytes.len() < 4 {
        return None;
    }
    let name_len = u32::from_le_bytes(bytes[0..4].try_into().ok()?) as usize;
    if 4 + name_len > bytes.len() {
        return None;
    }
    let name = std::str::from_utf8(&bytes[4..4 + name_len])
        .ok()?
        .to_string();
    let source = std::str::from_utf8(&bytes[4 + name_len..])
        .ok()?
        .to_string();
    Some(EmbeddedFiles(vec![EmbeddedFile { name, source }]))
}

fn build_trailer(compressed_len: u64, uncompressed_len: u64, version: u32) -> [u8; 32] {
    let mut trailer = [0u8; 32];
    trailer[0..8].copy_from_slice(&compressed_len.to_le_bytes());
    trailer[8..16].copy_from_slice(&uncompressed_len.to_le_bytes());
    trailer[16..20].copy_from_slice(&version.to_le_bytes());
    // 20..24 reserved (zeros).
    trailer[24..32].copy_from_slice(AOT_MAGIC);
    trailer
}

/// Append a compressed v2 ordered-file payload to an existing file.
/// zshrs-original — no C counterpart. C zsh's closest analog is the
/// `zcompile` builtin in Src/parse.c which writes parsed-AST data
/// to a separate `.zwc` file rather than appending to the binary.
pub fn append_embedded_files(out_path: &Path, files: &[EmbeddedFile]) -> io::Result<()> {
    let payload = encode_payload_v2(files);
    let compressed = zstd::stream::encode_all(&payload[..], 3)?;
    let mut f = OpenOptions::new().append(true).open(out_path)?;
    f.write_all(&compressed)?;
    let trailer = build_trailer(
        compressed.len() as u64,
        payload.len() as u64,
        AOT_VERSION_V2,
    );
    f.write_all(&trailer)?;
    f.sync_all()?;
    Ok(())
}

/// Fast probe: read the last 32 bytes of `exe` and return embedded files
/// in build-order if present. Decodes both v1 (legacy single-script) and
/// v2 (current ordered list). Called at zshrs startup before arg parsing.
/// zshrs-original — no C counterpart.
pub fn try_load_embedded(exe: &Path) -> Option<EmbeddedFiles> {
    let mut f = File::open(exe).ok()?;
    let size = f.metadata().ok()?.len();
    if size < TRAILER_LEN {
        return None;
    }
    f.seek(SeekFrom::End(-(TRAILER_LEN as i64))).ok()?;
    let mut trailer = [0u8; TRAILER_LEN as usize];
    f.read_exact(&mut trailer).ok()?;
    if &trailer[24..32] != AOT_MAGIC {
        return None;
    }
    let compressed_len = u64::from_le_bytes(trailer[0..8].try_into().ok()?);
    let uncompressed_len = u64::from_le_bytes(trailer[8..16].try_into().ok()?);
    let version = u32::from_le_bytes(trailer[16..20].try_into().ok()?);
    if compressed_len == 0 || compressed_len > size - TRAILER_LEN {
        return None;
    }
    let payload_start = size - TRAILER_LEN - compressed_len;
    f.seek(SeekFrom::Start(payload_start)).ok()?;
    let mut compressed = vec![0u8; compressed_len as usize];
    f.read_exact(&mut compressed).ok()?;
    let payload = zstd::stream::decode_all(&compressed[..]).ok()?;
    if payload.len() != uncompressed_len as usize {
        return None;
    }
    match version {
        AOT_VERSION_V1 => decode_payload_v1(&payload),
        AOT_VERSION_V2 => decode_payload_v2(&payload),
        _ => None,
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        let mut p = meta.permissions();
        p.set_mode(p.mode() | 0o111);
        let _ = fs::set_permissions(path, p);
    }
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) {}

/// Copy `src` to `dst`, skipping any existing AOT trailer on `src`. Prevents
/// nested builds from stacking trailers: building once with trailer-A then
/// building again with trailer-B would otherwise embed both, A then B.
fn copy_exe_without_trailer(src: &Path, dst: &Path) -> io::Result<()> {
    let mut sf = File::open(src)?;
    let size = sf.metadata()?.len();
    let keep = if size >= TRAILER_LEN {
        sf.seek(SeekFrom::End(-(TRAILER_LEN as i64)))?;
        let mut trailer = [0u8; TRAILER_LEN as usize];
        if sf.read_exact(&mut trailer).is_ok() && &trailer[24..32] == AOT_MAGIC {
            let compressed_len = u64::from_le_bytes(trailer[0..8].try_into().unwrap());
            if compressed_len > 0 && compressed_len <= size - TRAILER_LEN {
                size - TRAILER_LEN - compressed_len
            } else {
                size
            }
        } else {
            size
        }
    } else {
        size
    };
    sf.seek(SeekFrom::Start(0))?;
    let _ = fs::remove_file(dst);
    let mut df = File::create(dst)?;
    let mut remaining = keep;
    let mut buf = vec![0u8; 64 * 1024];
    while remaining > 0 {
        let n = std::cmp::min(remaining as usize, buf.len());
        sf.read_exact(&mut buf[..n])?;
        df.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    df.sync_all()?;
    Ok(())
}

/// `zbuild --in A --in B --out OUT`: bake A and B into a copy of the
/// running zshrs binary in input order, producing a self-contained AOT
/// executable. At runtime, all embedded files run sequentially under one
/// ShellExecutor — globals + functions from earlier files are visible
/// to later ones.
/// zshrs-original — no C counterpart. C zsh's `bin_zcompile()`
/// (Src/parse.c) writes a `.zwc` cache file but doesn't bundle into
/// the shell binary itself.
pub fn build(script_paths: &[PathBuf], out_path: &Path) -> Result<PathBuf, String> {
    if script_paths.is_empty() {
        return Err("zbuild: at least one --in PATH required".to_string());
    }
    let mut files: Vec<EmbeddedFile> = Vec::with_capacity(script_paths.len());
    for p in script_paths {
        let source = fs::read_to_string(p)
            .map_err(|e| format!("zbuild: cannot read {}: {}", p.display(), e))?;
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("script.zsh")
            .to_string();
        files.push(EmbeddedFile { name, source });
    }
    let exe = std::env::current_exe()
        .map_err(|e| format!("zbuild: locating current executable: {}", e))?;
    copy_exe_without_trailer(&exe, out_path).map_err(|e| {
        format!(
            "zbuild: copy {} -> {}: {}",
            exe.display(),
            out_path.display(),
            e
        )
    })?;
    append_embedded_files(out_path, &files).map_err(|e| format!("zbuild: write trailer: {}", e))?;
    set_executable(out_path);
    Ok(out_path.to_path_buf())
}

// ───────────────────────── Native AOT (`zbuild --native`) ──────────────────
//
// Unlike the source-trailer build above (which embeds the script text and
// re-runs it interpreted at startup), the native path compiles the script to a
// fusevm chunk, lowers that to native machine code via `fusevm::aot`
// (Cranelift `ObjectModule` → relocatable `.o`), and links the object against
// the zsh runtime staticlib (`libzsh.a`) into a standalone executable. The
// script's bytecode runs as native code with no interpreter dispatch loop;
// command execution, expansion, and cmdsubst still go through the embedded
// `ShellExecutor` (the shell runtime is linked in).

/// Frontend runtime hook invoked by `fusevm::aot::fusevm_aot_run_embedded` at
/// startup of a native AOT binary. Stands up a leaked-`'static` `ShellExecutor`,
/// seeds positional parameters from `argv`, installs it as the thread's
/// `CURRENT_EXECUTOR`, and registers the zsh builtins on the run VM — so host
/// calls from the native chunk reach a live shell exactly as the interpreter's
/// `run_chunk` arranges. The executor lives for the process (an AOT binary runs
/// one program and exits), so the install is intentionally permanent.
///
/// # Safety
/// `vm` is the live run VM passed by the fusevm runtime; borrowed only here.
#[no_mangle]
pub extern "C" fn fusevm_aot_register_builtins(vm: *mut fusevm::VM) {
    // SAFETY: the fusevm runtime hands us the live run VM for this call.
    let vm = unsafe { &mut *vm };
    let exec: &'static mut crate::vm_helper::ShellExecutor =
        Box::leak(Box::new(crate::vm_helper::ShellExecutor::new()));
    // Positional params $1.. come from argv after the program name.
    let args: Vec<String> = std::env::args().skip(1).collect();
    exec.set_pparams(args);
    let last = exec.last_status();
    crate::fusevm_bridge::register_builtins(vm);
    vm.last_status = last;
    // Install permanently: forget the RAII guard so CURRENT_EXECUTOR stays set
    // for the whole native run (the process is one-shot).
    std::mem::forget(crate::fusevm_bridge::ExecutorContext::enter(exec));
}

/// Compile zsh `source` to a fusevm chunk via the normal parse + compile path.
fn compile_source_to_chunk(source: &str) -> Result<fusevm::Chunk, String> {
    let program = crate::vm_helper::parse_isolated(source);
    let chunk = crate::compile_zsh::ZshCompiler::new().compile(&program);
    if chunk.ops.is_empty() {
        return Err("zbuild --native: script compiled to an empty chunk".to_string());
    }
    Ok(chunk)
}

/// Locate the zsh runtime staticlib to link against. `ZSHRS_AOT_RUNTIME_LIB`
/// overrides; otherwise look for `libzsh.a` beside the running executable
/// (the dev `target/<profile>/` layout).
fn runtime_staticlib() -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("ZSHRS_AOT_RUNTIME_LIB") {
        return Ok(PathBuf::from(p));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if let Some(dir) = exe.parent() {
        let cand = dir.join("libzsh.a");
        if cand.exists() {
            return Ok(cand);
        }
    }
    Err("could not locate libzsh.a (set ZSHRS_AOT_RUNTIME_LIB)".to_string())
}

/// `zbuild --native --in A.zsh [--in B.zsh] --out OUT`: AOT-compile the inputs
/// to native machine code and link a standalone executable. Multiple inputs are
/// concatenated in order into one program, matching the source-trailer build.
pub fn build_native(script_paths: &[PathBuf], out_path: &Path) -> Result<PathBuf, String> {
    if script_paths.is_empty() {
        return Err("zbuild --native: at least one --in PATH required".to_string());
    }
    let mut source = String::new();
    for p in script_paths {
        let s = fs::read_to_string(p)
            .map_err(|e| format!("zbuild --native: cannot read {}: {}", p.display(), e))?;
        source.push_str(&s);
        if !source.ends_with('\n') {
            source.push('\n');
        }
    }
    let chunk = compile_source_to_chunk(&source)?;

    let runtime_lib = runtime_staticlib()?;
    if !runtime_lib.exists() {
        return Err(format!(
            "zbuild --native: runtime staticlib not found at {}",
            runtime_lib.display()
        ));
    }

    let obj = out_path.with_extension("o");
    fusevm::aot::compile_object(&chunk, &obj).map_err(|e| format!("zbuild --native: {}", e))?;

    let stub = out_path.with_extension("aot_main.c");
    fs::write(
        &stub,
        b"extern long fusevm_aot_run_embedded(void);\nint main(void){return (int)fusevm_aot_run_embedded();}\n" as &[u8],
    )
    .map_err(|e| format!("zbuild --native: write entry stub: {}", e))?;

    let mut cmd = std::process::Command::new("cc");
    cmd.arg(&stub).arg(&obj).arg(&runtime_lib);
    // zsh's terminfo/termcap modules need the system terminal library.
    cmd.arg("-lncurses");
    if cfg!(target_os = "macos") {
        // Frameworks pulled by zsh's transitive deps: chrono/iana-time-zone
        // (CoreFoundation), notify/FSEvents (CoreServices), and TLS/keychain
        // and network-config crates (Security, SystemConfiguration).
        for fw in [
            "CoreFoundation",
            "CoreServices",
            "Security",
            "SystemConfiguration",
        ] {
            cmd.arg("-framework").arg(fw);
        }
    }
    cmd.arg("-o").arg(out_path);
    let status = cmd
        .status()
        .map_err(|e| format!("zbuild --native: invoking cc: {}", e))?;
    let _ = fs::remove_file(&stub);
    let _ = fs::remove_file(&obj);
    if !status.success() {
        return Err(format!(
            "zbuild --native: link failed (cc exit {:?})",
            status.code()
        ));
    }
    set_executable(out_path);
    Ok(out_path.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mkfile(name: &str, src: &str) -> EmbeddedFile {
        EmbeddedFile {
            name: name.into(),
            source: src.into(),
        }
    }

    #[test]
    fn build_trailer_layout_matches_spec() {
        let t = build_trailer(0x11_22_33_44, 0xAA_BB_CC_DD, AOT_VERSION_V2);
        assert_eq!(t.len(), TRAILER_LEN as usize);
        // little-endian u64 compressed_len
        assert_eq!(&t[0..8], &0x11_22_33_44u64.to_le_bytes());
        // little-endian u64 uncompressed_len
        assert_eq!(&t[8..16], &0xAA_BB_CC_DDu64.to_le_bytes());
        // little-endian u32 version
        assert_eq!(&t[16..20], &AOT_VERSION_V2.to_le_bytes());
        // 20..24 reserved zeros
        assert_eq!(&t[20..24], &[0u8; 4]);
        // magic
        assert_eq!(&t[24..32], AOT_MAGIC);
    }

    #[test]
    fn payload_v2_roundtrip_single_file() {
        let files = vec![mkfile("hello.zsh", "echo hi\n")];
        let encoded = encode_payload_v2(&files);
        let decoded = decode_payload_v2(&encoded).expect("decode v2");
        assert_eq!(decoded.0.len(), 1);
        assert_eq!(decoded.0[0].name, "hello.zsh");
        assert_eq!(decoded.0[0].source, "echo hi\n");
    }

    #[test]
    fn payload_v2_roundtrip_multiple_files_preserves_order() {
        let files = vec![
            mkfile("a.zsh", "echo a\n"),
            mkfile("b.zsh", "echo b\n"),
            mkfile("c.zsh", "echo c\n"),
        ];
        let encoded = encode_payload_v2(&files);
        let decoded = decode_payload_v2(&encoded).expect("decode v2");
        assert_eq!(decoded.0.len(), 3);
        assert_eq!(decoded.0[0].name, "a.zsh");
        assert_eq!(decoded.0[1].name, "b.zsh");
        assert_eq!(decoded.0[2].name, "c.zsh");
        assert_eq!(decoded.0[0].source, "echo a\n");
        assert_eq!(decoded.0[1].source, "echo b\n");
        assert_eq!(decoded.0[2].source, "echo c\n");
    }

    #[test]
    fn payload_v2_roundtrip_zero_files() {
        let files: Vec<EmbeddedFile> = vec![];
        let encoded = encode_payload_v2(&files);
        // u32 count=0 → 4 bytes of zero.
        assert_eq!(encoded.as_slice(), &0u32.to_le_bytes());
        let decoded = decode_payload_v2(&encoded).expect("decode zero-file v2");
        assert!(decoded.0.is_empty());
    }

    #[test]
    fn payload_v2_handles_empty_source() {
        let files = vec![mkfile("empty.zsh", "")];
        let encoded = encode_payload_v2(&files);
        let decoded = decode_payload_v2(&encoded).expect("decode empty source");
        assert_eq!(decoded.0[0].source, "");
        assert_eq!(decoded.0[0].name, "empty.zsh");
    }

    #[test]
    fn payload_v2_preserves_utf8_in_names_and_sources() {
        let files = vec![
            mkfile("名前.zsh", "echo こんにちは\n"),
            mkfile("emoji-🚀.zsh", "echo $'\\xf0\\x9f\\x9a\\x80'\n"),
        ];
        let encoded = encode_payload_v2(&files);
        let decoded = decode_payload_v2(&encoded).expect("decode utf8");
        assert_eq!(decoded.0[0].name, "名前.zsh");
        assert_eq!(decoded.0[0].source, "echo こんにちは\n");
        assert_eq!(decoded.0[1].name, "emoji-🚀.zsh");
    }

    #[test]
    fn payload_v2_rejects_truncated_input() {
        let files = vec![mkfile("x.zsh", "echo x\n")];
        let mut encoded = encode_payload_v2(&files);
        // Drop the last byte — source ends prematurely.
        encoded.pop();
        assert!(decode_payload_v2(&encoded).is_none());
    }

    #[test]
    fn payload_v2_rejects_empty_buffer() {
        assert!(decode_payload_v2(&[]).is_none());
    }

    #[test]
    fn payload_v2_rejects_lying_count_header() {
        // Header claims 5 files but no body follows.
        let mut buf = Vec::new();
        buf.extend_from_slice(&5u32.to_le_bytes());
        assert!(decode_payload_v2(&buf).is_none());
    }

    #[test]
    fn payload_v1_decodes_legacy_single_script() {
        let name = "old.zsh";
        let source = "echo legacy\n";
        let mut buf = Vec::new();
        buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
        buf.extend_from_slice(name.as_bytes());
        buf.extend_from_slice(source.as_bytes());
        let decoded = decode_payload_v1(&buf).expect("decode v1");
        assert_eq!(decoded.0.len(), 1);
        assert_eq!(decoded.0[0].name, name);
        assert_eq!(decoded.0[0].source, source);
    }

    #[test]
    fn payload_v1_rejects_short_buffer() {
        assert!(decode_payload_v1(&[]).is_none());
        assert!(decode_payload_v1(&[1, 2, 3]).is_none());
    }

    #[test]
    fn payload_v1_rejects_name_len_larger_than_buffer() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&999u32.to_le_bytes()); // name_len = 999
        buf.extend_from_slice(b"abc"); // only 3 bytes follow
        assert!(decode_payload_v1(&buf).is_none());
    }

    #[test]
    fn payload_v1_handles_empty_source_after_name() {
        let mut buf = Vec::new();
        let name = "x";
        buf.extend_from_slice(&(name.len() as u32).to_le_bytes());
        buf.extend_from_slice(name.as_bytes());
        // No source bytes after — should still decode with empty source.
        let decoded = decode_payload_v1(&buf).expect("decode v1 empty source");
        assert_eq!(decoded.0[0].source, "");
    }

    #[test]
    fn append_and_load_roundtrip_single_file() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        // Pre-populate with non-magic data simulating an existing binary.
        std::fs::write(tmp.path(), b"FAKE-ELF-PREFIX").expect("write prefix");
        let files = vec![mkfile("greet.zsh", "echo hello\n")];
        append_embedded_files(tmp.path(), &files).expect("append");
        let loaded = try_load_embedded(tmp.path()).expect("load back");
        assert_eq!(loaded.0.len(), 1);
        assert_eq!(loaded.0[0].name, "greet.zsh");
        assert_eq!(loaded.0[0].source, "echo hello\n");
    }

    #[test]
    fn append_and_load_roundtrip_multiple_files() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        std::fs::write(tmp.path(), b"PREFIX").expect("write prefix");
        let files = vec![
            mkfile("first.zsh", "first()  { :; }\n"),
            mkfile("second.zsh", "first\nsecond()  { :; }\n"),
            mkfile("third.zsh", "echo done\n"),
        ];
        append_embedded_files(tmp.path(), &files).expect("append");
        let loaded = try_load_embedded(tmp.path()).expect("load back");
        assert_eq!(loaded.0.len(), 3);
        assert_eq!(loaded.0[0].name, "first.zsh");
        assert_eq!(loaded.0[1].name, "second.zsh");
        assert_eq!(loaded.0[2].name, "third.zsh");
    }

    #[test]
    fn try_load_embedded_returns_none_without_magic() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        // Write at least TRAILER_LEN bytes but no magic in the trailing slot.
        std::fs::write(tmp.path(), vec![0u8; 64]).expect("write");
        assert!(try_load_embedded(tmp.path()).is_none());
    }

    #[test]
    fn try_load_embedded_returns_none_for_small_file() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        // Smaller than TRAILER_LEN — fast bail.
        std::fs::write(tmp.path(), b"tiny").expect("write");
        assert!(try_load_embedded(tmp.path()).is_none());
    }

    #[test]
    fn try_load_embedded_returns_none_for_missing_path() {
        let path = std::path::PathBuf::from("/this/path/does/not/exist/zshrs-aot");
        assert!(try_load_embedded(&path).is_none());
    }

    #[test]
    fn try_load_embedded_returns_none_for_zero_compressed_len() {
        // Forge a trailer with valid magic but compressed_len=0.
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let mut data = vec![0u8; 100];
        let trailer = build_trailer(0, 0, AOT_VERSION_V2);
        data.extend_from_slice(&trailer);
        std::fs::write(tmp.path(), &data).expect("write");
        assert!(try_load_embedded(tmp.path()).is_none());
    }

    #[test]
    fn try_load_embedded_rejects_unknown_version() {
        // Build a valid magic + size frame, but version=99 — unsupported.
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let payload = encode_payload_v2(&[mkfile("x.zsh", "y\n")]);
        let compressed = zstd::stream::encode_all(&payload[..], 3).expect("zstd");
        let mut data = vec![0u8; 32]; // prefix
        data.extend_from_slice(&compressed);
        let trailer = build_trailer(compressed.len() as u64, payload.len() as u64, 99);
        data.extend_from_slice(&trailer);
        std::fs::write(tmp.path(), &data).expect("write");
        assert!(try_load_embedded(tmp.path()).is_none());
    }

    #[test]
    fn try_load_embedded_rejects_corrupt_uncompressed_len() {
        // Magic + version OK, but uncompressed_len lies → decoder returns None.
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let payload = encode_payload_v2(&[mkfile("x.zsh", "y\n")]);
        let compressed = zstd::stream::encode_all(&payload[..], 3).expect("zstd");
        let mut data = vec![0u8; 32];
        data.extend_from_slice(&compressed);
        // Lie: claim uncompressed is one byte larger than reality.
        let trailer = build_trailer(
            compressed.len() as u64,
            payload.len() as u64 + 1,
            AOT_VERSION_V2,
        );
        data.extend_from_slice(&trailer);
        std::fs::write(tmp.path(), &data).expect("write");
        assert!(try_load_embedded(tmp.path()).is_none());
    }

    #[test]
    fn build_rejects_empty_input_list() {
        let out = std::path::PathBuf::from("/tmp/zshrs-aot-empty-out");
        let res = build(&[], &out);
        assert!(res.is_err());
        let err = res.unwrap_err();
        assert!(err.contains("at least one"), "got: {err}");
    }
}
