//! Zsh parser - Direct port from zsh/Src/parse.c
//!
//! This parser takes tokens from the ZshLexer and builds an AST.
//! It follows the zsh grammar closely, producing structures that
//! can be executed by the shell executor.

use crate::lexer::ZshLexer;
use crate::tokens::LexTok;
use serde::{Serialize, Deserialize};
use std::iter::Peekable;
use std::str::Chars;

/// AST node for a complete program (list of commands)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshProgram {
    pub lists: Vec<ZshList>,
}

/// A list is a sequence of sublists separated by ; or & or newline
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshList {
    pub sublist: ZshSublist,
    pub flags: ListFlags,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct ListFlags {
    /// Run asynchronously (&)
    pub async_: bool,
    /// Disown after running (&| or &!)
    pub disown: bool,
}

/// A sublist is pipelines connected by && or ||
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshSublist {
    pub pipe: ZshPipe,
    pub next: Option<(SublistOp, Box<ZshSublist>)>,
    pub flags: SublistFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SublistOp {
    And, // &&
    Or,  // ||
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SublistFlags {
    /// Coproc
    pub coproc: bool,
    /// Negated with !
    pub not: bool,
}

/// A pipeline is commands connected by |
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshPipe {
    pub cmd: ZshCommand,
    pub next: Option<Box<ZshPipe>>,
    pub lineno: u64,
}

/// A command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ZshCommand {
    Simple(ZshSimple),
    Subsh(Box<ZshProgram>), // (list)
    Cursh(Box<ZshProgram>), // {list}
    For(ZshFor),
    Case(ZshCase),
    If(ZshIf),
    While(ZshWhile),
    Until(ZshWhile),
    Repeat(ZshRepeat),
    FuncDef(ZshFuncDef),
    Time(Option<Box<ZshSublist>>),
    Cond(ZshCond), // [[ ... ]]
    Arith(String), // (( ... ))
    Try(ZshTry),   // { ... } always { ... }
}

/// A simple command (assignments, words, redirections)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshSimple {
    pub assigns: Vec<ZshAssign>,
    pub words: Vec<String>,
    pub redirs: Vec<ZshRedir>,
}

/// An assignment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshAssign {
    pub name: String,
    pub value: ZshAssignValue,
    pub append: bool, // +=
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ZshAssignValue {
    Scalar(String),
    Array(Vec<String>),
}

/// A redirection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshRedir {
    pub rtype: RedirType,
    pub fd: i32,
    pub name: String,
    pub heredoc: Option<HereDocInfo>,
    pub varid: Option<String>, // {var}>file
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HereDocInfo {
    pub content: String,
    pub terminator: String,
}

/// Redirection type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedirType {
    Write,        // >
    Writenow,     // >|
    Append,       // >>
    Appendnow,    // >>|
    Read,         // <
    ReadWrite,    // <>
    Heredoc,      // <<
    HeredocDash,  // <<-
    Herestr,      // <<<
    MergeIn,      // <&
    MergeOut,     // >&
    ErrWrite,     // &>
    ErrWritenow,  // &>|
    ErrAppend,    // >>&
    ErrAppendnow, // >>&|
    InPipe,       // < <(...)
    OutPipe,      // > >(...)
}

/// For loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshFor {
    pub var: String,
    pub list: ForList,
    pub body: Box<ZshProgram>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ForList {
    Words(Vec<String>),
    CStyle {
        init: String,
        cond: String,
        step: String,
    },
    Positional,
}

/// Case statement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshCase {
    pub word: String,
    pub arms: Vec<CaseArm>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaseArm {
    pub patterns: Vec<String>,
    pub body: ZshProgram,
    pub terminator: CaseTerm,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaseTerm {
    Break,    // ;;
    Continue, // ;&
    TestNext, // ;|
}

/// If statement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshIf {
    pub cond: Box<ZshProgram>,
    pub then: Box<ZshProgram>,
    pub elif: Vec<(ZshProgram, ZshProgram)>,
    pub else_: Option<Box<ZshProgram>>,
}

/// While/Until loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshWhile {
    pub cond: Box<ZshProgram>,
    pub body: Box<ZshProgram>,
    pub until: bool,
}

/// Repeat loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshRepeat {
    pub count: String,
    pub body: Box<ZshProgram>,
}

/// Function definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshFuncDef {
    pub names: Vec<String>,
    pub body: Box<ZshProgram>,
    pub tracing: bool,
}

/// Conditional expression [[ ... ]]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ZshCond {
    Not(Box<ZshCond>),
    And(Box<ZshCond>, Box<ZshCond>),
    Or(Box<ZshCond>, Box<ZshCond>),
    Unary(String, String),          // -f file, -n str, etc.
    Binary(String, String, String), // str = pat, a -eq b, etc.
    Regex(String, String),          // str =~ regex
}

/// Try/always block
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZshTry {
    pub try_block: Box<ZshProgram>,
    pub always: Box<ZshProgram>,
}

/// Zsh parameter expansion flags
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ZshParamFlag {
    Lower,                 // L - lowercase
    Upper,                 // U - uppercase
    Capitalize,            // C - capitalize words
    Join(String),          // j:sep: - join array with separator
    JoinNewline,           // F - join with newlines
    Split(String),         // s:sep: - split string into array
    SplitLines,            // f - split on newlines
    SplitWords,            // z - split into words (shell parsing)
    Type,                  // t - type of variable
    Words,                 // w - word splitting
    Quote,                 // q - quote result
    DoubleQuote,           // qq - double quote
    QuoteBackslash,        // b - quote with backslashes for patterns
    Unique,                // u - unique elements only
    Reverse,               // O - reverse sort
    Sort,                  // o - sort
    NumericSort,           // n - numeric sort
    IndexSort,             // a - sort in array index order
    Keys,                  // k - associative array keys
    Values,                // v - associative array values
    Length,                // # - length (character codes)
    CountChars,            // c - count total characters
    Expand,                // e - perform shell expansions
    PromptExpand,          // % - expand prompt escapes
    PromptExpandFull,      // %% - full prompt expansion
    Visible,               // V - make non-printable chars visible
    Directory,             // D - substitute directory names
    Head(usize),           // [1,n] - first n elements
    Tail(usize),           // [-n,-1] - last n elements
    PadLeft(usize, char),  // l:len:fill: - pad left
    PadRight(usize, char), // r:len:fill: - pad right
    Width(usize),          // m - use width for padding
    Match,                 // M - include matched portion
    Remove,                // R - include non-matched portion (complement of M)
    Subscript,             // S - subscript scanning
    Parameter,             // P - use value as parameter name (indirection)
    Glob,                  // ~ - glob patterns in pattern
}

/// List operator (for shell command lists)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListOp {
    And,     // &&
    Or,      // ||
    Semi,    // ;
    Amp,     // &
    Newline, // \n
}

/// Shell word - can be simple literal or complex expansion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShellWord {
    Literal(String),
    SingleQuoted(String),
    DoubleQuoted(Vec<ShellWord>),
    Variable(String),
    VariableBraced(String, Option<Box<VarModifier>>),
    ArrayVar(String, Box<ShellWord>),
    CommandSub(Box<ShellCommand>),
    ProcessSubIn(Box<ShellCommand>),
    ProcessSubOut(Box<ShellCommand>),
    ArithSub(String),
    ArrayLiteral(Vec<ShellWord>),
    Glob(String),
    Tilde(Option<String>),
    Concat(Vec<ShellWord>),
}

/// Variable modifier for parameter expansion
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VarModifier {
    Default(ShellWord),
    DefaultAssign(ShellWord),
    Error(ShellWord),
    Alternate(ShellWord),
    Length,
    ArrayLength,
    ArrayIndex(String),
    ArrayAll,
    Substring(i64, Option<i64>),
    RemovePrefix(ShellWord),
    RemovePrefixLong(ShellWord),
    RemoveSuffix(ShellWord),
    RemoveSuffixLong(ShellWord),
    Replace(ShellWord, ShellWord),
    ReplaceAll(ShellWord, ShellWord),
    Upper,
    Lower,
    ZshFlags(Vec<ZshParamFlag>),
}

/// Shell command - the old shell_ast compatible type
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ShellCommand {
    Simple(SimpleCommand),
    Pipeline(Vec<ShellCommand>, bool),
    List(Vec<(ShellCommand, ListOp)>),
    Compound(CompoundCommand),
    FunctionDef(String, Box<ShellCommand>),
}

/// Simple command with assignments, words, and redirects
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleCommand {
    pub assignments: Vec<(String, ShellWord, bool)>,
    pub words: Vec<ShellWord>,
    pub redirects: Vec<Redirect>,
}

/// Redirect
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redirect {
    pub fd: Option<i32>,
    pub op: RedirectOp,
    pub target: ShellWord,
    pub heredoc_content: Option<String>,
    pub fd_var: Option<String>,
}

/// Redirect operator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RedirectOp {
    Write,
    Append,
    Read,
    ReadWrite,
    Clobber,
    DupRead,
    DupWrite,
    HereDoc,
    HereString,
    WriteBoth,
    AppendBoth,
}

/// Compound command
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CompoundCommand {
    BraceGroup(Vec<ShellCommand>),
    Subshell(Vec<ShellCommand>),
    If {
        conditions: Vec<(Vec<ShellCommand>, Vec<ShellCommand>)>,
        else_part: Option<Vec<ShellCommand>>,
    },
    For {
        var: String,
        words: Option<Vec<ShellWord>>,
        body: Vec<ShellCommand>,
    },
    ForArith {
        init: String,
        cond: String,
        step: String,
        body: Vec<ShellCommand>,
    },
    While {
        condition: Vec<ShellCommand>,
        body: Vec<ShellCommand>,
    },
    Until {
        condition: Vec<ShellCommand>,
        body: Vec<ShellCommand>,
    },
    Case {
        word: ShellWord,
        cases: Vec<(Vec<ShellWord>, Vec<ShellCommand>, CaseTerminator)>,
    },
    Select {
        var: String,
        words: Option<Vec<ShellWord>>,
        body: Vec<ShellCommand>,
    },
    Coproc {
        name: Option<String>,
        body: Box<ShellCommand>,
    },
    /// repeat N do ... done
    Repeat {
        count: String,
        body: Vec<ShellCommand>,
    },
    /// { try-block } always { always-block }
    Try {
        try_body: Vec<ShellCommand>,
        always_body: Vec<ShellCommand>,
    },
    Cond(CondExpr),
    Arith(String),
    WithRedirects(Box<ShellCommand>, Vec<Redirect>),
}

/// Case terminator
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum CaseTerminator {
    Break,
    Fallthrough,
    Continue,
}

/// Conditional expression for [[ ]]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CondExpr {
    FileExists(ShellWord),
    FileRegular(ShellWord),
    FileDirectory(ShellWord),
    FileSymlink(ShellWord),
    FileReadable(ShellWord),
    FileWritable(ShellWord),
    FileExecutable(ShellWord),
    FileNonEmpty(ShellWord),
    StringEmpty(ShellWord),
    StringNonEmpty(ShellWord),
    StringEqual(ShellWord, ShellWord),
    StringNotEqual(ShellWord, ShellWord),
    StringMatch(ShellWord, ShellWord),
    StringLess(ShellWord, ShellWord),
    StringGreater(ShellWord, ShellWord),
    NumEqual(ShellWord, ShellWord),
    NumNotEqual(ShellWord, ShellWord),
    NumLess(ShellWord, ShellWord),
    NumLessEqual(ShellWord, ShellWord),
    NumGreater(ShellWord, ShellWord),
    NumGreaterEqual(ShellWord, ShellWord),
    Not(Box<CondExpr>),
    And(Box<CondExpr>, Box<CondExpr>),
    Or(Box<CondExpr>, Box<CondExpr>),
}

/// Parse errors
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseError {
    pub message: String,
    pub line: u64,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "parse error at line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

// ============================================================================
// ShellToken, ShellLexer, and ShellParser - compatibility layer for exec.rs
// ============================================================================

#[derive(Debug, Clone, PartialEq)]
pub enum ShellToken {
    Word(String),
    SingleQuotedWord(String),
    DoubleQuotedWord(String),
    Number(i64),
    Semi,
    Newline,
    Amp,
    AmpAmp,
    Pipe,
    PipePipe,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    DoubleLBracket,
    DoubleRBracket,
    Less,
    Greater,
    GreaterGreater,
    LessGreater,
    GreaterAmp,
    LessAmp,
    GreaterPipe,
    LessLess,
    LessLessLess,
    HereDoc(String, String),
    AmpGreater,
    AmpGreaterGreater,
    DoubleLParen,
    DoubleRParen,
    If,
    Then,
    Else,
    Elif,
    Fi,
    Case,
    Esac,
    For,
    While,
    Until,
    Do,
    Done,
    In,
    Function,
    Select,
    Time,
    Coproc,
    Typeset(String),
    Repeat,
    Always,
    Bang,
    DoubleSemi,
    SemiAmp,
    SemiSemiAmp,
    Eof,
}

pub struct ShellLexer<'a> {
    input: Peekable<Chars<'a>>,
    line: usize,
    col: usize,
    at_line_start: bool,
}

impl<'a> ShellLexer<'a> {
    pub fn new(input: &'a str) -> Self {
        Self {
            input: input.chars().peekable(),
            line: 1,
            col: 1,
            at_line_start: true,
        }
    }

    fn peek(&mut self) -> Option<char> {
        self.input.peek().copied()
    }

    fn next_char(&mut self) -> Option<char> {
        let c = self.input.next();
        if let Some(ch) = c {
            if ch == '\n' {
                self.line += 1;
                self.col = 1;
                self.at_line_start = true;
            } else {
                self.col += 1;
                self.at_line_start = false;
            }
        }
        c
    }

    fn skip_whitespace(&mut self) -> bool {
        let mut had_whitespace = false;
        while let Some(c) = self.peek() {
            if c == ' ' || c == '\t' {
                self.next_char();
                had_whitespace = true;
            } else if c == '\\' {
                self.next_char();
                if self.peek() == Some('\n') {
                    self.next_char();
                }
                had_whitespace = true;
            } else {
                break;
            }
        }
        had_whitespace
    }

    fn skip_comment(&mut self, after_whitespace: bool) {
        if after_whitespace && self.peek() == Some('#') {
            while let Some(c) = self.peek() {
                if c == '\n' {
                    break;
                }
                self.next_char();
            }
        }
    }

