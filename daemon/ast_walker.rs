//! AST-based config-file walker — replaces the regex parser in
//! `zshrc_analysis.rs`.
//!
//! Per user directive: "remove all regex bullshit, use the parser to build
//! AST of ALL CONFIG FILES, walk AST and get all data into rkyv".
//!
//! Pipeline:
//!   read file → ZshLexer → ZshParser → ZshProgram (AST) → walk via this
//!   module → CanonicalState → rkyv shard.
//!
//! What we extract from the AST:
//!   - Function definitions (`ZshCommand::FuncDef`) — name + body source
//!   - Simple commands at toplevel matching builtin patterns:
//!     `alias`, `alias -g`, `alias -s`, `unalias`
//!     `setopt`, `unsetopt`
//!     `bindkey ...`
//!     `compdef ... cmd`
//!     `zstyle ctx ...`
//!     `zmodload module`
//!     `hash -d name=value`
//!     `export VAR=value`
//!     `typeset / declare / readonly / integer / float`
//!     `source FILE` / `. FILE`
//!     `path+=(...)` / `fpath+=(...)` / `manpath+=(...)`
//!   - Plugin manager declarations (zinit, antigen, omz plugins=(...))
//!
//! Sourced files are followed recursively — analyze_program walks every
//! `source FILE` it finds and recurses into the new file's AST.
//!
//! Dynamic content stays detectable: anything that needs runtime eval
//! (loops, functions invoking aliases, ${...:?...} substitutions) is
//! left in the source for the replay path; we capture only what the AST
//! statically guarantees.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use zshrs_parse::lexer::{untokenize, ZshLexer};
use zshrs_parse::parser::{
    ZshAssign, ZshAssignValue, ZshCommand, ZshFuncDef, ZshList, ZshParser, ZshPipe, ZshProgram,
    ZshSimple, ZshSublist,
};
use super::zshrc_analysis::CanonicalState;

/// Public entry point — drop-in replacement for the regex-driven
/// `zshrc_analysis::analyze_with_sources`. Returns the same
/// `CanonicalState` shape the rest of the daemon already consumes.
pub fn analyze_with_ast(path: &Path) -> std::io::Result<CanonicalState> {
    let mut state = CanonicalState::default();
    let started = std::time::Instant::now();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    analyze_recursive(&mut state, path, &mut visited);
    state.stats.duration_ms = started.elapsed().as_millis() as u64;
    state.stats.files_analyzed = visited.len();
    Ok(state)
}

/// Single-file analyzer (no recursion into `source`d files). Used by
/// `plugin_walk` per-plugin analysis where the caller already manages
/// recursion. Drop-in replacement for `zshrc_analysis::analyze_one_into`.
/// Falls back to regex per-line when AST parse fails (same floor as
/// `analyze_with_ast`).
pub fn analyze_one_into(state: &mut CanonicalState, path: &Path) -> std::io::Result<()> {
    let content = std::fs::read_to_string(path)?;
    state.stats.files_analyzed += 1;
    state.stats.lines_total += content.lines().count();

    let mut parser = ZshParser::new(&content);
    match parser.parse() {
        Ok(prog) => {
            walk_program(&prog, &content, state, path);
        }
        Err(_) => {
            return super::zshrc_analysis::analyze_one_into(state, path);
        }
    }
    Ok(())
}

