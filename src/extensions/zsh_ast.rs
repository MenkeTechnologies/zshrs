//! Zsh AST types — Rust-only, NOT in zsh C.
//!
//! zsh C does NOT have an AST tree. Its parser emits a flat wordcode
//! stream (`Wordcode ecbuf[]`) directly via `par_event` → `par_list` →
//! `par_sublist` → `par_pline` → `par_cmd` → `par_simple` / `par_redir`
//! (Src/parse.c:485-3000). The wordcode is consumed by `execlist` /
//! `execpline` / `execcmd` in `Src/exec.c` via `WC_KIND`/`wc_code`/
//! `wc_data` macros walking `ecbuf`.
//!
//! zshrs built an AST tree as an intermediate step on the way to
//! wordcode. This file holds those Rust-only AST node types.
//! Originally lived in `src/ported/parse.rs` but relocated here for
//! P9e of the PORT_PLAN.md migration to make their non-C-faithful
//! nature explicit.
//!
//! Phase 9c (par_* wordcode emission) + Phase 9d (vm_helper wordcode
//! consumer rewrite) will eventually retire these types entirely —
//! the parser will emit wordcode directly and the executor will read
//! wordcode directly, matching the C pipeline. Until then, the AST
//! tree is the working IR.

pub use crate::extensions::heredoc_ast::HereDocInfo;
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
    pub rtype: i32,
    pub fd: i32,
    pub name: String,
    pub heredoc: Option<HereDocInfo>,
    pub varid: Option<String>, // {var}>file
    /// Index into the lexer-side `HEREDOCS` thread_local for body lookup. Filled in by
    /// `parse_redirection` for Heredoc/HeredocDash, then resolved into
    /// `heredoc.content` by `fill_heredoc_bodies` after process_heredocs
    /// has run for the line.
    #[serde(skip)]
    pub heredoc_idx: Option<usize>,
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

#[cfg(test)]
mod tests {
    use super::*;

    // === Default impls for flag structs (load-bearing for parser init) ===

    #[test]
    fn list_flags_default_all_false() {
        let f = ListFlags::default();
        assert!(!f.async_, "default ListFlags.async_ must be false");
        assert!(!f.disown, "default ListFlags.disown must be false");
    }

    #[test]
    fn sublist_flags_default_all_false() {
        let f = SublistFlags::default();
        assert!(!f.coproc);
        assert!(!f.not);
    }

    // === Serde round-trip: zshrs cache format must survive across versions ===

