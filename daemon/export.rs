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
    super::zsync::ensure_schema(state)?;
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

    let body = render(state, &target, &format, additive)?;
    Ok(json!({
        "target": target,
        "format": format,
        "body": body,
    }))
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
        // Binary / introspection-only targets refuse sh format.
        "shard" | "index" | "catalog" | "history" | "entry_stats" | "subscriptions" | "shells"
        | "plugins" | "compiled_files" | "daemon_state" | "_comps" | "_services" | "_patcomps"
        | "_describe_handlers" | "command_hash" | "autoload_table" | "functions" | "theme"
        | "zcompdump" | "script" | "sourced" => {
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
