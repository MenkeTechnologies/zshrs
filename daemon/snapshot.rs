//! `daemon.snapshot.*` ops — portable canonical-state snapshots.
//!
//! Per docs/DAEMON_AS_SERVICE.md: "save: capture state via the
//! recorder; load: atomic swap; diff: structured per-record diff;
//! bisect: first diverging record".
//!
//! Implementation uses the existing rkyv `CanonicalShard` as the
//! on-disk format — same scheme as the recorder bundle, so snapshots
//! are byte-identical to what `recorder_ingest` produces. Tag-based
//! naming: `~/.cache/zshrs/snapshots/<tag>.rkyv`. Tag is any
//! shell-safe string (matched against `[A-Za-z0-9._-]+` at op time).
//!
//! Op surface (v1 — publish/sign/verify deferred to a follow-up
//! round once the registry transport ships):
//!
//! | Op                | Args              | Returns                                    |
//! |-------------------|-------------------|--------------------------------------------|
//! | `snapshot_save`   | `{tag, notes?}`   | `{ok, tag, path, bytes, generation}`       |
//! | `snapshot_list`   | `{}`              | `{ok, snapshots: [...], count}`            |
//! | `snapshot_load`   | `{tag}`           | `{ok, tag, rows_restored}` (atomic swap)   |
//! | `snapshot_diff`   | `{a, b}`          | `{ok, added, removed, changed}`            |

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

/// All canonical subsystems the snapshot covers. Folded into the
/// rkyv `CanonicalShard` on save and replayed on load.
const SUBSYSTEMS: &[&str] = &[
    "alias", "galias", "salias", "function", "function_autoload",
    "env", "params", "params_typed", "bindkey", "compdef", "named_dir",
    "zstyle", "zmodload", "setopt", "trap", "sched", "source", "zle",
    "completion", "path", "fpath", "manpath",
];

fn tag_arg(args: &Value) -> std::result::Result<String, ErrPayload> {
    let tag = args
        .get("tag")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `tag`"))?;
    if tag.is_empty()
        || !tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(ErrPayload::new(
            "bad_args",
            "tag must match [A-Za-z0-9._-]+",
        ));
    }
    Ok(tag.to_string())
}

fn snapshot_path(state: &DaemonState, tag: &str) -> PathBuf {
    state.paths.snapshots_dir.join(format!("{tag}.rkyv"))
}

pub async fn op_snapshot_save(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag = tag_arg(&args)?;
    let _notes = args.get("notes").and_then(Value::as_str);
    let _ = std::fs::create_dir_all(&state.paths.snapshots_dir);

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let mut shard = state.canonical.snapshot_shard(now);
    // Override slug so the snapshot file lives under our snapshots/ dir
    // (write_canonical_shard composes the filename from slug+source_root,
    // not from a free-form name).
    shard.header.slug = format!("snapshot-{tag}");
    shard.header.source_root = format!("snapshot:{tag}");

    let bytes_before = if let Ok(meta) = std::fs::metadata(snapshot_path(state, &tag)) {
        meta.len()
    } else {
        0
    };

    // write_canonical_shard places the file under paths.images/, not
    // snapshots/. We want snapshots in their own dir for clarity, so
    // serialise + write directly.
    let bytes = rkyv::to_bytes::<_, 4096>(&shard)
        .map_err(|e| ErrPayload::new("snapshot_serialize", e.to_string()))?;
    let dest = snapshot_path(state, &tag);
    let tmp = dest.with_extension("rkyv.tmp");
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .mode(0o600)
        .open(&tmp)
        .map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
    f.write_all(&bytes)
        .map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
    f.sync_all()
        .map_err(|e| ErrPayload::new("snapshot_write", e.to_string()))?;
    drop(f);
    std::fs::rename(&tmp, &dest)
        .map_err(|e| ErrPayload::new("snapshot_rename", e.to_string()))?;

    let total_rows: i64 = SUBSYSTEMS
        .iter()
        .map(|s| state.canonical.rows_for(s).len() as i64)
        .sum();

    Ok(json!({
        "tag": tag,
        "path": dest.display().to_string(),
        "bytes": bytes.len(),
        "bytes_prev": bytes_before,
        "generation": now,
        "total_rows": total_rows,
    }))
}

pub async fn op_snapshot_list(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    let mut entries: Vec<Value> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&state.paths.snapshots_dir) {
        for ent in rd.flatten() {
            let p = ent.path();
            let name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            let tag = match name.strip_suffix(".rkyv") {
                Some(t) => t,
                None => continue,
            };
            let meta = match ent.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            entries.push(json!({
                "tag": tag,
                "path": p.display().to_string(),
                "bytes": meta.len(),
                "modified_secs": meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs()),
            }));
        }
    }
    entries.sort_by(|a, b| {
        a["tag"]
            .as_str()
            .unwrap_or("")
            .cmp(b["tag"].as_str().unwrap_or(""))
    });
    let count = entries.len();
    Ok(json!({ "snapshots": entries, "count": count }))
}

