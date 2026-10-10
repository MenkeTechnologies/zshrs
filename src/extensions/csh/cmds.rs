//! One csh command line → zsh. No control-structure keyword at its head.
//!
//! The line is first scanned into simple commands joined by `;` `&` `&&`
//! `||` `|` `|&` (csh word rules: `(` and every operator character break a
//! word, quotes and backslashes are kept verbatim). Each simple command is
//! then rendered by [`render_cmd`], which special-cases the builtins whose
//! syntax or output differs from zsh and sends everything else word by word
//! through [`super::words::translate_word`].
//!
//! Conventions of the emitted zsh:
//!   * csh variables are always zsh arrays (`set x = a` → `x=(a)`) so that
//!     `$#x` / `$x[2]` mean the same on both sides;
//!   * csh aliases become zsh functions, registered in the `_csh_alias` /
//!     `_csh_alias_ls` associative arrays so `alias`, `unalias`, `which` and
//!     `where` can list them;
//!   * shell variables that tcsh mirrors into the environment or special
//!     parameters (`home` `user` `term` `cwd` `prompt` `prompt2` `prompt3`)
//!     are renamed to their zsh parameter on both read and write.
//!
//! # Errors that end a script
//!
//! tcsh stops a script on an error raised in the shell process itself and
//! only fails the command on an error raised in a forked child. Verified
//! against tcsh 6.21 (`/bin/tcsh -f script`):
//!
//! | error | builtin in the shell | external / pipeline stage |
//! |-------|----------------------|----------------------------|
//! | `x: Undefined variable.` | script ends, status 1 | script ends, status 1 |
//! | `f: File exists.` (noclobber), `f: No such file or directory.`, `f: Permission denied.`, `f: Is a directory.` on a redirect | script ends | message, status 1, script continues |
//! | `cmd: No match.` (every glob word empty) | script ends | message, status 1, script continues |
//! | `cmd: Command not found.` | n/a | message, status 1, script continues |
//! | `Unknown user: u.` for `~u` | script ends | message, status 1, script continues |
//! | `EVENT: Event not found.` for a `!` word | the whole line is dropped and the script ends | |
//!
//! "Script ends" is emitted as [`ABORT`]: `exit 1` in a script, only the
//! failed command in an interactive shell. The conditions are checked by
//! self-contained zsh text wrapped around the command (see [`guard_command`]).

use super::expr::translate_at;
use super::lex::split_words;
use super::words::{translate_word, translate_word_globbed};

const NULL_COMMAND: &str = "Invalid null command.";
const MISSING_NAME: &str = "Missing name for redirect.";
const AMBIGUOUS_OUT: &str = "Ambiguous output redirect.";
const AMBIGUOUS_IN: &str = "Ambiguous input redirect.";
const BADLY_PLACED: &str = "Badly placed ()'s.";

/// Ends the script after an error tcsh treats as fatal. An interactive
/// shell only abandons the failed command.
const ABORT: &str = "{ [[ -o interactive ]] || exit 1; false; }";

/// [`ABORT`] for the body of a zsh function.
const ABORT_RET: &str = "{ [[ -o interactive ]] || exit 1; return 1; }";

/// Print `msg` on stderr and end the script (see [`ABORT`]).
fn fatal(msg: &str) -> String {
    format!("{{ print -u2 -r -- {}; {ABORT}; }}", sq(msg))
}

/// The builtins of tcsh 6.21 (`builtins` output). They run inside the shell
/// process, so their errors end a script; every other command is forked.
const TCSH_BUILTINS: &[&str] = &[
    ":", "@", "alias", "alloc", "bg", "bindkey", "break", "breaksw", "builtins", "bye", "case",
    "cd", "chdir", "complete", "continue", "default", "dirs", "echo", "echotc", "else", "end",
    "endif", "endsw", "eval", "exec", "exit", "fg", "filetest", "foreach", "glob", "goto",
    "hashstat", "history", "hup", "if", "jobs", "kill", "limit", "login", "logout", "ls-F", "nice",
    "nohup", "notify", "onintr", "popd", "printenv", "pushd", "rehash", "repeat", "sched", "set",
    "setenv", "settc", "setty", "shift", "source", "stop", "suspend", "switch", "telltc",
    "termname", "time", "umask", "unalias", "uncomplete", "unhash", "unlimit", "unset",
    "unsetenv", "wait", "watchlog", "where", "which", "while",
];

/// Translate a command line that may hold lists (`;` `&&` `||`), pipes
/// (`|` `|&`), background `&`, redirections (`>&` `>!` `>>&` `>>!` `<<`),
/// parenthesised subshells, and the builtins whose syntax differs from zsh
/// (`set` `unset` `setenv` `unsetenv` `alias` `unalias` `shift` `exit`
/// `source` `@` `limit` `unlimit` `jobs` `kill` `wait` `nice` `hup` `glob`
/// `dirs` `pushd` `popd` `which` `where` …).
/// Words go through [`super::words::translate_word`].
///
/// Commands are wrapped in the checks that reproduce tcsh's fatal errors
/// (see the module docs), so a line is usually `if CHECKS; then CMD; else
/// FAIL; fi`. A few translations depend on what earlier lines of the same
/// script set (`set -r`, `set echo_style`); a script is translated in order.
///
/// An empty line yields an empty string. Errors carry tcsh's message text
/// (`Invalid null command.`, `Ambiguous output redirect.`, …).
pub fn translate_line(line: &str) -> Result<String, String> {
    if let Some(event) = history_event(line) {
        return Ok(fatal(&format!("{event}: Event not found.")));
    }
    translate_cmds(line)
}

/// [`translate_line`] without history-substitution checking (alias bodies
/// carry `!` text that is only a history reference at invocation time).
fn translate_cmds(line: &str) -> Result<String, String> {
    let cmds = parse(line)?;
    let mut segments: Vec<(String, Sep)> = Vec::new();
    let mut pipes: Vec<String> = Vec::new();
    let mut ops: Vec<Sep> = Vec::new();
    let mut pipeline: Vec<Simple> = Vec::new();
    let mut conns: Vec<Sep> = Vec::new();
    let mut prev: Option<Sep> = None;

    for (cmd, sep) in cmds {
        if cmd.is_empty() {
            // csh drops empty commands around `;` and `&` and a leading `&&`;
            // an empty operand of `|` `||` or a trailing `&&` is an error.
            let bad_after = matches!(sep, Sep::Pipe | Sep::PipeErr | Sep::Or);
            let bad_before = matches!(prev, Some(Sep::Pipe | Sep::PipeErr | Sep::And | Sep::Or));
            if !cmd.ins.is_empty() || !cmd.outs.is_empty() || bad_after || bad_before {
                return Err(NULL_COMMAND.to_string());
            }
            prev = Some(sep);
            continue;
        }
        pipeline.push(cmd);
        prev = Some(sep);
        if matches!(sep, Sep::Pipe | Sep::PipeErr) {
            conns.push(sep);
            continue;
        }
        pipes.push(render_pipeline(&pipeline, &conns, sep == Sep::Bg)?);
        pipeline.clear();
        conns.clear();
        match sep {
            Sep::And | Sep::Or => ops.push(sep),
            _ => {
                segments.push((render_and_or(&pipes, &ops), sep));
                pipes.clear();
                ops.clear();
            }
        }
    }

    Ok(group_background(&segments))
}

/// What tcsh prints on stdout after starting a background job, even in a
/// script: `[job] pid`.
const JOB_NOTICE: &str = "print -r -- \"[${#jobstates}] $!\";";

/// Join `;`-separated segments. In csh `&` ends a whole `;` list: `a; b &`
/// backgrounds both (job text `( a; b )`), unlike zsh where only `b` is
/// backgrounded, so a multi-item list before `&` becomes `( a; b ) &`.
fn group_background(segments: &[(String, Sep)]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut group: Vec<&str> = Vec::new();
    for (text, sep) in segments {
        group.push(text);
        match sep {
            Sep::Bg if group.len() > 1 => {
                parts.push(format!("( {} ) &", group.join("; ")));
                parts.push(JOB_NOTICE.to_string());
                group.clear();
            }
            Sep::Bg => {
                parts.push(format!("{} &", group[0]));
                parts.push(JOB_NOTICE.to_string());
                group.clear();
            }
            _ => {}
        }
    }
    if !group.is_empty() {
        parts.push(group.join("; "));
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// Scanning
// ---------------------------------------------------------------------------

/// What follows a simple command.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Sep {
    Semi,
    And,
    Or,
    Pipe,
    PipeErr,
    Bg,
    End,
}

/// A redirection; for `Out` the flags spell the csh operator
/// (`>` `>>` with optional `&` and `!`).
#[derive(Clone, Debug)]
enum Redir {
    In(String),
    Here(String),
    Out { target: String, append: bool, err: bool, force: bool },
}

/// One simple command: a `( subshell )` or a word list, plus redirections.
#[derive(Default, Debug)]
struct Simple {
    sub: Option<String>,
    words: Vec<String>,
    ins: Vec<Redir>,
    outs: Vec<Redir>,
}

impl Simple {
    fn is_empty(&self) -> bool {
        self.sub.is_none() && self.words.is_empty()
    }

    fn at_command_start(&self) -> bool {
        self.is_empty() && self.ins.is_empty() && self.outs.is_empty()
    }
}

#[derive(Default)]
struct Parser {
    cmds: Vec<(Simple, Sep)>,
    cur: Simple,
    word: String,
    pending: Option<Redir>,
}

impl Parser {
    /// Hand the word being built to the current command (or to the
    /// redirection waiting for its target).
    fn flush_word(&mut self) -> Result<(), String> {
        if self.word.is_empty() {
            return Ok(());
        }
        let w = std::mem::take(&mut self.word);
        match self.pending.take() {
            Some(Redir::In(_)) => self.cur.ins.push(Redir::In(w)),
            Some(Redir::Here(_)) => self.cur.ins.push(Redir::Here(w)),
            Some(Redir::Out { append, err, force, .. }) => {
                self.cur.outs.push(Redir::Out { target: w, append, err, force })
            }
            None => {
                if self.cur.sub.is_some() {
                    return Err(BADLY_PLACED.to_string());
                }
                self.cur.words.push(w);
            }
        }
        Ok(())
    }

    fn start_redirect(&mut self, r: Redir) -> Result<(), String> {
        self.flush_word()?;
        if self.pending.is_some() {
            return Err(MISSING_NAME.to_string());
        }
        self.pending = Some(r);
        Ok(())
    }

    fn end_command(&mut self, sep: Sep) -> Result<(), String> {
        self.flush_word()?;
        if self.pending.is_some() {
            return Err(MISSING_NAME.to_string());
        }
        let cur = std::mem::take(&mut self.cur);
        self.cmds.push((cur, sep));
        Ok(())
    }
}

/// Index of the `)` matching the `(` at `start`, quotes and backslashes
/// honoured.
fn matching_paren(cs: &[char], start: usize) -> Result<usize, String> {
    let mut depth = 0usize;
    let mut quote: Option<char> = None;
    let mut i = start;
    while i < cs.len() {
        let c = cs[i];
        match quote {
            Some(q) => {
                if c == '\\' && q != '\'' {
                    i += 1;
                } else if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\\' => i += 1,
                '\'' | '"' | '`' => quote = Some(c),
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(i);
                    }
                }
                _ => {}
            },
        }
        i += 1;
    }
    Err("Too many ('s.".to_string())
}

fn parse(line: &str) -> Result<Vec<(Simple, Sep)>, String> {
    let cs: Vec<char> = line.chars().collect();
    let n = cs.len();
    let mut p = Parser::default();
    let mut i = 0;
    while i < n {
        let c = cs[i];
        let next = cs.get(i + 1).copied();
        match c {
            '\\' => {
                p.word.push(c);
                if let Some(d) = next {
                    p.word.push(d);
                }
                i += 2;
            }
            '\'' | '"' | '`' => {
                p.word.push(c);
                i += 1;
                loop {
                    let Some(&d) = cs.get(i) else {
                        return Err(format!("Unmatched '{c}'."));
                    };
                    p.word.push(d);
                    i += 1;
                    if d == c {
                        break;
                    }
                    // `\"` does not escape the quote inside "…" in csh
                    if d == '\\' && c == '"' && cs.get(i) == Some(&'"') {
                        continue;
                    }
                    if d == '\\' && c != '\'' {
                        // tcsh has no nested backquotes: an escaped one
                        // inside `…` leaves the first unmatched
                        if c == '`' && cs.get(i) == Some(&'`') {
                            return Err("Unmatched '`'.".to_string());
                        }
                        if let Some(&e) = cs.get(i) {
                            p.word.push(e);
                            i += 1;
                        }
                    }
                }
            }
            ':' if p.word.contains('$') => {
                p.word.push(c);
                i += 1;
                if let Some(end) = super::lex::subst_modifier_end(&cs, i) {
                    p.word.extend(&cs[i..=end]);
                    i = end + 1;
                }
            }
            ' ' | '\t' => {
                p.flush_word()?;
                i += 1;
            }
            ';' => {
                p.end_command(Sep::Semi)?;
                i += 1;
            }
            '&' if next == Some('&') => {
                p.end_command(Sep::And)?;
                i += 2;
            }
            '&' => {
                p.end_command(Sep::Bg)?;
                i += 1;
            }
            '|' if next == Some('|') => {
                p.end_command(Sep::Or)?;
                i += 2;
            }
            '|' if next == Some('&') => {
                p.end_command(Sep::PipeErr)?;
                i += 2;
            }
            '|' => {
                p.end_command(Sep::Pipe)?;
                i += 1;
            }
            '<' if next == Some('<') => {
                p.start_redirect(Redir::Here(String::new()))?;
                i += 2;
            }
            '<' => {
                p.start_redirect(Redir::In(String::new()))?;
                i += 1;
            }
            '>' => {
                let mut j = i + 1;
                let mut take = |ch: char| {
                    let hit = cs.get(j) == Some(&ch);
                    if hit {
                        j += 1;
                    }
                    hit
                };
                let append = take('>');
                let err = take('&');
                let force = take('!');
                p.start_redirect(Redir::Out { target: String::new(), append, err, force })?;
                i = j;
            }
            '(' => {
                p.flush_word()?;
                let close = matching_paren(&cs, i)?;
                let inner: String = cs[i + 1..close].iter().collect();
                if p.pending.is_none() && p.cur.at_command_start() {
                    p.cur.sub = Some(inner);
                } else {
                    p.word = format!("({inner})");
                    p.flush_word()?;
                }
                i = close + 1;
            }
            ')' => return Err("Too many )'s.".to_string()),
            _ => {
                p.word.push(c);
                i += 1;
            }
        }
    }
    p.end_command(Sep::End)?;
    Ok(p.cmds)
}

// ---------------------------------------------------------------------------
// Pipelines, lists, redirections
// ---------------------------------------------------------------------------

/// Check redirections against pipeline position (csh rejects an output
/// redirect on a non-final stage and an input redirect on a non-first one)
/// and render the stages joined by `|` / `|&`.
/// `background` marks a pipeline that ends in `&`: its stages run in forked
/// children, where an expression error in `@` only fails that stage.
fn render_pipeline(stages: &[Simple], conns: &[Sep], background: bool) -> Result<String, String> {
    let last = stages.len() - 1;
    for (k, s) in stages.iter().enumerate() {
        if s.outs.len() > 1 || (!s.outs.is_empty() && k < last) {
            return Err(AMBIGUOUS_OUT.to_string());
        }
        if s.ins.len() > 1 || (!s.ins.is_empty() && k > 0) {
            return Err(AMBIGUOUS_IN.to_string());
        }
    }
    let mut out = String::new();
    for (k, s) in stages.iter().enumerate() {
        if k > 0 {
            out.push_str(if conns[k - 1] == Sep::PipeErr { " |& " } else { " | " });
        }
        let forked = stages.len() > 1 || background;
        match render_simple(s, stages.len() > 1) {
            Ok(text) => out.push_str(&text),
            // tcsh runs `@ i |= 8` as the pipe `@ i | = 8`: the error comes
            // from a child and the script goes on
            Err(msg) if forked && s.words.first().is_some_and(|w| w == "@") => {
                out.push_str(&format!("{{ print -u2 -r -- {}; false; }}", sq(&msg)));
            }
            Err(msg) => return Err(msg),
        }
    }
    Ok(out)
}

