//! fish source → AST.
//!
//! One pass over the characters: fish has no separate token stream because
//! its quoting is context sensitive (`(` starts a command substitution in a
//! bare word but not inside `'…'` or `"…"`; `[` is an index only directly
//! after `$name`). Everything here follows the fish 4 grammar as documented
//! in `fish-doc` (`language`, `cmds/*`) and checked against /opt/homebrew/bin/fish.

/// `$name`, `$$name`, `$name[idx…]`.
#[derive(Debug, Clone, PartialEq)]
pub struct VarRef {
    pub name: String,
    /// `$$name`: the value of the variable named by `$name`.
    pub deref: bool,
    /// Whitespace-separated tokens between `[` and `]`, raw (`1`, `-1`, `2..4`, `$i`).
    pub index: Option<Vec<String>>,
}

/// One piece of a double-quoted string.
#[derive(Debug, Clone, PartialEq)]
pub enum DPart {
    Lit(String),
    Var(VarRef),
    Cmd(Script),
}

/// One piece of a word; a word is the concatenation of its parts.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    /// Unquoted text, escapes resolved. Literal in fish, so the emitter quotes it for zsh.
    Lit(String),
    /// zsh syntax passed through unchanged: `*`, `**`, brace-expansion `{`,`,`,`}`, leading `~`.
    Raw(String),
    /// `'…'` with `\\` and `\'` resolved.
    Single(String),
    /// `"…"`.
    Double(Vec<DPart>),
    /// Unquoted `$name`.
    Var(VarRef),
    /// `(cmd)` or `$(cmd)`, optionally indexed: `(cmd)[1]`.
    Cmd(Script, Option<Vec<String>>),
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Word {
    pub parts: Vec<Part>,
}

impl Word {
    /// The text of a word with no expansion in it (`foo`, `'foo'`, `"foo"`, `fo\o`).
    pub fn text(&self) -> Option<String> {
        let mut out = String::new();
        for p in &self.parts {
            match p {
                Part::Lit(s) | Part::Single(s) => out.push_str(s),
                Part::Double(ds) => {
                    for d in ds {
                        match d {
                            DPart::Lit(s) => out.push_str(s),
                            _ => return None,
                        }
                    }
                }
                _ => return None,
            }
        }
        Some(out)
    }

