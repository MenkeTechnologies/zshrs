//! fish's `complete` builtin and the completion engine behind TAB in `zshrs --fish`.
//!
//! zshrs-original — no zsh C counterpart. The reference is fish 4.9:
//! `src/complete.rs` (the option/argument matcher, `complete_param_for_command`,
//! the wrap chain), `src/builtins/complete.rs` (the builtin's options),
//! `src/builtins/commandline.rs` (what `commandline` reports while a completion
//! runs) and `src/autoload.rs` (where `<cmd>.fish` is looked up).
//!
//! How it fits under compsys: fish mode runs `compinit` and puts the
//! `_fish_complete` completer ahead of `_complete`. That completer hands the
//! text before the cursor to [`query`]; when a fish definition covers the
//! command (one registered by `complete`, or a `<cmd>.fish` found on
//! `$fish_complete_path` or among the installed fish's own scripts) the matches
//! come back with their descriptions, otherwise zsh's own completer for the
//! command runs. Command names, variables and `~user` stay with compsys.
//!
//! fish 4 ships its completion and function scripts inside the binary. They
//! are read from an installed `fish` with `status get-file` and cached under
//! `$ZSHRS_HOME/fish-data`; without a `fish` on `PATH` only the directories
//! on the search paths are used.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

// ---------------------------------------------------------------------------
// The definition store (fish `COMPLETION_MAP`, `WRAPPER_MAP`, tombstones).
// ---------------------------------------------------------------------------

/// fish `CompleteOptionType`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OptType {
    /// no option: arguments to the command itself
    ArgsOnly,
    /// `-x`
    Short,
    /// `-foo`
    SingleLong,
    /// `--foo`
    DoubleLong,
}

/// fish `CompletionMode`.
#[derive(Clone, Copy, Default, Debug)]
struct Mode {
    no_files: bool,
    force_files: bool,
    requires_param: bool,
}

/// fish `CompleteEntryOpt`: one `complete` line.
#[derive(Clone, Debug)]
struct Opt {
    option: String,
    typ: OptType,
    mode: Mode,
    /// `-a` text, fish source expanded at completion time
    comp: String,
    desc: String,
    /// `-n` texts, fish source; all must succeed
    conditions: Vec<String>,
    /// `-k`
    keep_order: bool,
}

impl Opt {
    fn expected_dash_count(&self) -> usize {
        match self.typ {
            OptType::ArgsOnly => 0,
            OptType::Short | OptType::SingleLong => 1,
            OptType::DoubleLong => 2,
        }
    }
}

/// fish `CompletionEntry`.
struct Entry {
    options: Vec<Opt>,
    /// creation order, for `complete` with no arguments
    order: usize,
}

/// One `complete_load` of a command.
#[derive(Clone, Copy)]
struct Loaded {
    /// `$ZSH_SUBSHELL` when it ran. A `$( … )` runs in-process and throws its
    /// function definitions away on return, while the options it registered
    /// here stay; a load made deeper than the current depth is redone.
    depth: i32,
    /// the command's option count before the file added its own
    options_before: usize,
}

#[derive(Default)]
struct Store {
    /// keyed by (command, is_path)
    map: BTreeMap<(String, bool), Entry>,
    next_order: usize,
    /// wrapping command → wrapped commands, in insertion order
    wrappers: BTreeMap<String, Vec<String>>,
    /// commands whose completions were erased explicitly: no autoload
    tombstones: BTreeSet<String>,
    /// commands whose `<cmd>.fish` was already looked for: the subshell depth
    /// it was sourced at and how many options the command had before
    loaded: BTreeMap<String, Loaded>,
    /// functions with no `<name>.fish` anywhere
    fn_missing: BTreeSet<String>,
}

fn store() -> std::sync::MutexGuard<'static, Store> {
    static STORE: OnceLock<Mutex<Store>> = OnceLock::new();
    let m = STORE.get_or_init(|| Mutex::new(Store::default()));
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// fish `complete_add`.
#[allow(clippy::too_many_arguments)]
fn complete_add(cmd: &str, is_path: bool, option: &str, typ: OptType, mode: Mode, conditions: &[String], comp: &str, desc: &str, keep_order: bool) {
    let mut st = store();
    let key = (cmd.to_string(), is_path);
    if !st.map.contains_key(&key) {
        let order = st.next_order;
        st.next_order += 1;
        st.map.insert(key.clone(), Entry { options: Vec::new(), order });
    }
    let e = st.map.get_mut(&key).expect("inserted above");
    e.options.push(Opt {
        option: option.to_string(),
        typ,
        mode,
        comp: comp.to_string(),
        desc: desc.to_string(),
        conditions: conditions.to_vec(),
        keep_order,
    });
}

/// fish `complete_remove`.
fn complete_remove(cmd: &str, is_path: bool, option: &str, typ: OptType) {
    let mut st = store();
    let key = (cmd.to_string(), is_path);
    if let Some(e) = st.map.get_mut(&key) {
        e.options.retain(|o| o.option != option || o.typ != typ);
        if e.options.is_empty() {
            st.map.remove(&key);
        }
    }
}

/// fish `complete_remove_all`.
fn complete_remove_all(cmd: &str, is_path: bool) {
    let mut st = store();
    let removed = st.map.remove(&(cmd.to_string(), is_path)).is_some();
    st.wrappers.remove(cmd);
    if !removed && !is_path {
        st.tombstones.insert(cmd.to_string());
    }
}

/// fish `complete_add_wrapper`.
fn complete_add_wrapper(cmd: &str, target: &str) {
    if cmd.is_empty() || target.is_empty() || cmd == target {
        return;
    }
    let mut st = store();
    let t = st.wrappers.entry(cmd.to_string()).or_default();
    if !t.iter().any(|x| x == target) {
        t.push(target.to_string());
    }
}

/// fish `complete_remove_wrapper`.
fn complete_remove_wrapper(cmd: &str, target: &str) {
    let mut st = store();
    if let Some(t) = st.wrappers.get_mut(cmd) {
        t.retain(|x| x != target);
    }
}

fn wrap_targets(cmd: &str) -> Vec<String> {
    store().wrappers.get(cmd).cloned().unwrap_or_default()
}

/// fish common `escape_string_script` with default flags (`escape`): the
/// backslash form of [`escape_for_insert`], but a string that needs escaping
/// and has no control character goes in quotes instead, `"` when it holds
/// more `'` than `"` and `$` together, else `'`; the empty string is `''`.
fn escape(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    let plain = escape_for_insert(s);
    let complex = s.chars().any(|c| matches!(c, '\x00'..='\x19' | '\x7f'));
    if plain == s || complex {
        return plain;
    }
    let count = |x: char| s.chars().filter(|&c| c == x).count();
    let quote = if count('\'') > count('"') + count('$') { '"' } else { '\'' };
    // escape_string_with_quote
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '$' if quote == '"' => out.push_str("\\$"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// fish `escape_string_script` with `no_quoted` (what the reader inserts for a
/// completion, `completion_apply_to_command_line`): backslashes only, control
/// characters as `\n` `\t` `\cX` `\xHH`. A `#` after an alphanumeric and a
/// `?` (fish 4's qmark-noglob) stay bare; `~` is escaped.
fn escape_for_insert(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_alnum = false;
    for c in s.chars() {
        match c {
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\x08' => out.push_str("\\b"),
            '\r' => out.push_str("\\r"),
            '\x1b' => out.push_str("\\e"),
            '\x7f' => out.push_str("\\x7f"),
            '\\' | '\'' => {
                out.push('\\');
                out.push(c);
            }
            '#' if prev_alnum => out.push(c),
            '?' => out.push(c),
            '&' | '$' | ' ' | '#' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '*' | '|' | ';' | '"' | '%'
            | '~' => {
                out.push('\\');
                out.push(c);
            }
            '\x01'..='\x19' => {
                out.push_str("\\c");
                out.push((b'a' + c as u8 - 1) as char);
            }
            '\x00' => out.push_str("\\x00"),
            c => out.push(c),
        }
        prev_alnum = c.is_alphanumeric();
    }
    out
}

/// fish `completion2string` / `complete_print`.
fn complete_print(cmd: &str) -> String {
    let st = store();
    let mut entries: Vec<_> = st.map.iter().collect();
    entries.sort_by_key(|(_, e)| e.order);
    let mut out = String::new();
    for ((name, is_path), e) in entries {
        if !cmd.is_empty() && name != cmd {
            continue;
        }
        for o in e.options.iter().rev() {
            out.push_str("complete");
            if o.keep_order {
                out.push_str(" -k");
            }
            let m = o.mode;
            if m.no_files && m.requires_param {
                out.push_str(" --exclusive");
            } else if m.no_files {
                out.push_str(" --no-files");
            } else if m.force_files {
                out.push_str(" --force-files");
            } else if m.requires_param {
                out.push_str(" --require-parameter");
            }
            if *is_path {
                out.push_str(&format!(" -p {}", escape(name)));
            } else {
                out.push(' ');
                out.push_str(&escape(name));
            }
            let flag = match o.typ {
                OptType::ArgsOnly => None,
                OptType::Short => Some('s'),
                OptType::SingleLong => Some('o'),
                OptType::DoubleLong => Some('l'),
            };
            if let Some(f) = flag {
                out.push_str(&format!(" -{f} {}", escape(&o.option)));
            }
            for (f, v) in [('d', &o.desc), ('a', &o.comp)] {
                if !v.is_empty() {
                    out.push_str(&format!(" -{f} {}", escape(v)));
                }
            }
            for c in &o.conditions {
                out.push_str(&format!(" -n {}", escape(c)));
            }
            out.push('\n');
        }
    }
    for (src, targets) in &st.wrappers {
        if !cmd.is_empty() && src != cmd {
            continue;
        }
        for t in targets {
            out.push_str(&format!("complete {} --wraps {}\n", escape(src), escape(t)));
        }
    }
    out
}

// ---------------------------------------------------------------------------
// The `complete` builtin (fish `src/builtins/complete.rs`).
// ---------------------------------------------------------------------------

/// Long spellings of `complete`'s options and whether they take a value:
/// 0 none, 1 required, 2 optional (attached with `=` only).
const LONG_OPTS: &[(&str, char, u8)] = &[
    ("exclusive", 'x', 0),
    ("no-files", 'f', 0),
    ("force-files", 'F', 0),
    ("require-parameter", 'r', 0),
    ("path", 'p', 1),
    ("command", 'c', 1),
    ("short-option", 's', 1),
    ("long-option", 'l', 1),
    ("old-option", 'o', 1),
    ("description", 'd', 1),
    ("arguments", 'a', 1),
    ("erase", 'e', 0),
    ("unauthoritative", 'u', 0),
    ("authoritative", 'A', 0),
    ("condition", 'n', 1),
    ("wraps", 'w', 1),
    ("do-complete", 'C', 2),
    ("help", 'h', 0),
    ("keep-order", 'k', 0),
    ("escape", '\x01', 0),
    ("color", '\x02', 1),
];

/// Short options: the letter and 0 none, 1 required, 2 optional.
const SHORT_OPTS: &[(char, u8)] = &[
    ('a', 1),
    ('c', 1),
    ('p', 1),
    ('s', 1),
    ('l', 1),
    ('o', 1),
    ('d', 1),
    ('f', 0),
    ('F', 0),
    ('r', 0),
    ('x', 0),
    ('e', 0),
    ('u', 0),
    ('A', 0),
    ('n', 1),
    ('C', 2),
    ('w', 1),
    ('h', 0),
    ('k', 0),
];

/// GNU-style option scan (fish's `WGetopter` permutes operands to the end):
/// the `(letter, value)` pairs in order, then the operands.
fn getopt(args: &[String]) -> Result<(Vec<(char, Option<String>)>, Vec<String>), String> {
    let mut opts = Vec::new();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        i += 1;
        if a == "--" {
            rest.extend(args[i..].iter().cloned());
            break;
        }
        if let Some(l) = a.strip_prefix("--") {
            let (name, inline) = match l.split_once('=') {
                Some((n, v)) => (n, Some(v.to_string())),
                None => (l, None),
            };
            // fish wgetopt `find_matching_long_opt`: an exact name, else the one
            // option the text abbreviates (ansible.fish writes `--l poll`)
            let exact = LONG_OPTS.iter().find(|(n, _, _)| *n == name);
            let mut prefixed = LONG_OPTS.iter().filter(|(n, _, _)| !name.is_empty() && n.starts_with(name));
            let found = match (exact, prefixed.next(), prefixed.next()) {
                (Some(o), _, _) => o,
                (None, Some(o), None) => o,
                (None, Some(_), Some(_)) => return Err(format!("complete: {a}: option is ambiguous")),
                _ => return Err(format!("complete: {a}: unknown option")),
            };
            let &(_, c, kind) = found;
            let val = match kind {
                1 => match inline {
                    Some(v) => Some(v),
                    None if i < args.len() => {
                        i += 1;
                        Some(args[i - 1].clone())
                    }
                    None => return Err(format!("complete: {a}: option requires an argument")),
                },
                2 => inline,
                _ => None,
            };
            opts.push((c, val));
            continue;
        }
        if a.len() > 1 && a.starts_with('-') {
            let chars: Vec<char> = a.chars().skip(1).collect();
            let mut j = 0;
            while j < chars.len() {
                let c = chars[j];
                let Some(&(_, kind)) = SHORT_OPTS.iter().find(|(s, _)| *s == c) else {
                    return Err(format!("complete: -{c}: unknown option"));
                };
                let tail: String = chars[j + 1..].iter().collect();
                match kind {
                    0 => {
                        opts.push((c, None));
                        j += 1;
                    }
                    1 => {
                        let v = if !tail.is_empty() {
                            tail
                        } else if i < args.len() {
                            i += 1;
                            args[i - 1].clone()
                        } else {
                            return Err(format!("complete: -{c}: option requires an argument"));
                        };
                        opts.push((c, Some(v)));
                        break;
                    }
                    _ => {
                        opts.push((c, (!tail.is_empty()).then_some(tail)));
                        break;
                    }
                }
            }
            continue;
        }
        rest.push(a.clone());
    }
    Ok((opts, rest))
}

/// `complete …` — fish's builtin. `argv[0]` is the command name.
pub fn builtin_complete(argv: &[String]) -> i32 {
    let (opts, rest) = match getopt(&argv[1..]) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{msg}");
            return 2;
        }
    };
    let mut mode = Mode::default();
    let mut have_x = false;
    let mut remove = false;
    let mut keep_order = false;
    let mut short = String::new();
    let mut gnu: Vec<String> = Vec::new();
    let mut old: Vec<String> = Vec::new();
    let mut comp = String::new();
    let mut desc = String::new();
    let mut conditions: Vec<String> = Vec::new();
    let mut cmds: Vec<String> = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    let mut wraps: Vec<String> = Vec::new();
    let mut do_complete = false;
    let mut do_complete_param: Option<String> = None;
    let mut unescape_output = true;
    for (c, v) in opts {
        let v = v.unwrap_or_default();
        match c {
            'x' => {
                mode.no_files = true;
                mode.requires_param = true;
                have_x = true;
            }
            'f' => mode.no_files = true,
            'F' => mode.force_files = true,
            'r' => mode.requires_param = true,
            'k' => keep_order = true,
            'p' => paths.push(v),
            'c' => cmds.push(v),
            'd' => desc = v,
            's' | 'l' | 'o' if v.is_empty() => {
                eprintln!("complete: -{c} requires a non-empty string");
                return 2;
            }
            's' => short.push_str(&v),
            'l' => gnu.push(v),
            'o' => old.push(v),
            'a' => comp = v,
            'e' => remove = true,
            'n' => conditions.push(v),
            'w' => wraps.push(v),
            'C' => {
                do_complete = true;
                if !v.is_empty() {
                    do_complete_param = Some(v);
                }
            }
            '\x01' => unescape_output = false,
            _ => {}
        }
    }
    if mode.no_files && mode.force_files {
        let which = if have_x { "'--exclusive' and '--force-files'" } else { "'--no-files' and '--force-files'" };
        eprintln!("complete: invalid option combination, {which}");
        return 2;
    }
    if !rest.is_empty() {
        if do_complete && do_complete_param.is_none() && rest.len() == 1 {
            do_complete_param = Some(rest[0].clone());
        } else if !do_complete && cmds.is_empty() && rest.len() == 1 {
            cmds.push(rest[0].clone());
        } else {
            eprintln!("complete: too many arguments");
            return 2;
        }
    }

    if do_complete {
        // fish: with no string to complete, `-C` needs the live command line
        let line = match do_complete_param {
            Some(p) => p,
            None if transient().is_empty() && !crate::ported::zsh_h::interact() => {
                eprintln!("complete: Can not get commandline in non-interactive mode");
                return 1;
            }
            None => transient_or_empty(),
        };
        let _ = unescape_output;
        tracing::debug!(target: "fish_complete", line = %line, "complete -C");
        let mut out = String::new();
        for c in complete_dash_c(&line) {
            out.push_str(&c.word);
            if !c.desc.is_empty() {
                out.push('\t');
                out.push_str(&c.desc);
            }
            out.push('\n');
        }
        print!("{out}");
        return 0;
    }

    let nothing_given = paths.is_empty()
        && gnu.is_empty()
        && short.is_empty()
        && old.is_empty()
        && !remove
        && comp.is_empty()
        && desc.is_empty()
        && conditions.is_empty()
        && wraps.is_empty()
        && !mode.no_files
        && !mode.force_files
        && !mode.requires_param;
    if nothing_given {
        if cmds.is_empty() {
            print!("{}", complete_print(""));
        } else {
            for c in &cmds {
                print!("{}", complete_print(c));
            }
        }
        return 0;
    }

    let targets: Vec<(String, bool)> = cmds
        .iter()
        .map(|c| (c.clone(), false))
        .chain(paths.iter().map(|p| (p.clone(), true)))
        .collect();
    for (cmd, is_path) in &targets {
        if remove {
            let mut removed = false;
            for s in short.chars() {
                complete_remove(cmd, *is_path, &s.to_string(), OptType::Short);
                removed = true;
            }
            for o in &old {
                complete_remove(cmd, *is_path, o, OptType::SingleLong);
                removed = true;
            }
            for g in &gnu {
                complete_remove(cmd, *is_path, g, OptType::DoubleLong);
                removed = true;
            }
            for w in &wraps {
                complete_remove_wrapper(cmd, w);
                removed = true;
            }
            if !removed {
                complete_remove_all(cmd, *is_path);
            }
            continue;
        }
        for s in short.chars() {
            complete_add(cmd, *is_path, &s.to_string(), OptType::Short, mode, &conditions, &comp, &desc, keep_order);
        }
        for g in &gnu {
            complete_add(cmd, *is_path, g, OptType::DoubleLong, mode, &conditions, &comp, &desc, keep_order);
        }
        for o in &old {
            complete_add(cmd, *is_path, o, OptType::SingleLong, mode, &conditions, &comp, &desc, keep_order);
        }
        if short.is_empty() && gnu.is_empty() && old.is_empty() {
            complete_add(cmd, *is_path, "", OptType::ArgsOnly, mode, &conditions, &comp, &desc, keep_order);
        }
    }
    if !remove {
        for w in &wraps {
            for c in &cmds {
                complete_add_wrapper(c, w);
            }
        }
    }
    0
}

