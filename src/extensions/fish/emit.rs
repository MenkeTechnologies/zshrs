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
    /// Lexical scopes, innermost last; see [`Frame`].
    scopes: Vec<Frame>,
    local_id: usize,
    /// Set while emitting the words of `set` and `count`, where a wildcard with no
    /// match is an empty list rather than an error.
    null_glob: bool,
    /// True while emitting a statement that only runs if the one before it succeeded or
    /// failed (`a; or set -l x …`).
    pub(super) guarded: bool,
    /// Source line of the statement being emitted (for `status line-number`).
    pub(super) cur_line: usize,
    /// The script names variables at run time or uses `eval`: top-level locals keep their names.
    pub(super) plain_top: bool,
    /// Variables some `--on-variable` handler in this script watches; only their
    /// `set`s are followed by the handler call.
    pub(super) watched_vars: std::collections::HashSet<String>,
    /// Lines a guarded declaration needs ahead of the whole `and`/`or` chain.
    pub(super) pre_chain: Vec<String>,
}

/// One lexical scope. fish scopes `set -l` to the enclosing block (`if`, `while`,
/// `for`, `switch` case, `begin`) and to a function body; zsh has only function scope.
/// A block-local variable therefore lives under a unique zsh name for as long as its
/// block is being emitted, and a function body is a `barrier`, since a fish function
/// does not see the locals of the block it is defined in.
#[derive(Default)]
struct Frame {
    vars: std::collections::HashMap<String, String>,
    barrier: bool,
    /// The scope names variables at run time (`set $name`, `$$name`), which only plain
    /// names can follow; its locals are not renamed.
    plain: bool,
}

