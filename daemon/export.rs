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

use rusqlite::OptionalExtension;
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
    let name = args.get("name").and_then(Value::as_str).map(str::to_string);

    // `format=pdf` — render the canonical state of `target` as a paginated
    // PDF document. We delegate the data-dump to the existing `text`
    // renderer (one line per row, human-readable), then lay that text
    // into a PDF via printpdf. Response carries the bytes base64-encoded
    // so the existing JSON envelope stays uniform across formats; HTTP
    // clients decode with `base64 -d`. No native deps; printpdf is
    // pure-Rust.
    if format == "pdf" {
        let body = render(state, &target, "text", additive)?;
        let pdf_bytes = pdf_render(&target, &body)
            .map_err(|e| ErrPayload::new("pdf_render", e.to_string()))?;
        use base64::Engine as _;
        let body_base64 = base64::engine::general_purpose::STANDARD.encode(&pdf_bytes);
        return Ok(json!({
            "target": target,
            "format": "pdf",
            "body_base64": body_base64,
            "byte_count": pdf_bytes.len(),
        }));
    }

    // `functions <name>` — single named function, in the requested format.
    // sh: emit `function <name> { <body> }`; disasm: stub until the bytecode
    // emitter lands. Per docs/DAEMON.md "functions [<name>] All function
    // bytecode, or one named function (+ disassembly with --format disasm)".
    if let (Some(n), "functions") = (name.as_deref(), target.as_str()) {
        // Unified function namespace per zsh semantics: inline-defined
        // (subsystem `function`) and fpath-autoload (subsystem
        // `function_autoload`) are queryable by the same name. Inline
        // wins on collision (same as zsh — explicit definition shadows
        // autoload).
        let row = state
            .canonical
            .row("function", &n)
            .or_else(|| state.canonical.row("function_autoload", &n))
            .ok_or_else(|| ErrPayload::new("no_function", format!("function `{}` not found", n)))?;
        let body = unjson(&row.value);
        let out_str = match format.as_str() {
            "sh" | "" => format!("function {} {{\n{}\n}}\n", n, body),
            "disasm" => format!(
                "# function {} — bytecode disassembly\n# (not yet wired: \
                 daemon stores source bytes in v1; bytecode emitter arrives \
                 with the parser-in-daemon work)\n# source:\n{}\n",
                n, body
            ),
            "json" => json!({ "name": n, "body": body }).to_string(),
            "text" => format!("# function: {}\n{}\n", n, body),
            other => {
                return Err(ErrPayload::new(
                    "bad_format",
                    format!("function format `{}` not supported", other),
                ));
            }
        };
        return Ok(json!({ "target": target, "format": format, "name": n, "body": out_str }));
    }

    // `script <path>` / `sourced <path>` — single compiled-file row by exact
    // path. Per docs/DAEMON.md "script <path>: bytecode for a cached `zshrs
    // FILE` script; sourced <path>: bytecode for a sourced file (single)".
    if let (Some(path), "script" | "sourced") = (name.as_deref(), target.as_str()) {
        let row = state
            .with_catalog(|conn| {
                conn.query_row(
                    "SELECT path, kind, mtime, inode, last_used_at, use_count, bytes_in, sensitive, parent_paths \
                     FROM compiled_files WHERE path = ?",
                    rusqlite::params![path],
                    |r| {
                        Ok(json!({
                            "path": r.get::<_, String>(0)?,
                            "kind": r.get::<_, String>(1)?,
                            "mtime": r.get::<_, i64>(2)?,
                            "inode": r.get::<_, i64>(3)?,
                            "last_used_at": r.get::<_, Option<i64>>(4)?,
                            "use_count": r.get::<_, i64>(5)?,
                            "bytes_in": r.get::<_, i64>(6)?,
                            "sensitive": r.get::<_, bool>(7)?,
                            "parent_paths": r.get::<_, String>(8).unwrap_or_default(),
                        }))
                    },
                )
                .optional()})
            .map_err(ErrPayload::from)?;
        let body = match row {
            Some(v) => match format.as_str() {
                "json" | "sh" | "text" | "" => serde_json::to_string_pretty(&v).unwrap_or_default(),
                "disasm" => format!(
                    "# {} {} — bytecode disassembly\n# (not yet wired: v1 \
                     stores source bytes; bytecode emitter arrives with the \
                     parser-in-daemon work)\n",
                    target, path
                ),
                other => {
                    return Err(ErrPayload::new(
                        "bad_format",
                        format!("{} format `{}` not supported", target, other),
                    ));
                }
            },
            None => format!("# {} {} — not in compiled_files\n", target, path),
        };
        return Ok(json!({ "target": target, "format": format, "name": path, "body": body }));
    }

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

    // Sensitive-content guard per DAEMON.md "Sensitive content" + the
    // "no friction" CLAUDE.md rule: refuse plaintext print of sensitive
    // sourced files unless --show-sensitive is passed. Implemented by
    // checking the compiled_files.sensitive flag for `script <path>` and
    // `sourced <path>` targets.
    let show_sensitive = args
        .get("show_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !show_sensitive {
        if let Some(path) = args.get("name").and_then(Value::as_str) {
            if (target == "script" || target == "sourced")
                && is_path_sensitive(state, path).unwrap_or(false)
            {
                return Err(ErrPayload::new(
                    "sensitive",
                    format!(
                        "`{}` is flagged sensitive; pass --show-sensitive to print contents",
                        path
                    ),
                ));
            }
        }
    }
    // For `--all` archive-style export, exclude sensitive files unless the
    // caller passed --include-sensitive.
    let include_sensitive = args
        .get("include_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let filter = args
        .get("filter")
        .and_then(Value::as_str)
        .map(str::to_string);
    let range = args
        .get("range")
        .and_then(Value::as_str)
        .map(str::to_string);
    let all_flag = args.get("all").and_then(Value::as_bool).unwrap_or(false);

    // Targets that honor filter/range/all natively, per DAEMON.md examples
    // (lines 552-555: command_hash --filter, history --filter --range,
    // subscriptions --all, plugins --filter).
    if let Some(special) = render_filtered(
        state,
        &target,
        &format,
        filter.as_deref(),
        range.as_deref(),
        all_flag,
        include_sensitive,
    ) {
        let body = special?;
        return Ok(json!({
            "target": target,
            "format": format,
            "filter": filter,
            "range": range,
            "all": all_flag,
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

/// Compiled-files sensitive-flag lookup. Used by view/export to gate
/// terminal output.
fn is_path_sensitive(state: &Arc<DaemonState>, path: &str) -> rusqlite::Result<bool> {
    state.with_catalog(|conn| -> rusqlite::Result<bool> {
        let n: Option<bool> = conn
            .query_row(
                "SELECT sensitive FROM compiled_files WHERE path = ?",
                rusqlite::params![path],
                |r| r.get::<_, bool>(0),
            )
            .optional()?;
        Ok(n.unwrap_or(false))
    })
}

/// Honor --filter / --range / --all on the targets where the spec mentions
/// them. Returns Some(Ok(body)) on a hit, Some(Err) on a hit-but-failed,
/// None on a miss (caller falls through to the regular render path).
fn render_filtered(
    state: &Arc<DaemonState>,
    target: &str,
    format: &str,
    filter: Option<&str>,
    range: Option<&str>,
    all_flag: bool,
    _include_sensitive: bool,
) -> Option<std::result::Result<String, ErrPayload>> {
    match target {
        // command_hash --filter '<glob>': scan catalog entries kind='command'
        // and filter by the glob pattern.
        "command_hash" => {
            let pat = filter?;
            let glob_re = match glob_to_regex(pat) {
                Ok(r) => r,
                Err(e) => return Some(Err(ErrPayload::new("bad_filter", e))),
            };
            let res = state.with_catalog(|conn| -> rusqlite::Result<Vec<(String, String)>> {
                let mut stmt = conn.prepare(
                    "SELECT fq_name, source_loc FROM entries WHERE plugin_id='system' \
                     AND kind='command' ORDER BY fq_name ASC",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        let fq: String = r.get(0)?;
                        let src: String = r.get(1).unwrap_or_default();
                        Ok((fq.strip_prefix("cmd:").unwrap_or(&fq).to_string(), src))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            });
            let rows = match res {
                Ok(r) => r,
                Err(e) => return Some(Err(ErrPayload::new("catalog", e.to_string()))),
            };
            let filtered: Vec<&(String, String)> = rows
                .iter()
                .filter(|(name, _)| glob_re.is_match(name))
                .collect();
            let body = match format {
                "json" => {
                    let arr: Vec<_> = filtered
                        .iter()
                        .map(|(n, p)| json!({"name": n, "path": p}))
                        .collect();
                    serde_json::to_string_pretty(&arr).unwrap_or_default()
                }
                _ => {
                    let mut s = String::new();
                    for (n, p) in &filtered {
                        s.push_str(&format!("{}\t{}\n", n, p));
                    }
                    s
                }
            };
            Some(Ok(body))
        }
        // history --filter '<query>' --range 7d: FTS query + time bound.
        "history" => {
            if filter.is_none() && range.is_none() {
                return None;
            }
            let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            let from_ns = range
                .and_then(parse_range)
                .map(|secs| now_ns - secs * 1_000_000_000);
            let limit: i64 = 10_000;
            let res = state.with_history(
                |conn| -> rusqlite::Result<Vec<super::history::HistoryRow>> {
                    super::history::query(conn, filter, "fts", None, from_ns, None, limit, true)
                },
            );
            let rows = match res {
                Ok(r) => r,
                Err(e) => return Some(Err(ErrPayload::new("history_query", e.to_string()))),
            };
            let body = match format {
                "json" => serde_json::to_string_pretty(&rows).unwrap_or_default(),
                "csv" => {
                    let mut s = String::from("ts_ns,exit_code,duration_ns,cwd,line\n");
                    for r in &rows {
                        s.push_str(&format!(
                            "{},{},{},{},{}\n",
                            r.ts_ns,
                            r.exit_code.unwrap_or(0),
                            r.duration_ns.unwrap_or(0),
                            r.cwd.as_deref().unwrap_or(""),
                            r.line.replace('\n', " ")
                        ));
                    }
                    s
                }
                _ => {
                    let mut s = String::new();
                    for r in &rows {
                        s.push_str(&format!("{}\t{}\n", r.ts_ns, r.line));
                    }
                    s
                }
            };
            Some(Ok(body))
        }
        // subscriptions --all: fan out across every shell, not just the
        // calling one. Without --all, render only the caller's view (which
        // already happens via the regular subscriptions render path).
        "subscriptions" if all_flag => {
            let subs = state.list_all_subscriptions();
            let body = match format {
                "json" => serde_json::to_string_pretty(
                    &subs
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
                        .collect::<Vec<_>>(),
                )
                .unwrap_or_default(),
                _ => {
                    let mut s = String::new();
                    for sub in &subs {
                        s.push_str(&format!(
                            "shell:{}\tid:{}\t{}\t{}\n",
                            sub.client_id,
                            sub.id,
                            sub.pattern,
                            if sub.paused { "paused" } else { "active" }
                        ));
                    }
                    s
                }
            };
            Some(Ok(body))
        }
        _ => None,
    }
}

/// Convert a shell glob (`*`, `?`, character classes) to a regex anchored at
/// both ends. Used by --filter on command_hash. Errors out on malformed glob.
fn glob_to_regex(glob: &str) -> std::result::Result<regex::Regex, String> {
    let mut re = String::with_capacity(glob.len() + 4);
    re.push('^');
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '[' => {
                re.push('[');
                while let Some(&next) = chars.peek() {
                    if next == ']' {
                        chars.next();
                        re.push(']');
                        break;
                    }
                    re.push(chars.next().unwrap());
                }
            }
            '.' | '+' | '(' | ')' | '|' | '^' | '$' | '{' | '}' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            _ => re.push(c),
        }
    }
    re.push('$');
    regex::Regex::new(&re).map_err(|e| format!("bad glob `{}`: {}", glob, e))
}

/// Parse a `--range` value like `7d`, `24h`, `30m`, `60s` into seconds.
fn parse_range(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (digits, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit())?);
    let n: i64 = digits.parse().ok()?;
    match unit {
        "s" => Some(n),
        "m" => Some(n * 60),
        "h" => Some(n * 3600),
        "d" => Some(n * 86400),
        "w" => Some(n * 86400 * 7),
        _ => None,
    }
}

/// All shell-state-eval-compatible subsystems, in the dependency order that
/// `eval $(zcache export --all-state)` expects (path before commands; fpath
/// before autoloads; aliases last so user-mutable state wins). Includes
/// the four zsh assoc-array hashes (_comps / _services / _patcomps /
/// _postpatcomps) so a single `eval` call rehydrates the full completion
/// state too.
const ALL_STATE_TARGETS: &[&str] = &[
    "setopt",   // option mask first; downstream behavior depends on this
    "zmodload", // modules before features that need them
    "path",     // PATH before command_hash + functions
    "fpath",    // FPATH before autoload_table
    "manpath",
    "named_dir",
    "env", // exported env before params
    "params",
    "zstyle",
    "bindkey",
    "compdef",
    "command_hash",
    "autoload_table",
    "functions",
    "_comps",        // assoc-array hashes — restored after compdef so the
    "_services",     //   `compdef X Y` records above are reflected here too;
    "_patcomps",     //   if the user only edits the assoc directly, this
    "_postpatcomps", // restores the post-edit state.
    "_describe_handlers",
    "aliases", // alias is last so it can shadow function/builtin
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
        "csv" => render_csv(state, target),
        "sql" => render_sql(state, target),
        "native" => render_native(state, target),
        "disasm" => render_disasm(state, target),
        "zcompdump" => render_zcompdump_format(state, target),
        "zsh-histfile" => render_zsh_histfile(state, target),
        other => Err(ErrPayload::new(
            "bad_format",
            format!(
                "format `{}` not supported (try sh|json|yaml|text|csv|sql|native|disasm|zcompdump|zsh-histfile)",
                other
            ),
        )),
    }
}

/// Render history.db rows in zsh's EXTENDED_HISTORY format
/// (`: <epoch>:<duration>;<command>`). Per docs/DAEMON.md:425
/// "Backwards-export to legacy zsh format available via
/// `zcache export history --format zsh-histfile` for round-trip."
fn render_zsh_histfile(
    state: &DaemonState,
    target: &str,
) -> std::result::Result<String, ErrPayload> {
    if target != "history" {
        return Err(ErrPayload::new(
            "bad_format",
            format!(
                "zsh-histfile only valid for target `history`, got `{}`",
                target
            ),
        ));
    }
    let rows = state
        .with_history(
            |conn| -> rusqlite::Result<Vec<super::history::HistoryRow>> {
                super::history::query(conn, None, "fts", None, None, None, 1_000_000, false)
            },
        )
        .map_err(|e| ErrPayload::new("history_query", e.to_string()))?;
    let mut out = String::new();
    for r in &rows {
        let secs = r.ts_ns / 1_000_000_000;
        let dur_secs = r.duration_ns.unwrap_or(0) / 1_000_000_000;
        // Replace embedded newlines with `\` continuation per zsh hist format.
        let line_escaped = r.line.replace('\n', "\\\n");
        out.push_str(&format!(": {}:{};{}\n", secs, dur_secs, line_escaped));
    }
    Ok(out)
}

fn read_canonical(
    state: &DaemonState,
    subsystem: &str,
) -> std::result::Result<Vec<CanonicalRow>, ErrPayload> {
    // rkyv-backed in-memory state is the source of truth — SQLite is only a
    // hydrated view target (refreshed by `zcache hydrate-view`).
    Ok(super::zsync::read_canonical_rows_inmem(state, subsystem))
}

/// Render a zsh assoc-array hash from a canonical subsystem. Output:
///
///   typeset -gA <name>
///   <name>=(
///   'k1' 'v1'
///   'k2' 'v2'
///   ...
///   )
///
/// On `--additive` the wipe is suppressed (the `=( ... )` literal already
/// reset-replaces the array); the comment switches to additive form
/// (insertions only via `<name>+=(...)`).
fn push_zsh_assoc_hash(
    out: &mut String,
    array_name: &str,
    subsystem: &str,
    state: &DaemonState,
    additive: bool,
) -> std::result::Result<(), ErrPayload> {
    let rows = read_canonical(state, subsystem)?;
    out.push_str(&format!("typeset -gA {}\n", array_name));
    if additive {
        out.push_str(&format!("{}+=(\n", array_name));
    } else {
        out.push_str(&format!("{}=(\n", array_name));
    }
    for r in &rows {
        let v = unjson(&r.value);
        out.push_str(&format!(
            "'{}' '{}'\n",
            zsh_singlequote_escape(&r.key),
            zsh_singlequote_escape(&v)
        ));
    }
    out.push_str(")\n");
    Ok(())
}

/// Escape `'` inside a zsh single-quoted string per the standard `'\''`
/// closing/escaping/opening trick. Other characters are emitted verbatim.
fn zsh_singlequote_escape(s: &str) -> String {
    s.replace('\'', "'\\''")
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
        // The zsh associative-array hashes themselves: emit as
        //   typeset -gA NAME
        //   NAME=( 'k1' 'v1' 'k2' 'v2' ... )
        // so `eval $(zcache export _comps)` populates the running shell's
        // _comps directly. _comps is keyed off the canonical "compdef"
        // subsystem (one source of truth — same data as what `compdef`
        // shell-builtin records). The other three (_services, _patcomps,
        // _postpatcomps) live under their own subsystems populated by
        // `zcache import zcompdump`.
        "_comps" => {
            push_zsh_assoc_hash(&mut out, "_comps", "compdef", state, additive)?;
        }
        "_services" => {
            push_zsh_assoc_hash(&mut out, "_services", "service", state, additive)?;
        }
        "_patcomps" => {
            push_zsh_assoc_hash(&mut out, "_patcomps", "patcomp", state, additive)?;
        }
        "_postpatcomps" => {
            push_zsh_assoc_hash(&mut out, "_postpatcomps", "postpatcomp", state, additive)?;
        }
        "_describe_handlers" => {
            push_zsh_assoc_hash(
                &mut out,
                "_describe_handlers",
                "describe_handler",
                state,
                additive,
            )?;
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
            let mut names: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
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
            // "function"; emit as `function name { body }`. To keep the output
            // parser-safe even when the captured body has unbalanced braces /
            // unquoted special characters (the analyzer is regex-based and
            // can't always extract clean balanced bodies), we wrap each one
            // in an `eval`-guarded heredoc-style block that zsh parses
            // verbatim. This way a downstream `eval $(zcache export
            // functions)` doesn't fail on a single bad capture; it just
            // skips the bad row at runtime.
            let func_rows = read_canonical(state, "function")?;
            for r in &func_rows {
                let body = unjson(&r.value);
                // If the body looks balanced (count of `{` == `}`), emit the
                // direct form. Otherwise emit a here-doc that becomes the
                // body, which preserves arbitrary content safely.
                let opens = body.matches('{').count();
                let closes = body.matches('}').count();
                if opens == closes {
                    out.push_str(&format!("function {} {{\n{}\n}}\n", r.key, body));
                } else {
                    // Heredoc-defined body — zsh parses the entire heredoc
                    // verbatim, so unbalanced braces inside don't trip the
                    // parser. The `eval` is needed because `function name {}`
                    // is a parser construct, not a runtime expression.
                    out.push_str(&format!(
                        "eval \"function {} {{ $(cat <<'__ZSHRS_FN_END__'\n{}\n__ZSHRS_FN_END__\n) }}\"\n",
                        r.key, body
                    ));
                }
            }
        }
        // Binary / introspection-only targets refuse sh format. (The
        // assoc-array hashes _comps / _services / _patcomps /
        // _postpatcomps / _describe_handlers were here pre-eval-output;
        // they're handled above now.)
        "shard" | "index" | "catalog" | "history" | "entry_stats" | "subscriptions" | "shells"
        | "plugins" | "compiled_files" | "daemon_state" | "theme" | "zcompdump" | "script"
        | "sourced" => {
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
        "history" => {
            // Read directly from history.db. Recent rows first, capped at 10k.
            let rows = state
                .with_history(|conn| {
                    super::history::query(conn, None, "match", None, None, None, 10000, true)
                })
                .map_err(|e: rusqlite::Error| ErrPayload::new("history_query", e.to_string()))?;
            let payload: Vec<Value> = rows
                .into_iter()
                .map(|r| serde_json::to_value(&r).unwrap_or(Value::Null))
                .collect();
            return Ok(serde_json::to_string_pretty(&payload).unwrap_or_default());
        }
        "index" => {
            // Top-level index.rkyv: one entry per shard with slug, generation,
            // entry_count, byte_size. Per docs/DAEMON.md "Cache layout" line
            // 192. If the file is missing (cold cache before first_init),
            // returns a "not_built" status so the caller can react.
            let idx = super::shard::read_index(&state.paths)
                .map_err(|e| ErrPayload::new("read_index", e.to_string()))?;
            if idx.entries.is_empty() && idx.generation == 0 {
                return Ok(json!({
                    "status": "not_built",
                    "path": state.paths.index_rkyv.display().to_string(),
                    "note": "run `zcache rebuild` or `zcache first-init` to populate",
                })
                .to_string());
            }
            let payload = json!({
                "magic": format!("{:#x}", idx.magic),
                "format_version": idx.format_version,
                "generation": idx.generation,
                "built_at_ns": idx.built_at_ns,
                "entries": idx.entries.iter().map(|e| json!({
                    "slug": e.slug,
                    "source_root": e.source_root,
                    "generation": e.generation,
                    "built_at_ns": e.built_at_ns,
                    "entry_count": e.entry_count,
                    "byte_size": e.byte_size,
                    "path": e.path,
                })).collect::<Vec<_>>(),
                "entry_total": idx.entries.len(),
            });
            return Ok(serde_json::to_string_pretty(&payload).unwrap_or_default());
        }
        "theme" => {
            // Theme resolution (PROMPT/RPROMPT segment evaluation) is not
            // wired in the daemon yet — depends on the prompt-renderer
            // integration. Returns a status stub so callers can detect.
            let payload = json!({
                "status": "not_implemented",
                "note": "theme resolution lives shell-side; no canonical state yet"
            });
            return Ok(serde_json::to_string_pretty(&payload).unwrap_or_default());
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
    // Assoc-array hashes (_comps / _services / _patcomps / _postpatcomps /
    // _describe_handlers) read from their underlying canonical subsystem.
    if let Some(sub) = assoc_subsystem_for(target) {
        let rows = read_canonical(state, sub)?;
        let payload: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "key": r.key,
                    "value": serde_json::from_str::<Value>(&r.value)
                        .unwrap_or(Value::String(r.value.clone()))
                })
            })
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

/// CSV renderer — tabular for history / entry_stats / shells / plugins (per
/// docs/DAEMON.md format table). Falls back to a generic "key,value" CSV
/// for canonical-state targets so it works for everything.
fn render_csv(state: &DaemonState, target: &str) -> std::result::Result<String, ErrPayload> {
    fn esc(s: &str) -> String {
        // RFC 4180-ish — wrap in quotes if comma/quote/newline; double-quote
        // embedded `"`.
        if s.contains(',') || s.contains('"') || s.contains('\n') {
            format!("\"{}\"", s.replace('"', "\"\""))
        } else {
            s.to_string()
        }
    }
    match target {
        "history" => {
            let rows = state
                .with_history(|conn| {
                    super::history::query(conn, None, "match", None, None, None, 100_000, true)
                })
                .map_err(|e: rusqlite::Error| ErrPayload::new("history_query", e.to_string()))?;
            let mut out =
                String::from("id,line,ts_ns,exit_code,cwd,duration_ns,sessid,hostname,shell_id\n");
            for r in rows {
                out.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{}\n",
                    r.id,
                    esc(&r.line),
                    r.ts_ns,
                    r.exit_code.map(|c| c.to_string()).unwrap_or_default(),
                    r.cwd.as_deref().map(esc).unwrap_or_default(),
                    r.duration_ns.map(|d| d.to_string()).unwrap_or_default(),
                    r.sessid.as_deref().map(esc).unwrap_or_default(),
                    r.hostname.as_deref().map(esc).unwrap_or_default(),
                    r.shell_id.map(|s| s.to_string()).unwrap_or_default(),
                ));
            }
            Ok(out)
        }
        "entry_stats" => {
            let rows: Vec<(String, Option<i64>, i64, i64)> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT fq_name, last_called_at, call_count, total_ns FROM entry_stats \
                         ORDER BY call_count DESC, fq_name ASC",
                    )?;
                    let rows = stmt
                        .query_map([], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, Option<i64>>(1)?,
                                r.get::<_, i64>(2)?,
                                r.get::<_, i64>(3)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            let mut out = String::from("fq_name,last_called_at,call_count,total_ns\n");
            for (fq, last, count, total) in rows {
                out.push_str(&format!(
                    "{},{},{},{}\n",
                    esc(&fq),
                    last.map(|l| l.to_string()).unwrap_or_default(),
                    count,
                    total
                ));
            }
            Ok(out)
        }
        "shells" => {
            let mut out = String::from(
                "client_id,session_id,pid,tty,cwd,argv0,tags,login_time,uptime_secs\n",
            );
            for s in state.snapshot_sessions() {
                out.push_str(&format!(
                    "{},{},{},{},{},{},{},{},{}\n",
                    s.client_id,
                    esc(&s.session_id),
                    s.pid,
                    s.tty.as_deref().map(esc).unwrap_or_default(),
                    s.cwd.as_deref().map(esc).unwrap_or_default(),
                    s.argv0.as_deref().map(esc).unwrap_or_default(),
                    esc(&s.tags.join("|")),
                    esc(&s.login_time),
                    s.uptime_secs,
                ));
            }
            Ok(out)
        }
        "plugins" => {
            let rows: Vec<(
                String,
                Option<String>,
                Option<String>,
                Option<i64>,
                Option<bool>,
            )> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT name, version, source, installed_at, enabled FROM plugins \
                             ORDER BY name",
                    )?;
                    let rows = stmt
                        .query_map([], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, Option<String>>(1)?,
                                r.get::<_, Option<String>>(2)?,
                                r.get::<_, Option<i64>>(3)?,
                                r.get::<_, Option<bool>>(4)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            let mut out = String::from("name,version,source,installed_at,enabled\n");
            for (n, v, s, t, e) in rows {
                out.push_str(&format!(
                    "{},{},{},{},{}\n",
                    esc(&n),
                    v.as_deref().map(esc).unwrap_or_default(),
                    s.as_deref().map(esc).unwrap_or_default(),
                    t.map(|t| t.to_string()).unwrap_or_default(),
                    e.map(|b| b.to_string()).unwrap_or_default(),
                ));
            }
            Ok(out)
        }
        // Generic CSV for any canonical-state target.
        _ => {
            let subsystem = normalize_subsystem(target)?;
            let rows = read_canonical(state, &subsystem)?;
            let mut out = String::from("key,value\n");
            for r in rows {
                out.push_str(&format!("{},{}\n", esc(&r.key), esc(&unjson(&r.value))));
            }
            Ok(out)
        }
    }
}