// ---------------------------------------------------------------------------
// fish tokens: just enough of fish's tokenizer for the command line before the
// cursor (quotes, escapes, `(…)` / `$(…)` / `{…}` nesting, separators,
// redirections, comments).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum TokKind {
    Str,
    /// `;` `&` `|` `&&` `||` newline
    Sep,
    Redirect,
}

#[derive(Clone, Copy, Debug)]
struct Tok {
    start: usize,
    end: usize,
    kind: TokKind,
}

/// fish tokenizer.rs `PipeOrRedir::try_from` for an operator starting at `i`
/// (`|`, `&|`, `&&`, `||` are taken as separators before this is asked): its
/// end and whether it is a redirection or a pipe. `>|` / `2>|` are pipes in
/// fish (stdout / that fd into the next command), not bash's clobbering
/// redirection; `>&`'s fd target (`2>&1`) is a token of its own.
fn is_redirect_at(s: &[char], i: usize) -> Option<(usize, TokKind)> {
    let mut j = i;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
    }
    let has_fd = j > i;
    let at = |k: usize, c: char| s.get(k) == Some(&c);
    match s.get(j)? {
        '>' => {
            j += 1;
            if at(j, '>') {
                j += 1;
            }
            if at(j, '|') {
                return Some((j + 1, TokKind::Sep));
            }
            if at(j, '&') || at(j, '?') {
                j += 1;
            }
        }
        '<' => {
            j += 1;
            if at(j, '&') || at(j, '?') {
                j += 1;
            }
        }
        '&' if !has_fd && at(j + 1, '>') => {
            j += 2;
            if at(j, '>') {
                j += 1;
            }
            if at(j, '?') {
                j += 1;
            }
        }
        _ => return None,
    }
    Some((j, TokKind::Redirect))
}

/// fish parse_util `get_quote` at the end of the line: the quote left open in
/// the token the cursor ends, if any (`echo "a $US` → `"`).
fn open_quote(line: &str) -> Option<char> {
    let s: Vec<char> = line.chars().collect();
    let tok = tokenize(&s).into_iter().last().filter(|t| t.end == s.len())?;
    let mut quote = None;
    let mut i = tok.start;
    while i < tok.end {
        match (quote, s[i]) {
            (_, '\\') => i += 1,
            (None, q @ ('"' | '\'')) => quote = Some(q),
            (Some(q), c) if c == q => quote = None,
            _ => {}
        }
        i += 1;
    }
    quote
}

/// Tokenize `s`, accepting an unfinished last token (open quote or bracket).
fn tokenize(s: &[char]) -> Vec<Tok> {
    tokenize_ending(s).0
}

/// [`tokenize`], and whether `s` ends inside a comment.
fn tokenize_ending(s: &[char]) -> (Vec<Tok>, bool) {
    let mut out = Vec::new();
    let mut in_comment = false;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        if c == ' ' || c == '\t' {
            i += 1;
            continue;
        }
        if c == '\\' && s.get(i + 1) == Some(&'\n') {
            i += 2;
            continue;
        }
        if c == '#' {
            while i < s.len() && s[i] != '\n' {
                i += 1;
            }
            in_comment = i == s.len();
            continue;
        }
        if c == '\n' || c == ';' {
            out.push(Tok { start: i, end: i + 1, kind: TokKind::Sep });
            i += 1;
            continue;
        }
        if c == '|' || (c == '&' && s.get(i + 1) != Some(&'>')) {
            let len = if s.get(i + 1) == Some(&c) { 2 } else { 1 };
            out.push(Tok { start: i, end: i + len, kind: TokKind::Sep });
            i += len;
            continue;
        }
        if let Some((end, kind)) = is_redirect_at(s, i) {
            out.push(Tok { start: i, end, kind });
            i = end;
            continue;
        }
        let start = i;
        let mut paren = 0usize;
        let mut brace = 0usize;
        while i < s.len() {
            let c = s[i];
            match c {
                '\\' => {
                    i += 2;
                    continue;
                }
                '\'' => {
                    i += 1;
                    while i < s.len() && s[i] != '\'' {
                        if s[i] == '\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                '"' => {
                    i += 1;
                    while i < s.len() && s[i] != '"' {
                        if s[i] == '\\' {
                            i += 1;
                        } else if s[i] == '$' && s.get(i + 1) == Some(&'(') {
                            // `"$(…)"` nests; skip to its close
                            let close = matching_paren(s, i + 1);
                            i = close;
                        }
                        i += 1;
                    }
                }
                '(' => paren += 1,
                ')' if paren > 0 => paren -= 1,
                '{' => brace += 1,
                '}' if brace > 0 => brace -= 1,
                _ if paren > 0 => {}
                ' ' | '\t' if brace > 0 => {}
                ' ' | '\t' | '\n' | ';' | '|' => break,
                ')' => break,
                '&' if matches!(s.get(i + 1), None | Some(' ' | '\t' | '\n' | '&' | ';')) => break,
                '<' | '>' => break,
                _ => {}
            }
            i += 1;
        }
        let end = i.min(s.len());
        if end > start {
            out.push(Tok { start, end, kind: TokKind::Str });
        }
        if i < s.len() && s[i] == ')' {
            i += 1;
        }
    }
    (out, in_comment)
}

/// Index of the `)` closing the `(` at `open`, or `s.len()` when unclosed.
fn matching_paren(s: &[char], open: usize) -> usize {
    let mut depth = 0usize;
    let mut i = open;
    let mut quote: Option<char> = None;
    while i < s.len() {
        let c = s[i];
        match quote {
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\\' => i += 1,
                '\'' | '"' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    s.len()
}

/// Start of the innermost command substitution (`(…)` / `$(…)`) still open at
/// the end of `s` — fish `get_cmdsubst_extent` for a cursor at the end.
fn innermost_cmdsubst_start(s: &[char]) -> usize {
    let mut stack: Vec<usize> = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match quote {
            Some('\'') => {
                if c == '\\' {
                    i += 1;
                } else if c == '\'' {
                    quote = None;
                }
            }
            Some(_) => {
                if c == '\\' {
                    i += 1;
                } else if c == '"' {
                    quote = None;
                } else if c == '$' && s.get(i + 1) == Some(&'(') {
                    stack.push(i + 2);
                    quote = None;
                    i += 1;
                }
            }
            None => match c {
                '\\' => i += 1,
                '\'' | '"' => quote = Some(c),
                '(' => stack.push(i + 1),
                ')' => {
                    stack.pop();
                }
                _ => {}
            },
        }
        i += 1;
    }
    stack.last().copied().unwrap_or(0)
}

/// fish `PROG_COMPLETE_SEP`: separates a completion from its description.
const PROG_COMPLETE_SEP: char = '\t';

/// fish `ANY_STRING`: an unquoted `*` in a word being completed, told apart from
/// a quoted or escaped one (see [`unescape_wild`]).
const ANY_STRING: char = '\u{F701}';
/// fish `BRACE_BEGIN` / `BRACE_END` / `BRACE_SEP` / `BRACE_SPACE`: an unquoted
/// `{`, `}`, `,` and space of a brace expansion in a word being completed.
const BRACE_BEGIN: char = '\u{F702}';
const BRACE_END: char = '\u{F703}';
const BRACE_SEP: char = '\u{F704}';
const BRACE_SPACE: char = '\u{F705}';
/// fish `VARIABLE_EXPAND`: a `$` that expands (unquoted or in double quotes).
const VARIABLE_EXPAND: char = '\u{F706}';

/// A word from [`unescape_wild`] as typed, its markers back to the characters.
fn wild_text(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            ANY_STRING => '*',
            BRACE_BEGIN => '{',
            BRACE_END => '}',
            BRACE_SEP => ',',
            BRACE_SPACE => ' ',
            VARIABLE_EXPAND => '$',
            c => c,
        })
        .collect()
}

/// fish's script unescape, incomplete-tolerant: quotes removed, backslash
/// escapes decoded. Variables, `~` and substitutions stay as written.
fn unescape(tok: &str) -> String {
    unescape_impl(tok, false)
}

/// [`unescape`], with each unquoted `*` as [`ANY_STRING`] and the braces of
/// a brace expansion as [`BRACE_BEGIN`] … (fish's `unescape_string` with
/// `UNESCAPE_SPECIAL`). `?` is literal: fish 4 has `qmark-noglob` on.
fn unescape_wild(tok: &str) -> String {
    unescape_impl(tok, true)
}

fn unescape_impl(tok: &str, wild: bool) -> String {
    let s: Vec<char> = tok.chars().collect();
    let mut out: Vec<char> = Vec::new();
    // fish common `unescape_string_internal`: the open braces (where each ended
    // up in `out`), and where in the input each `,` or `$` inside them was
    let (mut braces, mut vars_or_seps): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
    let mut brace_count = 0usize;
    let mut i = 0;
    while i < s.len() {
        let c = s[i];
        match c {
            '\\' => {
                i += 1;
                let Some(&n) = s.get(i) else { break };
                out.push(match n {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    'e' => '\x1b',
                    'a' => '\x07',
                    'b' => '\x08',
                    'f' => '\x0c',
                    'v' => '\x0b',
                    '\n' => {
                        i += 1;
                        continue;
                    }
                    other => other,
                });
            }
            '\'' => {
                i += 1;
                while i < s.len() && s[i] != '\'' {
                    if s[i] == '\\' && matches!(s.get(i + 1), Some('\\' | '\'')) {
                        i += 1;
                    }
                    out.push(s[i]);
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < s.len() && s[i] != '"' {
                    if s[i] == '\\' && matches!(s.get(i + 1), Some('\\' | '"' | '$' | '\n')) {
                        i += 1;
                        if s[i] == '\n' {
                            i += 1;
                            continue;
                        }
                    } else if wild && s[i] == '$' && s.get(i + 1) != Some(&'(') {
                        out.push(VARIABLE_EXPAND);
                        i += 1;
                        continue;
                    }
                    out.push(s[i]);
                    i += 1;
                }
            }
            '*' if wild => out.push(ANY_STRING),
            '$' if wild && s.get(i + 1) != Some(&'(') => {
                vars_or_seps.push(i);
                out.push(VARIABLE_EXPAND);
            }
            '{' if wild => {
                brace_count += 1;
                braces.push(out.len());
                out.push(BRACE_BEGIN);
            }
            '}' if wild => {
                brace_count = brace_count.saturating_sub(1);
                let mut end = BRACE_END;
                if let Some(brace) = braces.pop() {
                    // fish #4632: a brace with zero or one items is literal.
                    // (fish compares the input position of the last `,`/`$`
                    // with the output position of the brace; so does this.)
                    if vars_or_seps.last().is_none_or(|&p| p < brace) {
                        out[brace] = '{';
                        for c in &mut out[brace + 1..] {
                            if *c == BRACE_SPACE {
                                *c = ' ';
                            }
                        }
                        end = '}';
                    }
                    while vars_or_seps.last().is_some_and(|&p| p > brace) {
                        vars_or_seps.pop();
                    }
                }
                out.push(end);
            }
            ',' if wild && brace_count > 0 => {
                vars_or_seps.push(i);
                out.push(BRACE_SEP);
            }
            ' ' if wild && brace_count > 0 => out.push(BRACE_SPACE),
            _ => out.push(c),
        }
        i += 1;
    }
    out.into_iter().collect()
}

/// fish expand.rs `expand_braces` for a word being completed: every word the
/// brace expansions in `input` (from [`unescape_wild`]) make. A brace left
/// open gets its closing brace made up, keeping only the last item.
fn expand_braces(input: &[char], for_completions: bool) -> Vec<String> {
    let (mut count, mut begin, mut end, mut last_sep) = (0i32, None, None, None);
    for (pos, &c) in input.iter().enumerate() {
        match c {
            BRACE_BEGIN => {
                if count == 0 {
                    begin = Some(pos);
                }
                count += 1;
            }
            BRACE_END => {
                count -= 1;
                if count < 0 {
                    return Vec::new();
                }
                if count == 0 {
                    end = Some(pos);
                }
            }
            BRACE_SEP if count == 1 => last_sep = Some(pos),
            _ => {}
        }
    }
    if count > 0 {
        if !for_completions {
            return Vec::new();
        }
        let b = begin.expect("an open brace");
        let mut synth: Vec<char> = match last_sep {
            Some(sep) => input[..=b].iter().chain(&input[sep + 1..]).copied().collect(),
            None => input.to_vec(),
        };
        synth.push(BRACE_END);
        return expand_braces(&synth, false);
    }
    let (Some(begin), Some(end)) = (begin, end) else {
        return vec![input.iter().map(|&c| if c == BRACE_SPACE { ' ' } else { c }).collect()];
    };
    let mut out = Vec::new();
    let (mut depth, mut item_begin) = (0, begin + 1);
    for pos in begin + 1..=end {
        let c = input[pos];
        if depth == 0 && (c == BRACE_SEP || pos == end) {
            let trim = |c: &char| *c == BRACE_SPACE || *c == '\0';
            let item = &input[item_begin..pos];
            let lead = item.iter().take_while(|c| trim(c)).count();
            let tail = item[lead..].iter().rev().take_while(|c| trim(c)).count();
            let whole: Vec<char> = input[..begin]
                .iter()
                .chain(&item[lead..item.len() - tail])
                .chain(&input[end + 1..])
                .copied()
                .collect();
            out.extend(expand_braces(&whole, for_completions));
            item_begin = pos + 1;
        }
        match c {
            BRACE_BEGIN => depth += 1,
            BRACE_END => depth -= 1,
            _ => {}
        }
    }
    out
}

/// fish `variable_assignment_equals_pos`: `NAME=…` with a valid name.
fn is_var_assignment(tok: &str) -> bool {
    match tok.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name.chars().all(|c| c.is_alphanumeric() || c == '_')
        }
        None => false,
    }
}


/// fish `CmdString::new`: the command's path — searched on `$PATH` for a
/// bare name, made absolute when relative (`wrealpath`) — or `None` when it
/// is not found. `complete -p` entries compare against this path and `-c`
/// entries against its basename, both for equality: a `*` in a name is a
/// literal character (fish's `wildcard_match` needs its internal wildcard
/// tokens, which a quoted `-p '*clang*'` does not carry, so clang.fish's rule
/// matches nothing in fish either).
fn command_path(cmd: &str) -> Option<String> {
    let found = if cmd.contains('/') {
        Path::new(cmd).is_file().then(|| cmd.to_string())
    } else {
        let path = crate::ported::params::getsparam("PATH").unwrap_or_default();
        path.split(':')
            .filter(|d| !d.is_empty())
            .map(|d| format!("{d}/{cmd}"))
            .find(|p| is_executable(p))
    }?;
    if found.starts_with('/') {
        Some(found)
    } else {
        std::fs::canonicalize(&found).ok().map(|p| p.to_string_lossy().into_owned())
    }
}