/// Join pipelines with `&&` / `||` using csh precedence: `&&` binds tighter
/// than `||` (`true || a && b` runs nothing), zsh is left-associative, so an
/// `&&` chain that follows a `||` is wrapped in `{ …; }`.
fn render_and_or(pipes: &[String], ops: &[Sep]) -> String {
    let mut operands: Vec<Vec<&str>> = vec![vec![pipes[0].as_str()]];
    for (op, p) in ops.iter().zip(&pipes[1..]) {
        if *op == Sep::Or {
            operands.push(vec![p.as_str()]);
        } else {
            operands.last_mut().expect("one operand").push(p.as_str());
        }
    }
    let rendered: Vec<String> = operands
        .iter()
        .enumerate()
        .map(|(i, chain)| {
            let joined = chain.join(" && ");
            if i > 0 && chain.len() > 1 {
                format!("{{ {joined}; }}")
            } else {
                joined
            }
        })
        .collect();
    rendered.join(" || ")
}

fn render_simple(s: &Simple, in_pipe: bool) -> Result<String, String> {
    let mut text = match &s.sub {
        Some(inner) => {
            if inner.trim().is_empty() {
                return Err(NULL_COMMAND.to_string());
            }
            format!("( {} )", translate_cmds(inner)?)
        }
        None => render_cmd(&s.words)?,
    };
    for r in s.ins.iter().chain(&s.outs) {
        text.push(' ');
        text.push_str(&render_redirect(r));
    }
    Ok(guard_command(s, text, in_pipe))
}

/// One redirection in zsh syntax. `>&` splits into `> f 2>&1`; `>!` is zsh's
/// `>|`. A quoted here-document delimiter is the terminator *with* its
/// quotes in csh (verified against tcsh), so it is re-quoted whole.
fn render_redirect(r: &Redir) -> String {
    match r {
        Redir::In(t) => format!("< {}", tw(t)),
        Redir::Here(d) => {
            if d.contains(['\'', '"', '\\']) {
                format!("<<{}", sq(d))
            } else {
                format!("<<{d}")
            }
        }
        Redir::Out { target, append, err, force } => {
            let op = match (append, force) {
                (false, false) => ">",
                (false, true) => ">|",
                (true, false) => ">>",
                (true, true) => ">>|",
            };
            let dup = if *err { " 2>&1" } else { "" };
            format!("{op} {}{dup}", tw(target))
        }
    }
}

// ---------------------------------------------------------------------------
// Error guards
// ---------------------------------------------------------------------------

/// Shell variables tcsh defines at startup (or whose zsh counterpart is
/// always set); a reference to one is never an `Undefined variable.` error.
const ALWAYS_DEFINED: &[&str] = &[
    "_", "addsuffix", "argv", "autologout", "command", "cwd", "dirstack", "echo_style", "edit",
    "euid", "gid", "group", "histchars", "history", "home", "host", "HOST", "hostname", "HOSTTYPE",
    "loginsh", "MACHTYPE", "OSTYPE", "owd", "path", "prompt", "prompt2", "prompt3", "savehist",
    "shell", "shlvl", "status", "term", "tcsh", "tty", "uid", "user", "VENDOR", "version",
    // environment tcsh exports itself at startup
    "GROUP", "HOME", "LOGNAME", "PATH", "PWD", "SHELL", "SHLVL", "TERM", "USER",
];

/// The event text of the first history reference (`!x`, `a!b`, `!!`, …) in
/// `line`, which tcsh rejects with `EVENT: Event not found.` because a
/// script has no history. `\!` is literal; `!` before a blank, `=`, `~`,
/// `(`, a closing bracket or an operator is literal too (verified against
/// tcsh). Single quotes do not protect a `!`.
fn history_event(line: &str) -> Option<String> {
    let cs: Vec<char> = line.chars().collect();
    let mut i = 0;
    while i < cs.len() {
        match cs[i] {
            '\\' => i += 1,
            '!' => {
                let rest = &cs[i + 1..];
                let Some(&next) = rest.first() else { break };
                match next {
                    ' ' | '\t' | '=' | '~' | '(' | ')' | '}' | ';' | '&' | '|' | '<' | '>' | '\'' | '"'
                    | '`' | '#' => {}
                    ':' | '*' | '^' | '$' | '%' | '!' => return Some("0".to_string()),
                    '-' if rest.get(1).is_some_and(|c| c.is_ascii_digit()) => return Some("0".to_string()),
                    '?' | '{' => {
                        let close = if next == '?' { '?' } else { '}' };
                        let text: String = rest[1..].iter().take_while(|&&c| c != close).collect();
                        return Some(text);
                    }
                    _ => {
                        let stop = [' ', '\t', ':', ';', '&', '|', '<', '>', '(', ')', '\'', '"', '`'];
                        let text: String = rest.iter().take_while(|c| !stop.contains(c)).collect();
                        return Some(text);
                    }
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Names of the variables a word dereferences (`$x`, `${x}`, `$#x`, `$x[1]`)
/// outside single quotes and backslash escapes; `$?x`, `$1`, `$$`, `$<` and
/// the always-defined variables are not listed.
pub(crate) fn variable_refs(word: &str) -> Vec<String> {
    let cs: Vec<char> = word.chars().collect();
    let mut names: Vec<String> = Vec::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (None, '\'' | '"') => quote = Some(c),
            (Some('\''), _) => {}
            (_, '\\') => i += 1,
            (_, '$') => {
                let braced = cs.get(i) == Some(&'{');
                if braced {
                    i += 1;
                }
                if cs.get(i) == Some(&'#') {
                    i += 1;
                }
                let start = i;
                while i < cs.len() && (cs[i].is_ascii_alphanumeric() || cs[i] == '_') {
                    i += 1;
                }
                let name: String = cs[start..i].iter().collect();
                let ident = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
                if ident && !ALWAYS_DEFINED.contains(&name.as_str()) && !names.contains(&name) {
                    names.push(name);
                }
            }
            _ => {}
        }
    }
    names
}

/// True when `word` holds a pattern tcsh expands: `*`, `?` or a `[…]` class
/// outside quotes, backslash escapes and `$…` references (`$x[2]` is a
/// subscript, not a class).
pub(crate) fn is_glob_word(word: &str) -> bool {
    let cs: Vec<char> = word.chars().collect();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        i += 1;
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '\\') => i += 1,
            (None, '$') => {
                if cs.get(i) == Some(&'{') {
                    while i < cs.len() && cs[i] != '}' {
                        i += 1;
                    }
                    i += 1;
                } else if cs.get(i) == Some(&'*') {
                    i += 1; // `$*` is a variable, not a pattern
                } else {
                    while i < cs.len() && (cs[i].is_ascii_alphanumeric() || matches!(cs[i], '_' | '?' | '#')) {
                        i += 1;
                    }
                }
                if cs.get(i) == Some(&'[') {
                    while i < cs.len() && cs[i] != ']' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            (None, '*' | '?') => return true,
            (None, '[') if cs[i..].iter().position(|&x| x == ']').is_some_and(|n| n > 0) => return true,
            _ => {}
        }
    }
    false
}

/// Probe text for an input redirection: prints tcsh's message and fails when
/// `target` cannot be read.
fn input_probe(target: &str) -> String {
    format!(
        "() {{ if [[ ! -e $1 ]]; then print -u2 -r -- \"$1: No such file or directory.\"; \
elif [[ ! -r $1 ]]; then print -u2 -r -- \"$1: Permission denied.\"; else return 0; fi; return 1; }} {target}"
    )
}

/// Probe text for an output redirection, including tcsh's `noclobber`
/// rules: `>` onto an existing file (not a character device) is
/// `File exists.`, `>>` onto a missing file is `No such file or directory.`;
/// `>!` and `>>!` ignore `noclobber`.
fn output_probe(target: &str, append: bool, force: bool) -> String {
    let mut conds: Vec<(&str, &str)> = Vec::new();
    if !force && !append {
        conds.push(("-o noclobber && -e $1 && ! -c $1", "File exists."));
    }
    if !force && append {
        conds.push(("-o noclobber && ! -e $1", "No such file or directory."));
    }
    conds.push(("-d $1", "Is a directory."));
    conds.push(("-e $1 && ! -w $1", "Permission denied."));
    conds.push(("! -e $1 && ! -d ${1:h}", "No such file or directory."));
    conds.push(("! -e $1 && ! -w ${1:h}", "Permission denied."));
    let chain: String = conds
        .iter()
        .enumerate()
        .map(|(k, (cond, msg))| {
            format!("{} [[ {cond} ]]; then _m={}; ", if k == 0 { "if" } else { "elif" }, sq(msg))
        })
        .collect();
    format!(
        "() {{ local _m; {chain}else return 0; fi; print -u2 -r -- \"$1: $_m\"; return 1; }} {target}"
    )
}

/// Guard for a command word containing `/`: tcsh reports
/// `p: Command not found.` for a missing path and `p: Permission denied.`
/// for a directory or a file without the execute bit (zsh says
/// `no such file or directory` with status 127).
fn path_command_probe(word: &str) -> String {
    format!(
        "() {{ if [[ ! -e $1 ]]; then print -u2 -r -- \"$1: Command not found.\"; \
elif [[ -d $1 || ! -x $1 ]]; then print -u2 -r -- \"$1: Permission denied.\"; else return 0; fi; return 1; }} {word}"
    )
}

/// `~name` with an unknown user is `Unknown user: name.`; zsh fails the
/// expansion of the word, which a silenced `:` observes.
fn tilde_probe(name: &str) -> String {
    format!("( : ~{name} ) 2>/dev/null || {{ print -u2 -r -- 'Unknown user: {name}.'; false; }}")
}

/// `${+x}` check for one variable reference.
fn defined_probe(name: &str) -> String {
    // braced so that several probes can be joined with `&&`
    format!("{{ (( ${{+{name}}} )) || {{ print -u2 -r -- '{name}: Undefined variable.'; false; }}; }}")
}

/// The user name of an unquoted `~name` word, if it is one.
fn tilde_user(word: &str) -> Option<&str> {
    let name = word.strip_prefix('~')?;
    let end = name.find('/').unwrap_or(name.len());
    let name = &name[..end];
    let ok = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    ok.then_some(name)
}

/// Heads whose arguments are patterns or expressions, not filename words:
/// no `No match.` check applies.
fn skips_glob_check(head: &str) -> bool {
    matches!(head, "unset" | "unsetenv" | "unalias" | "set" | "setenv" | "@" | "eval" | "repeat" | "if")
}

/// Wrap `text` (one rendered simple command, redirections included) in the
/// checks tcsh performs before running it, so that the failures print tcsh's
/// message and end or continue the script exactly as tcsh does (module
/// docs). `in_pipe` marks a pipeline stage, which tcsh forks.
fn guard_command(s: &Simple, text: String, in_pipe: bool) -> String {
    let head = s.words.first().map(String::as_str);
    let in_shell = !in_pipe && head.is_some_and(|h| TCSH_BUILTINS.contains(&h));

    // Failures that end the script wherever the command runs.
    let mut always: Vec<String> = Vec::new();
    if head != Some("eval") {
        for w in &s.words {
            for name in variable_refs(w) {
                let probe = defined_probe(&name);
                if !always.contains(&probe) {
                    always.push(probe);
                }
            }
            for probe in super::words::subscript_probes(w) {
                if !always.contains(&probe) {
                    always.push(probe);
                }
            }
        }
    }

    // Failures that end the script only for a builtin.
    let mut local: Vec<String> = Vec::new();
    let skip_dev = |t: &str| t.starts_with("/dev/") || is_glob_word(t) || t.contains('`');
    for r in &s.ins {
        if let Redir::In(t) = r {
            if !skip_dev(t) {
                local.push(input_probe(&tw(t)));
            }
        }
    }
    for r in &s.outs {
        if let Redir::Out { target, append, force, .. } = r {
            if !skip_dev(target) {
                local.push(output_probe(&tw(target), *append, *force));
            }
        }
    }
    if let Some(h) = head {
        let plain = !h.contains(['$', '`', '*', '?', '[', '\'', '"', '\\']);
        if h.contains('/') && plain && !TCSH_BUILTINS.contains(&h) {
            local.push(path_command_probe(&tw(h)));
        }
        for w in &s.words[1..] {
            if let Some(name) = tilde_user(w) {
                local.push(tilde_probe(name));
            }
        }
        let globs: Vec<String> = s.words[1..].iter().filter(|w| is_glob_word(w)).map(|w| tw(w)).collect();
        if plain && !globs.is_empty() && !skips_glob_check(h) {
            local.push(format!(
                "{{ [[ ! -o cshnullglob ]] || () {{ setopt localoptions nullglob; local -a _g; _g=({}); (( $#_g )); }} \"$@\" || \
{{ print -u2 -r -- {}; false; }}; }}",
                globs.join(" "),
                sq(&format!("{h}: No match."))
            ));
        }
    }

    if local.is_empty() && always.is_empty() {
        return text;
    }
    // The checks overwrite `$?`; a command that reads it (`$status`) gets
    // the value from before them back.
    let keeps_status = text.contains("$?");
    let mut out = if keeps_status { format!("() {{ return $1; }} $_csh_s; {text}") } else { text };
    if !local.is_empty() {
        let fail = if in_shell { ABORT } else { "false" };
        out = format!("if {}; then {out}; else {fail}; fi", local.join(" && "));
    }
    if !always.is_empty() {
        out = format!("if {}; then {out}; else {ABORT}; fi", always.join(" && "));
    }
    if keeps_status {
        out = out.replacen("if ", "if _csh_s=$? && ", 1);
    }
    out
}

// ---------------------------------------------------------------------------
// Word helpers
// ---------------------------------------------------------------------------

/// Single-quote `s` for zsh.
fn sq(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// csh variable whose value tcsh mirrors into a zsh special parameter.
fn mirrored_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "cwd" => "PWD",
        "home" => "HOME",
        "user" => "USER",
        "term" => "TERM",
        "prompt" => "PROMPT",
        "prompt2" => "PROMPT2",
        "prompt3" => "PROMPT3",
        _ => return None,
    })
}

/// Rewrite `$cwd` `${home}` `$?user` … to the zsh parameter, outside single
/// quotes.
fn map_special_vars(word: &str) -> String {
    let cs: Vec<char> = word.chars().collect();
    let mut out = String::with_capacity(word.len());
    let mut in_sq = false;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        out.push(c);
        i += 1;
        match c {
            '\'' => in_sq = !in_sq,
            '\\' if !in_sq => {
                if let Some(&d) = cs.get(i) {
                    out.push(d);
                    i += 1;
                }
            }
            '$' if !in_sq => {
                let mut is_set = false;
                for pre in ['{', '?', '#'] {
                    if cs.get(i) == Some(&pre) {
                        is_set |= pre == '?';
                        out.push(pre);
                        i += 1;
                    }
                }
                let start = i;
                while i < cs.len() && (cs[i].is_ascii_alphanumeric() || cs[i] == '_') {
                    i += 1;
                }
                let name: String = cs[start..i].iter().collect();
                match mirrored_name(&name) {
                    // `$?prompt` asks whether the shell is interactive (a
                    // script has no prompt), which the word translator
                    // answers; PROMPT itself always has a value.
                    _ if is_set && name == "prompt" => out.push_str(&name),
                    Some(z) => out.push_str(z),
                    None => out.push_str(&name),
                }
            }
            _ => {}
        }
    }
    out
}

