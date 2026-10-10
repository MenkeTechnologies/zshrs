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
//! * a comparison, `{ cmd }` or file-inquiry value used where a number is
//!   needed is first evaluated into a `_csh_tN` temporary, in a `{ ...; }`
//!   group that is itself lazy under `&&`/`||`.
//!
//! # Run-time errors
//!
//! tcsh raises these while it evaluates, so the translation does too: the
//! emitted code prints tcsh's text (with the `if:`/`@:`/`exit:` prefix where
//! tcsh prints one) to stderr and runs `exit 1`, after any earlier output.
//!
//! * a non-numeric operand in numeric position: `Expression Syntax.` when
//!   the text does not start with a digit or `-`, `Badly formed number.`
//!   otherwise. The check is made on the *value*, so `abc`, `1x`, `$s` and
//!   `"$s"` are treated alike;
//! * an unquoted expansion that yields two or more words (`set l = (1 2); if
//!   ($l > 0)`, also `$l == 1`): `Expression Syntax.`, or the `@` leftover
//!   text `Variable name must begin with a letter.` at paren depth 0 of
//!   `@ n = ...`;
//! * an unquoted expansion that yields no word (`set x = ()`, `set x = ""`)
//!   is *absent* from the token stream: before `&&`/`||`, or as the last
//!   token under `@`, that is `Expression Syntax.` (`Assignment missing
//!   expression.` when it is the whole `@ n = $x` value); after `-e` it is
//!   `Missing file name.`; elsewhere it is an empty operand;
//! * `Division by 0.` / `Mod by 0.` (no prefix);
//! * `@ l[i] = e`: `Subscript error.` for a non-digit subscript,
//!   `name: Undefined variable.`, `Subscript out of range.` (checked after
//!   the value is evaluated);
//! * a glob word that matches nothing: `pattern: No match.`.
//!
//! Errors that are about the *shape* of the expression (`1 1`, `1 == 1 ==
//! 1`, `Too many ('s.`, `Missing '}'.`, `Missing file name.` after a
//! literal `-e`, …) are still returned as `Err` from the entry points.
//!
//! # tcsh evaluation rules reproduced
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
//! * A shift does not chain: after one `<<`/`>>` a further `<<` or `>>` is
//!   read as the relational `<`/`>` (`1 << 2 << 3` is `(1 << 2) < 3`, i.e.
//!   0; `256 >> 2 >> 1` is `(256 >> 2) > 1`, i.e. 1).
//! * Numbers are decimal only: `010` is ten, `0x10`/`1.5` are
//!   `Badly formed number.`, a non-numeric word in numeric position is
//!   `Expression Syntax.`; an empty value is 0.
//! * Short-circuiting skips `{ cmd }` and the arithmetic/relational
//!   operators (`+ - * / % < > <= >=`, so `0 && 5 / 0` is fine), but the
//!   operands of `&& || | ^ & << >> ! ~` are converted even when skipped:
//!   `0 && 1x` and `1 || 1x | 1` fail with `Badly formed number.` while
//!   `0 && 1x + 1` does not.
//! * `{ cmd }` runs `cmd` in a subshell, stdout/stderr untouched, and is 1
//!   when it exits 0.
//! * A word with an unquoted `* ? [` is glob-expanded and the matches are
//!   joined with one space into a single operand (`* == *` is true, `-e
//!   *.txt` tests the file named `a.txt b.txt`); `~/x` and `{a,b}` expand
//!   as in words. The right side of `=~`/`!~` stays a pattern.
//! * File inquiry letters may be stacked (`-ez f` is exists-and-empty).
//!   Value operators (`Z N P Pmode U U: G G: A A: M M: C C: D I F L`) must
//!   be last, return `-1` (`:` for `F`) when the file is missing, and are
//!   read through `zstat`; `L` anywhere but last tests the link itself;
//!   `X` looks the name up as a tcsh builtin or in `$PATH`; `m` and `K`
//!   are always 0.
//!
//! # Known divergences
//!
//! * A variable's words are operands, not further tokens: tcsh re-parses
//!   them, so `set l = (1 + 2); if ($l == 3)` is true there and an error
//!   here, and a variable whose value is `-e`, `-` or `==` acts as that
//!   operator in tcsh but is plain text here.
//! * Glob words are expanded where they are evaluated, not before the whole
//!   expression as tcsh does, so a `No match.` inside a skipped branch is
//!   silent. `~user` for an unknown user stays literal (tcsh errors).
//! * `x: Undefined variable.` and `x: Subscript out of range.` for `$x`
//!   operands are raised by tcsh while it expands the line; this layer sees
//!   an empty value (the words layer owns those errors).
//! * An `L` before other letters is honoured for the type, ownership and
//!   size letters only; `r w x t X` still follow the link.
//! * `@` results are stored as scalars, so `$#n` after `@ n = 10` counts
//!   characters.

use super::cmds::translate_line;
use super::words::{translate_word, zsh_var_name};

const SYNTAX: &str = "Expression Syntax.";
const BADNUM: &str = "Badly formed number.";
const NO_FILE: &str = "Missing file name.";
const NO_EXPR: &str = "Assignment missing expression.";
const MALFORMED: &str = "Malformed file inquiry.";
const NAME_ERR: &str = "Variable name must begin with a letter.";
const SUBSCRIPT: &str = "Subscript error.";
const SUBSCRIPT_RANGE: &str = "Subscript out of range.";

/// Start of every run-time error statement the emitter writes; used by
/// [`for_command`] to retarget the command prefix.
const FAIL_HEAD: &str = "print -ru2 -- '";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Translate the text between the outer parens of `if (...)` /
/// `while (...)` / `else if (...)` into a zsh command whose exit status is
/// the csh truth value (usable directly after `if ` / `while `).
///
/// Errors carry tcsh's text with the `if: ` command prefix where tcsh
/// prints one (`if: Expression Syntax.`, `if: Missing file name.`,
/// `if: Missing '}'.`); `Too many ('s.` and `Too many )'s.` are printed by
/// tcsh without a prefix. Value-dependent errors (`Badly formed number.`,
/// `Division by 0.`, …) are not returned: the translation prints them when
/// it runs, with an `if: ` prefix that a `while` caller retargets by
/// passing the returned text through [`for_command`].
pub fn translate_condition(expr: &str) -> Result<String, String> {
    let cond = translate_condition_inner(expr).map_err(|e| prefixed("if", e))?;
    Ok(with_reference_checks(expr, cond))
}

/// tcsh expands every `$x` of an expression before evaluating it, so an
/// undefined variable (or an out-of-range subscript) ends the command with
/// its message first. The operand layer sees an empty value instead, so the
/// checks are run ahead of the condition `cond`.
fn with_reference_checks(expr: &str, cond: String) -> String {
    let mut probes: Vec<String> = Vec::new();
    for word in expr.split_whitespace() {
        for name in super::cmds::variable_refs(word) {
            let probe = format!(
                "{{ (( ${{+{name}}} )) || {{ print -u2 -r -- '{name}: Undefined variable.'; \
[[ -o interactive ]] || exit 1; false; }}; }}"
            );
            if !probes.contains(&probe) {
                probes.push(probe);
            }
        }
        for probe in super::words::subscript_probes(word) {
            if !probes.contains(&probe) {
                probes.push(probe);
            }
        }
    }
    if probes.is_empty() {
        return cond;
    }
    // the probes overwrite `$?`, which the condition may read (`$status`)
    format!(
        "{{ _csh_q=$?; {} && {{ () {{ return $1; }} $_csh_q; {cond}; }}; }}",
        probes.join(" && ")
    )
}

/// Translate the text after `@` (`n = 2 + 3`, `n++`, `x += 4`,
/// `a[2] = 1`, or empty) into zsh.
///
/// * empty: lists the shell variables (the `set` translation).
/// * `name = e`: `name=$(( ... ))` — a plain scalar assignment, so the
///   variable never becomes a zsh integer-typed parameter.
/// * `name op= e` for `+ - * / % ^ | & << >> || &&`: `name=$(( cur op (e) ))`
///   where `cur` is the first word of `$name` (0 when empty or unset).
/// * `name++` / `name--`: `name=$(( cur + 1 ))`.
/// * `name[i] ...`: the subscript is checked (`Subscript error.`,
///   `Undefined variable.`, `Subscript out of range.`) before the element
///   is assigned.
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