// ---------------------------------------------------------------------------
// The engine (fish `Completer`).
// ---------------------------------------------------------------------------

/// One completion: the whole word to insert, its description, and whether a
/// space should follow (fish `WantsSuffix`).
#[derive(Clone, Debug, PartialEq)]
pub struct Comp {
    pub word: String,
    pub desc: String,
    pub no_space: bool,
    /// from a `-k` definition: keeps its place instead of being sorted
    pub keep: bool,
    /// What fish matches the typed word against when it is not the word
    /// itself: an `-a` result is matched with its `\tdescription` still on
    /// (fish wildcard_complete), so `ksh --` finds `-e\tfail on non-zero…`.
    pub matched_as: Option<String>,
    /// The rank this completion can be no better than: a path reached through
    /// a fuzzily matched directory (`s/e/f` → `src/extensions/fish.rs`) is at
    /// least as fuzzy as that directory's match (fish
    /// `expand_literal_intermediate_segment_with_fuzz`).
    pub rank_floor: u32,
    /// Completed after a `=` or `:` in the word (`--file=RE`, `a:b`): fish ranks
    /// these below every other match (`StringFuzzyMatch::rank`, `from_separator
    /// << 4`), so an option's own values (`--backup=none`) win over files.
    pub from_separator: bool,
}

/// What a query found.
#[derive(Debug, Default)]
pub struct Answer {
    pub comps: Vec<Comp>,
    /// file completions apply too
    pub do_file: bool,
    /// a `-k` definition contributed: keep the order
    pub keep_order: bool,
    /// the command is `cd` or wraps it: its argument completes directories
    pub cd: bool,
}

/// How fish code is run: `test` answers a `-n` condition, `args` expands an
/// `-a` argument list into words (`word\tdescription`).
trait Eval {
    fn test(&mut self, cond: &str) -> bool;
    fn args(&mut self, args: &str) -> Vec<String>;
    /// Make sure `<cmd>.fish` was looked for.
    fn load(&mut self, cmd: &str);
}

struct Completer<'a> {
    eval: &'a mut dyn Eval,
    cond_cache: HashMap<String, bool>,
    out: Answer,
    /// any fish definition covered the command or a command it wraps
    covered: bool,
}

/// fish `resolve_auto_space`.
fn auto_no_space(word: &str) -> bool {
    matches!(word.chars().last(), Some('/' | '=' | '@' | ':' | '.' | ',' | '-'))
}

fn leading_dash_count(s: &str) -> usize {
    s.chars().take_while(|&c| c == '-').count()
}

/// fish `param_match`.
fn param_match(o: &Opt, optstr: &str) -> bool {
    if o.typ == OptType::ArgsOnly {
        return false;
    }
    let d = leading_dash_count(optstr);
    d == o.expected_dash_count() && o.option == optstr[d..]
}

/// fish `param_match2`: `--color=auto` / `-std=c++26`, the offset after `=`.
fn param_match2(o: &Opt, optstr: &str) -> Option<usize> {
    if o.option.is_empty() {
        return None;
    }
    let d = leading_dash_count(optstr);
    if d != o.expected_dash_count() || !optstr[d..].starts_with(&o.option) {
        return None;
    }
    let at = d + o.option.len();
    (optstr[at..].starts_with('=')).then_some(at + 1)
}

impl Completer<'_> {
    fn condition_test(&mut self, c: &str) -> bool {
        if c.is_empty() {
            return true;
        }
        if let Some(&v) = self.cond_cache.get(c) {
            return v;
        }
        let v = self.eval.test(c);
        self.cond_cache.insert(c.to_string(), v);
        v
    }

    fn conditions_test(&mut self, cs: &[String]) -> bool {
        cs.iter().all(|c| self.condition_test(c))
    }

    /// fish `short_option_pos`, positions in chars.
    fn short_option_pos(&mut self, arg: &[char], options: &[Opt]) -> Option<usize> {
        if arg.len() <= 1 || arg[0] != '-' || arg.get(1) == Some(&'-') {
            return None;
        }
        for (pos, &ch) in arg.iter().enumerate().skip(1) {
            let mut requires_param = None;
            for o in options {
                if o.typ == OptType::Short && o.option.starts_with(ch) && self.conditions_test(&o.conditions) {
                    requires_param = Some(o.mode.requires_param);
                    break;
                }
            }
            match requires_param {
                Some(true) => return Some(pos),
                Some(false) => {}
                None if pos == 1 => return None,
                None => return Some(pos - 1),
            }
        }
        Some(arg.len() - 1)
    }

    /// fish `complete_from_args`; every word gets `prefix` in front.
    fn from_args(&mut self, prefix: &str, args: &str, desc: &str, keep_order: bool) {
        if args.is_empty() {
            return;
        }
        if keep_order {
            self.out.keep_order = true;
        }
        for w in self.eval.args(args) {
            let raw = format!("{prefix}{w}");
            let (word, d) = match w.split_once('\t') {
                Some((w, d)) => (w.to_string(), d.to_string()),
                None => (w, desc.to_string()),
            };
            if word.is_empty() {
                continue;
            }
            let word = format!("{prefix}{word}");
            let no_space = auto_no_space(&word);
            self.out.comps.push(Comp {
                word,
                desc: d,
                no_space,
                keep: keep_order,
                matched_as: Some(raw),
                rank_floor: 0,
                from_separator: false,
            });
        }
    }

    /// fish `complete_param_for_command`: `s` is the argument being completed,
    /// `popt` the one before it.
    fn param_for_command(&mut self, cmd: &str, popt: &str, s: &str, use_switches: bool, do_file: &mut bool) {
        let mut use_files = true;
        let mut has_force = false;
        let path = command_path(cmd);
        let name = path
            .as_deref()
            .map_or(cmd, |p| p.rsplit('/').next().unwrap_or(p))
            .to_string();
        self.eval.load(&name);
        let all: Vec<Vec<Opt>> = {
            let st = store();
            if st.tombstones.contains(&name) {
                self.covered = true;
            }
            st.map
                .iter()
                .filter(|((n, is_path), _)| if *is_path { path.as_deref() == Some(n.as_str()) } else { *n == name })
                .map(|(_, e)| e.options.iter().rev().cloned().collect())
                .collect()
        };
        if !all.is_empty() {
            self.covered = true;
        }
        let sc: Vec<char> = s.chars().collect();
        let pc: Vec<char> = popt.chars().collect();
        for options in all {
            let short_pos = self.short_option_pos(&sc, &options);
            let mut last_requires_param: Option<bool> = None;
            let mut use_common = true;
            if use_switches {
                if s.starts_with('-') {
                    // a combined option and argument: `--color=auto`, `-I/usr/include`
                    for o in &options {
                        let arg_offset = if o.typ == OptType::Short {
                            let Some(p) = short_pos else { continue };
                            if !o.option.starts_with(sc[p]) {
                                continue;
                            }
                            Some(p + 1)
                        } else {
                            param_match2(o, s).map(|b| s[..b].chars().count())
                        };
                        if !self.conditions_test(&o.conditions) {
                            continue;
                        }
                        if o.typ == OptType::Short {
                            let r = last_requires_param.get_or_insert(o.mode.requires_param);
                            *r &= o.mode.requires_param;
                        }
                        if let Some(off) = arg_offset {
                            if o.mode.requires_param {
                                use_common = false;
                            }
                            if o.mode.no_files {
                                use_files = false;
                            }
                            if o.mode.force_files {
                                has_force = true;
                            }
                            let prefix: String = sc[..off].iter().collect();
                            self.from_args(&prefix, &o.comp, &o.desc, o.keep_order);
                        }
                    }
                } else if popt.starts_with('-') {
                    // the previous word may be an option that takes this one
                    for o in &options {
                        if o.typ == OptType::SingleLong && param_match(o, popt) && self.conditions_test(&o.conditions) {
                            if o.mode.requires_param {
                                use_common = false;
                            }
                            if o.mode.no_files {
                                use_files = false;
                            }
                            if o.mode.force_files {
                                has_force = true;
                            }
                            self.from_args("", &o.comp, &o.desc, o.keep_order);
                        }
                    }
                    let prev_short_pos = self.short_option_pos(&pc, &options);
                    for o in &options {
                        if !o.mode.requires_param {
                            continue;
                        }
                        let matched = match o.typ {
                            OptType::Short => prev_short_pos
                                .is_some_and(|p| p + 1 == pc.len() && o.option.starts_with(pc[p])),
                            OptType::DoubleLong => param_match(o, popt),
                            _ => false,
                        };
                        if matched && self.conditions_test(&o.conditions) {
                            use_common = false;
                            if o.mode.no_files {
                                use_files = false;
                            }
                            if o.mode.force_files {
                                has_force = true;
                            }
                            self.from_args("", &o.comp, &o.desc, o.keep_order);
                        }
                    }
                }
            }
            if !use_common {
                continue;
            }
            let last_requires_param = last_requires_param.unwrap_or(false);
            for o in &options {
                if !self.conditions_test(&o.conditions) {
                    continue;
                }
                if o.option.is_empty() {
                    use_files &= !o.mode.no_files;
                    has_force |= o.mode.force_files;
                    self.from_args("", &o.comp, &o.desc, o.keep_order);
                }
                if !use_switches || s.is_empty() {
                    continue;
                }
                if o.typ == OptType::Short {
                    let ch = o.option.chars().next().unwrap_or('-');
                    match short_pos {
                        Some(p) => {
                            // only after the last short option, when it takes no
                            // parameter, and not twice
                            if p + 1 != sc.len() || last_requires_param || sc.contains(&ch) {
                                continue;
                            }
                        }
                        None if s != "-" => continue,
                        None => {}
                    }
                    self.out.comps.push(Comp {
                        word: format!("{s}{ch}"),
                        desc: o.desc.clone(),
                        no_space: false,
                        keep: false,
        matched_as: None,
        rank_floor: 0,
        from_separator: false,
                    });
                    continue;
                }
                if o.typ != OptType::SingleLong && o.typ != OptType::DoubleLong {
                    continue;
                }
                let whole = format!("{}{}", "-".repeat(o.expected_dash_count()), o.option);
                // fish matches the typed word against the option fuzzily (`-v` finds
                // `--verbose`); the best kind of match is chosen later
                if !s.starts_with('-') || fuzzy_rank(s, &whole).is_none() {
                    continue;
                }
                if o.typ == OptType::DoubleLong && !o.comp.is_empty() && !o.mode.requires_param {
                    // an optional argument is only reachable as `--switch=arg`
                    self.out.comps.push(Comp {
                        word: format!("{whole}="),
                        desc: o.desc.clone(),
                        no_space: true,
                        keep: false,
        matched_as: None,
        rank_floor: 0,
        from_separator: false,
                    });
                }
                self.out.comps.push(Comp {
                    word: whole,
                    desc: o.desc.clone(),
                    no_space: false,
                    keep: false,
        matched_as: None,
        rank_floor: 0,
        from_separator: false,
                });
            }
        }
        if has_force {
            *do_file = true;
        } else if !use_files {
            *do_file = false;
        }
    }

    /// fish `walk_wrap_chain` + `complete_custom`.
    #[allow(clippy::too_many_arguments)]
    fn walk_wrap_chain(&mut self, cmd: &str, cmdline: &str, cmd_range: (usize, usize), prev: &str, cur: &str, had_ddash: bool, do_file: &mut bool, depth: usize, visited: &mut HashSet<String>) {
        if depth > 24 {
            return;
        }
        {
            let _t = TransientGuard::set(cmdline);
            self.param_for_command(cmd, prev, cur, !had_ddash, do_file);
        }
        for wt in wrap_targets(cmd) {
            let wchars: Vec<char> = wt.chars().collect();
            let Some(tok) = tokenize(&wchars)
                .into_iter()
                .filter(|t| t.kind == TokKind::Str)
                .find(|t| !is_var_assignment(&wchars[t.start..t.end].iter().collect::<String>()))
            else {
                continue;
            };
            let wrapped = unescape(&wchars[tok.start..tok.end].iter().collect::<String>());
            if !visited.insert(wrapped.clone()) {
                continue;
            }
            let cl: Vec<char> = cmdline.chars().collect();
            let mut faux: Vec<char> = cl[..cmd_range.0].to_vec();
            faux.extend(wchars.iter());
            faux.extend(cl[cmd_range.1.min(cl.len())..].iter());
            let faux: String = faux.into_iter().collect();
            let range = (cmd_range.0 + tok.start, cmd_range.0 + tok.end);
            self.walk_wrap_chain(&wrapped, &faux, range, prev, cur, had_ddash, do_file, depth + 1, visited);
        }
    }
}

/// The parsed shape of the command line before the cursor.
struct Line {
    /// fish `effective_cmdline`: the line up to the cursor (from the command
    /// word when `VAR=` words before it were skipped)
    effective: String,
    cmd: String,
    /// the command word's extent in `effective`
    cmd_range: (usize, usize),
    prev: String,
    cur: String,
    /// `cur` with its unquoted `*`s as [`ANY_STRING`], for file completion
    cur_wild: String,
    /// the word as typed: where it splits at `=`/`:` is decided on it
    cur_raw: String,
    had_ddash: bool,
    in_redirection: bool,
}

/// What the cursor is on.
enum Position {
    /// completing the command word itself (or nothing typed yet)
    Command(String),
    /// the current word ends in a variable name (`$HO`, `foo$HO`, `"$HO`):
    /// `before` is the word up to and including the `$`, `name` the part typed
    Variable { before: String, name: String },
    /// `~us`: the part after the `~`
    User(String),
    /// nothing fish completes (a comment, a redirection where the command goes)
    Nothing,
    Argument(Line),
}

/// fish `try_complete_variable`: the index of the `$` whose variable name
/// runs to the end of `s`, quoting taken into account.
fn variable_start(s: &[char]) -> Option<usize> {
    #[derive(PartialEq)]
    enum Q {
        None,
        Single,
        Double,
    }
    let mut mode = Q::None;
    let mut start = None;
    let mut skip = false;
    for (i, &c) in s.iter().enumerate() {
        if skip {
            skip = false;
            continue;
        }
        if !(c.is_alphanumeric() || c == '_') {
            start = None;
        }
        match c {
            '\\' => skip = true,
            '$' if mode != Q::Single => start = Some(i),
            '\'' if mode == Q::Single => mode = Q::None,
            '\'' if mode == Q::None => mode = Q::Single,
            '"' if mode == Q::Double => mode = Q::None,
            '"' if mode == Q::None => mode = Q::Double,
            _ => {}
        }
    }
    start
}

/// The current word as a variable or `~user` completion, when it is one.
fn special_position(raw: &str) -> Option<Position> {
    let chars: Vec<char> = raw.chars().collect();
    if let Some(at) = variable_start(&chars) {
        return Some(Position::Variable {
            before: chars[..=at].iter().collect(),
            name: chars[at + 1..].iter().collect(),
        });
    }
    let user = raw.strip_prefix('~')?;
    (!user.contains(['/', '~'])).then(|| Position::User(user.to_string()))
}

/// fish `perform_for_commandline_impl`, up to the point where the argument
/// completers run.
fn locate(text: &str) -> Position {
    let all: Vec<char> = text.chars().collect();
    let sub = innermost_cmdsubst_start(&all);
    let s = &all[sub..];
    let (toks, in_comment) = tokenize_ending(s);
    // fish: nothing completes inside a comment
    if in_comment {
        return Position::Nothing;
    }
    // the last process: tokens after the last separator
    let first = toks.iter().rposition(|t| t.kind == TokKind::Sep).map_or(0, |p| p + 1);
    let mut toks: Vec<Tok> = toks[first..].to_vec();
    let at_end = |t: &Tok| t.end >= s.len();
    let mut assigned = false;
    // `VAR=val` before the command word. `and`, `not`, `command`, `time`, ...
    // stay: fish completes them from their own scripts (not.fish, command.fish,
    // ...), which hand the rest of the line to `__fish_complete_subcommand`.
    while let Some(t) = toks.first() {
        let w: String = s[t.start..t.end].iter().collect();
        if at_end(t) || t.kind != TokKind::Str || !is_var_assignment(&w) {
            break;
        }
        toks.remove(0);
        assigned = true;
    }
    let Some(cmd_tok) = toks.first().copied() else {
        return Position::Command(String::new());
    };
    let tok_text = |t: &Tok| -> String { s[t.start..t.end].iter().collect() };
    // a variable name or `~user` being typed, wherever the word is
    if let Some(last) = toks.last().filter(|t| at_end(t) && t.kind == TokKind::Str) {
        if let Some(p) = special_position(&tok_text(last)) {
            return p;
        }
    }
    if cmd_tok.kind != TokKind::Str {
        return Position::Nothing;
    }
    let last = *toks.last().expect("non-empty");
    let cursor_in_ws = !at_end(&last);
    if !cursor_in_ws && toks.len() == 1 {
        return Position::Command(tok_text(&cmd_tok));
    }
    // the cursor right after a redirection operator (`echo >`): fish's current
    // argument is empty and a file goes there
    let (cur_raw, prev_tok) = if cursor_in_ws || last.kind == TokKind::Redirect {
        (String::new(), Some(last))
    } else {
        (tok_text(&last), (toks.len() >= 2).then(|| toks[toks.len() - 2]))
    };
    let in_redirection = (!cursor_in_ws && last.kind == TokKind::Redirect)
        || prev_tok.is_some_and(|p| p.kind == TokKind::Redirect);
    let prev = prev_tok
        .filter(|p| p.kind == TokKind::Str)
        .map(|p| unescape(&tok_text(&p)))
        .unwrap_or_default();
    let upto = if cursor_in_ws { toks.len() } else { toks.len() - 1 };
    let had_ddash = toks[1..upto.max(1)].iter().any(|t| tok_text(t) == "--");
    // fish `effective_cmdline`: what `commandline` reports while the completers
    // run is the whole line up to the cursor (`cwebp | cwebp -` has two
    // commands in `commandline -cx`), from the command word only when `VAR=`
    // words before it were taken off
    let base = if assigned { cmd_tok.start } else { 0 };
    let effective: String = s[base..].iter().collect();
    Position::Argument(Line {
        effective,
        cmd: unescape(&tok_text(&cmd_tok)),
        cmd_range: (cmd_tok.start - base, cmd_tok.end - base),
        prev,
        cur: unescape(&cur_raw),
        cur_wild: unescape_wild(&cur_raw),
        cur_raw,
        had_ddash,
        in_redirection,
    })
}

