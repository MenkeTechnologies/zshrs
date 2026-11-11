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

/// fish's `escape` for printing: bare when nothing needs quoting, otherwise
/// single quotes with `\'` and `\\` inside.
fn escape(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || "_-+=.,/:@%^".contains(c)) {
        return s.to_string();
    }
    let mut out = String::from("'");
    for c in s.chars() {
        if c == '\'' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('\'');
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

fn is_redirect_at(s: &[char], i: usize) -> Option<usize> {
    let mut j = i;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
    }
    let rest: String = s[j..s.len().min(j + 3)].iter().collect();
    let op = ["&>>", "&>", ">>", ">&", ">?", ">|", "<&", ">", "<"]
        .into_iter()
        .find(|op| rest.starts_with(op))?;
    if j > i && op.starts_with('&') {
        return None;
    }
    let mut end = j + op.chars().count();
    // `2>&1`, `>&-`: the fd target belongs to the operator
    if op.ends_with('&') && end < s.len() && (s[end].is_ascii_digit() || s[end] == '-') {
        end += 1;
    }
    Some(end)
}

/// Tokenize `s`, accepting an unfinished last token (open quote or bracket).
fn tokenize(s: &[char]) -> Vec<Tok> {
    let mut out = Vec::new();
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
        if let Some(end) = is_redirect_at(s, i) {
            out.push(Tok { start: i, end, kind: TokKind::Redirect });
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
    out
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

/// fish's script unescape, incomplete-tolerant: quotes removed, backslash
/// escapes decoded. Variables, `~` and substitutions stay as written.
fn unescape(tok: &str) -> String {
    let s: Vec<char> = tok.chars().collect();
    let mut out = String::new();
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
                    }
                    out.push(s[i]);
                    i += 1;
                }
            }
            _ => out.push(c),
        }
        i += 1;
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


/// fish `wildcard_match` for command names: `*` and `?`.
fn wildcard_match(s: &str, pat: &str) -> bool {
    fn go(s: &[char], p: &[char]) -> bool {
        match p.first() {
            None => s.is_empty(),
            Some('*') => (0..=s.len()).any(|k| go(&s[k..], &p[1..])),
            Some('?') => !s.is_empty() && go(&s[1..], &p[1..]),
            Some(c) => s.first() == Some(c) && go(&s[1..], &p[1..]),
        }
    }
    let s: Vec<char> = s.chars().collect();
    let p: Vec<char> = pat.chars().collect();
    go(&s, &p)
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
}

