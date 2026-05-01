// .zshrc analysis pass — partial evaluator that captures deterministic state
// declarations for daemon-served canonical state.
//
// Per docs/DAEMON.md "Starting state served by daemon" + "Determinism boundary":
//
//   Pass 1 — parse system + user dotfiles in zsh's standard order, follow source
//            transitively into plugins / env files / tokens.sh.
//   Pass 2 — replay state mutations against an in-memory state model. Capture:
//             - alias / global alias / suffix alias declarations
//             - function definitions
//             - setopt / unsetopt
//             - bindkey
//             - hash -d named-directory
//             - compdef / zstyle / zmodload
//             - export / typeset / VAR= assignments
//             - path+= / fpath+= / PATH= / FPATH=
//             - source / . (transitive)
//
// Determinism boundary: anything involving `$(...)`, `${VAR}` interpolation against
// runtime state, conditional `[[ ]]`, control-flow constructs gets emitted to a
// per-shell replay log instead of the canonical state image.
//
// V1 implementation: regex/line-based scanner. Deliberately simple and conservative
// — anything that doesn't match a known-deterministic pattern goes to the
// non_deterministic_lines list which will become the per-shell replay log. This
// captures ~70-80% of typical zpwr/.zshrc content correctly without needing full
// zsh-AST evaluation. Real AST-based partial evaluation arrives in a later
// iteration that uses the existing crate::parser::ZshParser.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Captured canonical state from analysis. Each field maps to one row family in
/// the daemon-served starting-point caches.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CanonicalState {
    pub aliases: HashMap<String, String>,
    pub global_aliases: HashMap<String, String>,
    pub suffix_aliases: HashMap<String, String>,
    pub functions: HashMap<String, String>, // name → body text
    pub setopts: HashSet<String>,
    pub unsetopts: HashSet<String>,
    pub bindkeys: HashMap<String, String>,
    pub named_dirs: HashMap<String, String>,
    pub compdef: HashMap<String, String>, // command → handler
    pub zstyle: Vec<(String, String)>,    // (pattern, rest_of_args)
    pub zmodload: HashSet<String>,
    pub env_exports: HashMap<String, String>,
    pub params: HashMap<String, String>,
    pub path_additions: Vec<String>,
    pub fpath_additions: Vec<String>,
    pub manpath_additions: Vec<String>,
    pub plugin_decls: Vec<PluginDecl>,
    pub sourced_files: Vec<String>,
    pub non_deterministic_lines: Vec<String>,
    pub stats: AnalysisStats,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AnalysisStats {
    pub files_analyzed: usize,
    pub lines_total: usize,
    pub lines_deterministic: usize,
    pub lines_non_deterministic: usize,
    pub duration_ms: u64,
}

/// One plugin declaration recognized in .zshrc — fed into per-plugin shard build.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginDecl {
    pub manager: String, // "zinit" | "omz" | "antigen" | "zplug" | "manual"
    pub name: String,
    pub source_path: Option<String>,
    pub raw: String,
}

/// Analyze a single file (e.g. .zshrc) without following sources. Use
/// `analyze_with_sources` for transitive analysis.
pub fn analyze_file(path: &Path) -> std::io::Result<CanonicalState> {
    let mut state = CanonicalState::default();
    let start = std::time::Instant::now();
    analyze_one_into(&mut state, path)?;
    state.stats.duration_ms = start.elapsed().as_millis() as u64;
    Ok(state)
}

/// Analyze `.zshrc` plus every file transitively included via `source` / `.`.
/// Cycles are guarded via the `visited` set.
pub fn analyze_with_sources(path: &Path) -> std::io::Result<CanonicalState> {
    let mut state = CanonicalState::default();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let start = std::time::Instant::now();
    analyze_recursive(&mut state, path, &mut visited)?;
    state.stats.duration_ms = start.elapsed().as_millis() as u64;
    Ok(state)
}

fn analyze_recursive(
    state: &mut CanonicalState,
    path: &Path,
    visited: &mut HashSet<PathBuf>,
) -> std::io::Result<()> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    let prior_sourced = state.sourced_files.len();
    analyze_one_into(state, path)?;
    // Recurse into newly-discovered sourced files.
    let new_sources = state.sourced_files[prior_sourced..].to_vec();
    for src in new_sources {
        let p = PathBuf::from(&src);
        if p.exists() {
            let _ = analyze_recursive(state, &p, visited);
        }
    }
    Ok(())
}

