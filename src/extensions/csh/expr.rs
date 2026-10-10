//! csh expressions → zsh conditions and arithmetic.
//!
//! Reference: tcsh 6.21 (`/bin/tcsh`, sh.exp.c behaviour); every rule below
//! was observed against it.
//!
//! # Value model
//!
//! tcsh evaluates an expression over *strings*. `== != =~ !~` compare
//! strings (`5 == 05` is false, `=~`/`!~` match the right side as a glob);
//! every other operator converts its operands to numbers first, and every
//! operator result (`1`/`0` or a number) is a string again. The translator
//! keeps that split:
//!
//! * string comparisons become `[[ "l" == "r" ]]` (RHS of `=~` a pattern);
//! * numeric operators become zsh arithmetic;
//! * `&& || !` over comparisons become shell `&& || !` lists, so tcsh's
//!   short-circuit order (and the `{ cmd }` side effects it skips) holds;
//! * a comparison or `{ cmd }` used where a number is needed is first
//!   evaluated into a `_csh_tN` temporary, in a `{ ...; }` group that is
//!   itself lazy under `&&`/`||`.
//!
//! # tcsh quirks reproduced
//!
//! * Operators are recognised only as whole whitespace-separated words
//!   (`1==1` is the single word `1==1` → `Badly formed number.`); `( ) < >
//!   | &` always split words. `<=`/`>=` are joined.
//! * There is no unary minus: `-` (and `+ * / % == …`) in operand position
//!   is an empty operand, so `2 * - 3` is `-3`, `- 2 * 3` is `-6`.
//! * A missing operand is the empty string / 0 (`if (1 ==)` is false).
//!   Directly under `@` a missing trailing operand is `Expression Syntax.`.
//! * `==`-level operators do not chain (`1 == 1 == 1` is an error);
//!   relational and arithmetic levels are left-associative.
//! * Numbers are decimal only: `010` is ten, `0x10`/`1.5` are
//!   `Badly formed number.`, a non-numeric word in numeric position is
//!   `Expression Syntax.`; an empty value is 0.
//! * `{ cmd }` runs `cmd` in a subshell, stdout/stderr untouched, and is 1
//!   when it exits 0.
//! * File inquiry letters may be stacked (`-ez f` is exists-and-empty).
//!
//! # Known divergences
//!
//! * A literal that is not a number (`abc`, `0x10`) in numeric position is
//!   rejected at translation time; tcsh only reports it when the operand is
//!   evaluated, so it is silent in a branch skipped by `||`/`&&`.
//! * A variable whose value is not numeric is not diagnosed at run time
//!   (zsh reads it as 0); `Division by 0.`/`Mod by 0.` become zsh's
//!   `division by zero`.
//! * Chained shifts (`1 << 2 << 3`) do not match tcsh, whose result there
//!   is not the left-associative one; single shifts do.
//! * `@ name[i] = e` does not check the subscript range.

use super::cmds::translate_line;
use super::words::translate_word;

const SYNTAX: &str = "Expression Syntax.";
const BADNUM: &str = "Badly formed number.";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Translate the text between the outer parens of `if (...)` /
/// `while (...)` / `else if (...)` into a zsh command whose exit status is
/// the csh truth value (usable directly after `if ` / `while `).
///
/// Errors carry tcsh's text with the `if: ` command prefix where tcsh
/// prints one (`if: Expression Syntax.`, `if: Badly formed number.`,
/// `if: Missing file name.`, `if: Missing '}'.`); `Too many ('s.` and
/// `Too many )'s.` are printed by tcsh without a prefix. Callers translating
/// `while`/`exit` rewrite the prefix with [`for_command`].
pub fn translate_condition(expr: &str) -> Result<String, String> {
    translate_condition_inner(expr).map_err(|e| prefixed("if", e))
}

/// Translate the text after `@` (`n = 2 + 3`, `n++`, `x += 4`,
/// `a[2] = 1`, or empty) into zsh.
///
/// * empty: lists the shell variables (the `set` translation).
/// * `name = e`: `name=$(( ... ))` — a plain scalar assignment, so the
///   variable never becomes a zsh integer-typed parameter.
/// * `name op= e` for `+ - * / % ^ | & << >> || &&`: `name=$(( name op (e) ))`.
/// * `name++` / `name--`: `name=$(( name + 1 ))`.
///
/// Errors carry the `@: ` prefix tcsh prints.
pub fn translate_at(body: &str) -> Result<String, String> {
    translate_at_inner(body).map_err(|e| prefixed("@", e))
}

/// Translate the expression of `exit (expr)` into a complete zsh `exit`
/// statement (`exit $(( ... ))`). The value is the numeric value of the
/// expression, like `@`. Errors carry the `exit: ` prefix.
pub fn translate_exit(expr: &str) -> Result<String, String> {
    translate_exit_inner(expr).map_err(|e| prefixed("exit", e))
}

/// Re-target an error produced by [`translate_condition`] to another
/// command (`while`, `exit`, …): `if: Expression Syntax.` becomes
/// `while: Expression Syntax.`. Unprefixed messages pass through.
pub fn for_command(err: &str, cmd: &str) -> String {
    match err.strip_prefix("if: ") {
        Some(rest) => format!("{cmd}: {rest}"),
        None => err.to_string(),
    }
}

/// Messages tcsh prints without a command prefix.
fn prefixed(cmd: &str, msg: String) -> String {
    if msg.starts_with("Too many") || msg.contains(": Event not found.") {
        msg
    } else {
        format!("{cmd}: {msg}")
    }
}

// ---------------------------------------------------------------------------
// Tokens
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Word,
    Op,
}

/// One csh expression word with its byte span in the source (the span is
/// how `{ cmd }` recovers the raw command text).
#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    text: String,
    start: usize,
    end: usize,
}

