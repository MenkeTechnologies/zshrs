// zsync — overlay-to-canonical promotion + opt-in mid-session refresh.
//
// Per docs/DAEMON.md "Promoting client-local changes to daemon canonical":
//   - push_canonical: client overlay → daemon canonical for future shells.
//   - pull_canonical: explicit mid-session refresh.
//   - diff_canonical: overlay-vs-canonical diff.
//   - canonical_changed event: pushed to subscribers after a successful push.
//
// V1 stores canonical state as JSON in a `canonical` table inside catalog.db
// keyed by (subsystem, key). One subsystem per row family (path, fpath, alias,
// named_dir, env, params, zstyle, bindkey, setopt, zmodload, function, compdef).
// Future iteration replaces JSON with rkyv-archived per-subsystem images served
// to clients via mmap; v1 returns JSON over IPC for simplicity.

use std::sync::Arc;

use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;
use super::Result;

/// Bring up the `canonical` table in catalog.db. Idempotent.
pub fn ensure_schema(state: &DaemonState) -> Result<()> {
    state.with_catalog(|conn| -> std::result::Result<(), super::DaemonError> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS canonical (
                subsystem    TEXT NOT NULL,
                key          TEXT NOT NULL,
                value        TEXT NOT NULL,
                set_at_ns    INTEGER NOT NULL,
                set_by_shell INTEGER,
                PRIMARY KEY (subsystem, key)
            );
            CREATE INDEX IF NOT EXISTS canonical_subsystem_idx ON canonical(subsystem);
            "#,
        )?;
        Ok(())
    })
}

/// Recognized subsystems. Anything outside this list is rejected.
const VALID_SUBSYSTEMS: &[&str] = &[
    "path",
    "fpath",
    "manpath",
    "named_dir",
    "alias",
    "galias",
    "salias",
    "function",
    "compdef",
    "env",
    "params",
    "zstyle",
    "bindkey",
    "setopt",
    "zmodload",
];

fn validate_subsystem(s: &str) -> std::result::Result<(), ErrPayload> {
    if VALID_SUBSYSTEMS.contains(&s) {
        Ok(())
    } else {
        Err(ErrPayload::new(
            "bad_subsystem",
            format!(
                "subsystem `{}` not recognized; valid: {}",
                s,
                VALID_SUBSYSTEMS.join(", ")
            ),
        ))
    }
}

fn now_ns_i64() -> i64 {
    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
}

// ---- IPC op handlers ----

pub async fn op_push_canonical(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let subsystem = args
        .get("subsystem")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `subsystem`"))?
        .to_string();
    validate_subsystem(&subsystem)?;

    let value = args
        .get("value")
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `value`"))?;

    let entries = serialize_pushed_value(value)?;
    if entries.is_empty() {
        return Err(ErrPayload::new("bad_value", "empty `value`"));
    }

    // Mutate in-memory rkyv-backed canonical state. SQLite is not touched
    // here — `zcache hydrate-view` (or implicit calls) refresh it lazily.
    for (key, json_val) in &entries {
        state
            .canonical
            .upsert(&subsystem, key, json_val, Some(client_id));
    }

    // Persist to rkyv shard atomically (single write per push, all subsystems
    // serialized into the user-overlay promotion shard).
    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    if let Err(e) = state.canonical.persist(generation) {
        tracing::warn!(?e, "canonical: persist to rkyv failed");
    }

    let count = state.canonical.rows_for(&subsystem).len() as i64;
    let now = now_ns_i64();

    // Emit canonical_changed event to every subscriber.
    let event_payload = json!({
        "subsystem": subsystem,
        "row_count": count,
        "set_at_ns": now,
        "set_by_shell": client_id,
    });
    let frame = super::ipc::Frame::event("canonical_changed", event_payload);
    state.broadcast(frame, &[]);

    Ok(json!({
        "promoted": entries.len(),
        "subsystem": subsystem,
        "row_count": count,
    }))
}

pub async fn op_pull_canonical(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let subsystem = args
        .get("subsystem")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `subsystem`"))?
        .to_string();
    validate_subsystem(&subsystem)?;

    let rows = state.canonical.rows_for(&subsystem);
    Ok(json!({
        "subsystem": subsystem,
        "rows": rows.iter().map(|r| json!({
            "key": r.key,
            "value": r.value,
            "set_at_ns": r.set_at_ns,
            "set_by_shell": r.set_by_shell,
        })).collect::<Vec<_>>(),
    }))
}