/// Run the argument completers for `line`. `None` when no fish definition
/// covers the command (its zsh completer should run instead).
fn complete_argument(line: &Line, eval: &mut dyn Eval) -> Option<Answer> {
    if line.in_redirection {
        return None;
    }
    let mut c = Completer {
        eval,
        cond_cache: HashMap::new(),
        out: Answer::default(),
        covered: false,
    };
    let mut do_file = true;
    let mut visited = HashSet::new();
    c.walk_wrap_chain(&line.cmd, &line.effective, line.cmd_range, &line.prev, &line.cur, line.had_ddash, &mut do_file, 0, &mut visited);
    if !c.covered {
        return None;
    }
    let mut seen = HashSet::new();
    c.out.comps.retain(|x| seen.insert(x.word.clone()));
    c.out.do_file = do_file;
    c.out.cd = line.cmd.rsplit('/').next() == Some("cd") || visited.contains("cd");
    Some(c.out)
}

// ---------------------------------------------------------------------------
// Running fish code from the engine.
// ---------------------------------------------------------------------------

/// The command line `commandline` reports while a completion runs (fish's
/// `transient_commandline`): the effective line up to the cursor.
fn transient() -> std::sync::MutexGuard<'static, Vec<String>> {
    static T: OnceLock<Mutex<Vec<String>>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap_or_else(|e| e.into_inner())
}

struct TransientGuard;

impl TransientGuard {
    fn set(line: &str) -> Self {
        transient().push(line.to_string());
        TransientGuard
    }
}

impl Drop for TransientGuard {
    fn drop(&mut self) {
        transient().pop();
    }
}

fn transient_or_empty() -> String {
    transient().last().cloned().unwrap_or_default()
}

/// Turn newlines between the words of an `-a` text into spaces: it is an
/// argument list, where a newline separates words as a space does. Inside a
/// `( … )` substitution a newline separates commands and stays.
fn flatten_newlines(src: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut esc = false;
    let mut depth = 0usize;
    for c in src.chars() {
        if esc {
            esc = false;
            out.push(c);
            continue;
        }
        match (quote, c) {
            (_, '\\') => esc = true,
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '(') => depth += 1,
            (None, ')') => depth = depth.saturating_sub(1),
            (None, '\n') if depth == 0 => {
                out.push(' ');
                continue;
            }
            _ => {}
        }
        out.push(c);
    }
    out
}

/// Keeps `$?` and the error state across the engine's own shell runs: fish
/// restores its last statuses around completion loading and argument
/// expansion, and an error inside a completion script (a no-match glob in a
/// helper an `-a` list calls) ends that piece of code, not the rest of the
/// script being loaded nor the command line that asked for the completion.
struct KeepStatus {
    lastval: i32,
    errflag: i32,
}

impl KeepStatus {
    fn new() -> Self {
        use std::sync::atomic::Ordering::Relaxed;
        let errflag = crate::ported::utils::errflag.load(Relaxed);
        crate::ported::utils::errflag.fetch_and(!crate::ported::zsh_h::ERRFLAG_ERROR, Relaxed);
        KeepStatus {
            lastval: crate::ported::builtin::LASTVAL.load(Relaxed),
            errflag,
        }
    }
}

impl Drop for KeepStatus {
    fn drop(&mut self) {
        use std::sync::atomic::Ordering::Relaxed;
        crate::ported::builtin::LASTVAL.store(self.lastval, Relaxed);
        crate::ported::utils::errflag.store(self.errflag, Relaxed);
    }
}

/// Run `cmd` and capture its output in the CURRENT shell, as fish runs a
/// completion condition or an `-a` expansion (`exec_subshell` is not a fork in
/// fish): a `set -g` cache a completion script keeps — git.fish's
/// `__fish_git_cmdline` — must survive from one condition to the next.
fn capture_in_shell(cmd: &str) -> String {
    crate::fusevm_bridge::try_with_executor(|e| e.run_shared_state_substitution(cmd))
        .unwrap_or_else(|| crate::ported::exec::run_command_substitution(cmd))
}

/// The live shell: fish code translated and run in-process.
struct ShellEval;

impl Eval for ShellEval {
    fn test(&mut self, cond: &str) -> bool {
        let _keep = KeepStatus::new();
        let Ok(z) = crate::fish::translate(cond) else {
            return false;
        };
        let out = capture_in_shell(&format!("{{\n{z}\n}} >/dev/null 2>&1 </dev/null; print -rn -- $?"));
        tracing::debug!(target: "fish_complete", cond, status = %out.trim(), "condition");
        out.trim() == "0"
    }

    fn args(&mut self, args: &str) -> Vec<String> {
        let _keep = KeepStatus::new();
        let src = format!("__fish_cl_words {}", flatten_newlines(args));
        let Ok(z) = crate::fish::translate(&src) else {
            return Vec::new();
        };
        let out = capture_in_shell(&format!("{{\n{z}\n}} 2>/dev/null </dev/null"));
        tracing::debug!(target: "fish_complete", args, words = out.lines().count(), "argument list");
        out.lines().map(str::to_string).collect()
    }

    fn load(&mut self, cmd: &str) {
        complete_load(cmd);
    }
}

/// Whether `name` runs as something: function, builtin, native command, or `PATH`.
fn command_exists(name: &str) -> bool {
    // fish `builtin_exists`: its builtins count even where zsh has no such
    // command (`not`, `and`, `begin`, `string`, ...)
    FISH_BUILTINS.iter().any(|(b, _)| *b == name)
        || crate::ported::utils::getshfunc(name).is_some()
        || crate::ported::builtin::BUILTINS.iter().any(|b| b.node.nam == name)
        || crate::native_cmds::is_registered(name)
        || crate::ported::exec::findcmd(name, 0, 0).is_some()
        // fish `exists_no_autoload`: a function that could be autoloaded counts
        || find_script("functions", &format!("{name}.fish")).is_some()
}

/// Translate and run a fish file in the current shell, as fish's autoloader
/// sources it: a top-level `exit` ends the file, not the shell (rm.fish ends
/// with `test "$uname" = Darwin; and exit 0`). Same shape as the preamble's
/// `source`: translated in source mode, run as an anonymous function.
fn source_fish(text: &str, origin: &str) -> bool {
    let _keep = KeepStatus::new();
    crate::fish::set_source_mode(true);
    let translated = crate::fish::translate(text);
    crate::fish::set_source_mode(false);
    match translated {
        Ok(z) => {
            crate::ported::exec::execstring(&format!("() {{\n{z}\n}}"), 0, 0, origin);
            true
        }
        Err(e) => {
            tracing::warn!(origin, error = %e, "fish: script does not translate");
            false
        }
    }
}

/// fish `complete_load`: look for `<cmd>.fish` once — per subshell level:
/// a load done inside a `$( … )` lost its functions when that substitution
/// returned, so a shallower caller drops the options it added and sources the
/// file again.
fn complete_load(cmd: &str) {
    let depth = crate::ported::exec::zsh_subshell.load(std::sync::atomic::Ordering::Relaxed);
    {
        let mut st = store();
        if st.tombstones.contains(cmd) {
            return;
        }
        let key = (cmd.to_string(), false);
        if let Some(prev) = st.loaded.get(cmd).copied() {
            if prev.depth <= depth {
                return;
            }
            if let Some(e) = st.map.get_mut(&key) {
                e.options.truncate(prev.options_before);
            }
        }
        let options_before = st.map.get(&key).map_or(0, |e| e.options.len());
        st.loaded.insert(cmd.to_string(), Loaded { depth, options_before });
    }
    // providers for a tool that is not installed only produce errors
    if !command_exists(cmd) {
        return;
    }
    // the function first: its file may hold a `--wraps` (fish #2466)
    autoload_function(cmd);
    let file = format!("{cmd}.fish");
    if let Some((text, origin)) = find_script("completions", &file) {
        source_fish(&text, &origin);
    }
}

/// `__fish_autoload NAME`: define the function `NAME` from `NAME.fish` on
/// `$fish_function_path` or among fish's own functions. 0 when it is defined.
pub fn autoload_function(name: &str) -> bool {
    if crate::ported::utils::getshfunc(name).is_some() {
        return true;
    }
    // Looked up again whenever it is not defined: a definition made inside a
    // `$( … )` is gone once that substitution returns.
    if name.is_empty() || name.contains('/') || store().fn_missing.contains(name) {
        return false;
    }
    let Some((text, origin)) = find_script("functions", &format!("{name}.fish")) else {
        store().fn_missing.insert(name.to_string());
        return false;
    };
    source_fish(&text, &origin) && crate::ported::utils::getshfunc(name).is_some()
}

// ---------------------------------------------------------------------------
// Where scripts come from: the fish search paths and the installed fish.
// ---------------------------------------------------------------------------

/// What an installed fish told us once: its sysconf and build-time vendor
/// dirs, and the scripts it carries inside its binary.
pub(crate) struct FishData {
    exe: PathBuf,
    cache: PathBuf,
    /// the installed fish's `$__fish_sysconf_dir`
    pub sysconf: String,
    /// the installed fish's `$__fish_data_dir`, `$__fish_help_dir`, `$__fish_man_dir`, `$__fish_bin_dir`
    pub dirs: [String; 4],
    extra_completions: String,
    extra_functions: String,
    files: HashSet<String>,
}

fn env_dir(var: &str, fallback: &str) -> String {
    match std::env::var(var) {
        Ok(v) if v.starts_with('/') => v,
        _ => format!("{}/{fallback}", std::env::var("HOME").unwrap_or_default()),
    }
}

fn zshrs_home() -> PathBuf {
    match std::env::var_os("ZSHRS_HOME") {
        Some(h) => PathBuf::from(h),
        None => PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".zshrs"),
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var("PATH").ok()?;
    path.split(':')
        .filter(|d| !d.is_empty())
        .map(|d| Path::new(d).join(name))
        .find(|p| p.is_file())
}

/// Write `data` to `path` via a temporary file and a rename, so a concurrent
/// shell never reads half a file.
fn write_atomic(path: &Path, data: &[u8]) {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    if std::fs::write(&tmp, data).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Run `fish -N -c ARGS…` and return its output when it succeeded. The output
/// is read to the end before the wait: the shell's SIGCHLD handler may reap
/// the child first (ECHILD), and then the whole output is all there is to go by.
fn run_fish(exe: &Path, args: &[&str]) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut child = std::process::Command::new(exe)
        .arg("-N")
        .arg("-c")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let mut out = Vec::new();
    child.stdout.take()?.read_to_end(&mut out).ok()?;
    match child.wait() {
        Ok(st) if !st.success() => None,
        _ => Some(out),
    }
}

/// The installed fish: its directories and the scripts it embeds. Found once;
/// a failed lookup is tried again next time.
pub(crate) fn fish_data() -> Option<&'static FishData> {
    static DATA: OnceLock<FishData> = OnceLock::new();
    if let Some(d) = DATA.get() {
        return Some(d);
    }
    let found = (|| {
        let exe = find_on_path("fish")?;
        let meta = std::fs::metadata(&exe).ok()?;
        let stamp = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let cache = zshrs_home().join("fish-data").join(format!("{}-{stamp}", meta.len()));
        // the index names its lines' layout: a new layout is a new file
        let index = cache.join("index-2");
        let text = match std::fs::read_to_string(&index) {
            Ok(t) => t,
            Err(_) => {
                let out = run_fish(
                    &exe,
                    &[
                        "echo $__fish_sysconf_dir; echo $__fish_data_dir; echo $__fish_help_dir; echo $__fish_man_dir; echo $__fish_bin_dir; \
                         status get-file __fish_build_paths.fish 2>/dev/null | source; \
                         echo $__extra_completionsdir; echo $__extra_functionsdir; status list-files",
                    ],
                )?;
                write_atomic(&index, &out);
                String::from_utf8_lossy(&out).into_owned()
            }
        };
        let mut lines = text.lines();
        let sysconf = lines.next()?.to_string();
        let mut dirs: [String; 4] = Default::default();
        for d in &mut dirs {
            *d = lines.next()?.to_string();
        }
        let extra_completions = lines.next()?.to_string();
        let extra_functions = lines.next()?.to_string();
        Some(FishData {
            exe,
            cache,
            sysconf,
            dirs,
            extra_completions,
            extra_functions,
            files: lines.map(str::to_string).collect(),
        })
    })()?;
    Some(DATA.get_or_init(|| found))
}

/// A script fish carries inside its binary (`completions/git.fish`), read once
/// with `status get-file` and kept in the cache.
fn embedded(rel: &str) -> Option<String> {
    let d = fish_data()?;
    if !d.files.contains(rel) {
        return None;
    }
    let path = d.cache.join(rel);
    if let Ok(t) = std::fs::read_to_string(&path) {
        return Some(t);
    }
    let out = run_fish(&d.exe, &["status get-file $argv[1]", rel])?;
    write_atomic(&path, &out);
    Some(String::from_utf8_lossy(&out).into_owned())
}

/// fish's default `fish_function_path` / `fish_complete_path` (share/config.fish).
/// fish's `share/config.fish` sets `$fish_function_path` and `$fish_complete_path`
/// to these defaults before the user's configuration runs, so a `set -p
/// fish_complete_path ~/x` there adds to them; `--no-config` (`-f`) leaves them
/// unset. The startup assignments, for the translation preamble.
pub(crate) fn search_path_assignments() -> String {
    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::RCS) {
        return String::new();
    }
    let q = |s: &String| format!("'{}'", s.replace('\'', "'\\''"));
    ["functions", "completions"]
        .iter()
        .map(|kind| {
            let var = if *kind == "functions" { "fish_function_path" } else { "fish_complete_path" };
            let dirs: Vec<String> = default_search_path(kind).iter().map(q).collect();
            format!("typeset -ga {var}=({})\n", dirs.join(" "))
        })
        .collect()
}

fn default_search_path(kind: &str) -> Vec<String> {
    let config = format!("{}/fish", env_dir("XDG_CONFIG_HOME", ".config"));
    let user_data = format!("{}/fish", env_dir("XDG_DATA_HOME", ".local/share"));
    let cache = format!("{}/fish", env_dir("XDG_CACHE_HOME", ".cache"));
    let xdg: Vec<String> = match std::env::var("XDG_DATA_DIRS") {
        Ok(v) if !v.is_empty() => v
            .split(':')
            .filter(|d| !d.is_empty())
            .map(|d| format!("{}/fish", d.trim_end_matches('/')))
            .collect(),
        _ => vec!["/usr/local/share/fish".to_string(), "/usr/share/fish".to_string()],
    };
    let vendor = if kind == "functions" { "vendor_functions.d" } else { "vendor_completions.d" };
    let mut out = vec![format!("{config}/{kind}")];
    let data = fish_data();
    if let Some(d) = data {
        out.push(format!("{}/{kind}", d.sysconf));
    }
    out.push(format!("{user_data}/{vendor}"));
    out.extend(xdg.iter().map(|d| format!("{d}/{vendor}")));
    if let Some(d) = data {
        let extra = if kind == "functions" { &d.extra_functions } else { &d.extra_completions };
        if !extra.is_empty() && !out.contains(extra) {
            out.push(extra.clone());
        }
    }
    if kind == "completions" {
        out.push(format!("{cache}/generated_completions"));
    }
    out
}

