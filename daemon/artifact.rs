//! `daemon.artifact.*` ops — content-addressed artifact cache.
//!
//! Per docs/DAEMON_AS_SERVICE.md: "content-addressed cache, rkyv-backed
//! for zero-copy reads". Replaces sccache / ccache / per-tool caches
//! with a single shared artifact store keyed by sha256. Multiple names
//! can point at the same digest (deduplication is automatic).
//!
//! Filesystem layout:
//!
//! ```
//! ~/.zshrs/artifacts/
//!   ab/                      ← first 2 hex chars of digest as shard dir
//!     abcdef0123…ff          ← full sha256 as filename, content == bytes
//!   cd/
//!     cd1234…
//!   names.db                 ← sqlite mapping name → digest + metadata
//! ```
//!
//! Storage is dedup'd: putting the same content under two different
//! names creates one blob file + two `names.db` rows.
//!
//! Op surface:
//!
//! | Op                          | Args                       | Returns                              |
//! |-----------------------------|----------------------------|--------------------------------------|
//! | `artifact_put`              | `{name, value}`            | `{ok, digest, name, bytes}`          |
//! | `artifact_get`              | `{name}`                   | `{ok, digest, value, bytes}` or 404  |
//! | `artifact_get_by_digest`    | `{digest}`                 | `{ok, digest, value, bytes}` or 404  |
//! | `artifact_gc`               | `{max_age_secs?, max_bytes?}` | `{ok, freed_bytes, removed: N}`  |
//! | `artifact_list`             | `{prefix?}`                | `{ok, entries: [...], count}`        |

use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

fn names_db_path(state: &DaemonState) -> PathBuf {
    state.paths.artifacts_dir.join("names.db")
}