/// What a query found.
#[derive(Debug, Default)]
pub struct Answer {
    pub comps: Vec<Comp>,
    /// file completions apply too
    pub do_file: bool,
    /// a `-k` definition contributed: keep the order
    pub keep_order: bool,
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
            });
        }
    }

    /// fish `complete_param_for_command`: `s` is the argument being completed,
    /// `popt` the one before it.
    fn param_for_command(&mut self, cmd: &str, popt: &str, s: &str, use_switches: bool, do_file: &mut bool) {
        let mut use_files = true;
        let mut has_force = false;
        let name = cmd.rsplit('/').next().unwrap_or(cmd).to_string();
        self.eval.load(&name);
        let all: Vec<Vec<Opt>> = {
            let st = store();
            if st.tombstones.contains(&name) {
                self.covered = true;
            }
            st.map
                .iter()
                .filter(|((n, is_path), _)| {
                    let subject = if *is_path { cmd } else { name.as_str() };
                    wildcard_match(subject, n)
                })
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
                    });
                }
                self.out.comps.push(Comp {
                    word: whole,
                    desc: o.desc.clone(),
                    no_space: false,
                    keep: false,
        matched_as: None,
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
    /// the effective command line: from the command word to the cursor
    effective: String,
    cmd: String,
    /// the command word's extent in `effective`
    cmd_range: (usize, usize),
    prev: String,
    cur: String,
    had_ddash: bool,
    in_redirection: bool,
}

/// What the cursor is on.
enum Position {
    /// completing the command word itself (or nothing typed yet)
    Command(String),
    /// a `$var` or `~user`: compsys does those
    Special,
    Argument(Line),
}

/// fish `perform_for_commandline_impl`, up to the point where the argument
/// completers run.
fn locate(text: &str) -> Position {
    let all: Vec<char> = text.chars().collect();
    let sub = innermost_cmdsubst_start(&all);
    let s = &all[sub..];
    let toks = tokenize(s);
    // the last process: tokens after the last separator
    let first = toks.iter().rposition(|t| t.kind == TokKind::Sep).map_or(0, |p| p + 1);
    let mut toks: Vec<Tok> = toks[first..].to_vec();
    let at_end = |t: &Tok| t.end >= s.len();
    // `VAR=val` before the command word. `and`, `not`, `command`, `time`, ...
    // stay: fish completes them from their own scripts (not.fish, command.fish,
    // ...), which hand the rest of the line to `__fish_complete_subcommand`.
    while let Some(t) = toks.first() {
        let w: String = s[t.start..t.end].iter().collect();
        if at_end(t) || t.kind != TokKind::Str || !is_var_assignment(&w) {
            break;
        }
        toks.remove(0);
    }
    let Some(cmd_tok) = toks.first().copied() else {
        return Position::Command(String::new());
    };
    if cmd_tok.kind != TokKind::Str {
        return Position::Special;
    }
    let tok_text = |t: &Tok| -> String { s[t.start..t.end].iter().collect() };
    let last = *toks.last().expect("non-empty");
    let cursor_in_ws = !at_end(&last);
    if !cursor_in_ws && toks.len() == 1 {
        return Position::Command(tok_text(&cmd_tok));
    }
    let (cur_raw, prev_tok) = if cursor_in_ws {
        (String::new(), Some(last))
    } else {
        (tok_text(&last), (toks.len() >= 2).then(|| toks[toks.len() - 2]))
    };
    if cur_raw.contains('$') || cur_raw.starts_with('~') && !cur_raw.contains('/') {
        return Position::Special;
    }
    let in_redirection = (!cursor_in_ws && last.kind == TokKind::Redirect)
        || prev_tok.is_some_and(|p| p.kind == TokKind::Redirect);
    let prev = prev_tok
        .filter(|p| p.kind == TokKind::Str)
        .map(|p| unescape(&tok_text(&p)))
        .unwrap_or_default();
    let upto = if cursor_in_ws { toks.len() } else { toks.len() - 1 };
    let had_ddash = toks[1..upto.max(1)].iter().any(|t| tok_text(t) == "--");
    let effective: String = s[cmd_tok.start..].iter().collect();
    Position::Argument(Line {
        effective,
        cmd: unescape(&tok_text(&cmd_tok)),
        cmd_range: (0, cmd_tok.end - cmd_tok.start),
        prev,
        cur: unescape(&cur_raw),
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
struct FishData {
    exe: PathBuf,
    cache: PathBuf,
    sysconf: String,
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

fn fish_data() -> Option<&'static FishData> {
    static DATA: OnceLock<Option<FishData>> = OnceLock::new();
    DATA.get_or_init(|| {
        let exe = find_on_path("fish")?;
        let meta = std::fs::metadata(&exe).ok()?;
        let stamp = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_secs());
        let cache = zshrs_home().join("fish-data").join(format!("{}-{stamp}", meta.len()));
        let index = cache.join("index");
        let text = match std::fs::read_to_string(&index) {
            Ok(t) => t,
            Err(_) => {
                let out = std::process::Command::new(&exe)
                    .args([
                        "-N",
                        "-c",
                        "echo $__fish_sysconf_dir; status get-file __fish_build_paths.fish 2>/dev/null | source; \
                         echo $__extra_completionsdir; echo $__extra_functionsdir; status list-files",
                    ])
                    .stdin(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .output()
                    .ok()?;
                if !out.status.success() {
                    return None;
                }
                write_atomic(&index, &out.stdout);
                String::from_utf8_lossy(&out.stdout).into_owned()
            }
        };
        let mut lines = text.lines();
        let sysconf = lines.next()?.to_string();
        let extra_completions = lines.next()?.to_string();
        let extra_functions = lines.next()?.to_string();
        Some(FishData {
            exe,
            cache,
            sysconf,
            extra_completions,
            extra_functions,
            files: lines.map(str::to_string).collect(),
        })
    })
    .as_ref()
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
    let out = std::process::Command::new(&d.exe)
        .args(["-N", "-c", "status get-file $argv[1]", rel])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    write_atomic(&path, &out.stdout);
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// fish's default `fish_function_path` / `fish_complete_path` (share/config.fish).
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
    let (typed, comps) = match locate(text) {
        Position::Command(prefix) => {
            let typed = unescape(&prefix);
            let comps = command_names(&typed);
            (typed, comps)
        }
        Position::Special => (String::new(), Vec::new()),
        Position::Argument(line) => {
            let ans = complete_argument(&line, &mut ShellEval);
            let do_file = ans.as_ref().is_none_or(|a| a.do_file);
            let mut comps = ans.map(|a| a.comps).unwrap_or_default();
            if do_file {
                comps.extend(file_names(&line.cur, false));
            }
            (line.cur, comps)
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
        (false, false) => wcsfilecmp(sort_key(&typed, &a.word), sort_key(&typed, &b.word)),
    });
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

/// The rank of `c` for the typed word (see [`Comp::matched_as`]).
fn rank_of(typed: &str, c: &Comp) -> Option<u32> {
    fuzzy_rank(typed, c.matched_as.as_deref().unwrap_or(&c.word))
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
    }
}

/// fish `complete_cmd` for the command word: executables, then directories,
/// then functions (autoloadable ones included), then builtins; the first
/// source to offer a name gives its description.
fn command_names(prefix: &str) -> Vec<Comp> {
    if prefix.contains('/') || prefix.starts_with('~') {
        // a path: executables and directories under it
        return file_names(prefix, true)
            .into_iter()
            .filter(|c| c.word.ends_with('/') || is_executable(&expand_tilde(&c.word)))
            .collect();
    }
    let mut out = Vec::new();
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
    if let Ok(rd) = std::fs::read_dir(".") {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_dir() && (!name.starts_with('.') || prefix.starts_with('.')) {
                out.push(comp(format!("{name}/"), "directory", true));
            }
        }
    }
    for f in function_names(prefix.starts_with('_')) {
        out.push(comp(f, "", false));
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
fn file_names(prefix: &str, describe: bool) -> Vec<Comp> {
    let (dir, base) = match prefix.rfind('/') {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    let real_dir = if dir.is_empty() { ".".to_string() } else { expand_tilde(dir) };
    let Ok(rd) = std::fs::read_dir(&real_dir) else {
        return Vec::new();
    };
    let mut out: Vec<Comp> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            // no fuzzy file matching for a word that starts with a dash (fish #568)
            let fuzzy_ok = !prefix.starts_with('-');
            let matched = fuzzy_rank(base, &name).is_some_and(|r| fuzzy_ok || r <= PREFIX_ICASE);
            if (name.starts_with('.') && !base.starts_with('.')) || !matched {
                return None;
            }
            let path = e.path();
            let is_dir = path.is_dir();
            let desc = match () {
                _ if !describe => "",
                _ if is_dir => "directory",
                _ if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) => "command link",
                _ => "command",
            };
            Some(comp(format!("{dir}{name}{}", if is_dir { "/" } else { "" }), desc, is_dir))
        })
        .collect();
    out.sort_by(|a, b| wcsfilecmp(&a.word, &b.word));
    out
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
/// (descriptions), `__fish_cs` (`1` = no space after), `__fish_cf` (`1` =
/// files too), `__fish_ck` (`1` = keep the order). Status 1 when no fish
/// definition covers the command.
fn query_cmd(argv: &[String]) -> i32 {
    let text = argv.iter().skip(1).find(|a| *a != "--").cloned().unwrap_or_default();
    let Position::Argument(line) = locate(&text) else {
        return 1;
    };
    let Some(mut ans) = complete_argument(&line, &mut ShellEval) else {
        return 1;
    };
    // fish keeps only the best kind of match. Files count as prefix matches
    // when one exists; when the best is weaker than a prefix (`-v` finding
    // `--verbose`) the word is replaced rather than extended (`__fish_cu`).
    let ranks: Vec<Option<u32>> = ans.comps.iter().map(|c| rank_of(&line.cur, c)).collect();
    let files_match = ans.do_file && file_names(&line.cur, false).iter().any(|c| fuzzy_rank(&line.cur, &c.word) == Some(PREFIX));
    let best = ranks.iter().flatten().copied().chain(files_match.then_some(PREFIX)).min();
    let mut keep = ranks.iter().map(|r| r.is_some() && *r == best);
    ans.comps.retain(|_| keep.next().unwrap_or(false));
    let replace = best.is_some_and(|b| b > PREFIX);
    if replace {
        ans.do_file = false;
    }
    use crate::ported::params::{setaparam, setsparam};
    setsparam("__fish_cu", if replace { "1" } else { "0" });
    setaparam("__fish_cw", ans.comps.iter().map(|c| c.word.clone()).collect());
    setaparam("__fish_cd", ans.comps.iter().map(|c| c.desc.clone()).collect());
    setaparam("__fish_cs", ans.comps.iter().map(|c| if c.no_space { "1" } else { "0" }.to_string()).collect());
    setsparam("__fish_cf", if ans.do_file { "1" } else { "0" });
    setsparam("__fish_ck", if ans.keep_order { "1" } else { "0" });
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
        assert!(matches!(locate("echo $HO"), Position::Special));
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
        assert_eq!(complete_print("zt4"), "complete zt4 -s a -d 'it\\'s'\n");
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
}