/// Re-target a message produced for `if` to another command (`while`,
/// `exit`, …): the error `if: Expression Syntax.` becomes `while:
/// Expression Syntax.`, and every run-time error statement inside
/// translated text (`print -ru2 -- 'if: …'`) gets the new prefix. Messages
/// without the `if: ` prefix pass through.
pub fn for_command(err: &str, cmd: &str) -> String {
    if let Some(rest) = err.strip_prefix("if: ") {
        return format!("{cmd}: {rest}");
    }
    err.replace(&format!("{FAIL_HEAD}if: "), &format!("{FAIL_HEAD}{cmd}: "))
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

/// An operand word and what happens to the expression when its expansion
/// yields no word or several.
#[derive(Debug, Clone)]
struct Word {
    /// The raw csh word (quotes and `$` intact).
    raw: String,
    /// Error when the word expands to nothing and the parse then breaks
    /// (next token is `&&`/`||`, or it is the last token under `@`).
    vanish: Option<&'static str>,
    /// Error when the word expands to two or more words.
    multi: &'static str,
    /// The next token is `&&`/`||` (after `-e` it is taken as the file name).
    logic_next: bool,
}

#[derive(Debug, Clone)]
enum Node {
    /// A missing operand: the empty string, numerically 0.
    Empty,
    Word(Word),
    /// `-ez file`: a file inquiry (test letters, optional value operator).
    File { inq: Inquiry, operand: Word },
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
/// Index of the relational level, which also reads a second `<<`/`>>`.
const RELATIONAL: usize = 6;
/// Index of the shift level, which does not chain.
const SHIFT: usize = 7;

/// How the parser treats the end of the expression.
#[derive(Clone, Copy)]
enum Mode {
    /// `if (...)`: a missing trailing operand is an empty operand.
    Condition,
    /// The value after `@ name =` or `exit`: a missing trailing operand is
    /// `Expression Syntax.`; `leftover` is the error for extra words and
    /// `sole` the error when the whole value is one vanished word.
    Value { leftover: &'static str, sole: &'static str },
}

struct Parser<'a> {
    src: &'a str,
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
    mode: Mode,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str, mode: Mode) -> Result<Self, String> {
        let toks = tokenize(src)?;
        check_parens(&toks)?;
        Ok(Parser { src, toks, pos: 0, depth: 0, mode })
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
        self.binary(0)
    }

    fn at_end(&self) -> bool {
        self.pos >= self.toks.len()
    }

    /// The operator of `level` at the cursor. The relational level also
    /// accepts `<<`/`>>` (a shift that follows a shift) as `<`/`>`.
    fn operator_at(&self, level: usize) -> Option<&'static str> {
        let t = self.peek().filter(|t| t.kind == Kind::Op)?;
        if let Some(op) = LEVELS[level].iter().find(|o| **o == t.text) {
            return Some(op);
        }
        match (level, t.text.as_str()) {
            (RELATIONAL, "<<") => Some("<"),
            (RELATIONAL, ">>") => Some(">"),
            _ => None,
        }
    }

    fn binary(&mut self, level: usize) -> Result<Node, String> {
        if level == LEVELS.len() {
            return self.primary();
        }
        let mut left = self.binary(level + 1)?;
        while let Some(op) = self.operator_at(level) {
            self.pos += 1;
            let right = if matches!(op, "=~" | "!~") { self.string_operand()? } else { self.binary(level + 1)? };
            left = Node::Bin(op, Box::new(left), Box::new(right));
            if level == EQUALITY || level == SHIFT {
                break;
            }
        }
        Ok(left)
    }

    fn strict_end(&self) -> bool {
        matches!(self.mode, Mode::Value { .. })
    }

    /// Build the operand for the word token at `idx`.
    fn word(&self, raw: String, idx: usize) -> Word {
        let (leftover, sole) = match self.mode {
            Mode::Value { leftover, sole } => (leftover, sole),
            Mode::Condition => (SYNTAX, SYNTAX),
        };
        let before_logic = matches!(
            self.toks.get(idx + 1),
            Some(t) if t.kind == Kind::Op && (t.text == "&&" || t.text == "||")
        );
        let vanish = if before_logic {
            Some(SYNTAX)
        } else if self.strict_end() && self.depth == 0 && idx + 1 == self.toks.len() {
            Some(if idx == 0 { sole } else { SYNTAX })
        } else {
            None
        };
        Word {
            raw,
            vanish,
            multi: if self.depth == 0 { leftover } else { SYNTAX },
            logic_next: before_logic,
        }
    }

    fn primary(&mut self) -> Result<Node, String> {
        let Some(tok) = self.peek().cloned() else {
            return if self.strict_end() && self.depth == 0 {
                Err(SYNTAX.into())
            } else {
                Ok(Node::Empty)
            };
        };
        if tok.kind == Kind::Word {
            let idx = self.pos;
            self.pos += 1;
            return match file_inquiry(&tok.text)? {
                Some(inq) => self.file_operand(inq),
                None => Ok(Node::Word(self.word(tok.text, idx))),
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

    /// Right operand of `=~`/`!~`. A bare `*`, `+`, `-`, `/`, `%`, `^` there
    /// is the literal pattern (`abc =~ *` matches), not an empty operand;
    /// after `==`/`!=` it is an operator between empty operands (`* == *`
    /// compares `0` with `0`).
    fn string_operand(&mut self) -> Result<Node, String> {
        if let Some(t) = self.peek() {
            if t.kind == Kind::Op && matches!(t.text.as_str(), "*" | "+" | "-" | "/" | "%" | "^") {
                let raw = t.text.clone();
                let idx = self.pos;
                self.pos += 1;
                return Ok(Node::Word(self.word(raw, idx)));
            }
        }
        self.binary(EQUALITY + 1)
    }

    /// The file word after `-e` etc.; a following operator (or nothing) is
    /// `Missing file name.`. A lone `/` is a
    /// file name (the root directory), not the division operator.
    fn file_operand(&mut self, inq: Inquiry) -> Result<Node, String> {
        match self.peek() {
            Some(t) if t.kind == Kind::Word || (t.kind == Kind::Op && t.text == "/") => {
                let (raw, idx) = (t.text.clone(), self.pos);
                self.pos += 1;
                let operand = self.word(raw, idx);
                Ok(Node::File { inq, operand })
            }
            _ => Err(NO_FILE.into()),
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

// ---------------------------------------------------------------------------
// File inquiry
// ---------------------------------------------------------------------------

/// Yes/no inquiry letters (`X` is a command lookup; `m` and `K` are
/// always false in tcsh 6.21 on this platform).
const TEST_LETTERS: &str = "edfrwxozslbcpSugktXmK";
/// First letters of the value operators.
const VALUE_LETTERS: &str = "ZNPUGAMCDIFL";

/// tcsh builtins `-X name` accepts besides commands found in `$PATH`.
const BUILTINS: &str = "alias|alloc|bg|bindkey|break|breaksw|builtins|bye|case|cd|chdir|complete|\
continue|default|dirs|echo|echotc|else|end|endif|endsw|eval|exec|exit|fg|filetest|foreach|\
getspath|getxvers|glob|goto|hashstat|history|hup|if|jobs|kill|limit|log|login|logout|ls-F|\
migrate|newgrp|nice|nohup|notify|onintr|popd|printenv|pushd|rehash|repeat|rootnode|sched|set|\
setenv|setpath|setspath|settc|setty|setxvers|shift|source|stop|suspend|switch|telltc|termname|\
time|umask|unalias|uncomplete|unhash|universe|unlimit|unset|unsetenv|ver|wait|warp|watchlog|\
where|which|while";

/// The value a file-inquiry operator returns instead of 0/1.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Size,
    Links,
    Device,
    Inode,
    /// `-F`: `device:inode`.
    FileId,
    /// `-L` as the last letter: the symlink target.
    LinkTarget,
    /// `-A -M -C`; `text` is the `ctime` form (`-A:`).
    Time { field: &'static str, text: bool },
    /// `-P`, `-P:`, `-Pmode`, `-Pmode:`.
    Perm { mask: Option<u32>, zero: bool },
    /// `-U -G`; `name` is the `-U:` form.
    Owner { field: &'static str, name: bool },
}

/// A parsed `-ez` / `-fZ` / `-P22:` word.
#[derive(Debug, Clone, PartialEq)]
struct Inquiry {
    /// Yes/no letters, every one of which must hold.
    tests: String,
    /// An `L` before the last letter: look at the link, not its target.
    lstat: bool,
    value: Option<Value>,
}

/// Consume a `:` at `chars[*i]`.
fn take_colon(chars: &[char], i: &mut usize) -> bool {
    let has = chars.get(*i) == Some(&':');
    *i += usize::from(has);
    has
}

/// Classify a word as a file inquiry. `Ok(None)` when it is an ordinary
/// operand (`-5`, `-q`); `Ok(Some(..))` for `-e`, `-ez`, `-M:`, …; an
/// error for `-e/tmp` or a value operator that is not last
/// (`Malformed file inquiry.`).
fn file_inquiry(word: &str) -> Result<Option<Inquiry>, String> {
    let Some(rest) = word.strip_prefix('-') else { return Ok(None) };
    let chars: Vec<char> = rest.chars().collect();
    match chars.first() {
        Some(c) if TEST_LETTERS.contains(*c) || VALUE_LETTERS.contains(*c) => {}
        _ => return Ok(None),
    }
    let mut inq = Inquiry { tests: String::new(), lstat: false, value: None };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        if inq.value.is_some() {
            return Err(MALFORMED.into());
        }
        if TEST_LETTERS.contains(c) {
            inq.tests.push(c);
            continue;
        }
        if c == 'L' && i < chars.len() {
            inq.lstat = true;
            continue;
        }
        inq.value = Some(match c {
            'Z' => Value::Size,
            'N' => Value::Links,
            'D' => Value::Device,
            'I' => Value::Inode,
            'F' => Value::FileId,
            'L' => Value::LinkTarget,
            'A' => Value::Time { field: "atime", text: take_colon(&chars, &mut i) },
            'M' => Value::Time { field: "mtime", text: take_colon(&chars, &mut i) },
            'C' => Value::Time { field: "ctime", text: take_colon(&chars, &mut i) },
            'U' => Value::Owner { field: "uid", name: take_colon(&chars, &mut i) },
            'G' => Value::Owner { field: "gid", name: take_colon(&chars, &mut i) },
            'P' => {
                let start = i;
                while chars.get(i).is_some_and(|d| ('0'..='7').contains(d)) {
                    i += 1;
                }
                let digits: String = chars[start..i].iter().collect();
                let mask = u32::from_str_radix(&digits, 8).ok();
                Value::Perm { mask, zero: take_colon(&chars, &mut i) }
            }
            _ => return Err(MALFORMED.into()),
        });
    }
    Ok(Some(inq))
}

// ---------------------------------------------------------------------------
// Word classification
// ---------------------------------------------------------------------------

/// What a csh operand word turns into at run time.
#[derive(Debug, PartialEq)]
enum Shape {
    /// No expansion: exactly this string, one word.
    Literal(String),
    /// An expansion that is always one decimal number (`$#x`, `$?x`, `$$`).
    Count,
    /// Expansions only inside double quotes: always exactly one word.
    Quoted,
    /// An unquoted variable or command expansion: zero or more words.
    Split,
    /// An unquoted glob pattern, brace list or `~`: expanded, the results
    /// joined with a space into one word.
    Glob,
}

/// What a scan of the raw word found outside single quotes.
#[derive(Default)]
struct WordInfo {
    unquoted_expansion: bool,
    quoted_expansion: bool,
    glob: bool,
    brace: bool,
    tilde: bool,
}

/// Index just past the `$...` variable reference whose `$` is at
/// `chars[i]`, so a `[1]` subscript or `$?` is not read as a glob.
fn skip_reference(chars: &[char], i: usize) -> usize {
    let at = |j: usize| chars.get(j).copied();
    let mut j = i + 1;
    match at(j) {
        Some('{') => {
            while at(j).is_some_and(|c| c != '}') {
                j += 1;
            }
            j + 1
        }
        Some('#' | '?') => {
            j += 1;
            while at(j).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                j += 1;
            }
            j
        }
        Some(c) if c.is_ascii_alphanumeric() || c == '_' => {
            while at(j).is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                j += 1;
            }
            if at(j) == Some('[') {
                while at(j).is_some_and(|c| c != ']') {
                    j += 1;
                }
                j += 1;
            }
            j
        }
        _ => j + 1,
    }
}

fn scan_word(raw: &str) -> WordInfo {
    let chars: Vec<char> = raw.chars().collect();
    let mut info = WordInfo::default();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match (quote, c) {
            (Some('\''), '\'') => quote = None,
            (Some('\''), _) => {}
            (Some(_), '"') => quote = None,
            (Some(_), '\\') => i += 1,
            (Some(_), '$' | '`') => info.quoted_expansion = true,
            (Some(_), _) => {}
            (None, '\\') => i += 1,
            (None, '\'' | '"') => quote = Some(c),
            (None, '$') => {
                info.unquoted_expansion = true;
                i = skip_reference(&chars, i) - 1;
            }
            (None, '`') => {
                info.unquoted_expansion = true;
                i += 1;
                while i < chars.len() && chars[i] != '`' {
                    i += 1;
                }
            }
            (None, '*' | '?') => info.glob = true,
            (None, '[') if chars[i + 1..].contains(&']') => info.glob = true,
            (None, '{') => info.brace = true,
            (None, '~') if i == 0 => info.tilde = true,
            (None, _) => {}
        }
        i += 1;
    }
    info
}

/// A translated word that is always one decimal number.
fn is_count(w: &str) -> bool {
    if w == "$$" || w == "$?" {
        return true;
    }
    let name = w
        .strip_prefix("${#")
        .or_else(|| w.strip_prefix("${+"))
        .and_then(|r| r.strip_suffix('}'));
    name.is_some_and(|n| {
        !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn classify(raw: &str) -> Shape {
    let info = scan_word(raw);
    if info.glob || info.tilde || info.brace {
        Shape::Glob
    } else if info.unquoted_expansion {
        if is_count(&translate_word(raw)) {
            Shape::Count
        } else {
            Shape::Split
        }
    } else if info.quoted_expansion {
        Shape::Quoted
    } else {
        Shape::Literal(unquote(raw))
    }
}

/// The csh value of a word without expansions: quotes removed, an unquoted
/// backslash escapes the next character, and inside `"..."` a backslash is
/// literal.
fn unquote(raw: &str) -> String {
    let mut out = String::new();
    let mut quote: Option<char> = None;
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '\'' | '"') => quote = Some(c),
            (Some(q), _) if c == q => quote = None,
            (None, '\\') => out.extend(chars.next()),
            _ => out.push(c),
        }
    }
    out
}

/// tcsh's numeric conversion of a literal: empty is 0; a leading `-` and
/// decimal digits; anything else is an error (`Expression Syntax.` when it
/// does not even start like a number, `Badly formed number.` otherwise).
fn literal_number(s: &str) -> Result<String, &'static str> {
    if s.is_empty() {
        return Ok("0".into());
    }
    if !s.starts_with('-') && !s.starts_with(|c: char| c.is_ascii_digit()) {
        return Err(SYNTAX);
    }
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(BADNUM);
    }
    let trimmed = digits.trim_start_matches('0');
    Ok(match (trimmed.is_empty(), s.starts_with('-')) {
        (true, _) => "0".into(),
        (false, true) => format!("-{trimmed}"),
        (false, false) => trimmed.to_string(),
    })
}

