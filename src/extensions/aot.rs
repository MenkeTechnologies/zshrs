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
use std::path::{Path, PathBuf};
use std::os::unix::fs::PermissionsExt;

/// 8-byte trailer magic.
pub const AOT_MAGIC: &[u8; 8] = b"ZSHRSAOT";
/// Trailer format version 1: single script (legacy decode-only).
pub const AOT_VERSION_V1: u32 = 1;
/// Trailer format version 2: ordered file list (current build output).
pub const AOT_VERSION_V2: u32 = 2;
/// Fixed trailer length: `8 (cl) + 8 (ul) + 4 (ver) + 4 (rsv) + 8 (magic)`.
pub const TRAILER_LEN: u64 = 32;

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