    pub fn next_token(&mut self) -> ShellToken {
        let was_at_line_start = self.at_line_start;
        let had_whitespace = self.skip_whitespace();
        self.skip_comment(had_whitespace || was_at_line_start);

        let c = match self.peek() {
            Some(c) => c,
            None => return ShellToken::Eof,
        };

        if c == '\n' {
            self.next_char();
            self.at_line_start = true;
            return ShellToken::Newline;
        }

        self.at_line_start = false;

        if c == ';' {
            self.next_char();
            if self.peek() == Some(';') {
                self.next_char();
                if self.peek() == Some('&') {
                    self.next_char();
                    return ShellToken::SemiSemiAmp;
                }
                return ShellToken::DoubleSemi;
            }
            if self.peek() == Some('&') {
                self.next_char();
                return ShellToken::SemiAmp;
            }
            return ShellToken::Semi;
        }

        if c == '&' {
            self.next_char();
            match self.peek() {
                Some('&') => {
                    self.next_char();
                    return ShellToken::AmpAmp;
                }
                Some('>') => {
                    self.next_char();
                    if self.peek() == Some('>') {
                        self.next_char();
                        return ShellToken::AmpGreaterGreater;
                    }
                    return ShellToken::AmpGreater;
                }
                _ => return ShellToken::Amp,
            }
        }

        if c == '|' {
            self.next_char();
            if self.peek() == Some('|') {
                self.next_char();
                return ShellToken::PipePipe;
            }
            return ShellToken::Pipe;
        }

        if c == '<' {
            self.next_char();
            match self.peek() {
                Some('(') => {
                    self.next_char();
                    let cmd = self.read_process_sub();
                    return ShellToken::Word(format!("<({}", cmd));
                }
                Some('<') => {
                    self.next_char();
                    if self.peek() == Some('<') {
                        self.next_char();
                        return ShellToken::LessLessLess;
                    }
                    return self.read_heredoc();
                }
                Some('>') => {
                    self.next_char();
                    return ShellToken::LessGreater;
                }
                Some('&') => {
                    self.next_char();
                    return ShellToken::LessAmp;
                }
                _ => return ShellToken::Less,
            }
        }

        if c == '>' {
            self.next_char();
            match self.peek() {
                Some('(') => {
                    self.next_char();
                    let cmd = self.read_process_sub();
                    return ShellToken::Word(format!(">({}", cmd));
                }
                Some('>') => {
                    self.next_char();
                    return ShellToken::GreaterGreater;
                }
                Some('&') => {
                    self.next_char();
                    return ShellToken::GreaterAmp;
                }
                Some('|') => {
                    self.next_char();
                    return ShellToken::GreaterPipe;
                }
                _ => return ShellToken::Greater,
            }
        }

        if c == '(' {
            self.next_char();
            if self.peek() == Some('(') {
                self.next_char();
                return ShellToken::DoubleLParen;
            }
            return ShellToken::LParen;
        }

        if c == ')' {
            self.next_char();
            if self.peek() == Some(')') {
                self.next_char();
                return ShellToken::DoubleRParen;
            }
            return ShellToken::RParen;
        }

        if c == '[' {
            self.next_char();
            if self.peek() == Some('[') {
                self.next_char();
                return ShellToken::DoubleLBracket;
            }
            if let Some(next_ch) = self.peek() {
                if !next_ch.is_whitespace() && next_ch != ']' {
                    let mut pattern = String::from("[");
                    while let Some(ch) = self.peek() {
                        pattern.push(self.next_char().unwrap());
                        if ch == ']' {
                            while let Some(c2) = self.peek() {
                                if c2.is_whitespace()
                                    || c2 == ';'
                                    || c2 == '&'
                                    || c2 == '|'
                                    || c2 == '<'
                                    || c2 == '>'
                                    || c2 == ')'
                                    || c2 == '\n'
                                {
                                    break;
                                }
                                pattern.push(self.next_char().unwrap());
                            }
                            return ShellToken::Word(pattern);
                        }
                        if ch.is_whitespace() {
                            return ShellToken::Word(pattern);
                        }
                    }
                    return ShellToken::Word(pattern);
                }
            }
            return ShellToken::LBracket;
        }

        if c == ']' {
            self.next_char();
            if self.peek() == Some(']') {
                self.next_char();
                return ShellToken::DoubleRBracket;
            }
            return ShellToken::RBracket;
        }

        if c == '{' {
            self.next_char();
            match self.peek() {
                Some(' ') | Some('\t') | Some('\n') | None => {
                    return ShellToken::LBrace;
                }
                _ => {
                    let mut word = String::from("{");
                    let mut depth = 1;
                    while let Some(ch) = self.peek() {
                        if ch == '{' {
                            depth += 1;
                            word.push(self.next_char().unwrap());
                        } else if ch == '}' {
                            depth -= 1;
                            word.push(self.next_char().unwrap());
                            if depth == 0 {
                                break;
                            }
                        } else if (ch == ' ' || ch == '\t' || ch == '\n') && depth == 1 {
                            break;
                        } else {
                            word.push(self.next_char().unwrap());
                        }
                    }
                    while let Some(ch) = self.peek() {
                        if ch.is_whitespace()
                            || ch == ';'
                            || ch == '&'
                            || ch == '|'
                            || ch == '<'
                            || ch == '>'
                            || ch == '('
                            || ch == ')'
                        {
                            break;
                        }
                        word.push(self.next_char().unwrap());
                    }
                    return ShellToken::Word(word);
                }
            }
        }

        if c == '}' {
            self.next_char();
            return ShellToken::RBrace;
        }

        if c == '!' {
            self.next_char();
            if self.peek() == Some('=') {
                self.next_char();
                return ShellToken::Word("!=".to_string());
            }
            if self.peek() == Some('(') {
                let mut word = String::from("!(");
                self.next_char();
                let mut depth = 1;
                while let Some(ch) = self.peek() {
                    word.push(self.next_char().unwrap());
                    if ch == '(' {
                        depth += 1;
                    } else if ch == ')' {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                }
                while let Some(ch) = self.peek() {
                    if ch.is_whitespace()
                        || ch == ';'
                        || ch == '&'
                        || ch == '|'
                        || ch == '<'
                        || ch == '>'
                    {
                        break;
                    }
                    word.push(self.next_char().unwrap());
                }
                return ShellToken::Word(word);
            }
            return ShellToken::Bang;
        }

        if c.is_alphanumeric()
            || c == '_'
            || c == '/'
            || c == '.'
            || c == '-'
            || c == '$'
            || c == '\''
            || c == '"'
            || c == '~'
            || c == '*'
            || c == '?'
            || c == '%'
            || c == '+'
            || c == '@'
            || c == ':'
            || c == '='
            || c == '`'
        {
            return self.read_word();
        }

        self.next_char();
        ShellToken::Word(c.to_string())
    }

    fn read_process_sub(&mut self) -> String {
        let mut content = String::new();
        let mut depth = 1;
        while let Some(c) = self.next_char() {
            if c == '(' {
                depth += 1;
                content.push(c);
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    content.push(')');
                    break;
                }
                content.push(c);
            } else {
                content.push(c);
            }
        }
        content
    }

    fn read_heredoc(&mut self) -> ShellToken {
        while self.peek() == Some(' ') || self.peek() == Some('\t') {
            self.next_char();
        }
        let quoted = self.peek() == Some('\'') || self.peek() == Some('"');
        if quoted {
            self.next_char();
        }
        let mut delimiter = String::new();
        while let Some(c) = self.peek() {
            if c == '\n' || c == ' ' || c == '\t' {
                break;
            }
            if quoted && (c == '\'' || c == '"') {
                self.next_char();
                break;
            }
            delimiter.push(self.next_char().unwrap());
        }
        while let Some(c) = self.peek() {
            if c == '\n' {
                self.next_char();
                break;
            }
            self.next_char();
        }
        let mut content = String::new();
        let mut current_line = String::new();
        while let Some(c) = self.next_char() {
            if c == '\n' {
                if current_line.trim() == delimiter {
                    break;
                }
                content.push_str(&current_line);
                content.push('\n');
                current_line.clear();
            } else {
                current_line.push(c);
            }
        }
        ShellToken::HereDoc(delimiter, content)
    }

    fn read_word(&mut self) -> ShellToken {
        let mut word = String::new();
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' | '\n' | ';' | '&' | '<' | '>' => break,
                '[' => {
                    word.push(self.next_char().unwrap());
                    let mut bracket_depth = 1;
                    while let Some(ch) = self.peek() {
                        word.push(self.next_char().unwrap());
                        if ch == '[' {
                            bracket_depth += 1;
                        } else if ch == ']' {
                            bracket_depth -= 1;
                            if bracket_depth == 0 {
                                break;
                            }
                        }
                        if ch == ' ' || ch == '\t' || ch == '\n' {
                            break;
                        }
                    }
                }
                ']' => {
                    if word.is_empty() {
                        break;
                    }
                    word.push(self.next_char().unwrap());
                }
                '|' | '(' | ')' => {
                    if c == '(' && !word.is_empty() {
                        let last_char = word.chars().last().unwrap();
                        if matches!(last_char, '?' | '*' | '+' | '@' | '!') {
                            word.push(self.next_char().unwrap());
                            let mut depth = 1;
                            while let Some(ch) = self.peek() {
                                word.push(self.next_char().unwrap());
                                if ch == '(' {
                                    depth += 1;
                                } else if ch == ')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                            continue;
                        }
                        if last_char == '=' {
                            word.push(self.next_char().unwrap());
                            let mut depth = 1;
                            let mut in_sq = false;
                            let mut in_dq = false;
                            while let Some(ch) = self.peek() {
                                if in_sq {
                                    if ch == '\'' {
                                        in_sq = false;
                                    }
                                    word.push(self.next_char().unwrap());
                                } else if in_dq {
                                    if ch == '"' {
                                        in_dq = false;
                                    } else if ch == '\\' {
                                        word.push(self.next_char().unwrap());
                                        if self.peek().is_some() {
                                            word.push(self.next_char().unwrap());
                                        }
                                        continue;
                                    }
                                    word.push(self.next_char().unwrap());
                                } else {
                                    match ch {
                                        '\'' => {
                                            in_sq = true;
                                            word.push(self.next_char().unwrap());
                                        }
                                        '"' => {
                                            in_dq = true;
                                            word.push(self.next_char().unwrap());
                                        }
                                        '(' => {
                                            depth += 1;
                                            word.push(self.next_char().unwrap());
                                        }
                                        ')' => {
                                            depth -= 1;
                                            word.push(self.next_char().unwrap());
                                            if depth == 0 {
                                                break;
                                            }
                                        }
                                        '\\' => {
                                            word.push(self.next_char().unwrap());
                                            if self.peek().is_some() {
                                                word.push(self.next_char().unwrap());
                                            }
                                        }
                                        _ => word.push(self.next_char().unwrap()),
                                    }
                                }
                            }
                            continue;
                        }
                    }
                    break;
                }
                '{' => {
                    word.push(self.next_char().unwrap());
                    let mut depth = 1;
                    while let Some(ch) = self.peek() {
                        if ch == '{' {
                            depth += 1;
                            word.push(self.next_char().unwrap());
                        } else if ch == '}' {
                            depth -= 1;
                            word.push(self.next_char().unwrap());
                            if depth == 0 {
                                break;
                            }
                        } else if ch == ' ' || ch == '\t' || ch == '\n' {
                            break;
                        } else {
                            word.push(self.next_char().unwrap());
                        }
                    }
                }
                '}' => break,
                '$' => {
                    word.push(self.next_char().unwrap());
                    if self.peek() == Some('\'') {
                        word.push(self.next_char().unwrap());
                        while let Some(ch) = self.peek() {
                            if ch == '\'' {
                                word.push(self.next_char().unwrap());
                                break;
                            } else if ch == '\\' {
                                word.push(self.next_char().unwrap());
                                if self.peek().is_some() {
                                    word.push(self.next_char().unwrap());
                                }
                            } else {
                                word.push(self.next_char().unwrap());
                            }
                        }
                    } else if self.peek() == Some('{') {
                        word.push(self.next_char().unwrap());
                        let mut depth = 1;
                        while let Some(ch) = self.peek() {
                            if ch == '{' {
                                depth += 1;
                            } else if ch == '}' {
                                depth -= 1;
                                if depth == 0 {
                                    word.push(self.next_char().unwrap());
                                    break;
                                }
                            }
                            word.push(self.next_char().unwrap());
                        }
                    } else if self.peek() == Some('(') {
                        word.push(self.next_char().unwrap());
                        let mut depth = 1;
                        while let Some(ch) = self.peek() {
                            if ch == '(' {
                                depth += 1;
                            } else if ch == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    word.push(self.next_char().unwrap());
                                    break;
                                }
                            }
                            word.push(self.next_char().unwrap());
                        }
                    }
                }
                '=' => {
                    word.push(self.next_char().unwrap());
                    if self.peek() == Some('(') {
                        word.push(self.next_char().unwrap());
                        let mut depth = 1;
                        while let Some(ch) = self.peek() {
                            if ch == '(' {
                                depth += 1;
                            } else if ch == ')' {
                                depth -= 1;
                                if depth == 0 {
                                    word.push(self.next_char().unwrap());
                                    break;
                                }
                            }
                            word.push(self.next_char().unwrap());
                        }
                    }
                }
                '`' => {
                    // Backtick command substitution — keep backticks in the word
                    // so expand_string can see and execute them.
                    word.push(self.next_char().unwrap()); // opening `
                    while let Some(ch) = self.peek() {
                        if ch == '`' {
                            word.push(self.next_char().unwrap()); // closing `
                            break;
                        }
                        word.push(self.next_char().unwrap());
                    }
                }
                '\'' => {
                    self.next_char();
                    while let Some(ch) = self.peek() {
                        if ch == '\'' {
                            self.next_char();
                            break;
                        }
                        let c = self.next_char().unwrap();
                        if matches!(c, '`' | '$' | '(' | ')') {
                            word.push('\x00');
                        }
                        word.push(c);
                    }
                }
                '"' => {
                    self.next_char();
                    while let Some(ch) = self.peek() {
                        if ch == '"' {
                            self.next_char();
                            break;
                        }
                        if ch == '\\' {
                            self.next_char();
                            if let Some(escaped) = self.peek() {
                                match escaped {
                                    '$' | '`' | '"' | '\\' | '\n' => {
                                        word.push(self.next_char().unwrap());
                                    }
                                    _ => {
                                        word.push('\\');
                                        word.push(self.next_char().unwrap());
                                    }
                                }
                            } else {
                                word.push('\\');
                            }
                        } else {
                            word.push(self.next_char().unwrap());
                        }
                    }
                }
                '\\' => {
                    self.next_char();
                    if let Some(escaped) = self.next_char() {
                        word.push(escaped);
                    }
                }
                _ => {
                    word.push(self.next_char().unwrap());
                }
            }
        }

        match word.as_str() {
            "if" => ShellToken::If,
            "then" => ShellToken::Then,
            "else" => ShellToken::Else,
            "elif" => ShellToken::Elif,
            "fi" => ShellToken::Fi,
            "case" => ShellToken::Case,
            "esac" => ShellToken::Esac,
            "for" => ShellToken::For,
            "while" => ShellToken::While,
            "until" => ShellToken::Until,
            "do" => ShellToken::Do,
            "done" => ShellToken::Done,
            "in" => ShellToken::In,
            "function" => ShellToken::Function,
            "select" => ShellToken::Select,
            "time" => ShellToken::Time,
            "coproc" => ShellToken::Coproc,
            "repeat" => ShellToken::Repeat,
            // "always" is NOT a keyword — it's context-dependent.
            // Checked as a string in parse_brace_group_or_try per C zsh par_subsh().
            "typeset" | "local" | "declare" | "export" | "readonly" | "integer" | "float" => {
                ShellToken::Typeset(word)
            }
            _ => ShellToken::Word(word),
        }
    }
}