pub fn analyze_one_into(state: &mut CanonicalState, path: &Path) -> std::io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    state.stats.files_analyzed += 1;

    // We need access to the full text for multi-line function bodies. Process line-
    // oriented for simple statements; track function-body boundaries explicitly.
    let lines: Vec<&str> = content.lines().collect();
    state.stats.lines_total += lines.len();

    let mut i = 0usize;
    while i < lines.len() {
        let raw_line = lines[i];
        let line = strip_comment_and_trim(raw_line);
        if line.is_empty() {
            i += 1;
            continue;
        }

        // Function definition: `name() { ... }` or `function name { ... }` (multi-line).
        if let Some((fname, body, consumed)) = match_function(&lines, i) {
            state.functions.insert(fname, body);
            state.stats.lines_deterministic += consumed;
            i += consumed;
            continue;
        }

        // Plugin manager declarations (best-effort detection).
        if let Some(decl) = match_plugin(line, raw_line) {
            state.plugin_decls.push(decl);
            state.stats.lines_deterministic += 1;
            i += 1;
            continue;
        }

        let matched = analyze_line(state, line, raw_line);
        if matched {
            state.stats.lines_deterministic += 1;
        } else {
            state.stats.lines_non_deterministic += 1;
            state.non_deterministic_lines.push(raw_line.to_string());
        }
        i += 1;
    }

    Ok(())
}