    /// The text of a word made only of unquoted literal characters. Keywords are bare
    /// words: `'if'` is a command named `if`, not the keyword.
    pub fn bare(&self) -> Option<&str> {
        match self.parts.as_slice() {
            [Part::Lit(s)] => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Redir {
    pub fd: Option<u32>,
    /// `>`, `>>`, `<`, `>&`, `<&`, `>?`, `&>`, `&>>`.
    pub op: String,
    pub target: Word,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Cmd {
    Simple {
        words: Vec<Word>,
        redirs: Vec<Redir>,
    },
    If {
        /// `(condition, body)` per `if` / `else if`.
        arms: Vec<(Vec<Stmt>, Script)>,
        els: Option<Script>,
        redirs: Vec<Redir>,
    },
    While {
        cond: Vec<Stmt>,
        body: Script,
        redirs: Vec<Redir>,
    },
    For {
        var: Word,
        list: Vec<Word>,
        body: Script,
        redirs: Vec<Redir>,
    },
    Switch {
        subject: Word,
        cases: Vec<(Vec<Word>, Script)>,
        redirs: Vec<Redir>,
    },
    Function {
        /// Name and options exactly as written.
        header: Vec<Word>,
        body: Script,
    },
    Begin {
        body: Script,
        redirs: Vec<Redir>,
    },
}

/// How a job is joined to the one before it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Conn {
    /// Separator (newline or `;`).
    Seq,
    /// `&&` or the `and` modifier.
    And,
    /// `||` or the `or` modifier.
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pipe {
    /// `|`
    Out,
    /// `&|`
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub negate: bool,
    /// Each command and the pipe that follows it (the last one's is unused).
    pub cmds: Vec<(Cmd, Pipe)>,
    pub bg: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub conn: Conn,
    pub job: Job,
}

pub type Script = Vec<Stmt>;

type PResult<T> = Result<T, String>;

pub struct Parser {
    s: Vec<char>,
    i: usize,
}

/// Parse a whole fish script.
pub fn parse(src: &str) -> PResult<Script> {
    let mut p = Parser {
        s: src.chars().collect(),
        i: 0,
    };
    let script = p.script(&[])?;
    match p.peek_kw() {
        Some(kw) => Err(format!("fish: '{kw}' outside of a block")),
        None if p.i < p.s.len() => Err("fish: Expected a command, but found ')'".to_string()),
        None => Ok(script),
    }
}

const WORD_END: &[char] = &[' ', '\t', '\n', ';', '|', '&', '<', '>', ')'];

impl Parser {
    fn peek(&self) -> Option<char> {
        self.s.get(self.i).copied()
    }

    fn at(&self, off: usize) -> Option<char> {
        self.s.get(self.i + off).copied()
    }

    fn starts_with(&self, lit: &str) -> bool {
        lit.chars().enumerate().all(|(k, c)| self.at(k) == Some(c))
    }

    /// Spaces, tabs and `\<newline>` continuations.
    fn skip_blank(&mut self) {
        loop {
            match self.peek() {
                Some(' ' | '\t') => self.i += 1,
                Some('\\') if self.at(1) == Some('\n') => self.i += 2,
                _ => break,
            }
        }
    }

    fn skip_comment(&mut self) {
        while !matches!(self.peek(), None | Some('\n')) {
            self.i += 1;
        }
    }

    /// Blank space, newlines, `;` and comments between statements.
    fn skip_separators(&mut self) {
        loop {
            self.skip_blank();
            match self.peek() {
                Some('\n' | ';') => self.i += 1,
                Some('#') => self.skip_comment(),
                _ => break,
            }
        }
    }

    /// The bare word at the cursor, without consuming it.
    fn peek_kw(&self) -> Option<String> {
        let mut j = self.i;
        let mut w = String::new();
        while let Some(&c) = self.s.get(j) {
            if WORD_END.contains(&c) || matches!(c, '(' | '\'' | '"' | '$' | '\\' | '#') {
                break;
            }
            w.push(c);
            j += 1;
        }
        let kw = matches!(
            w.as_str(),
            "if" | "else" | "end" | "for" | "while" | "switch" | "case" | "function" | "begin"
        );
        kw.then_some(w)
    }

    fn eat_kw(&mut self, kw: &str) {
        self.i += kw.chars().count();
    }

    /// Statements until EOF or one of the `terms` keywords (left unconsumed).
    fn script(&mut self, terms: &[&str]) -> PResult<Script> {
        let mut out = Vec::new();
        let mut conn = Conn::Seq;
        loop {
            self.skip_separators();
            if self.peek().is_none() || self.peek() == Some(')') {
                break;
            }
            if let Some(kw) = self.peek_kw() {
                if terms.contains(&kw.as_str()) {
                    break;
                }
            }
            match self.peek_word_text().as_deref() {
                Some("and") => {
                    self.i += 3;
                    conn = Conn::And;
                    continue;
                }
                Some("or") => {
                    self.i += 2;
                    conn = Conn::Or;
                    continue;
                }
                _ => {}
            }
            let job = self.job()?;
            out.push(Stmt { conn, job });
            conn = Conn::Seq;
            // `a && b || c` on one line.
            self.skip_blank();
            if self.starts_with("&&") {
                self.i += 2;
                conn = Conn::And;
                self.skip_blank_nl();
            } else if self.starts_with("||") {
                self.i += 2;
                conn = Conn::Or;
                self.skip_blank_nl();
            }
        }
        Ok(out)
    }

    fn skip_blank_nl(&mut self) {
        loop {
            self.skip_blank();
            match self.peek() {
                Some('\n') => self.i += 1,
                Some('#') => self.skip_comment(),
                _ => break,
            }
        }
    }

    /// The unquoted literal word at the cursor (`and`, `or`, `not`, `in`), unconsumed.
    fn peek_word_text(&self) -> Option<String> {
        let mut j = self.i;
        let mut w = String::new();
        while let Some(&c) = self.s.get(j) {
            if WORD_END.contains(&c) || matches!(c, '(' | '\'' | '"' | '$' | '\\') {
                // a word like `and` ends only at a real delimiter
                if w.is_empty() || matches!(c, '(' | '\'' | '"' | '$' | '\\') {
                    return None;
                }
                break;
            }
            w.push(c);
            j += 1;
        }
        (!w.is_empty()).then_some(w)
    }

    /// `and` / `or` chained onto a condition (`if a; and b`).
    fn condition(&mut self) -> PResult<Vec<Stmt>> {
        let mut out = vec![Stmt {
            conn: Conn::Seq,
            job: self.job()?,
        }];
        loop {
            let save = self.i;
            self.skip_separators();
            let conn = match self.peek_word_text().as_deref() {
                Some("and") => Conn::And,
                Some("or") => Conn::Or,
                _ => {
                    self.i = save;
                    return Ok(out);
                }
            };
            self.i += if conn == Conn::And { 3 } else { 2 };
            self.skip_blank();
            out.push(Stmt {
                conn,
                job: self.job()?,
            });
        }
    }

    fn job(&mut self) -> PResult<Job> {
        let mut negate = false;
        loop {
            self.skip_blank();
            match self.peek_word_text().as_deref() {
                Some("not" | "!") => {
                    negate = !negate;
                    self.i += if self.peek() == Some('!') { 1 } else { 3 };
                }
                _ => break,
            }
        }
        let mut cmds = Vec::new();
        let bg;
        loop {
            let cmd = self.command()?;
            self.skip_blank();
            let pipe = if self.starts_with("&|") {
                self.i += 2;
                Some(Pipe::Both)
            } else if self.peek() == Some('|') && self.at(1) != Some('|') {
                self.i += 1;
                Some(Pipe::Out)
            } else {
                None
            };
            match pipe {
                Some(p) => {
                    cmds.push((cmd, p));
                    self.skip_blank_nl();
                }
                None => {
                    cmds.push((cmd, Pipe::Out));
                    bg = self.peek() == Some('&')
                        && !matches!(self.at(1), Some('&' | '>' | '|'));
                    if bg {
                        self.i += 1;
                    }
                    break;
                }
            }
        }
        Ok(Job { negate, cmds, bg })
    }

    fn command(&mut self) -> PResult<Cmd> {
        self.skip_blank();
        let kw = self.peek_kw();
        match kw.as_deref() {
            Some("if") => self.cmd_if(),
            Some("while") => {
                self.eat_kw("while");
                let cond = self.condition()?;
                let body = self.script(&["end"])?;
                self.expect_end()?;
                Ok(Cmd::While {
                    cond,
                    body,
                    redirs: self.trailing_redirs()?,
                })
            }
            Some("for") => self.cmd_for(),
            Some("switch") => self.cmd_switch(),
            Some("function") => {
                self.eat_kw("function");
                let header = self.words_to_eol()?;
                let body = self.script(&["end"])?;
                self.expect_end()?;
                Ok(Cmd::Function { header, body })
            }
            Some("begin") => {
                self.eat_kw("begin");
                let body = self.script(&["end"])?;
                self.expect_end()?;
                Ok(Cmd::Begin {
                    body,
                    redirs: self.trailing_redirs()?,
                })
            }
            Some(other) => Err(format!("fish: '{other}' outside of a block")),
            None => self.simple(),
        }
    }

    fn expect_end(&mut self) -> PResult<()> {
        self.skip_separators();
        if self.peek_kw().as_deref() == Some("end") {
            self.eat_kw("end");
            Ok(())
        } else {
            Err("fish: Missing end to balance this block".to_string())
        }
    }

    fn cmd_if(&mut self) -> PResult<Cmd> {
        self.eat_kw("if");
        let mut arms = Vec::new();
        let mut els = None;
        let mut cond = self.condition()?;
        loop {
            let body = self.script(&["else", "end"])?;
            arms.push((cond, body));
            self.skip_separators();
            if self.peek_kw().as_deref() == Some("else") {
                self.eat_kw("else");
                self.skip_blank();
                if self.peek_kw().as_deref() == Some("if") {
                    self.eat_kw("if");
                    cond = self.condition()?;
                    continue;
                }
                els = Some(self.script(&["end"])?);
            }
            break;
        }
        self.expect_end()?;
        Ok(Cmd::If {
            arms,
            els,
            redirs: self.trailing_redirs()?,
        })
    }

    fn cmd_for(&mut self) -> PResult<Cmd> {
        self.eat_kw("for");
        self.skip_blank();
        let var = self.word()?.ok_or("fish: for: Expected a variable name")?;
        self.skip_blank();
        if self.peek_word_text().as_deref() != Some("in") {
            return Err("fish: for: Expected the word 'in'".to_string());
        }
        self.i += 2;
        let list = self.words_to_eol()?;
        let body = self.script(&["end"])?;
        self.expect_end()?;
        Ok(Cmd::For {
            var,
            list,
            body,
            redirs: self.trailing_redirs()?,
        })
    }

    fn cmd_switch(&mut self) -> PResult<Cmd> {
        self.eat_kw("switch");
        self.skip_blank();
        let subject = self.word()?.ok_or("fish: switch: Expected exactly one argument")?;
        let mut cases = Vec::new();
        loop {
            self.skip_separators();
            match self.peek_kw().as_deref() {
                Some("case") => {
                    self.eat_kw("case");
                    let pats = self.words_to_eol()?;
                    let body = self.script(&["case", "end"])?;
                    cases.push((pats, body));
                }
                Some("end") => break,
                _ => return Err("fish: switch: Expected a 'case' or 'end'".to_string()),
            }
        }
        self.expect_end()?;
        Ok(Cmd::Switch {
            subject,
            cases,
            redirs: self.trailing_redirs()?,
        })
    }

    /// Words up to the end of the statement (newline or `;`), used by `for`, `case` and
    /// `function` headers.
    fn words_to_eol(&mut self) -> PResult<Vec<Word>> {
        let mut out = Vec::new();
        loop {
            self.skip_blank();
            if matches!(self.peek(), None | Some('\n' | ';')) {
                return Ok(out);
            }
            if self.peek() == Some('#') {
                self.skip_comment();
                continue;
            }
            match self.word()? {
                Some(w) => out.push(w),
                None => return Err("fish: Expected a word".to_string()),
            }
        }
    }

    fn trailing_redirs(&mut self) -> PResult<Vec<Redir>> {
        let mut redirs = Vec::new();
        loop {
            self.skip_blank();
            match self.redirect()? {
                Some(r) => redirs.push(r),
                None => return Ok(redirs),
            }
        }
    }

    fn simple(&mut self) -> PResult<Cmd> {
        let mut words = Vec::new();
        let mut redirs = Vec::new();
        loop {
            self.skip_blank();
            match self.peek() {
                None | Some('\n' | ';' | ')') => break,
                Some('#') if words.is_empty() || self.at_word_start() => {
                    self.skip_comment();
                    break;
                }
                Some('|') => break,
                Some('&') if !matches!(self.at(1), Some('>')) => break,
                _ => {}
            }
            if let Some(r) = self.redirect()? {
                redirs.push(r);
                continue;
            }
            match self.word()? {
                Some(w) => words.push(w),
                None => break,
            }
        }
        if words.is_empty() && redirs.is_empty() {
            return Err("fish: Expected a command".to_string());
        }
        Ok(Cmd::Simple { words, redirs })
    }

    /// True when the cursor is at the start of a word (the previous char is blank).
    fn at_word_start(&self) -> bool {
        self.i == 0 || matches!(self.s[self.i - 1], ' ' | '\t' | '\n')
    }

    /// `[N]>`, `>>`, `<`, `>&`, `<&`, `>?`, `&>`, `&>>` and their target.
    fn redirect(&mut self) -> PResult<Option<Redir>> {
        let save = self.i;
        let mut fd = None;
        let mut j = self.i;
        let mut digits = String::new();
        while let Some(c) = self.s.get(j).filter(|c| c.is_ascii_digit()) {
            digits.push(*c);
            j += 1;
        }
        if !digits.is_empty() && matches!(self.s.get(j), Some('<' | '>')) {
            fd = digits.parse().ok();
            self.i = j;
        }
        let op = ["&>>", "&>", ">>", ">&", ">?", ">", "<&", "<"]
            .into_iter()
            .find(|op| self.starts_with(op));
        let Some(op) = op else {
            self.i = save;
            return Ok(None);
        };
        if fd.is_some() && op.starts_with('&') {
            self.i = save;
            return Ok(None);
        }
        self.i += op.len();
        self.skip_blank();
        let target = self.word()?.ok_or("fish: Expected a string, but found end of the statement")?;
        Ok(Some(Redir {
            fd,
            op: op.to_string(),
            target,
        }))
    }

    /// One word, or `None` at a delimiter.
    fn word(&mut self) -> PResult<Option<Word>> {
        let mut parts: Vec<Part> = Vec::new();
        let start = self.i;
        let mut lit = String::new();
        // nesting depth of live brace expansions in this word
        let mut braces: Vec<bool> = Vec::new();
        macro_rules! flush {
            () => {
                if !lit.is_empty() {
                    parts.push(Part::Lit(std::mem::take(&mut lit)));
                }
            };
        }
        while let Some(c) = self.peek() {
            if WORD_END.contains(&c) {
                break;
            }
            match c {
                '\\' => {
                    self.i += 1;
                    if let Some(s) = self.escape()? {
                        lit.push_str(&s);
                    }
                }
                '\'' => {
                    flush!();
                    self.i += 1;
                    parts.push(Part::Single(self.single_quoted()?));
                }
                '"' => {
                    flush!();
                    self.i += 1;
                    parts.push(Part::Double(self.double_quoted()?));
                }
                '$' => {
                    if let Some(p) = self.dollar(false)? {
                        flush!();
                        parts.push(p);
                    } else {
                        lit.push('$');
                        self.i += 1;
                    }
                }
                '(' => {
                    flush!();
                    self.i += 1;
                    parts.push(self.cmd_subst()?);
                }
                '*' => {
                    flush!();
                    let mut stars = String::new();
                    while self.peek() == Some('*') {
                        stars.push('*');
                        self.i += 1;
                    }
                    parts.push(Part::Raw(stars));
                }
                '~' if self.i == start => {
                    self.i += 1;
                    let mut t = String::from("~");
                    while let Some(n) = self.peek().filter(|n| n.is_alphanumeric() || matches!(n, '_' | '-' | '.')) {
                        t.push(n);
                        self.i += 1;
                    }
                    parts.push(Part::Raw(t));
                }
                '{' if self.brace_expansion_ahead().is_some() => {
                    flush!();
                    // `{a,b}` expands; `{$x}` (no comma) just yields its contents
                    let real = self.brace_expansion_ahead() == Some(true);
                    braces.push(real);
                    self.i += 1;
                    if real {
                        parts.push(Part::Raw("{".to_string()));
                    }
                }
                ',' | '}' if !braces.is_empty() => {
                    flush!();
                    let real = *braces.last().unwrap_or(&true);
                    if c == '}' {
                        braces.pop();
                    }
                    self.i += 1;
                    if real {
                        parts.push(Part::Raw(c.to_string()));
                    } else if c == ',' {
                        lit.push(',');
                    }
                }
                _ => {
                    lit.push(c);
                    self.i += 1;
                }
            }
        }
        flush!();
        Ok((!parts.is_empty() || self.i > start).then_some(Word { parts }))
    }

    /// At `{`: `Some(true)` for a brace expansion (matching `}` with a top-level comma),
    /// `Some(false)` for braces around a variable (`{$x}`), `None` for literal braces.
    fn brace_expansion_ahead(&self) -> Option<bool> {
        let mut depth = 0i32;
        let mut comma = false;
        let mut var = false;
        let mut j = self.i;
        while let Some(&c) = self.s.get(j) {
            match c {
                '\\' => j += 1,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return if comma {
                            Some(true)
                        } else if var {
                            Some(false)
                        } else {
                            None
                        };
                    }
                }
                ',' if depth == 1 => comma = true,
                '$' if depth == 1 => var = true,
                ' ' | '\t' | '\n' | ';' | '|' | '&' | '<' | '>' => return None,
                _ => {}
            }
            j += 1;
        }
        None
    }

    /// After a backslash outside quotes: the text it stands for, or `None` for a continuation.
    fn escape(&mut self) -> PResult<Option<String>> {
        let Some(c) = self.peek() else {
            return Ok(Some("\\".to_string()));
        };
        self.i += 1;
        let simple = |ch: char| Ok(Some(ch.to_string()));
        match c {
            '\n' => Ok(None),
            'n' => simple('\n'),
            't' => simple('\t'),
            'r' => simple('\r'),
            'e' => simple('\x1b'),
            'a' => simple('\x07'),
            'b' => simple('\x08'),
            'f' => simple('\x0c'),
            'v' => simple('\x0b'),
            'x' | 'X' => Ok(Some(self.hex_escape(2, c))),
            'u' => Ok(Some(self.hex_escape(4, c))),
            'U' => Ok(Some(self.hex_escape(8, c))),
            'c' => match self.peek() {
                Some(l) if l.is_ascii_alphabetic() => {
                    self.i += 1;
                    simple(char::from((l.to_ascii_lowercase() as u8) - b'a' + 1))
                }
                _ => simple('c'),
            },
            other => simple(other),
        }
    }

    fn hex_escape(&mut self, max: usize, lead: char) -> String {
        let mut v = 0u32;
        let mut n = 0;
        while n < max {
            match self.peek().and_then(|d| d.to_digit(16)) {
                Some(d) => {
                    v = v * 16 + d;
                    self.i += 1;
                    n += 1;
                }
                None => break,
            }
        }
        if n == 0 {
            return lead.to_string();
        }
        char::from_u32(v).map(String::from).unwrap_or_default()
    }

    /// After the opening `'`.
    fn single_quoted(&mut self) -> PResult<String> {
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return Err("fish: Unexpected end of string, quotes are not balanced".to_string()),
                Some('\'') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some('\\') if matches!(self.at(1), Some('\\' | '\'')) => {
                    out.push(self.at(1).unwrap_or('\\'));
                    self.i += 2;
                }
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }

    /// After the opening `"`.
    fn double_quoted(&mut self) -> PResult<Vec<DPart>> {
        let mut out = Vec::new();
        let mut lit = String::new();
        loop {
            match self.peek() {
                None => return Err("fish: Unexpected end of string, quotes are not balanced".to_string()),
                Some('"') => {
                    self.i += 1;
                    break;
                }
                Some('\\') => match self.at(1) {
                    Some(n @ ('"' | '$' | '\\')) => {
                        lit.push(n);
                        self.i += 2;
                    }
                    Some('\n') => self.i += 2,
                    _ => {
                        lit.push('\\');
                        self.i += 1;
                    }
                },
                Some('$') => match self.dollar(true)? {
                    Some(Part::Var(v)) => {
                        if !lit.is_empty() {
                            out.push(DPart::Lit(std::mem::take(&mut lit)));
                        }
                        out.push(DPart::Var(v));
                    }
                    Some(Part::Cmd(s, _)) => {
                        if !lit.is_empty() {
                            out.push(DPart::Lit(std::mem::take(&mut lit)));
                        }
                        out.push(DPart::Cmd(s));
                    }
                    _ => {
                        lit.push('$');
                        self.i += 1;
                    }
                },
                Some(c) => {
                    lit.push(c);
                    self.i += 1;
                }
            }
        }
        if !lit.is_empty() {
            out.push(DPart::Lit(lit));
        }
        Ok(out)
    }

    /// At `$`: a variable reference or `$(…)`; `None` when the `$` is a plain character.
    fn dollar(&mut self, quoted: bool) -> PResult<Option<Part>> {
        let mut j = self.i + 1;
        if self.s.get(j) == Some(&'(') {
            self.i = j + 1;
            return Ok(Some(self.cmd_subst()?));
        }
        let mut deref = false;
        while self.s.get(j) == Some(&'$') {
            deref = true;
            j += 1;
        }
        let mut name = String::new();
        while let Some(&c) = self.s.get(j) {
            if c.is_alphanumeric() || c == '_' {
                name.push(c);
                j += 1;
            } else {
                break;
            }
        }
        if name.is_empty() {
            return Ok(None);
        }
        self.i = j;
        let index = self.index_suffix(quoted)?;
        Ok(Some(Part::Var(VarRef { name, deref, index })))
    }

    /// `[…]` directly after a variable name or a command substitution.
    fn index_suffix(&mut self, _quoted: bool) -> PResult<Option<Vec<String>>> {
        if self.peek() != Some('[') {
            return Ok(None);
        }
        let mut depth = 0i32;
        let mut j = self.i;
        let mut body = String::new();
        while let Some(&c) = self.s.get(j) {
            match c {
                '[' => {
                    depth += 1;
                    if depth > 1 {
                        body.push(c);
                    }
                }
                ']' => {
                    depth -= 1;
                    if depth == 0 {
                        self.i = j + 1;
                        let toks = split_index_tokens(&body);
                        return Ok(Some(toks));
                    }
                    body.push(c);
                }
                '\n' => break,
                _ => body.push(c),
            }
            j += 1;
        }
        // an unmatched `[` after a name is just a character
        Ok(None)
    }

    /// After `(`: the substitution up to its matching `)`.
    fn cmd_subst(&mut self) -> PResult<Part> {
        let start = self.i;
        let mut depth = 1usize;
        let mut quote: Option<char> = None;
        while let Some(c) = self.peek() {
            match (quote, c) {
                (_, '\\') => self.i += 1,
                (None, '\'' | '"') => quote = Some(c),
                (Some(q), _) if c == q => quote = None,
                (Some('"'), '$') if self.at(1) == Some('(') => {
                    depth += 1;
                    self.i += 1;
                }
                (None, '(') => depth += 1,
                (None, ')') => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                (Some('"'), '(') => {}
                _ => {}
            }
            self.i += 1;
        }
        if self.peek() != Some(')') {
            return Err("fish: Unexpected end of string, parentheses do not match".to_string());
        }
        let inner: String = self.s[start..self.i].iter().collect();
        self.i += 1;
        let script = parse(&inner)?;
        let index = self.index_suffix(false)?;
        Ok(Part::Cmd(script, index))
    }
}

/// Whitespace-separated tokens of an index, except inside `( … )`: `$x[(count $x)]`.
fn split_index_tokens(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    for c in body.chars() {
        match c {
            '(' => {
                depth += 1;
                cur.push(c);
            }
            ')' => {
                depth -= 1;
                cur.push(c);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_words(src: &str) -> Vec<Word> {
        match &parse(src).unwrap()[0].job.cmds[0].0 {
            Cmd::Simple { words, .. } => words.clone(),
            other => panic!("not a simple command: {other:?}"),
        }
    }

    #[test]
    fn single_quote_escapes_only_backslash_and_quote() {
        let w = &simple_words(r"echo 'a\'b\\c\n'")[1];
        assert_eq!(w.text().unwrap(), "a'b\\c\\n");
    }

    #[test]
    fn unquoted_escapes_resolve() {
        let w = &simple_words(r"echo a\tb\ c\*")[1];
        assert_eq!(w.text().unwrap(), "a\tb c*");
    }

    #[test]
    fn command_substitution_nests_and_indexes() {
        let w = &simple_words("echo (string split , (echo a,b))[2]")[1];
        match &w.parts[0] {
            Part::Cmd(inner, Some(idx)) => {
                assert_eq!(idx, &vec!["2".to_string()]);
                assert_eq!(inner.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn brace_needs_a_comma() {
        let w = &simple_words("echo {a,b}{c}")[1];
        assert!(w.parts.contains(&Part::Raw("{".into())));
        assert_eq!(w.parts.iter().filter(|p| **p == Part::Raw("{".into())).count(), 1);
    }

    #[test]
    fn if_condition_takes_following_and_or() {
        let s = parse("if a\nand b\n echo x\nend").unwrap();
        match &s[0].job.cmds[0].0 {
            Cmd::If { arms, .. } => {
                assert_eq!(arms[0].0.len(), 2);
                assert_eq!(arms[0].0[1].conn, Conn::And);
                assert_eq!(arms[0].1.len(), 1);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn else_if_chains_and_missing_end_is_an_error() {
        assert!(parse("if a; b; else if c; d; else; e; end").is_ok());
        assert!(parse("if a; b").is_err());
        assert!(parse("end").is_err());
    }

    #[test]
    fn redirect_with_fd_and_dup() {
        match &parse("cmd 2>&1 >out").unwrap()[0].job.cmds[0].0 {
            Cmd::Simple { redirs, .. } => {
                assert_eq!(redirs[0].fd, Some(2));
                assert_eq!(redirs[0].op, ">&");
                assert_eq!(redirs[1].op, ">");
            }
            other => panic!("{other:?}"),
        }
    }
}