/// Words that are operators when they stand alone.
fn is_op_word(w: &str) -> bool {
    matches!(
        w,
        "==" | "!=" | "=~" | "!~" | "+" | "-" | "*" | "/" | "%" | "^" | "!" | "~" | "{" | "}"
    )
}

/// Split an expression the way the csh lexer does: whitespace separates;
/// `( ) < > | &` are always their own tokens (`<<`, `>>`, `&&`, `||`,
/// `<=`, `>=` joined); quotes, backslashes, backticks and `${...}` keep a
/// word together. A word that starts with `!` and is not `!=`/`!~` is
/// history substitution, which tcsh reports as `<rest>: Event not found.`.
fn tokenize(src: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<(usize, char)> = src.char_indices().collect();
    let n = chars.len();
    let byte_at = |i: usize| if i < n { chars[i].0 } else { src.len() };
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        let (start, c) = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '(' || c == ')' {
            i += 1;
            out.push(Token { kind: Kind::Op, text: c.to_string(), start, end: byte_at(i) });
        } else if "<>|&".contains(c) {
            let mut text = c.to_string();
            i += 1;
            if i < n && chars[i].1 == c {
                text.push(c);
                i += 1;
            } else if (c == '<' || c == '>') && i < n && chars[i].1 == '=' {
                text.push('=');
                i += 1;
            }
            out.push(Token { kind: Kind::Op, text, start, end: byte_at(i) });
        } else {
            while i < n {
                let c = chars[i].1;
                if c.is_whitespace() || "()<>|&".contains(c) {
                    break;
                }
                match c {
                    '\\' => i += 2,
                    '\'' | '"' | '`' => {
                        i += 1;
                        while i < n && chars[i].1 != c {
                            if chars[i].1 == '\\' && c != '\'' {
                                i += 1;
                            }
                            i += 1;
                        }
                        i += 1;
                    }
                    '$' if i + 1 < n && chars[i + 1].1 == '{' => {
                        while i < n && chars[i].1 != '}' {
                            i += 1;
                        }
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            i = i.min(n);
            let end = byte_at(i);
            let word = &src[start..end];
            if word.len() > 1 && word.starts_with('!') && word != "!=" && word != "!~" {
                return Err(format!("{}: Event not found.", &word[1..]));
            }
            let kind = if is_op_word(word) { Kind::Op } else { Kind::Word };
            out.push(Token { kind, text: word.to_string(), start, end });
        }
    }
    Ok(out)
}

/// tcsh rejects unbalanced parentheses before evaluating anything.
fn check_parens(toks: &[Token]) -> Result<(), String> {
    let mut depth = 0i32;
    for t in toks.iter().filter(|t| t.kind == Kind::Op) {
        match t.text.as_str() {
            "(" => depth += 1,
            ")" => {
                depth -= 1;
                if depth < 0 {
                    return Err("Too many )'s.".into());
                }
            }
            _ => {}
        }
    }
    if depth > 0 {
        return Err("Too many ('s.".into());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Node {
    /// A missing operand: the empty string, numerically 0.
    Empty,
    /// One csh operand word, raw (quotes and `$` intact).
    Word(String),
    /// `-ez file`: stacked inquiry letters and the raw file word.
    File { letters: String, operand: String },
    /// `{ cmd }`: raw csh command text.
    Cmd(String),
    /// `!`
    Not(Box<Node>),
    /// `~`
    BitNot(Box<Node>),
    Bin(&'static str, Box<Node>, Box<Node>),
}

/// Binary operator levels, loosest first (sh.exp.c exp0..exp6).
const LEVELS: [&[&str]; 10] = [
    &["||"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!=", "=~", "!~"],
    &["<=", ">=", "<", ">"],
    &["<<", ">>"],
    &["+", "-"],
    &["*", "/", "%"],
];
/// Index of the `== != =~ !~` level, which does not chain.
const EQUALITY: usize = 5;

/// File inquiry letters tcsh accepts that are translated.
const FILE_LETTERS: &str = "edfrwxozslbcpSugkt";
/// File inquiry letters tcsh accepts that have no zsh equivalent here.
const FILE_LETTERS_UNSUPPORTED: &str = "mACDGIKLMNPUXZ";

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
    /// Directly under `@`: a missing trailing operand is a syntax error.
    strict_end: bool,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, strict_end: bool) -> Result<Self, String> {
        let toks = tokenize(src)?;
        check_parens(&toks)?;
        Ok(Parser { src, toks, pos: 0, depth: 0, strict_end })
    }

    fn peek(&self) -> Option<&Token> {
        self.toks.get(self.pos)
    }

    fn at_op(&self, text: &str) -> bool {
        matches!(self.peek(), Some(t) if t.kind == Kind::Op && t.text == text)
    }

    /// Parse a whole expression; leftover tokens are an error for the
    /// caller to word.
    fn parse(&mut self) -> Result<Node, String> {
        let n = self.binary(0)?;
        Ok(n)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    fn binary(&mut self, level: usize) -> Result<Node, String> {
        if level == LEVELS.len() {
            return self.primary();
        }
        let mut left = self.binary(level + 1)?;
        loop {
            let op = match self.peek() {
                Some(t) if t.kind == Kind::Op => LEVELS[level].iter().find(|o| **o == t.text),
                _ => None,
            };
            let Some(op) = op else { break };
            self.pos += 1;
            let right = if level == EQUALITY { self.string_operand()? } else { self.binary(level + 1)? };
            left = Node::Bin(op, Box::new(left), Box::new(right));
            if level == EQUALITY {
                break;
            }
        }
        Ok(left)
    }

    fn primary(&mut self) -> Result<Node, String> {
        let Some(tok) = self.peek().cloned() else {
            return if self.strict_end && self.depth == 0 {
                Err(SYNTAX.into())
            } else {
                Ok(Node::Empty)
            };
        };
        if tok.kind == Kind::Word {
            self.pos += 1;
            return match file_inquiry(&tok.text)? {
                Some(letters) => self.file_operand(letters),
                None => Ok(Node::Word(tok.text)),
            };
        }
        match tok.text.as_str() {
            "(" => {
                self.pos += 1;
                self.depth += 1;
                let inner = if self.at_op(")") { Node::Empty } else { self.binary(0)? };
                if !self.at_op(")") {
                    return Err(SYNTAX.into());
                }
                self.pos += 1;
                self.depth -= 1;
                Ok(inner)
            }
            "!" => {
                self.pos += 1;
                Ok(Node::Not(Box::new(self.primary()?)))
            }
            "~" => {
                self.pos += 1;
                Ok(Node::BitNot(Box::new(self.primary()?)))
            }
            "{" => self.command(tok.end),
            "&&" | "||" | "}" => Err(SYNTAX.into()),
            // Every other operator in operand position: an empty operand
            // that leaves the operator for the enclosing level.
            _ => Ok(Node::Empty),
        }
    }

    /// Right operand of `== != =~ !~`. A bare `*`, `+`, `-`, `/`, `%`, `^`
    /// there is the literal word (`abc =~ *` matches), not an empty
    /// operand.
    fn string_operand(&mut self) -> Result<Node, String> {
        if let Some(t) = self.peek() {
            if t.kind == Kind::Op && matches!(t.text.as_str(), "*" | "+" | "-" | "/" | "%" | "^") {
                let text = t.text.clone();
                self.pos += 1;
                return Ok(Node::Word(text));
            }
        }
        self.binary(EQUALITY + 1)
    }

    /// The file word after `-e` etc.; a following operator (or nothing) is
    /// `Missing file name.`.
    fn file_operand(&mut self, letters: String) -> Result<Node, String> {
        match self.peek() {
            Some(t) if t.kind == Kind::Word => {
                let operand = t.text.clone();
                self.pos += 1;
                Ok(Node::File { letters, operand })
            }
            _ => Err("Missing file name.".into()),
        }
    }

    /// `{ cmd }`: raw text up to the first `}` word.
    fn command(&mut self, open_end: usize) -> Result<Node, String> {
        self.pos += 1;
        while let Some(t) = self.peek() {
            if t.kind == Kind::Op && t.text == "}" {
                let text = self.src[open_end..t.start].trim().to_string();
                self.pos += 1;
                return Ok(Node::Cmd(text));
            }
            self.pos += 1;
        }
        Err("Missing '}'.".into())
    }
}

/// Classify a word as a file inquiry. `Ok(None)` when it is an ordinary
/// operand; `Ok(Some(letters))` for `-e`, `-ez`, …; an error for `-e/tmp`
/// (`Malformed file inquiry.`) or a letter with no zsh equivalent.
fn file_inquiry(word: &str) -> Result<Option<String>, String> {
    let Some(rest) = word.strip_prefix('-') else { return Ok(None) };
    let Some(first) = rest.chars().next() else { return Ok(None) };
    if !FILE_LETTERS.contains(first) && !FILE_LETTERS_UNSUPPORTED.contains(first) {
        return Ok(None);
    }
    for c in rest.chars() {
        if FILE_LETTERS_UNSUPPORTED.contains(c) {
            return Err(format!("-{c}: file inquiry has no zsh translation"));
        }
        if !FILE_LETTERS.contains(c) {
            return Err("Malformed file inquiry.".into());
        }
    }
    Ok(Some(rest.to_string()))
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// A zsh command and whether it is a single command that can follow `!`,
/// `&&` or `||` without grouping.
struct Cond {
    text: String,
    simple: bool,
}

impl Cond {
    fn grouped(&self) -> String {
        if self.simple {
            self.text.clone()
        } else {
            format!("{{ {}; }}", self.text)
        }
    }
}

/// A zsh arithmetic expression; `atomic` when it needs no parentheses as
/// an operand.
struct Arith {
    text: String,
    atomic: bool,
}

impl Arith {
    fn atom(text: String) -> Self {
        Arith { text, atomic: true }
    }

    fn operand(&self) -> String {
        if self.atomic {
            self.text.clone()
        } else {
            format!("({})", self.text)
        }
    }
}

#[derive(Default)]
struct Emitter {
    temps: usize,
}

/// Operators that yield a truth value through `[[ ]]` / a command.
fn is_cond_only(n: &Node) -> bool {
    match n {
        Node::File { .. } | Node::Cmd(_) => true,
        Node::Bin(op, _, _) => LEVELS[EQUALITY].contains(op),
        _ => false,
    }
}

/// True when `n` contains something that cannot be written in `(( ))`.
fn has_cond(n: &Node) -> bool {
    match n {
        Node::Empty | Node::Word(_) => false,
        Node::File { .. } | Node::Cmd(_) => true,
        Node::Not(x) | Node::BitNot(x) => has_cond(x),
        Node::Bin(op, l, r) => LEVELS[EQUALITY].contains(op) || has_cond(l) || has_cond(r),
    }
}

/// Logical nodes whose truth is best computed by shell `&&`/`||`/`!`.
fn is_shell_logic(n: &Node) -> bool {
    match n {
        Node::Not(x) => has_cond(x),
        Node::Bin(op, _, _) => (*op == "&&" || *op == "||") && has_cond(n),
        _ => false,
    }
}

impl Emitter {
    fn temp(&mut self) -> String {
        self.temps += 1;
        format!("_csh_t{}", self.temps)
    }

    /// Wrap `pre` statements and `test` into one command.
    fn with_pre(pre: Vec<String>, test: String) -> Cond {
        if pre.is_empty() {
            Cond { text: test, simple: true }
        } else {
            Cond { text: format!("{{ {}; {}; }}", pre.join("; "), test), simple: true }
        }
    }

    fn cond(&mut self, n: &Node) -> Result<Cond, String> {
        match n {
            Node::Cmd(text) => {
                let body = if text.is_empty() { "true".to_string() } else { translate_line(text)? };
                Ok(Cond { text: format!("( {} )", body.trim()), simple: true })
            }
            Node::File { letters, operand } => {
                let file = quote_string(&translate_word(operand));
                let parts: Vec<String> = letters.chars().map(|c| file_test(c, &file)).collect();
                Ok(Cond { text: format!("[[ {} ]]", parts.join(" && ")), simple: true })
            }
            Node::Not(x) if has_cond(x) => {
                let inner = self.cond(x)?;
                let operand = if inner.text.starts_with('!') {
                    format!("{{ {}; }}", inner.text)
                } else {
                    inner.grouped()
                };
                Ok(Cond { text: format!("! {operand}"), simple: true })
            }
            Node::Bin(op, l, r) if (*op == "&&" || *op == "||") && has_cond(n) => {
                let (l, r) = (self.cond(l)?, self.cond(r)?);
                Ok(Cond { text: format!("{} {op} {}", l.grouped(), r.grouped()), simple: false })
            }
            Node::Bin(op, l, r) if LEVELS[EQUALITY].contains(op) => {
                let mut pre = Vec::new();
                let left = self.string(l, &mut pre)?;
                let test = if *op == "=~" || *op == "!~" {
                    let pat = self.pattern(r, &mut pre)?;
                    let cmp = if *op == "=~" { "==" } else { "!=" };
                    format!("[[ {left} {cmp} {pat} ]]")
                } else {
                    let right = self.string(r, &mut pre)?;
                    format!("[[ {left} {op} {right} ]]")
                };
                Ok(Self::with_pre(pre, test))
            }
            _ => {
                let mut pre = Vec::new();
                let a = self.arith(n, &mut pre)?;
                Ok(Self::with_pre(pre, format!("(( {} ))", a.text)))
            }
        }
    }

    /// Arithmetic text for `n`; comparisons and commands are evaluated
    /// into a temporary pushed on `pre`.
    fn arith(&mut self, n: &Node, pre: &mut Vec<String>) -> Result<Arith, String> {
        if is_cond_only(n) || is_shell_logic(n) {
            let c = self.cond(n)?;
            let t = self.temp();
            pre.push(format!("{}; {t}=$(( $? == 0 ))", c.text));
            return Ok(Arith::atom(t));
        }
        match n {
            Node::Empty => Ok(Arith::atom("0".into())),
            Node::Word(raw) => numeric_word(raw),
            Node::Not(x) => Ok(Arith { text: format!("!{}", self.arith(x, pre)?.operand()), atomic: false }),
            Node::BitNot(x) => Ok(Arith { text: format!("~{}", self.arith(x, pre)?.operand()), atomic: false }),
            Node::Bin(op, l, r) => {
                let (l, r) = (self.arith(l, pre)?, self.arith(r, pre)?);
                Ok(Arith { text: format!("{} {op} {}", l.operand(), r.operand()), atomic: false })
            }
            Node::File { .. } | Node::Cmd(_) => unreachable!("handled as cond-only"),
        }
    }

    /// The string value of `n` as one quoted zsh word.
    fn string(&mut self, n: &Node, pre: &mut Vec<String>) -> Result<String, String> {
        match n {
            Node::Empty => Ok("\"\"".into()),
            Node::Word(raw) => Ok(quote_string(&translate_word(raw))),
            _ => {
                let a = self.arith(n, pre)?;
                Ok(if is_integer(&a.text) { a.text } else { format!("\"$(( {} ))\"", a.text) })
            }
        }
    }

    /// The right side of `=~`/`!~` as a zsh pattern word. A csh pattern is
    /// a glob even when quoted.
    fn pattern(&mut self, n: &Node, pre: &mut Vec<String>) -> Result<String, String> {
        match n {
            Node::Word(raw) => {
                let pat = glob_pattern(&translate_word(raw));
                Ok(if pat.is_empty() { "\"\"".into() } else { pat })
            }
            _ => self.string(n, pre),
        }
    }
}

/// `-e` style letter → zsh `[[ ]]` test on the quoted file word.
fn file_test(letter: char, file: &str) -> String {
    match letter {
        'z' => format!("-e {file} && ! -s {file}"),
        'o' => format!("-O {file}"),
        'l' => format!("-L {file}"),
        c => format!("-{c} {file}"),
    }
}

fn is_integer(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
}

/// The word with its quotes and backslashes removed, when it contains no
/// expansion that could change its value at run time.
fn static_value(raw: &str) -> Option<String> {
    let brace_list = raw.contains('{') && raw.contains(',');
    if raw.contains(['$', '`', '*', '?', '[']) || brace_list || raw.starts_with('~') {
        return None;
    }
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None | Some('"'), '\\') => out.extend(chars.next()),
            _ => out.push(c),
        }
    }
    Some(out)
}

/// tcsh's numeric conversion of a literal: empty is 0; a leading `-` and
/// decimal digits; anything else is an error (`Expression Syntax.` when it
/// does not even start like a number, `Badly formed number.` otherwise).
fn literal_number(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Ok("0".into());
    }
    if !s.starts_with('-') && !s.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(SYNTAX.into());
    }
    let digits = s.strip_prefix('-').unwrap_or(s);
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(BADNUM.into());
    }
    let trimmed = digits.trim_start_matches('0');
    Ok(match (trimmed.is_empty(), s.starts_with('-')) {
        (true, _) => "0".into(),
        (false, true) => format!("-{trimmed}"),
        (false, false) => trimmed.to_string(),
    })
}

