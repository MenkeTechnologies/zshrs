//! `daemon.definitions.*` ops — the recorder's catalog as a queryable
//! API. Per docs/DAEMON_AS_SERVICE.md §"DEFINITIONS":
//!
//! | Op                       | Args                  | Returns                   |
//! |--------------------------|-----------------------|---------------------------|
//! | `definitions_query`      | `{kind?, name?, prefix?, limit?}` | `{records, count}` |
//! | `definitions_kinds`      | `{}`                  | `{kinds: [...], count}`   |
//! | `definitions_subscribe`  | (HTTP-only via SSE)   | streamed `event: defs`    |
//!
//! Storage substrate is the canonical engine — the same one
//! `recorder_ingest` writes to. Each subsystem (`alias`, `function`,
//! `env`, `bindkey`, `compdef`, `setopt`, `zle`, `completion`,
//! `params_typed`, ...) maps to one `kind`. The query side is
//! read-only — recorder is the writer.

use std::sync::Arc;

use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;

/// Every subsystem the canonical engine knows about that maps to a
/// recorder DefKind. Source of truth for `definitions_kinds` + the
/// "kind not found" error in `definitions_query`. Keep in sync with
/// the recorder's DefKind enum + the daemon's recorder_ingest op.
const KNOWN_KINDS: &[&str] = &[
    "alias",
    "galias",
    "salias",
    "function",
    "function_autoload",
    "env",
    "params",
    "params_typed",
    "bindkey",
    "compdef",
    "named_dir",
    "zstyle",
    "zmodload",
    "setopt",
    "trap",
    "sched",
    "source",
    "zle",
    "completion",
    "path",
    "fpath",
    "manpath",
];

pub async fn op_definitions_kinds(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    // Filter to kinds that actually have rows so `kinds` reflects what
    // the user can usefully query right now.
    let mut populated: Vec<&str> = KNOWN_KINDS
        .iter()
        .copied()
        .filter(|k| !state.canonical.rows_for(k).is_empty())
        .collect();
    populated.sort();
    let count = populated.len();
    Ok(json!({
        "kinds": populated,
        "count": count,
        "all_known": KNOWN_KINDS,
    }))
}

pub async fn op_definitions_query(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let kind_arg = args.get("kind").and_then(Value::as_str);
    let name_arg = args.get("name").and_then(Value::as_str);
    let prefix_arg = args.get("prefix").and_then(Value::as_str);
    let shell_id_filter = args.get("shell_id").and_then(Value::as_str);
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10_000) as usize;

    // Restrict to KNOWN_KINDS so the caller can't accidentally probe
    // catalog-internal subsystems via this op (catalog/canonical hide
    // a few admin tables that aren't recorder definitions).
    let kinds_to_scan: Vec<&str> = match kind_arg {
        Some(k) => {
            if !KNOWN_KINDS.contains(&k) {
                return Err(ErrPayload::new(
                    "no_such_kind",
                    format!(
                        "kind `{k}` is not a known recorder definition kind \
                         (known: {})",
                        KNOWN_KINDS.join(", ")
                    ),
                ));
            }
            vec![k]
        }
        None => KNOWN_KINDS.to_vec(),
    };

    let mut records: Vec<Value> = Vec::new();
    'outer: for kind in &kinds_to_scan {
        for row in state.canonical.rows_for(kind) {
            if let Some(want) = name_arg {
                if row.key != want {
                    continue;
                }
            }
            if let Some(p) = prefix_arg {
                if !row.key.starts_with(p) {
                    continue;
                }
            }
            if let Some(want_shell) = shell_id_filter {
                // None on the row = legacy / pre-shell_id = "zshrs" by
                // convention. Match against that so users running
                // `--shell-id zshrs` see pre-tagged rows.
                let row_shell = row.shell_id.as_deref().unwrap_or("zshrs");
                if row_shell != want_shell {
                    continue;
                }
            }
            records.push(json!({
                "kind": kind,
                "name": row.key,
                "value": unjson_str(&row.value),
                "set_at_ns": row.set_at_ns,
                "set_by_shell": row.set_by_shell,
                "shell_id": row.shell_id.clone().unwrap_or_else(|| "zshrs".to_string()),
                "file": row.file,
                "line": row.line,
            }));
            if records.len() >= limit {
                break 'outer;
            }
        }
    }
    let count = records.len();
    Ok(json!({
        "records": records,
        "count": count,
        "limit": limit,
        "filters": {
            "kind": kind_arg,
            "name": name_arg,
            "prefix": prefix_arg,
            "shell_id": shell_id_filter,
        },
    }))
}

