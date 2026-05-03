//! `daemon.schedule.*` ops — cron-equivalent recurring + one-shot jobs.
//!
//! Per docs/DAEMON_AS_SERVICE.md: replaces cron / anacron / systemd-user-
//! timers / launchd for personal jobs. State persists across daemon
//! restarts (sqlite-backed); a single tokio task drives the tick loop.
//!
//! State table (sibling to cache.db at `~/.zshrs/cache.db`,
//! shared file but separate table — same SQLite handle for free):
//!
//! ```sql
//! CREATE TABLE schedule (
//!     id            INTEGER PRIMARY KEY AUTOINCREMENT,
//!     cron_expr     TEXT,                -- 6-field "sec min hr dom mon dow" or NULL for one-shot
//!     fire_at_ns    INTEGER,             -- epoch-ns for one-shot (NULL for recurring)
//!     argv_json     TEXT NOT NULL,       -- JSON array of strings
//!     env_json      TEXT,                -- JSON object {ENV: val} or NULL
//!     cwd           TEXT,
//!     enabled       INTEGER NOT NULL DEFAULT 1,
//!     last_run_ns   INTEGER,
//!     last_job_id   INTEGER,
//!     created_at    INTEGER NOT NULL,
//!     notes         TEXT
//! );
//! ```
//!
//! Op surface:
//!
//! | Op                  | Args                                  | Returns                     |
//! |---------------------|---------------------------------------|-----------------------------|
//! | `schedule_add`      | `{cron_expr, command[], cwd?, env?, notes?}` | `{ok, schedule_id}`  |
//! | `schedule_add_once` | `{fire_at_unix_secs, command[], ...}` | `{ok, schedule_id}`         |
//! | `schedule_remove`   | `{id}`                                | `{ok, removed: bool}`       |
//! | `schedule_list`     | `{enabled_only?}`                     | `{ok, schedules: [...]}`    |
//!
//! Cron format is the standard 6-field "sec min hr dom mon dow"
//! (the `cron` crate uses 6 fields including seconds — different from
//! traditional 5-field crontab. Examples:
//!   `0 0 * * * *`        every hour on the hour
//!   `0 */15 * * * *`     every 15 minutes
//!   `0 0 3 * * *`        daily at 03:00:00
//!   `0 0 0 * * Mon-Fri`  weekdays at midnight
//!
//! Tick driver: a single tokio task wakes every second, reads all
//! enabled rows, asks `cron::Schedule::upcoming` whether any fired
//! since `last_run_ns`, and dispatches `job_submit` for each. The
//! one-second granularity is intentional (matches systemd-user-timers
//! default precision) and trades CPU cost for accuracy on jobs that
//! must fire at a specific second.

use std::str::FromStr;
use std::sync::Arc;

use rusqlite::{params, Connection};
use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

const TICK_SECS: u64 = 1;