fn open_names_db(state: &DaemonState) -> std::result::Result<Connection, ErrPayload> {
    let _ = std::fs::create_dir_all(&state.paths.artifacts_dir);
    let conn = Connection::open(names_db_path(state))
        .map_err(|e| ErrPayload::new("artifact_open", e.to_string()))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous  = NORMAL;
        CREATE TABLE IF NOT EXISTS artifacts (
            name        TEXT PRIMARY KEY,
            digest_hex  TEXT NOT NULL,
            bytes       INTEGER NOT NULL,
            created_at  INTEGER NOT NULL,
            updated_at  INTEGER NOT NULL,
            last_get_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS artifacts_digest ON artifacts(digest_hex);
        "#,
    )
    .map_err(|e| ErrPayload::new("artifact_schema", e.to_string()))?;
    Ok(conn)
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

fn blob_path(state: &DaemonState, digest_hex: &str) -> PathBuf {
    let prefix = &digest_hex[..2.min(digest_hex.len())];
    state
        .paths
        .artifacts_dir
        .join(prefix)
        .join(digest_hex)
}

fn name_arg(args: &Value) -> std::result::Result<String, ErrPayload> {
    args.get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `name`"))
}

fn digest_arg(args: &Value) -> std::result::Result<String, ErrPayload> {
    args.get("digest")
        .and_then(Value::as_str)
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
        .map(str::to_string)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing or malformed `digest` (need 64-hex sha256)"))
}

/// Decode the request `value` field. Accepts either:
///   - a JSON string (treated as UTF-8 text bytes), or
///   - `{"value_base64": "..."}`-style on-the-wire binary, OR
///   - a raw JSON value (object/array → its JSON text serialised)
fn decode_value(args: &Value) -> std::result::Result<Vec<u8>, ErrPayload> {
    if let Some(b64) = args.get("value_base64").and_then(Value::as_str) {
        return base64::engine::general_purpose::STANDARD
            .decode(b64)
            .map_err(|e| ErrPayload::new("bad_args", format!("value_base64 decode: {e}")));
    }
    match args.get("value") {
        Some(Value::String(s)) => Ok(s.as_bytes().to_vec()),
        Some(other) => Ok(other.to_string().into_bytes()),
        None => Err(ErrPayload::new("bad_args", "missing `value` or `value_base64`")),
    }
}

pub async fn op_artifact_put(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let name = name_arg(&args)?;
    let bytes = decode_value(&args)?;
    let mut h = Sha256::new();
    h.update(&bytes);
    let digest = h.finalize();
    let digest_hex = hex_lower(&digest);
    let blob = blob_path(state, &digest_hex);
    if !blob.exists() {
        if let Some(p) = blob.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        // Atomic write via tmp + rename so concurrent puts of the same
        // digest don't corrupt the blob.
        let tmp = blob.with_extension("tmp");
        std::fs::write(&tmp, &bytes)
            .map_err(|e| ErrPayload::new("artifact_write", e.to_string()))?;
        std::fs::rename(&tmp, &blob)
            .map_err(|e| ErrPayload::new("artifact_rename", e.to_string()))?;
    }

    let now = now_ns();
    let conn = open_names_db(state)?;
    conn.execute(
        r#"
        INSERT INTO artifacts (name, digest_hex, bytes, created_at, updated_at, last_get_at)
        VALUES (?1, ?2, ?3, ?4, ?4, NULL)
        ON CONFLICT(name) DO UPDATE SET
            digest_hex = excluded.digest_hex,
            bytes      = excluded.bytes,
            updated_at = excluded.updated_at
        "#,
        params![name, digest_hex, bytes.len() as i64, now],
    )
    .map_err(|e| ErrPayload::new("artifact_index", e.to_string()))?;
    Ok(json!({
        "name": name,
        "digest": digest_hex,
        "bytes": bytes.len(),
    }))
}

pub async fn op_artifact_get(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let name = name_arg(&args)?;
    let conn = open_names_db(state)?;
    let row: Option<(String, i64)> = conn
        .query_row(
            "SELECT digest_hex, bytes FROM artifacts WHERE name = ?1",
            params![name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| ErrPayload::new("artifact_get", e.to_string()))?;
    let (digest, _) = row.ok_or_else(|| {
        ErrPayload::new("no_such_file", format!("artifact `{name}` not found"))
    })?;
    let _ = conn.execute(
        "UPDATE artifacts SET last_get_at = ?1 WHERE name = ?2",
        params![now_ns(), name],
    );
    fetch_blob(state, &digest, Some(&name))
}

pub async fn op_artifact_get_by_digest(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let digest = digest_arg(&args)?;
    fetch_blob(state, &digest, None)
}

fn fetch_blob(
    state: &DaemonState,
    digest: &str,
    name: Option<&str>,
) -> OpResult {
    let blob = blob_path(state, digest);
    if !blob.exists() {
        return Err(ErrPayload::new(
            "no_such_file",
            format!("blob {digest} missing on disk (did artifact_gc reap it?)"),
        ));
    }
    let bytes = std::fs::read(&blob)
        .map_err(|e| ErrPayload::new("artifact_read", e.to_string()))?;
    let value_b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(json!({
        "digest": digest,
        "name": name,
        "bytes": bytes.len(),
        "value_base64": value_b64,
    }))
}

pub async fn op_artifact_gc(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let max_age_secs = args.get("max_age_secs").and_then(Value::as_i64);
    let max_bytes = args.get("max_bytes").and_then(Value::as_i64);
    let now = now_ns();

    let conn = open_names_db(state)?;
    let cutoff_ns: Option<i64> =
        max_age_secs.map(|s| now - s.saturating_mul(1_000_000_000));

    // Collect candidates for removal: rows where last_get_at (or
    // updated_at if never read) is older than cutoff_ns.
    let mut to_remove: Vec<(String, String, i64)> = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT name, digest_hex, bytes, COALESCE(last_get_at, updated_at) FROM artifacts")
            .map_err(|e| ErrPayload::new("artifact_gc_scan", e.to_string()))?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })
            .map_err(|e| ErrPayload::new("artifact_gc_scan", e.to_string()))?;
        for row in rows.flatten() {
            if let Some(cut) = cutoff_ns {
                if row.3 < cut {
                    to_remove.push((row.0, row.1, row.2));
                }
            }
        }
    }

    // Apply max_bytes by removing oldest-touched first until under.
    if let Some(cap) = max_bytes {
        let mut remaining: Vec<(String, String, i64, i64)> = {
            let mut stmt = conn
                .prepare("SELECT name, digest_hex, bytes, COALESCE(last_get_at, updated_at) FROM artifacts ORDER BY 4 ASC")
                .map_err(|e| ErrPayload::new("artifact_gc_cap", e.to_string()))?;
            let rs = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                    ))
                })
                .map_err(|e| ErrPayload::new("artifact_gc_cap", e.to_string()))?;
            rs.flatten().collect()
        };
        let total: i64 = remaining.iter().map(|r| r.2).sum();
        let mut over = total - cap;
        while over > 0 {
            if remaining.is_empty() {
                break;
            }
            let r = remaining.remove(0);
            over -= r.2;
            if !to_remove.iter().any(|x| x.0 == r.0) {
                to_remove.push((r.0, r.1, r.2));
            }
        }
    }

    let mut freed_bytes: i64 = 0;
    let mut removed: i64 = 0;
    for (name, digest, bytes) in &to_remove {
        let _ = conn.execute("DELETE FROM artifacts WHERE name = ?1", params![name]);
        // Only remove the blob if no remaining row references this digest.
        let still_referenced: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM artifacts WHERE digest_hex = ?1",
                params![digest],
                |r| r.get(0),
            )
            .unwrap_or(0);
        if still_referenced == 0 {
            let blob = blob_path(state, digest);
            if blob.exists() {
                let _ = std::fs::remove_file(&blob);
                freed_bytes += *bytes;
            }
        }
        removed += 1;
    }
    Ok(json!({
        "removed": removed,
        "freed_bytes": freed_bytes,
    }))
}

pub async fn op_artifact_list(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let prefix = args
        .get("prefix")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let conn = open_names_db(state)?;
    let glob = format!("{}%", prefix.replace('%', "\\%").replace('_', "\\_"));
    let mut stmt = conn
        .prepare("SELECT name, digest_hex, bytes, created_at, updated_at, last_get_at FROM artifacts WHERE name LIKE ?1 ESCAPE '\\' ORDER BY name")
        .map_err(|e| ErrPayload::new("artifact_list", e.to_string()))?;
    let entries: Vec<Value> = stmt
        .query_map(params![glob], |r| {
            Ok(json!({
                "name": r.get::<_, String>(0)?,
                "digest": r.get::<_, String>(1)?,
                "bytes": r.get::<_, i64>(2)?,
                "created_at": r.get::<_, i64>(3)?,
                "updated_at": r.get::<_, i64>(4)?,
                "last_get_at": r.get::<_, Option<i64>>(5)?,
            }))
        })
        .map_err(|e| ErrPayload::new("artifact_list", e.to_string()))?
        .flatten()
        .collect();
    let count = entries.len();
    Ok(json!({ "entries": entries, "count": count }))
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02x}", b));
    }
    s
}