/// A csh operand word in numeric position. Literals are validated now;
/// words with expansions are read at run time, empty meaning 0.
fn numeric_word(raw: &str) -> Result<Arith, String> {
    if let Some(v) = static_value(raw) {
        let n = literal_number(&v)?;
        return Ok(if n.starts_with('-') { Arith { text: n, atomic: false } } else { Arith::atom(n) });
    }
    let w = translate_word(raw);
    if let Some(name) = plain_param(&w) {
        return Ok(Arith::atom(name.to_string()));
    }
    Ok(Arith::atom(format!("${{${{:-{w}}}:-0}}")))
}

/// `$name` / `${name}` → `name`.
fn plain_param(w: &str) -> Option<&str> {
    let name = w
        .strip_prefix("${")
        .and_then(|r| r.strip_suffix('}'))
        .or_else(|| w.strip_prefix('$'))?;
    let mut chars = name.chars();
    let first = chars.next()?;
    (first.is_ascii_alphabetic() || first == '_')
        .then_some(())
        .filter(|_| chars.all(|c| c.is_ascii_alphanumeric() || c == '_'))
        .map(|_| name)
}

/// Quote a translated word so `[[ ]]` compares it as a literal string
/// (zsh would otherwise read an unquoted `$x` on the right as a pattern).
fn quote_string(w: &str) -> String {
    let whole = |q: char| {
        w.len() >= 2 && w.starts_with(q) && w.ends_with(q) && !w[1..w.len() - 1].contains(q)
    };
    if whole('"') || whole('\'') {
        w.to_string()
    } else if w.contains(['"', '\'', '\\', '`']) {
        format!("\"${{:-{w}}}\"")
    } else {
        format!("\"{w}\"")
    }
}

