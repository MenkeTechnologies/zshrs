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

use super::expr::translate_at;
use super::lex::split_words;
use super::words::translate_word;

const NULL_COMMAND: &str = "Invalid null command.";
const MISSING_NAME: &str = "Missing name for redirect.";
const AMBIGUOUS_OUT: &str = "Ambiguous output redirect.";
const AMBIGUOUS_IN: &str = "Ambiguous input redirect.";
const BADLY_PLACED: &str = "Badly placed ()'s.";

/// Translate a command line that may hold lists (`;` `&&` `||`), pipes
/// (`|` `|&`), background `&`, redirections (`>&` `>!` `>>&` `>>!` `<<`),
/// parenthesised subshells, and the builtins whose syntax differs from zsh
/// (`set` `unset` `setenv` `unsetenv` `alias` `unalias` `shift` `exit`
/// `source` `@` `limit` `unlimit` `rehash` `hashstat` `which` …).
/// Words go through [`super::words::translate_word`].
///
/// An empty line yields an empty string. Errors carry tcsh's message text
/// (`Invalid null command.`, `Ambiguous output redirect.`, …).
pub fn translate_line(line: &str) -> Result<String, String> {
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
        pipes.push(render_pipeline(&pipeline, &conns)?);
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
                group.clear();
            }
            Sep::Bg => {
                parts.push(format!("{} &", group[0]));
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
                    if d == '\\' && c != '\'' {
                        if let Some(&e) = cs.get(i) {
                            p.word.push(e);
                            i += 1;
                        }
                    }
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
fn render_pipeline(stages: &[Simple], conns: &[Sep]) -> Result<String, String> {
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
        out.push_str(&render_simple(s)?);
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

fn render_simple(s: &Simple) -> Result<String, String> {
    let mut text = match &s.sub {
        Some(inner) => {
            if inner.trim().is_empty() {
                return Err(NULL_COMMAND.to_string());
            }
            format!("( {} )", translate_line(inner)?)
        }
        None => render_cmd(&s.words)?,
    };
    for r in s.ins.iter().chain(&s.outs) {
        text.push(' ');
        text.push_str(&render_redirect(r));
    }
    Ok(text)
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
                for pre in ['{', '?', '#'] {
                    if cs.get(i) == Some(&pre) {
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
                    Some(z) => out.push_str(z),
                    None => out.push_str(&name),
                }
            }
            _ => {}
        }
    }
    out
}