/// csh word → zsh word, with the mirrored variable names applied first and
/// an unclosed `[` made literal (csh keeps `echo [a` as `[a`; zsh calls it a
/// bad pattern).
fn tw(word: &str) -> String {
    translate_word_globbed(&escape_unclosed_brackets(&map_special_vars(word)))
}

/// Backslash every unquoted `[` that has no `]` after it, outside `$x[…]`
/// subscripts.
fn escape_unclosed_brackets(word: &str) -> String {
    if !word.contains('[') {
        return word.to_string();
    }
    let cs: Vec<char> = word.chars().collect();
    let mut out = String::with_capacity(word.len() + 2);
    let mut quote: Option<char> = None;
    let mut after_var = false;
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        match (quote, c) {
            (Some(q), _) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '\'' | '"' | '`') => quote = Some(c),
            (None, '\\') => {
                out.push(c);
                if let Some(&d) = cs.get(i + 1) {
                    out.push(d);
                }
                i += 2;
                after_var = false;
                continue;
            }
            (None, '[') if !after_var && !cs[i + 1..].contains(&']') => {
                out.push('\\');
            }
            _ => {}
        }
        after_var = quote.is_none() && (c == '}' || is_var_name_tail(&cs[..=i]));
        out.push(c);
        i += 1;
    }
    out
}

/// True when `prefix` ends in `$name` (a variable reference whose `[` starts
/// a subscript).
fn is_var_name_tail(prefix: &[char]) -> bool {
    let end = prefix.len();
    let start = prefix
        .iter()
        .rposition(|c| !(c.is_ascii_alphanumeric() || *c == '_'))
        .map_or(0, |p| p + 1);
    start < end && start > 0 && prefix[start - 1] == '$'
}

/// csh quote removal: `'…'` literal, `"…"` contents, `\c` → `c`.
fn dequote(word: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (Some('\''), _) => out.push(c),
            (_, '\\') => {
                if let Some(d) = chars.next() {
                    out.push(d);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// True when `word` holds `*`, `?` or `[` outside quotes and escapes.
fn has_glob(word: &str) -> bool {
    scan_unquoted(word, |c| matches!(c, '*' | '?' | '['))
}

/// True when `word` would be expanded by the shell on first parse: an
/// unquoted-by-single-quote `$` or backquote.
fn has_expansion(word: &str) -> bool {
    let mut in_sq = false;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => in_sq = !in_sq,
            '\\' if !in_sq => {
                chars.next();
            }
            '$' | '`' if !in_sq => return true,
            _ => {}
        }
    }
    false
}

/// Run `pred` over the characters of `word` that are outside any quotes and
/// not backslash-escaped; true when it matches one.
fn scan_unquoted(word: &str, pred: impl Fn(char) -> bool) -> bool {
    let mut quote: Option<char> = None;
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\\' => {
                    chars.next();
                }
                '\'' | '"' | '`' => quote = Some(c),
                _ if pred(c) => return true,
                _ => {}
            },
        }
    }
    false
}

/// Join zsh statements into one command: a lone statement as is, several
/// wrapped in `{ …; }` so they bind as a unit to pipes and redirections.
fn seq(stmts: Vec<String>) -> String {
    match stmts.len() {
        1 => stmts.into_iter().next().expect("one statement"),
        _ => format!("{{ {}; }}", stmts.join("; ")),
    }
}

fn is_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Strip one `( … )` group: the text inside, trimmed.
fn paren_inner(word: &str) -> Option<&str> {
    word.strip_prefix('(')?.strip_suffix(')').map(str::trim)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Render one simple command (head word plus arguments, no redirections).
fn render_cmd(words: &[String]) -> Result<String, String> {
    let head = words[0].as_str();
    let args = &words[1..];
    let allows_parens = matches!(head, "set" | "@" | "exit");
    if !allows_parens && words.iter().any(|w| w.starts_with('(')) {
        return Err(BADLY_PLACED.to_string());
    }
    // a word with an alias is that alias's function, whatever builtin it names
    if !matches!(head, "alias" | "unalias") && STATE.with(|st| st.borrow().aliases.contains(head)) {
        return Ok(generic(head, args));
    }
    match head {
        "set" => cmd_set(args),
        "unset" => cmd_unset(args),
        "setenv" => cmd_setenv(args),
        "unsetenv" => cmd_unsetenv(args),
        "alias" => cmd_alias(args),
        "unalias" => cmd_unalias(args),
        "shift" => cmd_shift(args),
        "exit" => cmd_exit(args),
        "source" => cmd_source(args),
        "@" => translate_at(&args.join(" ")),
        "eval" => cmd_eval(args),
        "repeat" => cmd_repeat(args),
        "nohup" if args.is_empty() => Ok("trap '' HUP".to_string()),
        "which" | "where" if args.is_empty() => Err(format!("{head}: Too few arguments.")),
        "which" => Ok(cmd_which(args)),
        "where" => Ok(cmd_where(args)),
        "umask" => cmd_umask(args),
        "history" => Ok(cmd_history(args)),
        "dirs" => Ok(cmd_dirs(args)),
        "pushd" => Ok(cmd_pushd(args)),
        "popd" => Ok(cmd_popd(args)),
        "echo" => Ok(cmd_echo(args)),
        // programmable completion and terminal capabilities have no effect
        // in a script, but an argument-less `uncomplete`/`settc`/`filetest`
        // is tcsh's usage error
        "complete" => Ok(":".to_string()),
        "uncomplete" | "settc" | "filetest" if args.is_empty() => {
            Ok(fatal(&format!("{head}: Too few arguments.")))
        }
        "uncomplete" | "settc" => Ok(":".to_string()),
        "filetest" => Ok(cmd_filetest(args)),
        "hashstat" => Ok(cmd_hashstat()),
        "unhash" => {
            STATE.with(|st| st.borrow_mut().hashed = false);
            Ok(":".to_string())
        }
        "rehash" => {
            STATE.with(|st| st.borrow_mut().hashed = true);
            Ok(generic("rehash", args))
        }
        "chdir" | "cd" if args.len() > 1 => Ok(fatal(&format!("{head}: Too many arguments."))),
        "chdir" => Ok(generic("cd", args)),
        "jobs" => Ok(cmd_jobs(args)),
        "fg" | "bg" => Ok(cmd_fg_bg(head, args)),
        "kill" => Ok(cmd_kill(args)),
        "wait" => Ok(WAIT.to_string()),
        "time" => cmd_time(args),
        "nice" => cmd_nice(args),
        "hup" => cmd_hup(args),
        "glob" => Ok(generic("() { print -rn -- \"${(pj:\\0:)@}\"; }", args)),
        "limit" => cmd_limit(args),
        "unlimit" => cmd_unlimit(args),
        "termname" => Ok("print -r -- \"$TERM\"".to_string()),
        "stop" if args.is_empty() => Ok(fatal("stop: Too few arguments.")),
        "stop" => Ok(format!("{{ [[ -o monitor ]] && {}; true; }}", generic("kill -STOP", args))),
        "suspend" => Ok("{ [[ -o monitor ]] && suspend; true; }".to_string()),
        "notify" if args.is_empty() => {
            Ok(format!("if (( ${{#jobstates}} )); then :; else {}; fi", fatal("notify: No current job.")))
        }
        "notify" => Ok(":".to_string()),
        "builtins" => Ok(format!("print -rl -- {}", TCSH_BUILTINS.iter().map(|b| sq(b)).collect::<Vec<_>>().join(" "))),
        "watchlog" => Ok(generic("log", args)),
        "newgrp" => Ok(generic("exec newgrp", args)),
        "ls-F" => Ok(generic(LS_F, args)),
        "printenv" => cmd_printenv(args),
        "login" | "logout" => Ok(fatal("Not a login shell.")),
        "bye" => Ok("exit 0".to_string()),
        _ => Ok(generic(&tw(head), args)),
    }
}

fn generic(head: &str, args: &[String]) -> String {
    let mut parts = vec![head.to_string()];
    parts.extend(args.iter().map(|w| tw(w)));
    parts.join(" ")
}

// --- set / unset / setenv / unsetenv ---------------------------------------

/// csh option-style variables and the zsh option they drive
/// (`(option, inverted)`: `nonomatch` is the inverse of zsh `nomatch`).
fn option_var(name: &str) -> Option<(&'static str, bool)> {
    Some(match name {
        "noclobber" => ("noclobber", false),
        "ignoreeof" => ("ignoreeof", false),
        "noglob" => ("noglob", false),
        "notify" => ("notify", false),
        "nonomatch" => ("nomatch cshnullglob", true),
        _ => return None,
    })
}

/// Right-hand side of one `set` assignment.
enum Val {
    /// `set x` — a single empty word.
    Empty,
    Word(String),
    /// `(a b c)`, the text between the parentheses.
    List(String),
}

const SET_LISTING: &str = "() { local _n; for _n in ${(ok)parameters}; do case ${parameters[$_n]} in \
array*) print -r -- \"$_n\"$'\\t'\"(${(P)_n})\";; \
scalar*|integer*|float*) print -r -- \"$_n\"$'\\t'\"${(P)_n}\";; esac; done; }";

/// `set` with csh's argument grammar (verified against tcsh): a sequence of
/// `name`, `name = word`, `name=word`, `name = (list)`, `name[i] = word`.
fn cmd_set(args: &[String]) -> Result<String, String> {
    let mut args = args;
    let mut readonly = false;
    while let Some(f) = args.first() {
        let is_flag = f.len() > 1 && f.starts_with('-') && f[1..].chars().all(|c| "rfl".contains(c));
        if !is_flag {
            break;
        }
        readonly |= f.contains('r');
        args = &args[1..];
    }
    if args.is_empty() {
        return Ok(SET_LISTING.to_string());
    }

    let mut stmts = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let w = args[i].as_str();
        i += 1;
        let (name, sub, rest) = split_set_name(w)?;
        if name == "path" {
            STATE.with(|st| st.borrow_mut().hashed = true);
        }
        let val = set_value(args, &mut i, rest);
        // `set status = n` sets the exit status the next `$status` reads
        if name == "status" && sub.is_none() {
            let n = match &val {
                Val::Word(w) => tw(w),
                _ => "0".to_string(),
            };
            stmts.push(format!("() {{ return {n}; }}"));
            continue;
        }
        // a nested group is where tcsh expects the next variable name
        if matches!(&val, Val::List(inner) if inner.contains('(')) {
            return Err("set: Variable name must begin with a letter.".to_string());
        }
        if name == "echo_style" && sub.is_none() {
            if let Val::Word(v) = &val {
                record_echo_style(&dequote(v));
            }
        }
        let assigned = seq(set_statements(name, sub, val, readonly)?);
        let known_readonly = STATE.with(|st| st.borrow().readonly.contains(name));
        if readonly {
            STATE.with(|st| st.borrow_mut().readonly.insert(name.to_string()));
        }
        stmts.push(if known_readonly { guard_readonly("set", name, assigned) } else { assigned });
    }
    Ok(seq(stmts))
}

/// Split `name[sub]rest` and validate the name as tcsh does.
fn split_set_name(w: &str) -> Result<(&str, Option<&str>, &str), String> {
    let starts_ok = w.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !starts_ok {
        return Err("set: Variable name must begin with a letter.".to_string());
    }
    let end = w
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(w.len());
    let (name, mut rest) = w.split_at(end);
    let mut sub = None;
    if let Some(body) = rest.strip_prefix('[') {
        let close = body.find(']').ok_or("set: Subscript error.")?;
        sub = Some(&body[..close]);
        rest = &body[close + 1..];
    }
    if !rest.is_empty() && !rest.starts_with('=') {
        return Err("set: Variable name must contain alphanumeric characters.".to_string());
    }
    Ok((name, sub, rest))
}

/// Value of one assignment, given the text after the name (`rest`) and the
/// words still to come. tcsh accepts `x`, `x = w`, `x=w`, `x = (l)`, `x=(l)`
/// and `x= (l)`; with `x=` or a trailing `x =` and no group the value is the
/// empty word, and the next word starts a new assignment.
fn set_value(args: &[String], i: &mut usize, rest: &str) -> Val {
    let group = |i: &mut usize| match args.get(*i) {
        Some(next) if next.starts_with('(') => {
            *i += 1;
            paren_inner(next).map(|l| Val::List(l.to_string()))
        }
        _ => None,
    };
    if let Some(glued) = rest.strip_prefix('=') {
        if !glued.is_empty() {
            return Val::Word(glued.to_string());
        }
        return group(i).unwrap_or(Val::Empty);
    }
    match args.get(*i).map(String::as_str) {
        Some("=") => {
            *i += 1;
            match args.get(*i) {
                Some(w) if w.starts_with('(') => group(i).unwrap_or(Val::Empty),
                Some(w) => {
                    *i += 1;
                    Val::Word(w.clone())
                }
                None => Val::Empty,
            }
        }
        Some(next) if next.starts_with("=(") => {
            *i += 1;
            paren_inner(&next[1..]).map_or(Val::Empty, |l| Val::List(l.to_string()))
        }
        _ => Val::Empty,
    }
}

fn set_statements(name: &str, sub: Option<&str>, val: Val, readonly: bool) -> Result<Vec<String>, String> {
    let elems: Vec<String> = match &val {
        Val::Empty => vec!["''".to_string()],
        Val::Word(w) => vec![tw(w)],
        Val::List(inner) => split_words(inner).iter().map(|w| tw(w)).collect(),
    };
    if let Some(s) = sub {
        // a subscript is a number, or a `$` reference that expands to one
        if !s.contains('$') && !s.chars().all(|c| c.is_ascii_digit() || c == '-' || c == '*') {
            return Err("set: Subscript error.".to_string());
        }
        if s.contains('-') && s.chars().all(|c| c.is_ascii_digit() || c == '-') {
            return Err("set: Subscript error.".to_string());
        }
        if matches!(val, Val::List(_)) {
            return Err("set: Syntax Error.".to_string());
        }
        // tcsh aborts on an unset variable or an index past the end.
        let idx = tw(s);
        return Ok(vec![format!(
            "{{ if (( ! ${{+{name}}} )); then print -u2 -r -- '{name}: Undefined variable.'; return 1; \
elif (( {idx} > ${{#{name}}} )); then print -u2 -r -- 'set: Subscript out of range.'; return 1; fi; \
{name}[{idx}]={}; }}",
            elems[0]
        )]);
    }
    let mut out = Vec::new();
    if let Some((opt, inverted)) = option_var(name) {
        out.push(format!("{} {opt}", if inverted { "unsetopt" } else { "setopt" }));
    }
    if let Some(z) = mirrored_name(name) {
        let first = elems.first().cloned().unwrap_or_else(|| "''".to_string());
        let export = if matches!(z, "HOME" | "USER" | "TERM") { "export " } else { "" };
        out.push(format!("{export}{z}={first}"));
        return Ok(out);
    }
    if name == "history" || name == "savehist" {
        let z = if name == "history" { "HISTSIZE" } else { "SAVEHIST" };
        let first = elems.first().cloned().unwrap_or_else(|| "''".to_string());
        out.push(format!("{z}={first}"));
        if name == "history" {
            return Ok(out);
        }
    }
    let ro = if readonly { "readonly " } else { "" };
    let list = if matches!(&val, Val::List(inner) if inner.trim().is_empty()) {
        String::new()
    } else {
        elems.join(" ")
    };
    out.push(format!("{ro}{name}=({list})"));
    Ok(out)
}