/// SQL renderer — INSERT statements suitable for replaying into a fresh
/// catalog.db / history.db. Spec'd for catalog/entries/entry_stats/plugins/
/// history.
fn render_sql(state: &DaemonState, target: &str) -> std::result::Result<String, ErrPayload> {
    fn sql_esc(s: &str) -> String {
        format!("'{}'", s.replace('\'', "''"))
    }
    match target {
        "entries" => {
            let rows: Vec<(String, String, String, String, i64, String)> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT fq_name, plugin_id, kind, image_path, byte_offset, source_loc FROM entries",
                    )?;
                    let rows = stmt
                        .query_map([], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, String>(1)?,
                                r.get::<_, String>(2)?,
                                r.get::<_, String>(3)?,
                                r.get::<_, i64>(4)?,
                                r.get::<_, String>(5).unwrap_or_default(),
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            let mut out = String::new();
            for (fq, pid, kind, ipath, off, src) in rows {
                out.push_str(&format!(
                    "INSERT INTO entries(fq_name,plugin_id,kind,image_path,byte_offset,source_loc) VALUES ({},{},{},{},{},{});\n",
                    sql_esc(&fq), sql_esc(&pid), sql_esc(&kind), sql_esc(&ipath), off, sql_esc(&src)
                ));
            }
            Ok(out)
        }
        "entry_stats" => {
            let rows: Vec<(String, Option<i64>, i64, i64)> = state
                .with_catalog(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT fq_name, last_called_at, call_count, total_ns FROM entry_stats",
                    )?;
                    let rows = stmt
                        .query_map([], |r| {
                            Ok((
                                r.get::<_, String>(0)?,
                                r.get::<_, Option<i64>>(1)?,
                                r.get::<_, i64>(2)?,
                                r.get::<_, i64>(3)?,
                            ))
                        })?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    Ok::<_, rusqlite::Error>(rows)
                })
                .map_err(ErrPayload::from)?;
            let mut out = String::new();
            for (fq, last, count, total) in rows {
                out.push_str(&format!(
                    "INSERT INTO entry_stats(fq_name,last_called_at,call_count,total_ns) VALUES ({},{},{},{});\n",
                    sql_esc(&fq),
                    last.map(|l| l.to_string()).unwrap_or_else(|| "NULL".into()),
                    count,
                    total
                ));
            }
            Ok(out)
        }
        "plugins" => {
            let rows: Vec<(String, Option<String>, Option<String>, Option<i64>, Option<bool>)> =
                state
                    .with_catalog(|conn| {
                        let mut stmt = conn.prepare(
                            "SELECT name, version, source, installed_at, enabled FROM plugins",
                        )?;
                        let rows = stmt
                            .query_map([], |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, Option<String>>(1)?,
                                    r.get::<_, Option<String>>(2)?,
                                    r.get::<_, Option<i64>>(3)?,
                                    r.get::<_, Option<bool>>(4)?,
                                ))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?;
                        Ok::<_, rusqlite::Error>(rows)
                    })
                    .map_err(ErrPayload::from)?;
            let mut out = String::new();
            for (n, v, s, t, e) in rows {
                out.push_str(&format!(
                    "INSERT INTO plugins(name,version,source,installed_at,enabled) VALUES ({},{},{},{},{});\n",
                    sql_esc(&n),
                    v.as_deref().map(sql_esc).unwrap_or_else(|| "NULL".into()),
                    s.as_deref().map(sql_esc).unwrap_or_else(|| "NULL".into()),
                    t.map(|t| t.to_string()).unwrap_or_else(|| "NULL".into()),
                    e.map(|b| if b { "1".to_string() } else { "0".to_string() })
                        .unwrap_or_else(|| "NULL".to_string()),
                ));
            }
            Ok(out)
        }
        "history" => {
            let rows = state
                .with_history(|conn| {
                    super::history::query(conn, None, "match", None, None, None, 100_000, true)
                })
                .map_err(|e: rusqlite::Error| ErrPayload::new("history_query", e.to_string()))?;
            let mut out = String::new();
            for r in rows {
                out.push_str(&format!(
                    "INSERT INTO history(line,ts_ns,exit_code,cwd,duration_ns,sessid,hostname,shell_id) VALUES ({},{},{},{},{},{},{},{});\n",
                    sql_esc(&r.line),
                    r.ts_ns,
                    r.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "NULL".into()),
                    r.cwd.as_deref().map(sql_esc).unwrap_or_else(|| "NULL".into()),
                    r.duration_ns.map(|d| d.to_string()).unwrap_or_else(|| "NULL".into()),
                    r.sessid.as_deref().map(sql_esc).unwrap_or_else(|| "NULL".into()),
                    r.hostname.as_deref().map(sql_esc).unwrap_or_else(|| "NULL".into()),
                    r.shell_id.map(|s| s.to_string()).unwrap_or_else(|| "NULL".into()),
                ));
            }
            Ok(out)
        }
        "catalog" => {
            // Compose all catalog tables together — entries + entry_stats + plugins.
            let mut out = String::new();
            out.push_str(&render_sql(state, "entries")?);
            out.push_str(&render_sql(state, "entry_stats")?);
            out.push_str(&render_sql(state, "plugins")?);
            Ok(out)
        }
        other => Err(ErrPayload::new(
            "format_unsupported_for_target",
            format!(
                "sql format not supported for `{}` (try catalog|entries|entry_stats|plugins|history)",
                other
            ),
        )),
    }
}