    #[test]
    fn zsh_program_empty_round_trips() {
        // Empty program is the parse result for an empty input.
        let p = ZshProgram { lists: vec![] };
        let json = serde_json::to_string(&p).expect("serialize");
        let back: ZshProgram = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.lists.len(), 0);
    }

    #[test]
    fn zsh_simple_round_trips_with_assigns_and_redirs() {
        // Simple command: FOO=bar BAZ=qux echo hi >out
        let simple = ZshSimple {
            assigns: vec![
                ZshAssign {
                    name: "FOO".to_string(),
                    value: ZshAssignValue::Scalar("bar".to_string()),
                    append: false,
                },
                ZshAssign {
                    name: "BAZ".to_string(),
                    value: ZshAssignValue::Scalar("qux".to_string()),
                    append: true,
                },
            ],
            words: vec!["echo".to_string(), "hi".to_string()],
            redirs: vec![ZshRedir {
                rtype: 0,
                fd: 1,
                name: "out".to_string(),
                heredoc: None,
                varid: None,
                heredoc_idx: None,
            }],
        };
        let json = serde_json::to_string(&simple).expect("serialize");
        let back: ZshSimple = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.assigns.len(), 2);
        assert_eq!(back.assigns[0].name, "FOO");
        assert!(back.assigns[1].append, "+= flag must round-trip");
        assert_eq!(back.words, vec!["echo", "hi"]);
        assert_eq!(back.redirs.len(), 1);
        assert_eq!(back.redirs[0].name, "out");
    }

    #[test]
    fn assign_value_array_variant_round_trips() {
        let v = ZshAssignValue::Array(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
        let json = serde_json::to_string(&v).expect("serialize");
        let back: ZshAssignValue = serde_json::from_str(&json).expect("deserialize");
        match back {
            ZshAssignValue::Array(items) => assert_eq!(items, vec!["a", "b", "c"]),
            ZshAssignValue::Scalar(s) => panic!("expected Array, got Scalar({s:?})"),
        }
    }

    #[test]
    fn for_list_c_style_round_trips() {
        let fl = ForList::CStyle {
            init: "i=0".to_string(),
            cond: "i<10".to_string(),
            step: "i++".to_string(),
        };
        let json = serde_json::to_string(&fl).expect("serialize");
        let back: ForList = serde_json::from_str(&json).expect("deserialize");
        match back {
            ForList::CStyle { init, cond, step } => {
                assert_eq!(init, "i=0");
                assert_eq!(cond, "i<10");
                assert_eq!(step, "i++");
            }
            _ => panic!("expected CStyle variant"),
        }
    }

    #[test]
    fn for_list_positional_round_trips() {
        // Positional: `for x do ... done` (no in-words clause).
        let fl = ForList::Positional;
        let json = serde_json::to_string(&fl).expect("serialize");
        let back: ForList = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(back, ForList::Positional));
    }

    #[test]
    fn case_terminator_all_variants_round_trip() {
        // ;; / ;& / ;| terminator variants — each must survive serde.
        for t in [CaseTerm::Break, CaseTerm::Continue, CaseTerm::TestNext] {
            let json = serde_json::to_string(&t).expect("serialize");
            let back: CaseTerm = serde_json::from_str(&json).expect("deserialize");
            // CaseTerm is Copy + PartialEq, so equality is meaningful.
            assert_eq!(back, t);
        }
    }

    #[test]
    fn sublist_op_round_trips_both_variants() {
        for op in [SublistOp::And, SublistOp::Or] {
            let json = serde_json::to_string(&op).expect("serialize");
            let back: SublistOp = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, op);
        }
    }

    #[test]
    fn list_op_round_trips_all_variants() {
        for op in [
            ListOp::And,
            ListOp::Or,
            ListOp::Semi,
            ListOp::Amp,
            ListOp::Newline,
        ] {
            let json = serde_json::to_string(&op).expect("serialize");
            let back: ListOp = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, op);
        }
    }

    #[test]
    fn shell_word_concat_round_trips_nested() {
        // Concat is the AST shape used for word-internal sub-expansions —
        // verify nesting survives.
        let w = ShellWord::Concat(vec![
            ShellWord::Literal("foo".to_string()),
            ShellWord::Literal("bar".to_string()),
            ShellWord::Concat(vec![ShellWord::Literal("baz".to_string())]),
        ]);
        let json = serde_json::to_string(&w).expect("serialize");
        let back: ShellWord = serde_json::from_str(&json).expect("deserialize");
        match back {
            ShellWord::Concat(parts) => {
                assert_eq!(parts.len(), 3, "outer concat must preserve element count");
                match &parts[2] {
                    ShellWord::Concat(inner) => assert_eq!(inner.len(), 1),
                    _ => panic!("nested concat lost"),
                }
            }
            _ => panic!("expected Concat top-level"),
        }
    }

    #[test]
    fn zsh_cond_nested_serialization() {
        // [[ -f file && ! -d dir ]] type compound — verify nested
        // And/Not survive a round-trip.
        let c = ZshCond::And(
            Box::new(ZshCond::Unary("-f".to_string(), "file".to_string())),
            Box::new(ZshCond::Not(Box::new(ZshCond::Unary(
                "-d".to_string(),
                "dir".to_string(),
            )))),
        );
        let json = serde_json::to_string(&c).expect("serialize");
        let back: ZshCond = serde_json::from_str(&json).expect("deserialize");
        match back {
            ZshCond::And(lhs, rhs) => {
                assert!(matches!(*lhs, ZshCond::Unary(_, _)));
                assert!(matches!(*rhs, ZshCond::Not(_)));
            }
            _ => panic!("expected And at root"),
        }
    }

    #[test]
    fn redirect_op_all_variants_round_trip() {
        for op in [
            RedirectOp::Write,
            RedirectOp::Append,
            RedirectOp::Read,
            RedirectOp::ReadWrite,
            RedirectOp::Clobber,
            RedirectOp::DupRead,
            RedirectOp::DupWrite,
            RedirectOp::HereDoc,
            RedirectOp::HereString,
            RedirectOp::WriteBoth,
            RedirectOp::AppendBoth,
        ] {
            let json = serde_json::to_string(&op).expect("serialize");
            let back: RedirectOp = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, op);
        }
    }

    #[test]
    fn zsh_funcdef_serde_default_fields() {
        // ZshFuncDef has #[serde(default)] on auto_call_args and
        // body_source — JSON missing those fields must still decode.
        let json = r#"{
            "names": ["myfn"],
            "body": { "lists": [] },
            "tracing": false
        }"#;
        let fd: ZshFuncDef = serde_json::from_str(json).expect("default fields must apply");
        assert_eq!(fd.names, vec!["myfn"]);
        assert!(fd.auto_call_args.is_none());
        assert!(fd.body_source.is_none());
        assert!(!fd.tracing);
    }

    #[test]
    fn zsh_pipe_merge_stderr_default_when_missing() {
        // `merge_stderr` defaults to false via #[serde(default)] —
        // older cache entries lacking the field must still decode.
        let json = r#"{
            "cmd": { "Simple": { "assigns": [], "words": ["x"], "redirs": [] } },
            "next": null,
            "lineno": 1
        }"#;
        let pipe: ZshPipe = serde_json::from_str(json).expect("default must apply");
        assert!(!pipe.merge_stderr);
        assert_eq!(pipe.lineno, 1);
    }
}