/// Turn a translated csh pattern word into a zsh `[[ ]]` pattern: quotes
/// are dropped (csh quoting does not protect `*?[`), characters zsh would
/// treat specially but csh does not are backslashed, and `$name`
/// expansions become `${~name}` so their value is matched as a pattern.
fn glob_pattern(w: &str) -> String {
    let chars: Vec<char> = w.chars().collect();
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut in_class = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            // csh `=~` ignores backslash protection: `\*` still globs.
            '\\' if i + 1 < chars.len() => {
                i += 1;
                match chars[i] {
                    c if "*?[]".contains(c) => out.push(c),
                    c if c.is_alphanumeric() || "/._-,=+:@%".contains(c) => out.push(c),
                    c => {
                        out.push('\\');
                        out.push(c);
                    }
                }
            }
            '\'' | '"' if quote.is_none() => quote = Some(c),
            c if quote == Some(c) => quote = None,
            '$' if quote != Some('\'') => i = copy_expansion(&chars, i, &mut out),
            '[' => {
                in_class = true;
                out.push(c);
            }
            ']' => {
                in_class = false;
                out.push(c);
            }
            '*' | '?' => out.push(c),
            c if in_class || c.is_alphanumeric() || "/._-,=+:@%".contains(c) => out.push(c),
            c => {
                out.push('\\');
                out.push(c);
            }
        }
        i += 1;
    }
    out
}