/// `if` that refuses to change a variable declared `set -r`: tcsh reports
/// `CMD: $NAME is read-only.` and ends the script.
fn guard_readonly(cmd: &str, name: &str, action: String) -> String {
    format!(
        "if [[ ${{parameters[{name}]-}} == *readonly* ]]; then {}; else {action}; fi",
        fatal(&format!("{cmd}: ${name} is read-only."))
    )
}

/// Remember a `set echo_style = …` for the `echo` lines that follow.
fn record_echo_style(value: &str) {
    let style = match value {
        "bsd" => EchoStyle::Bsd,
        "sysv" => EchoStyle::Sysv,
        "both" => EchoStyle::Both,
        "none" => EchoStyle::None,
        _ => return,
    };
    STATE.with(|st| st.borrow_mut().echo_style = style);
}

/// Unset every parameter whose name matches one of the shell patterns and
/// whose export flag equals `exported`. csh keeps shell variables and the
/// environment apart (`unset` touches the first, `unsetenv` the second);
/// zsh has one namespace, so the export flag stands in for the split.
fn unset_matching(pats: &[String], exported: bool) -> String {
    format!(
        "() {{ local _n _p; for _p; do for _n in ${{(k)parameters[(I)$_p]}}; do \
[[ ${{parameters[$_n]}} == *export* ]] {} unset -- $_n; done; done; }} {}",
        if exported { "&&" } else { "||" },
        pats.iter().map(|p| sq(p)).collect::<Vec<_>>().join(" ")
    )
}

fn cmd_unset(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("unset: Too few arguments.".to_string());
    }
    let mut stmts = Vec::new();
    let mut pats = Vec::new();
    for w in args {
        let name = dequote(w);
        if has_glob(w) {
            pats.push(name);
            continue;
        }
        if let Some((opt, inverted)) = option_var(&name) {
            stmts.push(format!("{} {opt}", if inverted { "setopt" } else { "unsetopt" }));
        }
        if name == "echo_style" {
            record_echo_style("bsd");
        }
        let known_readonly = STATE.with(|st| st.borrow().readonly.contains(&name));
        match mirrored_name(&name) {
            Some(z) => stmts.push(format!("unset {z}")),
            None if known_readonly => {
                let plain = format!("unset -- {name}");
                stmts.push(guard_readonly("unset", &name, plain));
            }
            None => pats.push(name),
        }
    }
    if !pats.is_empty() {
        stmts.push(unset_matching(&pats, false));
    }
    Ok(seq(stmts))
}

fn check_env_name(name: &str, cmd: &str) -> Result<(), String> {
    let first_ok = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    if !first_ok {
        return Err(format!("{cmd}: Variable name must begin with a letter."));
    }
    if !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(format!("{cmd}: Variable name must contain alphanumeric characters."));
    }
    Ok(())
}

/// `hashstat`: tcsh reports the hash table geometry once a command hash
/// exists, and nothing before.
fn cmd_hashstat() -> String {
    if STATE.with(|st| st.borrow().hashed) {
        "print -r -- '512 hash buckets of 8 bits each'".to_string()
    } else {
        ":".to_string()
    }
}

fn cmd_setenv(args: &[String]) -> Result<String, String> {
    if args.first().is_some_and(|n| n == "PATH") {
        STATE.with(|st| st.borrow_mut().hashed = true);
    }
    match args {
        [] => Ok("printenv".to_string()),
        [name] => {
            check_env_name(name, "setenv")?;
            Ok(format!("export {name}=''"))
        }
        [name, value] => {
            check_env_name(name, "setenv")?;
            if has_glob(value) {
                // csh expands an unquoted pattern and joins the matches with spaces.
                Ok(format!("() {{ export {name}=\"$*\"; }} {}", tw(value)))
            } else {
                Ok(format!("export {name}={}", tw(value)))
            }
        }
        _ => Err("setenv: Too many arguments.".to_string()),
    }
}

fn cmd_unsetenv(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("unsetenv: Too few arguments.".to_string());
    }
    let pats: Vec<String> = args.iter().map(|w| dequote(w)).collect();
    Ok(unset_matching(&pats, true))
}

// --- alias / unalias --------------------------------------------------------

/// Names that are zsh builtins and so need `builtin` (not `command`) when an
/// alias of the same name refers to itself.
const SELF_ALIAS_BUILTINS: &[&str] = &[
    "cd", "pwd", "pushd", "popd", "dirs", "kill", "jobs", "fg", "bg", "wait", "umask", "limit",
    "unlimit", "rehash", "source", "exec", "eval", "test", "time",
];

const ALIAS_LISTING: &str = "() { local _k; for _k in ${(ko)_csh_alias_ls}; do \
print -r -- \"$_k\"$'\\t'\"${_csh_alias_ls[$_k]}\"; done; }";

fn cmd_alias(args: &[String]) -> Result<String, String> {
    let Some(name_raw) = args.first() else {
        return Ok(ALIAS_LISTING.to_string());
    };
    let name = dequote(name_raw);
    if args.len() == 1 {
        return Ok(format!(
            "() {{ (( ${{+_csh_alias[$1]}} )) && print -r -- \"${{_csh_alias[$1]}}\"; true; }} {}",
            sq(&name)
        ));
    }
    if name == "alias" || name == "unalias" {
        return Err(format!("{name}: Too dangerous to alias that."));
    }

    let body_words = &args[1..];
    let body = body_words.iter().map(|w| dequote_alias_word(w)).collect::<Vec<_>>().join(" ");
    let listing = if body_words.len() > 1 { format!("({body})") } else { body.clone() };

    // the body is translated as if the name were not an alias (an alias
    // of `echo` that runs `echo` means the builtin)
    STATE.with(|st| st.borrow_mut().aliases.remove(&name));
    let (mut csh_body, reps, needed) = alias_refs(&body);
    if name != "echo" && csh_body.split_whitespace().next() == Some(name.as_str()) {
        let kw = if SELF_ALIAS_BUILTINS.contains(&name.as_str()) { "builtin" } else { "command" };
        csh_body = format!("{kw} {csh_body}");
    }
    if csh_body.trim().is_empty() {
        csh_body = ":".to_string();
    }
    // (a body ending in a redirect operator takes its target from the call)
    let subshell = parse(&csh_body)
        .or_else(|_| parse(&format!("{csh_body} {ARGS_MARK}")))?
        .last()
        .is_some_and(|(c, _)| c.sub.is_some());
    // Without a history reference tcsh appends the call's arguments to the
    // body; the mark travels through translation as one more word so that
    // it lands inside whatever the last command was wrapped into.
    let append_args = reps.is_empty() && !subshell;
    // a body that opens with a control keyword is a whole statement
    let control = matches!(
        csh_body.split_whitespace().next(),
        Some("if" | "while" | "foreach" | "switch")
    );
    let source = if append_args { format!("{csh_body} {ARGS_MARK}") } else { csh_body };
    let translated = if control {
        super::translate(&source)?.trim_end().to_string()
    } else {
        translate_cmds(&source)?
    };
    let mut zbody = restore_refs(&translated, &reps).replace(ARGS_MARK, "\"$@\"");
    if subshell {
        // arguments cannot follow a `( … )` group
        zbody = format!(
            "if (( $# )); then print -u2 -r -- {}; {ABORT_RET}; else {zbody}; fi",
            sq(BADLY_PLACED)
        );
    }
    if needed > 0 {
        zbody = format!(
            "if (( $# < {needed} )); then print -u2 -r -- 'Bad ! arg selector.'; {ABORT_RET}; fi; {zbody}"
        );
    }

    let safe = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'));
    if !safe {
        return Ok(format!("alias -- {}={}", sq(&name), sq(&body)));
    }
    STATE.with(|st| st.borrow_mut().aliases.insert(name.clone()));
    Ok(format!(
        "{{ typeset -gA _csh_alias _csh_alias_ls; {name}() {{ {zbody}; }}; \
_csh_alias[{name}]={}; _csh_alias_ls[{name}]={}; {ALIAS_LOOP_CHECK} {}; }}",
        sq(&body),
        sq(&listing),
        sq(&name)
    ))
}

/// Run after each alias definition. tcsh follows the first word of an alias
/// through other aliases; a chain that comes back to an alias already seen
/// (other than an alias naming itself, which refers to the real command)
/// is `Alias loop.` when any member is invoked, and ends the script.
/// Members of a loop are redefined to print that error.
const ALIAS_LOOP_CHECK: &str = "() { local _n=$1 _m; local -a _c _w; while (( ${+_csh_alias[$_n]} )); do \
_c+=($_n); _m=\"${${_csh_alias[$_n]}%% *}\"; [[ $_m == $_n ]] && return; \
if (( ${_c[(Ie)$_m]} )); then for _m in $_c; do functions[$_m]='print -u2 -r -- \"Alias loop.\"; \
{ [[ -o interactive ]] || exit 1; return 1; }'; done; return; fi; _n=$_m; done; }";

/// Quote removal for one alias word. tcsh stores the alias value as plain
/// text (quotes and backslashes removed) and lexes it again when the alias
/// is used, so `\|` is a pipe, `\*` a glob and `\!` (the way to keep a
/// history reference for invocation time) a plain `!`.
fn dequote_alias_word(w: &str) -> String {
    dequote(w).replace("\\!", "!")
}

/// Which words of the alias invocation a `!` reference selects.
enum Sel {
    All,
    Arg(usize),
    Last,
    Range(usize, usize),
    From(usize),
    ToPenultimate(usize),
}

#[derive(Clone, Copy)]
enum Ctx {
    Bare,
    Double,
    Single,
}

/// Parse the text after a `!`: `*` `^` `$` or `:sel` with an optional
/// `:h :t :r :e` modifier tail. Returns the selector, modifiers and the
/// number of characters consumed.
fn parse_ref(rest: &[char]) -> Option<(Sel, String, usize)> {
    let digits = |from: usize| -> (usize, usize) {
        let mut j = from;
        while rest.get(j).is_some_and(|c| c.is_ascii_digit()) {
            j += 1;
        }
        (rest[from..j].iter().collect::<String>().parse().unwrap_or(0), j)
    };
    let (sel, mut used) = match *rest.first()? {
        '*' => (Sel::All, 1),
        '^' => (Sel::Arg(1), 1),
        '$' => (Sel::Last, 1),
        ':' => match *rest.get(1)? {
            '*' => (Sel::All, 2),
            '^' => (Sel::Arg(1), 2),
            '$' => (Sel::Last, 2),
            c if c.is_ascii_digit() => {
                let (n, j) = digits(1);
                match rest.get(j) {
                    Some('-') if rest.get(j + 1).is_some_and(|c| c.is_ascii_digit()) => {
                        let (m, k) = digits(j + 1);
                        (Sel::Range(n, m), k)
                    }
                    Some('-') => (Sel::ToPenultimate(n), j + 1),
                    Some('*') => (Sel::From(n), j + 1),
                    _ => (Sel::Arg(n), j),
                }
            }
            _ => return None,
        },
        _ => return None,
    };
    let mut mods = String::new();
    if matches!(sel, Sel::Arg(_) | Sel::Last) {
        while rest.get(used) == Some(&':') && rest.get(used + 1).is_some_and(|c| "htreul".contains(*c)) {
            mods.push(rest[used + 1]);
            used += 2;
        }
    }
    Some((sel, mods, used))
}

fn ref_expansion(sel: &Sel, mods: &str, ctx: Ctx) -> String {
    let single = |x: String| -> String {
        let mut e = format!("${{{x}}}");
        for m in mods.chars() {
            e = match m {
                // csh `:u` / `:l` change only the first character
                'u' => format!("${{(U)${{{e}[1]}}}}${{{e}[2,-1]}}"),
                'l' => format!("${{(L)${{{e}[1]}}}}${{{e}[2,-1]}}"),
                _ => format!("${{{e}:{m}}}"),
            };
        }
        match ctx {
            Ctx::Bare => format!("\"{e}\""),
            Ctx::Double => e,
            Ctx::Single => format!("'\"{e}\"'"),
        }
    };
    let many = |x: String| -> String {
        match ctx {
            Ctx::Bare => format!("\"${{{x}}}\""),
            Ctx::Double => format!("${{(j: :){x}}}"),
            Ctx::Single => format!("'\"${{(j: :){x}}}\"'"),
        }
    };
    match sel {
        Sel::All => many("@".to_string()),
        Sel::Arg(n) => single(n.to_string()),
        Sel::Last => single("@[-1]".to_string()),
        Sel::Range(n, m) => many(format!("@[{n},{m}]")),
        Sel::From(n) => many(format!("@[{n},-1]")),
        Sel::ToPenultimate(n) => many(format!("@[{n},-2]")),
    }
}

/// Stands for the alias call's arguments while the body is translated.
const ARGS_MARK: char = '\u{E002}';
const REF_OPEN: char = '\u{E000}';
const REF_CLOSE: char = '\u{E001}';

/// Fewest alias arguments for which tcsh accepts the selector; fewer is
/// `Bad ! arg selector.` (verified: `:N` needs N, `:N-M` needs M, `:N-` needs
/// N+1, `*` `:N*` `$` never fail).
fn min_args(sel: &Sel) -> usize {
    match *sel {
        Sel::All | Sel::Last | Sel::From(_) => 0,
        Sel::Arg(n) => n,
        Sel::Range(_, m) => m,
        Sel::ToPenultimate(n) => n + 1,
    }
}

/// Replace every `!` argument reference in an alias body with a sentinel
/// (so the line translator never sees it) and collect the zsh text each
/// sentinel stands for, chosen by the quote context it appeared in.
fn alias_refs(body: &str) -> (String, Vec<String>, usize) {
    let cs: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut reps = Vec::new();
    let mut needed = 0usize;
    let (mut in_sq, mut in_dq) = (false, false);
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        match c {
            '\\' if !in_sq => {
                out.push(c);
                if let Some(&d) = cs.get(i + 1) {
                    out.push(d);
                }
                i += 2;
                continue;
            }
            '\'' if !in_dq => in_sq = !in_sq,
            '"' if !in_sq => in_dq = !in_dq,
            '!' => {
                if let Some((sel, mods, used)) = parse_ref(&cs[i + 1..]) {
                    let ctx = if in_sq {
                        Ctx::Single
                    } else if in_dq {
                        Ctx::Double
                    } else {
                        Ctx::Bare
                    };
                    needed = needed.max(min_args(&sel));
                    out.push(REF_OPEN);
                    out.push_str(&reps.len().to_string());
                    out.push(REF_CLOSE);
                    reps.push(ref_expansion(&sel, &mods, ctx));
                    i += 1 + used;
                    continue;
                }
            }
            _ => {}
        }
        out.push(c);
        i += 1;
    }
    (out, reps, needed)
}

/// Inverse of the sentinel substitution done by [`alias_refs`].
fn restore_refs(text: &str, reps: &[String]) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != REF_OPEN {
            out.push(c);
            continue;
        }
        let idx: String = chars.by_ref().take_while(|&d| d != REF_CLOSE).collect();
        if let Some(r) = idx.parse::<usize>().ok().and_then(|n| reps.get(n)) {
            out.push_str(r);
        }
    }
    out
}