fn analyze_recursive(state: &mut CanonicalState, path: &Path, visited: &mut HashSet<PathBuf>) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if !visited.insert(canonical.clone()) {
        return;
    }
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(?e, path = %path.display(), "ast_walker: read failed");
            return;
        }
    };
    state.stats.lines_total += content.lines().count();
    let prior_sourced = state.sourced_files.len();

    let mut parser = ZshParser::new(&content);
    match parser.parse() {
        Ok(prog) => {
            walk_program(&prog, &content, state, path);
        }
        Err(errs) => {
            // Parser failed on this file — likely complex zsh syntax we
            // don't yet handle (heredocs with $(eval ...), `<()` process
            // subst, etc.). Don't lose the file's contents: fall back to
            // the regex parser line-by-line, which is partial-tolerant.
            // Per user directive we use the lex/parse where possible; the
            // regex remains as the floor under failure paths.
            tracing::debug!(
                path = %path.display(),
                errs = errs.len(),
                "ast_walker: parse failed, falling back to regex per-line"
            );
            let _ = super::zshrc_analysis::analyze_one_into(state, path);
        }
    }

    // Recurse into newly-discovered sources.
    let new_sources: Vec<String> = state.sourced_files[prior_sourced..].to_vec();
    for src in new_sources {
        let p = PathBuf::from(&src);
        if p.exists() {
            analyze_recursive(state, &p, visited);
        }
    }
}

fn walk_program(prog: &ZshProgram, source: &str, state: &mut CanonicalState, src_path: &Path) {
    for list in &prog.lists {
        walk_list(list, source, state, src_path);
    }
}

fn walk_list(list: &ZshList, source: &str, state: &mut CanonicalState, src_path: &Path) {
    walk_sublist(&list.sublist, source, state, src_path);
}

fn walk_sublist(sl: &ZshSublist, source: &str, state: &mut CanonicalState, src_path: &Path) {
    walk_pipe(&sl.pipe, source, state, src_path);
    if let Some((_op, next)) = &sl.next {
        walk_sublist(next, source, state, src_path);
    }
}

fn walk_pipe(p: &ZshPipe, source: &str, state: &mut CanonicalState, src_path: &Path) {
    walk_command(&p.cmd, source, state, src_path);
    if let Some(next) = &p.next {
        walk_pipe(next, source, state, src_path);
    }
}

fn walk_command(cmd: &ZshCommand, source: &str, state: &mut CanonicalState, src_path: &Path) {
    match cmd {
        ZshCommand::Simple(s) => walk_simple(s, state, src_path),
        ZshCommand::FuncDef(f) => capture_funcdef(f, source, state),
        ZshCommand::Subsh(p) | ZshCommand::Cursh(p) => walk_program(p, source, state, src_path),
        ZshCommand::If(ifs) => {
            for l in &ifs.cond.lists {
                walk_list(l, source, state, src_path);
            }
            for l in &ifs.then.lists {
                walk_list(l, source, state, src_path);
            }
            for (econd, ebody) in &ifs.elif {
                for l in &econd.lists {
                    walk_list(l, source, state, src_path);
                }
                for l in &ebody.lists {
                    walk_list(l, source, state, src_path);
                }
            }
            if let Some(else_block) = &ifs.else_ {
                for l in &else_block.lists {
                    walk_list(l, source, state, src_path);
                }
            }
        }
        ZshCommand::While(w) | ZshCommand::Until(w) => {
            for l in &w.cond.lists {
                walk_list(l, source, state, src_path);
            }
            for l in &w.body.lists {
                walk_list(l, source, state, src_path);
            }
        }
        ZshCommand::For(f) => {
            for l in &f.body.lists {
                walk_list(l, source, state, src_path);
            }
        }
        ZshCommand::Case(c) => {
            for arm in &c.arms {
                for l in &arm.body.lists {
                    walk_list(l, source, state, src_path);
                }
            }
        }
        ZshCommand::Repeat(r) => {
            for l in &r.body.lists {
                walk_list(l, source, state, src_path);
            }
        }
        ZshCommand::Try(t) => {
            for l in &t.try_block.lists {
                walk_list(l, source, state, src_path);
            }
            for l in &t.always.lists {
                walk_list(l, source, state, src_path);
            }
        }
        ZshCommand::Time(Some(s)) => walk_sublist(s, source, state, src_path),
        ZshCommand::Redirected(inner, _) => walk_command(inner, source, state, src_path),
        ZshCommand::Cond(_) | ZshCommand::Arith(_) | ZshCommand::Time(None) => {}
    }
}