/// True when evaluating `w` as a number can end the script.
fn may_abort(w: &Word) -> bool {
    match classify(&w.raw) {
        Shape::Count => false,
        Shape::Literal(v) => literal_number(&v).is_err(),
        _ => true,
    }
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

/// A numeric operand: either ready arithmetic, or a word whose string →
/// number conversion tcsh performs only after both operands are evaluated.
enum Operand<'a> {
    Ready(Arith),
    Pending(&'a Word),
}

/// One piece of a file-inquiry test: an expression for `[[ ]]`, or a
/// complete command.
enum Frag {
    Dbl(String),
    Cmd(String),
}

struct Emitter {
    temps: usize,
    /// Command whose prefix run-time errors carry (`if`, `@`, `exit`).
    cmd: &'static str,
    /// A word read `$status`: when a guard or `{ cmd }` runs before it, `$?`
    /// is copied to `_csh_st` first and the word reads the copy.
    status: bool,
    /// A `{ cmd }` was emitted (it changes `$?`).
    ran_cmd: bool,
    /// First literal that is not a number, for the leftover-word rule.
    static_error: Option<String>,
}

/// Operators that yield a truth value through `[[ ]]` / a command.
fn is_cond_only(n: &Node) -> bool {
    match n {
        Node::File { inq, .. } => inq.value.is_none(),
        Node::Cmd(_) => true,
        Node::Bin(op, _, _) => LEVELS[EQUALITY].contains(op),
        _ => false,
    }
}

/// True when `n` contains something that cannot be written in `(( ))`.
fn has_cond(n: &Node) -> bool {
    match n {
        Node::Empty | Node::Word(_) => false,
        Node::File { inq, .. } => inq.value.is_none(),
        Node::Cmd(_) => true,
        Node::Not(x) | Node::BitNot(x) => has_cond(x),
        Node::Bin(op, l, r) => LEVELS[EQUALITY].contains(op) || has_cond(l) || has_cond(r),
    }
}

/// True when `n` is a number literal other than zero.
fn nonzero_literal(n: &Node) -> bool {
    match n {
        Node::Word(w) => matches!(
            classify(&w.raw),
            Shape::Literal(v) if literal_number(&v).is_ok_and(|n| n != "0")
        ),
        _ => false,
    }
}

/// True when evaluating `n` can run a command, read a file, or abort the
/// script, so `&&`/`||` over it must be real shell logic.
fn has_effects(n: &Node) -> bool {
    match n {
        Node::Empty => false,
        Node::Word(w) => may_abort(w),
        Node::File { .. } | Node::Cmd(_) => true,
        Node::Not(x) | Node::BitNot(x) => has_effects(x),
        Node::Bin(op, l, r) => {
            LEVELS[EQUALITY].contains(op)
                || has_effects(l)
                || has_effects(r)
                || (matches!(*op, "/" | "%") && !nonzero_literal(r))
        }
    }
}

/// Logical nodes whose truth is best computed by shell `&&`/`||`/`!`.
fn is_shell_logic(n: &Node) -> bool {
    match n {
        Node::Not(x) => has_cond(x),
        Node::Bin(op, _, _) => (*op == "&&" || *op == "||") && has_effects(n),
        _ => false,
    }
}

/// `'` quoting for a zsh message.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

impl Emitter {
    fn new(cmd: &'static str) -> Self {
        Emitter { temps: 0, cmd, status: false, ran_cmd: false, static_error: None }
    }

    fn next(&mut self) -> usize {
        self.temps += 1;
        self.temps
    }

    fn temp(&mut self) -> String {
        format!("_csh_t{}", self.next())
    }

    fn array(&mut self) -> String {
        format!("_csh_a{}", self.next())
    }

    /// Statement that prints `msg` as tcsh would and aborts. `prefixed`
    /// adds the command name (`if: `); tcsh omits it for `Division by 0.`
    /// and `No match.`.
    fn fail(&self, msg: &str, prefixed: bool) -> String {
        let text = if prefixed { format!("{}: {msg}", self.cmd) } else { msg.to_string() };
        format!("print -ru2 -- {}; exit 1", sh_quote(&text))
    }

    /// Wrap `pre` statements and `test` into one command.
    fn with_pre(pre: Vec<String>, test: String) -> Cond {
        if pre.is_empty() {
            Cond { text: test, simple: true }
        } else {
            Cond { text: format!("{{ {}; {}; }}", pre.join("; "), test), simple: true }
        }
    }

    /// Finish a condition: save `$?` first when a word reads `$status` and
    /// something runs before it.
    fn finish(&self, c: Cond) -> String {
        match (self.status, self.disturbs(&c.text)) {
            (true, true) => format!("{{ _csh_st=$?; {}; }}", c.text),
            (true, false) => c.text.replace("$_csh_st", "$?"),
            _ => c.text,
        }
    }

    /// True when something may run before a `$status` read in `text`: a
    /// guard, a `{ cmd }`, or the left half of an `&&`/`||` list.
    fn disturbs(&self, text: &str) -> bool {
        self.temps > 0 || self.ran_cmd || text.contains(" && ") || text.contains(" || ")
    }

    /// `pre` statements followed by `stmt`, as one command.
    fn group(&self, pre: Vec<String>, stmt: String) -> String {
        let mut text = if pre.is_empty() {
            stmt
        } else {
            format!("{{ {}; {stmt}; }}", pre.join("; "))
        };
        if self.status {
            text = if self.disturbs(&text) {
                format!("{{ _csh_st=$?; {text}; }}")
            } else {
                text.replace("$_csh_st", "$?")
            };
        }
        text
    }

    /// The zsh text of a csh word.
    fn word_text(&mut self, raw: &str) -> String {
        let w = translate_word(raw);
        if w.contains("$?") {
            self.status = true;
            w.replace("$?", "$_csh_st")
        } else {
            w
        }
    }

    // ---- words -----------------------------------------------------------

    /// `_csh_aN=( ... )` for an unquoted word, with the tcsh no-match error
    /// when it holds a glob. Returns the array name.
    fn elements(&mut self, w: &Word, glob: bool, pre: &mut Vec<String>) -> String {
        let a = self.array();
        let text = self.word_text(&w.raw);
        if glob && scan_word(&w.raw).glob {
            let pattern = unquote(&w.raw);
            let nomatch = self.fail(&format!("{pattern}: No match."), false);
            pre.push(format!("{a}=( {text}(N) ); (( $#{a} )) || {{ {nomatch}; }}"));
        } else {
            pre.push(format!("{a}=( {text} )"));
        }
        a
    }

    /// `case` that checks the element count of `a`: `zero` / `many` are the
    /// errors for no word / several words (`None` accepts them).
    fn count_guard(&self, a: &str, zero: Option<&str>, many: Option<&str>) -> String {
        let arm = |m: Option<&str>| match m {
            Some(m) => format!("{};;", self.fail(m, true)),
            None => ";;".to_string(),
        };
        format!("case $#{a} in (0) {} (1) ;; (*) {} esac", arm(zero), arm(many))
    }

    /// `case` that checks that the first word of `a` is a number (and the
    /// element count, as [`Emitter::count_guard`]).
    fn numeric_guard(&self, a: &str, zero: Option<&str>, many: Option<&str>) -> String {
        let fail = |m: &str| format!("{};;", self.fail(m, true));
        let zero_arm = match zero {
            Some(m) => format!("(0:) {}", fail(m)),
            None => String::new(),
        };
        let ok = if zero.is_some() { "(1:|1:<->|1:-<->)" } else { "(0:|1:|1:<->|1:-<->)" };
        let many_arm = match many {
            Some(m) => format!("(<->:|<->:<->|<->:-<->) {}", fail(m)),
            None => "(<->:|<->:<->|<->:-<->) ;;".to_string(),
        };
        format!(
            "case $#{a}:${{{a}[1]}} in {zero_arm} {ok} ;; {many_arm} (<->:[-0-9]*) {} (*) {} esac",
            fail(BADNUM),
            fail(SYNTAX)
        )
    }

    /// `case` that checks that the scalar `t` is a number.
    fn scalar_guard(&self, t: &str) -> String {
        format!(
            "case ${t} in (''|<->|-<->) ;; ([-0-9]*) {};; (*) {};; esac",
            self.fail(BADNUM, true),
            self.fail(SYNTAX, true)
        )
    }

    /// The numeric value of operand word `w`: validated now when it is a
    /// literal, otherwise by guard statements pushed on `pre`.
    fn convert(&mut self, w: &Word, pre: &mut Vec<String>) -> Arith {
        match classify(&w.raw) {
            Shape::Literal(v) => match literal_number(&v) {
                Ok(n) if n.starts_with('-') => Arith { text: n, atomic: false },
                Ok(n) => Arith::atom(n),
                Err(msg) => {
                    self.static_error.get_or_insert_with(|| msg.to_string());
                    pre.push(self.fail(msg, true));
                    Arith::atom("0".into())
                }
            },
            Shape::Count => Arith::atom(self.word_text(&w.raw)),
            Shape::Quoted => {
                let t = self.temp();
                let text = self.word_text(&w.raw);
                pre.push(format!("{t}={text}; {}", self.scalar_guard(&t)));
                Arith::atom(t)
            }
            Shape::Split => {
                let (a, t) = (self.array(), self.temp());
                let text = self.word_text(&w.raw);
                let guard = self.numeric_guard(&a, w.vanish, Some(w.multi));
                pre.push(format!("{a}=( {text} ); {guard}; {t}=${{{a}[1]}}"));
                Arith::atom(t)
            }
            Shape::Glob => {
                let t = self.temp();
                let a = self.elements(w, true, pre);
                pre.push(format!("{t}=\"${{{a}[*]}}\"; {}", self.scalar_guard(&t)));
                Arith::atom(t)
            }
        }
    }

    /// The string value of operand word `w` as one quoted zsh word.
    fn word_string(&mut self, w: &Word, pre: &mut Vec<String>) -> String {
        match classify(&w.raw) {
            Shape::Glob => {
                let a = self.elements(w, true, pre);
                format!("\"${{{a}[*]}}\"")
            }
            Shape::Split => {
                let a = self.elements(w, false, pre);
                pre.push(self.count_guard(&a, w.vanish, Some(w.multi)));
                format!("\"${{{a}[*]}}\"")
            }
            Shape::Literal(v) if w.raw.contains('\\') => sh_quote(&v),
            _ => {
                let text = self.word_text(&w.raw);
                quote_string(&text)
            }
        }
    }

    /// The file word after `-e`: no word is `Missing file name.`, several
    /// are `Expression Syntax.`.
    fn file_word(&mut self, w: &Word, pre: &mut Vec<String>) -> String {
        match classify(&w.raw) {
            Shape::Glob => self.word_string(w, pre),
            Shape::Split => {
                let a = self.elements(w, false, pre);
                let missing = if w.logic_next { SYNTAX } else { NO_FILE };
                pre.push(self.count_guard(&a, Some(missing), Some(w.multi)));
                format!("\"${{{a}[1]}}\"")
            }
            _ => self.word_string(w, pre),
        }
    }

    // ---- file inquiry ----------------------------------------------------

    /// Test of one inquiry letter on the link itself, from `zstat -L`
    /// (the stat array: mode is element 3, uid 5, size 8).
    fn lstat_fragment(&mut self, c: char, file: &str) -> Option<Frag> {
        let kind = |bits: u32| format!("(_A[3] & 8#170000) == 8#{bits:o}");
        let test = match c {
            'e' => "1".to_string(),
            'd' => kind(0o040000),
            'f' => kind(0o100000),
            'l' => kind(0o120000),
            'b' => kind(0o060000),
            'c' => kind(0o020000),
            'p' => kind(0o010000),
            'S' => kind(0o140000),
            'u' => "_A[3] & 8#4000".to_string(),
            'g' => "_A[3] & 8#2000".to_string(),
            'k' => "_A[3] & 8#1000".to_string(),
            'o' => "_A[5] == UID".to_string(),
            'z' => "_A[8] == 0".to_string(),
            's' => "_A[8] > 0".to_string(),
            _ => return None,
        };
        let a = self.array();
        Some(Frag::Cmd(format!(
            "{{ zmodload -F zsh/stat b:zstat 2>/dev/null; \
             zstat -L -A {a} -- {file} 2>/dev/null && (( {} )); }}",
            test.replace("_A", &a)
        )))
    }

    /// The yes/no letters of `inq` on the quoted file word as one command.
    fn test_command(&mut self, inq: &Inquiry, file: &str) -> Cond {
        let mut frags = Vec::new();
        for c in inq.tests.chars() {
            if inq.lstat {
                if let Some(f) = self.lstat_fragment(c, file) {
                    frags.push(f);
                    continue;
                }
            }
            frags.push(match c {
                'z' => Frag::Dbl(format!("-e {file} && ! -s {file}")),
                'o' => Frag::Dbl(format!("-O {file}")),
                'l' => Frag::Dbl(format!("-L {file}")),
                't' => Frag::Dbl(format!("{file} == <-> && -t {file}")),
                'X' => Frag::Cmd(format!(
                    "case {file} in (*/*) false;; ({BUILTINS}) true;; \
                     (*) whence -p -- {file} >/dev/null;; esac"
                )),
                'm' | 'K' => Frag::Cmd("false".into()),
                c => Frag::Dbl(format!("-{c} {file}")),
            });
        }
        let mut parts: Vec<String> = Vec::new();
        let mut dbl: Vec<String> = Vec::new();
        let flush = |parts: &mut Vec<String>, dbl: &mut Vec<String>| {
            if !dbl.is_empty() {
                parts.push(format!("[[ {} ]]", dbl.join(" && ")));
                dbl.clear();
            }
        };
        for f in frags {
            match f {
                Frag::Dbl(s) => dbl.push(s),
                Frag::Cmd(c) => {
                    flush(&mut parts, &mut dbl);
                    parts.push(c);
                }
            }
        }
        flush(&mut parts, &mut dbl);
        Cond { simple: parts.len() == 1, text: parts.join(" && ") }
    }

    /// Statements that put the value operator's result for `file` in a
    /// new temporary (`-1`, or `:` for `-F`, when the file is missing; 0
    /// when a preceding test letter fails). Returns the temporary.
    fn file_value(&mut self, inq: &Inquiry, file: &str, pre: &mut Vec<String>) -> String {
        let Some(value) = &inq.value else { unreachable!("test inquiries have no value") };
        let (t, a) = (self.temp(), self.array());
        let link = if inq.lstat { "-L " } else { "" };
        // `zstat` accepts one `+field`; with none it stores every field
        // (device inode mode nlink uid gid rdev size atime mtime ctime ...).
        let load = |flags: &str, field: &str| {
            let field = if field.is_empty() { String::new() } else { format!("{field} ") };
            format!("zstat {link}{flags}-A {a} {field}-- {file} 2>/dev/null")
        };
        let first = format!("${{{a}[1]}}");
        let simple = |fields: &str, flags: &str| {
            format!("{t}=-1; {} && {t}={first}", load(flags, fields))
        };
        let body = match value {
            Value::Size => simple("+size", ""),
            Value::Links => simple("+nlink", ""),
            Value::Device => simple("+device", ""),
            Value::Inode => simple("+inode", ""),
            Value::Time { field, text: false } => simple(&format!("+{field}"), ""),
            Value::Time { field, text: true } => {
                simple(&format!("+{field}"), "-F '%a %b %e %H:%M:%S %Y' ")
            }
            Value::Owner { field, name: false } => simple(&format!("+{field}"), ""),
            Value::Owner { field, name: true } => simple(&format!("+{field}"), "-s "),
            Value::FileId => format!(
                "{t}=:; {} && {t}=${{{a}[1]}}:${{{a}[2]}}",
                load("", "")
            ),
            Value::LinkTarget => format!(
                "{t}=-1; {} && [[ -n {first} ]] && {t}={first}",
                load("", "+link")
            ),
            Value::Perm { mask, zero } => {
                let mask = mask.map_or(String::new(), |m| format!(" & 8#{m:o}"));
                let lead = if *zero { "0" } else { "" };
                format!(
                    "{t}=-1; {} && {t}={lead}$(( [##8] {first} & 8#7777{mask} ))",
                    load("", "+mode")
                )
            }
        };
        let body = format!("zmodload -F zsh/stat b:zstat 2>/dev/null; {body}");
        if inq.tests.is_empty() {
            pre.push(body);
        } else {
            let tests = self.test_command(inq, file);
            pre.push(format!("if {}; then {body}; else {t}=0; fi", tests.text));
        }
        t
    }

    // ---- conditions ------------------------------------------------------

    fn cond(&mut self, n: &Node) -> Result<Cond, String> {
        match n {
            Node::Cmd(text) => {
                self.ran_cmd = true;
                let body = if text.is_empty() { "true".to_string() } else { translate_line(text)? };
                Ok(Cond { text: format!("( {} )", body.trim()), simple: true })
            }
            Node::File { inq, operand } if inq.value.is_none() => {
                let mut pre = Vec::new();
                let file = self.file_word(operand, &mut pre);
                let test = self.test_command(inq, &file);
                if pre.is_empty() {
                    Ok(test)
                } else {
                    Ok(Self::with_pre(pre, test.text))
                }
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
            Node::Bin(op, l, r) if (*op == "&&" || *op == "||") && has_effects(n) => {
                let (lc, rc) = (self.cond(l)?, self.cond(r)?);
                // tcsh still converts the operands of `&& || | ^ & << >> ! ~`
                // inside the half it skips.
                let mut skipped = Vec::new();
                let pending = self.skipped(r, &mut skipped);
                self.settle_skipped(pending, &mut skipped);
                Ok(match (skipped.is_empty(), *op) {
                    (true, _) => Cond {
                        text: format!("{} {op} {}", lc.grouped(), rc.grouped()),
                        simple: false,
                    },
                    (false, "&&") => Cond {
                        text: format!(
                            "if {}; then {}; else {}; false; fi",
                            lc.text,
                            rc.text,
                            skipped.join("; ")
                        ),
                        simple: true,
                    },
                    (false, _) => Cond {
                        text: format!(
                            "if {}; then {}; true; else {}; fi",
                            lc.text,
                            skipped.join("; "),
                            rc.text
                        ),
                        simple: true,
                    },
                })
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

    /// Walk the half of `&&`/`||` that tcsh skips, pushing the conversion
    /// checks it still makes. Returns the word the node evaluates to, which
    /// the consumer converts.
    fn skipped<'a>(&mut self, n: &'a Node, out: &mut Vec<String>) -> Option<&'a Word> {
        match n {
            Node::Word(w) => Some(w),
            Node::Empty | Node::Cmd(_) | Node::File { .. } => None,
            Node::Not(x) | Node::BitNot(x) => {
                let p = self.skipped(x, out);
                self.settle_skipped(p, out);
                None
            }
            Node::Bin(op, l, r) => {
                let pl = self.skipped(l, out);
                if matches!(*op, "&&" | "||") {
                    // converted before the right side is evaluated
                    self.settle_skipped(pl, out);
                    let pr = self.skipped(r, out);
                    self.settle_skipped(pr, out);
                } else {
                    let pr = self.skipped(r, out);
                    if matches!(*op, "|" | "^" | "&" | "<<" | ">>") {
                        self.settle_skipped(pl, out);
                        self.settle_skipped(pr, out);
                    }
                }
                None
            }
        }
    }

    fn settle_skipped(&mut self, w: Option<&Word>, out: &mut Vec<String>) {
        if let Some(w) = w {
            self.convert(w, out);
        }
    }

    // ---- arithmetic ------------------------------------------------------

    fn operand<'a>(&mut self, n: &'a Node, pre: &mut Vec<String>) -> Result<Operand<'a>, String> {
        match n {
            Node::Word(w) => Ok(Operand::Pending(w)),
            _ => Ok(Operand::Ready(self.arith(n, pre)?)),
        }
    }

    fn settle(&mut self, o: Operand, pre: &mut Vec<String>) -> Arith {
        match o {
            Operand::Ready(a) => a,
            Operand::Pending(w) => self.convert(w, pre),
        }
    }

    /// Guard a divisor: `Division by 0.` / `Mod by 0.` at run time.
    fn divisor(&mut self, op: &str, d: Arith, pre: &mut Vec<String>) -> Arith {
        if d.text.parse::<i64>().is_ok_and(|v| v != 0) {
            return d;
        }
        let t = self.temp();
        let msg = if op == "%" { "Mod by 0." } else { "Division by 0." };
        pre.push(format!("{t}=$(( {} )); (( {t} )) || {{ {}; }}", d.text, self.fail(msg, false)));
        Arith::atom(t)
    }

    /// Arithmetic text for `n`; comparisons and commands are evaluated
    /// into a temporary pushed on `pre`, conversions are checked there.
    fn arith(&mut self, n: &Node, pre: &mut Vec<String>) -> Result<Arith, String> {
        if is_cond_only(n) || is_shell_logic(n) {
            let c = self.cond(n)?;
            let t = self.temp();
            pre.push(format!("{}; {t}=$(( $? == 0 ))", c.text));
            return Ok(Arith::atom(t));
        }
        match n {
            Node::Empty => Ok(Arith::atom("0".into())),
            Node::Word(w) => Ok(self.convert(w, pre)),
            Node::Not(x) | Node::BitNot(x) => {
                let o = self.operand(x, pre)?;
                let a = self.settle(o, pre);
                let sign = if matches!(n, Node::Not(_)) { "!" } else { "~" };
                Ok(Arith { text: format!("{sign}{}", a.operand()), atomic: false })
            }
            Node::Bin(op, l, r) => {
                let lo = self.operand(l, pre)?;
                let ro = self.operand(r, pre)?;
                let la = self.settle(lo, pre);
                let mut ra = self.settle(ro, pre);
                if matches!(*op, "/" | "%") {
                    ra = self.divisor(op, ra, pre);
                }
                Ok(Arith { text: format!("{} {op} {}", la.operand(), ra.operand()), atomic: false })
            }
            Node::File { inq, operand } => {
                let file = self.file_word(operand, pre);
                let t = self.file_value(inq, &file, pre);
                pre.push(self.scalar_guard(&t));
                Ok(Arith::atom(t))
            }
            Node::Cmd(_) => unreachable!("handled as cond-only"),
        }
    }

    /// The string value of `n` as one quoted zsh word.
    fn string(&mut self, n: &Node, pre: &mut Vec<String>) -> Result<String, String> {
        match n {
            Node::Empty => Ok("\"\"".into()),
            Node::Word(w) => Ok(self.word_string(w, pre)),
            Node::File { inq, operand } if inq.value.is_some() => {
                let file = self.file_word(operand, pre);
                let t = self.file_value(inq, &file, pre);
                Ok(format!("\"${{{t}}}\""))
            }
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
            Node::Word(w) => {
                let pat = glob_pattern(&self.word_text(&w.raw));
                Ok(if pat.is_empty() { "\"\"".into() } else { pat })
            }
            _ => self.string(n, pre),
        }
    }
}

fn is_integer(s: &str) -> bool {
    let digits = s.strip_prefix('-').unwrap_or(s);
    !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())
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
    let mut p = Parser::new(expr, Mode::Condition)?;
    if p.at_end() {
        return Ok("false".into());
    }
    let node = p.parse()?;
    let mut em = Emitter::new("if");
    let cond = em.cond(&node)?;
    if !p.at_end() {
        // tcsh converts the value before it notices leftover words.
        return Err(em.static_error.take().unwrap_or_else(|| SYNTAX.into()));
    }
    Ok(em.finish(cond))
}