pub struct ShellParser<'a> {
    lexer: ShellLexer<'a>,
    current: ShellToken,
}

impl<'a> ShellParser<'a> {
    pub fn new(input: &'a str) -> Self {
        let mut lexer = ShellLexer::new(input);
        let current = lexer.next_token();
        Self { lexer, current }
    }

    fn parse_array_elements(content: &str) -> Vec<ShellWord> {
        let mut elements = Vec::new();
        let mut current = String::new();
        let mut chars = content.chars().peekable();
        let mut in_single_quote = false;
        let mut in_double_quote = false;

        while let Some(c) = chars.next() {
            if in_single_quote {
                if c == '\'' {
                    in_single_quote = false;
                    let marked: String = current
                        .chars()
                        .flat_map(|ch| {
                            if matches!(ch, '`' | '$' | '(' | ')') {
                                vec!['\x00', ch]
                            } else {
                                vec![ch]
                            }
                        })
                        .collect();
                    elements.push(ShellWord::Literal(marked));
                    current.clear();
                } else {
                    current.push(c);
                }
            } else if in_double_quote {
                if c == '"' {
                    in_double_quote = false;
                    elements.push(ShellWord::Literal(current.clone()));
                    current.clear();
                } else if c == '\\' {
                    if let Some(&next) = chars.peek() {
                        if matches!(next, '$' | '`' | '"' | '\\') {
                            chars.next();
                            current.push(next);
                        } else {
                            current.push(c);
                        }
                    } else {
                        current.push(c);
                    }
                } else {
                    current.push(c);
                }
            } else {
                match c {
                    '\'' => in_single_quote = true,
                    '"' => in_double_quote = true,
                    '$' if chars.peek() == Some(&'(') => {
                        // Command substitution $(...)  — flush current, collect balanced parens
                        if !current.is_empty() {
                            elements.push(ShellWord::Literal(current.clone()));
                            current.clear();
                        }
                        chars.next(); // consume '('
                        let mut depth = 1;
                        let mut cmd = String::new();
                        while let Some(ch) = chars.next() {
                            if ch == '(' { depth += 1; }
                            if ch == ')' { depth -= 1; if depth == 0 { break; } }
                            cmd.push(ch);
                        }
                        // Parse the command substitution content into an AST
                        let mut sub_parser = ShellParser::new(&cmd);
                        if let Ok(cmds) = sub_parser.parse_script() {
                            if cmds.len() == 1 {
                                elements.push(ShellWord::CommandSub(Box::new(cmds.into_iter().next().unwrap())));
                            } else if !cmds.is_empty() {
                                // Multiple commands — wrap in a brace group
                                elements.push(ShellWord::CommandSub(Box::new(
                                    ShellCommand::Compound(CompoundCommand::BraceGroup(cmds))
                                )));
                            }
                        }
                    }
                    ' ' | '\t' | '\n' => {
                        if !current.is_empty() {
                            elements.push(ShellWord::Literal(current.clone()));
                            current.clear();
                        }
                    }
                    '\\' => {
                        if let Some(next) = chars.next() {
                            current.push(next);
                        }
                    }
                    _ => current.push(c),
                }
            }
        }

        if !current.is_empty() {
            elements.push(ShellWord::Literal(current));
        }

        elements
    }

    fn advance(&mut self) -> ShellToken {
        std::mem::replace(&mut self.current, self.lexer.next_token())
    }