fn cmd_unalias(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Err("unalias: Too few arguments.".to_string());
    }
    STATE.with(|st| {
        let mut st = st.borrow_mut();
        for w in args {
            let pat = dequote(w);
            if pat.contains(['*', '?', '[']) {
                st.aliases.clear();
            } else {
                st.aliases.remove(&pat);
            }
        }
    });
    let pats: Vec<String> = args.iter().map(|w| sq(&dequote(w))).collect();
    Ok(format!(
        "() {{ local _k _p; for _p; do for _k in ${{(k)_csh_alias[(I)$_p]}}; do unset -f -- $_k; \
unset \"_csh_alias[$_k]\" \"_csh_alias_ls[$_k]\"; done; done; }} {}",
        pats.join(" ")
    ))
}

// --- shift / exit / source / eval / repeat -----------------------------------

fn cmd_shift(args: &[String]) -> Result<String, String> {
    const NO_MORE: &str =
        "print -u2 -r -- 'shift: No more words.'; { [[ -o interactive ]] || exit 1; false; }";
    match args {
        [] => Ok(format!("{{ if (( $# )); then shift; else {NO_MORE}; fi; }}")),
        [v] => {
            let name = dequote(v);
            let name = mirrored_name(&name).unwrap_or(&name);
            if name == "argv" {
                return cmd_shift(&[]);
            }
            Ok(format!(
                "{{ if (( ! ${{+{name}}} )); then print -u2 -r -- '{name}: Undefined variable.'; {ABORT}; \
elif (( ! ${{#{name}}} )); then {NO_MORE}; else {name}=(\"${{(@){name}[2,-1]}}\"); fi; }}"
            ))
        }
        _ => Err("shift: Too many arguments.".to_string()),
    }
}

/// `exit` takes a number, optionally parenthesised, or a `$var`; it does not
/// evaluate expressions (`exit (1+2)` is "Badly formed number." in tcsh).
/// Without an argument tcsh exits 0 (verified: it ignores `$status`).
fn cmd_exit(args: &[String]) -> Result<String, String> {
    let text = cmd_exit_text(args)?;
    // in a sourced file `exit` only leaves that file
    Ok(match text.strip_prefix("exit") {
        Some(rest) if super::source_mode() => format!("return{rest}"),
        _ => text,
    })
}

fn cmd_exit_text(args: &[String]) -> Result<String, String> {
    if args.len() > 1 {
        return Err("exit: Expression Syntax.".to_string());
    }
    let Some(raw) = args.first() else {
        return Ok("exit 0".to_string());
    };
    let inner = paren_inner(raw).unwrap_or(raw.as_str());
    let value = dequote(inner);
    if value.is_empty() {
        return Ok("exit 0".to_string());
    }
    // `exit (expr)`: the status is the value of the expression.
    if paren_inner(raw).is_some() && value.contains(char::is_whitespace) {
        return super::expr::translate_exit(inner);
    }
    if value.contains('$') {
        return Ok(format!("exit {}", tw(inner)));
    }
    let digits = value.strip_prefix('-').unwrap_or(&value);
    if is_digits(digits) {
        return Ok(format!("exit {value}"));
    }
    if value.starts_with(|c: char| c.is_ascii_digit() || c == '-') {
        return Err("exit: Badly formed number.".to_string());
    }
    Err("exit: Expression Syntax.".to_string())
}

/// `source [-h] file [args]`. `-h` only loads history (no-op). The sourced
/// file is csh text and the arguments become its `argv`: both are handled by
/// the `source` function the driver preamble defines, so the call is
/// emitted unchanged and only a missing file operand is an error here.
fn cmd_source(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        None => Err("source: Too few arguments.".to_string()),
        Some("-h") => Ok(":".to_string()),
        Some(_) => Ok(generic("source", args)),
    }
}

/// `eval` re-parses its arguments as csh. When no argument expands on the
/// first parse the resulting text is known now and is translated inline;
/// otherwise the zsh `eval` is emitted and the value is taken as-is.
fn cmd_eval(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Ok("eval".to_string());
    }
    if args.iter().any(|w| has_expansion(w)) {
        // The text is only known at run time and must reach `eval` as one
        // blank-joined string: an unquoted value would be split and globbed
        // first (a lone `|` would vanish), so each expanding word is passed
        // as if double-quoted.
        let words: Vec<String> = args
            .iter()
            .map(|w| {
                let plain = !w.contains(['"', '\'', '`', '\\']);
                if plain && has_expansion(w) {
                    tw(&format!("\"{w}\""))
                } else {
                    tw(w)
                }
            })
            .collect();
        return Ok(format!("eval {}", words.join(" ")));
    }
    let text = args.iter().map(|w| dequote(w)).collect::<Vec<_>>().join(" ");
    // The text is a whole csh program (`if`/`foreach` blocks included), so it
    // goes through the script translator, not the one-line command layer.
    let inner = super::translate(&text)?;
    let inner = inner.trim_end();
    if inner.contains('\n') {
        Ok(format!("{{ {inner}\n}}"))
    } else {
        Ok(format!("{{ {inner}; }}"))
    }
}

/// `repeat N cmd`: `cmd` is one simple command; a redirection on the line is
/// applied once around the whole loop (`repeat 2 echo a > f` leaves two
/// lines in `f`).
fn cmd_repeat(args: &[String]) -> Result<String, String> {
    if args.len() < 2 {
        return Err("repeat: Too few arguments.".to_string());
    }
    let count = dequote(&args[0]);
    if !count.contains('$') && !is_digits(count.strip_prefix('-').unwrap_or(&count)) {
        return Err("repeat: Badly formed number.".to_string());
    }
    let body = render_cmd(&args[1..])?;
    // An arithmetic `for`, not `repeat`: under the drop-in's sh-style
    // emulation `repeat` is not a reserved word (c:builtin.c:214), so the
    // zsh loop would be a parse error when read line by line. A nested
    // repeat gets its own counter.
    let var = format!("_csh_r{}", body.matches("_csh_r").count());
    Ok(format!(
        "for (( {var} = {}; {var} > 0; {var}-- )); do {body}; done",
        tw(&args[0])
    ))
}

// --- which / where / umask / history / echo / directory stack ----------------

/// `which` in tcsh wording: builtins (and the zsh functions that stand in for them
/// in `--csh` mode: `cd`, `pushd`, `popd`) as `NAME: shell built-in command.`,
/// aliases as `NAME: <tab> aliased to VALUE`, misses as
/// `NAME: Command not found.` with status 1.
fn cmd_which(args: &[String]) -> String {
    let body = "() { local _c _r=0; for _c; do if (( ${+_csh_alias[$_c]} )); then \
print -r -- \"$_c: \"$'\\t'\" aliased to ${_csh_alias[$_c]}\"; else \
case \"$(whence -w -- $_c)\" in \
*\": builtin\"|*\": reserved\"|*\": function\") print -r -- \"$_c: shell built-in command.\";; \
*\": none\") print -r -- \"$_c: Command not found.\"; _r=1;; \
*) whence -p -- $_c;; esac; fi; done; return $_r; }";
    generic(body, args)
}

/// `where`: every alias / builtin / `$PATH` match, status 1 when none.
fn cmd_where(args: &[String]) -> String {
    let body = "() { local _c _p _r=1; for _c; do if (( ${+_csh_alias[$_c]} )); then \
print -r -- \"$_c is aliased to ${_csh_alias[$_c]}\"; _r=0; fi; \
case \"$(whence -w -- $_c)\" in \
*\": builtin\"|*\": reserved\") print -r -- \"$_c is a shell built-in\"; _r=0;; \
*\": function\") (( ${+_csh_alias[$_c]} )) || { print -r -- \"$_c is a shell built-in\"; _r=0; };; esac; \
for _p in ${(f)\"$(whence -pa -- $_c)\"}; do print -r -- $_p; _r=0; done; done; return $_r; }";
    generic(body, args)
}

/// `umask` prints the mask without leading zeros (zsh prints `022`).
fn cmd_umask(args: &[String]) -> Result<String, String> {
    match args {
        [] => Ok("printf '%o\\n' $((8#$(umask)))".to_string()),
        [m] => {
            let v = dequote(m);
            let in_range = u32::from_str_radix(&v, 8).is_ok_and(|m| m <= 0o777);
            if !v.contains('$') && !(is_digits(&v) && in_range) {
                return Err("umask: Improper mask.".to_string());
            }
            Ok(format!("umask {}", tw(m)))
        }
        _ => Err("umask: Too many arguments.".to_string()),
    }
}

/// `history [-h] [-r] [-c] [N]` onto `fc`.
fn cmd_history(args: &[String]) -> String {
    let mut flags = String::new();
    let mut count = None;
    for a in args {
        match a.as_str() {
            "-c" => return "fc -p".to_string(),
            "-h" => flags.push('n'),
            "-r" => flags.push('r'),
            n if is_digits(n) => count = Some(n.to_string()),
            _ => return generic("history", args),
        }
    }
    let range = count.map_or("1".to_string(), |n| format!("-{n}"));
    // an empty history lists nothing (zsh's `fc` would say "no such event")
    format!("{{ (( HISTCMD > 1 )) && fc -l{flags} {range}; true; }}")
}

/// The directory stack line tcsh prints: entries separated by spaces with a
/// trailing space.
const STACK_LINE: &str = "print -r -- \"$(dirs) \"";

/// `dirs`: `-n` and `-p` print the same single line as bare `dirs`, `-l`
/// expands `~`, `-S` / `-L` (save/load) are not supported and ignored, an
/// unknown flag is tcsh's usage error.
fn cmd_dirs(args: &[String]) -> String {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["-n"] | ["-p"] => STACK_LINE.to_string(),
        ["-l"] => "print -r -- \"$(dirs -l) \"".to_string(),
        ["-S"] | ["-L"] => ":".to_string(),
        [flag, ..] if flag.starts_with('-') && !flag[1..].chars().all(|c| "plvnSLc".contains(c)) => {
            fatal("Usage: dirs [-plvnSLc].")
        }
        _ => generic("dirs", args),
    }
}

/// The `+N` operand of `pushd` / `popd`, when `args` is exactly that.
fn stack_index(args: &[String]) -> Option<&str> {
    match args {
        [a] => a.strip_prefix('+').filter(|n| is_digits(n)),
        _ => None,
    }
}

/// `pushd`: csh always prints the stack afterwards; bare `pushd` with an
/// empty stack is "No other directory.", `pushd +N` past the end is
/// "Directory stack not that deep.".
fn cmd_pushd(args: &[String]) -> String {
    if args.is_empty() {
        return format!(
            "{{ if (( $#dirstack )); then pushd -q && {STACK_LINE}; else \
print -u2 -r -- 'pushd: No other directory.'; false; fi; }}"
        );
    }
    if let Some(n) = stack_index(args) {
        return format!(
            "if (( {n} > $#dirstack )); then {}; else {{ pushd -q +{n} && {STACK_LINE}; }}; fi",
            fatal("pushd: Directory stack not that deep.")
        );
    }
    format!("{{ {} && {STACK_LINE}; }}", generic("pushd -q", args))
}

/// `popd`: prints the stack afterwards; `popd +N` past the end is
/// "Directory stack not that deep.".
fn cmd_popd(args: &[String]) -> String {
    if let Some(n) = stack_index(args) {
        return format!(
            "if (( {n} > $#dirstack )); then {}; else {{ popd -q +{n} && {STACK_LINE}; }}; fi",
            fatal("popd: Directory stack not that deep.")
        );
    }
    format!("{{ {} && {STACK_LINE}; }}", generic("popd -q", args))
}

/// How the tcsh `echo_style` variable shapes `echo`.
#[derive(Clone, Copy, PartialEq, Default)]
enum EchoStyle {
    /// `bsd` (default): `-n` suppresses the newline, no escapes.
    #[default]
    Bsd,
    /// `sysv`: backslash escapes, `-n` is an ordinary word.
    Sysv,
    /// `both`: `-n` and backslash escapes.
    Both,
    /// `none`: neither.
    None,
}

/// Translation-time state of the shell variables whose value changes how
/// later lines are translated. A script is translated in execution order,
/// so a `set` seen earlier decides the lines after it.
#[derive(Default)]
struct ShellState {
    echo_style: EchoStyle,
    /// Names declared `set -r`: later `set` / `unset` of them is an error.
    readonly: std::collections::BTreeSet<String>,
    /// The command hash exists (`rehash`, `set path`, `setenv PATH`): only
    /// then does `hashstat` have anything to report.
    hashed: bool,
    /// Names currently aliased: a call to one goes to its function, even
    /// when the word is also a builtin this translator renders inline.
    aliases: std::collections::BTreeSet<String>,
}

thread_local! {
    static STATE: std::cell::RefCell<ShellState> = std::cell::RefCell::new(ShellState::default());
}

/// True when `word` is an expansion that could yield `-n` as a whole: it
/// starts with `$` or a backquote, or is `-` followed by one.
fn may_expand_to_dash_n(word: &str) -> bool {
    let w = word.strip_prefix('-').unwrap_or(word);
    w.starts_with(['$', '`'])
}

/// tcsh `echo` under the current `echo_style` (default `bsd`: only a leading
/// `-n` is an option and backslashes are never interpreted, unlike zsh
/// `echo`, so `print` is used). When the first word comes from an expansion
/// its value is only known at run time and `-n` is tested there, as tcsh
/// does after expansion.
fn cmd_echo(args: &[String]) -> String {
    let (raw, dash_n) = match STATE.with(|s| s.borrow().echo_style) {
        EchoStyle::Bsd => (true, true),
        EchoStyle::Sysv => (false, false),
        EchoStyle::Both => (false, true),
        EchoStyle::None => (true, false),
    };
    let print = |n: bool| -> String {
        match (raw, n) {
            (true, true) => "print -rn --",
            (true, false) => "print -r --",
            (false, true) => "print -n --",
            (false, false) => "print --",
        }
        .to_string()
    };
    match args.first() {
        Some(a) if dash_n && dequote(a) == "-n" => generic(&print(true), &args[1..]),
        Some(a) if dash_n && may_expand_to_dash_n(a) && map_special_vars(a) == *a => {
            // The words go through an array assignment, not `() { … } args`:
            // the arguments of an anonymous call are not filename-generated,
            // so an unquoted `$pat` holding `a*` would stay unexpanded.
            format!(
                "{{ _csh_ea=({}); if [[ ${{_csh_ea[1]-}} == -n ]]; then {} \"${{(@)_csh_ea[2,-1]}}\"; else {} \"${{(@)_csh_ea}}\"; fi; }}",
                args.iter().map(|w| tw(w)).collect::<Vec<_>>().join(" "),
                print(true),
                print(false)
            )
        }
        _ => generic(&print(false), args),
    }
}

/// `ls-F`: `ls -F`, with tcsh's error for a missing operand (the script
/// ends). Not reproduced: its per-directory headers and column layout.
const LS_F: &str = "() { local _a; for _a; do [[ $_a == -* || -e $_a || -L $_a ]] || \
{ print -u2 -r -- \"$_a: No such file or directory.\"; { [[ -o interactive ]] || exit 1; return 1; }; }; done; \
command ls -F \"$@\"; }";

/// `printenv [NAME]`: tcsh's builtin prints the environment or the value of
/// one exported variable, status 1 when it is not in the environment.
fn cmd_printenv(args: &[String]) -> Result<String, String> {
    match args {
        [] => Ok("env".to_string()),
        [_] => Ok(generic(
            "() { if [[ ${parameters[$1]-} == *export* ]]; then print -r -- ${(P)1}; else return 1; fi; }",
            args,
        )),
        _ => Err("printenv: Too many arguments.".to_string()),
    }
}

// --- job control, signals, time ----------------------------------------------

