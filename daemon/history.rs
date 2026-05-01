// history.db — daemon-owned shared shell history with FTS5.
//
// Per docs/DAEMON.md "Daemon = sole writer" + "what gets logged at INFO level":
//   - Every shell sends history_append IPC per-command (line, ts_ns, exit_code, cwd,
//     duration_ns, sessid). Daemon serializes writes, no .zsh_history flat-file race.
//   - history_query supports FTS match, time-range, cwd filter, limit. Powers Ctrl-R
//     and the existing `history`/`fc` builtins (which can route to daemon when the
//     daemon-shared-history option is on).
//
// Schema deliberately keeps `line` plain-TEXT in the main table; the FTS5 virtual
// table mirrors it via triggers. This lets us preserve full row metadata (ts, cwd,
// exit_code) on the canonical row and search efficiently.

use std::path::Path;

use rusqlite::Connection;
use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::paths::CachePaths;
use super::state::DaemonState;
use super::Result;

pub const SCHEMA_VERSION: i64 = 1;

/// Open (or create) history.db, set WAL + FTS5 + triggers, ensure schema.
pub fn open(paths: &CachePaths) -> Result<Connection> {
    open_at(&paths.history_db)
}

pub fn open_at(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    super::paths::ensure_file_600(path)?;

    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;

    let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if v == 0 {
        create_schema(&conn)?;
        conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    } else if v != SCHEMA_VERSION {
        return Err(super::DaemonError::other(format!(
            "history.db schema version {} != expected {}",
            v, SCHEMA_VERSION
        )));
    }

    Ok(conn)
}

fn create_schema(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS history (
            id          INTEGER PRIMARY KEY,
            line        TEXT NOT NULL,
            ts_ns       INTEGER NOT NULL,
            exit_code   INTEGER,
            cwd         TEXT,
            duration_ns INTEGER,
            sessid      TEXT,
            hostname    TEXT,
            shell_id    INTEGER
        );

        CREATE INDEX IF NOT EXISTS history_ts_idx       ON history(ts_ns DESC);
        CREATE INDEX IF NOT EXISTS history_sessid_idx   ON history(sessid);
        CREATE INDEX IF NOT EXISTS history_shell_id_idx ON history(shell_id);

        CREATE VIRTUAL TABLE IF NOT EXISTS history_fts
            USING fts5(line, content='history', content_rowid='id');

        CREATE TRIGGER IF NOT EXISTS history_ai AFTER INSERT ON history BEGIN
            INSERT INTO history_fts(rowid, line) VALUES (new.id, new.line);
        END;
        CREATE TRIGGER IF NOT EXISTS history_ad AFTER DELETE ON history BEGIN
            INSERT INTO history_fts(history_fts, rowid, line) VALUES ('delete', old.id, old.line);
        END;
        CREATE TRIGGER IF NOT EXISTS history_au AFTER UPDATE ON history BEGIN
            INSERT INTO history_fts(history_fts, rowid, line) VALUES ('delete', old.id, old.line);
            INSERT INTO history_fts(rowid, line) VALUES (new.id, new.line);
        END;
        "#,
    )
}

/// One history row (returned by `history_query`).
#[derive(serde::Serialize, Debug, Clone)]
pub struct HistoryRow {
    pub id: i64,
    pub line: String,
    pub ts_ns: i64,
    pub exit_code: Option<i64>,
    pub cwd: Option<String>,
    pub duration_ns: Option<i64>,
    pub sessid: Option<String>,
    pub hostname: Option<String>,
    pub shell_id: Option<i64>,
}

pub fn append(
    conn: &Connection,
    line: &str,
    ts_ns: i64,
    exit_code: Option<i64>,
    cwd: Option<&str>,
    duration_ns: Option<i64>,
    sessid: Option<&str>,
    hostname: Option<&str>,
    shell_id: Option<i64>,
) -> rusqlite::Result<i64> {
    conn.execute(
        r#"INSERT INTO history (line, ts_ns, exit_code, cwd, duration_ns, sessid, hostname, shell_id)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?)"#,
        rusqlite::params![line, ts_ns, exit_code, cwd, duration_ns, sessid, hostname, shell_id],
    )?;
    Ok(conn.last_insert_rowid())
}

pub fn count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))
}

