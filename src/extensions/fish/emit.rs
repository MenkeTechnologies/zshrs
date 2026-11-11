//! fish AST → zsh source.
//!
//! The model: every fish variable is a list, and a zsh array is a list. A
//! variable reference expands to the elements without word splitting or empty
//! removal, and an undefined variable expands to no words at all:
//!
//! | fish | zsh |
//! |---|---|
//! | `$x` | `${x+"${x[@]}"}` (no words when unset, empty elements kept) |
//! | `"$x"` | `"${(j: :)x}"` |
//! | `$x[2]` | `"${(@)x[2,2]}"` |
//! | `$x[2..4]` | `"${(@)x[2,4]}"` |
//! | `$x.txt` | `${^x}.txt` (one word per element, as fish's cartesian product) |
//! | `(cmd)` | `${(f)"$(cmd)"}` (split on newlines only) |
//!
//! Exported variables are zsh scalars (`typeset -x`), because zsh does not
//! export arrays; a list is joined with `:` for names ending in `PATH` and
//! with a space otherwise, which is also what fish puts in the environment.

use super::parse::{Cmd, Conn, DPart, Job, Part, Pipe, Redir, Script, Stmt, VarRef, Word};

/// Lowercase fish variable names that collide with zsh special parameters. A fish
/// `for path in $list` must not rewrite `$PATH`, so these live under a prefix.
const ZSH_SPECIAL_LOWER: &[&str] = &[
    "path", "cdpath", "fpath", "manpath", "mailpath", "module_path", "signals", "options",
    "commands", "functions", "aliases", "parameters", "history", "dirstack", "widgets",
    "keymaps", "modules", "nameddirs", "userdirs", "galiases", "saliases", "funcstack",
    "funcfiletrace", "funcsourcetrace", "functrace", "jobtexts", "jobstates", "jobdirs",
    "builtins", "reswords", "patchars", "histchars", "termcap", "terminfo", "watch",
    "psvar", "zsh_eval_context", "dis_aliases", "dis_builtins", "dis_functions",
    "dis_galiases", "dis_reswords", "dis_saliases", "status", "pipestatus", "prompt",
];

/// Words that start a compound command in zsh but are plain command names in fish.
const ZSH_ONLY_RESERVED: &[&str] = &[
    "repeat", "select", "until", "foreach", "coproc", "nocorrect", "always", "[[", "((", "{", "}",
];

#[derive(Default)]
pub struct Emitter {
    /// Function nesting; `set -l` is `local` only inside one.
    pub(super) fn_depth: usize,
    /// `__fcN="$(…)"` assignments the command being emitted needs ahead of it.
    hoists: Vec<String>,
    hoist_id: usize,
}

/// The zsh parameter that holds a fish variable.
pub fn zname(name: &str) -> String {
    match name {
        "PATH" => "path".into(),
        "CDPATH" => "cdpath".into(),
        "MANPATH" => "manpath".into(),
        "hostname" => "HOST".into(),
        n if ZSH_SPECIAL_LOWER.contains(&n) && n != "pipestatus" => format!("_fish_v_{n}"),
        n => n.into(),
    }
}

/// fish splits variables named `*PATH` on `:` when expanded; zsh keeps them scalar.
/// The tied ones (`PATH` → `path`) already are arrays.
pub fn is_colon_scalar(name: &str) -> bool {
    name.ends_with("PATH") && !matches!(name, "PATH" | "CDPATH" | "MANPATH")
}