    fn expect(&mut self, expected: ShellToken) -> Result<(), String> {
        if self.current == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("Expected {:?}, got {:?}", expected, self.current))
        }
    }

    fn skip_newlines(&mut self) {
        while self.current == ShellToken::Newline {
            self.advance();
        }
    }

    fn skip_separators(&mut self) {
        while self.current == ShellToken::Newline || self.current == ShellToken::Semi {
            self.advance();
        }
    }

    #[tracing::instrument(level = "trace", skip_all)]
    pub fn parse_script(&mut self) -> Result<Vec<ShellCommand>, String> {
        let mut commands = Vec::new();
        self.skip_newlines();
        while self.current != ShellToken::Eof {
            if let Some(cmd) = self.parse_complete_command()? {
                commands.push(cmd);
            }
            self.skip_newlines();
        }
        Ok(commands)
    }

    fn parse_complete_command(&mut self) -> Result<Option<ShellCommand>, String> {
        self.skip_newlines();
        if self.current == ShellToken::Eof {
            return Ok(None);
        }
        let cmd = self.parse_list()?;
        match &self.current {
            ShellToken::Newline | ShellToken::Semi | ShellToken::Amp => {
                self.advance();
            }
            _ => {}
        }
        Ok(Some(cmd))
    }

    fn parse_list(&mut self) -> Result<ShellCommand, String> {
        let first = self.parse_pipeline()?;
        let mut items = vec![(first, ListOp::Semi)];

        loop {
            let op = match &self.current {
                ShellToken::AmpAmp => ListOp::And,
                ShellToken::PipePipe => ListOp::Or,
                ShellToken::Semi => ListOp::Semi,
                ShellToken::Amp => ListOp::Amp,
                ShellToken::Newline => break,
                _ => break,
            };

            self.advance();
            self.skip_newlines();

            if let Some(last) = items.last_mut() {
                last.1 = op;
            }

            if self.current == ShellToken::Eof
                || self.current == ShellToken::Then
                || self.current == ShellToken::Else
                || self.current == ShellToken::Elif
                || self.current == ShellToken::Fi
                || self.current == ShellToken::Do
                || self.current == ShellToken::Done
                || self.current == ShellToken::Esac
                || self.current == ShellToken::RBrace
                || self.current == ShellToken::RParen
            {
                break;
            }

            let next = self.parse_pipeline()?;
            items.push((next, ListOp::Semi));
        }

        if items.len() == 1 {
            let (cmd, op) = items.pop().unwrap();
            if matches!(op, ListOp::Amp) {
                Ok(ShellCommand::List(vec![(cmd, op)]))
            } else {
                Ok(cmd)
            }
        } else {
            Ok(ShellCommand::List(items))
        }
    }

    fn parse_pipeline(&mut self) -> Result<ShellCommand, String> {
        let negated = if self.current == ShellToken::Bang {
            self.advance();
            true
        } else {
            false
        };

        let first = self.parse_command()?;
        let mut cmds = vec![first];

        while self.current == ShellToken::Pipe {
            self.advance();
            self.skip_newlines();
            cmds.push(self.parse_command()?);
        }

        if cmds.len() == 1 && !negated {
            Ok(cmds.pop().unwrap())
        } else {
            Ok(ShellCommand::Pipeline(cmds, negated))
        }
    }

    fn parse_command(&mut self) -> Result<ShellCommand, String> {
        let cmd = match &self.current {
            ShellToken::If => self.parse_if(),
            ShellToken::For => self.parse_for(),
            ShellToken::While => self.parse_while(),
            ShellToken::Until => self.parse_until(),
            ShellToken::Case => self.parse_case(),
            ShellToken::Repeat => self.parse_repeat(),
            ShellToken::LBrace => self.parse_brace_group_or_try(),
            ShellToken::LParen => {
                self.advance();
                if self.current == ShellToken::RParen {
                    self.advance();
                    self.skip_newlines();
                    if self.current == ShellToken::LBrace {
                        let body = self.parse_brace_group()?;
                        Ok(ShellCommand::FunctionDef(String::new(), Box::new(body)))
                    } else {
                        Ok(ShellCommand::Compound(CompoundCommand::Subshell(vec![])))
                    }
                } else {
                    self.skip_newlines();
                    let body = self.parse_compound_list()?;
                    self.expect(ShellToken::RParen)?;
                    Ok(ShellCommand::Compound(CompoundCommand::Subshell(body)))
                }
            }
            ShellToken::DoubleLBracket => self.parse_cond_command(),
            ShellToken::DoubleLParen => self.parse_arith_command(),
            ShellToken::Function => self.parse_function(),
            ShellToken::Coproc => self.parse_coproc(),
            _ => self.parse_simple_command(),
        }?;

        let mut redirects = Vec::new();
        loop {
            if let ShellToken::Word(w) = &self.current {
                if w.chars().all(|c| c.is_ascii_digit()) {
                    let fd_str = w.clone();
                    self.advance();
                    match &self.current {
                        ShellToken::Less
                        | ShellToken::Greater
                        | ShellToken::GreaterGreater
                        | ShellToken::LessAmp
                        | ShellToken::GreaterAmp
                        | ShellToken::LessLess
                        | ShellToken::LessLessLess
                        | ShellToken::LessGreater
                        | ShellToken::GreaterPipe => {
                            let fd = fd_str.parse::<i32>().ok();
                            redirects.push(self.parse_redirect_with_fd(fd)?);
                            continue;
                        }
                        _ => break,
                    }
                }
            }

            match &self.current {
                ShellToken::Less
                | ShellToken::Greater
                | ShellToken::GreaterGreater
                | ShellToken::LessAmp
                | ShellToken::GreaterAmp
                | ShellToken::LessLess
                | ShellToken::LessLessLess
                | ShellToken::LessGreater
                | ShellToken::GreaterPipe
                | ShellToken::AmpGreater
                | ShellToken::AmpGreaterGreater => {
                    redirects.push(self.parse_redirect_with_fd(None)?);
                }
                _ => break,
            }
        }

        if !redirects.is_empty() {
            Ok(ShellCommand::Compound(CompoundCommand::WithRedirects(
                Box::new(cmd),
                redirects,
            )))
        } else {
            Ok(cmd)
        }
    }

    fn parse_simple_command(&mut self) -> Result<ShellCommand, String> {
        let mut cmd = SimpleCommand {
            assignments: Vec::new(),
            words: Vec::new(),
            redirects: Vec::new(),
        };

        loop {
            match &self.current {
                ShellToken::Word(w) => {
                    if w.starts_with('{') && w.ends_with('}') && w.len() > 2 {
                        let varname = w[1..w.len() - 1].to_string();
                        if varname.chars().all(|c| c.is_alphanumeric() || c == '_') {
                            let saved_word = w.clone();
                            self.advance();
                            match &self.current {
                                ShellToken::Less
                                | ShellToken::Greater
                                | ShellToken::GreaterGreater
                                | ShellToken::LessAmp
                                | ShellToken::GreaterAmp
                                | ShellToken::LessLess
                                | ShellToken::LessLessLess
                                | ShellToken::LessGreater
                                | ShellToken::GreaterPipe => {
                                    let mut redir = self.parse_redirect_with_fd(None)?;
                                    redir.fd_var = Some(varname);
                                    cmd.redirects.push(redir);
                                    continue;
                                }
                                _ => {
                                    cmd.words.push(ShellWord::Literal(saved_word));
                                    continue;
                                }
                            }
                        }
                    }

                    if w.chars().all(|c| c.is_ascii_digit()) {
                        let fd_str = w.clone();
                        self.advance();
                        match &self.current {
                            ShellToken::Less
                            | ShellToken::Greater
                            | ShellToken::GreaterGreater
                            | ShellToken::LessAmp
                            | ShellToken::GreaterAmp
                            | ShellToken::LessLess
                            | ShellToken::LessLessLess
                            | ShellToken::LessGreater
                            | ShellToken::GreaterPipe => {
                                let fd = fd_str.parse::<i32>().ok();
                                cmd.redirects.push(self.parse_redirect_with_fd(fd)?);
                                continue;
                            }
                            _ => {
                                cmd.words.push(ShellWord::Literal(fd_str));
                                continue;
                            }
                        }
                    }

                    if cmd.words.is_empty() && w.contains('=') && !w.starts_with('=') {
                        let (eq_pos, is_append) = if let Some(pos) = w.find("+=") {
                            (pos, true)
                        } else if let Some(pos) = w.find('=') {
                            (pos, false)
                        } else {
                            (0, false)
                        };

                        if eq_pos > 0 {
                            let var = w[..eq_pos].to_string();
                            let val_start = if is_append { eq_pos + 2 } else { eq_pos + 1 };
                            let val = w[val_start..].to_string();

                            let is_valid_var = if let Some(bracket_pos) = var.find('[') {
                                let name = &var[..bracket_pos];
                                let rest = &var[bracket_pos..];
                                name.chars().all(|c| c.is_alphanumeric() || c == '_')
                                    && rest.ends_with(']')
                            } else {
                                var.chars().all(|c| c.is_alphanumeric() || c == '_')
                            };
                            if is_valid_var {
                                if val.starts_with('(') && val.ends_with(')') {
                                    let array_content = &val[1..val.len() - 1];
                                    let elements = Self::parse_array_elements(array_content);
                                    cmd.assignments.push((
                                        var,
                                        ShellWord::ArrayLiteral(elements),
                                        is_append,
                                    ));
                                } else {
                                    cmd.assignments
                                        .push((var, ShellWord::Literal(val), is_append));
                                }
                                self.advance();
                                continue;
                            }
                        }
                    }

                    cmd.words.push(self.parse_word()?);
                }

                ShellToken::LBracket => {
                    cmd.words.push(ShellWord::Literal("[".to_string()));
                    self.advance();
                }
                ShellToken::RBracket => {
                    if !cmd.words.is_empty() {
                        cmd.words.push(ShellWord::Literal("]".to_string()));
                        self.advance();
                    } else {
                        break;
                    }
                }
                ShellToken::If
                | ShellToken::Then
                | ShellToken::Else
                | ShellToken::Elif
                | ShellToken::Fi
                | ShellToken::Case
                | ShellToken::Esac
                | ShellToken::For
                | ShellToken::While
                | ShellToken::Until
                | ShellToken::Do
                | ShellToken::Done
                | ShellToken::In
                | ShellToken::Function
                | ShellToken::Select
                | ShellToken::Time
                | ShellToken::Coproc
                | ShellToken::Repeat => {
                    if !cmd.words.is_empty() {
                        cmd.words.push(self.parse_word()?);
                    } else {
                        break;
                    }
                }

                ShellToken::Typeset(ref name) => {
                    if cmd.words.is_empty() {
                        let name = name.clone();
                        cmd.words.push(ShellWord::Literal(name));
                        self.advance();
                    } else {
                        cmd.words.push(self.parse_word()?);
                    }
                }

                ShellToken::Less
                | ShellToken::Greater
                | ShellToken::GreaterGreater
                | ShellToken::LessAmp
                | ShellToken::GreaterAmp
                | ShellToken::LessLess
                | ShellToken::LessLessLess
                | ShellToken::LessGreater
                | ShellToken::GreaterPipe
                | ShellToken::AmpGreater
                | ShellToken::AmpGreaterGreater
                | ShellToken::HereDoc(_, _) => {
                    cmd.redirects.push(self.parse_redirect_with_fd(None)?);
                }

                _ => break,
            }
        }

        if cmd.words.len() == 1 && self.current == ShellToken::LParen {
            if let ShellWord::Literal(name) = &cmd.words[0] {
                let name = name.clone();
                self.advance();
                self.expect(ShellToken::RParen)?;
                self.skip_newlines();
                let body = self.parse_command()?;
                return Ok(ShellCommand::FunctionDef(name, Box::new(body)));
            }
        }

        Ok(ShellCommand::Simple(cmd))
    }

    fn parse_word(&mut self) -> Result<ShellWord, String> {
        let token = self.advance();
        match token {
            ShellToken::Word(w) => Ok(ShellWord::Literal(w)),
            ShellToken::LBracket => Ok(ShellWord::Literal("[".to_string())),
            ShellToken::If => Ok(ShellWord::Literal("if".to_string())),
            ShellToken::Then => Ok(ShellWord::Literal("then".to_string())),
            ShellToken::Else => Ok(ShellWord::Literal("else".to_string())),
            ShellToken::Elif => Ok(ShellWord::Literal("elif".to_string())),
            ShellToken::Fi => Ok(ShellWord::Literal("fi".to_string())),
            ShellToken::Case => Ok(ShellWord::Literal("case".to_string())),
            ShellToken::Esac => Ok(ShellWord::Literal("esac".to_string())),
            ShellToken::For => Ok(ShellWord::Literal("for".to_string())),
            ShellToken::While => Ok(ShellWord::Literal("while".to_string())),
            ShellToken::Until => Ok(ShellWord::Literal("until".to_string())),
            ShellToken::Do => Ok(ShellWord::Literal("do".to_string())),
            ShellToken::Done => Ok(ShellWord::Literal("done".to_string())),
            ShellToken::In => Ok(ShellWord::Literal("in".to_string())),
            ShellToken::Function => Ok(ShellWord::Literal("function".to_string())),
            ShellToken::Select => Ok(ShellWord::Literal("select".to_string())),
            ShellToken::Time => Ok(ShellWord::Literal("time".to_string())),
            ShellToken::Coproc => Ok(ShellWord::Literal("coproc".to_string())),
            ShellToken::Typeset(name) => Ok(ShellWord::Literal(name)),
            ShellToken::Repeat => Ok(ShellWord::Literal("repeat".to_string())),
            _ => Err("Expected word".to_string()),
        }
    }

    fn parse_redirect_with_fd(&mut self, fd: Option<i32>) -> Result<Redirect, String> {
        if let ShellToken::HereDoc(delimiter, content) = &self.current {
            let delimiter = delimiter.clone();
            let content = content.clone();
            self.advance();
            return Ok(Redirect {
                fd,
                op: RedirectOp::HereDoc,
                target: ShellWord::Literal(delimiter),
                heredoc_content: Some(content),
                fd_var: None,
            });
        }

        let mut fd_var = None;
        if let ShellToken::Word(w) = &self.current {
            if w.starts_with('{') && w.ends_with('}') && w.len() > 2 {
                let varname = w[1..w.len() - 1].to_string();
                fd_var = Some(varname);
                self.advance();
            }
        }

        let op = match self.advance() {
            ShellToken::Less => RedirectOp::Read,
            ShellToken::Greater => RedirectOp::Write,
            ShellToken::GreaterGreater => RedirectOp::Append,
            ShellToken::LessAmp => RedirectOp::DupRead,
            ShellToken::GreaterAmp => RedirectOp::DupWrite,
            ShellToken::LessLess => RedirectOp::HereDoc,
            ShellToken::LessLessLess => RedirectOp::HereString,
            ShellToken::LessGreater => RedirectOp::ReadWrite,
            ShellToken::GreaterPipe => RedirectOp::Clobber,
            ShellToken::AmpGreater => RedirectOp::WriteBoth,
            ShellToken::AmpGreaterGreater => RedirectOp::AppendBoth,
            _ => return Err("Expected redirect operator".to_string()),
        };

        let target = self.parse_word()?;

        Ok(Redirect {
            fd,
            op,
            target,
            heredoc_content: None,
            fd_var,
        })
    }

    fn parse_if(&mut self) -> Result<ShellCommand, String> {
        let mut conditions = Vec::new();
        let mut else_part = None;
        let mut usebrace = false;

        let mut xtok = self.current.clone();
        loop {
            if xtok == ShellToken::Fi {
                self.advance();
                break;
            }

            self.advance();

            if xtok == ShellToken::Else {
                break;
            }

            self.skip_separators();

            if xtok != ShellToken::If && xtok != ShellToken::Elif {
                return Err(format!("Expected If or Elif, got {:?}", xtok));
            }

            let cond = self.parse_compound_list_until(&[ShellToken::Then, ShellToken::LBrace])?;
            self.skip_separators();
            xtok = ShellToken::Fi;

            if self.current == ShellToken::Then {
                usebrace = false;
                self.advance();
                let body = self.parse_compound_list()?;
                conditions.push((cond, body));
            } else if self.current == ShellToken::LBrace {
                usebrace = true;
                self.advance();
                self.skip_separators();
                let body = self.parse_compound_list_until(&[ShellToken::RBrace])?;
                if self.current != ShellToken::RBrace {
                    return Err(format!("Expected RBrace, got {:?}", self.current));
                }
                conditions.push((cond, body));
                self.advance();
                if self.current == ShellToken::Newline || self.current == ShellToken::Semi {
                    break;
                }
            } else {
                return Err(format!(
                    "Expected Then or LBrace after condition, got {:?}",
                    self.current
                ));
            }

            xtok = self.current.clone();
            if xtok != ShellToken::Elif && xtok != ShellToken::Else && xtok != ShellToken::Fi {
                break;
            }
        }

        if xtok == ShellToken::Else || self.current == ShellToken::Else {
            if self.current == ShellToken::Else {
                self.advance();
            }
            self.skip_separators();

            if self.current == ShellToken::LBrace && usebrace {
                self.advance();
                self.skip_separators();
                let body = self.parse_compound_list_until(&[ShellToken::RBrace])?;
                if self.current != ShellToken::RBrace {
                    return Err(format!("Expected RBrace in else, got {:?}", self.current));
                }
                self.advance();
                else_part = Some(body);
            } else {
                let body = self.parse_compound_list()?;
                if self.current != ShellToken::Fi {
                    return Err(format!("Expected Fi, got {:?}", self.current));
                }
                self.advance();
                else_part = Some(body);
            }
        }

        Ok(ShellCommand::Compound(CompoundCommand::If {
            conditions,
            else_part,
        }))
    }

    fn parse_for(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::For)?;
        self.skip_newlines();

        if self.current == ShellToken::DoubleLParen {
            return self.parse_for_arith();
        }

        let var = if let ShellToken::Word(w) = self.advance() {
            w
        } else {
            return Err("Expected variable name after 'for'".to_string());
        };

        while self.current == ShellToken::Newline {
            self.advance();
        }

        let words = if self.current == ShellToken::In {
            self.advance();
            let mut words = Vec::new();
            while let ShellToken::Word(_) = &self.current {
                words.push(self.parse_word()?);
            }
            Some(words)
        } else if self.current == ShellToken::LParen {
            self.advance();
            let mut words = Vec::new();
            while self.current != ShellToken::RParen && self.current != ShellToken::Eof {
                if let ShellToken::Word(_) = &self.current {
                    words.push(self.parse_word()?);
                } else if self.current == ShellToken::Newline {
                    self.advance();
                } else {
                    break;
                }
            }
            self.expect(ShellToken::RParen)?;
            Some(words)
        } else {
            None
        };

        self.skip_separators();

        let body = if self.current == ShellToken::LBrace {
            self.advance();
            let body = self.parse_compound_list_until(&[ShellToken::RBrace])?;
            self.expect(ShellToken::RBrace)?;
            body
        } else {
            self.expect(ShellToken::Do)?;
            let body = self.parse_compound_list()?;
            self.expect(ShellToken::Done)?;
            body
        };

        Ok(ShellCommand::Compound(CompoundCommand::For {
            var,
            words,
            body,
        }))
    }

    fn parse_for_arith(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::DoubleLParen)?;

        let mut parts = Vec::new();
        let mut current_part = String::new();
        let mut depth = 0;

        loop {
            match &self.current {
                ShellToken::DoubleRParen if depth == 0 => break,
                ShellToken::DoubleLParen => {
                    depth += 1;
                    current_part.push_str("((");
                    self.advance();
                }
                ShellToken::DoubleRParen => {
                    depth -= 1;
                    current_part.push_str("))");
                    self.advance();
                }
                ShellToken::Semi => {
                    parts.push(current_part.trim().to_string());
                    current_part = String::new();
                    self.advance();
                }
                ShellToken::Word(w) => {
                    current_part.push_str(w);
                    current_part.push(' ');
                    self.advance();
                }
                ShellToken::Less => {
                    current_part.push('<');
                    self.advance();
                }
                ShellToken::Greater => {
                    current_part.push('>');
                    self.advance();
                }
                ShellToken::LessLess => {
                    current_part.push_str("<<");
                    self.advance();
                }
                ShellToken::GreaterGreater => {
                    current_part.push_str(">>");
                    self.advance();
                }
                _ => {
                    self.advance();
                }
            }
        }
        parts.push(current_part.trim().to_string());

        self.expect(ShellToken::DoubleRParen)?;
        self.skip_newlines();

        match &self.current {
            ShellToken::Semi | ShellToken::Newline => {
                self.advance();
            }
            _ => {}
        }
        self.skip_newlines();

        self.expect(ShellToken::Do)?;
        self.skip_newlines();
        let body = self.parse_compound_list()?;
        self.expect(ShellToken::Done)?;

        Ok(ShellCommand::Compound(CompoundCommand::ForArith {
            init: parts.first().cloned().unwrap_or_default(),
            cond: parts.get(1).cloned().unwrap_or_default(),
            step: parts.get(2).cloned().unwrap_or_default(),
            body,
        }))
    }

    fn parse_while(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::While)?;
        let condition = self.parse_compound_list_until(&[ShellToken::Do, ShellToken::LBrace])?;
        self.skip_separators();

        let body = if self.current == ShellToken::LBrace {
            self.advance();
            let body = self.parse_compound_list_until(&[ShellToken::RBrace])?;
            self.expect(ShellToken::RBrace)?;
            body
        } else {
            self.expect(ShellToken::Do)?;
            let body = self.parse_compound_list()?;
            self.expect(ShellToken::Done)?;
            body
        };

        Ok(ShellCommand::Compound(CompoundCommand::While {
            condition,
            body,
        }))
    }

    fn parse_until(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::Until)?;
        let condition = self.parse_compound_list_until(&[ShellToken::Do, ShellToken::LBrace])?;
        self.skip_separators();

        let body = if self.current == ShellToken::LBrace {
            self.advance();
            let body = self.parse_compound_list_until(&[ShellToken::RBrace])?;
            self.expect(ShellToken::RBrace)?;
            body
        } else {
            self.expect(ShellToken::Do)?;
            let body = self.parse_compound_list()?;
            self.expect(ShellToken::Done)?;
            body
        };

        Ok(ShellCommand::Compound(CompoundCommand::Until {
            condition,
            body,
        }))
    }

    fn parse_case(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::Case)?;
        self.skip_newlines();
        let word = self.parse_word()?;
        self.skip_newlines();
        self.expect(ShellToken::In)?;
        self.skip_newlines();

        let mut cases = Vec::new();

        while self.current != ShellToken::Esac {
            let mut patterns = Vec::new();
            if self.current == ShellToken::LParen {
                self.advance();
            }

            loop {
                patterns.push(self.parse_word()?);
                if self.current == ShellToken::Pipe {
                    self.advance();
                } else {
                    break;
                }
            }

            self.expect(ShellToken::RParen)?;
            self.skip_newlines();

            let body = self.parse_compound_list()?;

            let term = match &self.current {
                ShellToken::DoubleSemi => {
                    self.advance();
                    CaseTerminator::Break
                }
                ShellToken::SemiAmp => {
                    self.advance();
                    CaseTerminator::Fallthrough
                }
                ShellToken::SemiSemiAmp => {
                    self.advance();
                    CaseTerminator::Continue
                }
                _ => CaseTerminator::Break,
            };

            cases.push((patterns, body, term));
            self.skip_newlines();
        }

        self.expect(ShellToken::Esac)?;

        Ok(ShellCommand::Compound(CompoundCommand::Case {
            word,
            cases,
        }))
    }

    fn parse_repeat(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::Repeat)?;

        let count = match &self.current {
            ShellToken::Word(w) => {
                let c = w.clone();
                self.advance();
                c
            }
            _ => return Err("expected count after 'repeat'".to_string()),
        };

        self.skip_separators();

        let body = if self.current == ShellToken::LBrace {
            self.advance();
            self.skip_newlines();
            let body = self.parse_compound_list_until(&[ShellToken::RBrace])?;
            self.expect(ShellToken::RBrace)?;
            body
        } else if self.current == ShellToken::Do {
            self.expect(ShellToken::Do)?;
            let body = self.parse_compound_list()?;
            self.expect(ShellToken::Done)?;
            body
        } else {
            // repeat N SIMPLE_COMMAND (no braces or do/done needed)
            let cmd = self.parse_command()?;
            vec![cmd]
        };

        Ok(ShellCommand::Compound(CompoundCommand::Repeat {
            count,
            body,
        }))
    }

    fn parse_brace_group_or_try(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::LBrace)?;
        self.skip_newlines();
        let try_body = self.parse_compound_list()?;
        self.expect(ShellToken::RBrace)?;

        // C zsh: tok == STRING && !strcmp(tokstr, "always")
        // "always" is context-dependent, only keyword after OUTBRACE.
        if matches!(&self.current, ShellToken::Word(w) if w == "always") {
            self.advance();
            self.expect(ShellToken::LBrace)?;
            self.skip_newlines();
            let always_body = self.parse_compound_list()?;
            self.expect(ShellToken::RBrace)?;

            Ok(ShellCommand::Compound(CompoundCommand::Try {
                try_body,
                always_body,
            }))
        } else {
            Ok(ShellCommand::Compound(CompoundCommand::BraceGroup(
                try_body,
            )))
        }
    }

    fn parse_brace_group(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::LBrace)?;
        self.skip_newlines();
        let body = self.parse_compound_list()?;
        self.expect(ShellToken::RBrace)?;

        Ok(ShellCommand::Compound(CompoundCommand::BraceGroup(body)))
    }

    fn parse_cond_command(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::DoubleLBracket)?;

        let mut tokens: Vec<String> = Vec::new();
        while self.current != ShellToken::DoubleRBracket && self.current != ShellToken::Eof {
            match &self.current {
                ShellToken::Word(w) => tokens.push(w.clone()),
                ShellToken::Bang => tokens.push("!".to_string()),
                ShellToken::AmpAmp => tokens.push("&&".to_string()),
                ShellToken::PipePipe => tokens.push("||".to_string()),
                ShellToken::LParen => tokens.push("(".to_string()),
                ShellToken::RParen => tokens.push(")".to_string()),
                ShellToken::Less => tokens.push("<".to_string()),
                ShellToken::Greater => tokens.push(">".to_string()),
                _ => {}
            }
            self.advance();
        }

        self.expect(ShellToken::DoubleRBracket)?;

        let expr = self.parse_cond_tokens(&tokens)?;
        Ok(ShellCommand::Compound(CompoundCommand::Cond(expr)))
    }

    fn parse_cond_tokens(&self, tokens: &[String]) -> Result<CondExpr, String> {
        if tokens.is_empty() {
            return Ok(CondExpr::StringNonEmpty(ShellWord::Literal(String::new())));
        }

        if tokens[0] == "!" {
            let inner = self.parse_cond_tokens(&tokens[1..])?;
            return Ok(CondExpr::Not(Box::new(inner)));
        }

        // Precedence: || (lowest) > && > comparisons (highest).
        // Scan for || first, then &&, then binary operators.
        // This matches the C implementation's precedence in cond.c.

        // Level 1: || (lowest precedence — split on rightmost to get left-associativity)
        for i in (0..tokens.len()).rev() {
            if tokens[i] == "||" {
                let left = self.parse_cond_tokens(&tokens[..i])?;
                let right = self.parse_cond_tokens(&tokens[i + 1..])?;
                return Ok(CondExpr::Or(Box::new(left), Box::new(right)));
            }
        }

        // Level 2: &&
        for i in (0..tokens.len()).rev() {
            if tokens[i] == "&&" {
                let left = self.parse_cond_tokens(&tokens[..i])?;
                let right = self.parse_cond_tokens(&tokens[i + 1..])?;
                return Ok(CondExpr::And(Box::new(left), Box::new(right)));
            }
        }

        // Level 3: binary comparison operators (highest precedence)
        for (i, tok) in tokens.iter().enumerate() {
            match tok.as_str() {
                "=" | "==" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::StringEqual(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "!=" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::StringNotEqual(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "=~" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::StringMatch(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "-eq" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::NumEqual(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "-ne" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::NumNotEqual(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "-lt" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::NumLess(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "-le" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::NumLessEqual(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "-gt" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::NumGreater(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "-ge" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::NumGreaterEqual(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                "<" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::StringLess(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                ">" => {
                    let left = tokens[..i].join(" ");
                    let right = tokens[i + 1..].join(" ");
                    return Ok(CondExpr::StringGreater(
                        ShellWord::Literal(left),
                        ShellWord::Literal(right),
                    ));
                }
                _ => {}
            }
        }

        if tokens.len() >= 2 {
            let op = &tokens[0];
            let arg = tokens[1..].join(" ");
            match op.as_str() {
                "-e" => return Ok(CondExpr::FileExists(ShellWord::Literal(arg))),
                "-f" => return Ok(CondExpr::FileRegular(ShellWord::Literal(arg))),
                "-d" => return Ok(CondExpr::FileDirectory(ShellWord::Literal(arg))),
                "-L" | "-h" => return Ok(CondExpr::FileSymlink(ShellWord::Literal(arg))),
                "-r" => return Ok(CondExpr::FileReadable(ShellWord::Literal(arg))),
                "-w" => return Ok(CondExpr::FileWritable(ShellWord::Literal(arg))),
                "-x" => return Ok(CondExpr::FileExecutable(ShellWord::Literal(arg))),
                "-s" => return Ok(CondExpr::FileNonEmpty(ShellWord::Literal(arg))),
                "-z" => return Ok(CondExpr::StringEmpty(ShellWord::Literal(arg))),
                "-n" => return Ok(CondExpr::StringNonEmpty(ShellWord::Literal(arg))),
                _ => {}
            }
        }

        let expr_str = tokens.join(" ");
        Ok(CondExpr::StringNonEmpty(ShellWord::Literal(expr_str)))
    }

    fn parse_arith_command(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::DoubleLParen)?;

        let mut expr = String::new();
        let mut depth = 1;

        while depth > 0 {
            match &self.current {
                ShellToken::DoubleLParen => {
                    depth += 1;
                    expr.push_str("((");
                }
                ShellToken::DoubleRParen => {
                    depth -= 1;
                    if depth > 0 {
                        expr.push_str("))");
                    }
                }
                ShellToken::Word(w) => {
                    expr.push_str(w);
                    expr.push(' ');
                }
                ShellToken::LParen => expr.push('('),
                ShellToken::RParen => expr.push(')'),
                ShellToken::LBracket => expr.push('['),
                ShellToken::RBracket => expr.push(']'),
                ShellToken::Less => expr.push('<'),
                ShellToken::Greater => expr.push('>'),
                ShellToken::LessLess => expr.push_str("<<"),
                ShellToken::GreaterGreater => expr.push_str(">>"),
                ShellToken::Bang => expr.push('!'),
                ShellToken::Eof => break,
                _ => {}
            }
            self.advance();
        }

        Ok(ShellCommand::Compound(CompoundCommand::Arith(
            expr.trim().to_string(),
        )))
    }

    fn parse_function(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::Function)?;
        self.skip_newlines();
        let name = if let ShellToken::Word(w) = self.advance() {
            w
        } else {
            return Err("Expected function name".to_string());
        };

        self.skip_newlines();
        if self.current == ShellToken::LParen {
            self.advance();
            self.expect(ShellToken::RParen)?;
            self.skip_newlines();
        }

        let body = self.parse_command()?;
        Ok(ShellCommand::FunctionDef(name, Box::new(body)))
    }

    fn parse_coproc(&mut self) -> Result<ShellCommand, String> {
        self.expect(ShellToken::Coproc)?;
        self.skip_newlines();

        let name = if let ShellToken::Word(w) = &self.current {
            let n = w.clone();
            self.advance();
            self.skip_newlines();
            Some(n)
        } else {
            None
        };

        let body = self.parse_command()?;
        Ok(ShellCommand::Compound(CompoundCommand::Coproc {
            name,
            body: Box::new(body),
        }))
    }

    fn parse_compound_list(&mut self) -> Result<Vec<ShellCommand>, String> {
        let mut commands = Vec::new();
        self.skip_newlines();

        while self.current != ShellToken::Eof
            && self.current != ShellToken::RBrace
            && self.current != ShellToken::RParen
            && self.current != ShellToken::Fi
            && self.current != ShellToken::Done
            && self.current != ShellToken::Esac
            && self.current != ShellToken::Elif
            && self.current != ShellToken::Else
            && self.current != ShellToken::DoubleSemi
            && self.current != ShellToken::SemiAmp
            && self.current != ShellToken::SemiSemiAmp
        {
            let cmd = self.parse_list()?;
            commands.push(cmd);
            match &self.current {
                ShellToken::Newline | ShellToken::Semi => {
                    self.advance();
                }
                _ => {}
            }
            self.skip_newlines();
        }

        Ok(commands)
    }

    fn parse_compound_list_until(
        &mut self,
        terminators: &[ShellToken],
    ) -> Result<Vec<ShellCommand>, String> {
        let mut commands = Vec::new();
        self.skip_newlines();

        while self.current != ShellToken::Eof && !terminators.contains(&self.current) {
            let cmd = self.parse_list()?;
            commands.push(cmd);
            match &self.current {
                ShellToken::Newline | ShellToken::Semi => {
                    self.advance();
                }
                _ => {}
            }
            self.skip_newlines();
        }

        Ok(commands)
    }
}

/// The Zsh Parser
pub struct ZshParser<'a> {
    lexer: ZshLexer<'a>,
    errors: Vec<ParseError>,
    /// Global iteration counter to prevent infinite loops
    global_iterations: usize,
    /// Recursion depth counter to prevent stack overflow
    recursion_depth: usize,
}

const MAX_RECURSION_DEPTH: usize = 500;

impl<'a> ZshParser<'a> {
    /// Create a new parser
    pub fn new(input: &'a str) -> Self {
        ZshParser {
            lexer: ZshLexer::new(input),
            errors: Vec::new(),
            global_iterations: 0,
            recursion_depth: 0,
        }
    }

    /// Check iteration limit; returns true if exceeded
    #[inline]
    fn check_limit(&mut self) -> bool {
        self.global_iterations += 1;
        self.global_iterations > 10_000
    }

    /// Check recursion depth; returns true if exceeded
    #[inline]
    fn check_recursion(&mut self) -> bool {
        self.recursion_depth > MAX_RECURSION_DEPTH
    }

    /// Parse the complete input
    pub fn parse(&mut self) -> Result<ZshProgram, Vec<ParseError>> {
        self.lexer.zshlex();

        let program = self.parse_program_until(None);

        if !self.errors.is_empty() {
            return Err(std::mem::take(&mut self.errors));
        }

        Ok(program)
    }

    /// Parse a program (list of lists)
    fn parse_program(&mut self) -> ZshProgram {
        self.parse_program_until(None)
    }

    /// Parse a program until we hit an end token
    fn parse_program_until(&mut self, end_tokens: Option<&[LexTok]>) -> ZshProgram {
        let mut lists = Vec::new();

        loop {
            if self.check_limit() {
                self.error("parser exceeded global iteration limit");
                break;
            }

            // Skip separators
            while self.lexer.tok == LexTok::Seper || self.lexer.tok == LexTok::Newlin {
                if self.check_limit() {
                    self.error("parser exceeded global iteration limit");
                    return ZshProgram { lists };
                }
                self.lexer.zshlex();
            }

            if self.lexer.tok == LexTok::Endinput || self.lexer.tok == LexTok::Lexerr {
                break;
            }

            // Check for end tokens
            if let Some(end_toks) = end_tokens {
                if end_toks.contains(&self.lexer.tok) {
                    break;
                }
            }

            // Also stop at these tokens when not explicitly looking for them
            // Note: Else/Elif/Then are NOT here - they're handled by parse_if
            // to allow nested if statements inside case arms, loops, etc.
            match self.lexer.tok {
                LexTok::Outbrace
                | LexTok::Dsemi
                | LexTok::Semiamp
                | LexTok::Semibar
                | LexTok::Done
                | LexTok::Fi
                | LexTok::Esac
                | LexTok::Zend => break,
                _ => {}
            }

            match self.parse_list() {
                Some(list) => lists.push(list),
                None => break,
            }
        }

        ZshProgram { lists }
    }

    /// Parse a list (sublist with optional & or ;)
    fn parse_list(&mut self) -> Option<ZshList> {
        let sublist = self.parse_sublist()?;

        let flags = match self.lexer.tok {
            LexTok::Amper => {
                self.lexer.zshlex();
                ListFlags {
                    async_: true,
                    disown: false,
                }
            }
            LexTok::Amperbang => {
                self.lexer.zshlex();
                ListFlags {
                    async_: true,
                    disown: true,
                }
            }
            LexTok::Seper | LexTok::Semi | LexTok::Newlin => {
                self.lexer.zshlex();
                ListFlags::default()
            }
            _ => ListFlags::default(),
        };

        Some(ZshList { sublist, flags })
    }

    /// Parse a sublist (pipelines connected by && or ||)
    fn parse_sublist(&mut self) -> Option<ZshSublist> {
        self.recursion_depth += 1;
        if self.check_recursion() {
            self.error("parse_sublist: max recursion depth exceeded");
            self.recursion_depth -= 1;
            return None;
        }

        let mut flags = SublistFlags::default();

        // Handle coproc and !
        if self.lexer.tok == LexTok::Coproc {
            flags.coproc = true;
            self.lexer.zshlex();
        } else if self.lexer.tok == LexTok::Bang {
            flags.not = true;
            self.lexer.zshlex();
        }

        let pipe = match self.parse_pipe() {
            Some(p) => p,
            None => {
                self.recursion_depth -= 1;
                return None;
            }
        };

        // Check for && or ||
        let next = match self.lexer.tok {
            LexTok::Damper => {
                self.lexer.zshlex();
                self.skip_separators();
                self.parse_sublist().map(|s| (SublistOp::And, Box::new(s)))
            }
            LexTok::Dbar => {
                self.lexer.zshlex();
                self.skip_separators();
                self.parse_sublist().map(|s| (SublistOp::Or, Box::new(s)))
            }
            _ => None,
        };

        self.recursion_depth -= 1;
        Some(ZshSublist { pipe, next, flags })
    }

    /// Parse a pipeline
    fn parse_pipe(&mut self) -> Option<ZshPipe> {
        self.recursion_depth += 1;
        if self.check_recursion() {
            self.error("parse_pipe: max recursion depth exceeded");
            self.recursion_depth -= 1;
            return None;
        }

        let lineno = self.lexer.toklineno;
        let cmd = match self.parse_cmd() {
            Some(c) => c,
            None => {
                self.recursion_depth -= 1;
                return None;
            }
        };

        // Check for | or |&
        let next = match self.lexer.tok {
            LexTok::Bar | LexTok::Baramp => {
                let _merge_stderr = self.lexer.tok == LexTok::Baramp;
                self.lexer.zshlex();
                self.skip_separators();
                self.parse_pipe().map(Box::new)
            }
            _ => None,
        };

        self.recursion_depth -= 1;
        Some(ZshPipe { cmd, next, lineno })
    }

    /// Parse a command
    fn parse_cmd(&mut self) -> Option<ZshCommand> {
        // Parse leading redirections
        let mut redirs = Vec::new();
        while self.lexer.tok.is_redirop() {
            if let Some(redir) = self.parse_redir() {
                redirs.push(redir);
            }
        }

        let cmd = match self.lexer.tok {
            LexTok::For | LexTok::Foreach => self.parse_for(),
            LexTok::Select => self.parse_select(),
            LexTok::Case => self.parse_case(),
            LexTok::If => self.parse_if(),
            LexTok::While => self.parse_while(false),
            LexTok::Until => self.parse_while(true),
            LexTok::Repeat => self.parse_repeat(),
            LexTok::Inpar => self.parse_subsh(),
            LexTok::Inbrace => self.parse_cursh(),
            LexTok::Func => self.parse_funcdef(),
            LexTok::Dinbrack => self.parse_cond(),
            LexTok::Dinpar => self.parse_arith(),
            LexTok::Time => self.parse_time(),
            _ => self.parse_simple(redirs),
        };

        // Parse trailing redirections
        if cmd.is_some() {
            while self.lexer.tok.is_redirop() {
                if let Some(_redir) = self.parse_redir() {
                    // Append to command redirections
                    // (for non-simple commands, we'd need to handle this differently)
                }
            }
        }

        cmd
    }

    /// Parse a simple command
    fn parse_simple(&mut self, mut redirs: Vec<ZshRedir>) -> Option<ZshCommand> {
        let mut assigns = Vec::new();
        let mut words = Vec::new();
        const MAX_ITERATIONS: usize = 10_000;
        let mut iterations = 0;

        // Parse leading assignments
        while self.lexer.tok == LexTok::Envstring || self.lexer.tok == LexTok::Envarray {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error("parse_simple: exceeded max iterations in assignments");
                return None;
            }
            if let Some(assign) = self.parse_assign() {
                assigns.push(assign);
            }
            self.lexer.zshlex();
        }

        // Parse words and redirections
        loop {
            iterations += 1;
            if iterations > MAX_ITERATIONS {
                self.error("parse_simple: exceeded max iterations");
                return None;
            }
            match self.lexer.tok {
                LexTok::String | LexTok::Typeset => {
                    let s = self.lexer.tokstr.clone();
                    if let Some(s) = s {
                        words.push(s);
                    }
                    self.lexer.zshlex();
                    // Check for function definition foo() { ... }
                    if words.len() == 1 && self.peek_inoutpar() {
                        return self.parse_inline_funcdef(words.pop().unwrap());
                    }
                }
                _ if self.lexer.tok.is_redirop() => {
                    match self.parse_redir() {
                        Some(redir) => redirs.push(redir),
                        None => break, // Error in redir parsing, stop
                    }
                }
                LexTok::Inoutpar if !words.is_empty() => {
                    // foo() { ... } style function
                    return self.parse_inline_funcdef(words.pop().unwrap());
                }
                _ => break,
            }
        }

        if assigns.is_empty() && words.is_empty() && redirs.is_empty() {
            return None;
        }

        Some(ZshCommand::Simple(ZshSimple {
            assigns,
            words,
            redirs,
        }))
    }

    /// Parse an assignment
    fn parse_assign(&mut self) -> Option<ZshAssign> {
        use crate::tokens::char_tokens;

        let tokstr = self.lexer.tokstr.as_ref()?;

        // Parse name=value or name+=value
        // The '=' is encoded as char_tokens::EQUALS in the token string
        let (name, value_str, append) = if let Some(pos) = tokstr.find(char_tokens::EQUALS) {
            let name_part = &tokstr[..pos];
            let (name, append) = if name_part.ends_with('+') {
                (&name_part[..name_part.len() - 1], true)
            } else {
                (name_part, false)
            };
            (
                name.to_string(),
                tokstr[pos + char_tokens::EQUALS.len_utf8()..].to_string(),
                append,
            )
        } else if let Some(pos) = tokstr.find('=') {
            // Fallback to literal '=' for compatibility
            let name_part = &tokstr[..pos];
            let (name, append) = if name_part.ends_with('+') {
                (&name_part[..name_part.len() - 1], true)
            } else {
                (name_part, false)
            };
            (name.to_string(), tokstr[pos + 1..].to_string(), append)
        } else {
            return None;
        };

        let value = if self.lexer.tok == LexTok::Envarray {
            // Array assignment: name=(...)
            let mut elements = Vec::new();
            self.lexer.zshlex(); // skip past token

            let mut arr_iters = 0;
            const MAX_ARRAY_ELEMENTS: usize = 10_000;
            while matches!(
                self.lexer.tok,
                LexTok::String | LexTok::Seper | LexTok::Newlin
            ) {
                arr_iters += 1;
                if arr_iters > MAX_ARRAY_ELEMENTS {
                    self.error("array assignment exceeded maximum elements");
                    break;
                }
                if self.lexer.tok == LexTok::String {
                    if let Some(ref s) = self.lexer.tokstr {
                        elements.push(s.clone());
                    }
                }
                self.lexer.zshlex();
            }

            // Expect OUTPAR
            if self.lexer.tok == LexTok::Outpar {
                self.lexer.zshlex();
            }

            ZshAssignValue::Array(elements)
        } else {
            ZshAssignValue::Scalar(value_str)
        };

        Some(ZshAssign {
            name,
            value,
            append,
        })
    }

    /// Parse a redirection
    fn parse_redir(&mut self) -> Option<ZshRedir> {
        let rtype = match self.lexer.tok {
            LexTok::Outang => RedirType::Write,
            LexTok::Outangbang => RedirType::Writenow,
            LexTok::Doutang => RedirType::Append,
            LexTok::Doutangbang => RedirType::Appendnow,
            LexTok::Inang => RedirType::Read,
            LexTok::Inoutang => RedirType::ReadWrite,
            LexTok::Dinang => RedirType::Heredoc,
            LexTok::Dinangdash => RedirType::HeredocDash,
            LexTok::Trinang => RedirType::Herestr,
            LexTok::Inangamp => RedirType::MergeIn,
            LexTok::Outangamp => RedirType::MergeOut,
            LexTok::Ampoutang => RedirType::ErrWrite,
            LexTok::Outangampbang => RedirType::ErrWritenow,
            LexTok::Doutangamp => RedirType::ErrAppend,
            LexTok::Doutangampbang => RedirType::ErrAppendnow,
            _ => return None,
        };

        let fd = if self.lexer.tokfd >= 0 {
            self.lexer.tokfd
        } else if matches!(
            rtype,
            RedirType::Read
                | RedirType::ReadWrite
                | RedirType::MergeIn
                | RedirType::Heredoc
                | RedirType::HeredocDash
                | RedirType::Herestr
        ) {
            0
        } else {
            1
        };

        self.lexer.zshlex();

        let name = match self.lexer.tok {
            LexTok::String | LexTok::Envstring => {
                let n = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.zshlex();
                n
            }
            _ => {
                self.error("expected word after redirection");
                return None;
            }
        };

        // Handle heredoc
        let heredoc = if matches!(rtype, RedirType::Heredoc | RedirType::HeredocDash) {
            // Heredoc content will be filled in by the lexer
            None // Placeholder
        } else {
            None
        };

        Some(ZshRedir {
            rtype,
            fd,
            name,
            heredoc,
            varid: None,
        })
    }

    /// Parse for/foreach loop
    fn parse_for(&mut self) -> Option<ZshCommand> {
        let is_foreach = self.lexer.tok == LexTok::Foreach;
        self.lexer.zshlex();

        // Check for C-style: for (( init; cond; step ))
        if self.lexer.tok == LexTok::Dinpar {
            return self.parse_for_cstyle();
        }

        // Get variable name
        let var = match self.lexer.tok {
            LexTok::String => {
                let v = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.zshlex();
                v
            }
            _ => {
                self.error("expected variable name in for");
                return None;
            }
        };

        // Skip newlines
        self.skip_separators();

        // Get list
        let list = if self.lexer.tok == LexTok::String {
            let s = self.lexer.tokstr.as_ref();
            if s.map(|s| s == "in").unwrap_or(false) {
                self.lexer.zshlex();
                let mut words = Vec::new();
                let mut word_count = 0;
                while self.lexer.tok == LexTok::String {
                    word_count += 1;
                    if word_count > 500 || self.check_limit() {
                        self.error("for: too many words");
                        return None;
                    }
                    if let Some(ref s) = self.lexer.tokstr {
                        words.push(s.clone());
                    }
                    self.lexer.zshlex();
                }
                ForList::Words(words)
            } else {
                ForList::Positional
            }
        } else if self.lexer.tok == LexTok::Inpar {
            // for var (...)
            self.lexer.zshlex();
            let mut words = Vec::new();
            let mut word_count = 0;
            while self.lexer.tok == LexTok::String || self.lexer.tok == LexTok::Seper {
                word_count += 1;
                if word_count > 500 || self.check_limit() {
                    self.error("for: too many words in parens");
                    return None;
                }
                if self.lexer.tok == LexTok::String {
                    if let Some(ref s) = self.lexer.tokstr {
                        words.push(s.clone());
                    }
                }
                self.lexer.zshlex();
            }
            if self.lexer.tok == LexTok::Outpar {
                self.lexer.zshlex();
            }
            ForList::Words(words)
        } else {
            ForList::Positional
        };

        // Skip to body
        self.skip_separators();

        // Parse body
        let body = self.parse_loop_body(is_foreach)?;

        Some(ZshCommand::For(ZshFor {
            var,
            list,
            body: Box::new(body),
        }))
    }

    /// Parse C-style for loop: for (( init; cond; step ))
    fn parse_for_cstyle(&mut self) -> Option<ZshCommand> {
        // We're at (( (Dinpar None) - the opening ((
        // Lexer returns:
        //   Dinpar None     - opening ((
        //   Dinpar "init"   - init expression, semicolon consumed
        //   Dinpar "cond"   - cond expression, semicolon consumed
        //   Doutpar "step"  - step expression, closing )) consumed

        self.lexer.zshlex(); // Get init: Dinpar "i=0"

        if self.lexer.tok != LexTok::Dinpar {
            self.error("expected init expression in for ((");
            return None;
        }
        let init = self.lexer.tokstr.clone().unwrap_or_default();

        self.lexer.zshlex(); // Get cond: Dinpar "i<10"

        if self.lexer.tok != LexTok::Dinpar {
            self.error("expected condition in for ((");
            return None;
        }
        let cond = self.lexer.tokstr.clone().unwrap_or_default();

        self.lexer.zshlex(); // Get step: Doutpar "i++"

        if self.lexer.tok != LexTok::Doutpar {
            self.error("expected )) in for");
            return None;
        }
        let step = self.lexer.tokstr.clone().unwrap_or_default();

        self.lexer.zshlex(); // Move past ))

        self.skip_separators();
        let body = self.parse_loop_body(false)?;

        Some(ZshCommand::For(ZshFor {
            var: String::new(),
            list: ForList::CStyle { init, cond, step },
            body: Box::new(body),
        }))
    }

    /// Parse select loop (same syntax as for)
    fn parse_select(&mut self) -> Option<ZshCommand> {
        self.parse_for()
    }

    /// Parse case statement
    fn parse_case(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip 'case'

        let word = match self.lexer.tok {
            LexTok::String => {
                let w = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.zshlex();
                w
            }
            _ => {
                self.error("expected word after case");
                return None;
            }
        };

        self.skip_separators();

        // Expect 'in' or {
        let use_brace = self.lexer.tok == LexTok::Inbrace;
        if self.lexer.tok == LexTok::String {
            let s = self.lexer.tokstr.as_ref();
            if s.map(|s| s != "in").unwrap_or(true) {
                self.error("expected 'in' in case");
                return None;
            }
        } else if !use_brace {
            self.error("expected 'in' or '{' in case");
            return None;
        }
        self.lexer.zshlex();

        let mut arms = Vec::new();
        const MAX_ARMS: usize = 10_000;

        loop {
            if arms.len() > MAX_ARMS {
                self.error("parse_case: too many arms");
                break;
            }

            // Set incasepat BEFORE skipping separators so lexer knows we're in case pattern context
            // This affects how [ and | are lexed
            self.lexer.incasepat = 1;

            self.skip_separators();

            // Check for end
            // Note: 'esac' might be String "esac" if incasepat > 0 prevents reserved word recognition
            let is_esac = self.lexer.tok == LexTok::Esac
                || (self.lexer.tok == LexTok::String
                    && self
                        .lexer
                        .tokstr
                        .as_ref()
                        .map(|s| s == "esac")
                        .unwrap_or(false));
            if (use_brace && self.lexer.tok == LexTok::Outbrace) || (!use_brace && is_esac) {
                self.lexer.incasepat = 0;
                self.lexer.zshlex();
                break;
            }

            // Also break on EOF
            if self.lexer.tok == LexTok::Endinput || self.lexer.tok == LexTok::Lexerr {
                self.lexer.incasepat = 0;
                break;
            }

            // Skip optional (
            if self.lexer.tok == LexTok::Inpar {
                self.lexer.zshlex();
            }

            // incasepat is already set above
            let mut patterns = Vec::new();
            let mut pattern_iterations = 0;
            loop {
                pattern_iterations += 1;
                if pattern_iterations > 1000 {
                    self.error("parse_case: too many pattern iterations");
                    self.lexer.incasepat = 0;
                    return None;
                }

                if self.lexer.tok == LexTok::String {
                    let s = self.lexer.tokstr.as_ref();
                    if s.map(|s| s == "esac").unwrap_or(false) {
                        break;
                    }
                    patterns.push(self.lexer.tokstr.clone().unwrap_or_default());
                    // After first pattern token, set incasepat=2 so ( is treated as part of pattern
                    self.lexer.incasepat = 2;
                    self.lexer.zshlex();
                } else if self.lexer.tok != LexTok::Bar {
                    break;
                }

                if self.lexer.tok == LexTok::Bar {
                    // Reset to 1 (start of next alternative pattern)
                    self.lexer.incasepat = 1;
                    self.lexer.zshlex();
                } else {
                    break;
                }
            }
            self.lexer.incasepat = 0;

            // Expect )
            if self.lexer.tok != LexTok::Outpar {
                self.error("expected ')' in case pattern");
                return None;
            }
            self.lexer.zshlex();

            // Parse body
            let body = self.parse_program();

            // Get terminator
            let terminator = match self.lexer.tok {
                LexTok::Dsemi => {
                    self.lexer.zshlex();
                    CaseTerm::Break
                }
                LexTok::Semiamp => {
                    self.lexer.zshlex();
                    CaseTerm::Continue
                }
                LexTok::Semibar => {
                    self.lexer.zshlex();
                    CaseTerm::TestNext
                }
                _ => CaseTerm::Break,
            };

            if !patterns.is_empty() {
                arms.push(CaseArm {
                    patterns,
                    body,
                    terminator,
                });
            }
        }

        Some(ZshCommand::Case(ZshCase { word, arms }))
    }

    /// Parse if statement
    fn parse_if(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip 'if'

        // Parse condition - stops at 'then' or '{' (zsh allows { instead of then)
        let cond = Box::new(self.parse_program_until(Some(&[LexTok::Then, LexTok::Inbrace])));

        self.skip_separators();

        // Expect 'then' or {
        let use_brace = self.lexer.tok == LexTok::Inbrace;
        if self.lexer.tok != LexTok::Then && !use_brace {
            self.error("expected 'then' or '{' after if condition");
            return None;
        }
        self.lexer.zshlex();

        // Parse then-body - stops at else/elif/fi, or } if using brace syntax
        let then = if use_brace {
            let body = self.parse_program_until(Some(&[LexTok::Outbrace]));
            if self.lexer.tok == LexTok::Outbrace {
                self.lexer.zshlex();
            }
            Box::new(body)
        } else {
            Box::new(self.parse_program_until(Some(&[LexTok::Else, LexTok::Elif, LexTok::Fi])))
        };

        // Parse elif and else (only for then/fi syntax, not brace syntax)
        let mut elif = Vec::new();
        let mut else_ = None;

        if !use_brace {
            loop {
                self.skip_separators();

                match self.lexer.tok {
                    LexTok::Elif => {
                        self.lexer.zshlex();
                        // elif condition stops at 'then' or '{'
                        let econd =
                            self.parse_program_until(Some(&[LexTok::Then, LexTok::Inbrace]));
                        self.skip_separators();

                        let elif_use_brace = self.lexer.tok == LexTok::Inbrace;
                        if self.lexer.tok != LexTok::Then && !elif_use_brace {
                            self.error("expected 'then' after elif");
                            return None;
                        }
                        self.lexer.zshlex();

                        // elif body stops at else/elif/fi or } if using braces
                        let ebody = if elif_use_brace {
                            let body = self.parse_program_until(Some(&[LexTok::Outbrace]));
                            if self.lexer.tok == LexTok::Outbrace {
                                self.lexer.zshlex();
                            }
                            body
                        } else {
                            self.parse_program_until(Some(&[
                                LexTok::Else,
                                LexTok::Elif,
                                LexTok::Fi,
                            ]))
                        };

                        elif.push((econd, ebody));
                    }
                    LexTok::Else => {
                        self.lexer.zshlex();
                        self.skip_separators();

                        let else_use_brace = self.lexer.tok == LexTok::Inbrace;
                        if else_use_brace {
                            self.lexer.zshlex();
                        }

                        // else body stops at 'fi' or '}'
                        else_ = Some(Box::new(if else_use_brace {
                            let body = self.parse_program_until(Some(&[LexTok::Outbrace]));
                            if self.lexer.tok == LexTok::Outbrace {
                                self.lexer.zshlex();
                            }
                            body
                        } else {
                            self.parse_program_until(Some(&[LexTok::Fi]))
                        }));

                        // Consume the 'fi' if present (not for brace syntax)
                        if !else_use_brace && self.lexer.tok == LexTok::Fi {
                            self.lexer.zshlex();
                        }
                        break;
                    }
                    LexTok::Fi => {
                        self.lexer.zshlex();
                        break;
                    }
                    _ => break,
                }
            }
        }

        Some(ZshCommand::If(ZshIf {
            cond,
            then,
            elif,
            else_,
        }))
    }

    /// Parse while/until loop
    fn parse_while(&mut self, until: bool) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip while/until

        let cond = Box::new(self.parse_program());

        self.skip_separators();
        let body = self.parse_loop_body(false)?;

        Some(ZshCommand::While(ZshWhile {
            cond,
            body: Box::new(body),
            until,
        }))
    }

    /// Parse repeat loop
    fn parse_repeat(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip 'repeat'

        let count = match self.lexer.tok {
            LexTok::String => {
                let c = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.zshlex();
                c
            }
            _ => {
                self.error("expected count after repeat");
                return None;
            }
        };

        self.skip_separators();
        let body = self.parse_loop_body(false)?;

        Some(ZshCommand::Repeat(ZshRepeat {
            count,
            body: Box::new(body),
        }))
    }

    /// Parse loop body (do...done, {...}, or shortloop)
    fn parse_loop_body(&mut self, foreach_style: bool) -> Option<ZshProgram> {
        if self.lexer.tok == LexTok::Doloop {
            self.lexer.zshlex();
            let body = self.parse_program();
            if self.lexer.tok == LexTok::Done {
                self.lexer.zshlex();
            }
            Some(body)
        } else if self.lexer.tok == LexTok::Inbrace {
            self.lexer.zshlex();
            let body = self.parse_program();
            if self.lexer.tok == LexTok::Outbrace {
                self.lexer.zshlex();
            }
            Some(body)
        } else if foreach_style {
            // foreach allows 'end' terminator
            let body = self.parse_program();
            if self.lexer.tok == LexTok::Zend {
                self.lexer.zshlex();
            }
            Some(body)
        } else {
            // Short loop - single command
            match self.parse_list() {
                Some(list) => Some(ZshProgram { lists: vec![list] }),
                None => None,
            }
        }
    }

    /// Parse (...) subshell
    fn parse_subsh(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip (
        let prog = self.parse_program();
        if self.lexer.tok == LexTok::Outpar {
            self.lexer.zshlex();
        }
        Some(ZshCommand::Subsh(Box::new(prog)))
    }

    /// Parse {...} cursh
    fn parse_cursh(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip {
        let prog = self.parse_program();

        // Check for { ... } always { ... }
        if self.lexer.tok == LexTok::Outbrace {
            self.lexer.zshlex();

            // Check for 'always'
            if self.lexer.tok == LexTok::String {
                let s = self.lexer.tokstr.as_ref();
                if s.map(|s| s == "always").unwrap_or(false) {
                    self.lexer.zshlex();
                    self.skip_separators();

                    if self.lexer.tok == LexTok::Inbrace {
                        self.lexer.zshlex();
                        let always = self.parse_program();
                        if self.lexer.tok == LexTok::Outbrace {
                            self.lexer.zshlex();
                        }
                        return Some(ZshCommand::Try(ZshTry {
                            try_block: Box::new(prog),
                            always: Box::new(always),
                        }));
                    }
                }
            }
        }

        Some(ZshCommand::Cursh(Box::new(prog)))
    }

    /// Parse function definition
    fn parse_funcdef(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip 'function'

        let mut names = Vec::new();
        let mut tracing = false;

        // Handle options like -T and function names
        loop {
            match self.lexer.tok {
                LexTok::String => {
                    let s = self.lexer.tokstr.as_ref()?;
                    if s.starts_with('-') {
                        if s.contains('T') {
                            tracing = true;
                        }
                        self.lexer.zshlex();
                        continue;
                    }
                    names.push(s.clone());
                    self.lexer.zshlex();
                }
                LexTok::Inbrace | LexTok::Inoutpar | LexTok::Seper | LexTok::Newlin => break,
                _ => break,
            }
        }

        // Optional ()
        if self.lexer.tok == LexTok::Inoutpar {
            self.lexer.zshlex();
        }

        self.skip_separators();

        // Parse body
        if self.lexer.tok == LexTok::Inbrace {
            self.lexer.zshlex();
            let body = self.parse_program();
            if self.lexer.tok == LexTok::Outbrace {
                self.lexer.zshlex();
            }
            Some(ZshCommand::FuncDef(ZshFuncDef {
                names,
                body: Box::new(body),
                tracing,
            }))
        } else {
            // Short form
            match self.parse_list() {
                Some(list) => Some(ZshCommand::FuncDef(ZshFuncDef {
                    names,
                    body: Box::new(ZshProgram { lists: vec![list] }),
                    tracing,
                })),
                None => None,
            }
        }
    }

    /// Parse inline function definition: name() { ... }
    fn parse_inline_funcdef(&mut self, name: String) -> Option<ZshCommand> {
        // Skip ()
        if self.lexer.tok == LexTok::Inoutpar {
            self.lexer.zshlex();
        }

        self.skip_separators();

        // Parse body
        if self.lexer.tok == LexTok::Inbrace {
            self.lexer.zshlex();
            let body = self.parse_program();
            if self.lexer.tok == LexTok::Outbrace {
                self.lexer.zshlex();
            }
            Some(ZshCommand::FuncDef(ZshFuncDef {
                names: vec![name],
                body: Box::new(body),
                tracing: false,
            }))
        } else {
            match self.parse_cmd() {
                Some(cmd) => {
                    let list = ZshList {
                        sublist: ZshSublist {
                            pipe: ZshPipe {
                                cmd,
                                next: None,
                                lineno: self.lexer.lineno,
                            },
                            next: None,
                            flags: SublistFlags::default(),
                        },
                        flags: ListFlags::default(),
                    };
                    Some(ZshCommand::FuncDef(ZshFuncDef {
                        names: vec![name],
                        body: Box::new(ZshProgram { lists: vec![list] }),
                        tracing: false,
                    }))
                }
                None => None,
            }
        }
    }

    /// Parse [[ ... ]] conditional
    fn parse_cond(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip [[
        let cond = self.parse_cond_expr();

        if self.lexer.tok == LexTok::Doutbrack {
            self.lexer.zshlex();
        }

        cond.map(ZshCommand::Cond)
    }

    /// Parse conditional expression
    fn parse_cond_expr(&mut self) -> Option<ZshCond> {
        self.parse_cond_or()
    }

    fn parse_cond_or(&mut self) -> Option<ZshCond> {
        self.recursion_depth += 1;
        if self.check_recursion() {
            self.error("parse_cond_or: max recursion depth exceeded");
            self.recursion_depth -= 1;
            return None;
        }

        let left = match self.parse_cond_and() {
            Some(l) => l,
            None => {
                self.recursion_depth -= 1;
                return None;
            }
        };

        self.skip_cond_separators();

        let result = if self.lexer.tok == LexTok::Dbar {
            self.lexer.zshlex();
            self.skip_cond_separators();
            match self.parse_cond_or() {
                Some(right) => Some(ZshCond::Or(Box::new(left), Box::new(right))),
                None => None,
            }
        } else {
            Some(left)
        };

        self.recursion_depth -= 1;
        result
    }

    fn parse_cond_and(&mut self) -> Option<ZshCond> {
        self.recursion_depth += 1;
        if self.check_recursion() {
            self.error("parse_cond_and: max recursion depth exceeded");
            self.recursion_depth -= 1;
            return None;
        }

        let left = match self.parse_cond_not() {
            Some(l) => l,
            None => {
                self.recursion_depth -= 1;
                return None;
            }
        };

        self.skip_cond_separators();

        let result = if self.lexer.tok == LexTok::Damper {
            self.lexer.zshlex();
            self.skip_cond_separators();
            match self.parse_cond_and() {
                Some(right) => Some(ZshCond::And(Box::new(left), Box::new(right))),
                None => None,
            }
        } else {
            Some(left)
        };

        self.recursion_depth -= 1;
        result
    }

    fn parse_cond_not(&mut self) -> Option<ZshCond> {
        self.recursion_depth += 1;
        if self.check_recursion() {
            self.error("parse_cond_not: max recursion depth exceeded");
            self.recursion_depth -= 1;
            return None;
        }

        self.skip_cond_separators();

        // ! can be either LexTok::Bang or String "!"
        let is_not = self.lexer.tok == LexTok::Bang
            || (self.lexer.tok == LexTok::String
                && self
                    .lexer
                    .tokstr
                    .as_ref()
                    .map(|s| s == "!")
                    .unwrap_or(false));
        if is_not {
            self.lexer.zshlex();
            let inner = match self.parse_cond_not() {
                Some(i) => i,
                None => {
                    self.recursion_depth -= 1;
                    return None;
                }
            };
            self.recursion_depth -= 1;
            return Some(ZshCond::Not(Box::new(inner)));
        }

        if self.lexer.tok == LexTok::Inpar {
            self.lexer.zshlex();
            self.skip_cond_separators();
            let inner = match self.parse_cond_expr() {
                Some(i) => i,
                None => {
                    self.recursion_depth -= 1;
                    return None;
                }
            };
            self.skip_cond_separators();
            if self.lexer.tok == LexTok::Outpar {
                self.lexer.zshlex();
            }
            self.recursion_depth -= 1;
            return Some(inner);
        }

        let result = self.parse_cond_primary();
        self.recursion_depth -= 1;
        result
    }

    fn parse_cond_primary(&mut self) -> Option<ZshCond> {
        let s1 = match self.lexer.tok {
            LexTok::String => {
                let s = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.zshlex();
                s
            }
            _ => return None,
        };

        self.skip_cond_separators();

        // Check for unary operator
        if s1.starts_with('-') && s1.len() == 2 {
            let s2 = match self.lexer.tok {
                LexTok::String => {
                    let s = self.lexer.tokstr.clone().unwrap_or_default();
                    self.lexer.zshlex();
                    s
                }
                _ => return Some(ZshCond::Unary("-n".to_string(), s1)),
            };
            return Some(ZshCond::Unary(s1, s2));
        }

        // Check for binary operator
        let op = match self.lexer.tok {
            LexTok::String => {
                let s = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.zshlex();
                s
            }
            LexTok::Inang => {
                self.lexer.zshlex();
                "<".to_string()
            }
            LexTok::Outang => {
                self.lexer.zshlex();
                ">".to_string()
            }
            _ => return Some(ZshCond::Unary("-n".to_string(), s1)),
        };

        self.skip_cond_separators();

        let s2 = match self.lexer.tok {
            LexTok::String => {
                let s = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.zshlex();
                s
            }
            _ => return Some(ZshCond::Binary(s1, op, String::new())),
        };

        if op == "=~" {
            Some(ZshCond::Regex(s1, s2))
        } else {
            Some(ZshCond::Binary(s1, op, s2))
        }
    }

    fn skip_cond_separators(&mut self) {
        while self.lexer.tok == LexTok::Seper && {
            let s = self.lexer.tokstr.as_ref();
            s.map(|s| !s.contains(';')).unwrap_or(true)
        } {
            self.lexer.zshlex();
        }
    }

    /// Parse (( ... )) arithmetic command
    fn parse_arith(&mut self) -> Option<ZshCommand> {
        let expr = self.lexer.tokstr.clone().unwrap_or_default();
        self.lexer.zshlex();
        Some(ZshCommand::Arith(expr))
    }

    /// Parse time command
    fn parse_time(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip 'time'

        // Check if there's a pipeline to time
        if self.lexer.tok == LexTok::Seper
            || self.lexer.tok == LexTok::Newlin
            || self.lexer.tok == LexTok::Endinput
        {
            Some(ZshCommand::Time(None))
        } else {
            let sublist = self.parse_sublist();
            Some(ZshCommand::Time(sublist.map(Box::new)))
        }
    }

    /// Check if next token is ()
    fn peek_inoutpar(&mut self) -> bool {
        self.lexer.tok == LexTok::Inoutpar
    }

    /// Skip separator tokens
    fn skip_separators(&mut self) {
        let mut iterations = 0;
        while self.lexer.tok == LexTok::Seper || self.lexer.tok == LexTok::Newlin {
            iterations += 1;
            if iterations > 100_000 {
                self.error("skip_separators: too many iterations");
                return;
            }
            self.lexer.zshlex();
        }
    }

    /// Record an error
    fn error(&mut self, msg: &str) {
        self.errors.push(ParseError {
            message: msg.to_string(),
            line: self.lexer.lineno,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> Result<ZshProgram, Vec<ParseError>> {
        let mut parser = ZshParser::new(input);
        parser.parse()
    }

    #[test]
    fn test_simple_command() {
        let prog = parse("echo hello world").unwrap();
        assert_eq!(prog.lists.len(), 1);
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                assert_eq!(s.words, vec!["echo", "hello", "world"]);
            }
            _ => panic!("expected simple command"),
        }
    }

    #[test]
    fn test_pipeline() {
        let prog = parse("ls | grep foo | wc -l").unwrap();
        assert_eq!(prog.lists.len(), 1);

        let pipe = &prog.lists[0].sublist.pipe;
        assert!(pipe.next.is_some());

        let pipe2 = pipe.next.as_ref().unwrap();
        assert!(pipe2.next.is_some());
    }

    #[test]
    fn test_and_or() {
        let prog = parse("cmd1 && cmd2 || cmd3").unwrap();
        let sublist = &prog.lists[0].sublist;

        assert!(sublist.next.is_some());
        let (op, _) = sublist.next.as_ref().unwrap();
        assert_eq!(*op, SublistOp::And);
    }

    #[test]
    fn test_if_then() {
        let prog = parse("if test -f foo; then echo yes; fi").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::If(_) => {}
            _ => panic!("expected if command"),
        }
    }

    #[test]
    fn test_for_loop() {
        let prog = parse("for i in a b c; do echo $i; done").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::For(f) => {
                assert_eq!(f.var, "i");
                match &f.list {
                    ForList::Words(w) => assert_eq!(w, &vec!["a", "b", "c"]),
                    _ => panic!("expected word list"),
                }
            }
            _ => panic!("expected for command"),
        }
    }

    #[test]
    fn test_case() {
        let prog = parse("case $x in a) echo a;; b) echo b;; esac").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Case(c) => {
                assert_eq!(c.arms.len(), 2);
            }
            _ => panic!("expected case command"),
        }
    }

    #[test]
    fn test_function() {
        // First test just parsing "function foo" to see what happens
        let prog = parse("function foo { }").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::FuncDef(f) => {
                assert_eq!(f.names, vec!["foo"]);
            }
            _ => panic!(
                "expected function, got {:?}",
                prog.lists[0].sublist.pipe.cmd
            ),
        }
    }

    #[test]
    fn test_redirection() {
        let prog = parse("echo hello > file.txt").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                assert_eq!(s.redirs.len(), 1);
                assert_eq!(s.redirs[0].rtype, RedirType::Write);
            }
            _ => panic!("expected simple command"),
        }
    }

    #[test]
    fn test_assignment() {
        let prog = parse("FOO=bar echo $FOO").unwrap();
        match &prog.lists[0].sublist.pipe.cmd {
            ZshCommand::Simple(s) => {
                assert_eq!(s.assigns.len(), 1);
                assert_eq!(s.assigns[0].name, "FOO");
            }
            _ => panic!("expected simple command"),
        }
    }

    #[test]
    fn test_parse_completion_function() {
        let input = r#"_2to3_fixes() {
  local -a fixes
  fixes=( ${${(M)${(f)"$(2to3 --list-fixes 2>/dev/null)"}:#*}//[[:space:]]/} )
  (( ${#fixes} )) && _describe -t fixes 'fix' fixes
}"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse completion function: {:?}",
            result.err()
        );
        let prog = result.unwrap();
        assert!(
            !prog.lists.is_empty(),
            "Expected at least one list in program"
        );
    }

    #[test]
    fn test_parse_array_with_complex_elements() {
        let input = r#"arguments=(
  '(- * :)'{-h,--help}'[show this help message and exit]'
  {-d,--doctests_only}'[fix up doctests only]'
  '*:filename:_files'
)"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse array assignment: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_parse_full_completion_file() {
        let input = r##"#compdef 2to3

# zsh completions for '2to3'

_2to3_fixes() {
  local -a fixes
  fixes=( ${${(M)${(f)"$(2to3 --list-fixes 2>/dev/null)"}:#*}//[[:space:]]/} )
  (( ${#fixes} )) && _describe -t fixes 'fix' fixes
}

local -a arguments

arguments=(
  '(- * :)'{-h,--help}'[show this help message and exit]'
  {-d,--doctests_only}'[fix up doctests only]'
  {-f,--fix}'[each FIX specifies a transformation; default: all]:fix name:_2to3_fixes'
  {-j,--processes}'[run 2to3 concurrently]:number: '
  {-x,--nofix}'[prevent a transformation from being run]:fix name:_2to3_fixes'
  {-l,--list-fixes}'[list available transformations]'
  {-p,--print-function}'[modify the grammar so that print() is a function]'
  {-v,--verbose}'[more verbose logging]'
  '--no-diffs[do not show diffs of the refactoring]'
  {-w,--write}'[write back modified files]'
  {-n,--nobackups}'[do not write backups for modified files]'
  {-o,--output-dir}'[put output files in this directory instead of overwriting]:directory:_directories'
  {-W,--write-unchanged-files}'[also write files even if no changes were required]'
  '--add-suffix[append this string to all output filenames]:suffix: '
  '*:filename:_files'
)

_arguments -s -S $arguments
"##;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse full completion file: {:?}",
            result.err()
        );
        let prog = result.unwrap();
        // Should have parsed successfully with at least one statement
        assert!(!prog.lists.is_empty(), "Expected at least one list");
    }

    #[test]
    fn test_parse_logs_sh() {
        let input = r#"#!/usr/bin/env bash
shopt -s globstar

if [[ $(uname) == Darwin ]]; then
    tail -f /var/log/**/*.log /var/log/**/*.out | lolcat
else
    if [[ $ZPWR_DISTRO_NAME == raspbian ]]; then
        tail -f /var/log/**/*.log | lolcat
    else
        printf "Unsupported...\n" >&2
    fi
fi
"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse logs.sh: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_parse_case_with_glob() {
        let input = r#"case "$ZPWR_OS_TYPE" in
    darwin*)  open_cmd='open'
      ;;
    cygwin*)  open_cmd='cygstart'
      ;;
    linux*)
        open_cmd='xdg-open'
      ;;
esac"#;
        let result = parse(input);
        assert!(
            result.is_ok(),
            "Failed to parse case with glob: {:?}",
            result.err()
        );
    }

    #[test]
    fn test_parse_case_with_nested_if() {
        // Test case with nested if and glob patterns
        let input = r##"function zpwrGetOpenCommand(){
    local open_cmd
    case "$ZPWR_OS_TYPE" in
        darwin*)  open_cmd='open' ;;
        cygwin*)  open_cmd='cygstart' ;;
        linux*)
            if [[ "$_zpwr_uname_r" != *icrosoft* ]];then
                open_cmd='nohup xdg-open'
            fi
            ;;
    esac
}"##;
        let result = parse(input);
        assert!(result.is_ok(), "Failed to parse: {:?}", result.err());
    }

    #[test]
    fn test_parse_zpwr_scripts() {
        use std::fs;
        use std::path::Path;
        use std::sync::mpsc;
        use std::thread;
        use std::time::{Duration, Instant};

        let scripts_dir = Path::new("/Users/wizard/.zpwr/scripts");
        if !scripts_dir.exists() {
            eprintln!("Skipping test: scripts directory not found");
            return;
        }

        let mut total = 0;
        let mut passed = 0;
        let mut failed_files = Vec::new();
        let mut timeout_files = Vec::new();

        for ext in &["sh", "zsh"] {
            let pattern = scripts_dir.join(format!("*.{}", ext));
            if let Ok(entries) = glob::glob(pattern.to_str().unwrap()) {
                for entry in entries.flatten() {
                    total += 1;
                    let file_path = entry.display().to_string();
                    let content = match fs::read_to_string(&entry) {
                        Ok(c) => c,
                        Err(e) => {
                            failed_files.push((file_path, format!("read error: {}", e)));
                            continue;
                        }
                    };

                    // Parse with timeout
                    let content_clone = content.clone();
                    let (tx, rx) = mpsc::channel();
                    let handle = thread::spawn(move || {
                        let result = parse(&content_clone);
                        let _ = tx.send(result);
                    });

                    match rx.recv_timeout(Duration::from_secs(2)) {
                        Ok(Ok(_)) => passed += 1,
                        Ok(Err(errors)) => {
                            let first_err = errors
                                .first()
                                .map(|e| format!("line {}: {}", e.line, e.message))
                                .unwrap_or_default();
                            failed_files.push((file_path, first_err));
                        }
                        Err(_) => {
                            timeout_files.push(file_path);
                            // Thread will be abandoned
                        }
                    }
                }
            }
        }

        eprintln!("\n=== ZPWR Scripts Parse Results ===");
        eprintln!("Passed: {}/{}", passed, total);

        if !timeout_files.is_empty() {
            eprintln!("\nTimeout files (>2s):");
            for file in &timeout_files {
                eprintln!("  {}", file);
            }
        }

        if !failed_files.is_empty() {
            eprintln!("\nFailed files:");
            for (file, err) in &failed_files {
                eprintln!("  {} - {}", file, err);
            }
        }

        // Allow some failures initially, but track progress
        let pass_rate = if total > 0 {
            (passed as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        eprintln!("Pass rate: {:.1}%", pass_rate);

        // Require at least 50% pass rate for now
        assert!(pass_rate >= 50.0, "Pass rate too low: {:.1}%", pass_rate);
    }

    #[test]
    #[ignore] // Uses threads that can't be killed on timeout; use integration test instead
    fn test_parse_zsh_stdlib_functions() {
        use std::fs;
        use std::path::Path;
        use std::sync::mpsc;
        use std::thread;
        use std::time::Duration;

        let functions_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("test_data/zsh_functions");
        if !functions_dir.exists() {
            eprintln!(
                "Skipping test: zsh_functions directory not found at {:?}",
                functions_dir
            );
            return;
        }

        let mut total = 0;
        let mut passed = 0;
        let mut failed_files = Vec::new();
        let mut timeout_files = Vec::new();

        if let Ok(entries) = fs::read_dir(&functions_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }

                total += 1;
                let file_path = path.display().to_string();
                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        failed_files.push((file_path, format!("read error: {}", e)));
                        continue;
                    }
                };

                // Parse with timeout
                let content_clone = content.clone();
                let (tx, rx) = mpsc::channel();
                thread::spawn(move || {
                    let result = parse(&content_clone);
                    let _ = tx.send(result);
                });

                match rx.recv_timeout(Duration::from_secs(2)) {
                    Ok(Ok(_)) => passed += 1,
                    Ok(Err(errors)) => {
                        let first_err = errors
                            .first()
                            .map(|e| format!("line {}: {}", e.line, e.message))
                            .unwrap_or_default();
                        failed_files.push((file_path, first_err));
                    }
                    Err(_) => {
                        timeout_files.push(file_path);
                    }
                }
            }
        }

        eprintln!("\n=== Zsh Stdlib Functions Parse Results ===");
        eprintln!("Passed: {}/{}", passed, total);

        if !timeout_files.is_empty() {
            eprintln!("\nTimeout files (>2s): {}", timeout_files.len());
            for file in timeout_files.iter().take(10) {
                eprintln!("  {}", file);
            }
            if timeout_files.len() > 10 {
                eprintln!("  ... and {} more", timeout_files.len() - 10);
            }
        }

        if !failed_files.is_empty() {
            eprintln!("\nFailed files: {}", failed_files.len());
            for (file, err) in failed_files.iter().take(20) {
                let filename = Path::new(file)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                eprintln!("  {} - {}", filename, err);
            }
            if failed_files.len() > 20 {
                eprintln!("  ... and {} more", failed_files.len() - 20);
            }
        }

        let pass_rate = if total > 0 {
            (passed as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        eprintln!("Pass rate: {:.1}%", pass_rate);

        // Require at least 50% pass rate
        assert!(pass_rate >= 50.0, "Pass rate too low: {:.1}%", pass_rate);
    }
}