thread_local! {
    /// Names declared `set -l` outside any block or function. Kept across translations
    /// in this process, so `set -l x 1` typed at the prompt is still local to the session
    /// on the next line; a sourced file or `eval` is translated in its own process and
    /// has its own.
    static TOP_LOCALS: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

thread_local! {
    /// Names exported with `set -x` outside functions. They are kept as a zsh array
    /// (`__fx_NAME`) tied to the exported scalar, because fish exports a list but a zsh
    /// array cannot be exported. Kept across translations like [`TOP_LOCALS`].
    static TOP_EXPORTED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// Remember that `name` is an exported list from now on.
pub fn note_exported(name: &str) {
    TOP_EXPORTED.with(|t| t.borrow_mut().insert(name.to_string()));
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
    /// The zsh parameter a fish name currently refers to, block-locals included.
    pub(super) fn zn(&self, name: &str) -> String {
        for f in self.scopes.iter().rev() {
            if let Some(m) = f.vars.get(name) {
                return m.clone();
            }
            if f.barrier {
                // a function does not see the locals of the script that defines it
                return zname(name);
            }
        }
        if TOP_LOCALS.with(|t| t.borrow().contains(name)) {
            return format!("_fl_top_{name}");
        }
        if TOP_EXPORTED.with(|t| t.borrow().contains(name)) {
            return format!("__fx_{name}");
        }
        zname(name)
    }

    /// Declare `name` local to the innermost scope, whatever that is, and return the zsh
    /// name it lives under.
    pub(super) fn declare_local(&mut self, name: &str) -> String {
        if self.block_level() > 0 || self.fn_depth > 0 {
            return self.declare_block_local(name);
        }
        if self.plain_top {
            return zname(name);
        }
        TOP_LOCALS.with(|t| t.borrow_mut().insert(name.to_string()));
        format!("_fl_top_{name}")
    }

    /// Does the innermost scope already map `name` to `to`?
    pub(super) fn is_mapped(&self, name: &str, to: &str) -> bool {
        self.scopes.last().is_some_and(|f| f.vars.get(name).is_some_and(|m| m == to))
    }

    /// Make `name` refer to the zsh parameter `to` in the innermost scope.
    pub(super) fn map_local(&mut self, name: &str, to: &str) {
        if let Some(f) = self.scopes.last_mut() {
            f.vars.insert(name.to_string(), to.to_string());
        }
    }

    /// Is a `local` declaration legal here? Inside a function, or in a sourced file,
    /// which `source` wraps in an anonymous function so its locals die with it.
    pub(super) fn in_local_scope(&self) -> bool {
        self.fn_depth > 0 || super::source_mode()
    }

    /// Blocks open since the enclosing function (or the script) began.
    pub(super) fn block_level(&self) -> usize {
        self.scopes.iter().rev().take_while(|f| !f.barrier).count()
    }

    /// Register `name` as local to the innermost block and return its zsh name.
    pub(super) fn declare_block_local(&mut self, name: &str) -> String {
        if self.scopes.iter().rev().take_while(|f| !f.barrier).any(|f| f.plain)
            || self.scopes.iter().rev().find(|f| f.barrier).is_some_and(|f| f.plain)
        {
            return zname(name);
        }
        if let Some(m) = self.scopes.last().and_then(|f| f.vars.get(name)) {
            return m.clone();
        }
        self.local_id += 1;
        let m = format!("_fl{}_{name}", self.local_id);
        if let Some(f) = self.scopes.last_mut() {
            f.vars.insert(name.to_string(), m.clone());
        }
        m
    }

    pub fn script(&mut self, script: &Script) -> String {
        let mut out = String::new();
        let mut chain_start = 0;
        for (n, st) in script.iter().enumerate() {
            self.cur_line = st.job.line;
            self.guarded = st.conn != Conn::Seq;
            let job = self.job(&st.job);
            self.guarded = false;
            match (n, st.conn) {
                (0, Conn::And) => out.push_str("(( ! $? )) && "),
                (0, Conn::Or) => out.push_str("(( $? )) && "),
                (0, _) => {}
                (_, Conn::Seq) => {
                    out.push('\n');
                    chain_start = out.len();
                }
                (_, Conn::And) => out.push_str(" && "),
                (_, Conn::Or) => out.push_str(" || "),
            }
            out.push_str(&job);
            if !self.pre_chain.is_empty() {
                let pre = std::mem::take(&mut self.pre_chain).join("\n") + "\n";
                out.insert_str(chain_start, &pre);
            }
        }
        out
    }

    /// A body, never empty: zsh rejects an empty `then`/`do`/`{}`.
    fn body(&mut self, s: &Script) -> String {
        self.scopes.push(Frame::default());
        let t = self.script(s);
        self.scopes.pop();
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
        // `not` inverts a status, but 127 (command not found) stays 127 in fish
        let wrap_not = j.negate && !j.bg;
        if j.negate && !wrap_not {
            out.push_str("! ");
        }
        for (n, (cmd, pipe)) in j.cmds.iter().enumerate() {
            out.push_str(&self.cmd(cmd));
            if n + 1 < j.cmds.len() {
                out.push_str(match pipe {
                    Pipe::Out => " | ",
                    Pipe::Both => " |& ",
                    Pipe::Err => " 2>&1 >/dev/null | ",
                });
            }
        }
        if j.bg {
            // (the registry is what `jobs` lists: zsh's own table is not visible in a pipeline)
            let text = squote(out.trim_start_matches("! "));
            out.push_str(&format!(" & __fish_jobreg $! {text}"));
        }
        if wrap_not {
            out = format!("{{ {out}; __fish_nrc=$?; if (( __fish_nrc == 127 )); then (exit 127); else (( __fish_nrc != 0 )); fi; }}");
        }
        if j.timed {
            // fish's `time` report, on stderr after the job
            return format!("{{ __fish_time_begin; {out}; __fish_time_end $?; }}");
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
        // (only an assignment: `set -q`, `-e` and `-S` report their own status)
        let sets_variable = matches!(c, Cmd::Simple { words, .. }
            if words.first().and_then(Word::bare) == Some("set")
                && !words[1..].iter().any(|w| w.bare().is_some_and(|t| {
                    t == "--query" || t == "--erase" || t == "--show"
                        || (t.starts_with('-') && !t.starts_with("--") && t[1..].contains(['q', 'e', 'S', 'n']))
                })));
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
                let v = var.text().map(|t| self.zn(&t)).unwrap_or_else(|| "_fish_v".into());
                let items: Vec<String> = list.iter().map(|w| self.word_list(w)).collect();
                // a loop variable is local to the function, even when a global of that name
                // exists; a list that reads the variable itself must see the outer value first
                let local_var = self.fn_depth > 0 && !v.starts_with("_fish") && !items.iter().any(|i| i.contains(&v));
                let pre = if local_var { format!("local {v}=\n") } else { String::new() };
                let text = format!(
                    "for {v} in {}\ndo\n{}\ndone{}",
                    items.join(" "),
                    self.body(body),
                    self.redirs(redirs)
                );
                let text = format!("{pre}{text}");
                if has_glob(list) {
                    null_glob_loop(&text)
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
            Cmd::Function { header, body, src, line } => self.function(header, body, src, *line),
            Cmd::WithEnv { assigns, inner } => {
                // the assignments live for the duration of the compound command
                let vars: Vec<String> = assigns.iter().filter_map(|w| self.assignment(w)).collect();
                self.fn_depth += 1;
                self.scopes.push(Frame::default());
                let text = self.cmd(inner);
                self.scopes.pop();
                self.fn_depth -= 1;
                format!("() {{\nlocal -x {}\n{text}\n}} \"$@\"", vars.join(" "))
            }
            Cmd::Begin { body, redirs } => {
                format!("{{\n{}\n}}{}", self.body(body), self.redirs(redirs))
            }
        }
    }

    /// A `case` pattern. fish compares with its wildcard matcher where only `*` is
    /// special (checked: `case "a?c"` does not match `abc`), so everything else is
    /// escaped.
    fn case_pattern(&mut self, w: &Word) -> String {
        match w.text() {
            // (an empty pattern matches the empty string; zsh's `()` is not a pattern)
            Some(t) if t.is_empty() => "''".to_string(),
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

    fn function(&mut self, header: &[Word], body: &Script, src: &str, line: usize) -> String {
        let mut name = None;
        let mut argnames: Vec<String> = Vec::new();
        let mut events: Vec<String> = Vec::new();
        let mut signals: Vec<String> = Vec::new();
        let mut watched: Vec<String> = Vec::new();
        let mut wraps: Vec<String> = Vec::new();
        let mut no_shadow = false;
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
                "-e" | "--on-event" => {
                    events.extend(header.get(i).and_then(Word::text));
                    i += 1;
                }
                "-s" | "--on-signal" => {
                    signals.extend(header.get(i).and_then(Word::text));
                    i += 1;
                }
                "-v" | "--on-variable" => {
                    watched.extend(header.get(i).and_then(Word::text));
                    i += 1;
                }
                "-w" | "--wraps" => {
                    wraps.extend(header.get(i).and_then(Word::text));
                    i += 1;
                }
                w if w.starts_with("--wraps=") => wraps.push(w["--wraps=".len()..].to_string()),
                "-d" | "--description" | "-j"
                | "--on-job-exit" | "-p" | "--on-process-exit" | "-V" | "--inherit-variable" => i += 1,
                "-S" | "--no-scope-shadowing" => no_shadow = true,
                s if s.starts_with('-') => {}
                _ if name.is_none() => name = Some(t),
                _ => {}
            }
        }
        let name = name.unwrap_or_default();
        self.fn_depth += 1;
        // the body is the function's own scope: its locals are `local`, under names no
        // function it calls can see
        // (`--no-scope-shadowing` keeps the locals of where it is defined in view)
        self.scopes.push(Frame {
            barrier: !no_shadow,
            plain: uses_dynamic_names(body),
            ..Frame::default()
        });
        let mut inner = String::new();
        for (k, a) in argnames.iter().enumerate() {
            let m = self.declare_block_local(a);
            inner.push_str(&format!("local -a {m}=(\"${{(@)argv[{},{}]}}\")\n", k + 1, k + 1));
        }
        let text = self.script(body);
        self.scopes.pop();
        inner.push_str(if text.trim().is_empty() { ":" } else { &text });
        self.fn_depth -= 1;
        let mut out = format!("function {} {{\n{inner}\n}}", quote_lit(&name));
        // what `functions NAME`, `type` and `funcsave` show: the definition as written, with
        // the header line in fish's canonical spelling
        let head = canonical_header(&header.iter().map(|w| w.text().unwrap_or_default()).collect::<Vec<_>>());
        let shown = match src.split_once('\n') {
            Some((_, rest)) if rest.trim() == "end" => format!("{head}\n\nend"),
            Some((_, rest)) => format!("{head}\n{rest}"),
            // one line, `function f; body; end`: fish shows the text between the header's
            // `;` and `end` as the body
            None => match src.split_once(';').and_then(|(_, r)| r.strip_suffix("end")) {
                // (an empty body is indented by fish)
                Some(body) if body.trim().is_empty() => format!("{head}\n    {body}\nend"),
                Some(body) => format!("{head}\n{body}\nend"),
                None => src.to_string(),
            },
        };
        out.push_str(&format!(
            "\n__fish_fnsrc[{q}]={}; __fish_fnline[{q}]={line}; __fish_fnfile[{q}]=\"${{__fish_srcfile:-${{(%):-%x}}}}\"",
            squote(&shown),
            q = quote_lit(&name)
        ));
        for e in &events {
            out.push_str(&format!("\n__fish_on_event {} {}", squote(e), quote_lit(&name)));
        }
        for v in &watched {
            out.push_str(&format!("\n__fish_on_variable {} {}", squote(v), quote_lit(&name)));
        }
        for sig in &signals {
            out.push_str(&format!("\ntrap {} {}", quote_lit(&name), quote_lit(sig)));
        }
        // fish `function --wraps`: the function completes like the wrapped command
        // (`complete_add_wrapper`)
        for w in &wraps {
            out.push_str(&format!("\n__fish_complete_wrap {} {}", quote_lit(&name), quote_lit(w)));
        }
        out
    }

    // ---- simple commands --------------------------------------------------------

    /// True when a word indexes a variable with a literal 0 (`$x[0]`, `$x[-0]`, `$x[1..0]`).
    fn zero_index(w: &Word) -> bool {
        let zero = |ix: &Option<Vec<String>>| {
            ix.as_ref().is_some_and(|toks| {
                toks.iter().any(|t| t.split("..").any(|e| matches!(e, "0" | "-0")))
            })
        };
        w.parts.iter().any(|p| match p {
            Part::Var(v) => zero(&v.index),
            Part::Double(ds) => ds.iter().any(|d| matches!(d, DPart::Var(v) if zero(&v.index))),
            _ => false,
        })
    }

    fn simple(&mut self, words: &[Word], redirs: &[Redir]) -> String {
        // `$list[0]`: fish refuses to run the command (status 121)
        if words.iter().any(Self::zero_index) {
            return "{ print -u2 -r -- 'fish: array indices start at 1, not 0.'; (exit 121); }".to_string();
        }
        // `NAME=value… cmd` is fish's `begin; set -lx NAME value…; cmd; end`: the
        // value expands to a list (`files=$dir/* cmd`) and the command's own words
        // see it (`files=… string join \n $files`), neither of which a zsh prefix
        // assignment does
        let n = words.iter().take_while(|w| Self::assignment_parts(w).is_some()).count();
        if n > 0 && n < words.len() {
            let lit = |s: &str| Word {
                parts: vec![Part::Lit(s.to_string())],
            };
            self.fn_depth += 1;
            self.scopes.push(Frame::default());
            let sets: Vec<String> = words[..n]
                .iter()
                .filter_map(Self::assignment_parts)
                .map(|(name, value)| {
                    // `NAME= cmd` is the empty string
                    let value = if value.parts.is_empty() { Word { parts: vec![Part::Single(String::new())] } } else { value };
                    self.set_cmd(&[lit("-lx"), lit(&name), value])
                })
                .collect();
            let text = self.simple(&words[n..], redirs);
            self.scopes.pop();
            self.fn_depth -= 1;
            return format!("() {{\n{}\n{text}\n}} \"$@\"", sets.join("\n"));
        }
        // leading `NAME=value` words are environment assignments for this command
        let mut prefix = Vec::new();
        let mut k = 0;
        while let Some(a) = words.get(k).and_then(|w| self.assignment(w)) {
            prefix.push(a);
            k += 1;
        }
        let rest = &words[k..];
        let name = rest.first().and_then(|w| w.bare()).map(str::to_string);
        let lenient_glob = matches!(name.as_deref(), Some("set" | "count"));
        self.null_glob = lenient_glob;
        let mut out = match name.as_deref() {
            Some("exit") if super::source_mode() && self.fn_depth == 0 => {
                let tail: Vec<String> = rest[1..].iter().map(|w| self.word_list(w)).collect();
                format!("return {}", tail.join(" "))
            }
            Some("status")
                if matches!(
                    rest.get(1).and_then(|w| w.bare()),
                    Some("line-number" | "current-line-number" | "-n")
                ) =>
            {
                format!("print -r -- {}", self.cur_line)
            }
            Some("set") => self.set_cmd(&rest[1..]),
            Some("read") => self.read_cmd(&rest[1..]),
            Some("alias") => self.alias_cmd(&rest[1..]),
            Some("abbr") => {
                let all: Vec<String> = rest[1..].iter().map(|w| self.word_list(w)).collect();
                format!("__fish_abbr {}", all.join(" "))
            }
            // fish's bare `test` is a usage error (status 2); zsh's is false (1)
            Some("test") if rest.len() == 1 => "__fish_ret 2".to_string(),
            // argparse sets the caller's `argv`, which a function cannot do: the parse
            // leaves the operands in `__fish_argv` and the caller's own `set --` takes them
            Some("argparse") => {
                let all: Vec<String> = rest[1..].iter().map(|w| self.word_list(w)).collect();
                format!(
                    "{{ __fish_argparse {}; __fish_cs=$?; set -- \"${{__fish_argv[@]}}\"; argv_opts=(\"${{__fish_argv_opts[@]}}\"); {}__fish_ret $__fish_cs; }}",
                    all.join(" "),
                    if self.fn_depth > 0 { "for __fn in \"${__fish_argflags[@]}\"; do __fv=(\"${(@P)__fn}\"); unset $__fn; builtin eval \"local -a $__fn=(\\\"\\${__fv[@]}\\\")\"; done; " } else { "" }
                )
            }
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
                    .is_some_and(|t| t.starts_with('-'));
                if name.as_deref() == Some("echo") && flagged {
                    all[0] = "__fish_echo".into();
                }
                // `command -s NAME` / `-q` / `-v` / `-a` look commands up; zsh's `command` has no -s
                if name.as_deref() == Some("command")
                    && rest.get(1).and_then(|w| w.bare()).is_some_and(|t| {
                        t.len() > 1 && t.starts_with('-') && !t.starts_with("--") && t[1..].chars().all(|c| "sqva".contains(c))
                    })
                {
                    all[0] = "__fish_command_query".into();
                }
                // `builtin -n` lists fish's builtins, `builtin -q NAME` asks about one
                if name.as_deref() == Some("builtin") {
                    match rest.get(1).and_then(|w| w.bare()) {
                        Some("-n" | "--names") => all = vec!["__fish_builtin_names".to_string()],
                        Some("-q" | "--query") => {
                            let tail: Vec<String> = all.iter().skip(2).cloned().collect();
                            all = std::iter::once("__fish_builtin_q".to_string()).chain(tail).collect();
                        }
                        _ => {}
                    }
                }
                // fish's printf converts numbers its own way; the native one follows it
                if name.as_deref() == Some("printf") {
                    all[0] = "__fish_printf".into();
                }
                // zshrs resolves these names before a function of the same name
                if let Some(n @ ("bind" | "commandline" | "complete" | "eval")) = name.as_deref() {
                    all[0] = format!("__fish_{n}");
                }
                all.join(" ")
            }
        };
        self.null_glob = false;
        if name.as_deref() == Some("string") {
            if let Some(map) = self.capture_map(&rest[1..]) {
                out = format!("__fish_capmap={} {out}", quote_lit(&map));
            }
        }
        if !prefix.is_empty() {
            out = format!("{} {out}", prefix.join(" "));
        }
        out.push_str(&self.redirs(redirs));
        // A wildcard with no match is an error in both shells, but zsh ends the whole
        // script on it and fish only skips the command. `eval` confines the abort.
        if !lenient_glob && has_glob(&rest[1.min(rest.len())..]) {
            out = isolate_glob(&out);
        }
        out
    }

    /// `string match -r PATTERN …` sets a variable per named group of PATTERN
    /// (`(?<name>…)`), which the native `string` does by name. A group named
    /// after a variable this code holds under another zsh name (a `set -l`
    /// local) goes to it as `name=zshname` in `__fish_capmap` — fish's own
    /// `__fish_anypython` declares `set -l minor_version` and fills it with
    /// `(?<minor_version>\d+)`. Only a literal pattern can be read here.
    fn capture_map(&self, args: &[Word]) -> Option<String> {
        let (sub, args) = args.split_first()?;
        if sub.bare() != Some("match") {
            return None;
        }
        let mut regex = false;
        let mut pattern = None;
        for (i, w) in args.iter().enumerate() {
            let t = w.text()?;
            match t.as_str() {
                "--" => {
                    pattern = args.get(i + 1);
                    break;
                }
                "--regex" => regex = true,
                f if f.starts_with("--") => {}
                f if f.starts_with('-') && f.len() > 1 => regex |= f.contains('r'),
                _ => {
                    pattern = Some(w);
                    break;
                }
            }
        }
        let pattern = pattern?.text()?;
        if !regex {
            return None;
        }
        let mut map = Vec::new();
        for start in ["(?<", "(?P<"] {
            for (at, _) in pattern.match_indices(start) {
                let rest = &pattern[at + start.len()..];
                let Some(name) = rest.split_once('>').map(|(n, _)| n) else { continue };
                let z = self.zn(name);
                if z != zname(name) {
                    map.push(format!("{name}={z}"));
                }
            }
        }
        (!map.is_empty()).then(|| map.join(" "))
    }

    /// `NAME=value`: the name and the value word, or `None` for any other word.
    fn assignment_parts(w: &Word) -> Option<(String, Word)> {
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
        Some((name.to_string(), tail))
    }

    /// `NAME=value` as an environment assignment word.
    fn assignment(&mut self, w: &Word) -> Option<String> {
        let (name, tail) = Self::assignment_parts(w)?;
        let name = name.as_str();
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
        let text = self.word_list_inner(w);
        if self.null_glob && has_glob(std::slice::from_ref(w)) && !text.starts_with('"') {
            format!("{text}(N)")
        } else {
            text
        }
    }

    fn word_list_inner(&mut self, w: &Word) -> String {
        // Brace groups are expanded here, not by zsh: fish varies the FIRST group fastest
        // (`{+,-}{a,b}` is `+a -a +b -b`), zsh the last.
        if w.parts.iter().any(|p| matches!(p, Part::Raw(r) if r == "{")) {
            let words: Vec<String> = expand_braces(&w.parts)
                .into_iter()
                .map(|parts| self.word_list_inner(&Word { parts }))
                .collect();
            return words.join(" ");
        }
        match w.parts.as_slice() {
            [] => "''".into(),
            [Part::Var(v)] => self.var_list(v),
            [Part::Cmd(s, idx)] => self.cmd_list(s, idx.as_deref()),
            // a list part makes the word as many words as it has elements: none for an
            // unset variable, which only a loop can express
            parts if parts.iter().any(|p| is_list_part(p) && !always_set(p)) => self.cartesian(parts),
            parts => parts.iter().map(|p| self.part(p, false)).collect(),
        }
    }

    /// A word with two or more list parts. fish varies the first list fastest
    /// (`$a$b` with a=(1 2) b=(x y) is `1x 2x 1y 2y`); zsh's `${^a}${^b}` varies the last,
    /// so the product is built by nested loops, first part innermost.
    fn cartesian(&mut self, parts: &[Part]) -> String {
        self.hoist_id += 1;
        let id = self.hoist_id;
        let mut word = String::new();
        let mut loops: Vec<String> = Vec::new();
        for p in parts {
            if is_list_part(p) {
                let var = format!("__fi{id}_{}", loops.len());
                let list = match p {
                    Part::Var(v) => self.var_list(v),
                    Part::Cmd(s, idx) => self.cmd_list(s, idx.as_deref()),
                    _ => String::new(),
                };
                loops.push(format!("for {var} in {list}; do"));
                word.push_str(&format!("\"${{{var}}}\""));
            } else {
                word.push_str(&self.part(p, false));
            }
        }
        if self.null_glob && parts.iter().any(|p| matches!(p, Part::Raw(r) if r.starts_with('*'))) {
            word.push_str("(N)");
        }
        let v = format!("__fc{id}");
        let mut text = format!("{v}=(); ");
        for l in loops.iter().rev() {
            text.push_str(l);
            text.push(' ');
        }
        text.push_str(&format!("{v}+=({word}); "));
        text.push_str(&vec!["done"; loops.len()].join("; "));
        self.hoists.push(text);
        format!("\"${{{v}[@]}}\"")
    }

    /// `$x[a..b]` where the direction is only known at run time (`-1..1`, `$i..$j`):
    /// the slice is built by `__fish_slice` into a hoisted array.
    fn dynamic_slice(&mut self, z: &str, a: &str, b: &str) -> String {
        self.hoist_id += 1;
        let v = format!("__fc{}", self.hoist_id);
        // `argv` is the positional parameters, which the helper cannot reach by name
        let src = if z == "argv" { format!("- {a} {b} \"$@\"") } else { format!("{z} {a} {b}") };
        self.hoists.push(format!("__fish_slice {v} {src}"));
        v
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
        // The output is captured with a trailing `x` so `$(…)` cannot strip its final
        // newlines: fish drops only ONE trailing newline and keeps empty lines, so
        // `(echo)` is one empty element and `(printf '')` is none.
        self.hoists.push(format!(
            "{v}=\"$( {inner}\n__fish_cs=$?; print -rn x; exit $__fish_cs )\""
        ));
        let out = format!("${{{v}%x}}");
        let list = format!("${{{out}:+\"${{(@f)${{{out}%$'\\n'}}}}\"}}");
        let Some(toks) = idx else { return list };
        // an index counts elements, so the list is made an array first
        self.hoist_id += 1;
        let arr = format!("__fa{}", self.hoist_id);
        self.hoists.push(format!("{arr}=({list})"));
        self.index_words(&arr, toks)
    }

    /// The words `$name[idx…]` stands for, `z` being the zsh array that holds the list.
    fn index_words(&mut self, z: &str, idx: &[String]) -> String {
        let idx = self.index_tokens(idx);
        idx.iter()
            .map(|t| match dyn_range(t) {
                Some((a, b)) => format!("\"${{{}[@]}}\"", self.dynamic_slice(z, a, b)),
                None => {
                    let (r, rev) = range_of(t);
                    format!("\"${{(@{}){z}[{r}]}}\"", if rev { "Oa" } else { "" })
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
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
                    None => self.map_index_vars(t),
                }
            })
            .collect()
    }

    /// `$name` inside an index token, renamed to the zsh parameter that holds it.
    fn map_index_vars(&self, tok: &str) -> String {
        let mut out = String::new();
        let mut chars = tok.chars().peekable();
        while let Some(c) = chars.next() {
            out.push(c);
            if c == '$' {
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    if n.is_alphanumeric() || n == '_' {
                        name.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                out.push_str(&if name.is_empty() { name } else { self.zn(&name) });
            }
        }
        out
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
        let z = self.zn(n);
        if v.deref {
            // the variable named by `$name` (its first element, when it is a list)
            return format!("\"{}\"", deref_expr(&z, "@P"));
        }
        if let Some(idx) = &v.index {
            return self.index_words(&z, idx);
        }
        if is_colon_scalar(n) && !z.starts_with("__fx_") {
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
        let z = self.zn(n);
        if v.deref || v.index.is_some() {
            return format!("\"{}\"", self.var_join(v));
        }
        if is_colon_scalar(n) && !z.starts_with("__fx_") {
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
        let z = self.zn(n);
        if v.deref {
            return deref_expr(&z, "P");
        }
        if let Some(idx) = &v.index {
            let idx = self.index_tokens(idx);
            let parts: Vec<String> = idx
                .iter()
                .map(|t| match dyn_range(t) {
                    Some((a, b)) => format!("${{(j: :){}}}", self.dynamic_slice(&z, a, b)),
                    None => {
                        let (r, rev) = range_of(t);
                        format!("${{({}j: :){z}[{r}]}}", if rev { "Oa" } else { "" })
                    }
                })
                .collect();
            return parts.join(" ");
        }
        if is_colon_scalar(n) && !z.starts_with("__fx_") {
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
        // fish's `$_` is empty outside an interactive command
        "_" => Some("\"\""),
        _ => None,
    }
}

/// One fish index token (`1`, `-1`, `2..4`, `$i`) as a zsh subscript range. A
/// descending range of plain numbers is returned with `true` so the caller reverses it.
fn range_of(t: &str) -> (String, bool) {
    match t.split_once("..") {
        Some((a, b)) => match (a.parse::<i64>(), b.parse::<i64>()) {
            _ if a.is_empty() || b.is_empty() => (format!("{},{}", if a.is_empty() { "1" } else { a }, if b.is_empty() { "-1" } else { b }), false),
            (Ok(x), Ok(y)) if x > y && (x < 0) == (y < 0) => (format!("{y},{x}"), true),
            _ => (format!("{a},{b}"), false),
        },
        None => (format!("{t},{t}"), false),
    }
}

/// An `a..b` index that `__fish_slice` has to resolve at run time: fish counts negative
/// indices from the end, reverses a range whose start is after its end, and returns nothing
/// when either end falls before the first element or the start is past the last. Only a
/// range goes through the helper.
fn dyn_range(t: &str) -> Option<(&str, &str)> {
    let (a, b) = t.split_once("..")?;
    let a = if a.is_empty() { "1" } else { a };
    let b = if b.is_empty() { "-1" } else { b };
    // (zsh's own `x[4,5]` on a three-element list is one empty word, not none, so even a
    // plain ascending range goes through the helper)
    Some((a, b))
}

/// Variables every shell session has, so `$HOME/x` need not pay for a loop.
fn always_set(p: &Part) -> bool {
    matches!(p, Part::Var(v) if matches!(v.name.as_str(), "HOME" | "PWD" | "OLDPWD" | "USER" | "SHELL"))
}

/// Does the part expand to a list that multiplies the word it is in?
fn is_list_part(p: &Part) -> bool {
    match p {
        Part::Var(v) => {
            !v.deref && special_scalar(&v.name).is_none() && valid_param(&v.name)
        }
        Part::Cmd(_, idx) => idx.is_none(),
        _ => false,
    }
}

/// fish index tokens as zsh subscript ranges.
fn ranges(tokens: &[String]) -> Vec<(String, bool)> {
    tokens.iter().map(|t| range_of(t)).collect()
}

/// Does any word contain an unquoted wildcard?
fn has_glob(words: &[Word]) -> bool {
    words.iter().any(|w| {
        w.parts
            .iter()
            .any(|p| matches!(p, Part::Raw(r) if r.starts_with('*')))
    })
}

/// Run `text` so that a wildcard with no match ends only this command, with status
/// 124 as in fish; zsh would end the whole script. The marker is set after the command
/// ran, so a failed expansion (which skips it) is told from a command that failed.
fn isolate_glob(text: &str) -> String {
    format!(
        "{{ __fish_g=; builtin eval {}; [[ -n $__fish_g ]] || __fish_cs=124; __fish_ret $__fish_cs; }}",
        squote(&format!("{text}; __fish_cs=$?; __fish_g=1"))
    )
}

/// A `for` list whose wildcard matches nothing is an empty list in fish, not an error.
fn null_glob_loop(text: &str) -> String {
    format!("() {{ setopt local_options null_glob; {text}\n}} \"$@\"")
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

/// Does the script name a variable at run time (`set $name …`, `read $name`, `$$name`)?
/// Such a function keeps its locals under their own names so the run-time name finds them.
pub(super) fn uses_dynamic_names(script: &Script) -> bool {
    fn word_dynamic(w: &Word) -> bool {
        w.parts.iter().any(|p| match p {
            Part::Var(v) => v.deref,
            Part::Double(ds) => ds.iter().any(|d| matches!(d, DPart::Var(v) if v.deref)),
            Part::Cmd(s, _) => uses_dynamic_names(s),
            _ => false,
        })
    }
    fn cmd_dynamic(c: &Cmd) -> bool {
        match c {
            Cmd::Simple { words, .. } => {
                let named = matches!(words.first().and_then(Word::bare), Some("set" | "read"))
                    && words[1..]
                        .iter()
                        .find(|w| !w.bare().is_some_and(|t| t.starts_with('-')))
                        .is_some_and(|w| w.text().is_none() && !matches!(w.parts.first(), Some(Part::Raw(_))));
                // `eval` runs code that names the caller's locals
                named || words.first().and_then(Word::bare) == Some("eval") || words.iter().any(word_dynamic)
            }
            Cmd::If { arms, els, .. } => {
                arms.iter().any(|(c, b)| script_dynamic(c) || uses_dynamic_names(b))
                    || els.as_ref().is_some_and(uses_dynamic_names)
            }
            Cmd::While { cond, body, .. } => script_dynamic(cond) || uses_dynamic_names(body),
            Cmd::For { list, body, .. } => list.iter().any(word_dynamic) || uses_dynamic_names(body),
            Cmd::Switch { cases, .. } => cases.iter().any(|(_, b)| uses_dynamic_names(b)),
            Cmd::Begin { body, .. } => uses_dynamic_names(body),
            Cmd::WithEnv { inner, .. } => cmd_dynamic(inner),
            Cmd::Function { .. } => false,
        }
    }
    fn script_dynamic(stmts: &[Stmt]) -> bool {
        stmts.iter().any(|st| st.job.cmds.iter().any(|(c, _)| cmd_dynamic(c)))
    }
    script_dynamic(script)
}

/// Expand the brace groups of a word (`Raw("{")` … `Raw(",")` … `Raw("}")`) into the words
/// they stand for, the first group varying fastest.
fn expand_braces(parts: &[Part]) -> Vec<Vec<Part>> {
    let is = |p: &Part, t: &str| matches!(p, Part::Raw(r) if r == t);
    // each segment is a list of alternatives; plain text is a segment with one
    let mut segments: Vec<Vec<Vec<Part>>> = Vec::new();
    let mut plain: Vec<Part> = Vec::new();
    let mut i = 0;
    while i < parts.len() {
        if !is(&parts[i], "{") {
            plain.push(parts[i].clone());
            i += 1;
            continue;
        }
        // find the matching `}` and the top-level commas
        let (mut depth, mut j) = (0usize, i);
        let mut cuts = vec![i];
        while j < parts.len() {
            if is(&parts[j], "{") {
                depth += 1;
            } else if is(&parts[j], "}") {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else if depth == 1 && is(&parts[j], ",") {
                cuts.push(j);
            }
            j += 1;
        }
        if j >= parts.len() {
            // unbalanced: leave the rest as it is
            plain.extend_from_slice(&parts[i..]);
            break;
        }
        cuts.push(j);
        if !plain.is_empty() {
            segments.push(vec![std::mem::take(&mut plain)]);
        }
        let mut options: Vec<Vec<Part>> = Vec::new();
        for pair in cuts.windows(2) {
            options.extend(expand_braces(&parts[pair[0] + 1..pair[1]]));
        }
        segments.push(options);
        i = j + 1;
    }
    if !plain.is_empty() || segments.is_empty() {
        segments.push(vec![plain]);
    }
    let mut acc: Vec<Vec<Part>> = vec![Vec::new()];
    for seg in segments.iter().rev() {
        acc = acc
            .iter()
            .flat_map(|later| {
                seg.iter().map(move |opt| {
                    let mut w = opt.clone();
                    w.extend(later.iter().cloned());
                    w
                })
            })
            .collect();
    }
    acc
}

/// A word as fish prints it back: bare when plain, single-quoted otherwise.
pub(super) fn fish_escape(s: &str) -> String {
    let plain = !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_./:@%+,=-".contains(c));
    if plain {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\\', "\\\\").replace('\'', "\\'"))
    }
}

/// `${(P)…}` of the variable named by the value of `z`, through the map of fish names to
/// the zsh parameters that hold them (`PATH` lives in `path`, an exported list in `__fx_NAME`).
fn deref_expr(z: &str, flags: &str) -> String {
    let n = format!("${{(j: :){z}}}");
    let fx = format!("${{parameters[__fx_{n}]:+__fx_{n}}}");
    format!("${{({flags})${{__fish_zmap[{n}]:-${{{fx}:-{n}}}}}}}")
}

/// The `function NAME …` line as fish prints a definition back: the name, then the options
/// in fish's fixed order and long spelling.
fn canonical_header(words: &[String]) -> String {
    let (mut wraps, mut desc, mut noshadow) = (Vec::new(), None, false);
    let (mut events, mut vars, mut sigs, mut jobs, mut procs, mut argn) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut name = None;
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        i += 1;
        let (opt, inline) = match w.split_once('=') {
            Some((o, v)) if o.starts_with("--") => (o, Some(v.to_string())),
            _ => (w, None),
        };
        let mut value = |i: &mut usize| {
            inline.clone().or_else(|| {
                let v = words.get(*i).cloned();
                *i += 1;
                v
            })
        };
        match opt {
            "-d" | "--description" => desc = value(&mut i),
            "-w" | "--wraps" => wraps.extend(value(&mut i)),
            "-e" | "--on-event" => events.extend(value(&mut i)),
            "-v" | "--on-variable" => vars.extend(value(&mut i)),
            "-j" | "--on-job-exit" => jobs.extend(value(&mut i)),
            "-p" | "--on-process-exit" => procs.extend(value(&mut i)),
            "-s" | "--on-signal" => sigs.extend(value(&mut i).map(|v| {
                if v.chars().all(|c| c.is_ascii_alphabetic()) && !v.starts_with("SIG") {
                    format!("SIG{v}")
                } else {
                    v
                }
            })),
            "-V" | "--inherit-variable" => {
                value(&mut i);
            }
            "-S" | "--no-scope-shadowing" => noshadow = true,
            "-a" | "--argument-names" => {
                while i < words.len() && !words[i].starts_with('-') {
                    argn.push(words[i].clone());
                    i += 1;
                }
            }
            _ if name.is_none() => name = Some(w.to_string()),
            _ => {}
        }
    }
    let mut head = String::from("function");
    if let Some(n) = &name {
        head.push(' ');
        head.push_str(&fish_escape(n));
    }
    let opt = |head: &mut String, flag: &str, v: &str| head.push_str(&format!(" {flag}{}", fish_escape(v)));
    for v in &wraps {
        opt(&mut head, "--wraps=", v);
    }
    if let Some(d) = &desc {
        opt(&mut head, "--description ", d);
    }
    if noshadow {
        head.push_str(" --no-scope-shadowing");
    }
    for (flag, list) in [("--on-event ", &events), ("--on-variable ", &vars), ("--on-signal ", &sigs), ("--on-job-exit ", &jobs), ("--on-process-exit ", &procs)] {
        for v in list {
            opt(&mut head, flag, v);
        }
    }
    if !argn.is_empty() {
        head.push_str(" --argument-names");
        for a in &argn {
            head.push(' ');
            head.push_str(&fish_escape(a));
        }
    }
    head
}
