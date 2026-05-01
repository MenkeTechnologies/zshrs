//! Ahead-of-time build: bake a shell script into a copy of the running `zshrs`
//! binary as a compressed trailer, producing a self-contained executable.
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
//! Payload v1 (single script, before zstd compression):
//!
//! ```text
//!   [u32 script_name_len]
//!   [script_name utf8]
//!   [source bytes utf8]
//! ```
//!
//! Direct port of `strykelang/strykelang/aot.rs` adapted for zsh source.
//! Same trailer-on-binary trick; ELF (Linux) and Mach-O (macOS) loaders ignore
//! bytes past the program-header-listed segments, so appending leaves the
//! original `zshrs` fully runnable.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// 8-byte trailer magic.
pub const AOT_MAGIC: &[u8; 8] = b"ZSHRSAOT";
/// Trailer format version 1: single script.
pub const AOT_VERSION_V1: u32 = 1;
/// Fixed trailer length: `8 (cl) + 8 (ul) + 4 (ver) + 4 (rsv) + 8 (magic)`.
pub const TRAILER_LEN: u64 = 32;

#[derive(Debug, Clone)]
pub struct EmbeddedScript {
    /// `__FILE__` / error-reporting name (e.g. `hello.zsh`).
    pub name: String,
    /// UTF-8 zsh source.
    pub source: String,
}

fn encode_payload_v1(name: &str, source: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + name.len() + source.len());
    let name_len = u32::try_from(name.len()).expect("script name length fits in u32");
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(name.as_bytes());
    out.extend_from_slice(source.as_bytes());
    out
}

fn decode_payload_v1(bytes: &[u8]) -> Option<EmbeddedScript> {
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
    Some(EmbeddedScript { name, source })
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

/// Append a compressed v1 script payload to an existing file.
pub fn append_embedded_script(out_path: &Path, name: &str, source: &str) -> io::Result<()> {
    let payload = encode_payload_v1(name, source);
    let compressed = zstd::stream::encode_all(&payload[..], 3)?;
    let mut f = OpenOptions::new().append(true).open(out_path)?;
    f.write_all(&compressed)?;
    let trailer = build_trailer(
        compressed.len() as u64,
        payload.len() as u64,
        AOT_VERSION_V1,
    );
    f.write_all(&trailer)?;
    f.sync_all()?;
    Ok(())
}

/// Fast probe: read the last 32 bytes of `exe` and return the embedded script
/// if present. Called at zshrs startup (before arg parsing) so an exe with a
/// trailer runs the embedded script directly instead of the REPL.
pub fn try_load_embedded_script(exe: &Path) -> Option<EmbeddedScript> {
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
        _ => None,
    }
}

#[cfg(unix)]
fn set_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
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

/// `zbuild --in SCRIPT --out OUT`: bake SCRIPT into a copy of the running
/// zshrs binary, producing a self-contained AOT executable.
pub fn build(script_path: &Path, out_path: &Path) -> Result<PathBuf, String> {
    let source = fs::read_to_string(script_path)
        .map_err(|e| format!("zbuild: cannot read {}: {}", script_path.display(), e))?;
    let script_name = script_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("script.zsh")
        .to_string();
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
    append_embedded_script(out_path, &script_name, &source)
        .map_err(|e| format!("zbuild: write trailer: {}", e))?;
    set_executable(out_path);
    Ok(out_path.to_path_buf())
}
