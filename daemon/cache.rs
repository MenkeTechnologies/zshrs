//! `daemon.cache.*` ops — persistent namespaced KV cache.
//!
//! Per docs/DAEMON_AS_SERVICE.md: "persistent KV per client namespace,
//! sqlite-backed". This is the daemon's general-purpose user-space
//! cache layer — the moral equivalent of a single-user Redis without
//! the dedicated 50MB-RAM service. Every shell, editor plugin, build
//! script, language runtime can drop key/value pairs in here and read
//! them back across process restarts.
//!
//! Schema (single SQLite file at `~/.zshrs/cache.db`):
//!
//! ```sql
//! CREATE TABLE kv (
//!     ns         TEXT NOT NULL,
//!     k          TEXT NOT NULL,
//!     v          BLOB NOT NULL,
//!     created_at INTEGER NOT NULL,    -- epoch ns
//!     updated_at INTEGER NOT NULL,    -- epoch ns
//!     expires_at INTEGER,             -- NULL = never; epoch ns otherwise
//!     PRIMARY KEY (ns, k)
//! );
//! CREATE INDEX kv_expires ON kv(expires_at) WHERE expires_at IS NOT NULL;
//! ```
//!
//! Op surface:
//!
//! | Op            | Args                                | Returns                              |
//! |---------------|-------------------------------------|--------------------------------------|
//! | `cache_put`   | `{ns, key, value, ttl_secs?}`       | `{ok, ns, key, bytes}`               |
//! | `cache_get`   | `{ns, key}`                         | `{ok, value, found}` or 404          |
//! | `cache_del`   | `{ns, key}`                         | `{ok, deleted: bool}`                |
//! | `cache_list`  | `{ns, prefix?}`                     | `{ok, keys: [...], count}`           |
//! | `cache_stats` | `{ns?}`                             | `{ok, key_count, byte_count, ...}`   |
//!
//! Namespaces (`ns`) are arbitrary strings; clients pick their own
//! (`ci-pipeline`, `vim-lsp`, `my-script`, etc.). No quota enforcement
//! in v1 — single-user trust model.

use std::sync::Arc;

use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

/// Open the cache database (creating the file + table on first use).
/// One connection per call — caller drops at end of request. SQLite
/// handles concurrent readers/writers via WAL mode.
fn open_db(state: &DaemonState) -> std::result::Result<Connection, ErrPayload> {
    let path = &state.paths.cache_db;
    let conn = Connection::open(path).map_err(|e| ErrPayload::new("cache_open", e.to_string()))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous  = NORMAL;
        CREATE TABLE IF NOT EXISTS kv (
            ns         TEXT NOT NULL,
            k          TEXT NOT NULL,
            v          BLOB NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            expires_at INTEGER,
            PRIMARY KEY (ns, k)
        );
        CREATE INDEX IF NOT EXISTS kv_expires
            ON kv(expires_at) WHERE expires_at IS NOT NULL;
        "#,
    )
    .map_err(|e| ErrPayload::new("cache_schema", e.to_string()))?;
    Ok(conn)
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

fn ns_arg(args: &Value) -> std::result::Result<String, ErrPayload> {
    args.get("ns")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `ns`"))
}

fn key_arg(args: &Value) -> std::result::Result<String, ErrPayload> {
    args.get("key")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `key`"))
}

pub async fn op_cache_put(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let ns = ns_arg(&args)?;
    let key = key_arg(&args)?;
    // Accept value as either a raw string OR a JSON value (object/array
    // serialised to its string form). Bytes encoding is UTF-8.
    let value: Vec<u8> = match args.get("value") {
        Some(Value::String(s)) => s.as_bytes().to_vec(),
        Some(other) => other.to_string().into_bytes(),
        None => return Err(ErrPayload::new("bad_args", "missing `value`")),
    };
    let ttl_secs = args.get("ttl_secs").and_then(Value::as_i64);
    let now = now_ns();
    let expires_at: Option<i64> = ttl_secs.map(|secs| now + secs.saturating_mul(1_000_000_000));

    let conn = open_db(state)?;
    let bytes = value.len();
    conn.execute(
        r#"
        INSERT INTO kv (ns, k, v, created_at, updated_at, expires_at)
        VALUES (?1, ?2, ?3, ?4, ?4, ?5)
        ON CONFLICT(ns, k) DO UPDATE SET
            v = excluded.v,
            updated_at = excluded.updated_at,
            expires_at = excluded.expires_at
        "#,
        params![ns, key, value, now, expires_at],
    )
    .map_err(|e| ErrPayload::new("cache_put", e.to_string()))?;

    Ok(json!({
        "ns": ns,
        "key": key,
        "bytes": bytes,
        "expires_at": expires_at,
    }))
}