/// Single-record write API for non-zshrs shells (per
/// docs/DAEMON_AS_SERVICE.md §"Third-party shell recorders" — the
/// `definitions emit` op the spec defines). bash/fish/zsh/etc.
/// shell wrappers call this from inside their own recorder hooks
/// to push one definition into the federated catalog.
///
/// Args (all string except line):
///   shell_id  REQUIRED — recording-shell identity per docs/SHELL_IDS.md
///   kind      REQUIRED — one of the KNOWN_KINDS (alias/function/env/...)
///   name      REQUIRED — definition name (alias name, function name, ...)
///   value     OPTIONAL — definition value (alias body, fn body, ...)
///   file      OPTIONAL — source file:line attribution
///   line      OPTIONAL — source line number
///   fn_chain  OPTIONAL — caller chain
pub async fn op_definitions_emit(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let shell_id = args
        .get("shell_id")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ErrPayload::new("bad_args", "missing `shell_id` (see docs/SHELL_IDS.md)")
        })?
        .to_string();
    let kind = args
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `kind`"))?;
    if !KNOWN_KINDS.contains(&kind) {
        return Err(ErrPayload::new(
            "no_such_kind",
            format!(
                "kind `{kind}` is not a known recorder definition kind \
                 (known: {})",
                KNOWN_KINDS.join(", ")
            ),
        ));
    }
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `name`"))?
        .to_string();
    let value = args
        .get("value")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    // Wire format mirrors what `recorder_ingest` produces — value is
    // JSON-string-encoded so canonical rows stay shape-uniform with
    // existing zshrs-recorder ingests.
    let json_value = serde_json::Value::String(value).to_string();
    let file = args
        .get("file")
        .and_then(Value::as_str)
        .map(str::to_string);
    let line = args
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok());
    let n = state.canonical.upsert_tagged_with_attrs(
        kind,
        &name,
        &json_value,
        None,
        Some(shell_id.clone()),
        file.clone(),
        line,
    );

    Ok(json!({
        "kind": kind,
        "name": name,
        "shell_id": shell_id,
        "wrote_rows": n,
        "file": file,
        "line": line,
        "fn_chain": args.get("fn_chain"),
    }))
}

/// Cross-shell or before/after diff over the canonical catalog.
/// Compares the `shell_a` view of each kind against the `shell_b`
/// view and reports added / removed / changed rows.
///
/// Args:
///   shell_a   REQUIRED — first shell_id (e.g. "zshrs")
///   shell_b   REQUIRED — second shell_id (e.g. "bash")
///   kind      OPTIONAL — restrict to one kind; default = every known kind
pub async fn op_definitions_diff(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let shell_a = args
        .get("shell_a")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `shell_a`"))?
        .to_string();
    let shell_b = args
        .get("shell_b")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `shell_b`"))?
        .to_string();
    let kind_filter = args.get("kind").and_then(Value::as_str);

    let kinds: Vec<&str> = match kind_filter {
        Some(k) if KNOWN_KINDS.contains(&k) => vec![k],
        Some(k) => {
            return Err(ErrPayload::new(
                "no_such_kind",
                format!("kind `{k}` is not a known recorder definition kind"),
            ));
        }
        None => KNOWN_KINDS.to_vec(),
    };

    let mut added: Vec<Value> = Vec::new();
    let mut removed: Vec<Value> = Vec::new();
    let mut changed: Vec<Value> = Vec::new();
    for kind in &kinds {
        let mut a_rows = std::collections::HashMap::<String, String>::new();
        let mut b_rows = std::collections::HashMap::<String, String>::new();
        for row in state.canonical.rows_for(kind) {
            let row_shell = row.shell_id.as_deref().unwrap_or("zshrs");
            if row_shell == shell_a {
                a_rows.insert(row.key.clone(), unjson_str(&row.value));
            }
            if row_shell == shell_b {
                b_rows.insert(row.key.clone(), unjson_str(&row.value));
            }
        }
        for (k, v) in &b_rows {
            if !a_rows.contains_key(k) {
                added.push(json!({"kind": kind, "name": k, "value": v}));
            }
        }
        for (k, v) in &a_rows {
            if !b_rows.contains_key(k) {
                removed.push(json!({"kind": kind, "name": k, "value": v}));
            } else if let Some(bv) = b_rows.get(k) {
                if bv != v {
                    changed
                        .push(json!({"kind": kind, "name": k, "from": v, "to": bv}));
                }
            }
        }
    }
    Ok(json!({
        "shell_a": shell_a,
        "shell_b": shell_b,
        "added": added,
        "removed": removed,
        "changed": changed,
        "summary": {
            "added": added.len(),
            "removed": removed.len(),
            "changed": changed.len(),
        },
    }))
}

/// `definitions_subscribe` — flip the per-session opt-in flag so this
/// client starts receiving `recorder_ingested` Frame::Event broadcasts
/// (the IPC counterpart of HTTP `/stream/definitions`). Off by default
/// so silent clients don't get every recorder bundle's summary on
/// their socket. Idempotent — calling twice from the same session is
/// a no-op (returns `was_subscribed: true` the second time).
pub async fn op_definitions_subscribe(
    state: &Arc<DaemonState>,
    client_id: u64,
    _args: Value,
) -> OpResult {
    let prev = state
        .set_definitions_subscribed(client_id, true)
        .ok_or_else(|| ErrPayload::new("no_session", "client session not registered"))?;
    Ok(json!({
        "subscribed": true,
        "was_subscribed": prev,
    }))
}

/// `definitions_unsubscribe` — clear the opt-in flag set by
/// `definitions_subscribe`. Idempotent. Sessions that never subscribed
/// are still allowed to call this (returns `was_subscribed: false`).
pub async fn op_definitions_unsubscribe(
    state: &Arc<DaemonState>,
    client_id: u64,
    _args: Value,
) -> OpResult {
    let prev = state
        .set_definitions_subscribed(client_id, false)
        .ok_or_else(|| ErrPayload::new("no_session", "client session not registered"))?;
    Ok(json!({
        "subscribed": false,
        "was_subscribed": prev,
    }))
}

/// JSON-decode a canonical-row value. The canonical engine wraps
/// scalar values in JSON quotes (`"foo"`); strip that for
/// presentation. Falls back to the raw string if the decode fails so
/// caller-side parsing isn't surprised by a value that wasn't actually
/// JSON-wrapped.
fn unjson_str(s: &str) -> String {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        if let Some(plain) = v.as_str() {
            return plain.to_string();
        }
    }
    s.to_string()
}