/// Copy the `$...` expansion starting at `chars[i]` into `out` (as a
/// pattern-substituting `${~name}` when it is a plain name); returns the
/// index of its last character.
fn copy_expansion(chars: &[char], i: usize, out: &mut String) -> usize {
    let at = |j: usize| chars.get(j).copied();
    let (open, close) = match at(i + 1) {
        Some('{') => ('{', '}'),
        Some('(') => ('(', ')'),
        _ => ('\0', '\0'),
    };
    if open != '\0' {
        let mut depth = 0;
        let mut j = i + 1;
        while let Some(c) = at(j) {
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            j += 1;
        }
        let inner: String = chars[i + 2..j.min(chars.len())].iter().collect();
        let plain = open == '{' && inner.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_');
        if plain {
            out.push_str(&format!("${{~{inner}}}"));
        } else {
            out.extend(&chars[i..(j + 1).min(chars.len())]);
        }
        return j;
    }
    let mut j = i + 1;
    while at(j).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
        j += 1;
    }
    let name: String = chars[i + 1..j].iter().collect();
    if name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
        out.push_str(&format!("${{~{name}}}"));
        j - 1
    } else {
        out.push('$');
        i
    }
}

// ---------------------------------------------------------------------------
// Entry points
// ---------------------------------------------------------------------------

fn translate_condition_inner(expr: &str) -> Result<String, String> {
    let mut p = Parser::new(expr, false)?;
    if p.at_end() {
        return Ok("false".into());
    }
    let node = p.parse()?;
    // tcsh converts the value before it notices leftover words.
    let cond = Emitter::default().cond(&node)?;
    if !p.at_end() {
        return Err(SYNTAX.into());
    }
    Ok(cond.text)
}

/// Parse `expr` as the value expression after `@ name =` / `exit`.
fn value_of(expr: &str, leftover: &str) -> Result<(Arith, Vec<String>), String> {
    let mut p = Parser::new(expr, true)?;
    let node = p.parse()?;
    let mut pre = Vec::new();
    let a = Emitter::default().arith(&node, &mut pre)?;
    if !p.at_end() {
        return Err(leftover.into());
    }
    Ok((a, pre))
}

fn group(pre: Vec<String>, stmt: String) -> String {
    if pre.is_empty() {
        stmt
    } else {
        format!("{{ {}; {stmt}; }}", pre.join("; "))
    }
}

fn translate_exit_inner(expr: &str) -> Result<String, String> {
    let (a, pre) = if expr.trim().is_empty() {
        (Arith::atom("0".into()), Vec::new())
    } else {
        value_of(expr, SYNTAX)?
    };
    Ok(group(pre, format!("exit $(( {} ))", a.text)))
}

const NAME_ERR: &str = "Variable name must begin with a letter.";

/// Assignment operators, longest first.
const ASSIGN_OPS: [&str; 15] = [
    "<<=", ">>=", "||=", "&&=", "++", "--", "+=", "-=", "*=", "/=", "%=", "^=", "|=", "&=", "=",
];

