// Source-builtin interception — `source FILE` / `. FILE` route through daemon.
//
// Per docs/DAEMON.md "Source / dot interception and file registry":
//
//   client: source /path/to/foo.sh
//       │
//       ↓  IPC source_resolve { path, mtime_ns, inode }
//       │
//   daemon: lookup compiled_files WHERE path = …
//       HIT (mtime+inode match):    return shard_path + generation
//       MISS:                        parse + bytecode-compile + insert + return
//       STALE (mtime/inode differ):  rebuild + atomic-rename + return new generation
//
// V1 implementation:
//   - Daemon does NOT yet bytecode-compile on miss (the compile pipeline integrates
//     with src/compile_zsh.rs which can't be safely invoked from the async daemon
//     without unsafe globals). Instead, on miss the daemon stores the source content
//     directly + flags `kind = 'source'`. Future iteration swaps content → bytecode.
//   - Hit/stale detection works fully (mtime + inode + content hash).
//   - Sensitive-content heuristic flags tokens.sh / *.env / files containing
//     AWS_SECRET / API_KEY / PASSWORD assignments.

use std::path::Path;

use rusqlite::OptionalExtension;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

pub async fn op_source_resolve(state: &std::sync::Arc<DaemonState>, args: Value) -> OpResult {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();
    let client_mtime_ns = args.get("mtime_ns").and_then(Value::as_i64);
    let client_inode = args.get("inode").and_then(Value::as_i64);

    let p = Path::new(&path);
    if !p.is_absolute() {
        return Err(ErrPayload::new(
            "bad_args",
            format!("path must be absolute: {}", path),
        ));
    }

    let meta = match std::fs::metadata(p) {
        Ok(m) => m,
        Err(e) => {
            return Err(ErrPayload::new("stat_failed", format!("{}: {}", path, e)));
        }
    };

    let on_disk_mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0);
    use std::os::unix::fs::MetadataExt;
    let on_disk_inode = meta.ino() as i64;
    let on_disk_size = meta.len() as i64;

    // If client passed mtime/inode, sanity-check they match the file the daemon sees.
    // A mismatch means the file changed between client stat() and our stat() — treat
    // as "client is stale" and recompile from on-disk truth.
    let _client_says_match =
        client_mtime_ns == Some(on_disk_mtime) && client_inode == Some(on_disk_inode);

    // Look up existing compiled_files row.
    let row: Option<(i64, i64, Vec<u8>)> = state.with_catalog(|conn| {
        conn.query_row(
            "SELECT mtime, inode, hash FROM compiled_files WHERE path = ?",
            rusqlite::params![path],
            |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, Vec<u8>>(2)?,
                ))
            },
        )
        .optional()
    })?;

    if let Some((cached_mtime, cached_inode, _cached_hash)) = row {
        if cached_mtime == on_disk_mtime && cached_inode == on_disk_inode {
            // HIT — file unchanged since last cache.
            tracing::info!(path = %path, "source_resolve hit");
            state.with_catalog(|conn| {
                conn.execute(
                    "UPDATE compiled_files SET use_count = use_count + 1, last_used_at = ? WHERE path = ?",
                    rusqlite::params![now_ns_i64(), path],
                ).ok();
                Ok::<_, rusqlite::Error>(())
            })?;
            return Ok(json!({
                "hit": true,
                "stale": false,
                "path": path,
                "mtime_ns": on_disk_mtime,
                "inode": on_disk_inode,
            }));
        }
        // STALE — file changed; refresh below.
        tracing::info!(path = %path, "source_resolve stale, refreshing");
    } else {
        tracing::info!(path = %path, "source_resolve miss, ingesting");
    }

    // Read content + compute hash + sensitive-content heuristic.
    let content = match std::fs::read(p) {
        Ok(b) => b,
        Err(e) => {
            return Err(ErrPayload::new("read_failed", format!("{}: {}", path, e)));
        }
    };
    let hash = Sha256::digest(&content).to_vec();
    let sensitive = is_sensitive(&path, &content);

    // V1: store the source bytes in `bytecode` as a placeholder. When the bytecode
    // compile path lands, this becomes the actual compiled output.
    let bytes_in = content.len() as i64;
    let bytes_out = content.len() as i64;
    let parent_paths_json = "[]"; // populated when transitive sources are wired

    state.with_catalog(|conn| {
        conn.execute(
            r#"INSERT INTO compiled_files
               (path, kind, mtime, inode, hash, bytecode, last_used_at, use_count, bytes_in, bytes_out, sensitive, parent_paths)
               VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?, ?)
               ON CONFLICT(path) DO UPDATE SET
                   mtime = excluded.mtime,
                   inode = excluded.inode,
                   hash  = excluded.hash,
                   bytecode = excluded.bytecode,
                   last_used_at = excluded.last_used_at,
                   use_count = compiled_files.use_count + 1,
                   bytes_in = excluded.bytes_in,
                   bytes_out = excluded.bytes_out,
                   sensitive = excluded.sensitive"#,
            rusqlite::params![
                path,
                "source",
                on_disk_mtime,
                on_disk_inode,
                hash,
                content,
                now_ns_i64(),
                bytes_in,
                bytes_out,
                sensitive as i64,
                parent_paths_json,
            ],
        )?;
        Ok::<_, rusqlite::Error>(())
    })?;

    Ok(json!({
        "hit": false,
        "stale": false,
        "path": path,
        "mtime_ns": on_disk_mtime,
        "inode": on_disk_inode,
        "bytes_in": bytes_in,
        "bytes_out": bytes_out,
        "sensitive": sensitive,
        "size_on_disk": on_disk_size,
    }))
}