/// A name usable as a zsh parameter, or `None` for things like `$1`.
fn valid_param(name: &str) -> bool {
    !name.is_empty() && !name.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// `'…'` for zsh.
pub fn squote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// A literal as a zsh word: bare when every character is plain, quoted otherwise.
pub fn quote_lit(s: &str) -> String {
    let plain = !s.is_empty()
        && !s.starts_with(['=', '~'])
        && s.chars().all(|c| c.is_ascii_alphanumeric() || "_./:@%+,=-".contains(c));
    if plain {
        s.to_string()
    } else {
        squote(s)
    }
}

/// Text inside a zsh double-quoted string.
fn dq_escape(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c == '\x7f' => {
                // leave the quotes, emit the control character, re-enter them
                out.push_str(&format!("\"$'\\x{:02x}'\"", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

impl Emitter {
    pub fn script(&mut self, script: &Script) -> String {
        let mut out = String::new();
        for (n, st) in script.iter().enumerate() {
            let job = self.job(&st.job);
            match (n, st.conn) {
                (0, Conn::And) => out.push_str("(( ! $? )) && "),
                (0, Conn::Or) => out.push_str("(( $? )) && "),
                (0, _) => {}
                (_, Conn::Seq) => out.push('\n'),
                (_, Conn::And) => out.push_str(" && "),
                (_, Conn::Or) => out.push_str(" || "),
            }
            out.push_str(&job);
        }
        out
    }

    /// A body, never empty: zsh rejects an empty `then`/`do`/`{}`.
    fn body(&mut self, s: &Script) -> String {
        let t = self.script(s);
        if t.trim().is_empty() {
            ":".into()
        } else {
            t
        }
    }

    /// `if`/`while` conditions: the `and`/`or` chain on one line.
    fn cond(&mut self, stmts: &[Stmt]) -> String {
        self.script(&stmts.to_vec())
    }

    fn job(&mut self, j: &Job) -> String {
        let mut out = String::new();
        if j.negate {
            out.push_str("! ");
        }
        for (n, (cmd, pipe)) in j.cmds.iter().enumerate() {
            out.push_str(&self.cmd(cmd));
            if n + 1 < j.cmds.len() {
                out.push_str(match pipe {
                    Pipe::Out => " | ",
                    Pipe::Both => " |& ",
                });
            }
        }
        if j.bg {
            out.push_str(" &");
        }
        out
    }

    /// One command. A command substitution in its words is run first, into a
    /// variable, so an output of no lines gives no words while blank lines inside the
    /// output stay words (`${(f)…}` alone can do only one of the two).
    fn cmd(&mut self, c: &Cmd) -> String {
        let outer = std::mem::take(&mut self.hoists);
        let text = self.cmd_inner(c);
        let mine = std::mem::replace(&mut self.hoists, outer);
        if mine.is_empty() {
            return text;
        }
        let sets_variable = matches!(c, Cmd::Simple { words, .. }
            if words.first().and_then(Word::bare) == Some("set"));
        // `set x (cmd)` leaves the status of `cmd`
        if sets_variable {
            format!("{{ {}; __fish_cs=$?; {text}; __fish_ret $__fish_cs; }}", mine.join("; "))
        } else {
            format!("{{ {}; {text}; }}", mine.join("; "))
        }
    }

    fn cmd_inner(&mut self, c: &Cmd) -> String {
        match c {
            Cmd::Simple { words, redirs } => self.simple(words, redirs),
            Cmd::If { arms, els, redirs } => {
                let mut out = String::new();
                for (n, (cond, body)) in arms.iter().enumerate() {
                    out.push_str(if n == 0 { "if " } else { "\nelif " });
                    out.push_str(&self.cond(cond));
                    out.push_str("\nthen\n");
                    out.push_str(&self.body(body));
                }
                if let Some(e) = els {
                    out.push_str("\nelse\n");
                    out.push_str(&self.body(e));
                }
                out.push_str("\nfi");
                out.push_str(&self.redirs(redirs));
                out
            }
            Cmd::While { cond, body, redirs } => format!(
                "while {}\ndo\n{}\ndone{}",
                self.cond(cond),
                self.body(body),
                self.redirs(redirs)
            ),
            Cmd::For {
                var,
                list,
                body,
                redirs,
            } => {
                let v = var.text().map(|t| zname(&t)).unwrap_or_else(|| "_fish_v".into());
                let items: Vec<String> = list.iter().map(|w| self.word_list(w)).collect();
                let text = format!(
                    "for {v} in {}\ndo\n{}\ndone{}",
                    items.join(" "),
                    self.body(body),
                    self.redirs(redirs)
                );
                if has_glob(list) {
                    format!("builtin eval {}", squote(&text))
                } else {
                    text
                }
            }
            Cmd::Switch {
                subject,
                cases,
                redirs,
            } => {
                let mut out = format!("case {} in\n", self.word_scalar(subject));
                for (pats, body) in cases {
                    let pats: Vec<String> = pats.iter().map(|p| self.case_pattern(p)).collect();
                    out.push_str(&format!("({})\n{}\n;;\n", pats.join("|"), self.body(body)));
                }
                out.push_str("esac");
                out.push_str(&self.redirs(redirs));
                out
            }
            Cmd::Function { header, body } => self.function(header, body),
            Cmd::Begin { body, redirs } => {
                // `set -l` inside `begin … end` is scoped to the block. An anonymous
                // function gives zsh the same scope, unless the block leaves through
                // `return`/`break`/`continue`, which would then leave only the function.
                if declares_local(body) && !leaves_block(body) {
                    self.fn_depth += 1;
                    let inner = self.body(body);
                    self.fn_depth -= 1;
                    format!("() {{\n{inner}\n}} \"$@\"{}", self.redirs(redirs))
                } else {
                    format!("{{\n{}\n}}{}", self.body(body), self.redirs(redirs))
                }
            }
        }
    }

    /// A `case` pattern. fish compares with its wildcard matcher where only `*` is
    /// special (checked: `case "a?c"` does not match `abc`), so everything else is
    /// escaped.
    fn case_pattern(&mut self, w: &Word) -> String {
        match w.text() {
            Some(t) => t
                .chars()
                .map(|c| match c {
                    '*' => "*".to_string(),
                    c if c.is_ascii_alphanumeric() || c == '_' => c.to_string(),
                    c if (c as u32) < 0x20 => format!("$'\\x{:02x}'", c as u32),
                    c => format!("\\{c}"),
                })
                .collect(),
            None => self.word_scalar(w),
        }
    }

    fn function(&mut self, header: &[Word], body: &Script) -> String {
        let mut name = None;
        let mut argnames: Vec<String> = Vec::new();
        let mut i = 0;
        while i < header.len() {
            let t = header[i].text().unwrap_or_default();
            i += 1;
            match t.as_str() {
                "-a" | "--argument-names" => {
                    while let Some(n) = header.get(i).and_then(Word::text).filter(|n| !n.starts_with('-')) {
                        argnames.push(n);
                        i += 1;
                    }
                }
                "-d" | "--description" | "-w" | "--wraps" | "-e" | "--on-event" | "-v"
                | "--on-variable" | "-j" | "--on-job-exit" | "-p" | "--on-process-exit" | "-s"
                | "--on-signal" | "-V" | "--inherit-variable" => i += 1,
                s if s.starts_with('-') => {}
                _ if name.is_none() => name = Some(t),
                _ => {}
            }
        }
        let name = name.unwrap_or_default();
        self.fn_depth += 1;
        let mut inner = String::new();
        for (k, a) in argnames.iter().enumerate() {
            inner.push_str(&format!(
                "local -a {}=(\"${{(@)argv[{},{}]}}\")\n",
                zname(a),
                k + 1,
                k + 1
            ));
        }
        inner.push_str(&self.body(body));
        self.fn_depth -= 1;
        format!("function {} {{\n{inner}\n}}", quote_lit(&name))
    }

    // ---- simple commands --------------------------------------------------------

    fn simple(&mut self, words: &[Word], redirs: &[Redir]) -> String {
        // leading `NAME=value` words are environment assignments for this command
        let mut prefix = Vec::new();
        let mut k = 0;
        while let Some(a) = words.get(k).and_then(|w| self.assignment(w)) {
            prefix.push(a);
            k += 1;
        }
        let rest = &words[k..];
        let name = rest.first().and_then(|w| w.bare()).map(str::to_string);
        let mut out = match name.as_deref() {
            Some("set") => self.set_cmd(&rest[1..]),
            Some("read") => self.read_cmd(&rest[1..]),
            Some("alias") => self.alias_cmd(&rest[1..]),
            Some("abbr") => self.abbr_cmd(&rest[1..]),
            Some(n) if ZSH_ONLY_RESERVED.contains(&n) => {
                let tail: Vec<String> = rest[1..].iter().map(|w| self.word_list(w)).collect();
                format!("{} {}", squote(n), tail.join(" "))
            }
            _ => {
                let mut all: Vec<String> = rest.iter().map(|w| self.word_list(w)).collect();
                // fish's `echo` takes -n -s -e -E (and `--`); zsh's does not
                let flagged = rest
                    .get(1)
                    .and_then(|w| w.bare())
                    .is_some_and(|t| t.len() > 1 && t.starts_with('-'));
                if name.as_deref() == Some("echo") && flagged {
                    all[0] = "__fish_echo".into();
                }
                all.join(" ")
            }
        };
        if !prefix.is_empty() {
            out = format!("{} {out}", prefix.join(" "));
        }
        out.push_str(&self.redirs(redirs));
        // A wildcard with no match is an error in both shells, but zsh ends the whole
        // script on it and fish only skips the command. `eval` confines the abort.
        if has_glob(&rest[1.min(rest.len())..]) {
            out = format!("builtin eval {}", squote(&out));
        }
        out
    }

    /// `NAME=value` as an environment assignment word.
    fn assignment(&mut self, w: &Word) -> Option<String> {
        let Some(Part::Lit(first)) = w.parts.first() else {
            return None;
        };
        let (name, value) = first.split_once('=')?;
        if name.is_empty()
            || name.starts_with(|c: char| c.is_ascii_digit())
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return None;
        }
        let mut tail = Word {
            parts: w.parts[1..].to_vec(),
        };
        if !value.is_empty() {
            tail.parts.insert(0, Part::Lit(value.to_string()));
        }
        let v = if tail.parts.is_empty() {
            "''".to_string()
        } else {
            self.word_scalar(&tail)
        };
        Some(format!("{}={v}", zname(name)))
    }

    pub(super) fn redirs(&mut self, rs: &[Redir]) -> String {
        let mut out = String::new();
        for r in rs {
            out.push(' ');
            if let Some(fd) = r.fd {
                out.push_str(&fd.to_string());
            }
            let target = self.word_scalar(&r.target);
            match r.op.as_str() {
                ">?" => out.push_str(&format!(">{target}")),
                ">&" | "<&" => {
                    out.push_str(&r.op);
                    out.push_str(&target);
                }
                op => {
                    out.push_str(op);
                    out.push(' ');
                    out.push_str(&target);
                }
            }
        }
        out
    }

    // ---- words ------------------------------------------------------------------

    /// The word as zsh text that expands to zero or more words.
    pub(super) fn word_list(&mut self, w: &Word) -> String {
        match w.parts.as_slice() {
            [] => "''".into(),
            [Part::Var(v)] => self.var_list(v),
            [Part::Cmd(s, idx)] => self.cmd_list(s, idx.as_deref()),
            parts => parts.iter().map(|p| self.part(p, false)).collect(),
        }
    }

    /// The word as zsh text that expands to exactly one word.
    pub(super) fn word_scalar(&mut self, w: &Word) -> String {
        if w.parts.is_empty() {
            return "''".into();
        }
        w.parts.iter().map(|p| self.part(p, true)).collect()
    }

    /// One part of a multi-part word. `scalar` joins lists; otherwise a list part
    /// distributes over its neighbours the way fish's cartesian product does.
    fn part(&mut self, p: &Part, scalar: bool) -> String {
        match p {
            Part::Lit(s) => quote_lit(s),
            Part::Raw(s) => s.clone(),
            Part::Single(s) => squote(s),
            Part::Double(ds) => self.double(ds),
            Part::Var(v) if scalar => format!("\"{}\"", self.var_join(v)),
            Part::Var(v) => self.var_rc(v),
            Part::Cmd(s, idx) if scalar => {
                let inner = self.script(s);
                match idx {
                    None => format!("\"$( {inner} )\""),
                    Some(_) => self.cmd_list(s, idx.as_deref()),
                }
            }
            Part::Cmd(s, idx) => format!("${{^{}}}", self.cmd_list(s, idx.as_deref())),
        }
    }

    fn double(&mut self, ds: &[DPart]) -> String {
        if ds.is_empty() {
            return "\"\"".into();
        }
        let mut out = String::from("\"");
        for d in ds {
            match d {
                DPart::Lit(s) => out.push_str(&dq_escape(s)),
                DPart::Var(v) => out.push_str(&self.var_join(v)),
                DPart::Cmd(s) => out.push_str(&format!("$( {} )", self.script(s))),
            }
        }
        out.push('"');
        out
    }

    /// `(cmd)` as a word list: one word per output line, none for empty output.
    fn cmd_list(&mut self, s: &Script, idx: Option<&[String]>) -> String {
        // `(… | string collect)` is one word, newlines and all, even when empty
        if idx.is_none() && ends_in_string_collect(s) {
            return format!("\"$( {} )\"", self.script(s));
        }
        let inner = self.script(s);
        self.hoist_id += 1;
        let v = format!("__fc{}", self.hoist_id);
        self.hoists.push(format!("{v}=\"$( {inner} )\""));
        match idx {
            None => format!("${{{v}:+\"${{(@f){v}}}\"}}"),
            Some(toks) => ranges(toks)
                .iter()
                .map(|(r, rev)| {
                    format!("${{{v}:+\"${{(@{})${{(@f){v}}}[{r}]}}\"}}", if *rev { "Oa" } else { "" })
                })
                .collect::<Vec<_>>()
                .join(" "),
        }
    }

    /// Index tokens with any `(cmd)` token replaced by the variable its output was
    /// hoisted into.
    fn index_tokens(&mut self, toks: &[String]) -> Vec<String> {
        toks.iter()
            .map(|t| {
                let inner = t.strip_prefix('(').and_then(|r| r.strip_suffix(')'));
                match inner.and_then(|i| super::parse::parse(i).ok()) {
                    Some(script) => {
                        let text = self.script(&script);
                        self.hoist_id += 1;
                        let v = format!("__fc{}", self.hoist_id);
                        self.hoists.push(format!("{v}=\"$( {text} )\""));
                        format!("${v}")
                    }
                    None => t.clone(),
                }
            })
            .collect()
    }

    /// `$x` as a word list.
    fn var_list(&mut self, v: &VarRef) -> String {
        let n = &v.name;
        if let Some(special) = special_scalar(n) {
            return special.into();
        }
        if !valid_param(n) {
            return "\"\"".into();
        }
        let z = zname(n);
        if v.deref {
            return format!("\"${{(@P){z}}}\"");
        }
        if let Some(idx) = &v.index {
            let idx = self.index_tokens(idx);
            return ranges(&idx)
                .iter()
                .map(|(r, rev)| format!("\"${{(@{}){z}[{r}]}}\"", if *rev { "Oa" } else { "" }))
                .collect::<Vec<_>>()
                .join(" ");
        }
        if is_colon_scalar(n) {
            return format!("${{{z}:+\"${{(@s.:.){z}}}\"}}");
        }
        format!("${{{z}+\"${{{z}[@]}}\"}}")
    }

    /// `$x` inside a larger word: one word per element.
    fn var_rc(&mut self, v: &VarRef) -> String {
        let n = &v.name;
        if let Some(special) = special_scalar(n) {
            return special.into();
        }
        if !valid_param(n) {
            return "\"\"".into();
        }
        let z = zname(n);
        if v.deref || v.index.is_some() {
            return format!("\"{}\"", self.var_join(v));
        }
        if is_colon_scalar(n) {
            return format!("${{^${{(@s.:.){z}}}}}");
        }
        format!("${{^{z}}}")
    }

    /// `$x` joined to one string, for use inside `"…"`.
    fn var_join(&mut self, v: &VarRef) -> String {
        let n = &v.name;
        if let Some(special) = special_scalar(n) {
            return special.into();
        }
        if !valid_param(n) {
            return String::new();
        }
        let z = zname(n);
        if v.deref {
            return format!("${{(P){z}}}");
        }
        if let Some(idx) = &v.index {
            let idx = self.index_tokens(idx);
            let parts: Vec<String> = ranges(&idx)
                .iter()
                .map(|(r, rev)| format!("${{({}j: :){z}[{r}]}}", if *rev { "Oa" } else { "" }))
                .collect();
            return parts.join(" ");
        }
        if is_colon_scalar(n) {
            return format!("${{{z}}}");
        }
        format!("${{(j: :){z}}}")
    }
}

/// Variables whose value is a scalar zsh already has under another spelling.
fn special_scalar(name: &str) -> Option<&'static str> {
    match name {
        "status" => Some("$?"),
        "fish_pid" => Some("$$"),
        "last_pid" => Some("$!"),
        _ => None,
    }
}

/// fish index tokens (`1`, `-1`, `2..4`, `$i`) as zsh subscript ranges. A descending
/// range of plain numbers is returned with `true` so the caller reverses it.
fn ranges(tokens: &[String]) -> Vec<(String, bool)> {
    tokens
        .iter()
        .map(|t| match t.split_once("..") {
            Some((a, b)) => match (a.parse::<i64>(), b.parse::<i64>()) {
                (Ok(x), Ok(y)) if x > y && (x < 0) == (y < 0) => (format!("{y},{x}"), true),
                _ => (format!("{a},{b}"), false),
            },
            None => (format!("{t},{t}"), false),
        })
        .collect()
}

/// Does any argument contain an unquoted wildcard?
fn has_glob(words: &[Word]) -> bool {
    words.iter().any(|w| {
        w.parts
            .iter()
            .any(|p| matches!(p, Part::Raw(r) if r.starts_with('*')))
    })
}

/// Words of every simple command in a script, not descending into function bodies.
fn simple_commands(script: &Script, out: &mut Vec<Vec<Word>>) {
    for st in script {
        for (cmd, _) in &st.job.cmds {
            match cmd {
                Cmd::Simple { words, .. } => out.push(words.clone()),
                Cmd::If { arms, els, .. } => {
                    for (cond, body) in arms {
                        simple_commands(cond, out);
                        simple_commands(body, out);
                    }
                    if let Some(e) = els {
                        simple_commands(e, out);
                    }
                }
                Cmd::While { cond, body, .. } => {
                    simple_commands(cond, out);
                    simple_commands(body, out);
                }
                Cmd::For { body, .. } | Cmd::Begin { body, .. } => simple_commands(body, out),
                Cmd::Switch { cases, .. } => cases.iter().for_each(|(_, b)| simple_commands(b, out)),
                Cmd::Function { .. } => {}
            }
        }
    }
}

/// Does the block contain a `set -l` / `set --local`?
fn declares_local(script: &Script) -> bool {
    let mut cmds = Vec::new();
    simple_commands(script, &mut cmds);
    cmds.iter().any(|w| {
        w.first().and_then(Word::bare) == Some("set")
            && w[1..].iter().any(|a| {
                a.bare().is_some_and(|t| {
                    t == "--local" || (t.starts_with('-') && !t.starts_with("--") && t.contains('l'))
                })
            })
    })
}

/// Does the block contain `return`, `break` or `continue`?
fn leaves_block(script: &Script) -> bool {
    let mut cmds = Vec::new();
    simple_commands(script, &mut cmds);
    cmds.iter()
        .any(|w| matches!(w.first().and_then(Word::bare), Some("return" | "break" | "continue")))
}

/// Is the last command of the script `string collect`?
fn ends_in_string_collect(s: &Script) -> bool {
    let Some(Stmt { job, .. }) = s.last() else {
        return false;
    };
    match job.cmds.last() {
        Some((Cmd::Simple { words, .. }, _)) => {
            words.first().and_then(Word::bare) == Some("string") && words.get(1).and_then(Word::bare) == Some("collect")
        }
        _ => false,
    }
}