/// tcsh's job line: `[N]  M STATE-padded-to-29 TEXT` with `M` `+` for the
/// oldest job, `-` for the next and blank otherwise; `jobs -l` puts the pid
/// before the state.
fn cmd_jobs(args: &[String]) -> String {
    let long = args.iter().any(|a| a == "-l");
    format!(
        "() {{ local _n _s _p _k; local -a _f; local _i=0; for _n in ${{(onk)jobstates}}; do \
_f=(${{(s.:.)jobstates[$_n]}}); _s=$_f[1]; _p=${{_f[3]%%=*}}; \
case $_s in running) _s=Running;; suspended*) _s=Suspended;; done) _s=Done;; esac; \
_k=' '; (( _i == 0 )) && _k='+'; (( _i == 1 )) && _k='-'; _i=$(( _i + 1 )); \
printf '[%d]  %s %s%-29s %s\\n' $_n \"$_k\" \"{}\" \"$_s\" \"$jobtexts[$_n]\"; done; }}",
        if long { "${_p} " } else { "" }
    )
}

/// `wait`: block for every background job and report each on stderr the way
/// tcsh does (`Done`, `Exit N`, or the terminating signal).
const WAIT: &str = "() { local _n _s _f _t _k; local _i=0; for _n in ${(onk)jobstates}; do \
_t=$jobtexts[$_n]; wait %$_n 2>/dev/null; _s=$?; (( _s == 127 )) && _s=0; \
case $_s in 0) _f=Done;; 129) _f=Hangup;; 130) _f=Interrupt;; 137) _f=Killed;; 143) _f=Terminated;; \
*) _f=\"Exit $_s\";; esac; _k=' '; (( _i == 0 )) && _k='+'; _i=$(( _i + 1 )); \
printf '[%d]  %s %-29s %s\\n' $_n \"$_k\" \"$_f\" \"$_t\" >&2; done; true; }";

/// `fg` / `bg`: a script has no job control (`No job control in this
/// shell.`, script ends); an interactive shell passes them through.
fn cmd_fg_bg(head: &str, args: &[String]) -> String {
    format!(
        "if [[ -o monitor ]]; then {}; else {}; fi",
        generic(head, args),
        fatal("No job control in this shell.")
    )
}

/// `kill -l`: tcsh prints an empty line, then each signal name followed by a
/// space on its own line.
const KILL_LIST: &str = "() { local _s; print -r -- ''; for _s in ${(@)signals[2,-3]}; do print -r -- \"$_s \"; done; }";

/// `kill [-sig] id…`: an unknown signal is `SIG: Unknown signal; kill -l
/// lists signals.` and a failing target `ID: No such process` (or
/// `Operation not permitted`; tcsh prints these two on stdout), both ending
/// the script.
const KILL_FN: &str = "() { local _a _s; if [[ $1 == -<-> || $1 == -[A-Za-z]* ]]; then _s=${${1#-}#SIG}; \
[[ $_s == <-> ]] || (( ${signals[(Ie)${_s:u}]} )) || { print -u2 -r -- \"$_s: Unknown signal; kill -l lists signals.\"; \
{ [[ -o interactive ]] || exit 1; return 1; }; }; fi; builtin kill \"$@\" 2>/dev/null && return 0; \
for _a in ${@:#-*}; do builtin kill -0 -- $_a 2>/dev/null && continue; \
if ps -p $_a >/dev/null 2>&1; then print -r -- \"$_a: Operation not permitted\"; \
else print -r -- \"$_a: No such process\"; fi; { [[ -o interactive ]] || exit 1; return 1; }; done; return 1; }";

fn cmd_kill(args: &[String]) -> String {
    match args.first().map(|a| dequote(a)) {
        None => fatal("kill: Too few arguments."),
        Some(a) if a == "-l" => KILL_LIST.to_string(),
        Some(_) => generic(KILL_FN, args),
    }
}

/// tcsh `time cmd` reports `0.000u 0.000s 0:00.01 0.0%<TAB>0+0k 0+0io 0pf+0w`
/// by default; zsh's reserved word takes the same fields from `TIMEFMT`.
const TIME_FMT: &str = "$'%*Uu %*Ss 0:%*E %P\\t0+0k 0+0io %Fpf+%Ww'";

/// `filetest -op file…`: a `1` or `0` per file on one line, as the matching\n/// `if (-op f)` would answer.
fn cmd_filetest(args: &[String]) -> String {
    let op = dequote(&args[0]);
    let letter = op.strip_prefix('-').and_then(|s| s.chars().next());
    match letter {
        Some(l) if "edfrwxzslbcpSugko".contains(l) && args.len() > 1 => format!(
            "() {{ local _f; local -a _o; for _f; do [[ -{l} $_f ]] && _o+=1 || _o+=0; done; print -r -- \"${{(j: :)_o}}\"; }} {}",
            args[1..].iter().map(|w| tw(w)).collect::<Vec<_>>().join(" ")
        ),
        _ => fatal("filetest: Too few arguments."),
    }
}

fn cmd_time(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Ok("times".to_string());
    }
    Ok(format!("{{ TIMEFMT={TIME_FMT}; time {}; }}", render_cmd(args)?))
}

/// `nice [+N|-N] cmd`: the increment is added to the shell's (default 4).
/// The external `nice -n` takes it; a builtin runs as is, since tcsh
/// applies priority to forked children only.
fn cmd_nice(args: &[String]) -> Result<String, String> {
    let Some(first) = args.first() else {
        return Ok(":".to_string());
    };
    let spec = dequote(first);
    let digits = spec.strip_prefix(['+', '-']).unwrap_or("");
    let (inc, rest) = if is_digits(digits) {
        (spec.trim_start_matches('+').to_string(), &args[1..])
    } else {
        ("4".to_string(), args)
    };
    if rest.is_empty() {
        return Ok(":".to_string());
    }
    let inner = render_cmd(rest)?;
    if TCSH_BUILTINS.contains(&rest[0].as_str()) {
        Ok(inner)
    } else {
        Ok(format!("command nice -n {inc} {inner}"))
    }
}

/// `hup cmd`: run `cmd` with SIGHUP ignored; bare `hup` ignores it in the
/// shell itself.
fn cmd_hup(args: &[String]) -> Result<String, String> {
    if args.is_empty() {
        return Ok("trap '' HUP".to_string());
    }
    Ok(format!("( trap '' HUP; {} )", render_cmd(args)?))
}

// --- limit / unlimit --------------------------------------------------------

/// tcsh resource name, zsh `ulimit` flag and the unit the flag works in
/// (`s` seconds, `b` 512-byte blocks, `k` kilobytes, `n` a count).
const RESOURCES: &[(&str, char, char)] = &[
    ("cputime", 't', 's'),
    ("filesize", 'f', 'b'),
    ("datasize", 'd', 'k'),
    ("stacksize", 's', 'k'),
    ("coredumpsize", 'c', 'b'),
    ("memoryuse", 'm', 'k'),
    ("descriptors", 'n', 'n'),
    ("memorylocked", 'l', 'k'),
    ("maxproc", 'u', 'n'),
];

/// zsh text that prints one resource as tcsh does: `unlimited`, `M:SS` /
/// `H:MM:SS`, `N kbytes`, or a bare count with a trailing blank. `scope` is
/// `-S` (soft) or `-H` (hard).
fn limit_line(name: &str, flag: char, unit: char, scope: &str) -> String {
    let shown = match unit {
        's' => "(( _v >= 3600 )) && _v=\"$(( _v / 3600 )):${(l:2::0:)$(( _v % 3600 / 60 ))}:${(l:2::0:)$(( _v % 60 ))}\" \
|| _v=\"$(( _v / 60 )):${(l:2::0:)$(( _v % 60 ))}\"",
        'b' => "_v=\"$(( _v / 2 )) kbytes\"",
        'k' => "_v=\"$_v kbytes\"",
        _ => "_v=\"$_v \"",
    };
    format!(
        "_v=$(ulimit {scope} -{flag} 2>/dev/null) || _v=unlimited; [[ $_v == unlimited ]] || {{ {shown}; }}; printf '%-12s %s\\n' {name} \"$_v\""
    )
}

/// Resolve a (possibly abbreviated) tcsh resource name; the error is tcsh's
/// message for an unknown or ambiguous prefix.
fn find_resource(name: &str, cmd: &str) -> Result<(&'static str, char, char), String> {
    if let Some(exact) = RESOURCES.iter().find(|(n, _, _)| *n == name) {
        return Ok(*exact);
    }
    let hits: Vec<_> = RESOURCES
        .iter()
        .filter(|(n, _, _)| !name.is_empty() && n.starts_with(name))
        .collect();
    match hits.as_slice() {
        [one] => Ok(**one),
        [] => Err(format!("{cmd}: No such limit.")),
        _ => Err(format!("{cmd}: Ambiguous.")),
    }
}

/// Size or time operand of `limit` in the resource's unit, or `None` when it
/// is not a literal tcsh accepts. Sizes take `k` `m` `g` (kbytes by default),
/// times `s` `m` `h` or `M:SS`.
fn limit_value(text: &str, unit: char) -> Option<String> {
    if text == "unlimited" {
        return Some("unlimited".to_string());
    }
    let (num, suffix) = match text.find(|c: char| !c.is_ascii_digit() && c != ':') {
        Some(i) => text.split_at(i),
        None => (text, ""),
    };
    if num.is_empty() {
        return None;
    }
    let n = |s: &str| s.parse::<u64>().ok();
    match unit {
        's' => {
            if let Some((m, s)) = num.split_once(':') {
                return Some((n(m)? * 60 + n(s)?).to_string());
            }
            let mult = match suffix {
                "" | "s" => 1,
                "m" => 60,
                "h" => 3600,
                _ => return None,
            };
            Some((n(num)? * mult).to_string())
        }
        'b' | 'k' => {
            let kb = match suffix {
                "" | "k" => n(num)?,
                "m" => n(num)? * 1024,
                "g" => n(num)? * 1024 * 1024,
                _ => return None,
            };
            Some(kb.to_string())
        }
        _ if suffix.is_empty() => Some(num.to_string()),
        _ => None,
    }
}

/// `limit [-h] [resource [value]]` onto `ulimit`.
fn cmd_limit(args: &[String]) -> Result<String, String> {
    let hard = args.first().is_some_and(|a| a == "-h");
    let args = if hard { &args[1..] } else { args };
    let scope = if hard { "-H" } else { "-S" };
    match args {
        [] => {
            let lines: Vec<String> =
                RESOURCES.iter().map(|(n, f, u)| limit_line(n, *f, *u, scope)).collect();
            Ok(format!("() {{ local _v; {}; }}", lines.join("; ")))
        }
        [name] => Ok(match find_resource(&dequote(name), "limit") {
            Ok((n, f, u)) => format!("() {{ local _v; {}; }}", limit_line(n, f, u, scope)),
            Err(e) => fatal(&e),
        }),
        [name, value] => {
            let (n, f, u) = match find_resource(&dequote(name), "limit") {
                Ok(r) => r,
                Err(e) => return Ok(fatal(&e)),
            };
            let raw = dequote(value);
            let Some(v) = limit_value(&raw, u) else {
                return Ok(fatal("limit: Improper or unknown scale factor."));
            };
            // zsh's own `limit` reads the same time and size operands
            // (`1:30`, `2h`, `4m`); it has no `memoryuse`.
            let h = if hard { "-h " } else { "" };
            if n == "memoryuse" {
                Ok(format!("ulimit {scope} -{f} {v}"))
            } else {
                Ok(format!("limit {h}{n} {raw}"))
            }
        }
        _ => Err("limit: Too many arguments.".to_string()),
    }
}