pub async fn op_diff_canonical(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let subsystem = args
        .get("subsystem")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `subsystem`"))?
        .to_string();
    validate_subsystem(&subsystem)?;

    let overlay = args.get("overlay").cloned().unwrap_or(Value::Null);
    let overlay_entries = serialize_pushed_value(&overlay).unwrap_or_default();

    let canonical_rows = state.canonical.rows_for(&subsystem);
    let canonical_map: std::collections::HashMap<&str, &str> = canonical_rows
        .iter()
        .map(|r| (r.key.as_str(), r.value.as_str()))
        .collect();

    let mut only_overlay: Vec<(String, String)> = Vec::new();
    let mut conflicts: Vec<(String, String, String)> = Vec::new();
    for (k, v) in &overlay_entries {
        match canonical_map.get(k.as_str()) {
            None => only_overlay.push((k.clone(), v.clone())),
            Some(can) if *can != v.as_str() => {
                conflicts.push((k.clone(), v.clone(), can.to_string()))
            }
            Some(_) => {}
        }
    }
    let only_canonical: Vec<(String, String)> = canonical_rows
        .iter()
        .filter(|r| !overlay_entries.iter().any(|(k, _)| k == &r.key))
        .map(|r| (r.key.clone(), r.value.clone()))
        .collect();

    Ok(json!({
        "subsystem": subsystem,
        "only_overlay": only_overlay,
        "only_canonical": only_canonical,
        "conflicts": conflicts.into_iter().map(|(k,o,c)| json!({"key":k,"overlay":o,"canonical":c})).collect::<Vec<_>>(),
    }))
}

/// Legacy SQL row shape kept for callers in export.rs (which still has its
/// own struct of the same name). Reads always resolve from the rkyv-backed
/// in-memory state via `state.canonical.rows_for(...)`.
#[derive(serde::Serialize, Debug)]
pub struct CanonicalRow {
    pub key: String,
    pub value: String,
    pub set_at_ns: i64,
    pub set_by_shell: Option<i64>,
}

/// Compatibility shim: returns the in-memory state in the legacy row shape so
/// existing readers (export.rs's render_*) don't have to change.
pub fn read_canonical_rows_inmem(state: &DaemonState, subsystem: &str) -> Vec<CanonicalRow> {
    state
        .canonical
        .rows_for(subsystem)
        .into_iter()
        .map(|r| CanonicalRow {
            key: r.key,
            value: r.value,
            set_at_ns: r.set_at_ns,
            set_by_shell: r.set_by_shell,
        })
        .collect()
}

/// Convert a pushed value into (key, value-as-json-string) pairs.
fn serialize_pushed_value(value: &Value) -> std::result::Result<Vec<(String, String)>, ErrPayload> {
    match value {
        Value::Object(map) => Ok(map
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect()),
        Value::Array(arr) => Ok(arr
            .iter()
            .enumerate()
            .map(|(i, v)| (i.to_string(), v.to_string()))
            .collect()),
        Value::String(s) => Ok(vec![(String::new(), s.clone())]),
        Value::Null => Ok(Vec::new()),
        _ => Ok(vec![(String::new(), value.to_string())]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh_state() -> (TempDir, Arc<DaemonState>) {
        let tmp = TempDir::new().unwrap();
        let paths = super::super::paths::CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let state = DaemonState::new(paths).unwrap();
        ensure_schema(&state).unwrap();
        (tmp, state)
    }

    #[tokio::test]
    async fn push_pull_roundtrip() {
        let (_tmp, state) = fresh_state();
        let args = json!({
            "subsystem": "alias",
            "value": { "ll": "ls -la", "gst": "git status" }
        });
        let r = op_push_canonical(&state, 1, args).await.unwrap();
        assert_eq!(r["promoted"].as_u64(), Some(2));

        let r = op_pull_canonical(&state, json!({ "subsystem": "alias" }))
            .await
            .unwrap();
        let rows = r["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn push_array_indexed() {
        let (_tmp, state) = fresh_state();
        let args = json!({
            "subsystem": "path",
            "value": ["/usr/local/bin", "/usr/bin", "/bin"]
        });
        let r = op_push_canonical(&state, 1, args).await.unwrap();
        assert_eq!(r["promoted"].as_u64(), Some(3));
    }

    #[tokio::test]
    async fn push_rejects_unknown_subsystem() {
        let (_tmp, state) = fresh_state();
        let r = op_push_canonical(
            &state,
            1,
            json!({ "subsystem": "definitely_not_real", "value": {"x": "y"} }),
        )
        .await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn diff_reports_conflicts_and_uniques() {
        let (_tmp, state) = fresh_state();
        op_push_canonical(
            &state,
            1,
            json!({ "subsystem": "alias", "value": { "ll": "ls -la", "gst": "git status" } }),
        )
        .await
        .unwrap();

        let args = json!({
            "subsystem": "alias",
            "overlay": { "ll": "ls -la", "gst": "git stash", "newish": "echo new" }
        });
        let r = op_diff_canonical(&state, args).await.unwrap();

        let only_overlay = r["only_overlay"].as_array().unwrap();
        assert_eq!(only_overlay.len(), 1); // newish

        let conflicts = r["conflicts"].as_array().unwrap();
        assert_eq!(conflicts.len(), 1); // gst differs
        let conflict = &conflicts[0];
        assert_eq!(conflict["key"], "gst");
    }
}