/// `$fish_function_path` / `$fish_complete_path` when the shell set them,
/// fish's defaults otherwise.
fn search_path(kind: &str) -> Vec<String> {
    let var = if kind == "functions" { "fish_function_path" } else { "fish_complete_path" };
    match crate::ported::params::getaparam(var) {
        Some(v) if !v.is_empty() => v,
        // `-f` is fish's `--no-config`: share/config.fish never sets the
        // paths, so only fish's own scripts are found
        _ if !crate::ported::zsh_h::isset(crate::ported::zsh_h::RCS) => Vec::new(),
        _ => default_search_path(kind),
    }
}

/// fish autoload order: the search path (with `generated_completions` held
/// back), then fish's own scripts, then `generated_completions`.
fn find_script(kind: &str, file: &str) -> Option<(String, String)> {
    let dirs = search_path(kind);
    let (generated, rest): (Vec<&String>, Vec<&String>) = dirs.iter().partition(|d| d.ends_with("/generated_completions"));
    for d in rest {
        let p = Path::new(d).join(file);
        if let Ok(t) = std::fs::read_to_string(&p) {
            return Some((t, p.to_string_lossy().into_owned()));
        }
    }
    let rel = format!("{kind}/{file}");
    if let Some(t) = embedded(&rel) {
        return Some((t, format!("embedded:{rel}")));
    }
    for d in generated {
        let p = Path::new(d).join(file);
        if let Ok(t) = std::fs::read_to_string(&p) {
            return Some((t, p.to_string_lossy().into_owned()));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// `complete -C`: the whole completion, printed (fish's `__fish_complete_subcommand`
// and friends call it).
// ---------------------------------------------------------------------------

fn complete_dash_c(text: &str) -> Vec<Comp> {
    let position = locate(text);
    let is_variable = matches!(position, Position::Variable { .. });
    // `typed`: what a match is ranked against; `lead`: what a match that keeps
    // what was typed starts with (the `$`, `~` and anything before them too)
    let (typed, lead, comps) = match position {
        Position::Command(prefix) => {
            let typed = unescape(&prefix);
            let comps = command_names(&typed);
            (typed.clone(), typed, comps)
        }
        Position::Nothing => (String::new(), String::new(), Vec::new()),
        Position::Variable { before, name } => {
            let comps = variable_comps(&before, &name);
            (name.clone(), format!("{before}{name}"), comps)
        }
        Position::User(name) => {
            let comps = user_comps(&name);
            (name.clone(), format!("~{name}"), comps)
        }
        Position::Argument(line) => {
            let ans = complete_argument(&line, &mut ShellEval);
            let do_file = ans.as_ref().is_none_or(|a| a.do_file);
            let ans_cd = ans.as_ref().is_some_and(|a| a.cd);
            let mut comps = ans.map(|a| a.comps).unwrap_or_default();
            if do_file {
                comps.extend(param_complete(&line.cur_raw, ans_cd, false));
            }
            (line.cur.clone(), line.cur, comps)
        }
    };
    // fish `sort_and_prioritize`: only the best kind of match survives
    let ranked: Vec<(u32, Comp)> = comps
        .into_iter()
        .filter_map(|c| rank_of(&typed, &c).map(|r| (r, c)))
        .collect();
    let best = ranked.iter().map(|(r, _)| *r).min();
    let mut comps: Vec<Comp> = ranked.into_iter().filter(|(r, _)| Some(*r) == best).map(|(_, c)| c).collect();
    // fish `sort_and_prioritize`: unique, then `-k` results first in their
    // own order, the rest in `wcsfilecmp` order
    let mut seen = HashSet::new();
    comps.retain(|c| seen.insert(c.word.clone()));
    comps.sort_by(|a, b| match (a.keep, b.keep) {
        (true, true) => std::cmp::Ordering::Equal,
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        // fish stores a same-case prefix match as the text after what was
        // typed and any other match as the whole word, and sorts on that
        (false, false) => wcsfilecmp(sort_key(&lead, &a.word), sort_key(&lead, &b.word)),
    });
    // builtin complete.rs: a match is printed as the token it makes, unescaped
    // (#1127); a variable's match carries the token's quotes and backslashes
    // (`"a b$HOME`), which come off (`a b$HOME`)
    if is_variable {
        for c in &mut comps {
            c.word = unescape(&c.word);
        }
    }
    comps
}

/// What fish keeps as the completion text: the rest of the word after a
/// same-case prefix, otherwise the whole word (it replaces the token).
fn sort_key<'a>(typed: &str, word: &'a str) -> &'a str {
    word.strip_prefix(typed).unwrap_or(word)
}

/// fish `StringFuzzyMatch` ranks, better first: a prefix (or a case-insensitive
/// prefix typed all lowercase, fish's smartcase), a case-insensitive prefix, a
/// substring, a case-insensitive substring, a subsequence.
const PREFIX: u32 = 0;
const PREFIX_ICASE: u32 = 1;
const SUBSTR: u32 = 2;
const SUBSTR_ICASE: u32 = 3;
const SUBSEQ: u32 = 4;
/// Added to the rank of a match made after a separator (see [`Comp::from_separator`]).
const FROM_SEPARATOR: u32 = 16;

/// The rank of `c` for the typed word (see [`Comp::matched_as`]).
fn rank_of(typed: &str, c: &Comp) -> Option<u32> {
    fuzzy_rank(typed, c.matched_as.as_deref().unwrap_or(&c.word)).map(|r| r.max(c.rank_floor) + if c.from_separator { FROM_SEPARATOR } else { 0 })
}

/// fish `string_fuzzy_match_string(typed, candidate)` as a rank.
fn fuzzy_rank(typed: &str, cand: &str) -> Option<u32> {
    if typed.chars().count() > cand.chars().count() {
        return None;
    }
    if cand.starts_with(typed) {
        return Some(PREFIX);
    }
    let smart = !typed.chars().any(char::is_uppercase);
    let fold = |s: &str| s.to_lowercase();
    let (t, c) = (fold(typed), fold(cand));
    if c.starts_with(&t) {
        return Some(if smart { PREFIX } else { PREFIX_ICASE });
    }
    if cand.contains(typed) {
        return Some(SUBSTR);
    }
    // fish `ifind(…, fuzzy)`: the case-insensitive substring step also takes
    // `_` for `-` (#3584), so `openssl -` finds `s_client`
    let dash = |x: &str| x.replace('_', "-");
    if dash(&c).contains(&dash(&t)) {
        return Some(if smart { SUBSTR } else { SUBSTR_ICASE });
    }
    let mut it = cand.chars();
    typed.chars().all(|ch| it.any(|x| x == ch)).then_some(SUBSEQ)
}

/// fish `wcsfilecmp`: numbers compare by value, letters without case, `-`
/// sorts after `Z` and `/` before everything; case and leading zeros only
/// break ties.
fn wcsfilecmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (mut ai, mut bi) = (0, 0);
    let mut ret = Ordering::Equal;
    while ai < a.len() && bi < b.len() {
        if a[ai].is_ascii_digit() && b[bi].is_ascii_digit() {
            // the numbers' values: leading zeros skipped, then the longer is
            // larger, else the first differing digit decides
            let za = a[ai..].iter().take_while(|c| **c == '0').count();
            let zb = b[bi..].iter().take_while(|c| **c == '0').count();
            let (mut i, mut j) = (ai + za, bi + zb);
            let mut digits = Ordering::Equal;
            loop {
                let ca = a.get(i).filter(|c| c.is_ascii_digit());
                let cb = b.get(j).filter(|c| c.is_ascii_digit());
                match (ca, cb) {
                    (Some(x), Some(y)) => {
                        if digits == Ordering::Equal {
                            digits = x.cmp(y);
                        }
                    }
                    (Some(_), None) => {
                        digits = Ordering::Greater;
                        break;
                    }
                    (None, Some(_)) => {
                        digits = Ordering::Less;
                        break;
                    }
                    (None, None) => break,
                }
                i += 1;
                j += 1;
            }
            ret = digits;
            ai = i;
            bi = j;
            if ret != Ordering::Equal || ai == a.len() || bi == b.len() {
                break;
            }
            continue;
        }
        if a[ai] == b[bi] {
            ai += 1;
            bi += 1;
            continue;
        }
        let key = |c: char| -> char {
            let c = match c {
                '-' => '[',
                '/' => '\0',
                c => c,
            };
            c.to_uppercase().next().unwrap_or(c)
        };
        match key(a[ai]).cmp(&key(b[bi])) {
            Ordering::Equal => {
                ai += 1;
                bi += 1;
            }
            o => {
                ret = o;
                break;
            }
        }
    }
    if ret != Ordering::Equal {
        return ret;
    }
    match (ai == a.len(), bi == b.len()) {
        (true, true) => a.cmp(&b),
        (true, false) => Ordering::Less,
        _ => Ordering::Greater,
    }
}

/// fish `builtin_get_names` / `builtin_get_desc` (fish 4.9
/// `src/builtins/shared/misc.rs`): the builtins and their one-line descriptions.
const FISH_BUILTINS: &[(&str, &str)] = &[
    ("!", "Negate exit status of job"),
    (".", "Evaluate contents of file"),
    (":", "Return a successful result"),
    ("[", "Test a condition"),
    ("_", "Translate a string"),
    ("abbr", "Manage abbreviations"),
    ("and", "Run command if last command succeeded"),
    ("argparse", "Parse options in fish script"),
    ("begin", "Create a block of code"),
    ("bg", "Send job to background"),
    ("bind", "Handle fish key bindings"),
    ("block", "Temporarily block delivery of events"),
    ("break", "Stop the innermost loop"),
    ("breakpoint", "Halt execution and start debug prompt"),
    ("builtin", "Run a builtin specifically"),
    ("case", "Block of code to run conditionally"),
    ("cd", "Change working directory"),
    ("command", "Run a command specifically"),
    ("commandline", "Set or get the commandline"),
    ("complete", "Edit command specific completions"),
    ("contains", "Search for a specified string in a list"),
    ("continue", "Skip over remaining innermost loop"),
    ("count", "Count the number of arguments"),
    ("disown", "Remove job from job list"),
    ("echo", "Print arguments"),
    ("else", "Evaluate block if condition is false"),
    ("emit", "Emit an event"),
    ("end", "End a block of commands"),
    ("eval", "Evaluate a string as a statement"),
    ("exec", "Run command in current process"),
    ("exit", "Exit the shell"),
    ("false", "Return an unsuccessful result"),
    ("fg", "Send job to foreground"),
    ("fish_indent", ""),
    ("fish_key_reader", "explore what characters keyboard keys send"),
    ("for", "Perform a set of commands multiple times"),
    ("function", "Define a new function"),
    ("functions", "List or remove functions"),
    ("history", "History of commands executed by user"),
    ("if", "Evaluate block if condition is true"),
    ("jobs", "Print currently running jobs"),
    ("math", "Evaluate math expressions"),
    ("not", "Negate exit status of job"),
    ("or", "Execute command if previous command failed"),
    ("path", "Handle paths"),
    ("printf", "Prints formatted text"),
    ("pwd", "Print the working directory"),
    ("random", "Generate random number"),
    ("read", "Read a line of input into variables"),
    ("realpath", "Show absolute path sans symlinks"),
    ("return", "Stop the currently evaluated function"),
    ("set", "Handle environment variables"),
    ("set_color", "Set the terminal color"),
    ("source", "Evaluate contents of file"),
    ("status", "Return status information about fish"),
    ("string", "Manipulate strings"),
    ("switch", "Conditionally run blocks of code"),
    ("test", "Test a condition"),
    ("time", "Measure how long a command or block takes"),
    ("true", "Return a successful result"),
    ("type", "Check if a thing is a thing"),
    ("ulimit", "Get/set resource usage limits"),
    ("wait", "Await background process completion"),
    ("while", "Perform a command multiple times"),
];

fn comp(word: String, desc: &str, no_space: bool) -> Comp {
    Comp {
        word,
        desc: desc.to_string(),
        no_space,
        keep: false,
        matched_as: None,
        rank_floor: 0,
        from_separator: false,
    }
}

/// fish `complete_cmd` for the command word: executables, then directories,
/// then functions (autoloadable ones included), then builtins; the first
/// source to offer a name gives its description.
/// fish `complete_cmd_desc`: executables that extend what was typed take
/// their description from `__fish_describe_command` (the whatis database,
/// one apropos call), first letter uppercased. Not for fewer than two typed
/// characters, a wildcard, or when nothing found is a command.
fn describe_commands(typed: &str, comps: &mut [Comp]) {
    let cmd = typed.rsplit('/').next().unwrap_or(typed);
    if cmd.chars().count() < 2 || cmd.contains(['*', '?']) {
        return;
    }
    if !comps.iter().any(|c| !c.word.ends_with('/') && fuzzy_rank(typed, &c.word).is_some()) {
        return;
    }
    let out = fish_output(&format!("functions -q __fish_describe_command && __fish_describe_command -- {}", escape(cmd)));
    let mut lookup = BTreeMap::new();
    for line in out.lines() {
        let Some((name, desc)) = line.split_once('\t') else { continue };
        let (Some(key), Some(first)) = (name.get(cmd.len()..), desc.chars().next()) else { continue };
        let desc: String = first.to_uppercase().chain(desc.chars().skip(1)).collect();
        lookup.insert(key.to_string(), desc);
    }
    // fish keeps a same-case prefix match as the rest of the word, and looks
    // that up; a word that replaces the token is never found
    for c in comps.iter_mut() {
        if let Some(desc) = c.word.strip_prefix(typed).and_then(|rest| lookup.get(rest)) {
            c.desc = desc.clone();
        }
    }
}

fn command_names(prefix: &str) -> Vec<Comp> {
    if prefix.contains('/') || prefix.starts_with('~') {
        // a path: executables and directories under it
        let mut out: Vec<Comp> = file_names(prefix, true)
            .into_iter()
            .filter(|c| c.word.ends_with('/') || is_executable(&expand_tilde(&c.word)))
            .collect();
        describe_commands(prefix, &mut out);
        return out;
    }
    // fish `complete_abbr` runs before `complete_cmd`: an abbreviation wins
    let mut out = abbr_comps();
    let path = crate::ported::params::getsparam("PATH").unwrap_or_default();
    for dir in path.split(':').filter(|d| !d.is_empty()) {
        let Ok(rd) = std::fs::read_dir(dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let p = e.path();
            if !is_executable(&p.to_string_lossy()) || p.is_dir() || (name.starts_with('.') && !prefix.starts_with('.')) {
                continue;
            }
            let link = std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink());
            out.push(comp(name, if link { "command link" } else { "command" }, false));
        }
    }
    describe_commands(prefix, &mut out);
    if let Ok(rd) = std::fs::read_dir(".") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() && (!name.starts_with('.') || prefix.starts_with('.')) {
                out.push(comp(format!("{name}/"), "directory", true));
            }
        }
    }
    // fish `complete_function_desc`: a defined function's `--description`
    // (an autoloadable one not loaded yet has none)
    let descs = crate::ported::subst::assoc_get("__fish_fndesc").unwrap_or_default();
    for f in function_names(prefix.starts_with('_')) {
        let desc = descs.get(&f).map_or("", String::as_str);
        out.push(comp(f, desc, false));
    }
    out.extend(
        FISH_BUILTINS
            .iter()
            .filter(|(n, _)| !n.starts_with('.') || prefix.starts_with('.'))
            .map(|(n, d)| comp(n.to_string(), d, false)),
    );
    out
}

/// fish `function::get_names`: the shell's functions — not the preamble's
/// stand-ins for fish builtins nor its internal helpers — and every
/// autoloadable `functions/*.fish` (search path and the installed fish).
/// Hidden (`_…`) names only with `hidden`. Sorted, unique.
pub fn function_names(hidden: bool) -> Vec<String> {
    let is_builtin = |n: &str| FISH_BUILTINS.iter().any(|(b, _)| *b == n);
    let mut fns: Vec<String> = crate::ported::hashtable::shfunctab_lock()
        .read()
        .map(|t| t.iter().map(|(n, _)| n.clone()).filter(|n| !is_builtin(n) && !preamble_functions().contains(n)).collect())
        .unwrap_or_default();
    for dir in search_path("functions") {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            fns.extend(rd.flatten().filter_map(|e| e.file_name().to_string_lossy().strip_suffix(".fish").map(str::to_string)));
        }
    }
    if let Some(d) = fish_data() {
        fns.extend(d.files.iter().filter_map(|f| f.strip_prefix("functions/")?.strip_suffix(".fish").map(str::to_string)));
    }
    fns.retain(|f| hidden || !f.starts_with('_'));
    // `functions -n` order: by code point
    fns.sort();
    fns.dedup();
    fns
}

/// Run fish source in the current shell and capture what it prints.
fn fish_output(src: &str) -> String {
    let _keep = KeepStatus::new();
    match crate::fish::translate(src) {
        Ok(z) => capture_in_shell(&format!("{{\n{z}\n}} 2>/dev/null </dev/null")),
        Err(_) => String::new(),
    }
}