/// `unlimit [-h] [resource…]`: raise each (all, when none is named) to
/// `unlimited`; failures to do so are silent.
fn cmd_unlimit(args: &[String]) -> Result<String, String> {
    let hard = args.first().is_some_and(|a| a == "-h");
    let args = if hard { &args[1..] } else { args };
    let scope = if hard { "-H" } else { "-S" };
    let mut flags: Vec<char> = Vec::new();
    if args.is_empty() {
        flags.extend(RESOURCES.iter().map(|(_, f, _)| *f));
    }
    for a in args {
        match find_resource(&dequote(a), "unlimit") {
            Ok((_, f, _)) => flags.push(f),
            Err(e) => return Ok(fatal(&e)),
        }
    }
    let mut stmts: Vec<String> =
        flags.iter().map(|f| format!("ulimit {scope} -{f} unlimited 2>/dev/null")).collect();
    stmts.push("true".to_string());
    Ok(format!("{{ {}; }}", stmts.join("; ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tr(line: &str) -> String {
        translate_line(line).unwrap_or_else(|e| panic!("{line}: {e}"))
    }

    fn err(line: &str) -> String {
        translate_line(line).expect_err(line)
    }

    /// True when `line` translates to `cmd` wrapped by the redirection
    /// probes (`if PROBE; then cmd; else FAIL; fi`).
    fn guarded(line: &str, cmd: &str) -> bool {
        let z = tr(line);
        z.contains("if () {")
            && (z.ends_with(&format!("then {cmd}; else false; fi"))
                || z.ends_with(&format!("then {cmd}; else {ABORT}; fi")))
    }

    #[test]
    fn and_binds_tighter_than_or() {
        // tcsh: `true || echo a && echo b` prints nothing.
        assert_eq!(
            tr("true || echo a && echo b"),
            "true || { print -r -- a && print -r -- b; }"
        );
        assert_eq!(tr("false && echo a || echo b"), "false && print -r -- a || print -r -- b");
    }

    #[test]
    fn leading_and_is_ignored_and_empty_commands_are_dropped() {
        assert_eq!(tr("&& echo a"), "print -r -- a");
        assert_eq!(tr("echo a ;; echo b"), "print -r -- a; print -r -- b");
        assert_eq!(tr("echo a;"), "print -r -- a");
        assert_eq!(tr("echo a & ; echo b"), "print -r -- a & print -r -- \"[${#jobstates}] $!\"; print -r -- b");
    }

    #[test]
    fn null_command_errors() {
        for l in ["| cat", "echo a |", "echo a &&", "|| echo a", "echo a && && echo b", "> /tmp/f", "()", "( )", "< f"] {
            assert_eq!(err(l), "Invalid null command.", "{l}");
        }
    }

    #[test]
    fn redirect_errors() {
        assert_eq!(err("echo a >"), "Missing name for redirect.");
        assert_eq!(err("echo a >&"), "Missing name for redirect.");
        assert_eq!(err("echo a <"), "Missing name for redirect.");
        assert_eq!(err("echo a > b > c"), "Ambiguous output redirect.");
        assert_eq!(err("echo a > f | cat"), "Ambiguous output redirect.");
        assert_eq!(err("echo a >& f | cat"), "Ambiguous output redirect.");
        assert_eq!(err("cat < f < f"), "Ambiguous input redirect.");
        assert_eq!(err("cat | cat < f"), "Ambiguous input redirect.");
        // a redirect to a file is probed first; the command itself is unchanged
        assert!(guarded("echo a | cat > f", "cat > f"));
        assert!(tr("cat < f | cat").ends_with("then cat < f; else false; fi | cat"));
    }

    #[test]
    fn paren_errors() {
        assert_eq!(err("echo (a b)"), "Badly placed ()'s.");
        assert_eq!(err("echo a(b)"), "Badly placed ()'s.");
        assert_eq!(err("(echo a) b"), "Badly placed ()'s.");
        assert_eq!(err("(echo a"), "Too many ('s.");
        assert_eq!(err("echo a)"), "Too many )'s.");
        assert_eq!(err("setenv A (b)"), "Badly placed ()'s.");
        assert_eq!(err("echo \"a"), "Unmatched '\"'.");
        assert_eq!(err("echo 'a"), "Unmatched '''.");
    }

    #[test]
    fn redirect_operators() {
        assert!(guarded("ls >! f", "ls >| f"));
        assert!(guarded("ls >> f", "ls >> f"));
        assert!(guarded("ls >>! f", "ls >>| f"));
        assert!(guarded("ls >& f", "ls > f 2>&1"));
        assert!(guarded("ls >>& f", "ls >> f 2>&1"));
        assert!(guarded("ls >&! f", "ls >| f 2>&1"));
        assert_eq!(tr("ls |& cat"), "ls |& cat");
        // redirect glued to words, and an fd-looking word stays a word
        assert!(guarded("echo a>f", "print -r -- a > f"));
        assert!(guarded("ls /x 2> f", "ls /x 2 > f"));
        // redirect target may precede the arguments
        assert!(guarded("echo > f a b", "print -r -- a b > f"));
    }

    #[test]
    fn heredoc_delimiter_keeps_csh_terminator_text() {
        assert_eq!(tr("cat <<EOF"), "cat <<EOF");
        assert_eq!(tr("cat << EOF"), "cat <<EOF");
        assert_eq!(tr("cat <<'EOF'"), "cat <<''\\''EOF'\\'''");
        assert!(guarded("cat <<EOF > f", "cat <<EOF > f"));
    }

    #[test]
    fn subshells_and_background() {
        // csh `&` ends a whole `;` list: `a; b &` backgrounds both.
        assert_eq!(tr("echo a; echo b &"), "( print -r -- a; print -r -- b ) & print -r -- \"[${#jobstates}] $!\";");
        assert_eq!(tr("echo a; echo b & echo c; echo d"), "( print -r -- a; print -r -- b ) & print -r -- \"[${#jobstates}] $!\"; print -r -- c; print -r -- d");
        assert_eq!(tr("echo a && echo b &"), "print -r -- a && print -r -- b & print -r -- \"[${#jobstates}] $!\";");
        assert_eq!(tr("(cd /usr; pwd)"), "( cd /usr; pwd )");
        assert_eq!(tr("(echo a; echo b) &"), "( print -r -- a; print -r -- b ) & print -r -- \"[${#jobstates}] $!\";");
        assert!(guarded("(echo a) > f", "( print -r -- a ) > f"));
        assert_eq!(tr("(echo a) | (cat)"), "( print -r -- a ) | ( cat )");
    }

    #[test]
    fn set_forms() {
        assert_eq!(tr("set"), SET_LISTING);
        assert_eq!(tr("set x"), "x=('')");
        assert_eq!(tr("set x = word"), "x=(word)");
        assert_eq!(tr("set x=val"), "x=(val)");
        assert_eq!(tr("set x = "), "x=('')");
        assert_eq!(tr("set x = (a b c)"), "x=(a b c)");
        assert_eq!(tr("set x=(a b c)"), "x=(a b c)");
        assert_eq!(tr("set x= (a b)"), "x=(a b)");
        assert_eq!(tr("set x = ()"), "x=()");
        assert!(tr("set x[2] = w").ends_with("x[2]=w; }"));
        assert!(tr("set x[2]=w").contains("'set: Subscript out of range.'"));
        assert_eq!(tr("set x=a=b"), "x=(a=b)");
        assert_eq!(tr("set x = \"a b\""), "x=(\"a b\")");
    }

    #[test]
    fn set_multiple_assignments_follow_tcsh_parse() {
        // `set x = a b` sets x=a and declares b empty.
        assert_eq!(tr("set x = a b"), "{ x=(a); b=(''); }");
        assert_eq!(tr("set a=1 b=2"), "{ a=(1); b=(2); }");
        assert_eq!(tr("set a = 1 b = 2"), "{ a=(1); b=(2); }");
        assert_eq!(tr("set x= val"), "{ x=(''); val=(''); }");
        assert_eq!(tr("set x = (a b) y = (c d)"), "{ x=(a b); y=(c d); }");
    }

    #[test]
    fn set_readonly_flag_and_empty_line() {
        assert_eq!(tr("set -r x = 1"), "readonly x=(1)");
        assert_eq!(tr(""), "");
        assert_eq!(tr("  ;  "), "");
    }

    #[test]
    fn set_errors() {
        assert_eq!(err("set 1x = 2"), "set: Variable name must begin with a letter.");
        assert_eq!(err("set x =val"), "set: Variable name must begin with a letter.");
        assert_eq!(err("set = x"), "set: Variable name must begin with a letter.");
        assert_eq!(err("set x-y=1"), "set: Variable name must contain alphanumeric characters.");
        assert_eq!(err("set x[1-2] = (p q)"), "set: Subscript error.");
        assert_eq!(err("set x[2] = (p q)"), "set: Syntax Error.");
        assert_eq!(err("set x = (a b"), "Too many ('s.");
    }

    #[test]
    fn set_mirrors_special_variables() {
        // path is tied to PATH in zsh itself.
        assert_eq!(tr("set path = (/bin /usr/bin)"), "path=(/bin /usr/bin)");
        assert_eq!(tr("set home = /tmp/h"), "export HOME=/tmp/h");
        assert_eq!(tr("set user=bob"), "export USER=bob");
        assert_eq!(tr("set prompt = \"> \""), "PROMPT=\"> \"");
        assert_eq!(tr("set prompt2 = x"), "PROMPT2=x");
        assert_eq!(tr("set history = 50"), "HISTSIZE=50");
        assert_eq!(tr("set noclobber"), "{ setopt noclobber; noclobber=(''); }");
        assert_eq!(tr("set nonomatch"), "{ unsetopt nomatch cshnullglob; nonomatch=(''); }");
        assert!(tr("unset noclobber").starts_with("{ unsetopt noclobber; "));
        // names are rewritten before the word layer sees them; single quotes protect
        let w = |s: &str| translate_word(s);
        assert_eq!(
            tr("echo $cwd ${home} $?user '$cwd'"),
            format!("print -r -- {} {} {} '$cwd'", w("$PWD"), w("${HOME}"), w("$?USER"))
        );
    }

    #[test]
    fn unset_plain_and_glob() {
        // shell variables only: exported (environment) parameters are skipped
        let g = tr("unset a b*");
        assert!(g.ends_with("} 'a' 'b*'") && g.contains("== *export* ]] || unset"), "{g}");
        assert_eq!(tr("unset home"), "unset HOME");
    }

    #[test]
    fn setenv_forms() {
        assert_eq!(tr("setenv"), "printenv");
        assert_eq!(tr("setenv FOO"), "export FOO=''");
        assert_eq!(tr("setenv FOO bar"), "export FOO=bar");
        assert_eq!(tr("setenv FOO \"a b\""), "export FOO=\"a b\"");
        assert_eq!(tr("setenv PATH /usr/bin:/bin"), "export PATH=/usr/bin:/bin");
        assert_eq!(err("setenv FOO bar baz"), "setenv: Too many arguments.");
        assert_eq!(err("setenv FOO=bar"), "setenv: Variable name must contain alphanumeric characters.");
        assert_eq!(err("setenv 1X a"), "setenv: Variable name must begin with a letter.");
        assert_eq!(err("unsetenv"), "unsetenv: Too few arguments.");
        assert_eq!(err("unset"), "unset: Too few arguments.");
        // environment only: shell variables of the same name are kept
        let g = tr("unsetenv A B*");
        assert!(g.ends_with("} 'A' 'B*'") && g.contains("== *export* ]] && unset"), "{g}");
    }

    #[test]
    fn alias_listing_query_and_definition() {
        assert_eq!(tr("alias"), ALIAS_LISTING);
        assert!(tr("alias nope").ends_with("} 'nope'"));
        let d = tr("alias ll \"ls -l\"");
        assert!(d.contains("ll() { ls -l \"$@\"; }"), "{d}");
        assert!(d.contains("_csh_alias[ll]='ls -l'"), "{d}");
        assert!(d.contains("_csh_alias_ls[ll]='ls -l'"), "{d}");
        // several words: listing shows the word list in parentheses
        assert!(tr("alias ll ls -l").contains("_csh_alias_ls[ll]='(ls -l)'"));
        assert_eq!(err("alias alias x"), "alias: Too dangerous to alias that.");
        assert_eq!(err("alias unalias x"), "unalias: Too dangerous to alias that.");
    }

    #[test]
    fn alias_history_references_become_positional_parameters() {
        let d = tr("alias g \"echo \\!*\"");
        assert!(d.contains("g() { print -r -- \"${@}\"; }"), "{d}");
        let d = tr("alias f 'echo \\!^ \\!:2 \\!$ \\!:2-3 \\!:2- \\!:2* \\!:0'");
        assert!(
            d.contains("print -r -- \"${1}\" \"${2}\" \"${@[-1]}\" \"${@[2,3]}\" \"${@[2,-2]}\" \"${@[2,-1]}\" \"${0}\";"),
            "{d}"
        );
        let d = tr("alias b 'echo \\!:1:t'");
        assert!(d.contains("\"${${1}:t}\""), "{d}");
        let d = tr("alias u 'echo \\!:1:u'");
        assert!(d.contains("\"${(U)${${1}[1]}}${${1}[2,-1]}\""), "{d}");
        // inside double quotes the reference is spliced as text
        let d = tr("alias q 'echo \"\\!*\"'");
        assert!(d.contains("\"${(j: :)@}\""), "{d}");
        // a body with a pipe or list keeps both halves
        let d = tr("alias p \"echo x | cat\"");
        assert!(d.contains("print -r -- x | cat \"$@\""), "{d}");
    }

    #[test]
    fn alias_self_reference_uses_command_or_builtin() {
        assert!(tr("alias ls ls -F").contains("ls() { command ls -F \"$@\"; }"));
        let d = tr("alias cd 'cd \\!*; echo in'");
        assert!(d.contains("cd() { builtin cd \"${@}\"; print -r -- in; }"), "{d}");
    }

    #[test]
    fn unalias_requires_a_name() {
        assert_eq!(err("unalias"), "unalias: Too few arguments.");
        assert!(tr("unalias a*").ends_with("'a*'"));
    }

    #[test]
    fn shift_forms() {
        assert!(tr("shift").contains("if (( $# )); then shift; else"));
        assert!(tr("shift argv").contains("if (( $# )); then shift; else"));
        assert!(tr("shift x").contains("x=(\"${(@)x[2,-1]}\")"));
        assert!(tr("shift x").contains("'x: Undefined variable.'"));
        assert_eq!(err("shift a b"), "shift: Too many arguments.");
    }

    #[test]
    fn exit_forms() {
        assert_eq!(tr("exit"), "exit 0");
        assert_eq!(tr("exit 3"), "exit 3");
        assert_eq!(tr("exit (3)"), "exit 3");
        assert_eq!(tr("exit(3)"), "exit 3");
        assert_eq!(tr("exit ( 3 )"), "exit 3");
        assert_eq!(tr("exit \"3\""), "exit 3");
        assert_eq!(tr("exit -1"), "exit -1");
        assert_eq!(tr("exit ()"), "exit 0");
        assert_eq!(err("exit (1+2)"), "exit: Badly formed number.");
        assert_eq!(err("exit 1a"), "exit: Badly formed number.");
        assert_eq!(err("exit a"), "exit: Expression Syntax.");
        assert_eq!(err("exit 1 2"), "exit: Expression Syntax.");
    }

    #[test]
    fn source_forms() {
        assert_eq!(tr("source f.csh"), "source f.csh");
        assert_eq!(tr("source f.csh a b"), "source f.csh a b");
        assert_eq!(tr("source -h f"), ":");
        assert_eq!(err("source"), "source: Too few arguments.");
    }

    #[test]
    fn at_delegates_to_expression_layer() {
        assert_eq!(tr("@ n = 1 + 2"), translate_at("n = 1 + 2").unwrap());
        // operators outside parentheses split the line, as in csh
        assert_eq!(err("@ x = 1 |"), "Invalid null command.");
    }

    #[test]
    fn eval_translates_static_text_as_csh() {
        assert_eq!(tr("eval echo hi"), "{ print -r -- hi; }");
        assert_eq!(tr("eval 'set x = (1 2)'"), "{ x=(1 2); }");
        assert_eq!(tr("eval \"echo a;echo b\""), "{ print -r -- a; print -r -- b; }");
        // a first-parse expansion means the text is only known at run time
        assert_eq!(tr("eval $c"), format!("eval {}", translate_word("\"$c\"")));
    }

    #[test]
    fn repeat_forms() {
        assert_eq!(
            tr("repeat 3 echo hi"),
            "for (( _csh_r0 = 3; _csh_r0 > 0; _csh_r0-- )); do print -r -- hi; done"
        );
        // redirect applies around the whole loop, pipe/list bind to the loop
        assert!(guarded(
            "repeat 2 echo a > f",
            "for (( _csh_r0 = 2; _csh_r0 > 0; _csh_r0-- )); do print -r -- a; done > f"
        ));
        assert_eq!(
            tr("repeat 2 echo a | wc -l"),
            "for (( _csh_r0 = 2; _csh_r0 > 0; _csh_r0-- )); do print -r -- a; done | wc -l"
        );
        assert_eq!(err("repeat"), "repeat: Too few arguments.");
        assert_eq!(err("repeat 3"), "repeat: Too few arguments.");
        assert_eq!(err("repeat a echo"), "repeat: Badly formed number.");
        assert_eq!(err("repeat 2 (echo a)"), "Badly placed ()'s.");
    }

    #[test]
    fn echo_is_bsd_style() {
        assert_eq!(tr("echo a b"), "print -r -- a b");
        assert_eq!(tr("echo -n a"), "print -rn -- a");
        assert_eq!(tr("echo \"-n\" a"), "print -rn -- a");
        assert_eq!(tr("echo -e a"), "print -r -- -e a");
        assert_eq!(tr("echo"), "print -r --");
    }

    #[test]
    fn umask_forms() {
        assert_eq!(tr("umask"), "printf '%o\\n' $((8#$(umask)))");
        assert_eq!(tr("umask 022"), "umask 022");
        assert_eq!(err("umask 99"), "umask: Improper mask.");
        assert_eq!(err("umask a"), "umask: Improper mask.");
        assert_eq!(err("umask 022 1"), "umask: Too many arguments.");
        assert_eq!(err("umask 7777"), "umask: Improper mask.");
        assert_eq!(err("umask 1000"), "umask: Improper mask.");
        assert_eq!(tr("umask 0777"), "umask 0777");
    }

    #[test]
    fn directory_stack() {
        assert_eq!(tr("dirs"), "print -r -- \"$(dirs) \"");
        assert_eq!(tr("dirs -v"), "dirs -v");
        assert_eq!(tr("pushd /usr"), "{ pushd -q /usr && print -r -- \"$(dirs) \"; }");
        assert_eq!(tr("popd"), "{ popd -q && print -r -- \"$(dirs) \"; }");
        assert!(tr("pushd").contains("'pushd: No other directory.'"));
    }

    #[test]
    fn misc_builtins() {
        assert_eq!(err("which"), "which: Too few arguments.");
        assert_eq!(err("where"), "where: Too few arguments.");
        assert_eq!(tr("hashstat"), ":");
        assert_eq!(tr("nohup"), "trap '' HUP");
        assert_eq!(tr("nohup sleep 1"), "nohup sleep 1");
        assert_eq!(tr("limit cputime 10"), "limit cputime 10");
        assert_eq!(tr("history 3"), "{ (( HISTCMD > 1 )) && fc -l -3; true; }");
        assert_eq!(tr("history -h"), "{ (( HISTCMD > 1 )) && fc -ln 1; true; }");
        assert_eq!(tr("chdir /usr"), "cd /usr");
        assert!(tr("which ls").ends_with("} ls"));
        assert!(tr("where ls nope").ends_with("} ls nope"));
    }
    // --- behaviour under zsh, expectations taken from `/bin/tcsh -f` ---------

    /// Translate each line and run the result under `/bin/zsh -f` with the
    /// driver's `cshnullglob`, in an empty scratch directory. `None` when no
    /// zsh is installed.
    fn run(name: &str, lines: &[&str]) -> Option<(String, String, i32)> {
        let zsh = ["/bin/zsh", "/usr/bin/zsh", "/usr/local/bin/zsh", "/opt/homebrew/bin/zsh"]
            .into_iter()
            .find(|p| std::path::Path::new(p).exists())?;
        let dir = std::env::temp_dir().join(format!("csh_cmds_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok()?;
        let mut code = String::from("setopt cshnullglob extendedglob\n");
        for l in lines {
            code.push_str(&tr(l));
            code.push('\n');
        }
        let o = std::process::Command::new(zsh)
            .args(["-f", "-c", &code])
            .current_dir(&dir)
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        let _ = std::fs::remove_dir_all(&dir);
        Some((
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
            o.status.code().unwrap_or(-1),
        ))
    }

    /// Assert stdout, stderr and exit status of the translated lines.
    fn expect(name: &str, lines: &[&str], out: &str, err: &str, rc: i32) {
        if let Some((o, e, r)) = run(name, lines) {
            assert_eq!((o.as_str(), e.as_str(), r), (out, err, rc), "{lines:?}");
        }
    }

    #[test]
    fn failed_redirect_of_a_builtin_ends_the_script() {
        expect(
            "redir_builtin",
            &["echo ok", "echo a > /nonexistent_zz/f", "echo not-reached"],
            "ok\n",
            "/nonexistent_zz/f: No such file or directory.\n",
            1,
        );
    }

    #[test]
    fn failed_redirect_of_an_external_command_continues() {
        expect(
            "redir_external",
            &["ls > /nonexistent_zz/f", "echo st=$status", "echo b > /nonexistent_zz/f || echo alt"],
            "st=1\n",
            "/nonexistent_zz/f: No such file or directory.\n/nonexistent_zz/f: No such file or directory.\n",
            1,
        );
    }

    #[test]
    fn noclobber_refuses_to_overwrite_and_a_forced_redirect_does_not() {
        expect(
            "noclobber",
            &["set noclobber", "echo a > f1", "echo b >! f1", "cat f1", "echo c > f1", "echo not-reached"],
            "b\n",
            "f1: File exists.\n",
            1,
        );
        // `>>` onto a missing file is refused under noclobber
        expect(
            "noclobber_append",
            &["set noclobber", "echo c >> nofile_zz", "echo not-reached"],
            "",
            "nofile_zz: No such file or directory.\n",
            1,
        );
    }

    #[test]
    fn missing_input_file_ends_a_builtin_but_not_an_external_command() {
        expect(
            "input_redirect",
            &["cat < nofile_zz", "echo st=$status", "echo a < nofile_zz", "echo not-reached"],
            "st=1\n",
            "nofile_zz: No such file or directory.\nnofile_zz: No such file or directory.\n",
            1,
        );
    }

    #[test]
    fn no_match_is_fatal_for_a_builtin_and_status_one_for_an_external_command() {
        expect(
            "no_match",
            &["ls *.nomatch_zz", "echo st=$status", "echo a *.nomatch_zz", "echo not-reached"],
            "st=1\n",
            "ls: No match.\necho: No match.\n",
            1,
        );
        // one matching pattern is enough
        expect("one_match", &["touch f1.c", "echo *.zz *.c"], "f1.c\n", "", 0);
    }

    #[test]
    fn undefined_variable_ends_the_script_even_for_an_external_command() {
        expect(
            "undefined",
            &["echo ok", "ls $undef_zz", "echo not-reached"],
            "ok\n",
            "undef_zz: Undefined variable.\n",
            1,
        );
        expect("defined", &["set x = a", "echo $x $#x $?undef_zz $status"], "a 1 0 0\n", "", 0);
    }

    #[test]
    fn history_reference_drops_the_whole_line_and_ends_the_script() {
        expect(
            "history",
            &["echo ok", "echo a; echo b!c", "echo not-reached"],
            "ok\n",
            "c: Event not found.\n",
            1,
        );
        expect("history_bang_bang", &["echo !!"], "", "0: Event not found.\n", 1);
        // an escaped or blank-followed `!` is literal
        expect("history_literal", &["echo a\\!b \\!x ! x a!=b"], "a!b !x ! x a!=b\n", "", 0);
    }

    #[test]
    fn history_event_text_follows_tcsh() {
        assert_eq!(history_event("echo a!b"), Some("b".to_string()));
        assert_eq!(history_event("echo !5"), Some("5".to_string()));
        assert_eq!(history_event("echo !-1"), Some("0".to_string()));
        assert_eq!(history_event("echo !?foo?"), Some("foo".to_string()));
        assert_eq!(history_event("echo !foo:2"), Some("foo".to_string()));
        assert_eq!(history_event("echo 'a!b'"), Some("b".to_string()));
        assert_eq!(history_event("echo a!~b !(x) a!"), None);
        assert_eq!(history_event("echo a;b !; !& !| !> !<"), None);
    }

    #[test]
    fn status_survives_the_guards() {
        expect(
            "status",
            &["sh -c \"exit 3\" >& /dev/null", "echo st=$status", "sh -c \"exit 4\"", "echo $status > f1", "cat f1"],
            "st=3\n4\n",
            "",
            0,
        );
    }

    #[test]
    fn command_path_errors() {
        expect(
            "path_command",
            &["./nosuch_zz", "echo st=$status", "/tmp", "echo st=$status"],
            "st=1\nst=1\n",
            "./nosuch_zz: Command not found.\n/tmp: Permission denied.\n",
            0,
        );
    }

    #[test]
    fn unknown_user_follows_the_builtin_rule() {
        expect(
            "tilde",
            &["ls ~nosuchuser_zz", "echo st=$status", "echo ~nosuchuser_zz", "echo not-reached"],
            "st=1\n",
            "Unknown user: nosuchuser_zz.\nUnknown user: nosuchuser_zz.\n",
            1,
        );
    }

    #[test]
    fn glob_words_ignore_subscripts_and_unclosed_brackets() {
        assert!(is_glob_word("*.c") && is_glob_word("a?") && is_glob_word("[a-z]x"));
        assert!(!is_glob_word("$x[2]") && !is_glob_word("${x}[1]") && !is_glob_word("[a"));
        assert!(!is_glob_word("'*'") && !is_glob_word("\"a?\"") && !is_glob_word("\\*"));
        assert!(!is_glob_word("$?x") && !is_glob_word("$#x"));
        expect("bracket_literal", &["echo [a"], "[a\n", "", 0);
        expect("class_without_match", &["echo [a x[1]"], "", "echo: No match.\n", 1);
    }

    #[test]
    fn variable_refs_skip_specials_and_single_quotes() {
        assert_eq!(variable_refs("$a ${b} $#c $d[1] \"$e\" '$f' \\$g $?h $1 $$ $status $argv"),
            vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn alias_value_is_relexed_text() {
        // `\|` is stored as `|` and runs as a pipe when the alias is used
        let d = tr("alias a3 echo hi \\| tr h H");
        assert!(d.contains("a3() { print -r -- hi | tr h H \"$@\"; }"), "{d}");
        assert!(d.contains("_csh_alias_ls[a3]='(echo hi | tr h H)'"), "{d}");
        expect("alias_pipe", &["alias a3 echo hi \\| tr h H", "a3"], "Hi\n", "", 0);
    }

    #[test]
    fn alias_loops_end_the_script_when_invoked() {
        expect(
            "alias_loop",
            &["alias x1 y1", "alias y1 x1", "echo before", "x1", "echo not-reached"],
            "before\n",
            "Alias loop.\n",
            1,
        );
        // an alias naming itself runs the real command, a chain does not loop
        expect(
            "alias_chain",
            &["alias a1 b1", "alias b1 c1", "alias c1 echo end", "a1 hi", "alias ls ls -F", "alias ls"],
            "end hi\nls -F\n",
            "",
            0,
        );
    }

    #[test]
    fn alias_argument_selectors_need_enough_arguments() {
        expect(
            "alias_selector",
            &["alias a1 \"echo \\!:1\"", "a1 x", "a1", "echo not-reached"],
            "x\n",
            "Bad ! arg selector.\n",
            1,
        );
        assert_eq!(min_args(&Sel::Arg(2)), 2);
        assert_eq!(min_args(&Sel::Range(1, 3)), 3);
        assert_eq!(min_args(&Sel::ToPenultimate(2)), 3);
        assert_eq!(min_args(&Sel::From(5)), 0);
        assert_eq!(min_args(&Sel::All), 0);
    }

    #[test]
    fn alias_body_that_is_a_subshell_takes_no_arguments() {
        expect(
            "alias_subshell",
            &["alias r \"(echo a; echo b)\"", "r", "r x", "echo not-reached"],
            "a\nb\n",
            "Badly placed ()'s.\n",
            1,
        );
        assert_eq!(err("alias q (p)"), "Badly placed ()'s.");
    }

    #[test]
    fn alias_with_unquoted_history_glob_is_no_match() {
        // `\!*` is a pattern for tcsh: no file named `!…` means `alias: No match.`
        expect("alias_glob", &["alias h echo \\!:1 end \\!*", "echo not-reached"], "", "alias: No match.\n", 1);
    }

    #[test]
    fn alias_with_semicolon_ends_at_the_semicolon() {
        expect(
            "alias_semicolon",
            &["alias a1 echo one; echo two", "alias a1", "a1 x"],
            "two\necho one\none x\n",
            "",
            0,
        );
    }

    #[test]
    fn readonly_variables_reject_set_and_unset() {
        expect(
            "readonly",
            &["set -r rr = 1", "echo $rr", "set rr = 2", "echo not-reached"],
            "1\n",
            "set: $rr is read-only.\n",
            1,
        );
        expect(
            "readonly_unset",
            &["set -r rr = 1", "unset rr", "echo not-reached"],
            "",
            "unset: $rr is read-only.\n",
            1,
        );
    }

    #[test]
    fn echo_reads_dash_n_after_expansion_and_honours_echo_style() {
        expect("echo_expanded_n", &["set o = -n", "echo $o x", "echo y"], "xy\n", "", 0);
        expect("echo_empty_first", &["set e = ''", "echo $e -n y"], "y", "", 0);
        expect(
            "echo_style_none",
            &["set echo_style = none", "echo -n a", "echo \"\"", "echo \"x\\ty\""],
            "-n a\n\nx\\ty\n",
            "",
            0,
        );
        expect(
            "echo_style_both",
            &["set echo_style = both", "echo \"a\\tb\"", "echo -n c", "echo", "unset echo_style", "echo \"a\\tb\""],
            "a\tb\nc\na\\tb\n",
            "",
            0,
        );
    }

    #[test]
    fn kill_errors() {
        expect(
            "kill_signal",
            &["kill -FOO 99999999", "echo not-reached"],
            "",
            "FOO: Unknown signal; kill -l lists signals.\n",
            1,
        );
        // a missing process is reported on stdout, without a period
        expect("kill_pid", &["kill 99999999", "echo not-reached"], "99999999: No such process\n", "", 1);
        expect("kill_none", &["kill", "echo not-reached"], "", "kill: Too few arguments.\n", 1);
    }

    #[test]
    fn job_control_builtins_in_a_script() {
        expect("fg", &["fg", "echo not-reached"], "", "No job control in this shell.\n", 1);
        expect("bg", &["bg", "echo not-reached"], "", "No job control in this shell.\n", 1);
        expect("stop", &["stop", "echo not-reached"], "", "stop: Too few arguments.\n", 1);
        expect("notify", &["notify", "echo not-reached"], "", "notify: No current job.\n", 1);
        expect("suspend", &["suspend", "echo ok"], "ok\n", "", 0);
    }

    #[test]
    fn login_builtins_in_a_non_login_shell() {
        expect("logout", &["logout", "echo not-reached"], "", "Not a login shell.\n", 1);
        expect("login", &["login", "echo not-reached"], "", "Not a login shell.\n", 1);
        expect("bye", &["echo ok", "bye", "echo not-reached"], "ok\n", "", 0);
    }

    #[test]
    fn glob_builtin_separates_with_nul_and_no_newline() {
        expect("glob", &["touch ga gb", "glob g?", "echo", "glob nomatch_zz*", "echo not-reached"],
            "ga\0gb\n", "glob: No match.\n", 1);
    }

    #[test]
    fn dirs_flags() {
        expect("dirs_usage", &["dirs -x", "echo not-reached"], "", "Usage: dirs [-plvnSLc].\n", 1);
        assert_eq!(tr("dirs -S"), ":");
        assert_eq!(tr("dirs -n"), "print -r -- \"$(dirs) \"");
        assert!(tr("popd +2").contains("popd: Directory stack not that deep."));
        assert!(tr("pushd +2").contains("pushd: Directory stack not that deep."));
    }

    #[test]
    fn cd_with_two_operands_is_an_error() {
        expect("cd_args", &["cd /usr /tmp", "echo not-reached"], "", "cd: Too many arguments.\n", 1);
    }

    #[test]
    fn printenv_prints_exported_values_only() {
        expect(
            "printenv",
            &["setenv ZZ_A 1", "printenv ZZ_A", "printenv ZZ_NOPE", "echo st=$status"],
            "1\nst=1\n",
            "",
            0,
        );
    }

    #[test]
    fn limit_forms() {
        // listing: `name`, padding to 12, a blank, the value; counts keep a
        // trailing blank, sizes are `N kbytes`, times `M:SS`
        let z = tr("limit");
        assert!(z.contains("printf '%-12s %s\\n' cputime"), "{z}");
        assert!(z.contains("printf '%-12s %s\\n' descriptors"), "{z}");
        assert_eq!(tr("limit cputime 10"), "limit cputime 10");
        assert_eq!(tr("limit -h stacksize 4m"), "limit -h stacksize 4m");
        assert_eq!(tr("limit nosuch 1"), fatal("limit: No such limit."));
        assert_eq!(tr("limit c"), fatal("limit: Ambiguous."));
        assert_eq!(tr("limit cputime 1x"), fatal("limit: Improper or unknown scale factor."));
        assert_eq!(tr("unlimit nosuch"), fatal("unlimit: No such limit."));
        assert_eq!(limit_value("1:30", 's').as_deref(), Some("90"));
        assert_eq!(limit_value("2h", 's').as_deref(), Some("7200"));
        assert_eq!(limit_value("4m", 'k').as_deref(), Some("4096"));
        assert_eq!(limit_value("1x", 'k'), None);
    }

    #[test]
    fn nice_and_hup() {
        assert_eq!(tr("nice"), ":");
        assert_eq!(tr("nice ls"), "command nice -n 4 ls");
        assert_eq!(tr("nice +7 ls -l"), "command nice -n 7 ls -l");
        assert_eq!(tr("nice -3 ls"), "command nice -n -3 ls");
        assert_eq!(tr("nice echo hi"), "print -r -- hi");
        assert_eq!(tr("hup"), "trap '' HUP");
        assert_eq!(tr("hup sleep 1"), "( trap '' HUP; sleep 1 )");
    }

    #[test]
    fn builtins_lists_the_tcsh_builtins() {
        let z = tr("builtins");
        assert!(z.starts_with("print -rl -- ':' '@' 'alias'"), "{z}");
        assert!(TCSH_BUILTINS.windows(2).all(|w| w[0] <= w[1]), "sorted as tcsh prints it");
    }

    #[test]
    fn source_passes_every_operand_through() {
        assert_eq!(tr("source ~/.cshrc.local a b"), format!("source {} a b", tw("~/.cshrc.local")));
        assert_eq!(err("source"), "source: Too few arguments.");
    }

}