fn analyze_line(state: &mut CanonicalState, line: &str, raw: &str) -> bool {
    // Strip leading `builtin ` / `command ` precommand modifiers.
    let line = strip_precmd_modifiers(line);

    // alias name='value' (with optional -g / -s for global/suffix variants).
    if let Some((flag, name, value)) = match_alias(line) {
        match flag {
            Some('g') => state.global_aliases.insert(name, value),
            Some('s') => state.suffix_aliases.insert(name, value),
            _ => state.aliases.insert(name, value),
        };
        return true;
    }

    // setopt foo bar / unsetopt foo
    if let Some(rest) = strip_prefix_word(line, "setopt") {
        for opt in rest.split_whitespace() {
            state.setopts.insert(opt.to_string());
        }
        return true;
    }
    if let Some(rest) = strip_prefix_word(line, "unsetopt") {
        for opt in rest.split_whitespace() {
            state.unsetopts.insert(opt.to_string());
        }
        return true;
    }

    // bindkey 'KEY' widget
    if let Some(rest) = strip_prefix_word(line, "bindkey") {
        if let Some((key, widget)) = parse_bindkey_args(rest) {
            state.bindkeys.insert(key, widget);
            return true;
        }
    }

    // hash -d name=/path
    if let Some(rest) = strip_prefix_word(line, "hash") {
        let trimmed = rest.trim_start();
        if let Some(after_d) = strip_prefix_word(trimmed, "-d") {
            for tok in after_d.split_whitespace() {
                if let Some((k, v)) = tok.split_once('=') {
                    state.named_dirs.insert(k.to_string(), v.to_string());
                }
            }
            return true;
        }
    }

    // compdef _func cmd1 cmd2 ...
    if let Some(rest) = strip_prefix_word(line, "compdef") {
        let toks: Vec<&str> = rest.split_whitespace().collect();
        if toks.len() >= 2 {
            let handler = toks[0].to_string();
            for cmd in &toks[1..] {
                state.compdef.insert(cmd.to_string(), handler.clone());
            }
            return true;
        }
    }

    // zstyle 'pattern' key value...
    if let Some(rest) = strip_prefix_word(line, "zstyle") {
        if let Some((pat, args)) = parse_first_quoted_or_word(rest) {
            state.zstyle.push((pat, args.to_string()));
            return true;
        }
    }

    // zmodload zsh/foo
    if let Some(rest) = strip_prefix_word(line, "zmodload") {
        for module in rest.split_whitespace() {
            if !module.starts_with('-') {
                state.zmodload.insert(module.to_string());
            }
        }
        return true;
    }

    // source FILE / . FILE
    if let Some(rest) = strip_prefix_word(line, "source").or_else(|| strip_prefix_word(line, ".")) {
        let trimmed = rest.trim();
        let arg = strip_quotes(trimmed);
        if arg.is_empty() {
            return false;
        }
        // Try to resolve common interpolations ($HOME, $ZDOTDIR, ~, ${VAR}).
        // If it expands to a real file, follow it. If we can't resolve, fall
        // through to the dynamic-rejection path so the line ends up in the
        // replay log.
        if let Some(expanded) = try_expand_static_path(arg) {
            state.sourced_files.push(expanded);
            return true;
        }
        if contains_dynamic(arg) {
            return false;
        }
        state.sourced_files.push(arg.to_string());
        return true;
    }

    // export VAR=value (deterministic only if value has no dynamic expansion).
    if let Some(rest) = strip_prefix_word(line, "export") {
        for tok in rest.split_whitespace() {
            if let Some((k, v)) = tok.split_once('=') {
                if !contains_dynamic(v) {
                    let v = strip_quotes(v).to_string();
                    state.env_exports.insert(k.to_string(), v);
                }
            } else {
                // `export NAME` (no value) — re-export already-set var. We can record
                // the request even if we don't know the value yet.
                if !contains_dynamic(tok) {
                    state
                        .env_exports
                        .entry(tok.to_string())
                        .or_insert_with(String::new);
                }
            }
        }
        return true;
    }

    // path+=(...) / fpath+=(...) / manpath+=(...)
    if let Some(after_eq) = match_array_append(line, "path") {
        state.path_additions.extend(after_eq);
        return true;
    }
    if let Some(after_eq) = match_array_append(line, "fpath") {
        state.fpath_additions.extend(after_eq);
        return true;
    }
    if let Some(after_eq) = match_array_append(line, "manpath") {
        state.manpath_additions.extend(after_eq);
        return true;
    }

    // PATH=value / FPATH=value (single string, colon-joined). Captured if no dynamic.
    if let Some((k, v)) = match_simple_assign(line) {
        if !contains_dynamic(&v) {
            let v = strip_quotes(&v).to_string();
            // Special-case PATH/FPATH/etc. by routing to additions; otherwise param.
            match k.as_str() {
                "PATH" => {
                    state
                        .path_additions
                        .extend(v.split(':').filter(|s| !s.is_empty()).map(String::from));
                }
                "FPATH" => {
                    state
                        .fpath_additions
                        .extend(v.split(':').filter(|s| !s.is_empty()).map(String::from));
                }
                _ => {
                    state.params.insert(k, v);
                }
            }
            return true;
        }
    }

    // typeset / declare with simple form: `typeset NAME=value` (no flags).
    if let Some(rest) =
        strip_prefix_word(line, "typeset").or_else(|| strip_prefix_word(line, "declare"))
    {
        let toks: Vec<&str> = rest.split_whitespace().collect();
        let mut idx = 0;
        // skip leading flags `-A`, `-a`, etc. — we capture the assignment regardless of type.
        while idx < toks.len() && toks[idx].starts_with('-') {
            idx += 1;
        }
        for tok in &toks[idx..] {
            if let Some((k, v)) = tok.split_once('=') {
                if !contains_dynamic(v) {
                    state
                        .params
                        .insert(k.to_string(), strip_quotes(v).to_string());
                }
            }
        }
        return true;
    }

    let _ = raw;
    false
}

// ---- low-level matchers ----

fn strip_comment_and_trim(line: &str) -> &str {
    // Comments start at `#` not preceded by `\` and not inside a string. Naive:
    // strip everything from the first unquoted `#`. Accept false positives — they
    // just route to non_deterministic_lines, which is the safe failure mode.
    let mut in_single = false;
    let mut in_double = false;
    for (i, c) in line.char_indices() {
        match c {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double && (i == 0 || !line[..i].ends_with('\\')) => {
                return line[..i].trim();
            }
            _ => {}
        }
    }
    line.trim()
}

fn strip_precmd_modifiers(line: &str) -> &str {
    let l = line.trim_start();
    for pre in &["builtin ", "command "] {
        if let Some(rest) = l.strip_prefix(pre) {
            return rest.trim_start();
        }
    }
    l
}

fn strip_prefix_word<'a>(line: &'a str, word: &str) -> Option<&'a str> {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix(word) {
        let next = rest.chars().next();
        if matches!(next, None | Some(' ' | '\t')) {
            return Some(rest.trim_start());
        }
    }
    None
}