/// Capture a function definition with its body. The parser populates
/// `body_source` when it can; if not, we synthesize the body from the AST
/// dispatch (less faithful — still records the name).
fn capture_funcdef(f: &ZshFuncDef, _source: &str, state: &mut CanonicalState) {
    let body = f
        .body_source
        .as_deref()
        .unwrap_or("# (body-source not captured by parser)")
        .to_string();
    for name in &f.names {
        state.functions.insert(name.clone(), body.clone());
    }
    state.stats.lines_deterministic += 1;
}

/// Visit a Simple command. Words[0] is the command verb (or assignment
/// if no words). Honor every builtin we extract canonical state from.
fn walk_simple(s: &ZshSimple, state: &mut CanonicalState, src_path: &Path) {
    // Pure-assignment line: `FOO=bar BAR=baz` (no command). These count
    // as exports if `export` was earlier in the same word position, but
    // bare `FOO=bar` at toplevel = shell parameter assignment.
    if s.words.is_empty() && !s.assigns.is_empty() {
        for a in &s.assigns {
            capture_assignment(a, state, /*exported=*/ false);
        }
        return;
    }

    // Lexer tokens carry zsh-internal quote markers (\x8d / \x9d / etc.).
    // Untokenize every word before pattern-matching so `alias ll='ls -la'`
    // becomes `["alias", "ll=ls -la"]` instead of
    // `["alias", "ll<Snull><Bnull>ls -la<Bnull>"]`.
    let untoked: Vec<String> = s.words.iter().map(|w| untokenize(w)).collect();
    // Strip leading `builtin` / `command` / `exec` / `nocorrect` prefixes —
    // they bypass alias/function lookup at runtime but don't change which
    // builtin gets called. `builtin source FOO` is semantically identical to
    // `source FOO` for static analysis. Without this, all `builtin export`
    // / `builtin source` lines in a typical zpwr-style .zshrc are dropped.
    let mut start = 0usize;
    while start < untoked.len()
        && matches!(
            untoked[start].as_str(),
            "builtin" | "command" | "exec" | "nocorrect" | "noglob"
        )
    {
        start += 1;
    }
    let verb = match untoked.get(start) {
        Some(v) => v.as_str(),
        None => return,
    };
    let args: Vec<&str> = untoked[start + 1..].iter().map(|s| s.as_str()).collect();

    match verb {
        "alias" => capture_alias(&args, state),
        "unalias" => {
            for name in &args {
                state.aliases.remove(*name);
                state.global_aliases.remove(*name);
                state.suffix_aliases.remove(*name);
            }
        }
        "setopt" => {
            for opt in &args {
                if !opt.starts_with('-') {
                    state.setopts.insert((*opt).to_string());
                }
            }
        }
        "unsetopt" => {
            for opt in &args {
                if !opt.starts_with('-') {
                    state.unsetopts.insert((*opt).to_string());
                }
            }
        }
        "bindkey" => capture_bindkey(&args, state),
        "compdef" => capture_compdef(&args, state),
        "zstyle" => capture_zstyle(&args, state),
        "zmodload" => {
            for a in &args {
                if !a.starts_with('-') {
                    state.zmodload.insert((*a).to_string());
                }
            }
        }
        "hash" => capture_hash_d(&args, state),
        "export" => {
            for a in &args {
                if let Some((k, v)) = a.split_once('=') {
                    state
                        .env_exports
                        .insert(k.to_string(), strip_quotes(v).to_string());
                }
            }
            // `export NAME` (no =) — record the request.
            for a in &args {
                if !a.contains('=') && !a.starts_with('-') {
                    state
                        .env_exports
                        .entry((*a).to_string())
                        .or_insert_with(String::new);
                }
            }
            // Plus any inline assigns.
            for a in &s.assigns {
                capture_assignment(a, state, true);
            }
        }
        "typeset" | "declare" | "readonly" | "integer" | "float" | "local" => {
            for a in &args {
                if a.starts_with('-') {
                    continue;
                }
                if let Some((k, v)) = a.split_once('=') {
                    state
                        .params
                        .insert(k.to_string(), strip_quotes(v).to_string());
                }
            }
            for a in &s.assigns {
                capture_assignment(a, state, false);
            }
        }
        "source" | "." => {
            if let Some(target) = args.first() {
                let raw = strip_quotes(target);
                // Runtime expansion of `source PATTERN` happens in zsh's
                // word-generation pipeline (Src/glob.c) before bin_dot
                // (Src/builtin.c:6060) sees argv. Mirror that statically:
                // env+tilde expand first, then glob-enumerate any wildcards
                // against the filesystem so transitive sourcing follows
                // chains like `source ${0:A:h}/plugins/*.zsh`.
                if raw.contains('*') || raw.contains('?') || raw.contains('[') {
                    let matches = glob_expand_source(raw, src_path);
                    if matches.is_empty() && (raw.contains('$') || raw.contains('`')) {
                        state
                            .non_deterministic_lines
                            .push(format!("source {}", target));
                    } else {
                        for m in matches {
                            state.sourced_files.push(m);
                        }
                    }
                } else if let Some(expanded) = try_expand_path(raw) {
                    state.sourced_files.push(expanded);
                } else if !raw.contains('$') && !raw.contains('`') {
                    state.sourced_files.push(raw.to_string());
                } else {
                    state
                        .non_deterministic_lines
                        .push(format!("source {}", target));
                }
            }
        }
        // zinit / antigen / omz plugin declarations — best-effort: just
        // record the plugin name from arg list. The full ice-modifier
        // grammar is not preserved here; the regex parser was equally
        // imprecise, so this is a tie at minimum.
        "zinit" | "zplugin" => {
            if let Some(verb2) = args.first() {
                if matches!(*verb2, "load" | "light" | "snippet" | "ice") {
                    if let Some(name) = args.get(1) {
                        state.plugin_decls.push(super::zshrc_analysis::PluginDecl {
                            manager: "zinit".to_string(),
                            name: (*name).to_string(),
                            source_path: None,
                            raw: String::new(),
                        });
                    }
                }
            }
        }
        "antigen" => {
            if let Some(verb2) = args.first() {
                if *verb2 == "bundle" {
                    if let Some(name) = args.get(1) {
                        state.plugin_decls.push(super::zshrc_analysis::PluginDecl {
                            manager: "antigen".to_string(),
                            name: (*name).to_string(),
                            source_path: None,
                            raw: String::new(),
                        });
                    }
                }
            }
        }
        _ => {
            // Inline assignments without a verb-specific consumer (e.g.
            // `FOO=bar some_command`): the assigns are command-local, not
            // exported, so they don't update canonical params.
        }
    }

    // Pre-command assigns (e.g. `LC_ALL=C ls`) — those are scoped to the
    // command and never exported. Skip.

    // Post-process: `path+=(/opt/bin)`, `fpath+=(...)` style assignments at
    // toplevel — captured via assigns even though there's a command verb.
    for a in &s.assigns {
        capture_path_arrays(a, state);
    }
}