pub async fn op_cache_get(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let ns = ns_arg(&args)?;
    let key = key_arg(&args)?;
    let now = now_ns();
    let conn = open_db(state)?;
    let row: Option<(Vec<u8>, i64, i64, Option<i64>)> = conn
        .query_row(
            "SELECT v, created_at, updated_at, expires_at FROM kv WHERE ns = ?1 AND k = ?2",
            params![ns, key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(|e| ErrPayload::new("cache_get", e.to_string()))?;
    let row = match row {
        Some(r) => r,
        None => {
            return Err(ErrPayload::new(
                "no_such_file",
                format!("ns={} key={} not found", ns, key),
            ));
        }
    };
    if let Some(exp) = row.3 {
        if exp < now {
            // Expired — drop and report not-found.
            let _ = conn.execute(
                "DELETE FROM kv WHERE ns = ?1 AND k = ?2",
                params![ns, key],
            );
            return Err(ErrPayload::new(
                "no_such_file",
                format!("ns={} key={} expired", ns, key),
            ));
        }
    }
    // Try to interpret bytes as UTF-8; fall back to raw byte array
    // wire-encoded as a JSON array of u8 if non-UTF-8 (binary data).
    let value_json = match String::from_utf8(row.0.clone()) {
        Ok(s) => Value::String(s),
        Err(_) => json!(row.0),
    };
    Ok(json!({
        "ns": ns,
        "key": key,
        "value": value_json,
        "bytes": row.0.len(),
        "created_at": row.1,
        "updated_at": row.2,
        "expires_at": row.3,
    }))
}

pub async fn op_cache_del(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let ns = ns_arg(&args)?;
    let key = key_arg(&args)?;
    let conn = open_db(state)?;
    let n = conn
        .execute(
            "DELETE FROM kv WHERE ns = ?1 AND k = ?2",
            params![ns, key],
        )
        .map_err(|e| ErrPayload::new("cache_del", e.to_string()))?;
    Ok(json!({ "ns": ns, "key": key, "deleted": n > 0 }))
}

pub async fn op_cache_list(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let ns = ns_arg(&args)?;
    let prefix = args
        .get("prefix")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_default();
    let now = now_ns();
    let conn = open_db(state)?;
    let glob_arg = format!("{}%", prefix.replace('%', "\\%").replace('_', "\\_"));
    let mut stmt = conn
        .prepare(
            "SELECT k FROM kv WHERE ns = ?1 AND k LIKE ?2 ESCAPE '\\' \
             AND (expires_at IS NULL OR expires_at >= ?3) ORDER BY k",
        )
        .map_err(|e| ErrPayload::new("cache_list", e.to_string()))?;
    let rows = stmt
        .query_map(params![ns, glob_arg, now], |r| r.get::<_, String>(0))
        .map_err(|e| ErrPayload::new("cache_list", e.to_string()))?;
    let mut keys: Vec<String> = Vec::new();
    for k in rows.flatten() {
        keys.push(k);
    }
    let count = keys.len();
    Ok(json!({ "ns": ns, "keys": keys, "count": count }))
}

pub async fn op_cache_stats(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let now = now_ns();
    let conn = open_db(state)?;
    if let Some(ns) = args.get("ns").and_then(Value::as_str) {
        let (key_count, byte_count): (i64, i64) = conn
            .query_row(
                "SELECT COUNT(*), COALESCE(SUM(LENGTH(v)), 0) FROM kv \
                 WHERE ns = ?1 AND (expires_at IS NULL OR expires_at >= ?2)",
                params![ns, now],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| ErrPayload::new("cache_stats", e.to_string()))?;
        Ok(json!({
            "ns": ns,
            "key_count": key_count,
            "byte_count": byte_count,
        }))
    } else {
        let total_keys: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM kv WHERE expires_at IS NULL OR expires_at >= ?1",
                params![now],
                |r| r.get(0),
            )
            .map_err(|e| ErrPayload::new("cache_stats", e.to_string()))?;
        let total_bytes: i64 = conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(v)), 0) FROM kv \
                 WHERE expires_at IS NULL OR expires_at >= ?1",
                params![now],
                |r| r.get(0),
            )
            .map_err(|e| ErrPayload::new("cache_stats", e.to_string()))?;
        let ns_count: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT ns) FROM kv \
                 WHERE expires_at IS NULL OR expires_at >= ?1",
                params![now],
                |r| r.get(0),
            )
            .map_err(|e| ErrPayload::new("cache_stats", e.to_string()))?;
        Ok(json!({
            "key_count": total_keys,
            "byte_count": total_bytes,
            "namespace_count": ns_count,
        }))
    }
}