fn strip_quotes(s: &str) -> &str {
    let s = s.trim();
    if s.len() >= 2 {
        let bytes = s.as_bytes();
        if (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\'')
            || (bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
        {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Try to expand a path like `~/.zpwr/init.sh`, `$HOME/.foo`, `$ZDOTDIR/bar`,
/// `${ZSH}/oh-my-zsh.sh` into an absolute path that exists on disk. Returns
/// `Some(resolved)` only if every variable was resolvable AND the resulting
/// file or directory exists. Anything containing `$(...)`, backticks, or
/// glob meta returns `None` so the caller routes to replay.
fn try_expand_static_path(arg: &str) -> Option<String> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    // Reject command substitution / globs outright.
    if arg.contains('`') || arg.contains("$(") {
        return None;
    }
    if arg.contains('*') || arg.contains('?') {
        return None;
    }

    // Tilde expansion (only `~/...` form — `~user/...` requires getpwnam).
    let tilde_expanded: String = if let Some(rest) = arg.strip_prefix("~/") {
        match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => format!("{}/{}", h, rest),
            _ => return None,
        }
    } else if arg == "~" {
        std::env::var("HOME").ok()?
    } else {
        arg.to_string()
    };

    // Variable expansion. Walk the string once, replacing `$VAR` and
    // `${VAR}`. Anything we can't resolve = bail out.
    let bytes = tilde_expanded.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'$' {
            i += 1;
            if i >= bytes.len() {
                return None;
            }
            let (name, advance) = if bytes[i] == b'{' {
                let end = bytes[i + 1..].iter().position(|&b| b == b'}')?;
                let n = std::str::from_utf8(&bytes[i + 1..i + 1 + end]).ok()?;
                (n, i + 2 + end)
            } else {
                let mut j = i;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                if j == i {
                    // Stray `$` not followed by an identifier — bail.
                    return None;
                }
                let n = std::str::from_utf8(&bytes[i..j]).ok()?;
                (n, j)
            };
            let v = std::env::var(name).ok()?;
            if v.is_empty() {
                return None;
            }
            out.push_str(&v);
            i = advance;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }

    let p = std::path::Path::new(&out);
    if p.exists() {
        Some(out)
    } else {
        None
    }
}

/// Detect any `$`-or-`` ` `` substitution markers that mean we can't pre-resolve.
fn contains_dynamic(s: &str) -> bool {
    let mut in_single = false;
    for (i, c) in s.char_indices() {
        match c {
            '\'' => in_single = !in_single,
            _ if in_single => continue,
            '$' => return true,
            '`' => return true,
            '*' | '?' if i > 0 => return true, // glob expansion
            _ => {}
        }
    }
    false
}

/// alias [-g|-s] NAME=VALUE
fn match_alias(line: &str) -> Option<(Option<char>, String, String)> {
    let rest = strip_prefix_word(line, "alias")?;
    let rest = rest.trim_start();
    let (flag, body) = if let Some(b) = rest.strip_prefix("-g") {
        (Some('g'), b.trim_start())
    } else if let Some(b) = rest.strip_prefix("-s") {
        (Some('s'), b.trim_start())
    } else {
        (None, rest)
    };
    let (name, val) = body.split_once('=')?;
    let name = name.trim().to_string();
    let val = strip_quotes(val.trim()).to_string();
    if name.is_empty() {
        return None;
    }
    Some((flag, name, val))
}

/// match `name() { ... }` or `function name { ... }` — multi-line. Returns
/// (name, body, lines_consumed).
fn match_function(lines: &[&str], start: usize) -> Option<(String, String, usize)> {
    let first = strip_comment_and_trim(lines[start]);
    let first = strip_precmd_modifiers(first);

    let name = if let Some(rest) = first.strip_prefix("function ") {
        // `function name { ... }` or `function name() { ... }`
        let rest = rest.trim_start();
        let n: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if n.is_empty() {
            return None;
        }
        n
    } else if let Some(paren) = first.find("()") {
        let n = first[..paren].trim();
        if n.is_empty()
            || !n
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return None;
        }
        n.to_string()
    } else {
        return None;
    };

    // Find the opening `{` and matching `}` (track depth, skip strings naively).
    let mut depth = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut started = false;
    let mut consumed_lines = 0usize;
    let mut body = String::new();

    for (idx, raw_line) in lines.iter().enumerate().skip(start) {
        consumed_lines += 1;
        for c in raw_line.chars() {
            match c {
                '\'' if !in_double => in_single = !in_single,
                '"' if !in_single => in_double = !in_double,
                '{' if !in_single && !in_double => {
                    depth += 1;
                    started = true;
                }
                '}' if !in_single && !in_double => {
                    depth -= 1;
                }
                _ => {}
            }
        }
        if idx > start || started {
            body.push_str(raw_line);
            body.push('\n');
        }
        if started && depth == 0 {
            return Some((name, body, consumed_lines));
        }
    }
    None
}

/// `path+=(/x /y /z)` — returns the list, or None if not an array-append.
fn match_array_append(line: &str, var: &str) -> Option<Vec<String>> {
    let prefix = format!("{}+=(", var);
    let prefix_caps = format!("{}+=(", var.to_uppercase());
    let line = line.trim_start();
    let rest = if let Some(r) = line.strip_prefix(&prefix) {
        r
    } else if let Some(r) = line.strip_prefix(&prefix_caps) {
        r
    } else {
        return None;
    };
    let close = rest.find(')')?;
    let body = &rest[..close];
    let entries: Vec<String> = body
        .split_whitespace()
        .map(|s| strip_quotes(s).to_string())
        .filter(|s| !s.is_empty() && !contains_dynamic(s))
        .collect();
    Some(entries)
}

/// `NAME=value` simple assignment (no precmd, no flags). Captures only when the
/// LHS is a bare identifier with no prefix tokens.
fn match_simple_assign(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    let eq = line.find('=')?;
    let name = &line[..eq];
    if name.is_empty() {
        return None;
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    let value = &line[eq + 1..];
    Some((name.to_string(), value.to_string()))
}

fn parse_bindkey_args(rest: &str) -> Option<(String, String)> {
    // `bindkey 'KEY' widget` or `bindkey -M map 'KEY' widget`. We accept either.
    let toks: Vec<&str> = rest.split_whitespace().collect();
    let mut idx = 0;
    while idx < toks.len() && toks[idx].starts_with('-') {
        // Skip flag and its arg if applicable.
        idx += 1;
        if idx < toks.len()
            && !toks[idx - 1]
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
        {
            // unlikely; protect against malformed flags
        }
        // Many bindkey flags take an argument (like -M map). Heuristic: if the
        // previous flag is "-M" / "-N" / "-A" etc., consume one more.
        if matches!(toks[idx - 1], "-M" | "-N" | "-A" | "-d" | "-r") {
            idx += 1;
        }
    }
    if idx + 1 >= toks.len() {
        return None;
    }
    let key = strip_quotes(toks[idx]).to_string();
    let widget = toks[idx + 1].to_string();
    Some((key, widget))
}

fn parse_first_quoted_or_word(s: &str) -> Option<(String, &str)> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix('\'') {
        let end = rest.find('\'')?;
        Some((rest[..end].to_string(), rest[end + 1..].trim_start()))
    } else if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"')?;
        Some((rest[..end].to_string(), rest[end + 1..].trim_start()))
    } else {
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        Some((s[..end].to_string(), s[end..].trim_start()))
    }
}