/// Query the history. Modes:
///  - `match`:    FTS5 match expression (`MATCH ?`)
///  - `like`:     LIKE pattern (`%...%`)
///  - `prefix`:   prefix LIKE (`%...`)
///  - none of the above: raw recent listing
pub fn query(
    conn: &Connection,
    filter: Option<&str>,
    mode: &str,
    cwd: Option<&str>,
    after_ns: Option<i64>,
    before_ns: Option<i64>,
    limit: i64,
    descending: bool,
) -> rusqlite::Result<Vec<HistoryRow>> {
    let mut sql = String::from(
        "SELECT h.id, h.line, h.ts_ns, h.exit_code, h.cwd, h.duration_ns, h.sessid, h.hostname, h.shell_id \
         FROM history h",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    let mut conds: Vec<String> = Vec::new();

    if let Some(f) = filter {
        match mode {
            "match" | "fts" => {
                sql.push_str(" JOIN history_fts ON h.id = history_fts.rowid");
                conds.push("history_fts.line MATCH ?".to_string());
                params.push(Box::new(f.to_string()));
            }
            "like" => {
                conds.push("h.line LIKE ?".to_string());
                params.push(Box::new(format!("%{}%", f)));
            }
            "prefix" => {
                conds.push("h.line LIKE ?".to_string());
                params.push(Box::new(format!("{}%", f)));
            }
            _ => {
                conds.push("h.line = ?".to_string());
                params.push(Box::new(f.to_string()));
            }
        }
    }

    if let Some(c) = cwd {
        conds.push("h.cwd = ?".to_string());
        params.push(Box::new(c.to_string()));
    }
    if let Some(a) = after_ns {
        conds.push("h.ts_ns >= ?".to_string());
        params.push(Box::new(a));
    }
    if let Some(b) = before_ns {
        conds.push("h.ts_ns <= ?".to_string());
        params.push(Box::new(b));
    }

    if !conds.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conds.join(" AND "));
    }

    sql.push_str(if descending {
        " ORDER BY h.ts_ns DESC, h.id DESC"
    } else {
        " ORDER BY h.ts_ns ASC, h.id ASC"
    });
    sql.push_str(" LIMIT ?");
    params.push(Box::new(limit));

    let mut stmt = conn.prepare(&sql)?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt
        .query_map(&param_refs[..], |r| {
            Ok(HistoryRow {
                id: r.get(0)?,
                line: r.get(1)?,
                ts_ns: r.get(2)?,
                exit_code: r.get(3)?,
                cwd: r.get(4)?,
                duration_ns: r.get(5)?,
                sessid: r.get(6)?,
                hostname: r.get(7)?,
                shell_id: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    Ok(rows)
}

// ---- IPC op handlers ----

pub async fn op_history_append(state: &std::sync::Arc<DaemonState>, args: Value) -> OpResult {
    let line = args
        .get("line")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `line`"))?
        .to_string();
    let ts_ns = args
        .get("ts_ns")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let exit_code = args.get("exit_code").and_then(Value::as_i64);
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    let duration_ns = args.get("duration_ns").and_then(Value::as_i64);
    let sessid = args
        .get("sessid")
        .and_then(Value::as_str)
        .map(str::to_string);
    let hostname = args
        .get("hostname")
        .and_then(Value::as_str)
        .map(str::to_string);
    let shell_id = args.get("shell_id").and_then(Value::as_i64);

    let id = state.with_history(|conn| {
        append(
            conn,
            &line,
            ts_ns,
            exit_code,
            cwd.as_deref(),
            duration_ns,
            sessid.as_deref(),
            hostname.as_deref(),
            shell_id,
        )
    })?;

    // Long-running command notice: if the command's duration exceeds the
    // configured threshold (default 30s, ZSHRS_LONG_CMD_THRESHOLD seconds),
    // publish a long_cmd_complete (or long_cmd_failed / long_cmd_signaled)
    // event to all of the user's other connected shells. Per docs/DAEMON.md.
    if std::env::var("ZSHRS_LONG_CMD_NOTICES")
        .ok()
        .map(|v| v != "0")
        .unwrap_or(true)
    {
        let threshold_secs: u64 = std::env::var("ZSHRS_LONG_CMD_THRESHOLD")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(30);
        let threshold_ns: i64 = (threshold_secs as i64) * 1_000_000_000;
        if let Some(dur) = duration_ns {
            if dur >= threshold_ns {
                let from_shell = shell_id.unwrap_or(0) as u64;
                let event_kind = if exit_code.map(|c| c < 0).unwrap_or(false) {
                    // Per zsh convention exit_code can be negative when the cmd died via signal
                    // (some shells encode -SIG; ours stores 128+sig in exit_code). We treat
                    // any negative as signaled.
                    "long_cmd_signaled"
                } else if exit_code.map(|c| c != 0).unwrap_or(false) {
                    "long_cmd_failed"
                } else {
                    "long_cmd_complete"
                };
                // NOTE: don't put "event" inside the payload — Frame::event
                // already adds an outer "event" field via flatten, and a
                // duplicate key on the wire breaks untagged-Frame deserialize
                // for streaming consumers (zsubscribe).
                let payload = json!({
                    "from_shell": from_shell,
                    "command": line,
                    "exit_code": exit_code,
                    "duration_ns": dur,
                    "cwd": cwd,
                    "ts_ns": ts_ns,
                });
                let frame = super::ipc::Frame::event(event_kind, payload);
                // Broadcast to every other shell (the originator already knows
                // the command finished — it just got control back).
                state.broadcast(frame, &[from_shell]);
            }
        }
    }

    // Payload key is `history_id` (not `id`) to avoid collision with the Frame::Response
    // envelope's `id` field (untagged-enum dispatch breaks on duplicate keys).
    Ok(json!({ "history_id": id }))
}

pub async fn op_history_query(state: &std::sync::Arc<DaemonState>, args: Value) -> OpResult {
    let filter = args
        .get("filter")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mode = args
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("match")
        .to_string();
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    let after_ns = args.get("after_ns").and_then(Value::as_i64);
    let before_ns = args.get("before_ns").and_then(Value::as_i64);
    let limit = args
        .get("limit")
        .and_then(Value::as_i64)
        .unwrap_or(100)
        .clamp(1, 100_000);
    let descending = args
        .get("descending")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    let rows = state.with_history(|conn| {
        query(
            conn,
            filter.as_deref(),
            &mode,
            cwd.as_deref(),
            after_ns,
            before_ns,
            limit,
            descending,
        )
    })?;
    let total = rows.len();

    Ok(json!({ "rows": rows, "count": total }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh() -> (TempDir, CachePaths, Connection) {
        let tmp = TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let conn = open(&paths).unwrap();
        (tmp, paths, conn)
    }

    #[test]
    fn schema_at_v1_with_fts() {
        let (_tmp, _p, conn) = fresh();
        let v: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);

        // FTS5 virtual table exists.
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='history_fts'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn append_and_count() {
        let (_tmp, _p, conn) = fresh();
        let id1 = append(
            &conn,
            "ls -la",
            100,
            Some(0),
            Some("/tmp"),
            Some(2_000_000),
            None,
            None,
            None,
        )
        .unwrap();
        let id2 = append(
            &conn,
            "git status",
            200,
            Some(0),
            Some("/tmp/repo"),
            Some(50_000_000),
            None,
            None,
            None,
        )
        .unwrap();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(count(&conn).unwrap(), 2);
    }

    #[test]
    fn query_recent() {
        let (_tmp, _p, conn) = fresh();
        for i in 1..=10 {
            append(
                &conn,
                &format!("echo {}", i),
                i,
                Some(0),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        }
        let rows = query(&conn, None, "match", None, None, None, 5, true).unwrap();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].line, "echo 10");
        assert_eq!(rows[4].line, "echo 6");
    }

    #[test]
    fn query_fts_match() {
        let (_tmp, _p, conn) = fresh();
        append(&conn, "git status", 1, None, None, None, None, None, None).unwrap();
        append(&conn, "cargo build", 2, None, None, None, None, None, None).unwrap();
        append(
            &conn,
            "git push origin main",
            3,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        append(&conn, "ls -la", 4, None, None, None, None, None, None).unwrap();

        let rows = query(&conn, Some("git"), "match", None, None, None, 100, true).unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|r| r.line == "git status"));
        assert!(rows.iter().any(|r| r.line == "git push origin main"));
    }

    #[test]
    fn query_like_match() {
        let (_tmp, _p, conn) = fresh();
        append(
            &conn,
            "echo cargo build && cargo test",
            1,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        append(&conn, "rm /tmp/foo", 2, None, None, None, None, None, None).unwrap();

        let rows = query(&conn, Some("cargo"), "like", None, None, None, 100, true).unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn query_time_range() {
        let (_tmp, _p, conn) = fresh();
        for i in 1..=10 {
            append(
                &conn,
                &format!("cmd {}", i),
                i * 100,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        }
        let rows = query(&conn, None, "match", None, Some(300), Some(700), 100, false).unwrap();
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r.ts_ns >= 300 && r.ts_ns <= 700));
    }

    #[test]
    fn query_cwd_filter() {
        let (_tmp, _p, conn) = fresh();
        append(&conn, "ls", 1, None, Some("/a"), None, None, None, None).unwrap();
        append(&conn, "pwd", 2, None, Some("/b"), None, None, None, None).unwrap();
        append(&conn, "cd /a", 3, None, Some("/a"), None, None, None, None).unwrap();
        let rows = query(&conn, None, "match", Some("/a"), None, None, 100, true).unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn delete_keeps_fts_in_sync() {
        let (_tmp, _p, conn) = fresh();
        append(&conn, "git status", 1, None, None, None, None, None, None).unwrap();
        append(&conn, "git push", 2, None, None, None, None, None, None).unwrap();

        // Delete first row; FTS trigger should remove it from the index.
        conn.execute("DELETE FROM history WHERE id = 1", [])
            .unwrap();
        let rows = query(&conn, Some("git"), "match", None, None, None, 100, true).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].line, "git push");
    }
}