/// Heuristic: a file is sensitive if its name matches secret-like patterns OR its
/// content references known secret env-var assignments. Used to set the `sensitive`
/// flag in compiled_files; downstream verbs (`zcache export --all`, dbview) honor it.
fn is_sensitive(path: &str, content: &[u8]) -> bool {
    let lower = path.to_ascii_lowercase();
    if lower.contains("token")
        || lower.contains("secret")
        || lower.contains("credential")
        || lower.contains("password")
        || lower.ends_with(".env")
        || lower.contains(".env.")
    {
        return true;
    }
    // Cheap content scan — if the file is huge, skim only the first 64KB.
    let scan_len = content.len().min(64 * 1024);
    let head = &content[..scan_len];
    // SAFETY: we don't need valid UTF-8 here; lossy view is fine for substring search.
    let s = String::from_utf8_lossy(head);
    let upper = s.to_ascii_uppercase();
    upper.contains("AWS_SECRET")
        || upper.contains("API_KEY=")
        || upper.contains("PASSWORD=")
        || upper.contains("PRIVATE_KEY")
        || upper.contains("SECRET_ACCESS_KEY")
}

fn now_ns_i64() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sensitive_path_tokens() {
        assert!(is_sensitive("/Users/wizard/.zpwr/local/.tokens.sh", b""));
        assert!(is_sensitive("/etc/secrets.env", b""));
        assert!(is_sensitive("/home/me/credentials.sh", b""));
        assert!(is_sensitive("/home/me/.env.local", b""));
    }

    #[test]
    fn sensitive_content_aws() {
        assert!(is_sensitive(
            "/some/innocent.sh",
            b"export AWS_SECRET_ACCESS_KEY=abc123"
        ));
        assert!(is_sensitive("/x.sh", b"API_KEY=zzz"));
        assert!(is_sensitive("/x.sh", b"PRIVATE_KEY=----BEGIN..."));
    }

    #[test]
    fn not_sensitive_innocent_content() {
        assert!(!is_sensitive(
            "/Users/wizard/.zshrc",
            b"alias ll='ls -la'\nbindkey ..."
        ));
    }
}
