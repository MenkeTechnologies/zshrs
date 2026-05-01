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
    // .zcompdump-derived structured subsystems.
    "service",
    "patcomp",
    "postpatcomp",
    "autoload_completion",
    // Raw zcompdump body (preserved byte-for-byte for round-trip).
    "zcompdump_raw",
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

    // Per docs/DAEMON.md "Daemon-side commit flow on push_canonical":
    //   1. Validate the pushed value (sane format, dirs exist for PATH/FPATH,
    //      no duplicate keys, etc.).
    validate_push_payload(&subsystem, &entries)?;

    // Detect newly-added directories for PATH/FPATH so we can delta-walk +
    // register fsnotify watches in step 3.
    let pre_dirs: Vec<String> = if matches!(subsystem.as_str(), "path" | "fpath" | "manpath") {
        state
            .canonical
            .rows_for(&subsystem)
            .into_iter()
            .map(|r| r.value.trim_matches('"').to_string())
            .collect()
    } else {
        Vec::new()
    };

    //   2. Update canonical state (in-memory + persisted rkyv shard).
    for (key, json_val) in &entries {
        state
            .canonical
            .upsert(&subsystem, key, json_val, Some(client_id));
    }
    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    if let Err(e) = state.canonical.persist(generation) {
        tracing::warn!(?e, "canonical: persist to rkyv failed");
    }

    //   3. Rebuild any derived hashtable that depends on the changed
    //      subsystem (e.g., command hash table for PATH change). Only the
    //      *new* directories get walked — bounded delta-walk per
    //      DAEMON.md:302-304.
    let mut walked_new_dirs: Vec<String> = Vec::new();
    if matches!(subsystem.as_str(), "path" | "fpath") {
        let post_dirs: Vec<String> = entries
            .iter()
            .map(|(_, v)| v.trim_matches('"').to_string())
            .collect();
        for d in &post_dirs {
            if !pre_dirs.contains(d) {
                walked_new_dirs.push(d.clone());
            }
        }
        if !walked_new_dirs.is_empty() {
            // Delta-walk: only the newly-added directories get enumerated.
            // walk::walk_paths handles both PATH-style (executables) and
            // FPATH-style (autoload functions) sources.
            let (path_in, fpath_in): (Vec<String>, Vec<String>) =
                if subsystem == "path" {
                    (walked_new_dirs.clone(), Vec::new())
                } else {
                    (Vec::new(), walked_new_dirs.clone())
                };
            let walk = super::walk::walk_paths(&path_in, &fpath_in);

            // Hydrate catalog with the new entries. We append (no clear) so
            // existing command_hash entries from prior PATH dirs survive.
            let _ = state.with_catalog(|conn| -> rusqlite::Result<()> {
                let tx = conn.unchecked_transaction()?;
                for (name, exe) in &walk.command_hash {
                    tx.execute(
                        "INSERT OR REPLACE INTO entries \
                         (fq_name, plugin_id, kind, image_path, byte_offset, source_loc) \
                         VALUES (?, 'system', 'command', '', 0, ?)",
                        rusqlite::params![format!("cmd:{}", name), exe],
                    )?;
                }
                for (name, file) in &walk.autoload_table {
                    tx.execute(
                        "INSERT OR REPLACE INTO entries \
                         (fq_name, plugin_id, kind, image_path, byte_offset, source_loc) \
                         VALUES (?, 'system', 'autoload', '', 0, ?)",
                        rusqlite::params![format!("fn:{}", name), file],
                    )?;
                }
                tx.commit()?;
                Ok(())
            });

            //   4. Register fsnotify watch on the new directory so future
            //      file additions are picked up automatically (steady-state).
            for d in &walked_new_dirs {
                let dir = std::path::PathBuf::from(d);
                if !dir.is_dir() {
                    continue;
                }
                let kind = if subsystem == "fpath" {
                    super::fsnotify::WatchKind::FpathDir
                } else {
                    super::fsnotify::WatchKind::Generic
                };
                let wp = super::fsnotify::WatchedPath {
                    path: dir.clone(),
                    shard_slug: format!("system-{}", super::shard::hash8(d)),
                    source_root: subsystem.clone(),
                    kind,
                };
                if let Err(e) = state.fs_watcher.watch_path(wp, false) {
                    tracing::warn!(?e, %d, "delta-walk: fsnotify watch failed");
                }
            }

            tracing::info!(
                subsystem = %subsystem,
                new_dirs = walked_new_dirs.len(),
                commands = walk.command_hash.len(),
                autoloads = walk.autoload_table.len(),
                "push_canonical delta-walk complete"
            );
        }
    }

    let count = state.canonical.rows_for(&subsystem).len() as i64;
    let now = now_ns_i64();

    //   5. Emit canonical_changed event to every subscriber.
    let event_payload = json!({
        "subsystem": subsystem,
        "row_count": count,
        "set_at_ns": now,
        "set_by_shell": client_id,
        "delta_walked_dirs": walked_new_dirs,
        "generation": generation,
    });
    let frame = super::ipc::Frame::event("canonical_changed", event_payload);
    state.broadcast(frame, &[]);

    Ok(json!({
        "promoted": entries.len(),
        "subsystem": subsystem,
        "row_count": count,
        "delta_walked_dirs": walked_new_dirs,
        "generation": generation,
    }))
}

/// Per docs/DAEMON.md "Daemon-side commit flow on push_canonical" step 1:
/// "Validate the pushed value (sane format, dirs exist for PATH/FPATH,
/// no duplicate keys, etc.)".
///
/// PATH / FPATH / MANPATH: every value must be a non-empty string and the
/// directory must exist. Duplicate values rejected.
///
/// Other subsystems: keys must not duplicate inside this single push.
fn validate_push_payload(
    subsystem: &str,
    entries: &[(String, String)],
) -> std::result::Result<(), ErrPayload> {
    use std::collections::HashSet;

    let mut seen_keys: HashSet<&str> = HashSet::new();
    for (k, _) in entries {
        if !seen_keys.insert(k.as_str()) {
            return Err(ErrPayload::new(
                "duplicate_key",
                format!("duplicate key `{}` in push for subsystem `{}`", k, subsystem),
            ));
        }
    }

    if matches!(subsystem, "path" | "fpath" | "manpath") {
        let mut seen_dirs: HashSet<String> = HashSet::new();
        for (_, v) in entries {
            let dir = v.trim_matches('"').to_string();
            if dir.is_empty() {
                return Err(ErrPayload::new(
                    "empty_dir",
                    format!("empty directory in `{}` push", subsystem),
                ));
            }
            if !seen_dirs.insert(dir.clone()) {
                return Err(ErrPayload::new(
                    "duplicate_dir",
                    format!("duplicate directory `{}` in `{}` push", dir, subsystem),
                ));
            }
            // PATH must have existing dirs to be useful — fpath/manpath
            // tolerate not-yet-existing entries because they often point
                // at install-on-demand paths.
            if subsystem == "path" {
                let p = std::path::Path::new(&dir);
                if !p.is_dir() {
                    return Err(ErrPayload::new(
                        "no_such_dir",
                        format!("`{}`: not a directory", dir),
                    ));
                }
            }
        }
    }

    Ok(())
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