fn open_db(state: &DaemonState) -> std::result::Result<Connection, ErrPayload> {
    // Reuse the cache.db file — schedule rows live in their own table
    // alongside the cache `kv` table. Single SQLite file keeps the
    // user-visible filesystem footprint small and lets us reuse the
    // PRAGMA journal_mode=WAL setup.
    let path = &state.paths.cache_db;
    let conn = Connection::open(path)
        .map_err(|e| ErrPayload::new("schedule_open", e.to_string()))?;
    conn.execute_batch(
        r#"
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous  = NORMAL;
        CREATE TABLE IF NOT EXISTS schedule (
            id           INTEGER PRIMARY KEY AUTOINCREMENT,
            cron_expr    TEXT,
            fire_at_ns   INTEGER,
            argv_json    TEXT NOT NULL,
            env_json     TEXT,
            cwd          TEXT,
            enabled      INTEGER NOT NULL DEFAULT 1,
            last_run_ns  INTEGER,
            last_job_id  INTEGER,
            created_at   INTEGER NOT NULL,
            notes        TEXT
        );
        CREATE INDEX IF NOT EXISTS schedule_enabled ON schedule(enabled);
        "#,
    )
    .map_err(|e| ErrPayload::new("schedule_schema", e.to_string()))?;
    Ok(conn)
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

fn argv_arg(args: &Value) -> std::result::Result<Vec<String>, ErrPayload> {
    let arr = args
        .get("command")
        .and_then(Value::as_array)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `command` array"))?;
    let v: Vec<String> = arr
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if v.is_empty() {
        return Err(ErrPayload::new("bad_args", "`command` is empty"));
    }
    Ok(v)
}

pub async fn op_schedule_add(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let cron_expr = args
        .get("cron_expr")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `cron_expr`"))?
        .to_string();
    // Validate the cron expression at add-time so the user gets immediate
    // feedback instead of a silent no-fire later.
    cron::Schedule::from_str(&cron_expr)
        .map_err(|e| ErrPayload::new("bad_cron", format!("invalid cron `{cron_expr}`: {e}")))?;
    let command = argv_arg(&args)?;
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    let env = args.get("env").map(|v| v.to_string());
    let notes = args.get("notes").and_then(Value::as_str).map(str::to_string);
    let argv_json = serde_json::to_string(&command).unwrap();
    let now = now_ns();

    let conn = open_db(state)?;
    conn.execute(
        r#"
        INSERT INTO schedule (cron_expr, fire_at_ns, argv_json, env_json, cwd, enabled, created_at, notes)
        VALUES (?1, NULL, ?2, ?3, ?4, 1, ?5, ?6)
        "#,
        params![cron_expr, argv_json, env, cwd, now, notes],
    )
    .map_err(|e| ErrPayload::new("schedule_insert", e.to_string()))?;
    let id = conn.last_insert_rowid();
    Ok(json!({ "schedule_id": id, "cron_expr": cron_expr }))
}

pub async fn op_schedule_add_once(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let fire_at_unix_secs = args
        .get("fire_at_unix_secs")
        .and_then(Value::as_i64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `fire_at_unix_secs`"))?;
    let fire_at_ns = fire_at_unix_secs.saturating_mul(1_000_000_000);
    if fire_at_ns < now_ns() {
        return Err(ErrPayload::new(
            "bad_args",
            "fire_at_unix_secs is in the past",
        ));
    }
    let command = argv_arg(&args)?;
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    let env = args.get("env").map(|v| v.to_string());
    let notes = args.get("notes").and_then(Value::as_str).map(str::to_string);
    let argv_json = serde_json::to_string(&command).unwrap();
    let now = now_ns();

    let conn = open_db(state)?;
    conn.execute(
        r#"
        INSERT INTO schedule (cron_expr, fire_at_ns, argv_json, env_json, cwd, enabled, created_at, notes)
        VALUES (NULL, ?1, ?2, ?3, ?4, 1, ?5, ?6)
        "#,
        params![fire_at_ns, argv_json, env, cwd, now, notes],
    )
    .map_err(|e| ErrPayload::new("schedule_insert", e.to_string()))?;
    let id = conn.last_insert_rowid();
    Ok(json!({ "schedule_id": id, "fire_at_ns": fire_at_ns }))
}

pub async fn op_schedule_remove(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_i64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    let conn = open_db(state)?;
    let n = conn
        .execute("DELETE FROM schedule WHERE id = ?1", params![id])
        .map_err(|e| ErrPayload::new("schedule_remove", e.to_string()))?;
    Ok(json!({ "id": id, "removed": n > 0 }))
}

pub async fn op_schedule_list(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let enabled_only = args
        .get("enabled_only")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let conn = open_db(state)?;
    let sql = if enabled_only {
        "SELECT id, cron_expr, fire_at_ns, argv_json, env_json, cwd, enabled, last_run_ns, last_job_id, created_at, notes FROM schedule WHERE enabled = 1 ORDER BY id"
    } else {
        "SELECT id, cron_expr, fire_at_ns, argv_json, env_json, cwd, enabled, last_run_ns, last_job_id, created_at, notes FROM schedule ORDER BY id"
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| ErrPayload::new("schedule_list", e.to_string()))?;
    let rows = stmt
        .query_map([], |r| {
            let argv_json: String = r.get(3)?;
            let argv: Vec<String> = serde_json::from_str(&argv_json).unwrap_or_default();
            Ok(json!({
                "id": r.get::<_, i64>(0)?,
                "cron_expr": r.get::<_, Option<String>>(1)?,
                "fire_at_ns": r.get::<_, Option<i64>>(2)?,
                "command": argv,
                "env_json": r.get::<_, Option<String>>(4)?,
                "cwd": r.get::<_, Option<String>>(5)?,
                "enabled": r.get::<_, i64>(6)? != 0,
                "last_run_ns": r.get::<_, Option<i64>>(7)?,
                "last_job_id": r.get::<_, Option<i64>>(8)?,
                "created_at": r.get::<_, i64>(9)?,
                "notes": r.get::<_, Option<String>>(10)?,
            }))
        })
        .map_err(|e| ErrPayload::new("schedule_list", e.to_string()))?;
    let entries: Vec<Value> = rows.flatten().collect();
    let count = entries.len();
    Ok(json!({ "schedules": entries, "count": count }))
}

/// Spawn the scheduler tick task. One per daemon. Wakes every
/// `TICK_SECS`, reads enabled rows, dispatches `job_submit` for each
/// row that should have fired since its `last_run_ns`. Survives daemon
/// restarts via the persisted state.
pub fn spawn_tick(state: Arc<DaemonState>) {
    tokio::spawn(async move {
        // A small startup delay so the daemon's other init
        // (catalog, fs_watcher, http) finishes before we start
        // dispatching jobs.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(TICK_SECS));
        // Don't backlog ticks if the daemon was paused.
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            if let Err(e) = tick_once(&state).await {
                tracing::warn!(?e, "schedule tick failed");
            }
        }
    });
}

async fn tick_once(state: &Arc<DaemonState>) -> std::result::Result<(), String> {
    let now = now_ns();
    let conn = open_db(state).map_err(|e| e.msg)?;

    // Read all enabled rows.
    #[derive(Debug)]
    struct Row {
        id: i64,
        cron_expr: Option<String>,
        fire_at_ns: Option<i64>,
        argv: Vec<String>,
        env_json: Option<String>,
        cwd: Option<String>,
        last_run_ns: Option<i64>,
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, cron_expr, fire_at_ns, argv_json, env_json, cwd, last_run_ns \
             FROM schedule WHERE enabled = 1",
        )
        .map_err(|e| e.to_string())?;
    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            let argv_json: String = r.get(3)?;
            let argv: Vec<String> = serde_json::from_str(&argv_json).unwrap_or_default();
            Ok(Row {
                id: r.get(0)?,
                cron_expr: r.get(1)?,
                fire_at_ns: r.get(2)?,
                argv,
                env_json: r.get(4)?,
                cwd: r.get(5)?,
                last_run_ns: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .flatten()
        .collect();

    for row in rows {
        let due = row_due(&row.cron_expr, row.fire_at_ns, row.last_run_ns, now);
        if !due {
            continue;
        }
        // Fire job_submit through the supervisor directly (in-process —
        // no IPC roundtrip).
        let env: std::collections::HashMap<String, String> = row
            .env_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();
        // Schedule-spawned jobs use a distinct synthetic client_id so
        // log/inspection can group them.
        const SCHEDULE_CLIENT_ID: u64 = 0xDEAD_BEEF_CAFE_F00D;
        match state.jobs.submit(
            SCHEDULE_CLIENT_ID,
            row.argv.clone(),
            row.cwd.clone(),
            vec!["scheduled".to_string()],
            env,
        ) {
            Ok(job_id) => {
                tracing::info!(
                    schedule_id = row.id,
                    job_id,
                    argv = ?row.argv,
                    "scheduled job dispatched"
                );
                let _ = conn.execute(
                    "UPDATE schedule SET last_run_ns = ?1, last_job_id = ?2 WHERE id = ?3",
                    params![now, job_id as i64, row.id],
                );
                // One-shot: disable after firing.
                if row.fire_at_ns.is_some() {
                    let _ = conn.execute(
                        "UPDATE schedule SET enabled = 0 WHERE id = ?1",
                        params![row.id],
                    );
                }
            }
            Err(e) => {
                tracing::warn!(
                    schedule_id = row.id,
                    ?e,
                    "scheduled job submit failed"
                );
            }
        }
    }
    Ok(())
}

fn row_due(
    cron_expr: &Option<String>,
    fire_at_ns: Option<i64>,
    last_run_ns: Option<i64>,
    now_ns: i64,
) -> bool {
    if let Some(at) = fire_at_ns {
        return last_run_ns.is_none() && now_ns >= at;
    }
    let Some(expr) = cron_expr else { return false };
    let sched = match cron::Schedule::from_str(expr) {
        Ok(s) => s,
        Err(_) => return false,
    };
    let after = chrono::DateTime::<chrono::Utc>::from_timestamp_nanos(last_run_ns.unwrap_or(0));
    // Walk the next upcoming fire and check if it's <= now. cron's
    // Schedule::after returns an iterator; `next()` is the next fire
    // strictly after `after`.
    if let Some(next) = sched.after(&after).next() {
        let next_ns = next.timestamp_nanos_opt().unwrap_or(i64::MAX);
        return next_ns <= now_ns;
    }
    false
}