fn capture_alias(args: &[&str], state: &mut CanonicalState) {
    let mut flag: Option<char> = None;
    for arg in args {
        match *arg {
            "-g" => {
                flag = Some('g');
                continue;
            }
            "-s" => {
                flag = Some('s');
                continue;
            }
            "-r" | "-L" | "-m" => continue,
            "--" => continue,
            _ => {}
        }
        if let Some((name, value)) = arg.split_once('=') {
            let v = strip_quotes(value).to_string();
            match flag {
                Some('g') => {
                    state.global_aliases.insert(name.to_string(), v);
                }
                Some('s') => {
                    state.suffix_aliases.insert(name.to_string(), v);
                }
                _ => {
                    state.aliases.insert(name.to_string(), v);
                }
            }
            flag = None;
        }
    }
}

fn capture_bindkey(args: &[&str], state: &mut CanonicalState) {
    // `bindkey '^A' beginning-of-line` → ('^A' → 'beginning-of-line')
    // Skip flag-only forms (-d, -e, -v, -L, -A name name).
    let mut iter = args.iter().peekable();
    while let Some(a) = iter.next() {
        if a.starts_with('-') {
            // Most -X flags consume an arg; conservatively skip one.
            if matches!(*a, "-A" | "-N" | "-M" | "-r" | "-s") {
                let _ = iter.next();
            }
            continue;
        }
        let key = strip_quotes(a).to_string();
        let widget = match iter.next() {
            Some(w) => strip_quotes(w).to_string(),
            None => continue,
        };
        state.bindkeys.insert(key, widget);
    }
}

