//! Zsh parser - Direct port from zsh/Src/parse.c
//!
//! This parser takes tokens from the ZshLexer and builds an AST.
//! It follows the zsh grammar closely, producing structures that
//! can be executed by the shell executor.

use crate::lexer::ZshLexer;
use crate::tokens::LexTok;
use serde::{Deserialize, Serialize};

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
    /// `|&` between this stage and the next — merge stderr into the
    /// pipe so the next stage's stdin sees both stdout AND stderr from
    /// this stage. When `next` is None this flag is meaningless.
    #[serde(default)]
    pub merge_stderr: bool,
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
    /// Compound command with trailing redirects:
    /// `{ cmd } 2>&1`, `(...) >file`, `if ...; fi >file`, etc.
    /// Simple commands carry redirects in their own struct; this wrapper
    /// is only used for compound forms.
    Redirected(Box<ZshCommand>, Vec<ZshRedir>),
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
    /// Index into ZshLexer.heredocs[] for body lookup. Filled in by
    /// `parse_redirection` for Heredoc/HeredocDash, then resolved into
    /// `heredoc.content` by `fill_heredoc_bodies` after process_heredocs
    /// has run for the line.
    #[serde(skip)]
    pub heredoc_idx: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HereDocInfo {
    pub content: String,
    pub terminator: String,
    /// Originally-quoted terminator (`<<'EOF'`, `<<"EOF"`). When true the
    /// body is passed verbatim — no `$var` / `$(cmd)` / `$((expr))`
    /// expansion. Plain `<<EOF` runs all expansions.
    #[serde(default)]
    pub quoted: bool,
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
    /// True if this was parsed as `select` rather than `for`. Both share
    /// the same parser, so the compiler routes on this flag.
    #[serde(default)]
    pub is_select: bool,
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
    /// Anonymous-function call args. `() { body } a b` parses as a
    /// FuncDef (auto-named) with `auto_call_args = Some(vec!["a", "b"])`.
    /// compile_funcdef registers the function then emits a Simple call
    /// with these args.
    #[serde(default)]
    pub auto_call_args: Option<Vec<String>>,
    /// Original source text of the function body (the bytes between
    /// `{` and `}`, without the braces themselves), captured at parse
    /// time. Populated for `function name { body }` and `function name() { body }`
    /// forms; left None for the synthesized inline-funcdef recovery
    /// path. ZshCompiler::compile_funcdef forwards it to
    /// `BUILTIN_REGISTER_COMPILED_FN` so introspection (`whence`, `which`,
    /// `${functions[name]}`) has canonical source text.
    #[serde(default)]
    pub body_source: Option<String>,
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
    QuoteIfNeeded,         // q+ - quote only if value contains shell-specials
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
    /// Plain text token. Most ZWC-decoded words land here. Goes through
    /// `expand_string` (plus glob/tilde/etc. as text-level transforms) for
    /// final output.
    Literal(String),
    /// Concatenation of sub-words. ZWC array decoding produces this with
    /// child Literals; nothing else constructs it now that the legacy
    /// hand-rolled parser is gone.
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
    Substring(i64, Option<i64>),
    RemovePrefix(ShellWord),
    RemovePrefixLong(ShellWord),
    RemoveSuffix(ShellWord),
    RemoveSuffixLong(ShellWord),
    Replace(ShellWord, ShellWord),
    ReplaceAll(ShellWord, ShellWord),
    Upper,
    Lower,
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

/// Walk every ZshRedir in the program and, for any with a `heredoc_idx`,
/// pull the body+terminator out of `bodies` and stuff into `heredoc`.
/// `bodies[i]` corresponds to the i-th heredoc registered by the lexer
/// during scanning (in source order).
fn fill_heredoc_bodies(prog: &mut ZshProgram, bodies: &[HereDocInfo]) {
    for list in &mut prog.lists {
        fill_in_sublist(&mut list.sublist, bodies);
    }
}

fn fill_in_sublist(sub: &mut ZshSublist, bodies: &[HereDocInfo]) {
    fill_in_pipe(&mut sub.pipe, bodies);
    if let Some(next) = &mut sub.next {
        fill_in_sublist(&mut next.1, bodies);
    }
}

fn fill_in_pipe(pipe: &mut ZshPipe, bodies: &[HereDocInfo]) {
    fill_in_command(&mut pipe.cmd, bodies);
    if let Some(next) = &mut pipe.next {
        fill_in_pipe(next, bodies);
    }
}

fn fill_in_command(cmd: &mut ZshCommand, bodies: &[HereDocInfo]) {
    match cmd {
        ZshCommand::Simple(s) => {
            for r in &mut s.redirs {
                resolve_redir(r, bodies);
            }
        }
        ZshCommand::Subsh(p) | ZshCommand::Cursh(p) => fill_heredoc_bodies(p, bodies),
        ZshCommand::FuncDef(f) => fill_heredoc_bodies(&mut f.body, bodies),
        ZshCommand::If(i) => {
            fill_heredoc_bodies(&mut i.cond, bodies);
            fill_heredoc_bodies(&mut i.then, bodies);
            for (c, b) in &mut i.elif {
                fill_heredoc_bodies(c, bodies);
                fill_heredoc_bodies(b, bodies);
            }
            if let Some(e) = &mut i.else_ {
                fill_heredoc_bodies(e, bodies);
            }
        }
        ZshCommand::While(w) | ZshCommand::Until(w) => {
            fill_heredoc_bodies(&mut w.cond, bodies);
            fill_heredoc_bodies(&mut w.body, bodies);
        }
        ZshCommand::For(f) => fill_heredoc_bodies(&mut f.body, bodies),
        ZshCommand::Case(c) => {
            for arm in &mut c.arms {
                fill_heredoc_bodies(&mut arm.body, bodies);
            }
        }
        ZshCommand::Repeat(r) => fill_heredoc_bodies(&mut r.body, bodies),
        ZshCommand::Time(Some(sublist)) => fill_in_sublist(sublist, bodies),
        ZshCommand::Try(t) => {
            fill_heredoc_bodies(&mut t.try_block, bodies);
            fill_heredoc_bodies(&mut t.always, bodies);
        }
        ZshCommand::Redirected(inner, redirs) => {
            for r in redirs {
                resolve_redir(r, bodies);
            }
            fill_in_command(inner, bodies);
        }
        ZshCommand::Time(None) | ZshCommand::Cond(_) | ZshCommand::Arith(_) => {}
    }
}

fn resolve_redir(r: &mut ZshRedir, bodies: &[HereDocInfo]) {
    if let Some(idx) = r.heredoc_idx {
        if let Some(info) = bodies.get(idx) {
            r.heredoc = Some(info.clone());
        }
    }
}

/// If `list` is a Simple containing one word that ends in the
/// `<INPAR><OUTPAR>` token pair (the lexer-port encoding of `()`),
/// return the bare name. Used by `parse_program_until` to detect
/// `name() {body}` style function definitions where the lexer
/// hasn't split the `()` from the name.
fn simple_name_with_inoutpar(list: &ZshList) -> Option<String> {
    if list.flags.async_ || list.sublist.next.is_some() {
        return None;
    }
    let pipe = &list.sublist.pipe;
    if pipe.next.is_some() {
        return None;
    }
    let simple = match &pipe.cmd {
        ZshCommand::Simple(s) => s,
        _ => return None,
    };
    if simple.words.len() != 1 || !simple.assigns.is_empty() || !simple.redirs.is_empty() {
        return None;
    }
    let w = &simple.words[0];
    let suffix = "\u{88}\u{8a}"; // INPAR + OUTPAR
    if !w.ends_with(suffix) {
        return None;
    }
    let bare = &w[..w.len() - suffix.len()];
    if bare.is_empty() {
        return None;
    }
    Some(crate::lexer::untokenize(bare))
}

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

        let mut program = self.parse_program_until(None);

        if !self.errors.is_empty() {
            return Err(std::mem::take(&mut self.errors));
        }

        // Post-pass: wire heredoc bodies (collected by lexer.process_heredocs)
        // back into ZshRedir.heredoc fields via heredoc_idx.
        let bodies: Vec<HereDocInfo> = self
            .lexer
            .heredocs
            .iter()
            .map(|h| HereDocInfo {
                content: h.content.clone(),
                terminator: h.terminator.clone(),
                quoted: h.quoted,
            })
            .collect();
        if !bodies.is_empty() {
            fill_heredoc_bodies(&mut program, &bodies);
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
                Some(list) => {
                    let detected_name = simple_name_with_inoutpar(&list);
                    lists.push(list);
                    // Synthesize a FuncDef for the `name() { body }` shape
                    // at parse time so body_source is captured while the
                    // lexer still has the input. The lexer port emits
                    // `name(` as a single Word ending in `<INPAR><OUTPAR>`,
                    // so the Simple list is followed by an Inbrace once
                    // separators are skipped.
                    if let Some(name) = detected_name {
                        // Skip separators on the real lexer; safe because
                        // parse_program's next iteration would also skip them.
                        while self.lexer.tok == LexTok::Seper
                            || self.lexer.tok == LexTok::Newlin
                        {
                            self.lexer.zshlex();
                        }
                        if self.lexer.tok == LexTok::Inbrace {
                            // Consume `{`, parse body, capture source.
                            self.lexer.zshlex();
                            let body_start = self.lexer.pos;
                            let body = self.parse_program();
                            let body_end = if self.lexer.tok == LexTok::Outbrace {
                                self.lexer.pos.saturating_sub(1)
                            } else {
                                self.lexer.pos
                            };
                            let body_source = self
                                .lexer
                                .input
                                .get(body_start..body_end)
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty());
                            if self.lexer.tok == LexTok::Outbrace {
                                self.lexer.zshlex();
                            }
                            // Replace the Simple list with a FuncDef list.
                            lists.pop();
                            let funcdef = ZshCommand::FuncDef(ZshFuncDef {
                                names: vec![name],
                                body: Box::new(body),
                                tracing: false,
                                auto_call_args: None,
                                body_source,
                            });
                            let synthetic = ZshList {
                                sublist: ZshSublist {
                                    pipe: ZshPipe {
                                        cmd: funcdef,
                                        next: None,
                                        lineno: self.lexer.lineno,
                                        merge_stderr: false,
                                    },
                                    next: None,
                                    flags: SublistFlags::default(),
                                },
                                flags: ListFlags::default(),
                            };
                            lists.push(synthetic);
                        }
                    }
                }
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
        let mut merge_stderr = false;
        let next = match self.lexer.tok {
            LexTok::Bar | LexTok::Baramp => {
                merge_stderr = self.lexer.tok == LexTok::Baramp;
                self.lexer.zshlex();
                self.skip_separators();
                self.parse_pipe().map(Box::new)
            }
            _ => None,
        };

        self.recursion_depth -= 1;
        Some(ZshPipe { cmd, next, lineno, merge_stderr })
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
            LexTok::Inoutpar => self.parse_anon_funcdef(),
            LexTok::Inbrace => self.parse_cursh(),
            LexTok::Func => self.parse_funcdef(),
            LexTok::Dinbrack => self.parse_cond(),
            LexTok::Dinpar => self.parse_arith(),
            LexTok::Time => self.parse_time(),
            _ => self.parse_simple(redirs),
        };

        // Parse trailing redirections. For Simple commands the redirs were
        // already captured inside parse_simple; for compound forms (Cursh,
        // Subsh, If, While, etc.) we collect them here and wrap in
        // ZshCommand::Redirected so compile_zsh can scope-bracket them.
        if let Some(inner) = cmd {
            let mut trailing: Vec<ZshRedir> = Vec::new();
            while self.lexer.tok.is_redirop() {
                if let Some(redir) = self.parse_redir() {
                    trailing.push(redir);
                }
            }
            if trailing.is_empty() {
                return Some(inner);
            }
            // Simple already absorbed its own redirs (compile path expects
            // them on ZshSimple), so don't double-wrap.
            if matches!(inner, ZshCommand::Simple(_)) {
                if let ZshCommand::Simple(mut s) = inner {
                    s.redirs.extend(trailing);
                    return Some(ZshCommand::Simple(s));
                }
                unreachable!()
            }
            return Some(ZshCommand::Redirected(Box::new(inner), trailing));
        }

        None
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
                    // `{name}>file` named-fd redirect: the lexer doesn't
                    // recognize this shape, so the bare word `{name}`
                    // arrives as a String. If it matches `{IDENT}` and
                    // the NEXT token is a redirop, pop it off as the
                    // varid for that redir.
                    if !words.is_empty() && self.lexer.tok.is_redirop() {
                        let last = words.last().unwrap();
                        let untoked = crate::lexer::untokenize(last);
                        if untoked.starts_with('{')
                            && untoked.ends_with('}')
                            && untoked.len() > 2
                        {
                            let name = &untoked[1..untoked.len() - 1];
                            if !name.is_empty()
                                && name
                                    .chars()
                                    .all(|c| c == '_' || c.is_ascii_alphanumeric())
                                && name
                                    .chars()
                                    .next()
                                    .map(|c| c == '_' || c.is_ascii_alphabetic())
                                    .unwrap_or(false)
                            {
                                let varid = name.to_string();
                                words.pop();
                                if let Some(mut redir) = self.parse_redir() {
                                    redir.varid = Some(varid);
                                    redirs.push(redir);
                                }
                                continue;
                            }
                        }
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

            // The closing OUTPAR is consumed here. The outer parse_simple
            // loop will then `zshlex()` past whatever follows (typically
            // a separator or the next word) — calling zshlex twice in
            // tandem (here AND in parse_simple) over-advances and merges
            // a following `name() { … }` funcdef into the same Simple.
            // We only consume Outpar; let the caller handle the rest.
            // Without this guard `g=(o1); f() { :; }` parsed as one
            // Simple with assigns=[g] and words=["f()"] (one token).
            if self.lexer.tok == LexTok::Outpar {
                // Note: do NOT zshlex() here. parse_simple's `self.lexer
                // .zshlex()` after `parse_assign` returns advances past
                // the Outpar onto the next significant token.
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

        // Heredoc body capture: when reading the terminator above, the
        // lexer pushed a HereDoc to self.lexer.heredocs[]. Record the
        // index so fill_heredoc_bodies() can wire content back after
        // process_heredocs() has run.
        let heredoc_idx = if matches!(rtype, RedirType::Heredoc | RedirType::HeredocDash) {
            if !self.lexer.heredocs.is_empty() {
                Some(self.lexer.heredocs.len() - 1)
            } else {
                None
            }
        } else {
            None
        };

        Some(ZshRedir {
            rtype,
            fd,
            name,
            heredoc: None,
            varid: None,
            heredoc_idx,
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

        // Get list. The lexer-port quirk: `for x (a b c)` arrives as a
        // single String token with the parens lexed-as-content
        // (`<INPAR>a b c<OUTPAR>`) instead of as separate Inpar/String/
        // Outpar tokens. Detect that shape and split it manually.
        let list = if self.lexer.tok == LexTok::String
            && self
                .lexer
                .tokstr
                .as_ref()
                .map(|s| s.starts_with('\u{88}') && s.ends_with('\u{8a}'))
                .unwrap_or(false)
        {
            let raw = self.lexer.tokstr.clone().unwrap_or_default();
            // Strip leading INPAR + trailing OUTPAR, then untokenize the
            // inner content and split on whitespace for the word list.
            let inner = &raw[raw.char_indices().nth(1).map(|(i, _)| i).unwrap_or(0)
                ..raw
                    .char_indices()
                    .last()
                    .map(|(i, _)| i)
                    .unwrap_or(raw.len())];
            let cleaned = crate::lexer::untokenize(inner);
            let words: Vec<String> = cleaned
                .split_whitespace()
                .map(|s| s.to_string())
                .collect();
            self.lexer.zshlex();
            ForList::Words(words)
        } else if self.lexer.tok == LexTok::String {
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
            is_select: false,
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
            is_select: false,
        }))
    }

    /// Parse select loop (same syntax as for)
    fn parse_select(&mut self) -> Option<ZshCommand> {
        // `select` shares parse_for's grammar (var, words, body) but the
        // compile path is different (interactive prompt loop).
        match self.parse_for()? {
            ZshCommand::For(mut f) => {
                f.is_select = true;
                Some(ZshCommand::For(f))
            }
            other => Some(other),
        }
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

    /// `() { body } arg1 arg2 …` — anonymous function. Defines a fresh
    /// function named `_zshrs_anon_N`, invokes it with the args, and the
    /// body runs with positional params set. Implemented as the desugared
    /// pair (FuncDef + Simple call) so the compile path doesn't need new
    /// machinery.
    fn parse_anon_funcdef(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip ()
        self.skip_separators();
        // No `{` after `()` → bare empty subshell shape `()`. Fall back
        // to a Subsh with an empty program so the status is 0 (matches
        // zsh's `()` no-op behavior).
        if self.lexer.tok != LexTok::Inbrace {
            return Some(ZshCommand::Subsh(Box::new(ZshProgram { lists: Vec::new() })));
        }
        self.lexer.zshlex(); // skip {
        let body = self.parse_program();
        if self.lexer.tok == LexTok::Outbrace {
            self.lexer.zshlex();
        }
        // Collect any trailing args until a separator. zsh's anon-fn form
        // `() { body } a b c` runs body with $1=a, $2=b, $3=c.
        let mut args = Vec::new();
        while self.lexer.tok == LexTok::String {
            if let Some(s) = self.lexer.tokstr.clone() {
                args.push(s);
            }
            self.lexer.zshlex();
        }

        // Generate a unique name. Module-level static would be cleaner but
        // a thread-local atomic is enough — anonymous functions are
        // ephemeral and the name isn't user-visible.
        use std::sync::atomic::{AtomicUsize, Ordering};
        static ANON_COUNTER: AtomicUsize = AtomicUsize::new(0);
        let n = ANON_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("_zshrs_anon_{}", n);
        Some(ZshCommand::FuncDef(ZshFuncDef {
            names: vec![name],
            body: Box::new(body),
            tracing: false,
            auto_call_args: Some(args),
            body_source: None,
        }))
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
            let body_start = self.lexer.pos;
            let body = self.parse_program();
            let body_end = if self.lexer.tok == LexTok::Outbrace {
                // Lexer has just consumed `}`; pos is past it. Body content
                // ends one byte before pos.
                self.lexer.pos.saturating_sub(1)
            } else {
                self.lexer.pos
            };
            let body_source = self
                .lexer
                .input
                .get(body_start..body_end)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if self.lexer.tok == LexTok::Outbrace {
                self.lexer.zshlex();
            }
            Some(ZshCommand::FuncDef(ZshFuncDef {
                names,
                body: Box::new(body),
                tracing,
                auto_call_args: None,
                body_source,
            }))
        } else {
            // Short form
            match self.parse_list() {
                Some(list) => Some(ZshCommand::FuncDef(ZshFuncDef {
                    names,
                    body: Box::new(ZshProgram { lists: vec![list] }),
                    tracing,
                    auto_call_args: None,
                    body_source: None,
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
            let body_start = self.lexer.pos;
            let body = self.parse_program();
            let body_end = if self.lexer.tok == LexTok::Outbrace {
                self.lexer.pos.saturating_sub(1)
            } else {
                self.lexer.pos
            };
            let body_source = self
                .lexer
                .input
                .get(body_start..body_end)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            if self.lexer.tok == LexTok::Outbrace {
                self.lexer.zshlex();
            }
            Some(ZshCommand::FuncDef(ZshFuncDef {
                names: vec![name],
                body: Box::new(body),
                tracing: false,
                auto_call_args: None,
                body_source,
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
                                merge_stderr: false,
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
                        auto_call_args: None,
                        body_source: None,
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
