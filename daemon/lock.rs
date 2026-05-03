//! `daemon.lock.*` ops — named cross-process mutual exclusion.
//!
//! Per docs/DAEMON_AS_SERVICE.md: "named cross-process locks with
//! PID-tied auto-release". Replaces per-script `flock(1)` with a
//! daemon-mediated registry that survives client disconnects but
//! cleans up when the holder process dies.
//!
//! Design:
//!   - Lock state lives in memory on `DaemonState::locks` (parking_lot
//!     Mutex over a HashMap). No on-disk persistence — a daemon
//!     restart releases every lock, which is the right semantics:
//!     locks held by processes that didn't get a release call were
//!     by definition crashed.
//!   - Holder identity is `(pid, token)` where `token` is a random
//!     u128 minted at acquire time. Release requires the matching
//!     token, so client A can't release client B's lock by name.
//!   - `lock_acquire` blocks (timeout-bounded) until the lock is free
//!     OR until the current holder's PID is found dead — in which
//!     case the lock is force-released and re-acquired.
//!   - `lock_try_acquire` is non-blocking: returns `busy` if held.
//!   - `lock_list` enumerates current holders for monitoring / stuck-
//!     lock diagnosis.
//!
//! Op surface:
//!
//! | Op                  | Args                    | Returns                       |
//! |---------------------|-------------------------|-------------------------------|
//! | `lock_acquire`      | `{name, timeout_secs?}` | `{ok, token, name}` or 408    |
//! | `lock_try_acquire`  | `{name}`                | `{ok, token, name}` or 409    |
//! | `lock_release`      | `{name, token}`         | `{ok, released: bool}`        |
//! | `lock_list`         | `{}`                    | `{ok, locks: [...], count}`   |

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

#[derive(Debug, Clone)]
pub struct LockEntry {
    pub holder_pid: i32,
    pub token: u128,
    pub acquired_at_ns: i64,
}

pub type LockTable = Mutex<HashMap<String, LockEntry>>;

/// Build the in-memory lock table. Called once from `DaemonState::new`.
pub fn new_table() -> LockTable {
    Mutex::new(HashMap::new())
}

fn name_arg(args: &Value) -> std::result::Result<String, ErrPayload> {
    args.get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `name`"))
}

fn caller_pid(args: &Value) -> i32 {
    // Client provides its PID so the daemon can probe for liveness on
    // crashed-holder force-release. Fall back to 0 (meaning "anonymous
    // / unknown") if not supplied.
    args.get("pid")
        .and_then(Value::as_i64)
        .map(|n| n as i32)
        .unwrap_or(0)
}

/// True if `pid` is a live process. SIGNAL 0 doesn't actually deliver
/// anything; it just probes liveness via the kernel's permission check.
fn pid_alive(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }
    // SAFETY: kill(pid, 0) is read-only — kernel returns 0 when the
    // pid exists and we have permission, -1/ESRCH when dead, -1/EPERM
    // when alive but owned by another user. Read errno via
    // std::io::Error::last_os_error() so we stay portable across
    // macOS (libc::__error) and Linux (libc::__errno_location).
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    matches!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EPERM)
    )
}

fn now_ns() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

fn try_acquire_inner(
    table: &LockTable,
    name: &str,
    pid: i32,
) -> Option<u128> {
    let mut g = table.lock();
    // If held by a dead PID, force-release.
    if let Some(existing) = g.get(name) {
        if !pid_alive(existing.holder_pid) {
            tracing::info!(
                lock = name,
                stale_pid = existing.holder_pid,
                "lock force-released (holder dead)"
            );
            g.remove(name);
        }
    }
    if g.contains_key(name) {
        return None;
    }
    let token = rand::random::<u128>();
    g.insert(
        name.to_string(),
        LockEntry {
            holder_pid: pid,
            token,
            acquired_at_ns: now_ns(),
        },
    );
    Some(token)
}

pub async fn op_lock_try_acquire(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let name = name_arg(&args)?;
    let pid = caller_pid(&args);
    match try_acquire_inner(&state.locks, &name, pid) {
        Some(token) => Ok(json!({
            "name": name,
            "token": token.to_string(),
            "holder_pid": pid,
        })),
        None => Err(ErrPayload::new(
            "busy",
            format!("lock `{name}` is held"),
        )),
    }
}

pub async fn op_lock_acquire(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let name = name_arg(&args)?;
    let pid = caller_pid(&args);
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let deadline = if timeout_secs > 0.0 {
        Some(Instant::now() + Duration::from_secs_f64(timeout_secs))
    } else {
        None
    };

    // Poll loop. tokio sleep so the runtime stays cooperative.
    const POLL: Duration = Duration::from_millis(50);
    loop {
        if let Some(token) = try_acquire_inner(&state.locks, &name, pid) {
            return Ok(json!({
                "name": name,
                "token": token.to_string(),
                "holder_pid": pid,
            }));
        }
        match deadline {
            Some(dl) if Instant::now() >= dl => {
                return Err(ErrPayload::new(
                    "timeout",
                    format!("lock `{name}` not acquired within {timeout_secs}s"),
                ));
            }
            None => {
                // No timeout requested = single attempt, then `busy`.
                return Err(ErrPayload::new(
                    "busy",
                    format!("lock `{name}` is held (pass `timeout_secs` to wait)"),
                ));
            }
            _ => tokio::time::sleep(POLL).await,
        }
    }
}

pub async fn op_lock_release(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let name = name_arg(&args)?;
    let token_str = args
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `token`"))?;
    let token: u128 = token_str
        .parse()
        .map_err(|_| ErrPayload::new("bad_args", "token must be a u128 string"))?;

    let mut g = state.locks.lock();
    let released = match g.get(&name) {
        Some(entry) if entry.token == token => {
            g.remove(&name);
            true
        }
        Some(_) => {
            return Err(ErrPayload::new(
                "wrong_token",
                format!("lock `{name}` token mismatch — refusing to release"),
            ));
        }
        None => false,
    };
    Ok(json!({ "name": name, "released": released }))
}

pub async fn op_lock_list(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    let now = now_ns();
    let g = state.locks.lock();
    let mut entries: Vec<Value> = g
        .iter()
        .map(|(name, e)| {
            json!({
                "name": name,
                "holder_pid": e.holder_pid,
                "alive": pid_alive(e.holder_pid),
                "acquired_at_ns": e.acquired_at_ns,
                "age_ns": now - e.acquired_at_ns,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .cmp(b["name"].as_str().unwrap_or(""))
    });
    let count = entries.len();
    Ok(json!({ "locks": entries, "count": count }))
}