fn translate_at_inner(body: &str) -> Result<String, String> {
    let body = body.trim();
    if body.is_empty() {
        return translate_line("set");
    }
    let first = body.chars().next().unwrap_or(' ');
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(NAME_ERR.into());
    }
    let name_len = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(body.len());
    let mut target = body[..name_len].to_string();
    let mut rest = &body[name_len..];
    if rest.starts_with('[') {
        let close = rest.find(']').ok_or("Subscript error.")?;
        let sub = translate_word(rest[1..close].trim());
        target = format!("{target}[{sub}]");
        rest = &rest[close + 1..];
    }
    let rest = rest.trim_start();
    let Some(op) = ASSIGN_OPS.iter().find(|o| rest.starts_with(**o)) else {
        return Err(if rest.is_empty() {
            "Assignment missing expression.".into()
        } else {
            "Unknown operator.".into()
        });
    };
    let expr = rest[op.len()..].trim();
    if *op == "++" || *op == "--" {
        if !expr.is_empty() {
            return Err(NAME_ERR.into());
        }
        let sign = &op[..1];
        return Ok(format!("{target}=$(( {target} {sign} 1 ))"));
    }
    if expr.is_empty() {
        return Err(if *op == "=" { "Assignment missing expression." } else { SYNTAX }.into());
    }
    let (a, pre) = value_of(expr, NAME_ERR)?;
    let value = match op.strip_suffix('=') {
        Some("") => a.text,
        Some(bin) => format!("{target} {bin} {}", a.operand()),
        None => unreachable!("every assignment operator ends in '='"),
    };
    let stmt = if is_integer(&value) {
        format!("{target}={value}")
    } else {
        format!("{target}=$(( {value} ))")
    };
    Ok(group(pre, stmt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Once;

    /// Scratch directory holding `emp` (empty file), `full` (non-empty),
    /// `lnk` (symlink to `full`) and `dd` (directory).
    fn fixtures() -> PathBuf {
        static INIT: Once = Once::new();
        let dir = std::env::temp_dir().join(format!("csh_expr_fixtures_{}", std::process::id()));
        INIT.call_once(|| {
            let _ = std::fs::create_dir_all(dir.join("dd"));
            let _ = std::fs::write(dir.join("emp"), "");
            let _ = std::fs::write(dir.join("full"), "hi\n");
            #[cfg(unix)]
            let _ = std::os::unix::fs::symlink("full", dir.join("lnk"));
        });
        dir
    }

    /// Run `script` under `zsh -f` in the fixture directory; `None` when
    /// zsh is not installed.
    fn zsh(script: &str) -> Option<String> {
        let out = Command::new("zsh")
            .args(["-f", "-c", script])
            .current_dir(fixtures())
            .output()
            .ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    const PRELUDE: &str = "x=5; e=''; s=abc; l=(7 8 9);";

    fn truth(expr: &str) -> Option<String> {
        let cond = translate_condition(expr).unwrap_or_else(|e| panic!("[{expr}] {e}"));
        zsh(&format!("{PRELUDE} if {cond}; then echo T; else echo F; fi"))
    }

    fn value(stmt: &str, show: &str) -> Option<String> {
        let code = translate_at(stmt).unwrap_or_else(|e| panic!("@ {stmt}: {e}"));
        zsh(&format!("{PRELUDE} {code}; echo {show}"))
    }

    /// Each row was run through `/bin/tcsh -f` (`if (EXPR) then`) and gave
    /// the listed truth value.
    #[test]
    fn conditions_match_tcsh() {
        let rows: &[(&str, &str)] = &[
            // == and != compare strings, never numbers
            ("5 == 5", "T"),
            ("5 == 05", "F"),
            ("010 == 10", "F"),
            ("\"5\" == 5", "T"),
            ("5 == 5.0", "F"),
            ("$x == 5", "T"),
            ("$e == \"\"", "T"),
            ("\"\" == \"\"", "T"),
            ("\"a b\" == \"a b\"", "T"),
            ("x != y", "T"),
            ("abc != abc", "F"),
            // =~ and !~ match the right side as a glob, quotes or not
            ("abc =~ a*", "T"),
            ("abc =~ \"a*\"", "T"),
            ("abc =~ a?c", "T"),
            ("abc =~ a[a-c]c", "T"),
            ("abc !~ b", "T"),
            ("abc =~ *b*", "T"),
            ("$s =~ a*", "T"),
            ("\"\" =~ *", "T"),
            ("\"\" =~ ?", "F"),
            ("\"\" =~ \"\"", "T"),
            ("(2 + 3) =~ 5", "T"),
            // relational and arithmetic operators are numeric
            ("3 > 2", "T"),
            ("3 >= 3", "T"),
            ("2 <= 1", "F"),
            ("1 <2", "T"),
            ("$x > 3", "T"),
            ("$e + 1", "T"),
            ("1 + 1 == 2", "T"),
            ("1 + 2 == \"3\"", "T"),
            ("3 > 2 > 1", "F"),
            ("1 <= 1 <= 1", "T"),
            ("(1 < 2) < 3", "T"),
            ("10 - 3 - 2 == 5", "T"),
            ("1 << 1 + 1 == 4", "T"),
            ("1 + 2 * 3 == 7", "T"),
            ("(1 + 2) * 3 == 9", "T"),
            ("-7 % 3 == -1", "T"),
            ("-7 / 2 == -3", "T"),
            // truthiness of a bare operand
            ("1", "T"),
            ("0", "F"),
            ("$e", "F"),
            ("$x", "T"),
            ("", "F"),
            ("010", "T"),
            // logical operators and precedence
            ("1 && 0", "F"),
            ("1&&0", "F"),
            ("0 || 0", "F"),
            ("0 && 1 || 1", "T"),
            ("1 || 0 && 0", "T"),
            ("1 && 1 || 0", "T"),
            ("5 == 5 && 6 == 6", "T"),
            ("1 && 2 && 0", "F"),
            ("1 ^ 1", "F"),
            ("1 | 2 ^ 3 & 4", "T"),
            ("6 & 1", "F"),
            ("! 1", "F"),
            ("! 0 + 1 == 2", "T"),
            ("! ! 1", "T"),
            ("!(0)", "T"),
            ("~ 1 + 1", "T"),
            ("~ 0 == -1", "T"),
            // no unary minus: `-` in operand position is an empty operand
            ("2 * - 3 == -3", "T"),
            ("- 2 * 3 == -6", "T"),
            ("2 - - 3 == -1", "T"),
            // a missing operand is the empty string
            ("1 ==", "F"),
            ("== 1", "F"),
            ("!= 1", "T"),
            ("1 +", "T"),
            ("(1 ==)", "F"),
            // file inquiry
            ("-e emp", "T"),
            ("-e nonex", "F"),
            ("-d dd", "T"),
            ("-d full", "F"),
            ("-f full", "T"),
            ("-f lnk", "T"),
            ("-l lnk", "T"),
            ("-l full", "F"),
            ("-z emp", "T"),
            ("-z full", "F"),
            ("-z nonex", "F"),
            ("-s full", "T"),
            ("-s emp", "F"),
            ("-r full", "T"),
            ("-w full", "T"),
            ("-x full", "F"),
            ("-o full", "T"),
            ("-e \"\"", "F"),
            ("-ez emp", "T"),
            ("-ez full", "F"),
            ("-fs full", "T"),
            ("-dz dd", "F"),
            ("! -e nonex", "T"),
            ("-e emp && -f full", "T"),
            ("-e emp && ! -d emp", "T"),
            ("(-e emp) && 1", "T"),
            ("-e emp == 1", "T"),
            ("-e emp == -e emp", "T"),
            ("-e nonex + 1 == 1", "T"),
            // { command } exit status
            ("{ true }", "T"),
            ("{ false }", "F"),
            ("! { false }", "T"),
            ("{ true } && { false }", "F"),
            ("{ false } || { true }", "T"),
            ("{ }", "T"),
            ("{ true } == 1", "T"),
            ("1 == { true }", "T"),
            ("{ false } + 1", "T"),
            ("1 || { echo hi }", "T"),
            ("0 && { echo hi }", "F"),
        ];
        if zsh("true").is_none() {
            return;
        }
        for (expr, want) in rows {
            let got = truth(expr).unwrap();
            assert_eq!(got, *want, "if ({expr})");
        }
    }

    /// tcsh evaluates `a || b` left to right and skips `b` (and its
    /// `{ cmd }` side effects) when `a` decides the result; `{ cmd }` output
    /// reaches stdout.
    #[test]
    fn short_circuit_skips_commands() {
        if zsh("true").is_none() {
            return;
        }
        assert_eq!(truth("1 || { echo hi }").unwrap(), "T");
        assert_eq!(truth("0 && { echo hi }").unwrap(), "F");
        assert_eq!(truth("{ echo hi } || 1").unwrap(), "hi\nT");
        // the skipped operand sits in a numeric position: still lazy
        assert_eq!(truth("1 || { echo hi } + 1").unwrap(), "T");
    }

    /// `{ cmd }` is a subshell: its variable assignments do not survive.
    #[test]
    fn command_runs_in_subshell() {
        if zsh("true").is_none() {
            return;
        }
        let cond = translate_condition("{ zq=1 }").unwrap();
        let out = zsh(&format!("if {cond}; then echo ${{+zq}}; fi")).unwrap();
        assert_eq!(out, "0");
    }

    /// Each row: `@` argument, expression printed afterwards, value tcsh
    /// printed (x=5, e='', s=abc, l=(7 8 9)).
    #[test]
    fn at_assignments_match_tcsh() {
        let rows: &[(&str, &str, &str)] = &[
            ("n = 1 + 2", "$n", "3"),
            ("n=1", "$n", "1"),
            ("n = (2 + 3) * 4", "$n", "20"),
            ("n = 7 / 2", "$n", "3"),
            ("n = -7 / 2", "$n", "-3"),
            ("n = -7 % 3", "$n", "-1"),
            ("n = 010 + 1", "$n", "11"),
            ("n = 2 * - 3", "$n", "-3"),
            ("n = - 2 * 3", "$n", "-6"),
            ("n = ~ 0", "$n", "-1"),
            ("n = ! 0", "$n", "1"),
            ("n = ! 1 + 1", "$n", "1"),
            ("n = 10 - 3 - 2", "$n", "5"),
            ("n = 1 << 1 + 1", "$n", "4"),
            ("n = 5 ^ 3", "$n", "6"),
            ("n = 1 || 0", "$n", "1"),
            ("n = 5 && 7", "$n", "1"),
            ("n = 5 == 5", "$n", "1"),
            ("n = ($s == abc)", "$n", "1"),
            ("n = ($s =~ a*)", "$n", "1"),
            ("n = (1 < 2) + 1", "$n", "2"),
            ("n = (2 + 3) =~ 5", "$n", "1"),
            ("n = { true }", "$n", "1"),
            ("n = { false } + 4", "$n", "4"),
            ("n = -e full", "$n", "1"),
            ("n = (-e emp) + 1", "$n", "2"),
            ("n = ()", "$n", "0"),
            ("n = (1 +)", "$n", "1"),
            ("n = $e", "$n", "0"),
            ("n = $x + 1", "$n", "6"),
            ("n = $e + 1", "$n", "1"),
            ("n = 99999999999", "$n", "99999999999"),
            ("n = 2147483648 * 2", "$n", "4294967296"),
            ("n++", "$n", "1"),
            ("x++", "$x", "6"),
            ("x--", "$x", "4"),
            ("x ++", "$x", "6"),
            ("x += 4", "$x", "9"),
            ("x -= 4", "$x", "1"),
            ("x *= 4", "$x", "20"),
            ("x /= 2", "$x", "2"),
            ("x %= 3", "$x", "2"),
            ("x ^= 1", "$x", "4"),
            ("x += 1 + 1", "$x", "7"),
            ("x *= 2 + 1", "$x", "15"),
            ("l[2] = 50", "$l", "7 50 9"),
            ("l[2]++", "$l", "7 9 9"),
            ("l[2] += 5", "$l", "7 13 9"),
        ];
        if zsh("true").is_none() {
            return;
        }
        for (stmt, show, want) in rows {
            let got = value(stmt, show).unwrap();
            assert_eq!(got, *want, "@ {stmt}");
        }
    }

    /// `@` stores a plain string: a later non-numeric assignment must not
    /// be coerced by an integer-typed parameter (tcsh variables are
    /// strings).
    #[test]
    fn at_target_stays_a_scalar() {
        if zsh("true").is_none() {
            return;
        }
        let code = translate_at("n = 3").unwrap();
        let out = zsh(&format!("{code}; n=hello; echo $n")).unwrap();
        assert_eq!(out, "hello");
    }

    /// Message text each case produced from `/bin/tcsh -f`.
    #[test]
    fn syntax_errors_carry_tcsh_text() {
        let cond: &[(&str, &str)] = &[
            ("1==1", "if: Badly formed number."),
            ("1.5 > 1", "if: Badly formed number."),
            ("0x10 + 1", "if: Badly formed number."),
            ("--1", "if: Badly formed number."),
            ("-q emp", "if: Badly formed number."),
            ("abc", "if: Expression Syntax."),
            ("a < b", "if: Expression Syntax."),
            ("1 1", "if: Expression Syntax."),
            ("1 == 1 == 1", "if: Expression Syntax."),
            ("1 =~ 1 =~ 1", "if: Expression Syntax."),
            ("1 && && 1", "if: Expression Syntax."),
            ("(1)(1)", "if: Expression Syntax."),
            ("1 ~ 1", "if: Expression Syntax."),
            ("}", "if: Expression Syntax."),
            ("+1", "if: Expression Syntax."),
            ("{true}", "if: Expression Syntax."),
            ("{ true } 1", "if: Expression Syntax."),
            ("-e emp 1", "if: Expression Syntax."),
            ("(1 == 1", "Too many ('s."),
            ("1 == 1)", "Too many )'s."),
            ("{ true", "if: Missing '}'."),
            ("-e", "if: Missing file name."),
            ("-e == -e", "if: Missing file name."),
            ("-e (emp)", "if: Missing file name."),
            ("-e/tmp", "if: Malformed file inquiry."),
            ("!1", "1: Event not found."),
            ("!x", "x: Event not found."),
        ];
        for (expr, want) in cond {
            assert_eq!(translate_condition(expr).unwrap_err(), *want, "if ({expr})");
        }
        let at: &[(&str, &str)] = &[
            ("n = 1 +", "@: Expression Syntax."),
            ("x += ", "@: Expression Syntax."),
            ("n = abc", "@: Expression Syntax."),
            ("n = 0x10", "@: Badly formed number."),
            ("n = 1.5", "@: Badly formed number."),
            ("n", "@: Assignment missing expression."),
            ("n =", "@: Assignment missing expression."),
            ("1x = 2", "@: Variable name must begin with a letter."),
            ("x = 1 2", "@: Variable name must begin with a letter."),
            ("x++ + 1", "@: Variable name must begin with a letter."),
            ("x-y = 2", "@: Unknown operator."),
            ("x = (1", "Too many ('s."),
            ("x = 1)", "Too many )'s."),
            ("x = {true}", "@: Expression Syntax."),
        ];
        for (body, want) in at {
            assert_eq!(translate_at(body).unwrap_err(), *want, "@ {body}");
        }
    }

    /// `while`/`exit` reuse the condition translator and rewrite the
    /// command prefix tcsh prints (`while: Expression Syntax.`).
    #[test]
    fn error_prefix_follows_the_command() {
        let e = translate_condition("abc").unwrap_err();
        assert_eq!(for_command(&e, "while"), "while: Expression Syntax.");
        assert_eq!(translate_exit("abc").unwrap_err(), "exit: Expression Syntax.");
        let e = translate_condition("(1").unwrap_err();
        assert_eq!(for_command(&e, "while"), "Too many ('s.");
    }

    /// `exit (expr)` takes the numeric value of the expression.
    #[test]
    fn exit_takes_the_numeric_value() {
        if zsh("true").is_none() {
            return;
        }
        for (expr, want) in [("1 + 2", "3"), ("$x == 5", "1"), ("", "0")] {
            let code = translate_exit(expr).unwrap();
            let out = zsh(&format!("{PRELUDE} ( {code} ); echo $?")).unwrap();
            assert_eq!(out, want, "exit ({expr})");
        }
    }

    /// Operators bind only as whole words; `<`/`>`/`(`/`)` split words.
    #[test]
    fn tokenizer_splits_like_the_csh_lexer() {
        let words = |s: &str| -> Vec<String> {
            tokenize(s).unwrap().into_iter().map(|t| t.text).collect()
        };
        assert_eq!(words("1<2"), ["1", "<", "2"]);
        assert_eq!(words("1 <= 2"), ["1", "<=", "2"]);
        assert_eq!(words("(1)&&(2)"), ["(", "1", ")", "&&", "(", "2", ")"]);
        assert_eq!(words("$x==1"), ["$x==1"]);
        assert_eq!(words("\"a b\" == 'c d'"), ["\"a b\"", "==", "'c d'"]);
        assert_eq!(words("${x[1]} == a"), ["${x[1]}", "==", "a"]);
    }
}