/// Parse `expr` as the value expression after `@ name =` / `exit` and
/// emit its arithmetic into `em`/`pre`.
fn value_of(
    em: &mut Emitter,
    expr: &str,
    leftover: &'static str,
    sole: &'static str,
    pre: &mut Vec<String>,
) -> Result<Arith, String> {
    let mut p = Parser::new(expr, Mode::Value { leftover, sole })?;
    let node = p.parse()?;
    let a = em.arith(&node, pre)?;
    if !p.at_end() {
        return Err(em.static_error.take().unwrap_or_else(|| leftover.into()));
    }
    Ok(a)
}

fn translate_exit_inner(expr: &str) -> Result<String, String> {
    let mut em = Emitter::new("exit");
    let mut pre = Vec::new();
    let a = if expr.trim().is_empty() {
        Arith::atom("0".into())
    } else {
        value_of(&mut em, expr, SYNTAX, SYNTAX, &mut pre)?
    };
    Ok(em.group(pre, format!("exit $(( {} ))", a.text)))
}

/// Assignment operators, longest first.
const ASSIGN_OPS: [&str; 15] = [
    "<<=", ">>=", "||=", "&&=", "++", "--", "+=", "-=", "*=", "/=", "%=", "^=", "|=", "&=", "=",
];

/// The `[...]` subscript of an `@` target, as arithmetic for zsh: a literal
/// number, or a temporary checked at run time (`Subscript error.`).
fn subscript_index(
    em: &mut Emitter,
    raw: &str,
    pre: &mut Vec<String>,
) -> Result<String, String> {
    if raw.contains(char::is_whitespace) {
        return Err(SUBSCRIPT.into());
    }
    match classify(raw) {
        Shape::Literal(v) if v.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(v.parse::<u64>().unwrap_or(0).to_string())
        }
        Shape::Literal(_) | Shape::Glob => Err(SUBSCRIPT.into()),
        _ => {
            let t = em.temp();
            let text = em.word_text(raw);
            pre.push(format!(
                "{t}={text}; [[ ${t} == (|<->) ]] || {{ {}; }}; {t}=$(( 10#${{{t}:-0}} ))",
                em.fail(SUBSCRIPT, true)
            ));
            Ok(t)
        }
    }
}

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
    let name = &body[..name_len];
    let zname = zsh_var_name(name);
    let mut rest = &body[name_len..];
    let mut em = Emitter::new("@");
    let mut pre = Vec::new();
    let mut index = None;
    if rest.starts_with('[') {
        let close = rest.find(']').ok_or(SUBSCRIPT)?;
        index = Some(subscript_index(&mut em, &rest[1..close], &mut pre)?);
        rest = &rest[close + 1..];
    }
    let target = match &index {
        Some(i) => format!("{zname}[{i}]"),
        None => zname.to_string(),
    };
    let rest = rest.trim_start();
    let Some(op) = ASSIGN_OPS.iter().find(|o| rest.starts_with(**o)) else {
        return Err(if rest.is_empty() { NO_EXPR.into() } else { "Unknown operator.".into() });
    };
    let expr = rest[op.len()..].trim();
    let incdec = *op == "++" || *op == "--";
    if incdec && !expr.is_empty() {
        return Err(NAME_ERR.into());
    }
    if !incdec && expr.is_empty() {
        return Err(if *op == "=" { NO_EXPR } else { SYNTAX }.into());
    }

    let value = if incdec {
        let cur = current_value(&mut em, zname, &index, &mut pre);
        format!("{} {} 1", cur.operand(), &op[..1])
    } else {
        let sole = if *op == "=" { NO_EXPR } else { SYNTAX };
        let leftover = if *op == "=" || index.is_some() { NAME_ERR } else { SYNTAX };
        let a = value_of(&mut em, expr, leftover, sole, &mut pre)?;
        match op.strip_suffix('=') {
            Some("") => a.text,
            Some(bin) => {
                let cur = current_value(&mut em, zname, &index, &mut pre);
                let a = if matches!(bin, "/" | "%") { em.divisor(bin, a, &mut pre) } else { a };
                format!("{} {bin} {}", cur.operand(), a.operand())
            }
            None => unreachable!("every assignment operator ends in '='"),
        }
    };
    if let Some(i) = &index {
        let undefined = em.fail(&format!("{name}: Undefined variable."), false);
        let range = em.fail(SUBSCRIPT_RANGE, true);
        pre.push(format!(
            "(( ${{+{zname}}} )) || {{ {undefined}; }}; \
             (( {i} >= 1 && {i} <= ${{#{zname}}} )) || {{ {range}; }}"
        ));
    }
    let stmt = if is_integer(&value) {
        format!("{target}={value}")
    } else {
        format!("{target}=$(( {value} ))")
    };
    Ok(em.group(pre, stmt))
}