/// fish `expand_escape_variable`: the elements one after another, each
/// quoted when there are several or it holds a space.
fn describe_value(elems: &[String]) -> String {
    elems
        .iter()
        .map(|e| {
            if (elems.len() > 1 || e.contains(' ')) && !e.contains(['\n', '\t', '\r', '\x08', '\x1b']) {
                format!("'{e}'")
            } else {
                escape(e)
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// fish `complete_variable`: every variable fish can see whose name the
/// typed part matches, described by its value (`Variable: …`). A word that is
/// just the variable gets a `/` instead of a space when the value is a
/// directory (reader.rs `maybe_add_slash`).
fn variable_comps(before: &str, typed: &str) -> Vec<Comp> {
    let names: Vec<String> = fish_output("set -n")
        .lines()
        .filter(|n| !n.is_empty() && fuzzy_rank(typed, n).is_some())
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return Vec::new();
    }
    // one shell run for all the values: per variable its element count, then
    // each element escaped onto its own line (a value may hold newlines).
    // fish skips a listed name it cannot read (`_`, an unset
    // `fish_terminal_color_theme`): `-1` marks one. No variable of its own:
    // this runs in the shell, and one would show up in `set -n`.
    let src: String = names
        .iter()
        .map(|n| format!("if set -q {n}\n    count ${n}\n    string escape -- ${n}\nelse\n    echo -1\nend\n"))
        .collect();
    let out = fish_output(&src);
    tracing::debug!(target: "fish_complete", src = %src, out = %out.chars().take(200).collect::<String>(), "variable values");
    let mut lines = out.lines();
    let mut values: Vec<Option<Vec<String>>> = Vec::with_capacity(names.len());
    for _ in &names {
        let n: i64 = lines.next().and_then(|c| c.trim().parse().ok()).unwrap_or(0);
        values.push((n >= 0).then(|| (0..n).filter_map(|_| lines.next().map(unescape)).collect()));
    }
    let lit = before.trim_end_matches('$');
    // fish #6288: $history is described by its newest 63 items, each through
    // expand_escape_string (quoted when it holds a space), not by its value
    let history_desc = || {
        fish_output("history --max 63")
            .lines()
            .map(|h| describe_value(std::slice::from_ref(&h.to_string())))
            .collect::<Vec<_>>()
            .join(" ")
    };
    names
        .iter()
        .zip(values)
        .filter_map(|(n, elems)| {
            if n == "history" {
                return Some(Comp {
                    word: format!("{before}{n}"),
                    desc: history_desc(),
                    no_space: false,
                    keep: false,
                    matched_as: Some(n.clone()),
                    rank_floor: 0,
                    from_separator: false,
                });
            }
            let elems = elems?;
            // fish `maybe_add_slash`: the token up to here, quotes and all, expanded
            let is_dir = elems.len() == 1
                && !lit.contains(['$', '(', '\''])
                && Path::new(&format!("{}{}", expand_tilde(&unescape(lit)), elems[0])).is_dir();
            Some(Comp {
                word: format!("{before}{n}{}", if is_dir { "/" } else { "" }),
                desc: format!("Variable: {}", describe_value(&elems)),
                no_space: is_dir,
                keep: false,
                matched_as: Some(n.clone()),
                rank_floor: 0,
                from_separator: false,
            })
        })
        .collect()
}

/// fish `try_complete_user`: user names the typed part begins (any case),
/// described `Home for NAME`, no space after.
fn user_comps(typed: &str) -> Vec<Comp> {
    let mut out = Vec::new();
    let started = std::time::Instant::now();
    unsafe { libc::setpwent() };
    loop {
        let pw = unsafe { libc::getpwent() };
        if pw.is_null() {
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*pw).pw_name) }.to_string_lossy().into_owned();
        if fuzzy_rank(typed, &name).is_some_and(|r| r <= PREFIX_ICASE) && !out.iter().any(|c: &Comp| c.matched_as.as_deref() == Some(name.as_str())) {
            out.push(Comp {
                word: format!("~{name}"),
                desc: format!("Home for {name}"),
                no_space: true,
                keep: false,
                matched_as: Some(name),
                rank_floor: 0,
                from_separator: false,
            });
        }
        // fish gives up after 200 ms (a slow directory service)
        if started.elapsed() > std::time::Duration::from_millis(200) {
            break;
        }
    }
    unsafe { libc::endpwent() };
    out
}

/// fish `complete_abbr`: the abbreviations (not the regex ones), described
/// `Abbreviation: REPLACEMENT`, no space after. Read from `abbr --show`.
fn abbr_comps() -> Vec<Comp> {
    let mut out = Vec::new();
    for line in fish_output("abbr --show").lines() {
        let chars: Vec<char> = line.chars().collect();
        let words: Vec<String> = tokenize(&chars)
            .iter()
            .filter(|t| t.kind == TokKind::Str)
            .map(|t| unescape(&chars[t.start..t.end].iter().collect::<String>()))
            .collect();
        if words.iter().any(|w| w == "--regex" || w == "-r") {
            continue;
        }
        let function = words.iter().position(|w| w == "--function" || w == "-f").and_then(|i| words.get(i + 1)).cloned();
        let Some(dd) = words.iter().position(|w| w == "--") else { continue };
        let Some(name) = words.get(dd + 1) else { continue };
        let replacement = function.or_else(|| words.get(dd + 2).cloned()).unwrap_or_default();
        out.push(Comp {
            word: name.clone(),
            desc: format!("Abbreviation: {replacement}"),
            no_space: true,
            keep: false,
            matched_as: None,
            rank_floor: 0,
            from_separator: false,
        });
    }
    out
}

/// `$NAME` in a path, replaced by the variable's value (its elements joined
/// by a space, as fish joins a list in a string); an unset name is empty.
fn expand_vars(p: &str) -> String {
    let mut out = String::new();
    let mut it = p.chars().peekable();
    while let Some(c) = it.next() {
        if c != '$' {
            out.push(c);
            continue;
        }
        let mut name = String::new();
        while let Some(&n) = it.peek().filter(|n| n.is_alphanumeric() || **n == '_') {
            name.push(n);
            it.next();
        }
        if name.is_empty() {
            out.push('$');
            continue;
        }
        let rest: String = it.clone().collect();
        let index = var_index(&rest);
        if !index.is_empty() {
            it.nth(index.chars().count() - 1);
        }
        let value = fish_output(&format!("string join ' ' -- ${name}{index}"));
        out.push_str(value.trim_end_matches('\n'));
    }
    out
}

fn expand_tilde(p: &str) -> String {
    match p.strip_prefix("~/") {
        Some(rest) => format!("{}/{rest}", std::env::var("HOME").unwrap_or_default()),
        None => p.to_string(),
    }
}

/// A regular file this user may execute (`access(X_OK)`, as fish checks:
/// root-only daemons like `/usr/sbin/cupsd` are not offered).
fn is_executable(p: &str) -> bool {
    let Ok(c) = std::ffi::CString::new(p) else { return false };
    std::fs::metadata(p).is_ok_and(|m| m.is_file()) && unsafe { libc::access(c.as_ptr(), libc::X_OK) } == 0
}

/// Functions the fish preamble defines for its own use (`__fish_cl_words`,
/// `command_not_found_handler`, ...). fish has none of them, so they are not
/// offered unless fish ships a function of that name.
fn preamble_functions() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        crate::fish::PREAMBLE
            .lines()
            .filter_map(|l| {
                let name = l.split("() {").next()?;
                (l.contains("() {") && !name.is_empty() && !name.contains([' ', '\t', '$'])).then(|| name.to_string())
            })
            .collect()
    })
}

/// The entries of the directory `prefix` names that the typed last component
/// matches (fish's fuzzy match: case-insensitive prefixes, substrings, ...;
/// the caller keeps the best kind). Dot entries only when a dot is typed.
/// `describe` gives command-position descriptions (`directory`, `command`,
/// `command link`); argument file completions carry none (fish #254).
/// fish `complete_param_expand`: the files an argument completes to. A word
/// with an `=` or `:` (the last one) also completes after it, the part
/// before kept in the words; one starting with a dash (`--file=RE`) only
/// after it. `cd`'s argument completes directories, a relative one in each
/// `$CDPATH` directory and then `.` (fish `PathFilter::Directory` with
/// `for_cd_argument`).
fn param_files(s: &str, cd: bool, reader: bool) -> Vec<Comp> {
    let last_segment = &s[s.rfind('/').map_or(0, |i| i + 1)..];
    if !s.contains(BRACE_BEGIN) && !last_segment.contains(VARIABLE_EXPAND) {
        // a variable in a directory part is expanded where the directory is
        // read and stays as typed in the words
        return param_files_expanded(&s.replace(VARIABLE_EXPAND, "$"), cd);
    }
    // fish expands braces and variables first (expand_string's stages) and
    // completes each word they make. A match that extends its word adds the
    // rest to the word as typed. The reader drops one that replaces its word
    // (reader_can_replace: no `$` `{` `}`);
    // `complete -C` applies it to the word alone, where a leading `{` opens a
    // brace block for fish's tokenizer, so the token it replaces starts after
    // the `}` closing that block (or after the `{` when none does) and what
    // comes before stays.
    let typed = wild_text(s);
    let kept = if typed.starts_with('{') {
        let mut depth = 0;
        let close = typed.char_indices().find(|&(_, c)| {
            depth += match c {
                '{' => 1,
                '}' => -1,
                _ => 0,
            };
            depth == 0
        });
        &typed[..close.map_or(1, |(i, _)| i + 1)]
    } else {
        ""
    };
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    for word in expand_braces(&chars, true).iter().flat_map(|w| expand_variables(w)) {
        let shown = word.replace(ANY_STRING, "*");
        for mut c in param_files_expanded(&word, cd) {
            let rank = rank_of(&shown, &c);
            let extends = c.word.starts_with(&shown) && rank.is_some_and(|r| r % FROM_SEPARATOR == PREFIX);
            if !extends && reader {
                continue;
            }
            c.word = if extends { format!("{typed}{}", &c.word[shown.len()..]) } else { format!("{kept}{}", c.word) };
            // ranked as fish ranked it against the word it was found for
            c.matched_as = Some(typed.clone());
            c.rank_floor = rank.unwrap_or(SUBSEQ) % FROM_SEPARATOR;
            out.push(c);
        }
    }
    out
}

/// fish expand.rs `expand_variables` for a word being completed: each
/// [`VARIABLE_EXPAND`] and the name after it become the variable's elements,
/// one word per element (a word with an empty or unset variable is gone).
fn expand_variables(word: &str) -> Vec<String> {
    let Some(at) = word.find(VARIABLE_EXPAND) else {
        return vec![word.to_string()];
    };
    let rest = &word[at + VARIABLE_EXPAND.len_utf8()..];
    let name_len = rest.find(|c: char| !(c.is_alphanumeric() || c == '_')).unwrap_or(rest.len());
    let (name, after) = rest.split_at(name_len);
    if name.is_empty() {
        // a lone `$` is itself
        return expand_variables(after).into_iter().map(|w| format!("{}${w}", &word[..at])).collect();
    }
    let index = var_index(after);
    let after = &after[index.len()..];
    let values = fish_output(&format!("string join0 -- ${name}{index}"));
    let tails = expand_variables(after);
    values
        .split_terminator('\0')
        .flat_map(|v| tails.iter().map(move |t| format!("{}{v}{t}", &word[..at])))
        .collect()
}

/// The `[…]` index right after a variable name (`$PATH[1]`, `$argv[2..-1]`),
/// handed to fish as written; empty when there is none, it is unclosed, or it
/// holds a command substitution (which completion does not run).
fn var_index(after: &str) -> &str {
    if !after.starts_with('[') {
        return "";
    }
    match after.find(']') {
        Some(close) if !after[..close].contains('(') => &after[..=close],
        _ => "",
    }
}

/// [`param_files`] for a word with no brace or variable expansion left in it.
fn param_files_expanded(s: &str, cd: bool) -> Vec<Comp> {
    let files = |p: &str| -> Vec<Comp> {
        if !cd {
            return file_names(p, false);
        }
        let real = expand_vars(&expand_tilde(p));
        let wds: Vec<String> = if ["/", "./", "../"].iter().any(|r| real.starts_with(r)) {
            vec![".".to_string()]
        } else {
            let cdpath = crate::ported::params::getsparam("CDPATH").unwrap_or_default();
            cdpath
                .split(':')
                .filter(|d| !cdpath.is_empty() || !d.is_empty())
                .map(|d| if d.is_empty() { "." } else { d }.to_string())
                .chain(std::iter::once(".".to_string()))
                .collect()
        };
        let mut seen = HashSet::new();
        wds.iter()
            .flat_map(|wd| file_names_in(p, false, wd, true))
            .filter(|c| seen.insert(c.word.clone()))
            .collect()
    };
    files(s)
}

/// fish `complete_param_expand`: the files for the word `raw` (as typed).
/// A `--foo=bar` / `-foo=bar` word completes just `bar`; `foo=bar` the whole
/// word and also just `bar`. The split is at the last `=` or `:` that is not
/// backslashed, decided on the word as typed, and there is none while a
/// quote is open (`--f='sp`).
fn param_complete(raw: &str, cd: bool, reader: bool) -> Vec<Comp> {
    let sep = if open_quote(raw).is_some() {
        None
    } else {
        raw.char_indices()
            .filter(|&(i, c)| matches!(c, '=' | ':') && raw[..i].chars().rev().take_while(|&b| b == '\\').count() % 2 == 0)
            .last()
            .map(|(i, _)| i)
    };
    let mut out = Vec::new();
    if sep.is_none() || !raw.starts_with('-') {
        out.extend(param_files(&unescape_wild(raw), cd, reader));
    }
    if let Some(i) = sep {
        let before = unescape(&raw[..=i]);
        out.extend(param_files(&unescape_wild(&raw[i + 1..]), cd, reader).into_iter().map(|mut c| {
            c.word = format!("{before}{}", c.word);
            c.matched_as = c.matched_as.map(|m| format!("{before}{m}"));
            c.from_separator = true;
            c
        }));
    }
    out
}

fn file_names(prefix: &str, describe: bool) -> Vec<Comp> {
    file_names_in(prefix, describe, ".", false)
}

/// [`file_names`] with relative paths read from the working directory `wd`,
/// and only directories when `dirs_only` (fish `PathFilter::Directory`).
/// `prefix` may carry [`ANY_STRING`] for its unquoted `*`s.
fn file_names_in(prefix: &str, describe: bool, wd: &str, dirs_only: bool) -> Vec<Comp> {
    let (dir, base) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    // no fuzzy file matching for a word that starts with a dash (fish #568)
    let fuzzy_ok = !prefix.starts_with('-');
    let typed_dir = dir.replace(ANY_STRING, "*");
    let typed = prefix.replace(ANY_STRING, "*");
    let segs: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir[..dir.len() - 1].split('/').collect() };
    let mut out = Vec::new();
    expand_dir_segments(Walked::default(), &segs, wd, None, &mut |found, fuzz| {
        let entries = if base.contains(ANY_STRING) {
            wild_entries(&found, base, wd, fuzzy_ok, dirs_only)
        } else {
            dir_entries(&found, base, wd, fuzzy_ok, describe, dirs_only)
        };
        let n = entries.len();
        for mut c in entries {
            if let Some(rank) = fuzz {
                // the word replaces the token; its last component still ranks
                // against what was typed for it, and no better than the
                // directories it went through
                c.matched_as = Some(format!("{typed_dir}{}", &c.word[found.shown.len()..]));
                c.rank_floor = c.rank_floor.max(rank);
            } else if c.matched_as.is_none() && base.contains(ANY_STRING) {
                // a wildcard match ranks by its last literal part (rank_floor)
                c.matched_as = Some(typed.clone());
            }
            out.push(c);
        }
        n
    });
    out.sort_by(|a, b| wcsfilecmp(&a.word, &b.word));
    out
}

/// A directory part being walked: as it goes in the words (`shown`, a
/// wildcard segment kept as typed: `s*/`) and as it is read (`real`, the
/// directory the wildcard matched: `src/`).
#[derive(Clone, Default)]
struct Walked {
    shown: String,
    real: String,
}

impl Walked {
    fn join(&self, shown: &str, real: &str) -> Walked {
        Walked {
            shown: format!("{}{shown}/", self.shown),
            real: format!("{}{real}/", self.real),
        }
    }
}

