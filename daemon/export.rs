// zcache view / export / import — universal cache dump/serialize.
//
// Per docs/DAEMON.md "Universal cache dump / view / export":
//   - Default `export` format = sh (eval-compatible) for shell-state targets,
//     native rkyv for binary-only targets, json/yaml as alternatives.
//   - Default `view` format = text (human-readable).
//   - `eval $(zcache export <target>)` resets overlay → canonical with a wipe prefix.
//
// V1 covers the high-leverage shell-state targets:
//   path, fpath, manpath, named_dir, aliases, galiases, saliases, env, params,
//   zstyle, bindkey, setopt, zmodload, compdef.
//
// Catalog/shard/binary-format targets (catalog, shard, index, daemon_state) emit
// json by default since they're not eval-replayable.

use std::sync::Arc;

use serde_json::{json, Value};

use super::ipc::ErrPayload;
use super::ops::OpResult;
use super::state::DaemonState;
use super::zsync::CanonicalRow;

pub async fn op_view(state: &Arc<DaemonState>, args: Value) -> OpResult {
    op_view_or_export(state, args, false).await
}

pub async fn op_export(state: &Arc<DaemonState>, args: Value) -> OpResult {
    op_view_or_export(state, args, true).await
}

async fn op_view_or_export(state: &Arc<DaemonState>, args: Value, _is_export: bool) -> OpResult {
    let target = args
        .get("target")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `target`"))?
        .to_string();
    let format = args
        .get("format")
        .and_then(Value::as_str)
        .unwrap_or("sh")
        .to_string();
    let additive = args
        .get("additive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // `--all-state` aggregator: concatenate every eval-compatible shell-state
    // target into one body. Per docs/DAEMON.md "Default semantics include a
    // wipe prefix so eval truly RESETS overlay back to canonical".
    if target == "all-state" || target == "--all-state" {
        let body = render_all_state(state, &format, additive)?;
        return Ok(json!({
            "target": "all-state",
            "format": format,
            "body": body,
        }));
    }

    let body = render(state, &target, &format, additive)?;
    Ok(json!({
        "target": target,
        "format": format,
        "body": body,
    }))
}

/// All shell-state-eval-compatible subsystems, in the dependency order that
/// `eval $(zcache export --all-state)` expects (path before commands; fpath
/// before autoloads; aliases last so user-mutable state wins).
const ALL_STATE_TARGETS: &[&str] = &[
    "setopt",     // option mask first; downstream behavior depends on this
    "zmodload",   // modules before features that need them
    "path",       // PATH before command_hash + functions
    "fpath",      // FPATH before autoload_table
    "manpath",
    "named_dir",
    "env",        // exported env before params
    "params",
    "zstyle",
    "bindkey",
    "compdef",
    "command_hash",
    "autoload_table",
    "functions",
    "aliases",    // alias is last so it can shadow function/builtin
    "galiases",
    "saliases",
];

fn render_all_state(
    state: &DaemonState,
    format: &str,
    additive: bool,
) -> std::result::Result<String, ErrPayload> {
    if format != "sh" {
        return Err(ErrPayload::new(
            "format_unsupported_for_all_state",
            format!("--all-state requires --format sh (got `{}`)", format),
        ));
    }
    let mut out = String::new();
    out.push_str("# zshrs --all-state snapshot — equivalent to `exec zshrs` minus the exec\n");
    out.push_str(&format!(
        "# generated: {}\n\n",
        chrono::Utc::now().to_rfc3339()
    ));
    for target in ALL_STATE_TARGETS {
        out.push_str(&format!("\n# ----- {} -----\n", target));
        match render_sh(state, target, additive) {
            Ok(s) if !s.is_empty() => out.push_str(&s),
            Ok(_) => {
                out.push_str(&format!("# (no entries for {})\n", target));
            }
            Err(e) => {
                out.push_str(&format!("# skipped {}: {}\n", target, e.msg));
            }
        }
    }
    Ok(out)
}

/// Render a target in the requested format. Returns the rendered string body.
fn render(
    state: &Arc<DaemonState>,
    target: &str,
    format: &str,
    additive: bool,
) -> std::result::Result<String, ErrPayload> {
    match format {
        "sh" => render_sh(state, target, additive),
        "json" => render_json(state, target),
        "yaml" => render_yaml(state, target),
        "text" => render_text(state, target),
        other => Err(ErrPayload::new(
            "bad_format",
            format!("format `{}` not supported (try sh|json|yaml|text)", other),
        )),
    }
}

fn read_canonical(
    state: &DaemonState,
    subsystem: &str,
) -> std::result::Result<Vec<CanonicalRow>, ErrPayload> {
    // rkyv-backed in-memory state is the source of truth — SQLite is only a
    // hydrated view target (refreshed by `zcache hydrate-view`).
    Ok(super::zsync::read_canonical_rows_inmem(state, subsystem))
}

/// Read walk-output entries (command_hash / autoload_table / completions)
/// from catalog.entries — these are the hydrated mirror of the system rkyv
/// shard's `cmd:*` / `fn:*` keys produced by walk.rs Pass 3+4.
fn read_walk_entries(
    state: &DaemonState,
    kind: &str,
) -> std::result::Result<Vec<(String, String)>, ErrPayload> {
    let prefix = match kind {
        "command" => "cmd:",
        "autoload" | "completion" => "fn:",
        other => {
            return Err(ErrPayload::new(
                "bad_kind",
                format!("unknown walk kind `{}`", other),
            ));
        }
    };
    let rows: Vec<(String, String)> = state
        .with_catalog(|conn| {
            let mut stmt = conn.prepare(
                "SELECT fq_name, source_loc FROM entries WHERE plugin_id = 'system' AND kind = ? \
                 ORDER BY fq_name ASC",
            )?;
            let rows = stmt
                .query_map(rusqlite::params![kind], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .map_err(ErrPayload::from)?;
    // Strip the "cmd:" / "fn:" prefix the daemon stores so the export uses
    // bare zsh names.
    Ok(rows
        .into_iter()
        .map(|(name, src)| {
            let bare = name.strip_prefix(prefix).unwrap_or(&name).to_string();
            (bare, src)
        })
        .collect())
}

/// Strip JSON quoting if the value-string came from canonical.value (which we stored
/// as JSON). e.g., `"hello"` → `hello`. Numbers/objects stay as JSON.
fn unjson(s: &str) -> String {
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        // serde_json round-trip to unescape correctly.
        if let Ok(v) = serde_json::from_str::<Value>(s) {
            if let Some(s2) = v.as_str() {
                return s2.to_string();
            }
        }
    }
    s.to_string()
}

fn shell_quote(v: &str) -> String {
    if v.is_empty() {
        return "''".to_string();
    }
    if v.chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '.' | '-' | ':' | ',' | '+'))
    {
        return v.to_string();
    }
    let escaped = v.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

fn render_sh(
    state: &DaemonState,
    target: &str,
    additive: bool,
) -> std::result::Result<String, ErrPayload> {
    let mut out = String::new();
    let rows = read_canonical(state, &normalize_subsystem(target)?)?;

    match target {
        "path" | "fpath" | "manpath" | "infopath" | "cdpath" | "ld_library_path" => {
            // Array-style: `path=(...)` (lower-case, ties to colon-string upper-case).
            let lower = target;
            let upper = target.to_uppercase();
            let dirs: Vec<String> = rows.iter().map(|r| unjson(&r.value)).collect();

            if !additive {
                out.push_str(&format!("{}=()\n", lower));
            }
            if !dirs.is_empty() {
                let joined = dirs
                    .iter()
                    .map(|d| shell_quote(d))
                    .collect::<Vec<_>>()
                    .join(" ");
                out.push_str(&format!("{}+=({})\n", lower, joined));
                let exported = dirs.join(":");
                out.push_str(&format!("export {}={}\n", upper, shell_quote(&exported)));
            }
        }
        "named_dir" => {
            if !additive {
                // No clean way to wipe all named dirs; `unhash -dm '*'` is the zsh form.
                out.push_str("unhash -dm '*' 2>/dev/null || true\n");
            }
            for r in &rows {
                out.push_str(&format!(
                    "hash -d {}={}\n",
                    r.key,
                    shell_quote(&unjson(&r.value))
                ));
            }
        }
        "aliases" => {
            if !additive {
                out.push_str("unalias -m '*' 2>/dev/null || true\n");
            }
            for r in &rows {
                out.push_str(&format!(
                    "alias {}={}\n",
                    r.key,
                    shell_quote(&unjson(&r.value))
                ));
            }
        }
        "galiases" => {
            if !additive {
                out.push_str(
                    "# wipe global aliases by re-listing as no-op (zsh has no -gm wipe)\n",
                );
            }
            for r in &rows {
                out.push_str(&format!(
                    "alias -g {}={}\n",
                    r.key,
                    shell_quote(&unjson(&r.value))
                ));
            }
        }
        "saliases" => {
            if !additive {
                out.push_str("# wipe suffix aliases\n");
            }
            for r in &rows {
                out.push_str(&format!(
                    "alias -s {}={}\n",
                    r.key,
                    shell_quote(&unjson(&r.value))
                ));
            }
        }
        "env" => {
            if !additive {
                out.push_str("# (no global wipe — env is process-state)\n");
            }
            for r in &rows {
                out.push_str(&format!(
                    "export {}={}\n",
                    r.key,
                    shell_quote(&unjson(&r.value))
                ));
            }
        }
        "params" => {
            if !additive {
                out.push_str("# (no global wipe for shell parameters)\n");
            }
            for r in &rows {
                let val = unjson(&r.value);
                // Best-effort type detection — array values come as JSON arrays.
                if val.starts_with('[') && val.ends_with(']') {
                    if let Ok(arr) = serde_json::from_str::<Vec<String>>(&val) {
                        let joined = arr
                            .iter()
                            .map(|s| shell_quote(s))
                            .collect::<Vec<_>>()
                            .join(" ");
                        out.push_str(&format!("typeset -ga {}=({})\n", r.key, joined));
                        continue;
                    }
                }
                out.push_str(&format!("typeset -g {}={}\n", r.key, shell_quote(&val)));
            }
        }
        "zstyle" => {
            for r in &rows {
                // `key` is the pattern, `value` is the rest of the args (already JSON-stringified).
                out.push_str(&format!(
                    "zstyle {} {}\n",
                    shell_quote(&r.key),
                    unjson(&r.value)
                ));
            }
        }
        "bindkey" => {
            if !additive {
                out.push_str("# (bindkey -d would clear; uncomment if desired)\n# bindkey -d\n");
            }
            for r in &rows {
                out.push_str(&format!(
                    "bindkey {} {}\n",
                    shell_quote(&r.key),
                    unjson(&r.value)
                ));
            }
        }
        "setopt" => {
            for r in &rows {
                let v = unjson(&r.value);
                if v == "on" || v == "true" || v == "1" {
                    out.push_str(&format!("setopt {}\n", r.key));
                } else {
                    out.push_str(&format!("unsetopt {}\n", r.key));
                }
            }
        }
        "zmodload" => {
            for r in &rows {
                out.push_str(&format!("zmodload {}\n", r.key));
            }
        }
        "compdef" => {
            for r in &rows {
                out.push_str(&format!("compdef {} {}\n", unjson(&r.value), r.key));
            }
        }
        "command_hash" => {
            // Sourced from catalog.entries WHERE kind='command' (mirror of the
            // system rkyv shard's `cmd:*` entries built by walk.rs Pass 3).
            let entries = read_walk_entries(state, "command")?;
            if !additive {
                out.push_str("hash -r\n");
            }
            for (name, path_str) in entries {
                out.push_str(&format!("hash {}={}\n", name, shell_quote(&path_str)));
            }
        }
        "autoload_table" => {
            let entries = read_walk_entries(state, "autoload")?;
            let completions = read_walk_entries(state, "completion")?;
            if !additive {
                out.push_str("# autoload table from $FPATH walk (Pass 3)\n");
            }
            // Single autoload -Uz with all names (zsh accepts batched form).
            let mut names: Vec<String> =
                entries.iter().map(|(n, _)| n.clone()).collect();
            names.extend(completions.iter().map(|(n, _)| n.clone()));
            names.sort();
            names.dedup();
            for chunk in names.chunks(64) {
                out.push_str("autoload -Uz");
                for n in chunk {
                    out.push(' ');
                    out.push_str(&shell_quote(n));
                }
                out.push('\n');
            }
        }
        "functions" => {
            // Function bodies analyzed from .zshrc are stored under subsystem
            // "function"; emit as `function name() { body }`.
            let func_rows = read_canonical(state, "function")?;
            for r in &func_rows {
                let body = unjson(&r.value);
                out.push_str(&format!("function {} {{\n{}\n}}\n", r.key, body));
            }
        }
        // Binary / introspection-only targets refuse sh format.
        "shard" | "index" | "catalog" | "history" | "entry_stats" | "subscriptions" | "shells"
        | "plugins" | "compiled_files" | "daemon_state" | "_comps" | "_services" | "_patcomps"
        | "_describe_handlers" | "theme" | "zcompdump" | "script" | "sourced" => {
            return Err(ErrPayload::new(
                "format_unsupported_for_target",
                format!(
                    "target `{}` does not support sh format; try --format json",
                    target
                ),
            ));
        }
        other => {
            return Err(ErrPayload::new(
                "unknown_target",
                format!("target `{}` not recognized", other),
            ));
        }
    }
    Ok(out)
}

fn render_json(state: &DaemonState, target: &str) -> std::result::Result<String, ErrPayload> {
    // Daemon-introspection targets (no canonical/walk subsystem mapping).
    match target {
        "shells" => {
            let sessions = state.snapshot_sessions();
            let payload: Vec<Value> = sessions
                .iter()
                .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
                .collect();
            return Ok(serde_json::to_string_pretty(&payload).unwrap_or_default());
        }
        "subscriptions" => {
            let subs = state.list_all_subscriptions();
            let payload: Vec<Value> = subs
                .iter()
                .map(|s| {
                    json!({
                        "id": s.id,
                        "client_id": s.client_id,
                        "pattern": s.pattern,
                        "scope_pat": s.scope_pat,
                        "topic_pat": s.topic_pat,
                        "paused": s.paused,
                    })
                })
                .collect();
            return Ok(serde_json::to_string_pretty(&payload).unwrap_or_default());
        }
        "daemon_state" => {
            let payload = json!({
                "pid": state.pid,
                "uptime_ms": state.uptime_ms(),
                "started_at": state.start_wall.to_rfc3339(),
                "session_count": state.session_count(),
                "canonical_rows": state.canonical.total_rows(),
                "subscription_count": state.list_all_subscriptions().len(),
                "watched_paths": state.fs_watcher.stats().watched_path_count,
            });
            return Ok(serde_json::to_string_pretty(&payload).unwrap_or_default());
        }
        "entry_stats" => {
            // Catalog.entry_stats is a hydrated mirror of per-entry runtime
            // counters. Read from sqlite (since rkyv stores cumulative blob,
            // not per-entry stats with these columns).
            let rows: Vec<Value> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT fq_name, last_called_at, call_count, total_ns FROM entry_stats \
                         ORDER BY call_count DESC, fq_name ASC LIMIT 10000",
                    )?;
                    let rows: Vec<Value> = stmt
                        .query_map([], |r| {
                            Ok(json!({
                                "fq_name": r.get::<_, String>(0)?,
                                "last_called_at": r.get::<_, Option<i64>>(1)?,
                                "call_count": r.get::<_, i64>(2)?,
                                "total_ns": r.get::<_, i64>(3)?,
                            }))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            return Ok(serde_json::to_string_pretty(&rows).unwrap_or_default());
        }
        "plugins" => {
            let rows: Vec<Value> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT name, version, source, installed_at, enabled FROM plugins \
                         ORDER BY name ASC",
                    )?;
                    let rows: Vec<Value> = stmt
                        .query_map([], |r| {
                            Ok(json!({
                                "name": r.get::<_, String>(0)?,
                                "version": r.get::<_, Option<String>>(1)?,
                                "source": r.get::<_, Option<String>>(2)?,
                                "installed_at": r.get::<_, Option<i64>>(3)?,
                                "enabled": r.get::<_, Option<bool>>(4)?,
                            }))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            return Ok(serde_json::to_string_pretty(&rows).unwrap_or_default());
        }
        "script" | "sourced" => {
            // compiled_files rows for kind='script' or kind='source' — the
            // ingested file registry from zsource / load_script.
            let kinds: &[&str] = if target == "script" {
                &["script", "zshrc"]
            } else {
                &["source", "zshrc", "plugin_init", "autoload"]
            };
            let placeholders = kinds
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 1))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT path, kind, mtime, inode, last_used_at, use_count, bytes_in, sensitive \
                 FROM compiled_files WHERE kind IN ({}) ORDER BY use_count DESC LIMIT 10000",
                placeholders
            );
            let rows: Vec<Value> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(&sql)?;
                    let params: Vec<&dyn rusqlite::ToSql> =
                        kinds.iter().map(|k| k as &dyn rusqlite::ToSql).collect();
                    let rows: Vec<Value> = stmt
                        .query_map(&params[..], |r| {
                            Ok(json!({
                                "path": r.get::<_, String>(0)?,
                                "kind": r.get::<_, String>(1)?,
                                "mtime": r.get::<_, i64>(2)?,
                                "inode": r.get::<_, i64>(3)?,
                                "last_used_at": r.get::<_, Option<i64>>(4)?,
                                "use_count": r.get::<_, i64>(5)?,
                                "bytes_in": r.get::<_, i64>(6)?,
                                "sensitive": r.get::<_, bool>(7)?,
                            }))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            return Ok(serde_json::to_string_pretty(&rows).unwrap_or_default());
        }
        "compiled_files" => {
            let rows: Vec<Value> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT path, kind, mtime, inode, last_used_at, use_count, bytes_in, \
                         bytes_out, sensitive FROM compiled_files ORDER BY use_count DESC LIMIT 10000",
                    )?;
                    let rows: Vec<Value> = stmt
                        .query_map([], |r| {
                            Ok(json!({
                                "path": r.get::<_, String>(0)?,
                                "kind": r.get::<_, String>(1)?,
                                "mtime": r.get::<_, i64>(2)?,
                                "inode": r.get::<_, i64>(3)?,
                                "last_used_at": r.get::<_, Option<i64>>(4)?,
                                "use_count": r.get::<_, i64>(5)?,
                                "bytes_in": r.get::<_, i64>(6)?,
                                "bytes_out": r.get::<_, i64>(7)?,
                                "sensitive": r.get::<_, bool>(8)?,
                            }))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            return Ok(serde_json::to_string_pretty(&rows).unwrap_or_default());
        }
        _ => {}
    }

    // Walk-output targets render from catalog.entries.
    let walk_kinds: Option<&[&str]> = match target {
        "command_hash" => Some(&["command"]),
        "autoload_table" => Some(&["autoload", "completion"]),
        _ => None,
    };
    if let Some(kinds) = walk_kinds {
        let mut all = Vec::new();
        for k in kinds {
            all.extend(read_walk_entries(state, k)?);
        }
        let payload: Vec<Value> = all
            .into_iter()
            .map(|(name, src)| json!({ "key": name, "value": src }))
            .collect();
        return Ok(serde_json::to_string_pretty(&payload).unwrap_or_default());
    }
    let subsystem = normalize_subsystem(target)?;
    let rows = read_canonical(state, &subsystem)?;
    let payload: Vec<Value> = rows
        .iter()
        .map(|r| json!({ "key": r.key, "value": serde_json::from_str::<Value>(&r.value).unwrap_or(Value::String(r.value.clone())) }))
        .collect();
    Ok(serde_json::to_string_pretty(&payload).unwrap_or_default())
}

fn render_yaml(state: &DaemonState, target: &str) -> std::result::Result<String, ErrPayload> {
    // Tiny YAML emitter — kv-list only (no toml dep needed; toml's already a dep
    // but yaml-style is more readable for this use case). Format:
    //   subsystem: target
    //   rows:
    //     - key: K
    //       value: V
    let subsystem = normalize_subsystem(target)?;
    let rows = read_canonical(state, &subsystem)?;
    let mut out = String::new();
    out.push_str(&format!("subsystem: {}\nrows:\n", subsystem));
    for r in &rows {
        out.push_str(&format!(
            "  - key: {}\n    value: {}\n",
            yaml_quote(&r.key),
            yaml_quote(&unjson(&r.value))
        ));
    }
    Ok(out)
}

fn render_text(state: &DaemonState, target: &str) -> std::result::Result<String, ErrPayload> {
    // Walk-output targets render from catalog.entries (the hydrated mirror of
    // the system rkyv shard). Normal canonical-state targets render from the
    // in-memory rkyv-backed canonical engine.
    if let Some(rendered) = render_walk_text(state, target)? {
        return Ok(rendered);
    }
    let subsystem = normalize_subsystem(target)?;
    let rows = read_canonical(state, &subsystem)?;
    let mut out = String::new();
    out.push_str(&format!("# subsystem: {}\n", subsystem));
    out.push_str(&format!("# {} entries\n\n", rows.len()));
    for r in &rows {
        out.push_str(&format!("{} = {}\n", r.key, unjson(&r.value)));
    }
    Ok(out)
}

fn render_walk_text(
    state: &DaemonState,
    target: &str,
) -> std::result::Result<Option<String>, ErrPayload> {
    let kinds: &[&str] = match target {
        "command_hash" => &["command"],
        "autoload_table" => &["autoload", "completion"],
        "functions" if read_canonical(state, "function")?.is_empty() => &["autoload"],
        _ => return Ok(None),
    };
    let mut all = Vec::new();
    for k in kinds {
        all.extend(read_walk_entries(state, k)?);
    }
    let mut out = String::new();
    out.push_str(&format!("# target: {}\n", target));
    out.push_str(&format!("# {} entries\n\n", all.len()));
    for (name, src) in all {
        out.push_str(&format!("{} = {}\n", name, src));
    }
    Ok(Some(out))
}

fn yaml_quote(v: &str) -> String {
    if v.contains(':') || v.contains('\n') || v.contains('"') || v.is_empty() {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v.to_string()
    }
}

/// Map external target name → canonical-table subsystem. Most are 1:1; a few
/// (path/fpath/manpath/etc.) match by lowercasing.
fn normalize_subsystem(target: &str) -> std::result::Result<String, ErrPayload> {
    Ok(match target {
        "path" | "fpath" | "manpath" | "infopath" | "cdpath" | "ld_library_path" => {
            target.to_string()
        }
        "named_dir" | "aliases" | "galiases" | "saliases" => match target {
            "aliases" => "alias".to_string(),
            "galiases" => "galias".to_string(),
            "saliases" => "salias".to_string(),
            other => other.to_string(),
        },
        "env" | "params" | "zstyle" | "bindkey" | "setopt" | "zmodload" | "compdef" => {
            target.to_string()
        }
        "function" => "function".to_string(),
        // Fall-through: treat target as the subsystem name.
        other => other.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fresh() -> (TempDir, Arc<DaemonState>) {
        let tmp = TempDir::new().unwrap();
        let paths = super::super::paths::CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let state = DaemonState::new(paths).unwrap();
        super::super::zsync::ensure_schema(&state).unwrap();
        (tmp, state)
    }

    async fn push(state: &Arc<DaemonState>, subsystem: &str, value: Value) {
        super::super::zsync::op_push_canonical(
            state,
            1,
            json!({ "subsystem": subsystem, "value": value }),
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn export_aliases_sh_with_wipe() {
        let (_tmp, state) = fresh();
        push(
            &state,
            "alias",
            json!({ "ll": "ls -la", "gst": "git status" }),
        )
        .await;

        let r = op_export(&state, json!({ "target": "aliases" }))
            .await
            .unwrap();
        let body = r["body"].as_str().unwrap();
        assert!(body.starts_with("unalias -m '*'"));
        assert!(body.contains("alias gst="));
        assert!(body.contains("alias ll="));
    }

    #[tokio::test]
    async fn export_path_sh_emits_array() {
        let (_tmp, state) = fresh();
        push(&state, "path", json!(["/usr/local/bin", "/usr/bin"])).await;

        let r = op_export(&state, json!({ "target": "path" }))
            .await
            .unwrap();
        let body = r["body"].as_str().unwrap();
        assert!(body.contains("path=()"));
        assert!(body.contains("path+=("));
        assert!(body.contains("/usr/local/bin"));
        assert!(body.contains("export PATH="));
    }

    #[tokio::test]
    async fn export_named_dir_sh() {
        let (_tmp, state) = fresh();
        push(&state, "named_dir", json!({ "proj": "/Users/wizard/p" })).await;
        let r = op_export(&state, json!({ "target": "named_dir" }))
            .await
            .unwrap();
        let body = r["body"].as_str().unwrap();
        assert!(body.contains("hash -d proj"));
    }

    #[tokio::test]
    async fn export_setopt_emits_setopt_unsetopt() {
        let (_tmp, state) = fresh();
        push(
            &state,
            "setopt",
            json!({ "extended_glob": "on", "beep": "off" }),
        )
        .await;
        let r = op_export(&state, json!({ "target": "setopt" }))
            .await
            .unwrap();
        let body = r["body"].as_str().unwrap();
        assert!(body.contains("setopt extended_glob"));
        assert!(body.contains("unsetopt beep"));
    }

    #[tokio::test]
    async fn export_json_format() {
        let (_tmp, state) = fresh();
        push(&state, "alias", json!({ "ll": "ls -la" })).await;
        let r = op_export(&state, json!({ "target": "aliases", "format": "json" }))
            .await
            .unwrap();
        let body = r["body"].as_str().unwrap();
        let parsed: Value = serde_json::from_str(body).unwrap();
        assert!(parsed.is_array());
    }

    #[tokio::test]
    async fn export_unsupported_format_returns_error() {
        let (_tmp, state) = fresh();
        let r = op_export(&state, json!({ "target": "shard", "format": "sh" })).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn export_additive_skips_wipe_prefix() {
        let (_tmp, state) = fresh();
        push(&state, "alias", json!({ "ll": "ls -la" })).await;
        let r = op_export(&state, json!({ "target": "aliases", "additive": true }))
            .await
            .unwrap();
        let body = r["body"].as_str().unwrap();
        assert!(!body.starts_with("unalias -m '*'"));
    }
}