/// csh word → zsh word, with the mirrored variable names applied first.
fn tw(word: &str) -> String {
    translate_word(&map_special_vars(word))
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
    let allows_parens = matches!(head, "set" | "@" | "alias" | "exit");
    if !allows_parens && words.iter().any(|w| w.starts_with('(')) {
        return Err(BADLY_PLACED.to_string());
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
        "hashstat" | "unhash" => Ok(":".to_string()),
        "chdir" => Ok(generic("cd", args)),
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
        "nonomatch" => ("nomatch", true),
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
        let val = set_value(args, &mut i, rest);
        stmts.extend(set_statements(name, sub, val, readonly)?);
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
        match mirrored_name(&name) {
            Some(z) => stmts.push(format!("unset {z}")),
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

fn cmd_setenv(args: &[String]) -> Result<String, String> {
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

    let (mut csh_body, reps) = alias_refs(&body);
    if name != "echo" && csh_body.split_whitespace().next() == Some(name.as_str()) {
        let kw = if SELF_ALIAS_BUILTINS.contains(&name.as_str()) { "builtin" } else { "command" };
        csh_body = format!("{kw} {csh_body}");
    }
    let mut zbody = if csh_body.trim().is_empty() { ":".to_string() } else { translate_line(&csh_body)? };
    zbody = restore_refs(&zbody, &reps);
    if reps.is_empty() {
        zbody.push_str(" \"$@\"");
    }

    let safe = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '+' | '-'));
    if !safe {
        return Ok(format!("alias -- {}={}", sq(&name), sq(&body)));
    }
    Ok(format!(
        "{{ typeset -gA _csh_alias _csh_alias_ls; {name}() {{ {zbody}; }}; \
_csh_alias[{name}]={}; _csh_alias_ls[{name}]={}; }}",
        sq(&body),
        sq(&listing)
    ))
}

/// Quote removal for one alias word: a wholly quoted word loses its quotes,
/// and `\!` (the csh way to keep a history reference for invocation time)
/// becomes `!`. Anything else is kept verbatim for the later re-parse.
fn dequote_alias_word(w: &str) -> String {
    let whole = |q: char| w.len() >= 2 && w.starts_with(q) && w.ends_with(q) && !w[1..w.len() - 1].contains(q);
    let inner = if whole('\'') || whole('"') { &w[1..w.len() - 1] } else { w };
    inner.replace("\\!", "!")
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

const REF_OPEN: char = '\u{E000}';
const REF_CLOSE: char = '\u{E001}';

/// Replace every `!` argument reference in an alias body with a sentinel
/// (so the line translator never sees it) and collect the zsh text each
/// sentinel stands for, chosen by the quote context it appeared in.
fn alias_refs(body: &str) -> (String, Vec<String>) {
    let cs: Vec<char> = body.chars().collect();
    let mut out = String::new();
    let mut reps = Vec::new();
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
    (out, reps)
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
    let pats: Vec<String> = args.iter().map(|w| sq(&dequote(w))).collect();
    Ok(format!(
        "() {{ local _k _p; for _p; do for _k in ${{(k)_csh_alias[(I)$_p]}}; do unset -f -- $_k; \
unset \"_csh_alias[$_k]\" \"_csh_alias_ls[$_k]\"; done; done; }} {}",
        pats.join(" ")
    ))
}

// --- shift / exit / source / eval / repeat -----------------------------------

fn cmd_shift(args: &[String]) -> Result<String, String> {
    const NO_MORE: &str = "print -u2 -r -- 'shift: No more words.'; false";
    match args {
        [] => Ok(format!("{{ if (( $# )); then shift; else {NO_MORE}; fi; }}")),
        [v] => {
            let name = dequote(v);
            let name = mirrored_name(&name).unwrap_or(&name);
            if name == "argv" {
                return cmd_shift(&[]);
            }
            Ok(format!(
                "{{ if (( ! ${{+{name}}} )); then print -u2 -r -- '{name}: Undefined variable.'; false; \
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

/// `source [-h] file`. tcsh ignores extra arguments; `-h` only loads history.
/// The sourced file is csh text: the engine must translate it on load.
fn cmd_source(args: &[String]) -> Result<String, String> {
    match args.first().map(String::as_str) {
        None => Err("source: Too few arguments.".to_string()),
        Some("-h") => Ok(":".to_string()),
        Some(file) => Ok(format!("source {}", tw(file))),
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
        return Ok(generic("eval", args));
    }
    let text = args.iter().map(|w| dequote(w)).collect::<Vec<_>>().join(" ");
    let inner = translate_line(&text)?;
    Ok(format!("{{ {inner}; }}"))
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
    Ok(format!("repeat {}; do {body}; done", tw(&args[0])))
}

// --- which / where / umask / history / echo / directory stack ----------------

/// `which` in tcsh wording: builtins as `NAME: shell built-in command.`,
/// aliases as `NAME: <tab> aliased to VALUE`, misses as
/// `NAME: Command not found.` with status 1.
fn cmd_which(args: &[String]) -> String {
    let body = "() { local _c _r=0; for _c; do if (( ${+_csh_alias[$_c]} )); then \
print -r -- \"$_c: \"$'\\t'\" aliased to ${_csh_alias[$_c]}\"; else \
case \"$(whence -w -- $_c)\" in \
*\": builtin\"|*\": reserved\") print -r -- \"$_c: shell built-in command.\";; \
*\": none\") print -r -- \"$_c: Command not found.\"; _r=1;; \
*) whence -p -- $_c;; esac; fi; done; return $_r; }";
    generic(body, args)
}

/// `where`: every alias / builtin / `$PATH` match, status 1 when none.
fn cmd_where(args: &[String]) -> String {
    let body = "() { local _c _p _r=1; for _c; do if (( ${+_csh_alias[$_c]} )); then \
print -r -- \"$_c is aliased to ${_csh_alias[$_c]}\"; _r=0; fi; \
case \"$(whence -w -- $_c)\" in \
*\": builtin\"|*\": reserved\") print -r -- \"$_c is a shell built-in\"; _r=0;; esac; \
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
    format!("fc -l{flags} {range}")
}

/// The directory stack line tcsh prints: entries separated by spaces with a
/// trailing space.
const STACK_LINE: &str = "print -r -- \"$(dirs) \"";

fn cmd_dirs(args: &[String]) -> String {
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => STACK_LINE.to_string(),
        ["-l"] => "print -r -- \"$(dirs -l) \"".to_string(),
        _ => generic("dirs", args),
    }
}

/// `pushd`: csh always prints the stack afterwards; bare `pushd` with an
/// empty stack is "No other directory.".
fn cmd_pushd(args: &[String]) -> String {
    if args.is_empty() {
        return format!(
            "{{ if (( $#dirstack )); then pushd -q && {STACK_LINE}; else \
print -u2 -r -- 'pushd: No other directory.'; false; fi; }}"
        );
    }
    format!("{{ {} && {STACK_LINE}; }}", generic("pushd -q", args))
}

fn cmd_popd(args: &[String]) -> String {
    format!("{{ {} && {STACK_LINE}; }}", generic("popd -q", args))
}

/// tcsh `echo` (default `echo_style bsd`) honours only a leading `-n` and
/// never interprets backslash escapes; zsh `echo` does, so use `print -r`.
fn cmd_echo(args: &[String]) -> String {
    match args.first() {
        Some(a) if dequote(a) == "-n" => generic("print -rn --", &args[1..]),
        _ => generic("print -r --", args),
    }
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
        assert_eq!(tr("echo a & ; echo b"), "print -r -- a & print -r -- b");
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
        assert_eq!(tr("echo a | cat > f"), "print -r -- a | cat > f");
        assert_eq!(tr("cat < f | cat"), "cat < f | cat");
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
        assert_eq!(tr("ls >! f"), "ls >| f");
        assert_eq!(tr("ls >> f"), "ls >> f");
        assert_eq!(tr("ls >>! f"), "ls >>| f");
        assert_eq!(tr("ls >& f"), "ls > f 2>&1");
        assert_eq!(tr("ls >>& f"), "ls >> f 2>&1");
        assert_eq!(tr("ls >&! f"), "ls >| f 2>&1");
        assert_eq!(tr("ls |& cat"), "ls |& cat");
        // redirect glued to words, and an fd-looking word stays a word
        assert_eq!(tr("echo a>f"), "print -r -- a > f");
        assert_eq!(tr("ls /x 2> f"), "ls /x 2 > f");
        // redirect target may precede the arguments
        assert_eq!(tr("echo > f a b"), "print -r -- a b > f");
    }

    #[test]
    fn heredoc_delimiter_keeps_csh_terminator_text() {
        assert_eq!(tr("cat <<EOF"), "cat <<EOF");
        assert_eq!(tr("cat << EOF"), "cat <<EOF");
        assert_eq!(tr("cat <<'EOF'"), "cat <<''\\''EOF'\\'''");
        assert_eq!(tr("cat <<EOF > f"), "cat <<EOF > f");
    }

    #[test]
    fn subshells_and_background() {
        // csh `&` ends a whole `;` list: `a; b &` backgrounds both.
        assert_eq!(tr("echo a; echo b &"), "( print -r -- a; print -r -- b ) &");
        assert_eq!(tr("echo a; echo b & echo c; echo d"), "( print -r -- a; print -r -- b ) & print -r -- c; print -r -- d");
        assert_eq!(tr("echo a && echo b &"), "print -r -- a && print -r -- b &");
        assert_eq!(tr("(cd /usr; pwd)"), "( cd /usr; pwd )");
        assert_eq!(tr("(echo a; echo b) &"), "( print -r -- a; print -r -- b ) &");
        assert_eq!(tr("(echo a) > f"), "( print -r -- a ) > f");
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
        assert_eq!(tr("set nonomatch"), "{ unsetopt nomatch; nonomatch=(''); }");
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
        assert_eq!(tr("source f.csh a b"), "source f.csh");
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
        assert_eq!(tr("eval $c"), format!("eval {}", translate_word("$c")));
    }

    #[test]
    fn repeat_forms() {
        assert_eq!(tr("repeat 3 echo hi"), "repeat 3; do print -r -- hi; done");
        // redirect applies around the whole loop, pipe/list bind to the loop
        assert_eq!(tr("repeat 2 echo a > f"), "repeat 2; do print -r -- a; done > f");
        assert_eq!(tr("repeat 2 echo a | wc -l"), "repeat 2; do print -r -- a; done | wc -l");
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
        assert_eq!(tr("history 3"), "fc -l -3");
        assert_eq!(tr("history -h"), "fc -ln 1");
        assert_eq!(tr("chdir /usr"), "cd /usr");
        assert!(tr("which ls").ends_with("} ls"));
        assert!(tr("where ls nope").ends_with("} ls nope"));
    }
}