/// fish `WildCardExpander::expand` over the directory segments of a path being
/// completed. A segment with a wildcard stands for every directory it matches
/// (`expand_intermediate_segment`; `**` is not expanded here). Another is
/// first taken literally; when that finds nothing and no such directory exists
/// (fish #3211), every directory whose name fuzzily matches it stands in for
/// it (`expand_literal_intermediate_segment_with_fuzz`, fish #94). `emit` gets
/// each whole directory part and, when a segment was matched fuzzily, the
/// fuzziest rank on the way, and says how many completions it found there;
/// the total is returned.
fn expand_dir_segments(found: Walked, segs: &[&str], wd: &str, fuzz: Option<u32>, emit: &mut dyn FnMut(Walked, Option<u32>) -> usize) -> usize {
    let Some((seg, rest)) = segs.split_first() else {
        return emit(found, fuzz);
    };
    let list_dirs = |dir: &str| -> Vec<String> {
        let Ok(rd) = std::fs::read_dir(real_path(dir, wd)) else {
            return Vec::new();
        };
        let mut v: Vec<String> = rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    };
    if seg.contains(ANY_STRING) {
        if seg.contains(&format!("{ANY_STRING}{ANY_STRING}")) {
            return 0;
        }
        let shown = seg.replace(ANY_STRING, "*");
        return list_dirs(&found.real)
            .into_iter()
            .filter(|name| wildcard_match(name, seg))
            .map(|name| expand_dir_segments(found.join(&shown, &name), rest, wd, fuzz, emit))
            .sum();
    }
    let literal = found.join(seg, seg);
    let found_literally = expand_dir_segments(literal.clone(), rest, wd, fuzz, emit);
    // `$VAR` / `~` segments are expanded, never fuzzily replaced
    if found_literally > 0 || seg.is_empty() || seg.contains(['$', '~']) || std::path::Path::new(&real_path(&literal.real, wd)).exists() {
        return found_literally;
    }
    list_dirs(&found.real)
        .into_iter()
        .filter_map(|name| fuzzy_rank(seg, &name).map(|rank| (name, rank)))
        .map(|(name, rank)| {
            let fuzz = Some(fuzz.map_or(rank, |f| f.max(rank)));
            expand_dir_segments(found.join(&name, &name), rest, wd, fuzz, emit)
        })
        .sum()
}

/// fish `wildcard_match` with `leading_dots_fail_to_match`: does the whole
/// `name` match `wc`, whose only wildcard is [`ANY_STRING`]? A name starting
/// with a dot needs a literal dot.
fn wildcard_match(name: &str, wc: &str) -> bool {
    if name.starts_with('.') && !wc.starts_with('.') {
        return false;
    }
    fn go(s: &[char], w: &[char]) -> bool {
        match w.split_first() {
            None => s.is_empty(),
            Some((&ANY_STRING, rest)) => (0..=s.len()).any(|i| go(&s[i..], rest)),
            Some((c, rest)) => s.first() == Some(c) && go(&s[1..], rest),
        }
    }
    let s: Vec<char> = name.chars().collect();
    let w: Vec<char> = wc.chars().collect();
    go(&s, &w)
}

/// One way a name completes a wildcard (fish `wildcard_complete_internal`):
/// the text after what was typed, or the whole name when the match is not a
/// same-case prefix (`requires_full_replacement`), and its rank.
struct WildMatch {
    text: String,
    replaces: bool,
    rank: u32,
}

/// fish `wildcard_complete_internal`. `s` is a name (a directory's with its
/// `/`), `wc` what was typed for it; each way it completes goes to `out`.
fn wildcard_complete(s: &[char], wc: &[char], orig: &str, replaces: bool, fuzzy: bool, first: bool, out: &mut Vec<WildMatch>) -> bool {
    // a hidden name needs a literal leading dot
    if first && s.first() == Some(&'.') && wc.first() != Some(&'.') {
        return false;
    }
    match wc.iter().position(|&c| c == ANY_STRING) {
        // no more wildcards: what is left is matched fuzzily
        None => {
            let (ss, ws): (String, String) = (s.iter().collect(), wc.iter().collect());
            let Some(rank) = fuzzy_rank(&ws, &ss) else {
                return false;
            };
            if !fuzzy && rank > PREFIX_ICASE {
                return false;
            }
            let samecase_prefix = ss.starts_with(&ws);
            let full = replaces || !samecase_prefix;
            let text = if full { orig.to_string() } else { ss[ws.len()..].to_string() };
            out.push(WildMatch { text, replaces: full, rank });
            true
        }
        // a literal part before the next wildcard: same case, else
        // case-insensitively and then the token is replaced
        Some(p @ 1..) => {
            if p >= s.len() {
                return false;
            }
            if s[..p] == wc[..p] {
                return wildcard_complete(&s[p..], &wc[p..], orig, replaces, fuzzy, false, out);
            }
            let fold = |v: &[char]| v.iter().flat_map(|c| c.to_lowercase()).collect::<String>();
            if fold(&s[..p]) == fold(&wc[..p]) {
                return wildcard_complete(&s[p..], &wc[p..], orig, true, fuzzy, false, out);
            }
            false
        }
        // `*`: as the last character it completes with nothing (`f*` → `f*`);
        // otherwise every place it can end, stopping at a same-case prefix
        // match (fish #929)
        Some(_) => {
            if wc.len() == 1 {
                return wildcard_complete(&[], &[], orig, replaces, fuzzy, false, out);
            }
            if wc[1] == ANY_STRING {
                return false;
            }
            let mut any = false;
            for i in 0..s.len() {
                let before = out.len();
                if wildcard_complete(&s[i..], &wc[1..], orig, replaces, fuzzy, false, out) {
                    any = true;
                    if out[before..].iter().any(|m| !m.replaces && m.rank == PREFIX) {
                        return true;
                    }
                }
            }
            any
        }
    }
}

/// The completions of a last component `base` holding a wildcard, in the
/// directory `dir` (fish `wildcard_expand` with `for_completions`): the typed
/// word extended (`*.m` → `*.md`), or the whole name when the match is not a
/// same-case prefix (`S*` → `src/`).
fn wild_entries(dir: &Walked, base: &str, wd: &str, fuzzy_ok: bool, dirs_only: bool) -> Vec<Comp> {
    let Ok(rd) = std::fs::read_dir(real_path(&dir.real, wd)) else {
        return Vec::new();
    };
    let wc: Vec<char> = base.chars().collect();
    let typed = format!("{}{}", dir.shown, base.replace(ANY_STRING, "*"));
    let mut out = Vec::new();
    for e in rd.flatten() {
        let is_dir = e.path().is_dir();
        if dirs_only && !is_dir {
            continue;
        }
        let name = format!("{}{}", e.file_name().to_string_lossy(), if is_dir { "/" } else { "" });
        let s: Vec<char> = name.chars().collect();
        let mut found = Vec::new();
        wildcard_complete(&s, &wc, &name, false, fuzzy_ok, true, &mut found);
        for m in found {
            let word = if m.replaces { format!("{}{}", dir.shown, m.text) } else { format!("{typed}{}", m.text) };
            let mut c = comp(word, "", is_dir && (m.replaces || m.text.ends_with('/')));
            c.rank_floor = m.rank;
            out.push(c);
        }
    }
    out
}

/// A directory part as typed (`$VAR`, `~/`) expanded to be read, a relative
/// one under the working directory `wd`; `wd` itself for none.
fn real_path(dir: &str, wd: &str) -> String {
    let dir = expand_vars(&expand_tilde(dir));
    match () {
        _ if dir.starts_with('/') => dir,
        _ if dir.is_empty() => wd.to_string(),
        _ if wd == "." => dir,
        _ => format!("{}/{dir}", wd.trim_end_matches('/')),
    }
}