/// Native rkyv-binary renderer — returns a base64-encoded payload of the
/// underlying shard bytes. Default for binary-only targets (shard, index,
/// catalog) per docs/DAEMON.md format table. For shell-state targets,
/// returns the canonical-state shard as the binary representation.
fn render_native(state: &DaemonState, target: &str) -> std::result::Result<String, ErrPayload> {
    let path = match target {
        "shard" | "index" | "catalog" => {
            return Err(ErrPayload::new(
                "format_unsupported_for_target",
                format!(
                    "native format on `{}` requires the dedicated op (export_shard / export_catalog) — use `zcache export {}` (default routes there)",
                    target, target
                ),
            ));
        }
        _ => {
            // Default: serve the master canonical-state shard, which carries
            // every shell-state subsystem.
            super::shard::shard_path(&state.paths, "user-overlay", "promotions")
        }
    };
    if !path.exists() {
        return Err(ErrPayload::new(
            "no_shard",
            format!("shard `{}` not built yet", path.display()),
        ));
    }
    let bytes = std::fs::read(&path).map_err(ErrPayload::from)?;
    // Emit base64 — this format is meant for programmatic consumers (e.g.
    // `curl … | base64 -d > shard.rkyv`). Not meant for terminal viewing.
    Ok(base64_encode(&bytes))
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = if chunk.len() > 1 { chunk[1] } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] } else { 0 };
        let n = ((b0 as u32) << 16) | ((b1 as u32) << 8) | (b2 as u32);
        out.push(ALPHA[((n >> 18) & 0x3f) as usize] as char);
        out.push(ALPHA[((n >> 12) & 0x3f) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHA[((n >> 6) & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHA[(n & 0x3f) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// Disassembled-bytecode renderer for function/script/shard targets.
/// Bytecode emitter waits for the parser-in-daemon work; emit a
/// stable stub now so callers can wire a `--format disasm` flag without
/// it breaking.
fn render_disasm(state: &DaemonState, target: &str) -> std::result::Result<String, ErrPayload> {
    match target {
        "function" | "functions" => {
            let rows = read_canonical(state, "function")?;
            let mut out = String::new();
            for r in &rows {
                let body = unjson(&r.value);
                out.push_str(&format!(
                    "; --- function: {} ---\n; (bytecode disassembly not yet wired; v1 \
                     stores source bytes — emitter arrives with the parser-in-daemon work)\n; source:\n",
                    r.key
                ));
                for line in body.lines() {
                    out.push_str(&format!(";   {}\n", line));
                }
                out.push('\n');
            }
            Ok(out)
        }
        "script" | "sourced" | "shard" => Ok(format!(
            "; --- {} disasm ---\n; (bytecode disassembly not yet wired; v1 stores source bytes\n; — emitter arrives with the parser-in-daemon work)\n",
            target
        )),
        other => Err(ErrPayload::new(
            "format_unsupported_for_target",
            format!(
                "disasm format on `{}` not applicable (try function|functions|script|sourced|shard)",
                other
            ),
        )),
    }
}

/// `--format zcompdump` — combined assoc-array dump in compinit's wire form.
/// Valid for compdef / _comps / _services / _patcomps / _describe_handlers
/// (any of those target names emits the FULL combined dump, not just one
/// section). Mirrors the round-trip path of the `zcompdump` target.
fn render_zcompdump_format(
    state: &DaemonState,
    target: &str,
) -> std::result::Result<String, ErrPayload> {
    let valid = matches!(
        target,
        "compdef"
            | "_comps"
            | "_services"
            | "_patcomps"
            | "_postpatcomps"
            | "_describe_handlers"
            | "zcompdump"
    );
    if !valid {
        return Err(ErrPayload::new(
            "format_unsupported_for_target",
            format!(
                "zcompdump format only valid for compdef / _comps / _services / _patcomps / _postpatcomps / _describe_handlers / zcompdump (got `{}`)",
                target
            ),
        ));
    }
    // Round-trip path (preferred): if a raw .zcompdump body was imported,
    // emit it verbatim. Otherwise synthesize.
    let raw = state
        .canonical
        .row("zcompdump_raw", "body")
        .and_then(|r| serde_json::from_str::<serde_json::Value>(&r.value).ok())
        .and_then(|v| v.as_str().map(|s| s.to_string()));
    if let Some(s) = raw {
        return Ok(s);
    }
    // Fall back to the structured synthesizer (lives next to op_export_zcompdump
    // in ops.rs — duplicate the format here so this renderer is self-contained).
    let mut out = String::new();
    let zsh_version = std::env::var("ZSH_VERSION").unwrap_or_else(|_| "5.9".to_string());
    let comps = read_canonical(state, "compdef")?;
    let services = read_canonical(state, "service")?;
    let patcomps = read_canonical(state, "patcomp")?;
    let postpatcomps = read_canonical(state, "postpatcomp")?;
    let autoloads = read_canonical(state, "autoload_completion")?;
    out.push_str(&format!(
        "#files: {}\tversion: {}\n\n",
        comps.len(),
        zsh_version
    ));
    let push_assoc = |out: &mut String, name: &str, rows: &[CanonicalRow]| {
        out.push_str(&format!("{}=(\n", name));
        for r in rows {
            out.push_str(&format!("'{}' '{}'\n", r.key, unjson(&r.value)));
        }
        out.push_str(")\n\n");
    };
    push_assoc(&mut out, "_comps", &comps);
    push_assoc(&mut out, "_services", &services);
    push_assoc(&mut out, "_patcomps", &patcomps);
    push_assoc(&mut out, "_postpatcomps", &postpatcomps);
    if !autoloads.is_empty() {
        out.push_str("\nautoload -Uz ");
        for (i, r) in autoloads.iter().enumerate() {
            if i > 0 && i % 5 == 0 {
                out.push_str("\\\n            ");
            } else if i > 0 {
                out.push(' ');
            }
            out.push_str(&r.key);
        }
        out.push('\n');
    }
    out.push_str("\ntypeset -gUa _comp_assocs\n_comp_assocs=( '' )\n");
    Ok(out)
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
    // history / index / theme / catalog-derived targets — re-use the JSON
    // renderer's data path and pretty-print it as text. Avoids reaching the
    // canonical-fall-through which would print "0 entries" for these.
    match target {
        "history" | "index" | "theme" | "shells" | "subscriptions" | "daemon_state"
        | "entry_stats" | "plugins" | "compiled_files" | "script" | "sourced" => {
            let json_body = render_json(state, target)?;
            return Ok(json_body);
        }
        _ => {}
    }
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
    // Assoc-array hashes read from canonical (text view).
    if let Some(sub) = assoc_subsystem_for(target) {
        let rows = read_canonical(state, sub)?;
        let mut out = String::new();
        out.push_str(&format!("# target: {}  (canonical: {})\n", target, sub));
        out.push_str(&format!("# {} entries\n\n", rows.len()));
        for r in &rows {
            out.push_str(&format!("{} = {}\n", r.key, unjson(&r.value)));
        }
        return Ok(Some(out));
    }
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

/// Map a `zcache view` / `zcache export` target name to the canonical
/// subsystem when the assoc-array form is renderable in JSON / text by
/// reading rows directly. Used so `zcache view _comps --format json`
/// returns the {key,value} list without needing a hand-rolled handler per
/// target.
fn assoc_subsystem_for(target: &str) -> Option<&'static str> {
    match target {
        "_comps" => Some("compdef"),
        "_services" => Some("service"),
        "_patcomps" => Some("patcomp"),
        "_postpatcomps" => Some("postpatcomp"),
        "_describe_handlers" => Some("describe_handler"),
        _ => None,
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

/// Render a daemon-state text dump as a paginated PDF document. Used by
/// `op_view_or_export` when `format=pdf`. The text body is the same one
/// the existing `text` renderer produces (one line per row), so the PDF
/// is "the data, paginated" rather than a designed report layout.
///
/// Page model: US Letter, 0.5-inch margins, monospace 10pt
/// (Helvetica because that's what built-in printpdf fonts cover; the
/// monospace look comes from fixed line height, not the glyph
/// metrics — fine for the dump-style content).
fn pdf_render(target: &str, body: &str) -> std::result::Result<Vec<u8>, String> {
    use printpdf::{BuiltinFont, Mm, PdfDocument};

    // Letter: 215.9mm × 279.4mm. Margins: 12.7mm (~0.5in). printpdf
    // 0.7's `Mm` is `f32`, so all geometry constants are f32 to avoid
    // a sea of `as f32` at every call site.
    const PAGE_W: f32 = 215.9;
    const PAGE_H: f32 = 279.4;
    const MARGIN: f32 = 12.7;
    const FONT_PT: f32 = 10.0;
    // 1pt ≈ 0.3528mm; 12pt line height for 10pt font.
    const LINE_H: f32 = 4.23;
    const HEADER_LINES: usize = 3;
    const MAX_LINE_CHARS: usize = 110;

    let usable_h = PAGE_H - 2.0 * MARGIN;
    let lines_per_page = ((usable_h / LINE_H) as usize).saturating_sub(HEADER_LINES);
    if lines_per_page == 0 {
        return Err("page geometry too small for content".to_string());
    }

    let title = format!("zshrs-daemon export — {target}");
    let timestamp = chrono::Utc::now().to_rfc3339();
    let footer_template = format!(
        "Generated {timestamp} • daemon v{version}",
        version = env!("CARGO_PKG_VERSION")
    );

    // Pre-wrap content into display lines so paging is deterministic.
    let mut display_lines: Vec<String> = Vec::new();
    for raw in body.lines() {
        if raw.chars().count() <= MAX_LINE_CHARS {
            display_lines.push(raw.to_string());
        } else {
            // Hard wrap at MAX_LINE_CHARS; preserves all content even on
            // pathologically long lines (long function bodies, etc.).
            let mut buf = String::new();
            for c in raw.chars() {
                buf.push(c);
                if buf.chars().count() >= MAX_LINE_CHARS {
                    display_lines.push(buf.clone());
                    buf.clear();
                }
            }
            if !buf.is_empty() {
                display_lines.push(buf);
            }
        }
    }
    if display_lines.is_empty() {
        display_lines.push(String::from("(empty)"));
    }

    let total_pages = display_lines.len().div_ceil(lines_per_page).max(1);
    let (doc, first_page, first_layer) =
        PdfDocument::new(&title, Mm(PAGE_W), Mm(PAGE_H), "page-1");
    let font = doc
        .add_builtin_font(BuiltinFont::Helvetica)
        .map_err(|e| format!("printpdf font: {e}"))?;
    let bold = doc
        .add_builtin_font(BuiltinFont::HelveticaBold)
        .map_err(|e| format!("printpdf bold font: {e}"))?;

    for (page_idx, chunk) in display_lines.chunks(lines_per_page).enumerate() {
        let layer = if page_idx == 0 {
            doc.get_page(first_page).get_layer(first_layer)
        } else {
            let (p, l) = doc.add_page(Mm(PAGE_W), Mm(PAGE_H), format!("page-{}", page_idx + 1));
            doc.get_page(p).get_layer(l)
        };

        // Header (title + page number).
        let mut y = PAGE_H - MARGIN;
        layer.use_text(&title, FONT_PT + 2.0, Mm(MARGIN), Mm(y), &bold);
        let pageno = format!("page {} / {}", page_idx + 1, total_pages);
        layer.use_text(
            &pageno,
            FONT_PT,
            Mm(PAGE_W - MARGIN - 30.0),
            Mm(y),
            &font,
        );
        y -= LINE_H * 2.0;

        // Body lines.
        for line in chunk {
            layer.use_text(line.as_str(), FONT_PT, Mm(MARGIN), Mm(y), &font);
            y -= LINE_H;
        }

        // Footer.
        layer.use_text(
            &footer_template,
            FONT_PT - 2.0,
            Mm(MARGIN),
            Mm(MARGIN / 2.0),
            &font,
        );
    }

    let mut buf: Vec<u8> = Vec::new();
    doc.save(&mut std::io::BufWriter::new(&mut buf))
        .map_err(|e| format!("printpdf save: {e}"))?;
    Ok(buf)
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