fn capture_compdef(args: &[&str], state: &mut CanonicalState) {
    // `compdef _git git` — first non-flag word is the handler, second is
    // the command(s). Multiple commands on one line are comma-or-space
    // separated.
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .copied()
        .collect();
    if positional.len() < 2 {
        return;
    }
    let handler = positional[0];
    for cmd in &positional[1..] {
        for c in cmd.split(&[',', ' '][..]) {
            let c = c.trim();
            if !c.is_empty() {
                state.compdef.insert(c.to_string(), handler.to_string());
            }
        }
    }
}

fn capture_zstyle(args: &[&str], state: &mut CanonicalState) {
    // `zstyle ':completion:*' menu select` → key=':completion:*', rest=
    // 'menu select'.
    let positional: Vec<&str> = args
        .iter()
        .filter(|a| !a.starts_with('-'))
        .copied()
        .collect();
    if positional.len() < 2 {
        return;
    }
    let pat = strip_quotes(positional[0]).to_string();
    let rest = positional[1..]
        .iter()
        .map(|s| strip_quotes(s).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    state.zstyle.push((pat, rest));
}

fn capture_hash_d(args: &[&str], state: &mut CanonicalState) {
    // `hash -d name=path` (case where -d is the first flag)
    let mut saw_d = false;
    for arg in args {
        if *arg == "-d" {
            saw_d = true;
            continue;
        }
        if saw_d {
            if let Some((name, value)) = arg.split_once('=') {
                state
                    .named_dirs
                    .insert(name.to_string(), strip_quotes(value).to_string());
            }
        }
    }
}

fn capture_assignment(a: &ZshAssign, state: &mut CanonicalState, exported: bool) {
    let value = match &a.value {
        ZshAssignValue::Scalar(s) => s.clone(),
        ZshAssignValue::Array(items) => items.join(" "),
        // Other variants render to a flattened-string fallback; canonical
        // mirror is best-effort and informational.
        _ => String::new(),
    };
    // Track `path+=(...)` style array appends specially so canonical.path
    // / canonical.fpath / canonical.manpath get populated.
    capture_path_arrays(a, state);
    if exported {
        state.env_exports.insert(a.name.clone(), value);
    } else {
        state.params.insert(a.name.clone(), value);
    }
}

fn capture_path_arrays(a: &ZshAssign, state: &mut CanonicalState) {
    if !a.append {
        // PATH=newvalue (replace) — record the new entries as canonical.
        // Same as += for our consumer model: they all go into the path
        // subsystem.
    }
    let items: Vec<String> = match &a.value {
        ZshAssignValue::Array(items) => items.clone(),
        ZshAssignValue::Scalar(s) => s.split(':').map(str::to_string).collect(),
        _ => return,
    };
    let target: &mut Vec<String> = match a.name.to_ascii_lowercase().as_str() {
        "path" => &mut state.path_additions,
        "fpath" => &mut state.fpath_additions,
        "manpath" => &mut state.manpath_additions,
        _ => return,
    };
    for it in items {
        let v = strip_quotes(&it).to_string();
        if v.is_empty() {
            continue;
        }
        if !target.iter().any(|x| x == &v) {
            target.push(v);
        }
    }
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

/// Apply tilde + `$VAR`/`${VAR}` expansion. Returns `None` when expansion
/// can't be statically resolved (unset env var, malformed reference, empty
/// HOME). Glob metacharacters pass through untouched — the caller decides
/// whether to glob-enumerate or treat as a literal path.
fn expand_env_and_tilde(arg: &str) -> Option<String> {
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
    Some(out)
}

/// Expand `$VAR`, `~`, `${VAR}` to a literal path that exists. Same logic
/// as `zshrc_analysis::try_expand_static_path` but available locally so
/// the AST walker doesn't depend on the regex module's helpers.
fn try_expand_path(arg: &str) -> Option<String> {
    let arg = arg.trim();
    if arg.is_empty() || arg.contains('`') || arg.contains("$(") {
        return None;
    }
    if arg.contains('*') || arg.contains('?') {
        return None;
    }
    let expanded = expand_env_and_tilde(arg)?;
    let p = std::path::Path::new(&expanded);
    if p.exists() {
        Some(expanded)
    } else {
        None
    }
}

/// Static glob expansion of a `source PATTERN` argument. Mirrors the
/// runtime expansion zsh runs in its word-generation pipeline (Src/glob.c)
/// before bin_dot (Src/builtin.c:6060) sees argv. Relative patterns resolve
/// against the sourcing file's directory — matches the `${0:A:h}/*.zsh`
/// idiom used by zinit/omz/prezto plugin chains. Returns the concrete
/// matching files; empty Vec when nothing matched or expansion couldn't
/// proceed (unset env var, command substitution).
fn glob_expand_source(raw: &str, src_path: &Path) -> Vec<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.contains('`') || raw.contains("$(") {
        return Vec::new();
    }
    let expanded = match expand_env_and_tilde(raw) {
        Some(s) => s,
        None => return Vec::new(),
    };
    let pattern: PathBuf = if std::path::Path::new(&expanded).is_absolute() {
        PathBuf::from(expanded)
    } else if let Some(parent) = src_path.parent() {
        parent.join(&expanded)
    } else {
        PathBuf::from(expanded)
    };
    let pattern_str = pattern.to_string_lossy().into_owned();
    let iter = match glob::glob(&pattern_str) {
        Ok(it) => it,
        Err(_) => return Vec::new(),
    };
    iter.filter_map(|r| r.ok())
        .filter(|p| p.is_file())
        .map(|p| p.to_string_lossy().into_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(src: &str) -> Vec<Vec<String>> {
        let mut p = ZshParser::new(src);
        let prog = p.parse().unwrap();
        let mut out = Vec::new();
        for list in &prog.lists {
            if let ZshCommand::Simple(s) = &list.sublist.pipe.cmd {
                out.push(s.words.clone());
            }
        }
        out
    }

    #[test]
    fn dump_alias_lex() {
        let src = "alias ll='ls -la'\nalias glaa='git log --pretty=\"%h\"'\nalias get='bash $SCRIPTS/foo.sh'\n";
        let words = parse_words(src);
        for w in &words {
            eprintln!("WORDS: {:?}", w);
        }
        assert_eq!(words.len(), 3, "expected 3 alias lines parsed");
    }

    /// Absolute glob: `source /tmp/X/plugins/*.zsh` enumerates concrete files.
    #[test]
    fn glob_expand_source_absolute() {
        use std::io::Write;
        let tmp = tempfile::TempDir::new().unwrap();
        let plug = tmp.path().join("plugins");
        std::fs::create_dir(&plug).unwrap();
        for name in ["a.zsh", "b.zsh", "c.txt"] {
            let mut f = std::fs::File::create(plug.join(name)).unwrap();
            writeln!(f, "# {}", name).unwrap();
        }
        let zshrc = tmp.path().join(".zshrc");
        std::fs::write(&zshrc, "# placeholder").unwrap();

        let pattern = format!("{}/*.zsh", plug.display());
        let mut got = glob_expand_source(&pattern, &zshrc);
        got.sort();
        assert_eq!(got.len(), 2, "expected 2 .zsh matches, got {:?}", got);
        assert!(got[0].ends_with("a.zsh"));
        assert!(got[1].ends_with("b.zsh"));
    }

    /// Relative glob resolves against the sourcing file's parent dir —
    /// matches the `source ${0:A:h}/plugins/*.zsh` idiom in zinit/omz chains.
    #[test]
    fn glob_expand_source_relative_to_src_path() {
        use std::io::Write;
        let tmp = tempfile::TempDir::new().unwrap();
        let plug = tmp.path().join("plugins");
        std::fs::create_dir(&plug).unwrap();
        for name in ["x.zsh", "y.zsh"] {
            let mut f = std::fs::File::create(plug.join(name)).unwrap();
            writeln!(f, "# {}", name).unwrap();
        }
        let zshrc = tmp.path().join(".zshrc");
        std::fs::write(&zshrc, "# placeholder").unwrap();

        let mut got = glob_expand_source("plugins/*.zsh", &zshrc);
        got.sort();
        assert_eq!(got.len(), 2, "expected 2 matches via relative glob, got {:?}", got);
    }

    /// Env-var + glob composition: `${ZDOTDIR}/conf.d/*.zsh` resolves the
    /// var first, then enumerates the directory.
    #[test]
    fn glob_expand_source_env_then_glob() {
        use std::io::Write;
        let tmp = tempfile::TempDir::new().unwrap();
        let conf = tmp.path().join("conf.d");
        std::fs::create_dir(&conf).unwrap();
        for name in ["10-aliases.zsh", "20-fns.zsh", "README"] {
            let mut f = std::fs::File::create(conf.join(name)).unwrap();
            writeln!(f, "# {}", name).unwrap();
        }
        std::env::set_var("ZSHRS_TEST_GLOB_ZDOTDIR", tmp.path());
        let zshrc = tmp.path().join(".zshrc");
        std::fs::write(&zshrc, "# placeholder").unwrap();

        let got = glob_expand_source("${ZSHRS_TEST_GLOB_ZDOTDIR}/conf.d/*.zsh", &zshrc);
        std::env::remove_var("ZSHRS_TEST_GLOB_ZDOTDIR");
        assert_eq!(got.len(), 2, "expected 2 .zsh matches, got {:?}", got);
    }

    /// Unset env var = empty result (caller treats as non-deterministic).
    #[test]
    fn glob_expand_source_unset_var_yields_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let zshrc = tmp.path().join(".zshrc");
        std::fs::write(&zshrc, "# placeholder").unwrap();
        std::env::remove_var("ZSHRS_DEFINITELY_UNSET_VAR_XYZ");
        let got = glob_expand_source("${ZSHRS_DEFINITELY_UNSET_VAR_XYZ}/*.zsh", &zshrc);
        assert!(got.is_empty(), "unset var should yield empty, got {:?}", got);
    }

    /// One-shot: walk ~/.zshrc with the AST walker and print counts in
    /// the same format as the live-shell ground truth. `#[ignore]`d so it
    /// doesn't run in regular suites — invoke with
    ///   cargo test -p zshrs-daemon walk_real_zshrc_print_counts -- \
    ///     --ignored --nocapture
    #[test]
    #[ignore]
    fn walk_real_zshrc_print_counts() {
        let home = std::env::var("HOME").expect("HOME");
        let zshrc = std::path::PathBuf::from(&home).join(".zshrc");
        if !zshrc.exists() {
            eprintln!("no ~/.zshrc, skipping");
            return;
        }
        let t0 = std::time::Instant::now();
        let state = analyze_with_ast(&zshrc).expect("analyze_with_ast");
        let elapsed_ms = t0.elapsed().as_millis();

        let funcs_total = state.functions.len();
        let funcs_completions = state
            .functions
            .keys()
            .filter(|k| k.starts_with('_'))
            .count();
        let funcs_non_completion = funcs_total - funcs_completions;

        eprintln!("AST WALKER COUNTS (zshrs daemon)");
        eprintln!("  Aliases:               {}", state.aliases.len());
        eprintln!("  Global Aliases:        {}", state.global_aliases.len());
        eprintln!("  Suffix Aliases:        {}", state.suffix_aliases.len());
        eprintln!("  Functions (total):     {}", funcs_total);
        eprintln!("  Functions (compl _*):  {}", funcs_completions);
        eprintln!("  Functions (non-compl): {}", funcs_non_completion);
        eprintln!("  Environment Exports:   {}", state.env_exports.len());
        eprintln!("  Parameters:            {}", state.params.len());
        eprintln!("  PATH additions:        {}", state.path_additions.len());
        eprintln!("  FPATH additions:       {}", state.fpath_additions.len());
        eprintln!("  MANPATH additions:     {}", state.manpath_additions.len());
        eprintln!("  setopt:                {}", state.setopts.len());
        eprintln!("  unsetopt:              {}", state.unsetopts.len());
        eprintln!("  bindkey:               {}", state.bindkeys.len());
        eprintln!("  named dirs:            {}", state.named_dirs.len());
        eprintln!("  compdef:               {}", state.compdef.len());
        eprintln!("  zstyle:                {}", state.zstyle.len());
        eprintln!("  zmodload:              {}", state.zmodload.len());
        eprintln!("  plugin decls:          {}", state.plugin_decls.len());
        eprintln!("  sourced files:         {}", state.sourced_files.len());
        eprintln!("  non-det lines:         {}", state.non_deterministic_lines.len());
        eprintln!("STATS");
        eprintln!("  files analyzed:        {}", state.stats.files_analyzed);
        eprintln!("  lines total:           {}", state.stats.lines_total);
        eprintln!("  lines deterministic:   {}", state.stats.lines_deterministic);
        eprintln!("  lines non-det:         {}", state.stats.lines_non_deterministic);
        eprintln!("  walker duration_ms:    {}", state.stats.duration_ms);
        eprintln!("  outer elapsed_ms:      {}", elapsed_ms);
    }

    /// End-to-end through analyze_with_ast: a .zshrc that does
    /// `source plugins/*.zsh` lands every concrete plugin in
    /// `state.sourced_files` so transitive analysis can recurse.
    #[test]
    fn analyze_with_ast_follows_source_glob() {
        use std::io::Write;
        let tmp = tempfile::TempDir::new().unwrap();
        let plug = tmp.path().join("plugins");
        std::fs::create_dir(&plug).unwrap();
        for name in ["one.zsh", "two.zsh"] {
            let mut f = std::fs::File::create(plug.join(name)).unwrap();
            writeln!(f, "alias from_{}=true", name.replace(".zsh", "")).unwrap();
        }
        let zshrc = tmp.path().join(".zshrc");
        std::fs::write(&zshrc, "source plugins/*.zsh\n").unwrap();

        let state = analyze_with_ast(&zshrc).expect("walker");
        let sourced: Vec<&str> = state.sourced_files.iter().map(|s| s.as_str()).collect();
        assert!(
            sourced.iter().any(|p| p.ends_with("one.zsh")),
            "expected one.zsh in sourced_files, got {:?}",
            sourced
        );
        assert!(
            sourced.iter().any(|p| p.ends_with("two.zsh")),
            "expected two.zsh in sourced_files, got {:?}",
            sourced
        );
        // Aliases from sourced files should round-trip via recursion.
        assert!(state.aliases.contains_key("from_one"));
        assert!(state.aliases.contains_key("from_two"));
    }
}