/// The entries of the directory `dir` that the last component `base` matches,
/// with `dir` kept as shown in the words: fish completes `$HOME/Doc` to
/// `$HOME/Documents/`.
fn dir_entries(dir: &Walked, base: &str, wd: &str, fuzzy_ok: bool, describe: bool, dirs_only: bool) -> Vec<Comp> {
    let real = real_path(&dir.real, wd);
    let Ok(rd) = std::fs::read_dir(&real) else {
        return Vec::new();
    };
    // fish `open_dir(base_dir, for_completions)`: completing lists `.` and
    // `..` too (DirIter::new_with_dots), which only a typed dot reaches
    let dots = [".", ".."].map(String::from);
    rd.flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .chain(dots)
        .filter_map(|name| {
            let matched = fuzzy_rank(base, &name).is_some_and(|r| fuzzy_ok || r <= PREFIX_ICASE);
            if (name.starts_with('.') && !base.starts_with('.')) || !matched {
                return None;
            }
            let path = Path::new(&real).join(&name);
            let is_dir = path.is_dir();
            if dirs_only && !is_dir {
                return None;
            }
            let desc = match () {
                _ if !describe => "",
                _ if is_dir => "directory",
                _ if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) => "command link",
                _ => "command",
            };
            let full = format!("{}{name}{}", dir.shown, if is_dir { "/" } else { "" });
            // fish `resolve_description`: a tab in the name starts an embedded
            // description, so the file `a<TAB>b` completes as `a`, described `b`
            Some(match full.split_once(PROG_COMPLETE_SEP) {
                Some((word, embedded)) => comp(word.to_string(), embedded, is_dir),
                None => comp(full, desc, is_dir),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// `commandline` while reading (fish `src/builtins/commandline.rs`).
// ---------------------------------------------------------------------------

/// Extent (char indices) of the job / process containing the end of `s[..cursor]`.
fn scope_extent(s: &[char], cursor: usize, process: bool) -> (usize, usize) {
    let sub = innermost_cmdsubst_start(&s[..cursor]);
    let toks = tokenize(s);
    let mut start = sub;
    let mut end = s.len();
    for t in &toks {
        if t.kind != TokKind::Sep || t.start < sub {
            continue;
        }
        let w: String = s[t.start..t.end].iter().collect();
        let splits = process || matches!(w.as_str(), ";" | "\n" | "&");
        if !splits {
            continue;
        }
        if t.end <= cursor {
            start = t.end;
        } else if t.start >= cursor {
            end = end.min(t.start);
        }
    }
    if sub > 0 {
        // the substitution ends at its `)`
        let close = matching_paren(s, sub - 1);
        end = end.min(close);
    }
    (start, end.max(start))
}

/// `commandline` with only reading options, against `buffer` / `cursor` — or
/// the completion's transient line while one runs. Prints like fish.
pub fn commandline_read(args: &[String], buffer: &str, cursor: usize) -> i32 {
    let (buffer, cursor) = match transient().last().cloned() {
        Some(t) => {
            let n = t.chars().count();
            (t, n)
        }
        None => (buffer.to_string(), cursor),
    };
    let s: Vec<char> = buffer.chars().collect();
    let cursor = cursor.min(s.len());
    let mut scope = 'b';
    let mut cut = false;
    let mut mode: Option<char> = None;
    let mut want_cursor = false;
    for a in args {
        if let Some(l) = a.strip_prefix("--") {
            match l {
                "current-buffer" => scope = 'b',
                "current-job" => scope = 'j',
                "current-process" => scope = 'p',
                "current-token" => scope = 't',
                "cut-at-cursor" => cut = true,
                "tokenize" => mode = Some('o'),
                "tokens-expanded" => mode = Some('x'),
                "tokens-raw" => mode = Some('r'),
                "cursor" => want_cursor = true,
                "is-valid" => return 0,
                "paging-mode" | "paging-full-mode" | "search-mode" => return 1,
                _ => {}
            }
            continue;
        }
        for c in a.chars().skip(1) {
            match c {
                'b' | 'j' | 'p' | 't' => scope = c,
                'c' => cut = true,
                'o' | 'x' => mode = Some(c),
                'C' => want_cursor = true,
                'L' => return 0,
                'P' | 'S' => return 1,
                _ => {}
            }
        }
    }
    if want_cursor {
        println!("{cursor}");
        return 0;
    }
    let (start, end) = match scope {
        'j' => scope_extent(&s, cursor, false),
        'p' => scope_extent(&s, cursor, true),
        't' => {
            let toks = tokenize(&s);
            match toks.iter().find(|t| t.kind == TokKind::Str && t.start <= cursor && cursor <= t.end) {
                Some(t) => (t.start, t.end),
                None => (cursor, cursor),
            }
        }
        _ => (0, s.len()),
    };
    let range = &s[start..end];
    let pos = cursor.saturating_sub(start).min(range.len());
    let Some(mode) = mode else {
        let text: String = if cut { range[..pos].iter().collect() } else { range.iter().collect() };
        println!("{text}");
        return 0;
    };
    let mut words: Vec<String> = Vec::new();
    let raw_tokens: Vec<String> = if scope == 't' {
        vec![range.iter().collect()]
    } else {
        let mut v = Vec::new();
        let mut redirect_target = false;
        for t in tokenize(range) {
            if cut && t.end >= pos {
                break;
            }
            let was_redirect = redirect_target;
            redirect_target = t.kind == TokKind::Redirect;
            if t.kind != TokKind::Str || was_redirect {
                continue;
            }
            v.push(range[t.start..t.end].iter().collect());
        }
        v
    };
    for t in raw_tokens {
        match mode {
            'r' => words.push(t),
            'x' if t.contains(['$', '~', '{', '*', '?']) && !t.contains('(') => {
                let got = ShellEval.args(&t);
                if got.is_empty() {
                    words.push(unescape(&t));
                } else {
                    words.extend(got);
                }
            }
            _ => words.push(unescape(&t)),
        }
    }
    let mut out = String::new();
    for w in words {
        out.push_str(&w);
        out.push('\n');
    }
    print!("{out}");
    0
}

// ---------------------------------------------------------------------------
// The commands the preamble and `_fish_complete` call.
// ---------------------------------------------------------------------------

/// `__fish_complete_query TEXT`: complete the argument at the end of TEXT
/// (the line before the cursor). Sets `__fish_cw` (words), `__fish_cd`
/// (descriptions), `__fish_cs` (`1` = no space after), `__fish_ce` (the
/// word escaped as fish inserts it, set only when zsh would quote it otherwise), `__fish_cf` (`1` =
/// files too), `__fish_ck` (`1` = keep the order), `__fish_ct` (the word as
/// typed), `__fish_cqt` (the quote the word left open). Status 1 when no fish definition covers the command.
fn query_cmd(argv: &[String]) -> i32 {
    let text = argv.iter().skip(1).find(|a| *a != "--").cloned().unwrap_or_default();
    // (what is ranked, the whole word as typed, the completions)
    let mut raw = String::new();
    let (typed, whole, mut ans) = match locate(&text) {
        Position::Argument(line) => {
            let Some(ans) = complete_argument(&line, &mut ShellEval) else {
                return 1;
            };
            raw = line.cur_raw;
            (line.cur, line.cur_wild, ans)
        }
        Position::Command(prefix) => {
            let typed = unescape(&prefix);
            let comps = command_names(&typed);
            (typed.clone(), typed, Answer { comps, ..Answer::default() })
        }
        Position::Variable { before, name } => {
            let comps = variable_comps(&before, &name);
            (name.clone(), format!("{before}{name}"), Answer { comps, ..Answer::default() })
        }
        Position::User(name) => {
            let comps = user_comps(&name);
            (name.clone(), format!("~{name}"), Answer { comps, ..Answer::default() })
        }
        // fish completes nothing here (a comment, a redirection in the
        // command's place); zsh's completers must not either
        Position::Nothing => (String::new(), String::new(), Answer::default()),
    };
    // an argument's word keeps its unquoted `*`s and braces marked, for file completion
    let wild = whole.contains([ANY_STRING, BRACE_BEGIN]);
    let whole_wild = whole;
    let whole = wild_text(&whole_wild);
    // the files come from here too, ranked with the rest as fish does: zsh's
    // _files knows nothing of fuzzy directories (`s/e/f`), `--opt=` and
    // `foo:` separators, `cd`'s directories through $CDPATH, or keeping a
    // `$HOME` in the word
    if ans.do_file {
        ans.comps.extend(param_complete(&raw, ans.cd, true));
        ans.do_file = false;
    }
    let ranks: Vec<Option<u32>> = ans.comps.iter().map(|c| rank_of(&typed, c)).collect();
    let best = ranks.iter().flatten().copied().min();
    let mut keep = ranks.iter().map(|r| r.is_some() && *r == best);
    ans.comps.retain(|_| keep.next().unwrap_or(false));
    // fish replaces the word when the best match is weaker than a prefix
    // (`-v` finding `--verbose`) or a word does not extend it (`s/e/f`)
    // fish `handle_completions` for several matches: none is inserted unless every
    // one is a prefix match. When all of them replace the word, their common
    // prefix goes in only if it is longer than the word; otherwise the common
    // prefix of those that extend the word goes in, and when there is none the
    // matches are only listed. (zsh inserts what all its matches share, so the
    // replacing ones are left out when the extending ones have more in common.)
    let extends = |c: &Comp| c.word.starts_with(whole.as_str());
    let common_len = |words: &mut dyn Iterator<Item = &Comp>| {
        words
            .map(|c| c.word.as_str())
            .reduce(|a, b| {
                let n = a.chars().zip(b.chars()).take_while(|(x, y)| x == y).map(|(x, _)| x.len_utf8()).sum();
                &a[..n]
            })
            .map_or(0, |c| c.chars().count())
    };
    let all_prefix = best.is_some_and(|b| b % FROM_SEPARATOR <= PREFIX_ICASE);
    let all_replace = !ans.comps.iter().any(extends);
    let typed_len = whole.chars().count();
    let mut list_only = false;
    if ans.comps.len() > 1 {
        if !all_prefix {
            list_only = true;
        } else if all_replace {
            list_only = common_len(&mut ans.comps.iter()) <= typed_len;
        } else if common_len(&mut ans.comps.iter().filter(|c| extends(c))) > typed_len {
            ans.comps.retain(extends);
        } else {
            list_only = true;
        }
    }
    let replace = best.is_some_and(|b| b > PREFIX) || !ans.comps.iter().all(extends);
    use crate::ported::params::{setaparam, setsparam};
    setsparam("__fish_cu", if replace { "1" } else { "0" });
    // a word through a variable, a `~user` or a wildcard goes in as written,
    // `$`, `~` and `*` and all
    let as_written = wild || whole.contains('$') || whole.starts_with('~');
    setsparam("__fish_cq", if as_written { "1" } else { "0" });
    // the word the matches are for, as typed (compsys may hold less of it)
    setsparam("__fish_ct", &whole);
    // the quote the word left open: fish closes it after a match that ends
    // the word (reader.rs `completion_apply_to_command_line`)
    setsparam("__fish_cqt", &open_quote(&text).map(String::from).unwrap_or_default());
    setaparam("__fish_cw", ans.comps.iter().map(|c| c.word.clone()).collect());
    // zsh would quote a control character `$'\n'`, which fish reads as `$` and
    // `\n`; such a match goes in escaped as fish escapes it
    setaparam(
        "__fish_ce",
        ans.comps
            .iter()
            .map(|c| if c.word.chars().any(char::is_control) { escape_for_insert(&c.word) } else { String::new() })
            .collect(),
    );
    setaparam("__fish_cd", ans.comps.iter().map(|c| c.desc.clone()).collect());
    setaparam("__fish_cs", ans.comps.iter().map(|c| if c.no_space { "1" } else { "0" }.to_string()).collect());
    setsparam("__fish_cf", if ans.do_file { "1" } else { "0" });
    setsparam("__fish_ck", if ans.keep_order { "1" } else { "0" });
    setsparam("__fish_clist", if list_only { "1" } else { "0" });
    0
}

/// These commands run shell code of their own (conditions, `-a` lists, loaded
/// scripts), which leaves the executor's status at whatever ran last; the
/// command's own result is put back so a `-c` run ending in `complete -C …`
/// exits with it.
fn settle(rc: i32) -> i32 {
    crate::fusevm_bridge::try_with_executor(|e| e.set_last_status(rc));
    crate::ported::builtin::LASTVAL.store(rc, std::sync::atomic::Ordering::Relaxed);
    rc
}

/// Register the fish completion commands in the native-command table.
pub fn register_all() {
    crate::native_cmds::register("__fish_complete", |argv| settle(builtin_complete(argv)));
    crate::native_cmds::register("__fish_complete_query", |argv| settle(query_cmd(argv)));
    // `function NAME --wraps TARGET`: fish `complete_add_wrapper` alone, with no
    // completion entry for NAME (unlike `complete -c NAME --wraps TARGET`)
    crate::native_cmds::register("__fish_complete_wrap", |argv| match (argv.get(1), argv.get(2)) {
        (Some(cmd), Some(target)) => {
            complete_add_wrapper(cmd, target);
            0
        }
        _ => 2,
    });
    crate::native_cmds::register("__fish_autoload", |argv| {
        settle(i32::from(!argv.get(1).is_some_and(|n| autoload_function(n))))
    });
    // `functions -n` / `builtin -n` in the preamble list what fish would
    crate::native_cmds::register("__fish_function_names", |argv| {
        let hidden = argv.iter().skip(1).any(|a| a == "-a");
        let mut out = String::new();
        for f in function_names(hidden) {
            out.push_str(&f);
            out.push('\n');
        }
        print!("{out}");
        0
    });
    crate::native_cmds::register("__fish_builtin_names", |_| {
        let mut out = String::new();
        for (n, _) in FISH_BUILTINS {
            out.push_str(n);
            out.push('\n');
        }
        print!("{out}");
        0
    });
    crate::native_cmds::register("__fish_cmdline_read", |argv| {
        // __fish_cmdline_read BUFFER CURSOR ARGS…
        let buffer = argv.get(1).cloned().unwrap_or_default();
        let cursor = argv.get(2).and_then(|c| c.parse().ok()).unwrap_or(0);
        settle(commandline_read(argv.get(3..).unwrap_or(&[]), &buffer, cursor))
    });
}

// ---------------------------------------------------------------------------
// TAB inside a fish `(…)` substitution.
// ---------------------------------------------------------------------------

/// Positions (in the editor buffer) where `$` was put in front of a fish `(`.
pub struct SubstRewrite(Vec<usize>);

/// Before a completion in `--fish`: zsh's completion lexer knows `$(…)` but
/// not fish's bare `(…)`, so every unquoted `(` before the cursor gains a `$`
/// for the duration (`ls (echo src/ext` completes like `ls $(echo src/ext`).
/// Dropping the value takes the `$`s back out.
pub fn subst_rewrite() -> Option<SubstRewrite> {
    use crate::ported::zle::zle_main::{ZLECS, ZLELINE, ZLELL};
    use std::sync::atomic::Ordering;
    if crate::emulation_startup::personality() != crate::emulation_startup::Personality::Fish {
        return None;
    }
    let mut line = ZLELINE.lock().ok()?;
    let cs = ZLECS.load(Ordering::SeqCst).min(line.len());
    let mut at = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < cs {
        let c = line[i];
        match quote {
            Some(q) => {
                if c == '\\' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\\' => i += 1,
                '\'' | '"' => quote = Some(c),
                '(' if i == 0 || line[i - 1] != '$' => at.push(i),
                _ => {}
            },
        }
        i += 1;
    }
    if at.is_empty() {
        return None;
    }
    let mut shifted = Vec::with_capacity(at.len());
    for (n, p) in at.iter().enumerate() {
        line.insert(p + n, '$');
        shifted.push(p + n);
    }
    ZLECS.store(cs + at.len(), Ordering::SeqCst);
    ZLELL.store(line.len(), Ordering::SeqCst);
    Some(SubstRewrite(shifted))
}

impl Drop for SubstRewrite {
    fn drop(&mut self) {
        use crate::ported::zle::zle_main::{ZLECS, ZLELINE, ZLELL};
        use std::sync::atomic::Ordering;
        let Ok(mut line) = ZLELINE.lock() else { return };
        // a completion only edits at or after the word it completed, which
        // starts after the last of these
        if !self.0.iter().all(|&p| line.get(p) == Some(&'$') && line.get(p + 1) == Some(&'(')) {
            return;
        }
        let mut cs = ZLECS.load(Ordering::SeqCst);
        for &p in self.0.iter().rev() {
            line.remove(p);
            if p < cs {
                cs -= 1;
            }
        }
        ZLECS.store(cs, Ordering::SeqCst);
        ZLELL.store(line.len(), Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Conditions answered from a table, `-a` lists split on spaces.
    struct Fake {
        conds: HashMap<&'static str, bool>,
    }

    impl Eval for Fake {
        fn test(&mut self, cond: &str) -> bool {
            *self.conds.get(cond).unwrap_or(&false)
        }
        fn args(&mut self, args: &str) -> Vec<String> {
            args.split(' ').filter(|w| !w.is_empty()).map(|w| w.replace("\\t", "\t")).collect()
        }
        fn load(&mut self, _cmd: &str) {}
    }

    fn add(argv: &[&str]) {
        let mut v = vec!["complete".to_string()];
        v.extend(argv.iter().map(|s| s.to_string()));
        assert_eq!(builtin_complete(&v), 0, "{argv:?}");
    }

    /// The words the completer offers for the argument at the end of `text`,
    /// narrowed to the typed prefix the way compadd narrows them.
    fn words(text: &str, conds: &[(&'static str, bool)]) -> Option<Vec<String>> {
        let Position::Argument(line) = locate(text) else { return None };
        let mut f = Fake { conds: conds.iter().copied().collect() };
        let cur = line.cur.clone();
        complete_argument(&line, &mut f).map(|a| a.comps.into_iter().map(|c| c.word).filter(|w| w.starts_with(&cur)).collect())
    }

    #[test]
    fn options_arguments_and_conditions() {
        add(&["-c", "zt1", "-s", "v", "-l", "verbose", "-d", "Be loud"]);
        add(&["-c", "zt1", "-l", "color", "-xa", "auto never"]);
        add(&["-c", "zt1", "-n", "seen-sub", "-s", "q"]);
        add(&["-c", "zt1", "-f", "-a", "start stop"]);
        assert_eq!(words("zt1 st", &[]).unwrap(), ["start", "stop"]);
        assert_eq!(words("zt1 --v", &[]).unwrap(), ["--verbose"]);
        // fish lists an entry's options newest first
        assert_eq!(words("zt1 -", &[]).unwrap(), ["--color", "--verbose", "-v"]);
        assert_eq!(words("zt1 -", &[("seen-sub", true)]).unwrap(), ["-q", "--color", "--verbose", "-v"]);
        // an option that requires a parameter takes the next word, and only it
        assert_eq!(words("zt1 --color ", &[]).unwrap(), ["auto", "never"]);
        assert_eq!(words("zt1 --color=a", &[]).unwrap(), ["--color=auto"]);
        // short options combine, never twice
        assert_eq!(words("zt1 -v", &[("seen-sub", true)]).unwrap(), ["-vq"]);
        assert_eq!(words("zt1 -vq", &[("seen-sub", true)]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn no_files_and_force_files() {
        add(&["-c", "zt5", "-f", "-a", "x"]);
        add(&["-c", "zt5", "-l", "out", "-F", "-r"]);
        let ans = |t: &str| {
            let Position::Argument(line) = locate(t) else { panic!("{t}") };
            complete_argument(&line, &mut Fake { conds: HashMap::new() }).unwrap()
        };
        assert!(!ans("zt5 ").do_file);
        assert!(ans("zt5 --out ").do_file);
    }

    #[test]
    fn command_word_variables_and_unknown_commands_are_left_to_compsys() {
        assert!(matches!(locate("gi"), Position::Command(_)));
        assert!(matches!(locate("echo hi; gi"), Position::Command(_)));
        assert!(matches!(locate("echo $HO"), Position::Variable { ref before, ref name } if before == "$" && name == "HO"));
        assert!(matches!(locate("echo \"a$HO"), Position::Variable { ref before, .. } if before == "\"a$"));
        assert!(matches!(locate("echo '$HO"), Position::Argument(_)));
        assert!(matches!(locate("echo $HOME/x"), Position::Argument(_)));
        assert!(matches!(locate("echo ~ro"), Position::User(ref u) if u == "ro"));
        assert!(matches!(locate("$HO"), Position::Variable { .. }));
        assert!(words("zt-nothing-defined arg", &[]).is_none());
    }

    #[test]
    fn the_process_at_the_cursor_is_found_through_separators_assignments_and_substitutions() {
        add(&["-c", "zt2", "-xa", "one two"]);
        assert_eq!(words("true; zt2 ", &[]).unwrap(), ["one", "two"]);
        assert_eq!(words("x | zt2 ", &[]).unwrap(), ["one", "two"]);
        assert_eq!(words("echo (zt2 ", &[]).unwrap(), ["one", "two"]);
        assert_eq!(words("echo $(zt2 o", &[]).unwrap(), ["one"]);
        assert_eq!(words("A=1 B=2 zt2 ", &[]).unwrap(), ["one", "two"]);
        // `not`, `command`, ... are the command: their own definitions answer
        add(&["-c", "zt2not", "-xa", "own"]);
        assert_eq!(words("true; zt2not zt2 ", &[]).unwrap(), ["own"]);
    }

    #[test]
    fn wraps_follow_the_chain() {
        add(&["-c", "zt3", "-xa", "base"]);
        add(&["-c", "zt3w", "--wraps", "zt3"]);
        assert_eq!(words("zt3w ", &[]).unwrap(), ["base"]);
    }

    #[test]
    fn long_options_may_be_abbreviated() {
        // ansible.fish: `complete -c ansible -s P --l poll -r`
        add(&["-c", "zt6", "--l", "poll", "--desc", "Poll"]);
        assert_eq!(words("zt6 --p", &[]).unwrap(), ["--poll"]);
        // `--co` is both --command and --condition
        let argv: Vec<String> = ["complete", "--co", "x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(builtin_complete(&argv), 2);
    }

    #[test]
    fn erase_and_print() {
        add(&["-c", "zt4", "-s", "a", "-d", "it's"]);
        // fish 4.9: `complete zt4 -s a -d "it's"`
        assert_eq!(complete_print("zt4"), "complete zt4 -s a -d \"it's\"\n");
        add(&["-e", "-c", "zt4"]);
        assert_eq!(complete_print("zt4"), "");
    }

    #[test]
    fn tokens_and_unescape() {
        let s: Vec<char> = "a 'b c' \"d$(e f)\" g\\ h 2>/dev/null x".chars().collect();
        let toks: Vec<String> = tokenize(&s).iter().map(|t| s[t.start..t.end].iter().collect()).collect();
        assert_eq!(toks, ["a", "'b c'", "\"d$(e f)\"", "g\\ h", "2>", "/dev/null", "x"]);
        assert_eq!(unescape("'it\\'s'\\ x\"y\\$\""), "it's xy$");
    }

    #[test]
    fn fuzzy_ranks_follow_fish() {
        assert_eq!(fuzzy_rank("--v", "--verbose"), Some(PREFIX));
        assert_eq!(fuzzy_rank("--V", "--verbose"), Some(PREFIX_ICASE));
        // all-lowercase input is smartcase: as good as an exact-case prefix
        assert_eq!(fuzzy_rank("ma", "MAINTAINERS.md"), Some(PREFIX));
        assert_eq!(fuzzy_rank("-v", "--verbose"), Some(SUBSTR));
        assert_eq!(fuzzy_rank("-V", "--verbose"), Some(SUBSTR_ICASE));
        assert_eq!(fuzzy_rank("vbs", "verbose"), Some(SUBSEQ));
        assert_eq!(fuzzy_rank("-", "s_client"), Some(SUBSTR));
        assert_eq!(fuzzy_rank("verbosely", "verbose"), None);
    }

    #[test]
    fn natural_order_matches_fish() {
        let mut v = vec!["file12", "File1", "file5", "--x", "-a", "a-b", "aZ", "a/b"];
        v.sort_by(|a, b| wcsfilecmp(a, b));
        // fish 4.9: `complete -c zz -f -a '<these>'; complete -C 'zz '`
        assert_eq!(v, ["a/b", "aZ", "a-b", "File1", "file5", "file12", "-a", "--x"]);
    }

    #[test]
    fn argument_lists_lose_newlines_only_between_words() {
        assert_eq!(flatten_newlines("a\nb 'c\nd'"), "a b 'c\nd'");
        // set.fish: a multi-line substitution keeps its command separators
        assert_eq!(flatten_newlines("(set -g |\n string match (\n echo x\n))\nz"), "(set -g |\n string match (\n echo x\n)) z");
    }

    #[test]
    fn innermost_substitution() {
        let s: Vec<char> = "echo (ls (pwd) src/".chars().collect();
        assert_eq!(innermost_cmdsubst_start(&s), 6);
        let s: Vec<char> = "echo '(' x".chars().collect();
        assert_eq!(innermost_cmdsubst_start(&s), 0);
    }

    #[test]
    fn inserted_matches_escape_as_fish() {
        // fish 4.9: `string escape --no-quoted -- <input>`
        let cases = [
            ("a b", "a\\ b"),
            ("x#y", "x#y"),
            ("#x", "\\#x"),
            ("a?b", "a?b"),
            ("a~t", "a\\~t"),
            ("it's", "it\\'s"),
            ("b\\s", "b\\\\s"),
            ("t\tx", "t\\tx"),
            ("n\nx", "n\\nx"),
            ("c\x07x", "c\\cgx"),
            ("e\x1bx", "e\\ex"),
            ("d\x7fx", "d\\x7fx"),
            ("u\x1ax", "u\x1ax"),
            ("%p", "\\%p"),
            ("a&b;c|d", "a\\&b\\;c\\|d"),
            ("(x)[y]{z}<>", "\\(x\\)\\[y\\]\\{z\\}\\<\\>"),
            ("a$b*c", "a\\$b\\*c"),
        ];
        for (input, want) in cases {
            assert_eq!(escape_for_insert(input), want, "{input:?}");
        }
    }

    /// Expected values are fish 4.9's `string escape` output.
    #[test]
    fn values_escape_as_fish() {
        let cases = [
            ("ok", "ok"),
            ("a,b", "a,b"),
            ("a#b", "a#b"),
            ("", "''"),
            ("a*b", "'a*b'"),
            ("a b", "'a b'"),
            ("#a", "'#a'"),
            ("x~", "'x~'"),
            ("a\\b", "'a\\\\b'"),
            ("a$b", "'a$b'"),
            ("a\"b", "'a\"b'"),
            ("a'b", "\"a'b\""),
            ("it''s", "\"it''s\""),
            ("a'$b", "'a\\'$b'"),
            ("it's \"x\"", "'it\\'s \"x\"'"),
            ("\n \t", "\\n\\ \\t"),
        ];
        for (input, want) in cases {
            assert_eq!(escape(input), want, "{input:?}");
        }
    }
}
