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
    Quote,                 // qq - single-quote always
    QuoteIfNeeded,         // q+ - single-quote only if needed
    DoubleQuote,           // qqq - double-quote
    DollarQuote,           // qqqq - $'...' style
    QuoteBackslash,        // q / b / B - backslash-escape special chars
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
    /// `@` flag — force array-context behavior even inside DQ. zsh's
    /// `"${(@o)arr}"` keeps the sort active and splices each element as
    /// its own word. Without this, the array-only flags became no-ops
    /// in DQ.
    At,
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
    /// `${var/#pat/repl}` — anchored at start (prefix only).
    /// Per Src/subst.c paramsubst's `/`-arm with SUB_START.
    ReplacePrefix(ShellWord, ShellWord),
    /// `${var/%pat/repl}` — anchored at end (suffix only).
    /// Per Src/subst.c paramsubst's `/`-arm with SUB_END.
    ReplaceSuffix(ShellWord, ShellWord),
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

/// Saved parse context. Direct port of zsh's `struct parse_stack`
/// declared in zsh/Src/zsh.h and used by parse.c:295-355
/// (`parse_context_save` / `parse_context_restore`). Pushes per-
/// parse-call state so a nested parse (e.g. inside command
/// substitution) doesn't clobber the outer parse.
///
/// zshrs port note: zsh's parse_stack tracks wordcode-buffer state
/// (ecbuf, eclen, ecused, ecnpats, ecstrs, ecsoffs, ecssub, ecnfunc).
/// zshrs builds AST trees instead so those fields collapse to a
/// recursion_depth + global_iterations save. The lexer-side fields
/// (incmdpos, incond, etc.) live on ZshLexer here so they get saved
/// via the lexer's own `LexStack` rather than being duplicated here.
#[derive(Debug, Default, Clone)]
pub struct ParseStack {
    pub recursion_depth: usize,
    pub global_iterations: usize,
}

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
/// Detect the `name() …` shape inside a Simple. Returns the function
/// name and (when the body was already inlined into the same Simple,
/// e.g. `foo() echo hi`) the rest of the words as the body's argv.
/// Returns None for non-funcdef shapes.
fn simple_name_with_inoutpar(list: &ZshList) -> Option<(Vec<String>, Vec<String>)> {
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
    if simple.words.is_empty() || !simple.assigns.is_empty() {
        return None;
    }
    let suffix = "\u{88}\u{8a}"; // INPAR + OUTPAR
                                 // Find the FIRST word ending in `()`. zsh accepts the
                                 // multi-name shorthand `fna fnb fnc() { body }` (parse.c:
                                 // par_funcdef wordlist) — words[0..i-1] are extra names,
                                 // words[i] is `lastname()`. Words after are the body argv
                                 // (one-line shorthand, `name() cmd args`).
    let par_idx = simple.words.iter().position(|w| w.ends_with(suffix))?;
    let mut names: Vec<String> = Vec::with_capacity(par_idx + 1);
    for w in &simple.words[..par_idx] {
        // Earlier names must be bare identifiers, NOT contain
        // tokens that imply they're not function names (no `()`,
        // no quotes, no expansions). zsh's lexer enforces this
        // at the wordlist level; we approximate by requiring the
        // word be an identifier-shaped token after untokenize.
        let bare = crate::lexer::untokenize(w);
        let valid = !bare.is_empty()
            && bare
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == '$');
        if !valid {
            return None;
        }
        names.push(bare);
    }
    let last = &simple.words[par_idx];
    let bare = &last[..last.len() - suffix.len()];
    if bare.is_empty() {
        return None;
    }
    names.push(crate::lexer::untokenize(bare));
    let rest = simple.words[par_idx + 1..].to_vec();
    Some((names, rest))
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

    /// Save parse context onto a `ParseStack`. Direct port of
    /// zsh/Src/parse.c:295-320 `parse_context_save`. Pushes
    /// recursion_depth + global_iterations and resets to zero so
    /// a nested parse can't trigger the outer parse's limits.
    /// Lexer-side state (incmdpos / incond / etc.) saves via the
    /// lexer's own `LexStack` since those fields live on ZshLexer.
    pub fn parse_context_save(&mut self, ps: &mut ParseStack) {
        // parse.c:299-317 — save parser state. zshrs collapses zsh's
        // wordcode-buffer fields (ecbuf/eclen/ecused/ecnpats/ecstrs/
        // ecsoffs/ecssub/ecnfunc) into the recursion+iteration pair
        // since the AST builder doesn't use a flat wordcode buffer.
        ps.recursion_depth = self.recursion_depth;
        ps.global_iterations = self.global_iterations;
        // parse.c:318-319 — clear the buffer + heredoc list so a
        // nested parse starts from a clean slate.
        self.recursion_depth = 0;
        self.global_iterations = 0;
    }

    /// Restore parse context from a `ParseStack`. Direct port of
    /// zsh/Src/parse.c:326-355 `parse_context_restore`. Inverse of
    /// `parse_context_save`. Also clears any half-built AST state
    /// to prevent leaking into the outer parse.
    pub fn parse_context_restore(&mut self, ps: &ParseStack) {
        // parse.c:330-331 — free any in-progress wordcode buffer.
        // zshrs has no equivalent — AST nodes are owned by their
        // parent so dropping the parser frees them.

        // parse.c:333-352 — restore saved state.
        self.recursion_depth = ps.recursion_depth;
        self.global_iterations = ps.global_iterations;

        // parse.c:354 — `errflag &= ~ERRFLAG_ERROR;` — clear the
        // error flag so the outer parse sees a clean state. zshrs
        // tracks errors per-parser; clearing means dropping any
        // partial errors collected during the nested parse.
        self.errors.clear();
    }

    /// Initialize parser status. Direct port of zsh/Src/parse.c:489-503
    /// `init_parse_status`. Clears the per-parse-call lexer flags
    /// so a fresh parse starts from cmd-position with no nesting
    /// state inherited from a prior parse.
    pub fn init_parse_status(&mut self) {
        // parse.c:500-502 — `incasepat = incond = inredir = infor =
        // intypeset = 0; inrepeat_ = 0; incmdpos = 1;`
        self.lexer.incasepat = 0;
        self.lexer.incond = 0;
        self.lexer.inredir = false;
        self.lexer.infor = 0;
        self.lexer.intypeset = false;
        self.lexer.incmdpos = true;
    }

    /// Initialize parser for a fresh parse. Direct port of
    /// zsh/Src/parse.c:507-525 `init_parse`. C source allocates a
    /// fresh wordcode buffer (ecbuf) sized EC_INIT_SIZE, resets the
    /// per-parse-call counters, and calls init_parse_status. zshrs
    /// has no flat wordcode buffer (AST is built inline) so this
    /// function reduces to init_parse_status + recursion_depth/
    /// global_iterations clear.
    pub fn init_parse(&mut self) {
        // parse.c:513-520 — init wordcode buffer. zshrs no-op.
        self.recursion_depth = 0;
        self.global_iterations = 0;
        // parse.c:522 — `init_parse_status();`
        self.init_parse_status();
    }

    /// Check whether the parsed program is empty. Direct port of
    /// zsh/Src/parse.c:583-587 `empty_eprog`. C version checks
    /// `*p->prog == WCB_END()` (single end-of-wordcode marker).
    /// zshrs version checks the AST node count.
    pub fn empty_eprog(prog: &ZshProgram) -> bool {
        prog.lists.is_empty()
    }

    /// Clear pending here-document list. Direct port of
    /// zsh/Src/parse.c:589-600 `clear_hdocs`. The C version walks
    /// the global `hdocs` linked list and frees each node. zshrs
    /// stores pending heredocs on the lexer's `heredocs` Vec —
    /// truncating it has the same effect.
    pub fn clear_hdocs(&mut self) {
        self.lexer.heredocs.clear();
    }

    /// Top-level parse-event entry. Direct port of zsh/Src/parse.c:
    /// 612-631 `parse_event`. Reads one event from the lexer (a
    /// sublist optionally followed by SEPER/AMPER/AMPERBANG) and
    /// returns the resulting ZshProgram.
    ///
    /// `endtok` is the token that terminates the event — usually
    /// ENDINPUT, but for command-style substitutions the closing
    /// `)` (zsh's CMD_SUBST_CLOSE).
    ///
    /// zshrs port note: zsh's parse_event returns an `Eprog` (heap-
    /// allocated wordcode program). zshrs returns a `ZshProgram`
    /// (AST root). Same role at the parse-output boundary.
    pub fn parse_event(&mut self, endtok: LexTok) -> Option<ZshProgram> {
        // parse.c:616-619 — reset state and prime the lexer.
        self.lexer.tok = LexTok::Endinput;
        self.lexer.incmdpos = true;
        self.lexer.zshlex();
        // parse.c:620 — `init_parse();`
        self.init_parse();

        // parse.c:622-625 — drive par_event; on failure clear hdocs.
        if !self.par_event(endtok) {
            self.clear_hdocs();
            return None;
        }
        // parse.c:626-628 — if endtok != ENDINPUT, this is a sub-
        // parse for a substitution that doesn't need its own eprog.
        // zshrs returns an empty program in that case (caller
        // discards).
        if endtok != LexTok::Endinput {
            return Some(ZshProgram { lists: Vec::new() });
        }
        // parse.c:630 — `bld_eprog(1);` — build the final eprog.
        // zshrs has already built the AST via parse_program_until,
        // but parse_event uses par_event directly so we need to
        // collect what par_event accumulated.
        Some(self.parse_program_until(None))
    }

    /// Parse one event (sublist with optional separator). Direct
    /// port of zsh/Src/parse.c:633-695 `par_event`. Returns true if
    /// an event was successfully parsed, false on EOF / endtok.
    ///
    /// zshrs port note: the C version emits wordcodes via ecadd/
    /// set_list_code; zshrs's parser builds AST nodes via
    /// parse_sublist + parse_list. Same flow, different output.
    pub fn par_event(&mut self, endtok: LexTok) -> bool {
        // parse.c:639-643 — skip leading SEPERs.
        while self.lexer.tok == LexTok::Seper {
            // parse.c:640-641 — at top-level (endtok == ENDINPUT),
            // a SEPER on a fresh line ends the event.
            if self.lexer.isnewlin > 0 && endtok == LexTok::Endinput {
                return false;
            }
            self.lexer.zshlex();
        }
        // parse.c:644-647 — terminate on EOF or matching close-token.
        if self.lexer.tok == LexTok::Endinput {
            return false;
        }
        if self.lexer.tok == endtok {
            return true;
        }
        // parse.c:649-... — drive parse_sublist + handle terminator.
        // zshrs's parse_sublist already builds the AST node directly.
        match self.parse_sublist() {
            Some(_) => {
                // parse.c:651-693 — terminator handling. zshrs's
                // parse_list wraps this; for parse_event we just
                // confirm the sublist parsed.
                true
            }
            None => false,
        }
    }

    /// Parse one list — non-recursing variant. Direct port of
    /// zsh/Src/parse.c:807-817 `par_list1`. Like par_list but
    /// doesn't recurse on the trailing-separator path; used by
    /// callers that only want one statement (e.g. each arm of a
    /// case body).
    pub fn par_list1(&mut self) -> Option<ZshSublist> {
        // parse.c:810-816 — body is a single par_sublist call wrapped
        // in the eu/ecused tracking that zshrs doesn't need (no
        // wordcode buffer).
        self.parse_sublist()
    }

    /// Wire a here-document body onto the redirection token that
    /// requested it. Direct port of zsh/Src/parse.c:2347-2361
    /// `setheredoc`. Called when a heredoc terminator has been
    /// matched and the body is ready to be attached to the redir.
    ///
    /// zshrs port note: zsh's setheredoc patches the wordcode
    /// in-place via `pc[1] = ecstrcode(doc); pc[2] = ecstrcode(term);`.
    /// zshrs threads heredoc bodies through `HereDocInfo` structs
    /// that resolve_redir applies during the post-parse fill_in pass.
    /// This method is the AST-side equivalent: writes back to the
    /// matching redir node by index.
    pub fn setheredoc(
        &mut self,
        _pc: usize,
        _redir_type: i32,
        _doc: &str,
        _term: &str,
        _munged_term: &str,
    ) {
        // zshrs's heredoc resolution happens in fill_in_command /
        // resolve_redir at parser.rs top. This stub exists for API
        // parity with the C signature; live wiring happens via
        // self.lexer.heredocs which the post-parse pass consumes.
    }

    /// Parse a wordlist for `for ... in WORDS;`. Direct port of
    /// zsh/Src/parse.c:2362-2378 `par_wordlist`. Reads STRING tokens
    /// until the next SEPER / SEMI / NEWLIN.
    pub fn par_wordlist(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        // parse.c:2362-2378 — collect STRINGs into the wordlist.
        while self.lexer.tok == LexTok::String {
            if let Some(text) = self.lexer.tokstr.clone() {
                out.push(text);
            }
            self.lexer.zshlex();
        }
        out
    }

    /// Parse a newline-separated wordlist. Direct port of
    /// zsh/Src/parse.c:2379-2398 `par_nl_wordlist`. Like
    /// par_wordlist but tolerates leading/trailing newlines.
    pub fn par_nl_wordlist(&mut self) -> Vec<String> {
        // parse.c:2380-2381 — skip leading newlines.
        while self.lexer.tok == LexTok::Newlin {
            self.lexer.zshlex();
        }
        let out = self.par_wordlist();
        // parse.c:2395-2397 — skip trailing newlines.
        while self.lexer.tok == LexTok::Newlin {
            self.lexer.zshlex();
        }
        out
    }

    /// Get the integer value of the next token in a cond expression.
    /// Direct port of zsh/Src/parse.c:2643-2658 `get_cond_num`.
    /// Used for `[[ N OP M ]]` numeric tests where N/M are integer
    /// literals or variable references.
    pub fn get_cond_num(&mut self) -> Option<i64> {
        if self.lexer.tok != LexTok::String {
            return None;
        }
        let text = self.lexer.tokstr.as_ref()?.clone();
        // parse.c:2647-2655 — parse as integer with optional sign.
        let parsed = text.parse::<i64>().ok()?;
        self.lexer.zshlex();
        Some(parsed)
    }

    /// Emit a parser-level error. Direct port of zsh/Src/parse.c
    /// 2733-2766 `yyerror`. C version fills a per-event error buffer
    /// and sets errflag. zshrs pushes onto self.errors which the
    /// caller drains via parse()'s Result return.
    pub fn yyerror(&mut self, msg: &str) {
        // parse.c:2735-2765 — zsh's yyerror collects the offending
        // token's literal text + line number. zshrs already does
        // this via self.error() with the lexer's toklineno.
        self.error(msg);
    }

    // ============================================================
    // Wordcode emission stubs (parse.c private helpers)
    //
    // The following functions are direct counterparts of zsh's
    // private wordcode-emission helpers in parse.c. zsh uses these
    // to write u32 opcodes into a flat `ecbuf` array; zshrs builds
    // an AST tree and never emits wordcode at the parse layer.
    // The implementations are documented stubs that preserve the
    // function signatures + cite the C source. Real wordcode would
    // be emitted later by compile_zsh.rs walking the AST.
    //
    // Listed for port-surface completeness so every parse.c symbol
    // has a Rust counterpart even when the algorithm is moot in the
    // AST architecture.
    // ============================================================

    /// Patch a list-placeholder wordcode with its actual opcode +
    /// jump distance. Direct port of zsh/Src/parse.c:736-749
    /// `set_list_code`. zsh emits an `ecadd(0)` placeholder before
    /// par_sublist runs, then comes back through set_list_code to
    /// rewrite the slot with WCB_LIST(type, distance) once the
    /// sublist's final length is known.
    ///
    /// zshrs port note: zshrs builds AST nodes inline so there's
    /// no placeholder to patch. The ZshList { sublist, flags }
    /// node is created with the right flags from the start.
    /// Stub provided for port-surface completeness.
    pub fn set_list_code(_p: usize, _type_code: i32, _cmplx: bool) {
        // parse.c:740-748 — wordcode patching. zshrs no-op.
    }

    /// Patch a sublist-placeholder wordcode with its actual opcode.
    /// Direct port of zsh/Src/parse.c:753-763 `set_sublist_code`.
    /// Same role as set_list_code at the sublist level.
    pub fn set_sublist_code(_p: usize, _type_code: i32, _flags: i32, _skip: i32, _cmplx: bool) {
        // parse.c:757-762 — wordcode patching. zshrs no-op.
    }

    /// Add one wordcode opcode to the buffer. Direct port of
    /// zsh/Src/parse.c:396-408 `ecadd`. Returns the index of the
    /// new opcode. zshrs no-op since the AST is built inline.
    pub fn ecadd(_c: u32) -> usize {
        // parse.c:399-407 — append to ecbuf with grow-on-demand.
        // zshrs no-op.
        0
    }

    /// Delete a wordcode at position p. Direct port of
    /// zsh/Src/parse.c:412-421 `ecdel`. zshrs no-op.
    pub fn ecdel(_p: usize) {
        // parse.c:415-420 — memmove + decrement ecused. zshrs no-op.
    }

    /// Encode a string into a wordcode value. Direct port of
    /// zsh/Src/parse.c:425-471 `ecstrcode`. C source packs short
    /// strings (≤4 chars) into a single wordcode + uses a binary
    /// tree (Eccstr) for longer strings; long-string slots are
    /// de-duplicated via hasher + strcmp. zshrs no-op since the
    /// AST stores strings directly.
    pub fn ecstrcode(_s: &str) -> u32 {
        // parse.c:432-470 — the actual encoding logic. zshrs no-op.
        0
    }

    /// Insert N empty wordcode slots at position p. Direct port of
    /// zsh/Src/parse.c:371-388 `ecispace`. Used to reserve space
    /// for a forward-jump opcode that will be patched once the
    /// jump target is known. zshrs no-op since AST jumps are
    /// resolved at compile_zsh time.
    pub fn ecispace(_p: usize, _n: usize) {
        // parse.c:376-387 — grow + memmove + adjust hdocs. zshrs no-op.
    }

    /// Adjust pending heredoc pointers when wordcodes shift. Direct
    /// port of zsh/Src/parse.c:359-367 `ecadjusthere`. Called
    /// internally by ecispace / ecdel after they shift the buffer.
    /// zshrs no-op since heredocs are tracked by index in the
    /// lexer's Vec, not by absolute wordcode offset.
    pub fn ecadjusthere(_p: usize, _d: i32) {
        // parse.c:362-366 — walk hdocs list, bump pc by d. zshrs no-op.
    }

    // ============================================================
    // Eprog runtime ops (parse.c:2767-2853)
    //
    // dupeprog / useeprog / freeeprog are zsh's reference-counting
    // helpers for executable programs. zshrs's AST is owned by
    // value (Rust ownership); cloning is a tree-deep copy via
    // Clone, "use" is a no-op (the executor borrows the AST), and
    // "free" is automatic on drop.
    // ============================================================

    /// Duplicate an Eprog. Direct port of zsh/Src/parse.c:2767-2812
    /// `dupeprog`. C version deep-copies the wordcode array + string
    /// table + pattern progs. zshrs uses Clone on the AST.
    pub fn dupeprog(prog: &ZshProgram) -> ZshProgram {
        prog.clone()
    }

    /// Increment an Eprog's reference count. Direct port of
    /// zsh/Src/parse.c:2813-2822 `useeprog`. zshrs no-op (Rust
    /// ownership).
    pub fn useeprog(_prog: &ZshProgram) {
        // parse.c:2815-2821 — `prog->nref++` if not heap-allocated.
        // zshrs no-op.
    }

    /// Decrement / free an Eprog. Direct port of
    /// zsh/Src/parse.c:2823-2854 `freeeprog`. zshrs no-op (drop on
    /// scope-exit).
    pub fn freeeprog(_prog: ZshProgram) {
        // parse.c:2825-2853 — decrement nref, free if zero. zshrs
        // drops via Rust ownership.
    }

    // ============================================================
    // Wordcode runtime getters (parse.c:2853-3060)
    //
    // These read packed wordcode out of a running Eprog at execution
    // time. zshrs's executor walks the AST directly so these are
    // stubs that preserve the C signatures + cite the source.
    // ============================================================

    /// Read a packed string from the wordcode stream. Direct port of
    /// zsh/Src/parse.c:2853-2887 `ecgetstr`. C version unpacks
    /// 4-char inline strings + indexes into the strs table for
    /// longer ones. zshrs no-op (AST stores strings directly).
    pub fn ecgetstr(_dup: bool) -> String {
        // parse.c:2858-2886 — wordcode unpack logic. zshrs no-op.
        String::new()
    }

    /// Read a packed string without consuming the wordcode pointer.
    /// Direct port of zsh/Src/parse.c:2890-2913 `ecrawstr`. zshrs
    /// no-op.
    pub fn ecrawstr() -> String {
        String::new()
    }

    /// Read a NUL-terminated string array from wordcode. Direct port
    /// of zsh/Src/parse.c:2916-2933 `ecgetarr`. zshrs no-op.
    pub fn ecgetarr(_num: usize, _dup: bool) -> Vec<String> {
        Vec::new()
    }

    /// Read a linked-list of strings from wordcode. Direct port of
    /// zsh/Src/parse.c:2936-2955 `ecgetlist`. zshrs no-op.
    pub fn ecgetlist(_num: usize, _dup: bool) -> Vec<String> {
        Vec::new()
    }

    /// Read a sequence of redirection wordcodes. Direct port of
    /// zsh/Src/parse.c:2958-2991 `ecgetredirs`. zshrs no-op
    /// (redirections live as AST ZshRedir nodes).
    pub fn ecgetredirs() -> Vec<ZshRedir> {
        Vec::new()
    }

    /// Copy consecutive redirection wordcodes into a new Eprog.
    /// Direct port of zsh/Src/parse.c:3001-3060 `eccopyredirs`.
    /// zshrs no-op.
    pub fn eccopyredirs() -> Option<ZshProgram> {
        None
    }

    /// Initialize the dummy Eprog used as a placeholder. Direct port
    /// of zsh/Src/parse.c:3068-3075 `init_eprog`. zshrs no-op since
    /// the AST has no equivalent dummy node — empty programs are
    /// just `ZshProgram { lists: vec![] }`.
    pub fn init_eprog() {
        // parse.c:3071-3074 — set up dummy_eprog_code = WCB_END().
        // zshrs no-op.
    }

    /// Parse the complete input
    pub fn parse(&mut self) -> Result<ZshProgram, Vec<ParseError>> {
        self.lexer.zshlex();

        let mut program = self.parse_program_until(None);

        if !self.errors.is_empty() {
            return Err(std::mem::take(&mut self.errors));
        }
        // Surface lexer-level errors (unmatched quote/heredoc/etc.)
        // that the parser silently rolls past. zsh aborts with a
        // diagnostic in this case; mirror it.
        if let Some(msg) = self.lexer.error.clone() {
            return Err(vec![ParseError {
                message: msg,
                line: 1,
            }]);
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
    /// Parse a complete program (top-level entry). Calls
    /// parse_program_until with no end-token sentinel. Direct port of
    /// zsh/Src/parse.c:614-720 `parse_event` / `parse_list` /
    /// `par_event` flow. C distinguishes COND_EVENT (single command
    /// for here-string) from full event parse; zshrs's parse_program
    /// is the full-event entry.
    fn parse_program(&mut self) -> ZshProgram {
        self.parse_program_until(None)
    }

    /// Parse a program until we hit an end token
    /// Parse a program until one of `end_tokens` is seen (or EOF).
    /// Drives parse_list in a loop. C equivalent: the body of par_event
    /// (parse.c:635-695) iterating par_list against the lexer.
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
                    let detected = simple_name_with_inoutpar(&list);
                    lists.push(list);
                    // Synthesize a FuncDef for the `name() { body }` shape
                    // at parse time so body_source is captured while the
                    // lexer still has the input. The lexer port emits
                    // `name(` as a single Word ending in `<INPAR><OUTPAR>`,
                    // so the Simple list is followed by an Inbrace once
                    // separators are skipped. For `name() cmd args` the
                    // body has already been swallowed into the same
                    // Simple's words tail — synthesize directly from there.
                    if let Some((names, body_argv)) = detected {
                        if !body_argv.is_empty() {
                            // One-line body already in the Simple. Build
                            // a Simple from body_argv as the function body.
                            lists.pop();
                            let body_simple = ZshCommand::Simple(ZshSimple {
                                assigns: Vec::new(),
                                words: body_argv,
                                redirs: Vec::new(),
                            });
                            let body_list = ZshList {
                                sublist: ZshSublist {
                                    pipe: ZshPipe {
                                        cmd: body_simple,
                                        next: None,
                                        lineno: self.lexer.lineno,
                                        merge_stderr: false,
                                    },
                                    next: None,
                                    flags: SublistFlags::default(),
                                },
                                flags: ListFlags::default(),
                            };
                            let funcdef = ZshCommand::FuncDef(ZshFuncDef {
                                names,
                                body: Box::new(ZshProgram {
                                    lists: vec![body_list],
                                }),
                                tracing: false,
                                auto_call_args: None,
                                body_source: None,
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
                            continue;
                        }
                        // Else: words.len() == 1 (only the trailing `name()`
                        // word), brace body follows. `names` may carry
                        // multiple identifiers from the `fna fnb fnc()`
                        // shorthand — all share the same brace body per
                        // src/zsh/Src/parse.c:1666 par_funcdef wordlist.
                        // Skip separators on the real lexer; safe because
                        // parse_program's next iteration would also skip them.
                        while self.lexer.tok == LexTok::Seper || self.lexer.tok == LexTok::Newlin {
                            self.lexer.zshlex();
                        }
                        if self.lexer.tok == LexTok::Inbrace {
                            // Capture body_start BEFORE the lexer
                            // advances past the first body token. The
                            // outer zshlex() consumed `{`; lexer.pos
                            // is now right after `{`. The next
                            // `zshlex()` would advance past `echo`,
                            // making body_start land mid-body and
                            // lose the first word — `typeset -f f`
                            // printed `a; echo b` instead of
                            // `echo a; echo b` for `f() { echo a;
                            // echo b }`.
                            let body_start = self.lexer.pos;
                            self.lexer.zshlex();
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
                                names,
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
                        } else if !matches!(
                            self.lexer.tok,
                            LexTok::Endinput | LexTok::Outbrace | LexTok::Seper | LexTok::Newlin
                        ) {
                            // No-brace one-line body: `foo() echo hello`.
                            // Parse a single command for the body.
                            let body_cmd = self.parse_cmd();
                            if let Some(cmd) = body_cmd {
                                let body_list = ZshList {
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
                                lists.pop();
                                let funcdef = ZshCommand::FuncDef(ZshFuncDef {
                                    names: names.clone(),
                                    body: Box::new(ZshProgram {
                                        lists: vec![body_list],
                                    }),
                                    tracing: false,
                                    auto_call_args: None,
                                    body_source: None,
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
                }
                None => break,
            }
        }

        ZshProgram { lists }
    }

    /// Parse a list (sublist with optional & or ;).
    ///
    /// Direct port of zsh/Src/parse.c:771-804 `par_list` (and the
    /// par_list1 wrapper at parse.c:807-817).
    ///
    /// **Structural divergence**: zsh's parse.c emits flat wordcode
    /// into the `ecbuf` u32 array via `ecadd(0)` (placeholder),
    /// `set_list_code(p, code, complexity)`, `wc_bdata(Z_END)`. zshrs
    /// builds an AST node `ZshList { sublist, flags }` instead. The
    /// async/sync/disown discrimination at parse.c:785-790 maps to
    /// zshrs's `ListFlags { async_, disown }` field — Z_SYNC is the
    /// default (no flags), Z_ASYNC = `&` = `async_=true`, Z_DISOWN +
    /// Z_ASYNC = `&!`/`&|` = both true. Same semantics, different
    /// representation. This divergence is repository-wide: every
    /// `par_*` function emits wordcode in C, every `parse_*` builds
    /// AST in Rust. The compile_zsh module then traverses the AST to
    /// emit fusevm bytecode, which serves the same role as zsh's
    /// wordcode but with a different opcode set and execution model.
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

    /// Parse a sublist (pipelines connected by && or ||).
    ///
    /// Direct port of zsh/Src/parse.c:825-867 `par_sublist` and
    /// par_sublist2 at parse.c:869-892. par_sublist handles the
    /// && / || conjunction and emits WC_SUBLIST opcodes; par_sublist2
    /// handles the leading `!` negation and `coproc` keyword.
    ///
    /// AST mapping: ZshSublist { pipe, conj_chain }, where `conj_chain`
    /// is a Vec<(ConjOp, ZshSublist)> for chained && / ||. C uses
    /// flat wordcode with WC_SUBLIST_AND / WC_SUBLIST_OR markers.
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
    /// Parse a pipeline (cmds joined by `|` / `|&`). Direct port of
    /// zsh/Src/parse.c:894-956 `par_pline`. AST: ZshPipe { cmds: Vec<ZshCommand> }.
    /// C emits WC_PIPE wordcodes per command; same flow.
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
        Some(ZshPipe {
            cmd,
            next,
            lineno,
            merge_stderr,
        })
    }

    /// Parse a command
    /// Parse a command — dispatches by leading token (FOR / CASE /
    /// IF / WHILE / UNTIL / REPEAT / FUNC / DINBRACK / DINPAR /
    /// INPAR subshell / INBRACE current-shell / TIME / NOCORRECT,
    /// else simple). Direct port of zsh/Src/parse.c:958-1085 `par_cmd`.
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
    /// Parse a simple command (assignments + words + redirections).
    /// Direct port of zsh/Src/parse.c:1836-2228 `par_simple` —
    /// the largest single function in parse.c. Handles ENVSTRING/
    /// ENVARRAY assignments at command head, intermixed redirs,
    /// typeset-style multi-assignment commands, and the trailing
    /// inout-par `()` that converts a simple command into an inline
    /// function definition.
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
                        if untoked.starts_with('{') && untoked.ends_with('}') && untoked.len() > 2 {
                            let name = &untoked[1..untoked.len() - 1];
                            if !name.is_empty()
                                && name.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
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
    /// Parse an assignment word `NAME=value` or `NAME=(arr items)`.
    /// Sub-routine of parse_simple. The C source handles assignments
    /// inline in par_simple via the ENVSTRING/ENVARRAY token paths
    /// (parse.c:1842-2000ish); zshrs splits it out to a dedicated
    /// helper for clarity.
    fn parse_assign(&mut self) -> Option<ZshAssign> {
        use crate::tokens::char_tokens;

        let tokstr = self.lexer.tokstr.as_ref()?;

        // Parse name=value or name+=value.
        let (name, value_str, append) = if self.lexer.tok == LexTok::Envarray {
            let (name, append) = if let Some(stripped) = tokstr.strip_suffix('+') {
                (stripped, true)
            } else {
                (tokstr.as_str(), false)
            };
            (name.to_string(), String::new(), append)
        } else if let Some(pos) = tokstr.find(char_tokens::EQUALS) {
            let name_part = &tokstr[..pos];
            let (name, append) = if let Some(stripped) = name_part.strip_suffix('+') {
                (stripped, true)
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
            let (name, append) = if let Some(stripped) = name_part.strip_suffix('+') {
                (stripped, true)
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
                //
                // Force `incmdpos=true` so the next zshlex() recognizes
                // a follow-up `b=(...)` / `b=val` as Envarray/Envstring.
                // The lexer flips incmdpos to false on bare Outpar (which
                // is correct for subshell-close context), but for an
                // array-assignment close more assigns/words may follow.
                self.lexer.incmdpos = true;
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
    /// Parse a redirection (>file, <file, >>file, <<HEREDOC, etc.).
    /// Direct port of zsh/Src/parse.c:2229-2346 `par_redir`. Returns
    /// a ZshRedir node carrying the operator type, fd, target word
    /// (or here-doc body / pipe-redir command), and any `{var}` style
    /// fd-binding parameter.
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
    /// Parse `for NAME in WORDS; do BODY; done` (foreach style) AND
    /// `for ((init; cond; incr)) do BODY done` (c-style). Direct port
    /// of zsh/Src/parse.c:1087-1207 `par_for`. parse_for_cstyle is the
    /// inner branch for the `((...))` arithmetic-header variant
    /// (parse.c:1100-1140 inside par_for).
    fn parse_for(&mut self) -> Option<ZshCommand> {
        let is_foreach = self.lexer.tok == LexTok::Foreach;
        self.lexer.zshlex();

        // Check for C-style: for (( init; cond; step ))
        if self.lexer.tok == LexTok::Dinpar {
            return self.parse_for_cstyle();
        }

        // Get variable name(s). zsh parse.c par_for accepts multiple
        // identifier tokens before `in`/`(`/newline — `for k v in ...`
        // assigns each iteration's pair of values to k and v in turn.
        // We store the names space-joined since variable identifiers
        // can't contain whitespace.
        let mut names: Vec<String> = Vec::new();
        while self.lexer.tok == LexTok::String {
            let v = self.lexer.tokstr.clone().unwrap_or_default();
            if v == "in" {
                break;
            }
            names.push(v);
            self.lexer.zshlex();
        }
        if names.is_empty() {
            self.error("expected variable name in for");
            return None;
        }
        let var = names.join(" ");

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
            let words: Vec<String> = cleaned.split_whitespace().map(|s| s.to_string()).collect();
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
                // After the `)` of a for-list, the next token is the
                // body opener — `do`/`{`. zsh's lexer needs incmdpos
                // set so `{` lexes as Inbrace (not as a literal). C
                // analogue: parse.c::par_for sets `incmdpos = 1`
                // after consuming the OUTPAR before the body parse.
                self.lexer.incmdpos = true;
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
    /// Parse the c-style `for ((init; cond; incr)) do BODY done`.
    /// Inner branch of zsh/Src/parse.c:1100-1140 inside par_for.
    /// Recognized when the token after FOR is DINPAR (the `((`
    /// detected by gettok via dbparens setup).
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
    /// Parse `select NAME in WORDS; do BODY; done`. Same shape as
    /// `for NAME in WORDS; do ...` but with menu-prompt semantics in
    /// the executor. C equivalent: the SELECT case in par_for at
    /// parse.c:1087-1207 (selects share parser flow with foreach).
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
    /// Parse `case WORD in PATTERN) BODY ;; ... esac`. Direct port
    /// of zsh/Src/parse.c:1209-1409 `par_case`. Each case arm is a
    /// (pattern_list, body, terminator) tuple where terminator is
    /// `;;` (default), `;&` (fallthrough), or `;|` (continue testing).
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
        // Set incasepat=1 BEFORE consuming "in" so the next token (which
        // could be a leading `(` of a paren-prefixed pattern like
        // `case foo in (a|b) …`) is lexed as Inpar, not as a glob-token.
        // Without this the `(` got swallowed into a gettokstr('(', false)
        // call and produced a String like "(foo)" — the parser then saw
        // the `)` inside a string instead of as a separate Outpar.
        self.lexer.incasepat = 1;
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

            // Skip optional `(`. zsh's case grammar: `case W in (P)…)`.
            // The leading `(` is paired with a matching `)` that closes
            // the pattern itself; the arm-close `)` follows separately.
            // Track whether we consumed it so we can skip the matching
            // `)` after pattern parsing — otherwise the arm-close would
            // be interpreted as the pattern-close and the actual body
            // would get the leftover `)`.
            let had_leading_paren = self.lexer.tok == LexTok::Inpar;
            if had_leading_paren {
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

            // zsh's `(P)` form (parse.c:1320-1360 hack) treats the entire
            // parenthesized contents as ONE zsh pattern with internal `|`
            // as the literal alternation operator — NOT as multiple
            // case-arm alternatives. Without a leading `(`, the bare
            // `P1|P2)` form splits into multiple alts. Mirror that here:
            // when a leading `(` was consumed, fold the |-separated
            // pieces back into a single pattern string.
            if had_leading_paren && patterns.len() > 1 {
                let joined = patterns.join("|");
                patterns = vec![joined];
            }

            // Expect ).  Also handle the `(P))` wrapped-pattern form:
            // when a leading `(` was consumed, accept an extra `)` —
            // the inner `)` closes the optional-paren wrapper, the
            // outer `)` is the arm-close. zsh accepts BOTH `(P) BODY`
            // (bare pattern, leading-paren is just the opt-marker, the
            // close is arm-close) and `(P)) BODY` (paren-wrapped
            // pattern, then arm-close). The first form is unambiguous
            // when the bare pattern was simple; the second is needed
            // when the body starts with `(`.
            if self.lexer.tok != LexTok::Outpar {
                self.error("expected ')' in case pattern");
                return None;
            }
            // Port of Src/parse.c:1310-1313 — when the case pattern
            // closes with `)`, set `incmdpos = 1` BEFORE consuming
            // the token so the first word of the arm body is lexed
            // in command position. Without this, `case X in X) c1=v ;;`
            // lexes `c1=v` as a plain STRING rather than an assignment
            // word, and exec treats it as a command name (yielding
            // "command not found: c1=v"). Subsequent statements after
            // `;` parse correctly because the `;` separator restores
            // command position; only the FIRST body word was broken.
            self.lexer.incmdpos = true;
            self.lexer.zshlex();
            if had_leading_paren && self.lexer.tok == LexTok::Outpar {
                self.lexer.incmdpos = true;
                self.lexer.zshlex();
            }

            // Parse body
            let body = self.parse_program();

            // Get terminator. Set incasepat=1 BEFORE the zshlex
            // advance so the next token (the next arm's pattern, like
            // `[a-z]`) gets tokenized in pattern context. Without
            // this, a `[`-prefixed pattern after the FIRST arm became
            // Inbrack instead of String and the pattern-loop bailed
            // out with "expected ')' in case pattern".
            let terminator = match self.lexer.tok {
                LexTok::Dsemi => {
                    self.lexer.incasepat = 1;
                    self.lexer.zshlex();
                    CaseTerm::Break
                }
                LexTok::Semiamp => {
                    self.lexer.incasepat = 1;
                    self.lexer.zshlex();
                    CaseTerm::Continue
                }
                LexTok::Semibar => {
                    self.lexer.incasepat = 1;
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
    /// Parse `if COND; then BODY; [elif COND; then BODY;]* [else BODY;] fi`.
    /// Direct port of zsh/Src/parse.c:1411-1519 `par_if`. The C source
    /// emits WC_IF wordcodes per arm; zshrs builds an AST chain of
    /// (cond, then_body) tuples plus an optional else_body.
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

        // Parse elif and else. zsh accepts the SAME elif/else
        // continuations for both classic `then/fi` AND the brace
        // form `{ ... } elif ... { ... } else { ... }`. Direct port
        // of zsh/Src/parse.c:1417-1500 par_if where the elif/else
        // arms are checked AFTER the body close regardless of which
        // delimiter style opened the block. Without this, zinit's
        //   if [[ -z $sel ]] { ... } else { ... }
        // hung the parser — `else` was treated as an external
        // command following the if-statement, which the lexer state
        // mis-classified inside the still-open function body.
        //
        // For brace-form: skip the `fi` consumption at the end of
        // the loop (no `fi` after a brace block), and `else` may
        // arrive after a `}` close. Skip-separators between the
        // body close and the elif/else token.
        let mut elif = Vec::new();
        let mut else_ = None;

        {
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
    /// Parse `while COND; do BODY; done` and `until COND; do BODY; done`.
    /// Direct port of zsh/Src/parse.c:1521-1563 `par_while`. The
    /// `until` variant is the same loop with the condition negated.
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
    /// Parse `repeat N; do BODY; done`. Direct port of
    /// zsh/Src/parse.c:1565-1617 `par_repeat`. The C source supports
    /// the SHORTLOOPS short-form `repeat N CMD` (no do/done) — zshrs's
    /// parser doesn't yet special-case that variant.
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
    /// Parse the `do BODY done` body of a for/while/until/select/
    /// repeat loop. Direct equivalent of zsh's parse.c handling
    /// inside the loop builders — they all consume DOLOOP, parse a
    /// list until DONE, and return the list. The `foreach_style`
    /// flag signals foreach (where short-form `for NAME in WORDS;
    /// CMD` may skip do/done) vs c-style (which always requires
    /// do/done).
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
            self.parse_list()
                .map(|list| ZshProgram { lists: vec![list] })
        }
    }

    /// Parse (...) subshell
    /// Parse a subshell `( ... )`. Direct port of zsh/Src/parse.c:1619-1670
    /// `par_subsh`. Body parses as a normal list; the subshell wrapper
    /// fork-isolates execution in the executor.
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
    /// Parse an anonymous function definition `() { BODY }` followed
    /// by call args. zsh treats `() { echo hi; } a b c` as defining
    /// and immediately calling an anon fn with args a/b/c. C
    /// equivalent: the INOUTPAR shape in par_simple at parse.c:1836+
    /// triggers an anon-funcdef path.
    fn parse_anon_funcdef(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip ()
        self.skip_separators();
        // No `{` after `()` → bare empty subshell shape `()`. Fall back
        // to a Subsh with an empty program so the status is 0 (matches
        // zsh's `()` no-op behavior).
        if self.lexer.tok != LexTok::Inbrace {
            return Some(ZshCommand::Subsh(Box::new(ZshProgram {
                lists: Vec::new(),
            })));
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
    /// Parse a current-shell brace block `{ BODY }`. C source
    /// par_cmd at parse.c:958-1085 handles INBRACE → emit WC_CURSH
    /// and recurses into the list. zshrs's parse_cursh extracts that
    /// arm into a dedicated method.
    fn parse_cursh(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip {
        let prog = self.parse_program();

        // Check for { ... } always { ... }. Direct port of zsh's
        // par_subsh at parse.c:1612-1660 — note the two `incmdpos = 1`
        // forces (parse.c:1632, 1637): after consuming the closing
        // OUTBRACE AND after matching the `always` keyword, the parser
        // explicitly resets command position so the next `{` lexes as
        // INBRACE. Without these resets the lexer's String-clears-cmdpos
        // rule (lex.rs:976-983) leaves the second `{` in word position,
        // turning `always { ... }` into a Simple `{` `echo` … and the
        // try/always pairing is silently lost.
        if self.lexer.tok == LexTok::Outbrace {
            self.lexer.incmdpos = true; // parse.c:1632 incmdpos = !zsh_construct
            self.lexer.zshlex();

            // Check for 'always'
            if self.lexer.tok == LexTok::String {
                let s = self.lexer.tokstr.as_ref();
                if s.map(|s| s == "always").unwrap_or(false) {
                    self.lexer.incmdpos = true; // parse.c:1637 incmdpos = 1
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
    /// Parse `function NAME { BODY }` or `NAME () { BODY }`. Direct
    /// port of zsh/Src/parse.c:1672-1785 `par_funcdef`. zsh handles
    /// the multiple keyword shapes (function FOO, FOO (), function FOO ()),
    /// the optional `[fname1 fname2 ...]` for multi-name function defs,
    /// and the `function FOO () { ... }` traditional/POSIX hybrid form.
    fn parse_funcdef(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip 'function'

        let mut names = Vec::new();
        let mut tracing = false;

        // Handle options like -T and function names. Two subtleties:
        //
        //   1. Flags: zsh's lexer encodes a leading `-` as
        //      `char_tokens::DASH` (\u{9b}) inside the String tokstr.
        //      The previous `s.starts_with('-')` check failed for
        //      `\u{9b}T`, so `function -T NAME { body }` slipped the
        //      `-T` token into `names` and the function got registered
        //      as `T` plus the intended `NAME`.
        //
        //   2. Body opener: zsh's lexer emits the opening `{` as a
        //      String (not LexTok::Inbrace) when it follows the String
        //      NAME — the preceding name token resets incmdpos to
        //      false, and only `{` immediately followed by `}` (the
        //      empty-body case) gets promoted to Inbrace. The funcdef
        //      parser must recognise the bare-`{` String as the body
        //      opener; otherwise `function NAME { body }` falls through
        //      to `_ => break`, no body parses, and the FuncDef never
        //      lands in the AST. This is consistent with C zsh's
        //      par_funcdef which knows it's in funcdef-header context
        //      and accepts the brace either way.
        loop {
            match self.lexer.tok {
                LexTok::String => {
                    let s = self.lexer.tokstr.as_ref()?;
                    if s == "{" {
                        // Funcdef body opener — break, body-parser branch handles it.
                        break;
                    }
                    let first = s.chars().next();
                    if matches!(first, Some('-') | Some('+'))
                        || matches!(first, Some(c) if c == crate::tokens::char_tokens::DASH)
                    {
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
        let saw_paren = self.lexer.tok == LexTok::Inoutpar;
        if saw_paren {
            self.lexer.zshlex();
        }

        self.skip_separators();

        // Body opener: real Inbrace OR a String("{") (the lexer emits
        // the latter after a String NAME — see comment above).
        let body_opener_is_string_brace = self.lexer.tok == LexTok::String
            && self.lexer.tokstr.as_deref() == Some("{");
        if self.lexer.tok == LexTok::Inbrace || body_opener_is_string_brace {
            // Capture body_start BEFORE the lexer advances past the
            // first body token. After the previous zshlex consumed
            // `{`, lexer.pos points just past `{` (which is where the
            // body source starts). The next `zshlex()` would advance
            // past the first token (`echo`), making body_start land
            // mid-body and lose the first word — `typeset -f f` would
            // print `a; echo b` for `{ echo a; echo b }`.
            let body_start = self.lexer.pos;
            self.lexer.zshlex();
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

            // Anonymous form `function () { body } a b c` (with `()`) or
            // `function { body } a b c` (zsh-only shorthand, no `()`). No
            // name was collected. Mirror parse_anon_funcdef: synthesize
            // `_zshrs_anon_N`, collect trailing args, set auto_call_args
            // so compile_funcdef registers + immediately calls the
            // function with the args as positional params.
            if names.is_empty() {
                let mut args = Vec::new();
                while self.lexer.tok == LexTok::String {
                    if let Some(s) = self.lexer.tokstr.clone() {
                        args.push(s);
                    }
                    self.lexer.zshlex();
                }
                use std::sync::atomic::{AtomicUsize, Ordering};
                static ANON_COUNTER: AtomicUsize = AtomicUsize::new(0);
                let n = ANON_COUNTER.fetch_add(1, Ordering::Relaxed);
                let name = format!("_zshrs_anon_kw_{}", n);
                return Some(ZshCommand::FuncDef(ZshFuncDef {
                    names: vec![name],
                    body: Box::new(body),
                    tracing,
                    auto_call_args: Some(args),
                    body_source,
                }));
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
            self.parse_list().map(|list| {
                ZshCommand::FuncDef(ZshFuncDef {
                    names,
                    body: Box::new(ZshProgram { lists: vec![list] }),
                    tracing,
                    auto_call_args: None,
                    body_source: None,
                })
            })
        }
    }

    /// Parse inline function definition: name() { ... }
    /// Parse the inline form `NAME () { BODY }` (POSIX-style funcdef
    /// without the `function` keyword). The name has already been
    /// consumed and pushed by parse_simple before this method fires.
    /// C source: handled inline in par_simple's INOUTPAR-after-name
    /// arm (parse.c:1836-2228).
    fn parse_inline_funcdef(&mut self, name: String) -> Option<ZshCommand> {
        // Skip ()
        if self.lexer.tok == LexTok::Inoutpar {
            self.lexer.zshlex();
        }

        self.skip_separators();

        // Parse body
        if self.lexer.tok == LexTok::Inbrace {
            // Same body_start-before-zshlex fix as parse_funcdef.
            let body_start = self.lexer.pos;
            self.lexer.zshlex();
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
    /// Parse `[[ EXPR ]]` conditional expression. Direct port of
    /// zsh/Src/parse.c:2409-2731 `par_cond` (and helpers par_cond_1,
    /// par_cond_2, par_cond_double, par_cond_triple, par_cond_multi
    /// at parse.c:2434-2731). Expression operators: `||` `&&` `!`
    /// + unary tests (-f, -d, -n, -z, etc.) + binary tests (=, !=,
    ///   <, >, ==, =~, -eq, -ne, -lt, -le, -gt, -ge, -nt, -ot, -ef).
    fn parse_cond(&mut self) -> Option<ZshCommand> {
        self.lexer.zshlex(); // skip [[
                             // Empty cond `[[ ]]` is a parse error in zsh — emit the
                             // diagnostic and return None so the caller produces a
                             // non-zero exit. Without this, `[[ ]]` silently passed and
                             // returned exit 0.
        if self.lexer.tok == LexTok::Doutbrack {
            self.error("parse error near `]]'");
            self.lexer.zshlex();
            return None;
        }
        let cond = self.parse_cond_expr();

        if self.lexer.tok == LexTok::Doutbrack {
            self.lexer.zshlex();
        }

        cond.map(ZshCommand::Cond)
    }

    /// Parse conditional expression
    /// Top of `[[ ]]` cond-expression parsing — entry to recursive
    /// descent (or → and → not → primary). Direct port of zsh's
    /// par_cond_1 at parse.c:2434-2475.
    fn parse_cond_expr(&mut self) -> Option<ZshCond> {
        self.parse_cond_or()
    }

    /// Cond-expression `||` level. C: inside par_cond_1 at
    /// parse.c:2434-2475 (the `cond_or` ladder).
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
            self.parse_cond_or()
                .map(|right| ZshCond::Or(Box::new(left), Box::new(right)))
        } else {
            Some(left)
        };

        self.recursion_depth -= 1;
        result
    }

    /// Cond-expression `&&` level. C: par_cond_2 at parse.c:2476-2625.
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
            self.parse_cond_and()
                .map(|right| ZshCond::And(Box::new(left), Box::new(right)))
        } else {
            Some(left)
        };

        self.recursion_depth -= 1;
        result
    }

    /// Cond-expression `!` negation level. C: handled inside
    /// par_cond_2 at parse.c:2476-2625 via the BANG token check.
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

    /// Cond-expression primary: unary tests (-f, -d, ...), binary
    /// tests (=, !=, <, >, ==, =~, -eq, -ne, ...), and parenthesized
    /// sub-expressions. Direct port of par_cond_double / par_cond_triple
    /// / par_cond_multi at parse.c:2626-2731 (chosen by arg count).
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

        // Check for unary operator. zsh's lexer tokenizes leading `-` as
        // `char_tokens::DASH` (\u{9b}) inside gettokstr (lex.c:1390-1400
        // LX2_DASH — `-` always becomes Dash, untokenized later). Match
        // either form here, and use char-count not byte-count since DASH
        // is 2 UTF-8 bytes (`\xc2\x9b`).
        let s1_chars: Vec<char> = s1.chars().collect();
        if s1_chars.len() == 2 && crate::tokens::is_dash(s1_chars[0]) {
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

        // Check for binary operator. Direct port of zsh/Src/parse.c:2601-2603:
        //   incond++;  /* parentheses do globbing */
        //   do condlex(); while (COND_SEP());
        //   incond--;  /* parentheses do grouping */
        // The bump makes the lexer treat `(` as a literal character inside
        // the RHS word (e.g. `[[ x =~ (foo) ]]`) instead of returning INPAR
        // and splitting the regex into multiple tokens.
        let op = match self.lexer.tok {
            LexTok::String => {
                let s = self.lexer.tokstr.clone().unwrap_or_default();
                self.lexer.incond += 1;
                self.lexer.zshlex();
                self.lexer.incond -= 1;
                s
            }
            LexTok::Inang => {
                self.lexer.incond += 1;
                self.lexer.zshlex();
                self.lexer.incond -= 1;
                "<".to_string()
            }
            LexTok::Outang => {
                self.lexer.incond += 1;
                self.lexer.zshlex();
                self.lexer.incond -= 1;
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
    /// Parse `(( EXPR ))` arithmetic command. C source: parse.c:1810-1834
    /// `par_dinbrack` (despite the name; the function actually handles
    /// DINPAR `(( ))` blocks too).
    fn parse_arith(&mut self) -> Option<ZshCommand> {
        let expr = self.lexer.tokstr.clone().unwrap_or_default();
        self.lexer.zshlex();
        Some(ZshCommand::Arith(expr))
    }

    /// Parse time command
    /// Parse `time CMD` (POSIX time keyword). Direct port of
    /// zsh/Src/parse.c:1787-1808 `par_time`. The `time` keyword
    /// times the execution of the following pipeline / cmd.
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