pub async fn op_snapshot_load(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag = tag_arg(&args)?;
    let dest = snapshot_path(state, &tag);
    if !dest.exists() {
        return Err(ErrPayload::new(
            "no_such_file",
            format!("snapshot `{tag}` not found"),
        ));
    }
    let shard = super::shard::read_canonical_shard(&dest)
        .map_err(|e| ErrPayload::new("snapshot_read", e.to_string()))?;

    let mut rows_restored: usize = 0;
    let canon = &state.canonical;
    rows_restored += canon.replace_subsystem(
        "alias",
        shard.aliases.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "galias",
        shard.global_aliases.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "salias",
        shard.suffix_aliases.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "function",
        shard.functions.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "env",
        shard.env_exports.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "params",
        shard.params.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "bindkey",
        shard.bindkeys.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "compdef",
        shard.compdef.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "named_dir",
        shard.named_dirs.iter().map(|(k, v)| (k.clone(), json_str(v))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "zstyle",
        shard
            .zstyle
            .iter()
            .enumerate()
            .map(|(i, (p, r))| (format!("{}:{}", i, p), json_str(r))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "zmodload",
        shard.zmodload.iter().map(|m| (m.clone(), json_str(""))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "setopt",
        shard
            .setopts
            .iter()
            .map(|o| (o.clone(), "\"on\"".to_string()))
            .chain(shard.unsetopts.iter().map(|o| (o.clone(), "\"off\"".to_string()))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "source",
        shard
            .sourced_files
            .iter()
            .enumerate()
            .map(|(i, p)| (i.to_string(), json_str(p))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "path",
        shard
            .path
            .iter()
            .enumerate()
            .map(|(i, d)| (i.to_string(), json_str(d))),
        None,
    );
    rows_restored += canon.replace_subsystem(
        "fpath",
        shard
            .fpath
            .iter()
            .enumerate()
            .map(|(i, d)| (i.to_string(), json_str(d))),
        None,
    );
    // Extras (zle widgets, completions, params_typed) — fold each
    // sub-bucket into its named subsystem.
    for (subsystem, table) in &shard.extras {
        rows_restored += canon.replace_subsystem(
            subsystem,
            table.iter().map(|(k, v)| (k.clone(), json_str(v))),
            None,
        );
    }
    Ok(json!({
        "tag": tag,
        "rows_restored": rows_restored,
        "generation": shard.header.generation,
    }))
}

pub async fn op_snapshot_diff(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let a_tag = args
        .get("a")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `a`"))?
        .to_string();
    let b_tag = args
        .get("b")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `b`"))?
        .to_string();
    let a_path = snapshot_path(state, &a_tag);
    let b_path = snapshot_path(state, &b_tag);
    let a = super::shard::read_canonical_shard(&a_path)
        .map_err(|e| ErrPayload::new("snapshot_read", format!("read `{a_tag}`: {e}")))?;
    let b = super::shard::read_canonical_shard(&b_path)
        .map_err(|e| ErrPayload::new("snapshot_read", format!("read `{b_tag}`: {e}")))?;

    let mut added: Vec<Value> = Vec::new();
    let mut removed: Vec<Value> = Vec::new();
    let mut changed: Vec<Value> = Vec::new();
    let pairs: &[(&_, &_, &str)] = &[
        (&a.aliases, &b.aliases, "alias"),
        (&a.global_aliases, &b.global_aliases, "galias"),
        (&a.suffix_aliases, &b.suffix_aliases, "salias"),
        (&a.functions, &b.functions, "function"),
        (&a.env_exports, &b.env_exports, "env"),
        (&a.params, &b.params, "params"),
        (&a.bindkeys, &b.bindkeys, "bindkey"),
        (&a.compdef, &b.compdef, "compdef"),
        (&a.named_dirs, &b.named_dirs, "named_dir"),
    ];
    for (am, bm, kind) in pairs {
        added.extend(diff_added(am, bm, kind));
        removed.extend(diff_removed(am, bm, kind));
        changed.extend(diff_changed(am, bm, kind));
    }
    Ok(json!({
        "a": a_tag,
        "b": b_tag,
        "added": added,
        "removed": removed,
        "changed": changed,
    }))
}

fn diff_added(
    a: &std::collections::HashMap<String, String>,
    b: &std::collections::HashMap<String, String>,
    kind: &str,
) -> Vec<Value> {
    b.iter()
        .filter(|(k, _)| !a.contains_key(*k))
        .map(|(k, v)| json!({"kind": kind, "name": k, "value": v}))
        .collect()
}

fn diff_removed(
    a: &std::collections::HashMap<String, String>,
    b: &std::collections::HashMap<String, String>,
    kind: &str,
) -> Vec<Value> {
    a.iter()
        .filter(|(k, _)| !b.contains_key(*k))
        .map(|(k, v)| json!({"kind": kind, "name": k, "value": v}))
        .collect()
}

fn diff_changed(
    a: &std::collections::HashMap<String, String>,
    b: &std::collections::HashMap<String, String>,
    kind: &str,
) -> Vec<Value> {
    a.iter()
        .filter_map(|(k, av)| {
            b.get(k).and_then(|bv| {
                if av != bv {
                    Some(json!({"kind": kind, "name": k, "from": av, "to": bv}))
                } else {
                    None
                }
            })
        })
        .collect()
}

fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}