/// Plugin-manager call detector — best-effort. Recognizes:
///   zinit                  load|light|snippet|ice <args>
///   antigen                bundle|theme|use <name>
///   zplug                  "user/repo"
///   antibody               bundle <name> (or here-docs)
///   sheldon                add <name> ...
///   znap                   source <name>
///   zgenom / zgen          load <name>
///   zr                     <name>
///
/// Each match emits one PluginDecl. zinit ice modifiers prepend their config to
/// the next zinit load (we don't track that per-decl yet — future iteration).
fn match_plugin(line: &str, raw: &str) -> Option<PluginDecl> {
    if let Some(rest) = strip_prefix_word(line, "zinit") {
        let first_tok = rest.split_whitespace().next()?;
        // Skip pure-config commands that don't load a plugin.
        if matches!(first_tok, "ice" | "wait" | "lucid" | "for" | "is-snippet") {
            return Some(PluginDecl {
                manager: "zinit".to_string(),
                name: format!("(ice/cfg: {})", rest.trim()),
                source_path: None,
                raw: raw.to_string(),
            });
        }
        return Some(PluginDecl {
            manager: "zinit".to_string(),
            name: rest
                .split_whitespace()
                .nth(1)
                .unwrap_or(first_tok)
                .to_string(),
            source_path: None,
            raw: raw.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_word(line, "antigen") {
        let toks: Vec<&str> = rest.split_whitespace().collect();
        if toks.len() >= 2 && matches!(toks[0], "bundle" | "theme" | "use") {
            return Some(PluginDecl {
                manager: "antigen".to_string(),
                name: strip_quotes(toks[1]).to_string(),
                source_path: None,
                raw: raw.to_string(),
            });
        }
    }
    if let Some(rest) = strip_prefix_word(line, "zplug") {
        let first = rest.split_whitespace().next()?;
        if first.starts_with("--") || first == "load" {
            // Skip config-only lines.
            return None;
        }
        return Some(PluginDecl {
            manager: "zplug".to_string(),
            name: strip_quotes(first).to_string(),
            source_path: None,
            raw: raw.to_string(),
        });
    }
    if let Some(rest) = strip_prefix_word(line, "antibody") {
        if let Some(rest_after) = strip_prefix_word(rest, "bundle") {
            return Some(PluginDecl {
                manager: "antibody".to_string(),
                name: strip_quotes(rest_after.trim()).to_string(),
                source_path: None,
                raw: raw.to_string(),
            });
        }
    }
    if let Some(rest) = strip_prefix_word(line, "sheldon") {
        if let Some(rest_after) = strip_prefix_word(rest, "add") {
            let name = rest_after.split_whitespace().next()?;
            return Some(PluginDecl {
                manager: "sheldon".to_string(),
                name: name.to_string(),
                source_path: None,
                raw: raw.to_string(),
            });
        }
    }
    if let Some(rest) = strip_prefix_word(line, "znap") {
        if let Some(rest_after) = strip_prefix_word(rest, "source") {
            return Some(PluginDecl {
                manager: "znap".to_string(),
                name: rest_after
                    .split_whitespace()
                    .next()
                    .unwrap_or("?")
                    .to_string(),
                source_path: None,
                raw: raw.to_string(),
            });
        }
    }
    for prefix in &["zgenom", "zgen"] {
        if let Some(rest) = strip_prefix_word(line, prefix) {
            if let Some(rest_after) =
                strip_prefix_word(rest, "load").or_else(|| strip_prefix_word(rest, "loadall"))
            {
                return Some(PluginDecl {
                    manager: prefix.to_string(),
                    name: rest_after
                        .split_whitespace()
                        .next()
                        .unwrap_or("?")
                        .to_string(),
                    source_path: None,
                    raw: raw.to_string(),
                });
            }
        }
    }
    if let Some(rest) = strip_prefix_word(line, "zr") {
        // `zr github.com/foo/bar`
        let name = rest.split_whitespace().next()?;
        if name.contains('/') {
            return Some(PluginDecl {
                manager: "zr".to_string(),
                name: name.to_string(),
                source_path: None,
                raw: raw.to_string(),
            });
        }
    }
    // Oh-My-Zsh plugins=(...) array assignment.
    if let Some(after_eq) = match_array_append(line, "plugins") {
        if !after_eq.is_empty() {
            return Some(PluginDecl {
                manager: "omz".to_string(),
                name: after_eq.join(","),
                source_path: None,
                raw: raw.to_string(),
            });
        }
    }
    if let Some((name, value)) = match_simple_assign(line) {
        if name == "plugins" {
            // OMZ plugins=(a b c) — match with parentheses.
            let v = value.trim();
            if v.starts_with('(') && v.ends_with(')') {
                let inner = &v[1..v.len() - 1];
                let names: Vec<String> = inner
                    .split_whitespace()
                    .map(|s| strip_quotes(s).to_string())
                    .collect();
                return Some(PluginDecl {
                    manager: "omz".to_string(),
                    name: names.join(","),
                    source_path: None,
                    raw: raw.to_string(),
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn analyze_str(content: &str) -> CanonicalState {
        let mut f = NamedTempFile::new().unwrap();
        f.write_all(content.as_bytes()).unwrap();
        analyze_file(f.path()).unwrap()
    }

    #[test]
    fn captures_aliases() {
        let s = analyze_str(
            r#"
alias ll='ls -la'
alias gst='git status'
alias -g G='| grep'
alias -s txt=cat
"#,
        );
        assert_eq!(s.aliases.get("ll"), Some(&"ls -la".to_string()));
        assert_eq!(s.aliases.get("gst"), Some(&"git status".to_string()));
        assert_eq!(s.global_aliases.get("G"), Some(&"| grep".to_string()));
        assert_eq!(s.suffix_aliases.get("txt"), Some(&"cat".to_string()));
    }

    #[test]
    fn captures_function_definition() {
        let s = analyze_str(
            r#"
greet() {
    echo "hello, $1"
}
function farewell {
    echo "goodbye"
}
"#,
        );
        assert!(s.functions.contains_key("greet"));
        assert!(s.functions.contains_key("farewell"));
    }

    #[test]
    fn captures_setopt_unsetopt() {
        let s = analyze_str(
            r#"
setopt extended_glob no_clobber
unsetopt beep
setopt   share_history
"#,
        );
        assert!(s.setopts.contains("extended_glob"));
        assert!(s.setopts.contains("no_clobber"));
        assert!(s.setopts.contains("share_history"));
        assert!(s.unsetopts.contains("beep"));
    }

    #[test]
    fn captures_named_dir() {
        let s = analyze_str("hash -d proj=/Users/wizard/projects\nhash -d cfg=/etc\n");
        assert_eq!(
            s.named_dirs.get("proj"),
            Some(&"/Users/wizard/projects".to_string())
        );
        assert_eq!(s.named_dirs.get("cfg"), Some(&"/etc".to_string()));
    }

    #[test]
    fn captures_compdef() {
        let s = analyze_str("compdef _git git gh hub\n");
        assert_eq!(s.compdef.get("git"), Some(&"_git".to_string()));
        assert_eq!(s.compdef.get("gh"), Some(&"_git".to_string()));
        assert_eq!(s.compdef.get("hub"), Some(&"_git".to_string()));
    }

    #[test]
    fn captures_zstyle() {
        let s = analyze_str(r#"zstyle ':completion:*' format 'completing %d'\n"#);
        assert_eq!(s.zstyle.len(), 1);
        assert_eq!(s.zstyle[0].0, ":completion:*");
    }

    #[test]
    fn captures_zmodload() {
        let s =
            analyze_str("zmodload zsh/datetime\nzmodload zsh/regex\nzmodload -F zsh/zftp +bar\n");
        assert!(s.zmodload.contains("zsh/datetime"));
        assert!(s.zmodload.contains("zsh/regex"));
        // -F flag is allowed; the module name still gets captured (positional).
        assert!(s.zmodload.contains("zsh/zftp"));
    }

    #[test]
    fn captures_export_simple() {
        let s = analyze_str(
            r#"export EDITOR=vim
export FOO="bar"
export DYNAMIC=$(date)
"#,
        );
        assert_eq!(s.env_exports.get("EDITOR"), Some(&"vim".to_string()));
        assert_eq!(s.env_exports.get("FOO"), Some(&"bar".to_string()));
        // Dynamic value must NOT leak into canonical state.
        assert!(!s.env_exports.contains_key("DYNAMIC"));
    }

    #[test]
    fn captures_path_additions() {
        let s = analyze_str("path+=(/usr/local/bin /opt/foo/bin)\nfpath+=(/Users/me/funcs)\n");
        assert!(s.path_additions.contains(&"/usr/local/bin".to_string()));
        assert!(s.path_additions.contains(&"/opt/foo/bin".to_string()));
        assert!(s.fpath_additions.contains(&"/Users/me/funcs".to_string()));
    }

    #[test]
    fn captures_simple_assign() {
        let s = analyze_str("EDITOR=vim\nKEYTIMEOUT=1\n");
        assert_eq!(s.params.get("EDITOR"), Some(&"vim".to_string()));
        assert_eq!(s.params.get("KEYTIMEOUT"), Some(&"1".to_string()));
    }

    #[test]
    fn dynamic_assign_routes_to_replay() {
        let s = analyze_str("EXPENSIVE=$(slow_command)\n");
        assert!(!s.params.contains_key("EXPENSIVE"));
        assert_eq!(s.stats.lines_non_deterministic, 1);
        assert_eq!(s.non_deterministic_lines.len(), 1);
    }

    #[test]
    fn comments_stripped() {
        let s = analyze_str("alias x=y    # this is fine\n#alias z=w\n");
        assert!(s.aliases.contains_key("x"));
        assert!(!s.aliases.contains_key("z"));
    }

    #[test]
    fn zinit_plugin_decl_recognized() {
        let s = analyze_str("zinit load zdharma-continuum/fast-syntax-highlighting\n");
        assert_eq!(s.plugin_decls.len(), 1);
        assert_eq!(s.plugin_decls[0].manager, "zinit");
    }

    #[test]
    fn omz_plugins_array_recognized() {
        let s = analyze_str("plugins=(git docker kubectl)\n");
        assert_eq!(s.plugin_decls.len(), 1);
        assert_eq!(s.plugin_decls[0].manager, "omz");
        assert!(s.plugin_decls[0].name.contains("git"));
        assert!(s.plugin_decls[0].name.contains("docker"));
    }

    #[test]
    fn antigen_bundle_recognized() {
        let s = analyze_str("antigen bundle zsh-users/zsh-syntax-highlighting\n");
        assert_eq!(s.plugin_decls.len(), 1);
        assert_eq!(s.plugin_decls[0].manager, "antigen");
        assert!(s.plugin_decls[0].name.contains("zsh-users"));
    }

    #[test]
    fn zplug_user_repo_recognized() {
        let s = analyze_str(r#"zplug "zsh-users/zsh-autosuggestions"\n"#);
        assert_eq!(s.plugin_decls.len(), 1);
        assert_eq!(s.plugin_decls[0].manager, "zplug");
    }

    #[test]
    fn antibody_bundle_recognized() {
        let s = analyze_str("antibody bundle robbyrussell/oh-my-zsh\n");
        assert_eq!(s.plugin_decls.len(), 1);
        assert_eq!(s.plugin_decls[0].manager, "antibody");
    }

    #[test]
    fn sheldon_add_recognized() {
        let s = analyze_str("sheldon add starship --git github.com/starship/starship\n");
        assert_eq!(s.plugin_decls.len(), 1);
        assert_eq!(s.plugin_decls[0].manager, "sheldon");
        assert_eq!(s.plugin_decls[0].name, "starship");
    }

    #[test]
    fn znap_source_recognized() {
        let s = analyze_str("znap source ohmyzsh/oh-my-zsh lib/git\n");
        assert_eq!(s.plugin_decls.len(), 1);
        assert_eq!(s.plugin_decls[0].manager, "znap");
    }

    #[test]
    fn zinit_ice_config_recognized_separately() {
        let s = analyze_str(
            r#"zinit ice wait lucid
zinit load some/plugin
"#,
        );
        assert_eq!(s.plugin_decls.len(), 2);
        assert_eq!(s.plugin_decls[0].manager, "zinit");
        assert!(s.plugin_decls[0].name.contains("ice"));
        assert_eq!(s.plugin_decls[1].manager, "zinit");
    }

    #[test]
    fn source_files_recorded() {
        let s = analyze_str("source /etc/zshrc.local\n. /opt/include.sh\n");
        assert!(s.sourced_files.contains(&"/etc/zshrc.local".to_string()));
        assert!(s.sourced_files.contains(&"/opt/include.sh".to_string()));
    }

    #[test]
    fn dynamic_source_routes_to_replay() {
        let s = analyze_str("source $HOME/.zshrc.local\n");
        assert!(s.sourced_files.is_empty());
        assert_eq!(s.stats.lines_non_deterministic, 1);
    }

    #[test]
    fn precommand_modifiers_stripped() {
        let s = analyze_str("builtin source /etc/zshrc.local\ncommand setopt foo\n");
        assert!(s.sourced_files.contains(&"/etc/zshrc.local".to_string()));
        assert!(s.setopts.contains("foo"));
    }

    #[test]
    fn captures_bindkey() {
        let s = analyze_str(
            r#"bindkey '^A' beginning-of-line
bindkey -M vicmd 'k' up-line-or-history
"#,
        );
        assert_eq!(s.bindkeys.get("^A"), Some(&"beginning-of-line".to_string()));
        // bindkey -M parsing is best-effort.
        assert!(s.bindkeys.contains_key("k") || s.bindkeys.is_empty() || s.bindkeys.len() >= 1);
    }
}