/// The current numeric value of the `@` target for `op=`/`++`: the first
/// word of `$name` (0 when empty or unset), or the subscripted element.
fn current_value(
    em: &mut Emitter,
    zname: &str,
    index: &Option<String>,
    pre: &mut Vec<String>,
) -> Arith {
    let t = em.temp();
    match index {
        Some(i) => {
            pre.push(format!("{t}=${{{zname}[{i}]}}; {}", em.scalar_guard(&t)));
        }
        None => {
            let a = em.array();
            let guard = em.numeric_guard(&a, None, None);
            pre.push(format!("{a}=( ${{={zname}}} ); {guard}; {t}=${{{a}[1]}}"));
        }
    }
    Arith::atom(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::Once;

    /// Scratch directory holding `emp` (empty file), `full` (non-empty),
    /// `lnk` (symlink to `full`), `dd` (directory) and the empty files
    /// `ga.txt`, `gb.txt`.
    fn fixtures() -> PathBuf {
        static INIT: Once = Once::new();
        let dir = std::env::temp_dir().join(format!("csh_expr_fixtures_{}", std::process::id()));
        INIT.call_once(|| {
            let _ = std::fs::create_dir_all(dir.join("dd"));
            let _ = std::fs::write(dir.join("emp"), "");
            let _ = std::fs::write(dir.join("full"), "hi\n");
            let _ = std::fs::write(dir.join("ga.txt"), "");
            let _ = std::fs::write(dir.join("gb.txt"), "");
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
            ("1 1", "if: Expression Syntax."),
            ("1x 1", "if: Badly formed number."),
            ("-q emp", "if: Badly formed number."),
            ("1 1x", "if: Expression Syntax."),
            ("1 == 1 == 1", "if: Expression Syntax."),
            ("1 =~ 1 =~ 1", "if: Expression Syntax."),
            ("1 && && 1", "if: Expression Syntax."),
            ("(1)(1)", "if: Expression Syntax."),
            ("1 ~ 1", "if: Expression Syntax."),
            ("}", "if: Expression Syntax."),
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
            ("n", "@: Assignment missing expression."),
            ("n =", "@: Assignment missing expression."),
            ("1x = 2", "@: Variable name must begin with a letter."),
            ("x = 1 2", "@: Variable name must begin with a letter."),
            ("x++ + 1", "@: Variable name must begin with a letter."),
            ("x-y = 2", "@: Unknown operator."),
            ("x = (1", "Too many ('s."),
            ("x = 1)", "Too many )'s."),
        ];
        for (body, want) in at {
            assert_eq!(translate_at(body).unwrap_err(), *want, "@ {body}");
        }
    }

    /// `while` reuses the condition translator: structural errors and the
    /// run-time error statements are retargeted to the `while:` prefix tcsh
    /// prints, `exit` and `@` carry their own.
    #[test]
    fn error_prefix_follows_the_command() {
        let e = translate_condition("1 1").unwrap_err();
        assert_eq!(for_command(&e, "while"), "while: Expression Syntax.");
        let e = translate_condition("(1").unwrap_err();
        assert_eq!(for_command(&e, "while"), "Too many ('s.");
        let text = translate_condition("1x").unwrap();
        assert!(text.contains("print -ru2 -- 'if: Badly formed number.'"), "{text}");
        let text = for_command(&text, "while");
        assert!(text.contains("print -ru2 -- 'while: Badly formed number.'"), "{text}");
        assert!(!text.contains("'if: "), "{text}");
        assert!(translate_exit("abc").unwrap().contains("'exit: Expression Syntax.'"));
        assert!(translate_at("n = 1x").unwrap().contains("'@: Badly formed number.'"));
        // no prefix on messages tcsh prints bare
        let text = for_command(&translate_condition("1 / 0").unwrap(), "while");
        assert!(text.contains("'Division by 0.'"), "{text}");
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

    /// `(stdout, stderr, status)` of `script` under `zsh -f` in the fixture
    /// directory; `None` when zsh is not installed.
    fn zsh_run(script: &str) -> Option<(String, String, i32)> {
        let out = Command::new("zsh")
            .args(["-f", "-c", script])
            .current_dir(fixtures())
            .output()
            .ok()?;
        Some((
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
            out.status.code().unwrap_or(-1),
        ))
    }

    /// Run `if (expr)` after `setup` (zsh syntax) with `pre`/`post` echoed
    /// around it.
    fn run_if(setup: &str, expr: &str) -> Option<(String, String, i32)> {
        let cond = translate_condition(expr).unwrap_or_else(|e| panic!("[{expr}] {e}"));
        zsh_run(&format!("{setup}; echo pre; if {cond}; then echo T; else echo F; fi; echo post"))
    }

    /// Run `@ stmt` after `setup`; prints `n|l|s` afterwards.
    fn run_at(setup: &str, stmt: &str) -> Option<(String, String, i32)> {
        let code = translate_at(stmt).unwrap_or_else(|e| panic!("@ {stmt}: {e}"));
        zsh_run(&format!("n=U; l=(1 2 3); s=S; {setup}; echo pre; {code}; echo \"$n|$l|$s\""))
    }

    /// Every row ran under `/bin/tcsh -f` (`echo pre; if (EXPR) ...; echo
    /// post`) and gave this stdout and stderr; a non-empty stderr is exit
    /// status 1. tcsh evaluates operands of `&& || | ^ & << >> ! ~` even in
    /// the skipped half; `+ - * / % < > <= >=` are not converted there.
    #[test]
    fn runtime_errors_match_tcsh() {
        const OK_T: &str = "pre\nT\npost";
        const OK_F: &str = "pre\nF\npost";
        const SYN: &str = "if: Expression Syntax.";
        const BAD: &str = "if: Badly formed number.";
        let rows: &[(&str, &str, &str, &str)] = &[
            // literals in numeric position, converted only when evaluated
            ("", "1x", "pre", BAD),
            ("", "1.5 > 1", "pre", BAD),
            ("", "0x10 + 1", "pre", BAD),
            ("", "--1", "pre", BAD),
            ("", "abc", "pre", SYN),
            ("", "a < b", "pre", SYN),
            ("", "+1", "pre", SYN),
            ("", "{true}", "pre", SYN),
            ("", "1==1", "pre", BAD),
            ("", "2 + 1x", "pre", BAD),
            ("", "1 || abc", "pre", SYN),
            ("", "0 && 1x", "pre", BAD),
            ("", "0 && (1x)", "pre", BAD),
            ("", "0 && ! 1x", "pre", BAD),
            ("", "0 && ~ 1x", "pre", BAD),
            ("", "0 && 1x << 2", "pre", BAD),
            ("", "0 && 1x | 1", "pre", BAD),
            ("", "0 && 1 && 1x", "pre", BAD),
            ("", "0 && 1x + 1", OK_F, ""),
            ("", "0 && 1 + 1x", OK_F, ""),
            ("", "0 && 1x * 2", OK_F, ""),
            ("", "0 && 1x < 2", OK_F, ""),
            ("", "0 && 1x == 1", OK_F, ""),
            ("", "0 && (1 + 1x) && 1", OK_F, ""),
            ("", "0 && 1x + 1 << 2", OK_F, ""),
            ("", "0 && !(1x < 2)", OK_F, ""),
            ("", "1 == 1 || 1x + 1", OK_T, ""),
            // variable values are converted the same way
            ("s=abc", "$s", "pre", SYN),
            ("s=1x", "$s", "pre", BAD),
            ("s=abc", "$s + 1", "pre", SYN),
            ("s=abc", "1 || $s", "pre", SYN),
            ("s=abc", "$s == abc", OK_T, ""),
            ("s=abc", "$s =~ a*", OK_T, ""),
            ("a=5; b=abc", "$a > 3 || $b > 3", OK_T, ""),
            ("a=5; b=abc", "$a > 3 || $b", "pre", SYN),
            ("a=5; b=1x", "$a < 3 && ($b + 1)", OK_F, ""),
            ("a=5; b=1x", "$a < 3 && ($b << 1)", "pre", BAD),
            ("a=5; b=1x", "$a < 3 && ! $b", "pre", BAD),
            ("a=5; b=1x", "{ echo ran } && $b", "pre\nran", BAD),
            ("a=5; b=1x", "{ false } && $b + 1", OK_F, ""),
            ("s='1 2'", "\"$s\"", "pre", BAD),
            ("s='1 2'", "\"$s\" == \"1 2\"", OK_T, ""),
            // two or more words from an unquoted expansion
            ("l=(1x 2)", "$l", "pre", BAD),
            ("l=(2 1x)", "$l", "pre", SYN),
            ("l=(1 2)", "$l == 1", "pre", SYN),
            ("l=(1 2)", "$l > 0", "pre", SYN),
            ("l=(1 2)", "1 || $l", "pre", SYN),
            ("f='a b'", "-e $f", "pre", SYN),
            ("f='a b'", "-e \"$f\"", OK_F, ""),
            // an expansion that yields no word vanishes from the expression
            ("z=()", "$z && 1", "pre", SYN),
            ("z=()", "1 == $z || 1", "pre", SYN),
            ("z=()", "! $z && 1", "pre", SYN),
            ("e=''", "$e || 1", "pre", SYN),
            ("e=''", "0 || $e", OK_F, ""),
            ("z=()", "1 && $z", OK_F, ""),
            ("z=()", "($z) && 1", OK_F, ""),
            ("z=()", "(1 + $z) && 1", OK_T, ""),
            ("z=()", "$z", OK_F, ""),
            ("z=()", "$z == \"\"", OK_T, ""),
            ("z=()", "$z != x", OK_T, ""),
            ("z=()", "$z =~ *", OK_T, ""),
            ("z=()", "$z + 1 == 1", OK_T, ""),
            ("z=()", "2 * $z", OK_F, ""),
            ("z=()", "! $z", OK_T, ""),
            ("z=()", "-e $z", "pre", "if: Missing file name."),
            ("e=''", "-e $e", "pre", "if: Missing file name."),
            ("z=()", "-e $z == 1", "pre", "if: Missing file name."),
            ("z=()", "-e $z && 1", "pre", SYN),
            ("z=()", "! -e $z", "pre", "if: Missing file name."),
            // division
            ("", "5 / 0", "pre", "Division by 0."),
            ("", "5 % 0 || 1", "pre", "Mod by 0."),
            ("", "0 / 0", "pre", "Division by 0."),
            ("z=0", "5 / $z", "pre", "Division by 0."),
            ("z=0", "5 % ($z + 0)", "pre", "Mod by 0."),
            ("", "/ == /", "pre", "Division by 0."),
            ("", "0 && 5 / 0", OK_F, ""),
            ("", "1 || 5 % 0", OK_T, ""),
            ("z=0", "$z == 0 || 5 / $z", OK_T, ""),
            ("z=0", "1 / ($z + 1) == 1", OK_T, ""),
            // a glob that matches nothing
            ("", "*.zzz == x", "pre", "*.zzz: No match."),
            ("", "-e *.zzz", "pre", "*.zzz: No match."),
            ("", "*.zzz =~ x", "pre", "*.zzz: No match."),
            ("", "x =~ *.zzz", OK_F, ""),
        ];
        if zsh("true").is_none() {
            return;
        }
        for (setup, expr, out, err) in rows {
            let (o, e, rc) = run_if(setup, expr).unwrap();
            assert_eq!((o.as_str(), e.as_str()), (*out, *err), "{setup}; if ({expr})");
            assert_eq!(rc, i32::from(!err.is_empty()), "status of {setup}; if ({expr})");
        }
    }

    /// Words with an unquoted glob character are expanded and the matches
    /// are joined with one space; `* == *` is the operator `*` between
    /// empty operands (`0 == 0`).
    #[test]
    fn glob_operands_are_expanded_and_joined() {
        const OK_T: &str = "pre\nT\npost";
        const OK_F: &str = "pre\nF\npost";
        let rows: &[(&str, &str)] = &[
            ("ga.txt == ga.*", OK_T),
            ("ga.* == ga.txt", OK_T),
            ("\"ga.*\" == ga.txt", OK_F),
            ("*.txt == \"ga.txt gb.txt\"", OK_T),
            ("*.txt =~ \"ga.txt gb.txt\"", OK_T),
            ("ga.txt =~ *.txt", OK_T),
            ("-e *.txt", OK_F),
            ("-e ga.*", OK_T),
            ("-f ga.txt && -e gb.*", OK_T),
            ("* == *", OK_T),
            ("* != x", OK_T),
            ("a == *", OK_F),
            ("abc =~ *", OK_T),
            ("2 * 3 == 6", OK_T),
            ("ga.[a-z]xt == ga.txt", OK_T),
            ("{a,b} == a", OK_F),
            ("x{y,z} == \"xy xz\"", OK_T),
            ("~/ =~ /*", OK_T),
        ];
        if zsh("true").is_none() {
            return;
        }
        for (expr, want) in rows {
            let (o, e, _) = run_if("", expr).unwrap();
            assert_eq!((o.as_str(), e.as_str()), (*want, ""), "if ({expr})");
        }
    }

    /// `1 << 2 << 3` is `(1 << 2) < 3`: after one shift, a second `<<`/`>>`
    /// is the relational `<`/`>` (values from `/bin/tcsh -f`).
    #[test]
    fn a_second_shift_is_a_comparison() {
        let rows: &[(&str, &str)] = &[
            ("1 << 2 << 3", "0"),
            ("256 >> 2 >> 1", "1"),
            ("1 << 3 >> 1", "1"),
            ("64 >> 1 << 2", "0"),
            ("1 << 2 << 3 << 1", "1"),
            ("3 << 1 << 1 << 1", "0"),
            ("1 << 1 << 1 << 1 << 1", "1"),
            ("8 >> 1 << 1 >> 1", "0"),
            ("1 << 2 << 3 + 0", "0"),
            ("2 << 1 << 1 == 8", "0"),
            ("(1 << 2) << 3", "32"),
            ("1 << (2 << 3)", "65536"),
            ("1 << 2 + 1", "8"),
            ("1 + 1 << 2", "8"),
            ("1 << 2 < 5", "1"),
        ];
        if zsh("true").is_none() {
            return;
        }
        for (expr, want) in rows {
            let (o, e, _) = run_at("", &format!("n = ({expr})")).unwrap();
            assert_eq!((o.as_str(), e.as_str()), (&*format!("pre\n{want}|1 2 3|S"), ""), "@ n = ({expr})");
        }
    }

    /// `@` with a subscript and with `op=`/`++` (all rows from tcsh):
    /// a non-digit index is `Subscript error.` before the value is
    /// evaluated, the range check follows the value, `op=` reads the first
    /// word of the variable.
    #[test]
    fn at_subscripts_and_wordlists_match_tcsh() {
        const NAME: &str = "@: Variable name must begin with a letter.";
        const SYN: &str = "@: Expression Syntax.";
        const RANGE: &str = "@: Subscript out of range.";
        let rows: &[(&str, &str, &str, &str)] = &[
            // setup, statement, "n|l|s" afterwards, stderr
            ("", "l[2] = 7", "U|1 7 3|S", ""),
            ("", "l[3] = 7", "U|1 2 7|S", ""),
            ("", "l[01] = 8", "U|8 2 3|S", ""),
            ("", "l[2]++", "U|1 3 3|S", ""),
            ("", "l[2]--", "U|1 1 3|S", ""),
            ("", "l[3] += 5", "U|1 2 8|S", ""),
            ("", "l[2] = (3 + 4) * 2", "U|1 14 3|S", ""),
            ("", "l[$#l] += 2", "U|1 2 5|S", ""),
            ("", "l[2] = $l[3] + 1", "U|1 4 3|S", ""),
            ("i=2", "l[$i] = 9", "U|1 9 3|S", ""),
            ("", "l[4] = 7", "", RANGE),
            ("", "l[0] = 7", "", RANGE),
            ("", "l[] = 1", "", RANGE),
            ("", "l[4]++", "", RANGE),
            ("", "l[4] += 1", "", RANGE),
            ("l=()", "l[1]++", "", RANGE),
            ("i=5", "l[$i] = 9", "", RANGE),
            ("i=0", "l[$i] = 9", "", RANGE),
            ("i=''", "l[$i] = 9", "", RANGE),
            ("i=()", "l[$i] = 9", "", RANGE),
            ("i=x", "l[$i] = 9", "", "@: Subscript error."),
            ("i=(1 2)", "l[$i] = 9", "", "@: Subscript error."),
            ("", "l[4] = 1x", "", "@: Badly formed number."),
            ("", "l[2] = 1x", "", "@: Badly formed number."),
            ("", "u[9] = 1", "", "u: Undefined variable."),
            ("", "u[1] += 1", "", "u: Undefined variable."),
            ("l=(1 2); y=7", "y[2] = 8", "", RANGE),
            ("y=7", "y[1] = 8", "U|1 2 3|S", ""),
            // a variable on the right
            ("y=(4 5)", "n = $y", "", NAME),
            ("y=(4 5)", "n = $y + 1", "", NAME),
            ("y=(4 5)", "n = 1 + $y", "", NAME),
            ("y=(4 5)", "n = ($y)", "", SYN),
            ("y=(4 5)", "n += $y", "", SYN),
            ("y=(4 5)", "l[1] += $y", "", NAME),
            ("y=(4 5)", "n = \"$y\"", "", "@: Badly formed number."),
            ("y=()", "n = $y", "", "@: Assignment missing expression."),
            ("y=()", "n = 1 + $y", "", SYN),
            ("y=()", "n += $y", "", SYN),
            ("y=()", "n = ($y)", "0|1 2 3|S", ""),
            ("y=()", "n = $y + 1", "1|1 2 3|S", ""),
            ("y=(4)", "n = $y * 2", "8|1 2 3|S", ""),
            // `op=` reads the first word, empty and unset are 0
            ("s=(5 6)", "s++", "U|1 2 3|6", ""),
            ("s=(5 6)", "s += 1", "U|1 2 3|6", ""),
            ("s=(5 6)", "s[2]++", "U|1 2 3|5 7", ""),
            ("s=(5 6)", "s = 1", "U|1 2 3|1", ""),
            ("s=''", "s++", "U|1 2 3|1", ""),
            ("s=()", "s += 1", "U|1 2 3|1", ""),
            ("unset s", "s += 2", "U|1 2 3|2", ""),
            ("s=-5", "s++", "U|1 2 3|-4", ""),
            ("s=007", "s++", "U|1 2 3|8", ""),
            ("s=5", "s -= 7", "U|1 2 3|-2", ""),
            ("s=5", "s *= 3 + 1", "U|1 2 3|20", ""),
            ("s=abc", "s++", "", SYN),
            ("s=1x", "s++", "", "@: Badly formed number."),
            ("s=(5 x)", "s[2]++", "", SYN),
            ("s=5", "s /= 0", "", "Division by 0."),
            ("s=5", "s %= 0", "", "Mod by 0."),
            // literals
            ("", "n = 1x", "", "@: Badly formed number."),
            ("", "n = abc", "", SYN),
            ("", "n = 0x10", "", "@: Badly formed number."),
            ("", "n = 5 / 0", "", "Division by 0."),
            ("", "n = (0 && 5 / 0)", "0|1 2 3|S", ""),
            ("", "n = (1 || 1x)", "", "@: Badly formed number."),
        ];
        if zsh("true").is_none() {
            return;
        }
        for (setup, stmt, shown, err) in rows {
            let (o, e, rc) = run_at(setup, stmt).unwrap();
            let want = if err.is_empty() { format!("pre\n{shown}") } else { "pre".to_string() };
            assert_eq!((o.as_str(), e.as_str()), (want.as_str(), *err), "{setup}; @ {stmt}");
            assert_eq!(rc, i32::from(!err.is_empty()), "status of {setup}; @ {stmt}");
        }
        // a subscript that is not a number is known before anything runs
        assert_eq!(translate_at("l[x] = 5").unwrap_err(), "@: Subscript error.");
        assert_eq!(translate_at("l[1-2] = 5").unwrap_err(), "@: Subscript error.");
        assert_eq!(translate_at("l[ 2 ] = 5").unwrap_err(), "@: Subscript error.");
    }

    /// File inquiry letters beyond `-e` (rows from `/bin/tcsh -f`, against
    /// `emp` (empty), `full` (3 bytes, mode 644), `lnk` -> `full`, `dd`
    /// (mode 755)): value operators, the stacked forms, `X`, `m`, `K`, `t`.
    #[test]
    fn file_inquiry_values_match_tcsh() {
        let rows: &[(&str, &str)] = &[
            ("-Z full == 3", "T"),
            ("-Z emp == 0", "T"),
            ("-Z nonex == -1", "T"),
            ("-Z full > 2", "T"),
            ("-Z emp", "F"),
            ("-Z nonex", "T"),
            ("-N full == 1", "T"),
            ("-N dd >= 2", "T"),
            ("-N nonex == -1", "T"),
            ("-P full == 644", "T"),
            ("-P: full == 0644", "T"),
            ("-P44 full == 44", "T"),
            ("-P22 full == 0", "T"),
            ("-P755 dd == 755", "T"),
            ("-P nonex == -1", "T"),
            ("-L lnk == full", "T"),
            ("-L full == -1", "T"),
            ("-L nonex == -1", "T"),
            ("-fL lnk == full", "T"),
            ("-dL lnk == 0", "T"),
            ("-F nonex == :", "T"),
            ("-F full =~ *:*", "T"),
            ("-D full > 0", "T"),
            ("-I full > 0", "T"),
            ("-A full > 1000000", "T"),
            ("-M full > 1000000", "T"),
            ("-C full > 1000000", "T"),
            ("-M nonex == -1", "T"),
            ("-M: full =~ *:*:*", "T"),
            ("-U full == 0", "F"),
            ("-U: full == -1", "F"),
            ("-fZ full == 3", "T"),
            ("-dZ full == 0", "T"),
            ("-ez emp", "T"),
            ("-ez full", "F"),
            ("-Z full + 1 == 4", "T"),
            ("-lLo lnk", "T"),
            ("-LZ lnk == 4", "T"),
            ("-Lf lnk", "F"),
            ("-Ld dd", "T"),
            ("-X cd", "T"),
            ("-X ls", "T"),
            ("-X /bin/ls", "F"),
            ("-X nonexcmd", "F"),
            ("-m full", "F"),
            ("-K full", "F"),
            ("-t x", "F"),
            ("-S full", "F"),
        ];
        if zsh("true").is_none() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = fixtures();
            let _ = std::fs::set_permissions(dir.join("full"), std::fs::Permissions::from_mode(0o644));
            let _ = std::fs::set_permissions(dir.join("dd"), std::fs::Permissions::from_mode(0o755));
        }
        for (expr, want) in rows {
            let (o, e, _) = run_if("", expr).unwrap();
            assert_eq!((o.as_str(), e.as_str()), (&*format!("pre\n{want}\npost"), ""), "if ({expr})");
        }
        // a value operator must be last, `-e/tmp` is not a letter
        for bad in ["-ZZ full", "-Zf full", "-e/tmp full"] {
            assert_eq!(translate_condition(bad).unwrap_err(), "if: Malformed file inquiry.", "{bad}");
        }
    }

    /// `$status` is a variable in tcsh: guards and `{ cmd }` that run before
    /// it must not disturb the value.
    #[test]
    fn status_is_read_before_guards_run() {
        if zsh("true").is_none() {
            return;
        }
        for (expr, want) in [
            ("$status == 1 && 5 > 3", "T"),
            ("5 > 3 && $status == 1", "T"),
            ("{ true } && $status == 1", "T"),
            ("$x + 1 > 0 && $status == 1", "T"),
            ("$status == 0", "F"),
        ] {
            let cond = translate_condition(expr).unwrap();
            let out = zsh(&format!("x=2; false; if {cond}; then echo T; else echo F; fi")).unwrap();
            assert_eq!(out, want, "false; if ({expr})");
        }
    }

    /// `exit (expr)` reports run-time conversion errors like `@`.
    #[test]
    fn exit_value_errors_are_runtime() {
        if zsh("true").is_none() {
            return;
        }
        let (o, e, rc) = zsh_run(&format!("echo pre; {}; echo post", translate_exit("abc").unwrap())).unwrap();
        assert_eq!((o.as_str(), e.as_str(), rc), ("pre", "exit: Expression Syntax.", 1));
        let (o, e, rc) = zsh_run(&format!("s=1x; echo pre; {}; echo post", translate_exit("$s + 1").unwrap())).unwrap();
        assert_eq!((o.as_str(), e.as_str(), rc), ("pre", "exit: Badly formed number.", 1));
    }
}
