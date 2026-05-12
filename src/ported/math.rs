//! Mathematical expression evaluation for zshrs
//!
//! Direct port from zsh/Src/math.c
//!
//! Supports:
//! - Integer and floating point arithmetic
//! - All C operators (+, -, *, /, %, <<, >>, &, |, ^, etc.)
//! - Zsh ** power operator
//! - Comparison operators (<, >, <=, >=, ==, !=)
//! - Logical operators (&&, ||, !)
//! - Ternary operator (? :)
//! - Assignment operators (=, +=, -=, *=, /=, etc.)
//! - Pre/post increment/decrement (++, --)
//! - Base conversion (`16#FF`, `2#1010`, `[16]FF`)
//! - Special values (Inf, NaN)
//! - Variable references and assignment

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use crate::ported::utils::zerr;
use std::env;
use crate::ported::exec::{
    self,
};

/// Re-export of `MN_INTEGER` (defined in zsh_h.rs as the Src/zsh.h:103 port).
pub use crate::ported::zsh_h::MN_INTEGER;
/// Re-export of `MN_FLOAT` (defined in zsh_h.rs as the Src/zsh.h:104 port).
pub use crate::ported::zsh_h::MN_FLOAT;
/// Re-export of `MN_UNSET` (defined in zsh_h.rs as the Src/zsh.h:105 port).
pub use crate::ported::zsh_h::MN_UNSET;
/// Re-export of `Mnumber` (defined in zsh_h.rs as the Src/zsh.h:95 port).
pub use crate::ported::zsh_h::Mnumber;

/// Math tokens — direct port of Src/math.c:109-162. C uses bare
/// `#define`s; the Rust port mirrors as `pub const` ints so
/// `static int mtok` (math.c:305) can be a plain `i32` and the
/// C precedence/type tables index by the literal numbers.
pub const M_INPAR: i32 = 0;       // c:109  '('
pub const M_OUTPAR: i32 = 1;      // c:110  ')'
pub const NOT: i32 = 2;           // c:111  '!'
pub const COMP: i32 = 3;          // c:112  '~'
pub const POSTPLUS: i32 = 4;      // c:113  x++
pub const POSTMINUS: i32 = 5;     // c:114  x--
pub const UPLUS: i32 = 6;         // c:115  +x
pub const UMINUS: i32 = 7;        // c:116  -x
pub const AND: i32 = 8;           // c:117  &
pub const XOR: i32 = 9;           // c:118  ^
pub const OR: i32 = 10;           // c:119  |
pub const MUL: i32 = 11;          // c:120  *
pub const DIV: i32 = 12;          // c:121  /
pub const MOD: i32 = 13;          // c:122  %
pub const PLUS: i32 = 14;         // c:123  +
pub const MINUS: i32 = 15;        // c:124  -
pub const SHLEFT: i32 = 16;       // c:125  <<
pub const SHRIGHT: i32 = 17;      // c:126  >>
pub const LES: i32 = 18;          // c:127  <
pub const LEQ: i32 = 19;          // c:128  <=
pub const GRE: i32 = 20;          // c:129  >
pub const GEQ: i32 = 21;          // c:130  >=
pub const DEQ: i32 = 22;          // c:131  ==
pub const NEQ: i32 = 23;          // c:132  !=
pub const DAND: i32 = 24;         // c:133  &&
pub const DOR: i32 = 25;          // c:134  ||
pub const DXOR: i32 = 26;         // c:135  ^^
pub const QUEST: i32 = 27;        // c:136  ?
pub const COLON: i32 = 28;        // c:137  :
pub const EQ: i32 = 29;           // c:138  =
pub const PLUSEQ: i32 = 30;       // c:139  +=
pub const MINUSEQ: i32 = 31;      // c:140  -=
pub const MULEQ: i32 = 32;        // c:141  *=
pub const DIVEQ: i32 = 33;        // c:142  /=
pub const MODEQ: i32 = 34;        // c:143  %=
pub const ANDEQ: i32 = 35;        // c:144  &=
pub const XOREQ: i32 = 36;        // c:145  ^=
pub const OREQ: i32 = 37;         // c:146  |=
pub const SHLEFTEQ: i32 = 38;     // c:147  <<=
pub const SHRIGHTEQ: i32 = 39;    // c:148  >>=
pub const DANDEQ: i32 = 40;       // c:149  &&=
pub const DOREQ: i32 = 41;        // c:150  ||=
pub const DXOREQ: i32 = 42;       // c:151  ^^=
pub const COMMA: i32 = 43;        // c:152  ,
pub const EOI: i32 = 44;          // c:153  end of input
pub const PREPLUS: i32 = 45;      // c:154  ++x
pub const PREMINUS: i32 = 46;     // c:155  --x
pub const NUM: i32 = 47;          // c:156  number literal
pub const ID: i32 = 48;           // c:157  identifier
pub const POWER: i32 = 49;        // c:158  **
pub const CID: i32 = 50;          // c:159  #identifier (char value)
pub const POWEREQ: i32 = 51;      // c:160  **=
pub const FUNC: i32 = 52;         // c:161  function call
/// Total token count — Src/math.c:162 `#define TOKCOUNT 53`. The
/// `c_prec`/`z_prec`/`type` arrays are sized by this.
pub const TOKCOUNT: usize = 53;

/// Operator associativity and type flags
const LR: u16 = 0x0000; // left-to-right
const RL: u16 = 0x0001; // right-to-left
const BOOL: u16 = 0x0002; // short-circuit boolean

const OP_A2: u16 = 0x0004; // 2 arguments
const OP_A2IR: u16 = 0x0008; // 2 args, return int
const OP_A2IO: u16 = 0x0010; // 2 args, must be int
const OP_E2: u16 = 0x0020; // 2 args with assignment
const OP_E2IO: u16 = 0x0040; // 2 args assign, must be int
const OP_OP: u16 = 0x0080; // expecting operator position
const OP_OPF: u16 = 0x0100; // followed by operator (after this, next is operator)

/// Zsh precedence table (default)
static Z_PREC: [u8; TOKCOUNT] = [
    1, 137, 2, 2, 2, // InPar OutPar Not Comp PostPlus
    2, 2, 2, 4, 5, // PostMinus UPlus UMinus And Xor
    6, 8, 8, 8, 9, // Or Mul Div Mod Plus
    9, 3, 3, 10, 10, // Minus ShLeft ShRight Les Leq
    10, 10, 11, 11, 12, // Gre Geq Deq Neq DAnd
    13, 13, 14, 15, 16, // DOr DXor Quest Colon Eq
    16, 16, 16, 16, 16, // PlusEq MinusEq MulEq DivEq ModEq
    16, 16, 16, 16, 16, // AndEq XorEq OrEq ShLeftEq ShRightEq
    16, 16, 16, 17, 200, // DAndEq DOrEq DXorEq Comma Eoi
    2, 2, 0, 0, 7, // PrePlus PreMinus Num Id Power
    0, 16, 0, // CId PowerEq Func
];

/// C precedence table (used with C_PRECEDENCES option)
static C_PREC: [u8; TOKCOUNT] = [
    1, 137, 2, 2, 2, 2, 2, 2, 9, 10, 11, 4, 4, 4, 5, 5, 6, 6, 7, 7, 7, 7, 8, 8, 12, 14, 13, 15, 16,
    17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 17, 18, 200, 2, 2, 0, 0, 3, 0, 17, 0,
];

/// Operator type table (matches C math.c type[] array)
static OP_TYPE: [u16; TOKCOUNT] = [
    // InPar, OutPar, Not, Comp, PostPlus
    LR,
    LR | OP_OP | OP_OPF,
    RL,
    RL,
    RL | OP_OP | OP_OPF,
    // PostMinus, UPlus, UMinus, And, Xor
    RL | OP_OP | OP_OPF,
    RL,
    RL,
    LR | OP_A2IO,
    LR | OP_A2IO,
    // Or, Mul, Div, Mod, Plus
    LR | OP_A2IO,
    LR | OP_A2,
    LR | OP_A2,
    LR | OP_A2,
    LR | OP_A2,
    // Minus, ShLeft, ShRight, Les, Leq
    LR | OP_A2,
    LR | OP_A2IO,
    LR | OP_A2IO,
    LR | OP_A2IR,
    LR | OP_A2IR,
    // Gre, Geq, Deq, Neq, DAnd
    LR | OP_A2IR,
    LR | OP_A2IR,
    LR | OP_A2IR,
    LR | OP_A2IR,
    BOOL | OP_A2IO,
    // DOr, DXor, Quest, Colon, Eq
    BOOL | OP_A2IO,
    LR | OP_A2IO,
    RL | OP_OP,
    RL | OP_OP,
    RL | OP_E2,
    // PlusEq, MinusEq, MulEq, DivEq, ModEq
    RL | OP_E2,
    RL | OP_E2,
    RL | OP_E2,
    RL | OP_E2,
    RL | OP_E2,
    // AndEq, XorEq, OrEq, ShLeftEq, ShRightEq
    RL | OP_E2IO,
    RL | OP_E2IO,
    RL | OP_E2IO,
    RL | OP_E2IO,
    RL | OP_E2IO,
    // DAndEq, DOrEq, DXorEq, Comma, Eoi
    BOOL | OP_E2IO,
    BOOL | OP_E2IO,
    RL | OP_A2IO,
    RL | OP_A2,
    RL | OP_OP,
    // PrePlus, PreMinus, Num, Id, Power
    RL,
    RL,
    LR | OP_OPF,
    LR | OP_OPF,
    RL | OP_A2,
    // CId, PowerEq, Func
    LR | OP_OPF,
    RL | OP_E2,
    LR | OP_OPF,
];

/// Port of `enum prec_type` from `Src/math.c`. `mathevall()` (line
/// 367) uses this to differentiate top-level expression evaluation
/// (`(())`, `$(())`) from function-argument evaluation
/// (`func(arg, arg, …)`) — argument-mode terminates parsing on
/// the first comma encountered at the top level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum prec_type {
    MPREC_TOP,
    MPREC_ARG,
}

// ============================================================
// Module-level math statics — direct port of Src/math.c globals.
//
// math.c declares each of these at file scope:
//   int noeval;                         // line 40
//   mnumber zero_mnumber;               // line 45
//   mnumber lastmathval;                // line 53
//   int lastbase;                       // line 58
//   static char *ptr;                   // line 60
//   static mnumber yyval;               // line 62
//   static char *yylval;                // line 63
//   static int mlevel = 0;              // line 67
//   static int unary = 1;               // line 71
//   static struct mathvalue *stack;     // (math.c body)
//   ... and a few derived from option flags (force_float, etc.).
//
// Rust port: thread_local!<Cell|RefCell<T>> per global. `mathevall`
// (math.c:367) saves these to its own locals (`xyyval`, `xyylval`,
// `xunary`, etc.) on entry and restores on exit so recursive math
// calls (function-arg eval, indirect string eval) don't clobber
// the outer evaluator's state.
//
// Cell for Copy types (i64/i32/usize/bool/Mnumber/i32/&'static
// slice). RefCell for owned/non-Copy (String, Vec, HashMap, Option).
// ============================================================

thread_local! {
    /// `static char *ptr` — current input cursor. Owned String in Rust
    /// (vs C's caller-owned char*) so the thread_local isn't a borrow.
    static M_INPUT: RefCell<String> = const { RefCell::new(String::new()) };
    /// Byte offset into `M_INPUT` of the next char to lex.
    static M_POS: Cell<usize> = const { Cell::new(0) };
    /// Byte offset where the current token began (post-whitespace).
    /// Used to format zsh-style "at `<remaining>'" error pointers.
    static M_TOK_START: Cell<usize> = const { Cell::new(0) };
    /// `static mnumber yyval` (math.c:62) — value lexed by zzlex.
    static M_YYVAL: Cell<Mnumber> = const { Cell::new(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }) };
    /// `static char *yylval` (math.c:63) — identifier or function-call
    /// text lexed by zzlex (caller side reads via `M_YYLVAL.with(...)`).
    static M_YYLVAL: RefCell<String> = const { RefCell::new(String::new()) };
    /// `static struct mathvalue *stack` — operand stack for the
    /// shunting-yard evaluator. Mirrors C's heap-grown array.
    static M_STACK: RefCell<Vec<MathValue>> = const { RefCell::new(Vec::new()) };
    /// `int mtok` — current token tag set by zzlex.
    static M_MTOK: Cell<i32> = const { Cell::new(EOI) };
    /// `static int unary` (math.c:71) — 1 when the parser is expecting
    /// an operand (so `+`/`-` mean unary plus/minus).
    static M_UNARY: Cell<bool> = const { Cell::new(true) };
    // nonzero means we are not evaluating, just parsing                    // c:37
    /// `int noeval` (math.c:40) — non-zero when in the parse-only side
    /// of `&&`/`||`/ternary; suppresses side-effects.
    static M_NOEVAL: Cell<i32> = const { Cell::new(0) };                    // c:40
    // last input base we used                                              // c:55
    /// `int lastbase` (math.c:58) — base of the last numeric literal
    /// (set by lexconstant, used by `$((…))` formatting).
    static M_LASTBASE: Cell<i32> = const { Cell::new(-1) };                 // c:58
    /// `int *prec` — active precedence table (Z_PREC or C_PREC).
    static M_PREC: Cell<&'static [u8; TOKCOUNT]> = const { Cell::new(&Z_PREC) };
    /// `setopt CPRECEDENCES` mirror.
    static M_C_PRECEDENCES: Cell<bool> = const { Cell::new(false) };
    /// `setopt FORCEFLOAT` mirror.
    static M_FORCE_FLOAT: Cell<bool> = const { Cell::new(false) };
    /// `setopt OCTALZEROES` mirror.
    static M_OCTAL_ZEROES: Cell<bool> = const { Cell::new(false) };
    /// In-memory params table (zshrs uses this instead of the C param
    /// table). Carries float/integer Mnumber results.
    static M_VARIABLES: RefCell<HashMap<String, Mnumber>> = RefCell::new(HashMap::new());
    /// Raw string values for variables whose contents aren't a plain
    /// number — recursively re-eval'd by `getmathparam` for
    /// `a="3+2"; $((a))` semantics.
    static M_STRING_VARIABLES: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
    /// `$?` — last command exit status, used by the `?` token in
    /// unary position.
    static M_LASTVAL: Cell<i32> = const { Cell::new(0) };
    /// `$$` — current process ID, lexed for the `$` token.
    static M_PID: Cell<i64> = const { Cell::new(0) };
    /// Error message accumulator. zsh C uses `setjmp`/`longjmp`; the
    /// Rust port returns errors via this Option then `mathevall`
    /// surfaces it as `Result::Err`.
    static M_ERROR: RefCell<Option<String>> = const { RefCell::new(None) };
}

// ============================================================
// WARNING: NOT IN MATH.C — every `m_*` fn below is a Rust-only
// thread_local accessor. C dereferences the corresponding module
// global directly (`yyval.u.l`, `*ptr++`, etc.) without an
// fn-shaped wrapper. The wrappers exist solely because Rust's
// `thread_local!` requires `.with(|c| ...)` for any access, and
// scattering 600 such closures throughout the evaluator would be
// unreadable. Allowlisted in tests/data/ported_fn_allowlist.txt.
// ============================================================
// Accessor helpers — each thread_local reads/writes via these so the
// migration from `s.X` → free-fn-only access is mechanical.

#[inline] fn m_input_clone() -> String { M_INPUT.with(|c| c.borrow().clone()) }
#[inline] fn m_input_set(v: String) { M_INPUT.with(|c| *c.borrow_mut() = v) }
#[inline] fn m_input_len() -> usize { M_INPUT.with(|c| c.borrow().len()) }
#[inline] fn m_input_byte(i: usize) -> u8 {
    M_INPUT.with(|c| c.borrow().as_bytes().get(i).copied().unwrap_or(0))
}
#[inline] fn m_input_slice_from(start: usize) -> String {
    M_INPUT.with(|c| c.borrow()[start..].to_string())
}
#[inline] fn m_input_slice(start: usize, end: usize) -> String {
    M_INPUT.with(|c| c.borrow()[start..end].to_string())
}

#[inline] fn m_pos() -> usize { M_POS.with(|c| c.get()) }
#[inline] fn m_pos_set(v: usize) { M_POS.with(|c| c.set(v)) }
#[inline] fn m_pos_sub(n: usize) { M_POS.with(|c| c.set(c.get() - n)) }
#[inline] fn m_pos_add(n: usize) { M_POS.with(|c| c.set(c.get() + n)) }

#[inline] fn m_tok_start() -> usize { M_TOK_START.with(|c| c.get()) }
#[inline] fn m_tok_start_set(v: usize) { M_TOK_START.with(|c| c.set(v)) }

#[inline] fn m_yyval() -> Mnumber { M_YYVAL.with(|c| c.get()) }
#[inline] fn m_yyval_set(v: Mnumber) { M_YYVAL.with(|c| c.set(v)) }

#[inline] fn m_yylval_clone() -> String { M_YYLVAL.with(|c| c.borrow().clone()) }
#[inline] fn m_yylval_set(v: String) { M_YYLVAL.with(|c| *c.borrow_mut() = v) }

#[inline] fn m_mtok() -> i32 { M_MTOK.with(|c| c.get()) }
#[inline] fn m_mtok_set(t: i32) { M_MTOK.with(|c| c.set(t)) }

#[inline] fn m_unary() -> bool { M_UNARY.with(|c| c.get()) }
#[inline] fn m_unary_set(v: bool) { M_UNARY.with(|c| c.set(v)) }

#[inline] fn m_noeval() -> i32 { M_NOEVAL.with(|c| c.get()) }
#[inline] fn m_noeval_set(v: i32) { M_NOEVAL.with(|c| c.set(v)) }
#[inline] fn m_noeval_inc() { M_NOEVAL.with(|c| c.set(c.get() + 1)) }
#[inline] fn m_noeval_dec() { M_NOEVAL.with(|c| c.set(c.get() - 1)) }

#[inline] fn m_lastbase_set(v: i32) { M_LASTBASE.with(|c| c.set(v)) }

/// Public getter for `lastbase` — used by `assignstrvalue` in
/// params.rs to inherit the input numeric base when a freshly
/// assigned integer parameter has none of its own.
pub fn lastbase() -> i32 { M_LASTBASE.with(|c| c.get()) }

#[inline] fn m_prec() -> &'static [u8; TOKCOUNT] { M_PREC.with(|c| c.get()) }
#[inline] fn m_prec_set(p: &'static [u8; TOKCOUNT]) { M_PREC.with(|c| c.set(p)) }

#[inline] fn m_c_precedences() -> bool { M_C_PRECEDENCES.with(|c| c.get()) }
#[inline] fn m_c_precedences_set(v: bool) { M_C_PRECEDENCES.with(|c| c.set(v)) }
#[inline] fn m_force_float() -> bool { M_FORCE_FLOAT.with(|c| c.get()) }
#[inline] fn m_force_float_set(v: bool) { M_FORCE_FLOAT.with(|c| c.set(v)) }
#[inline] fn m_octal_zeroes() -> bool { M_OCTAL_ZEROES.with(|c| c.get()) }
#[inline] fn m_octal_zeroes_set(v: bool) { M_OCTAL_ZEROES.with(|c| c.set(v)) }

#[inline] fn m_lastval_set(v: i32) { M_LASTVAL.with(|c| c.set(v)) }
#[inline] fn m_lastval() -> i32 { M_LASTVAL.with(|c| c.get()) }
#[inline] fn m_pid() -> i64 { M_PID.with(|c| c.get()) }
#[inline] fn m_pid_set(v: i64) { M_PID.with(|c| c.set(v)) }

#[inline] fn m_error_take() -> Option<String> { M_ERROR.with(|c| c.borrow_mut().take()) }
#[inline] fn m_error_some() -> bool { M_ERROR.with(|c| c.borrow().is_some()) }
#[inline] fn m_error_set(msg: String) {
    M_ERROR.with(|c| {
        if c.borrow().is_none() {
            *c.borrow_mut() = Some(msg);
        }
    })
}
#[inline] fn m_error_set_force(msg: String) {
    M_ERROR.with(|c| *c.borrow_mut() = Some(msg))
}
#[inline] fn m_error_clear() { M_ERROR.with(|c| *c.borrow_mut() = None) }

// Stack helpers — mathvalue stack operations.
#[inline] fn m_stack_push(v: MathValue) { M_STACK.with(|c| c.borrow_mut().push(v)) }
#[inline] fn m_stack_pop() -> Option<MathValue> { M_STACK.with(|c| c.borrow_mut().pop()) }
#[inline] fn m_stack_len() -> usize { M_STACK.with(|c| c.borrow().len()) }
#[inline] fn m_stack_is_empty() -> bool { M_STACK.with(|c| c.borrow().is_empty()) }
#[inline] fn m_stack_top_clone() -> Option<MathValue> { M_STACK.with(|c| c.borrow().last().cloned()) }

// Variable map helpers.
#[inline] fn m_variables_get(name: &str) -> Option<Mnumber> {
    M_VARIABLES.with(|c| c.borrow().get(name).copied())
}
#[inline] fn m_variables_insert(k: String, v: Mnumber) {
    M_VARIABLES.with(|c| { c.borrow_mut().insert(k, v); })
}
#[inline] fn m_variables_clone() -> HashMap<String, Mnumber> {
    M_VARIABLES.with(|c| c.borrow().clone())
}
#[inline] fn m_variables_set(map: HashMap<String, Mnumber>) {
    M_VARIABLES.with(|c| *c.borrow_mut() = map)
}

#[inline] fn m_string_variables_get(name: &str) -> Option<String> {
    M_STRING_VARIABLES.with(|c| c.borrow().get(name).cloned())
}
#[inline] fn m_string_variables_remove(name: &str) {
    M_STRING_VARIABLES.with(|c| { c.borrow_mut().remove(name); })
}
#[inline] fn m_string_variables_clone() -> HashMap<String, String> {
    M_STRING_VARIABLES.with(|c| c.borrow().clone())
}
#[inline] fn m_string_variables_set(map: HashMap<String, String>) {
    M_STRING_VARIABLES.with(|c| *c.borrow_mut() = map)
}
#[inline] fn m_string_variables_insert(k: String, v: String) {
    M_STRING_VARIABLES.with(|c| { c.borrow_mut().insert(k, v); })
}

/// Save/restore container — mirrors C `mathevall()` (Src/math.c:367)'s
/// stack locals (`xyyval`, `xyylval`, `xunary`, `xnoeval`, `xptr`,
/// etc.). Wrap recursive math eval (`callmathfunc` arg parsing,
/// `getmathparam` indirect-string eval) with `save_state()` /
/// `restore_state()` so the parent's evaluator state survives the
/// inner call's thread_local mutations.
#[allow(non_camel_case_types)]
struct xyy_locals {
    input: String,
    pos: usize,
    tok_start: usize,
    yyval: Mnumber,
    yylval: String,
    stack: Vec<MathValue>,
    mtok: i32,
    unary: bool,
    noeval: i32,
    error: Option<String>,
    variables: HashMap<String, Mnumber>,
    string_variables: HashMap<String, String>,
    prec: &'static [u8; TOKCOUNT],
    c_precedences: bool,
    force_float: bool,
    octal_zeroes: bool,
    lastbase: i32,
}

// WARNING: NOT IN MATH.C — Rust-only helper. C inlines the
// xyy* save/restore directly inside `mathevall()`'s body
// (math.c:367 onward); the Rust port factors it out because two
// callsites (callmathfunc arg parsing, getmathparam indirect-string
// eval) would each duplicate ~17 lines of save/restore code.
fn save_state() -> xyy_locals {
    xyy_locals {
        input: m_input_clone(),
        pos: m_pos(),
        tok_start: m_tok_start(),
        yyval: m_yyval(),
        yylval: m_yylval_clone(),
        stack: M_STACK.with(|c| c.borrow().clone()),
        mtok: m_mtok(),
        unary: m_unary(),
        noeval: m_noeval(),
        error: M_ERROR.with(|c| c.borrow().clone()),
        variables: m_variables_clone(),
        string_variables: m_string_variables_clone(),
        prec: m_prec(),
        c_precedences: m_c_precedences(),
        force_float: m_force_float(),
        octal_zeroes: m_octal_zeroes(),
        lastbase: M_LASTBASE.with(|c| c.get()),
    }
}

// WARNING: NOT IN MATH.C — Rust-only helper. See save_state above.
fn restore_state(saved: xyy_locals) {
    m_input_set(saved.input);
    m_pos_set(saved.pos);
    m_tok_start_set(saved.tok_start);
    m_yyval_set(saved.yyval);
    m_yylval_set(saved.yylval);
    M_STACK.with(|c| *c.borrow_mut() = saved.stack);
    m_mtok_set(saved.mtok);
    m_unary_set(saved.unary);
    m_noeval_set(saved.noeval);
    M_ERROR.with(|c| *c.borrow_mut() = saved.error);
    m_variables_set(saved.variables);
    m_string_variables_set(saved.string_variables);
    m_prec_set(saved.prec);
    m_c_precedences_set(saved.c_precedences);
    m_force_float_set(saved.force_float);
    m_octal_zeroes_set(saved.octal_zeroes);
    M_LASTBASE.with(|c| c.set(saved.lastbase));
}

/// Port of `struct mathvalue` from `Src/math.c`:
///
/// ```c
/// struct mathvalue {
///     char *lval;     /* lvalue string for variable write-back  */
///     Value pval;     /* resolved variable handle (or NULL)     */
///     mnumber val;    /* current numeric value                  */
/// };
/// ```
#[derive(Clone)]
pub(crate) struct MathValue {
    pub val: Mnumber,
    pub lval: Option<String>,
    /// `Value pval` slot from the C source. zsh uses it to cache the
    /// resolved parameter handle so write-back doesn't re-parse the
    /// `lval` string. Rust port leaves this as `()` for now — the
    /// resolved variable lives in `crate::ported::exec::ShellExecutor`'s
    /// `variables` map, looked up by `lval` on each access.
    pub pval: (),
}

impl Default for MathValue {
    fn default() -> Self {
        MathValue {
            val: Mnumber { l: 0, d: 0.0, type_: MN_INTEGER },
            lval: None,
            pval: (),
        }
    }
}

// MathState struct DELETED — state now lives in M_* thread_locals
// (matching C math.c's module statics + mathevall's xyy* save/restore).

// WARNING: NOT IN MATH.C — Rust-only initializer. C `mathevall()`
// (math.c:367) takes the input as a parameter and seeds the module
// statics inline at function entry; Rust port factors that seeding
// out so call sites can chain `with_*` setters before invoking
// `mathevall()`.
/// Initialize thread_local math state from a fresh input string.
/// Mirrors the entry-side state setup in C `mathevall()` (math.c:367).
pub(crate) fn new(input: &str) {
    m_input_set(input.to_string());
    m_pos_set(0);
    m_tok_start_set(0);
    m_yyval_set(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER });
    m_yylval_set(String::new());
    M_STACK.with(|c| { c.borrow_mut().clear(); });
    m_mtok_set(EOI);
    m_unary_set(true);
    m_noeval_set(0);
    m_lastbase_set(-1);
    m_prec_set(&Z_PREC);
    m_c_precedences_set(false);
    m_force_float_set(false);
    m_octal_zeroes_set(false);
    m_variables_set(HashMap::new());
    m_string_variables_set(HashMap::new());
    m_lastval_set(0);
    m_pid_set(std::process::id() as i64);
    m_error_clear();
}

// WARNING: NOT IN MATH.C — Rust-only setter. zsh C reads parameters
// directly from the global param table on demand; the Rust port
// caller seeds an in-memory map up front via this fn.
pub(crate) fn with_variables(vars: HashMap<String, Mnumber>) {
    m_variables_set(vars);
}

// WARNING: NOT IN MATH.C — Rust-only setter. Parses each value as
// numeric → `Mnumber` if possible, otherwise stores the raw string
// for `getmathparam`'s recursive-eval path (e.g. `a="3+2"; $((a))`).
/// Inject variables from string->string mapping (for shell integration)
pub(crate) fn with_string_variables(vars: &HashMap<String, String>) {
    for (k, v) in vars {
        if let Ok(i) = v.parse::<i64>() {
            m_variables_insert(k.clone(), Mnumber { l: i, d: 0.0, type_: MN_INTEGER });
        } else if let Ok(f) = v.parse::<f64>() {
            m_variables_insert(k.clone(), Mnumber { l: 0, d: f, type_: MN_FLOAT });
        } else if !v.is_empty() {
            // Non-numeric string — keep raw so getmathparam can
            // recursively evaluate it as an arith expression.
            // zsh: `a="3+2"; $((a))` returns 5.
            m_string_variables_insert(k.clone(), v.clone());
        }
    }
}

// WARNING: NOT IN MATH.C — Rust-only accessor. zsh C writes back
// to the global param table during evaluation; ShellExecutor
// integration uses this to harvest the post-eval variables map and
// merge it into its own `variables` table.
/// Extract modified variables as string->string mapping (for shell integration)
pub(crate) fn extract_string_variables() -> HashMap<String, String> {
    M_VARIABLES.with(|c| {
        c.borrow()
            .iter()
            .map(|(k, v)| (k.clone(), match v.type_ {
                MN_INTEGER => v.l.to_string(),
                MN_FLOAT => {
                    let f = v.d;
                    if isnan(f) { "NaN".to_string() }
                    else if isinf(f) { if f > 0.0 { "Inf".to_string() } else { "-Inf".to_string() } }
                    else { format!("{:.10}", f) }
                }
                _ => "0".to_string(),
            }))
            .collect()
    })
}

// WARNING: NOT IN MATH.C — Rust-only setopt mirror. zsh C reads
// the option flag directly from `isset(CPRECEDENCES)` inside
// `mathevall()`; this setter caches the bit so the evaluator
// avoids re-reading the option tree on every token.
pub(crate) fn with_c_precedences(enable: bool) {
    m_c_precedences_set(enable);
    m_prec_set(if enable { &C_PREC } else { &Z_PREC });
}

// WARNING: NOT IN MATH.C — Rust-only setopt mirror for FORCE_FLOAT.
pub(crate) fn with_force_float(enable: bool) {
    m_force_float_set(enable);
}

// WARNING: NOT IN MATH.C — Rust-only setopt mirror for OCTAL_ZEROES.
pub(crate) fn with_octal_zeroes(enable: bool) {
    m_octal_zeroes_set(enable);
}

// WARNING: NOT IN MATH.C — Rust-only setter for `$?` (last command
// status) so the `?`-token in unary position can read it. zsh C
// reads `lastval` directly as a global.
pub(crate) fn with_lastval(val: i32) {
    m_lastval_set(val);
}

    // WARNING: NOT IN MATH.C — Rust-only cursor read. C uses `*ptr`
    // directly without an fn-shaped wrapper.
    pub(crate) fn peek() -> Option<char> {
        m_input_clone()[m_pos()..].chars().next()
    }

    // WARNING: NOT IN MATH.C — Rust-only cursor advance. C uses
    // `*ptr++` directly.
    pub(crate) fn advance() -> Option<char> {
        let c = peek()?;
        m_pos_add(c.len_utf8());
        Some(c)
    }

    // WARNING: NOT IN MATH.C — Rust-only char classifier. C uses
    // ctype.h `idigit()` macro directly.
    fn is_digit(c: char) -> bool {
        c.is_ascii_digit()
    }

    // WARNING: NOT IN MATH.C — Rust-only char classifier. C uses
    // `iident()` / `isalpha()` macros directly.
    fn is_ident_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }

    // WARNING: NOT IN MATH.C — Rust-only char classifier. C uses
    // `iident()` macro directly.
    fn is_ident(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

/// Port of `lexconstant()` from `Src/math.c:462`.
///
/// Lex a numeric constant — decimal/hex/binary/octal integer or
/// floating-point literal. Sets `m_yyval()` and returns
/// `NUM`. Recognises `0x`/`0b` prefixes, base-prefix
/// (`16#FF`), trailing-dot float, scientific notation, and zsh's
/// underscore digit-grouping. Mirrors C's `zstrtol_underscore()`
/// for greedy base parsing (consume valid digits only, leave the
/// rest as the next token).
pub(crate) fn lexconstant() -> i32 {
        let _start = m_pos();
        let mut is_neg = false;

        // Handle leading minus for unary context
        if peek() == Some('-') {
            is_neg = true;
            advance();
        }

        // Check for hex/binary/octal
        if peek() == Some('0') {
            advance();
            match peek().map(|c| c.to_ascii_lowercase()) {
                Some('x') => {
                    // Hex: 0xFF
                    advance();
                    let hex_start = m_pos();
                    while let Some(c) = peek() {
                        if c.is_ascii_hexdigit() || c == '_' {
                            advance();
                        } else {
                            break;
                        }
                    }
                    let hex_str: String = m_input_clone()[hex_start..m_pos()]
                        .chars()
                        .filter(|&c| c != '_')
                        .collect();
                    let val = i64::from_str_radix(&hex_str, 16).unwrap_or(0);
                    m_lastbase_set(16);
                    m_yyval_set(if m_force_float() {
                        Mnumber { l: 0, d: if is_neg { -(val as f64) } else { val as f64 }, type_: MN_FLOAT }
                    } else {
                        Mnumber { l: if is_neg { -val } else { val }, d: 0.0, type_: MN_INTEGER }
                    });
                    return NUM;
                }
                Some('b') => {
                    // Binary: 0b1010
                    advance();
                    let bin_start = m_pos();
                    while let Some(c) = peek() {
                        if c == '0' || c == '1' || c == '_' {
                            advance();
                        } else {
                            break;
                        }
                    }
                    let bin_str: String = m_input_clone()[bin_start..m_pos()]
                        .chars()
                        .filter(|&c| c != '_')
                        .collect();
                    let val = i64::from_str_radix(&bin_str, 2).unwrap_or(0);
                    m_lastbase_set(2);
                    m_yyval_set(if m_force_float() {
                        Mnumber { l: 0, d: if is_neg { -(val as f64) } else { val as f64 }, type_: MN_FLOAT }
                    } else {
                        Mnumber { l: if is_neg { -val } else { val }, d: 0.0, type_: MN_INTEGER }
                    });
                    return NUM;
                }
                Some('o') | Some('O') => {
                    // zsh rejects `0o…` octal-prefix (Rust/Python form).
                    // Only `0x` (hex), `0b` (binary), and bare-leading-0
                    // (with `setopt octalzeroes`) are recognized. Emit
                    // the same diagnostic zsh produces — set s.error
                    // and return a stub Num so the caller's
                    // error-propagation path picks up the failure.
                    m_error_set(format!(
                        "bad math expression: operator expected at `{}'",
                        &m_input_clone()[m_pos()..]
                    ));
                    m_yyval_set(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER });
                    return NUM;
                }
                _ => {
                    // Could be octal or just 0
                    if m_octal_zeroes() {
                        // Check if this looks like octal
                        let oct_start = m_pos();
                        let mut is_octal = true;
                        while let Some(c) = peek() {
                            if c.is_ascii_digit() || c == '_' {
                                if ('8'..='9').contains(&c) {
                                    is_octal = false;
                                }
                                advance();
                            } else if c == '.' || c == 'e' || c == 'E' || c == '#' {
                                is_octal = false;
                                break;
                            } else {
                                break;
                            }
                        }
                        if is_octal && m_pos() > oct_start {
                            let oct_str: String = m_input_clone()[oct_start..m_pos()]
                                .chars()
                                .filter(|&c| c != '_')
                                .collect();
                            let val = i64::from_str_radix(&oct_str, 8).unwrap_or(0);
                            m_lastbase_set(8);
                            m_yyval_set(if m_force_float() {
                                Mnumber { l: 0, d: if is_neg { -(val as f64) } else { val as f64 }, type_: MN_FLOAT }
                            } else {
                                Mnumber { l: if is_neg { -val } else { val }, d: 0.0, type_: MN_INTEGER }
                            });
                            return NUM;
                        }
                        m_pos_set(oct_start);
                    }
                    // Put back the 0
                    m_pos_sub(1);
                }
            }
        }

        // Parse decimal integer or float
        let num_start = m_pos();
        while let Some(c) = peek() {
            if is_digit(c) || c == '_' {
                advance();
            } else {
                break;
            }
        }

        // Check for float
        if peek() == Some('.') || peek() == Some('e') || peek() == Some('E') {
            // Float
            if peek() == Some('.') {
                advance();
                while let Some(c) = peek() {
                    if is_digit(c) || c == '_' {
                        advance();
                    } else {
                        break;
                    }
                }
            }
            if peek() == Some('e') || peek() == Some('E') {
                advance();
                if peek() == Some('+') || peek() == Some('-') {
                    advance();
                }
                while let Some(c) = peek() {
                    if is_digit(c) || c == '_' {
                        advance();
                    } else {
                        break;
                    }
                }
            }
            let float_str: String = m_input_clone()[num_start..m_pos()]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let val: f64 = float_str.parse().unwrap_or(0.0);
            m_yyval_set(Mnumber { l: 0, d: if is_neg { -val } else { val }, type_: MN_FLOAT });
            return NUM;
        }

        // Check for base#value syntax (e.g., 16#FF)
        if peek() == Some('#') {
            advance();
            let base_str: String = m_input_clone()[num_start..m_pos() - 1]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let base: u32 = base_str.parse().unwrap_or(10);
            // zsh: `1#X` errors with "invalid base (must be 2 to 36 inclusive)".
            // i64::from_str_radix panics on out-of-range base; reject early.
            if !(2..=36).contains(&base) {
                m_error_set(format!(
                    "invalid base (must be 2 to 36 inclusive): {}",
                    base
                ));
                m_yyval_set(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER });
                return NUM;
            }
            m_lastbase_set(base as i32);

            // Mirror zsh's `zstrtol_underscore(ptr, &ptr, base, 1)`
            // semantics: consume ONLY chars valid for the base
            // (greedy), stopping at the first invalid digit.
            // Underscore-as-thousands-separator is allowed
            // mid-number. The remaining input becomes the next
            // token, which the parser will then trip on as
            // "operator expected at `<rest>'" via the regular
            // checkunary/parser path.
            //
            // Earlier version used Rust's `from_str_radix` which
            // is all-or-nothing — a single bad digit nuked the
            // entire literal. For `2#1011x` zsh consumes the
            // valid `1011` (= 11) and errors on the trailing `x`;
            // ours errored on the whole `1011x` as one chunk.
            // Same for `2#10112` (zsh: at `2`, ours: at `10112`).
            //
            // Empty-digit-sequence case (`10#`, `2#`) silently
            // yields 0, matching zsh's `zstrtol` returning 0 when
            // no valid digits follow.
            let mut val: i64 = 0;
            let base_i64 = base as i64;
            while let Some(c) = peek() {
                if c == '_' {
                    advance();
                    continue;
                }
                let digit_val: Option<u32> = if c.is_ascii_digit() {
                    Some(c as u32 - '0' as u32)
                } else if c.is_ascii_alphabetic() {
                    Some(c.to_ascii_lowercase() as u32 - 'a' as u32 + 10)
                } else {
                    None
                };
                let Some(d) = digit_val else {
                    break;
                };
                if d >= base {
                    break;
                }
                val = val.saturating_mul(base_i64).saturating_add(d as i64);
                advance();
            }
            m_yyval_set(if m_force_float() {
                Mnumber { l: 0, d: if is_neg { -(val as f64) } else { val as f64 }, type_: MN_FLOAT }
            } else {
                Mnumber { l: if is_neg { -val } else { val }, d: 0.0, type_: MN_INTEGER }
            });
            return NUM;
        }

        // Plain integer
        let int_str: String = m_input_clone()[num_start..m_pos()]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let val: i64 = int_str.parse().unwrap_or(0);
        m_yyval_set(if m_force_float() {
            Mnumber { l: 0, d: if is_neg { -(val as f64) } else { val as f64 }, type_: MN_FLOAT }
        } else {
            Mnumber { l: if is_neg { -val } else { val }, d: 0.0, type_: MN_INTEGER }
        });
        NUM
    }

/// Port of `zzlex()` from `Src/math.c:617`.
///
/// Main math-expression lexer — returns the next token, advancing
/// `m_pos()` and updating `m_yyval()` / `m_yylval_clone()` as side-effects.
/// Handles all operators, ident lookahead for `Func` vs `Id`,
/// `[base]value` / `[#base]EXPR` output-radix prefixes, char
/// constants (`#x`, `##varname`), and dispatches numeric literals
/// to `lexconstant()`.
pub(crate) fn zzlex() -> i32 {
        m_yyval_set(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER });

        loop {
            let pre_pos = m_pos();
            let c = match advance() {
                Some(c) => c,
                None => {
                    m_tok_start_set(pre_pos);
                    return EOI;
                }
            };

            if matches!(c, ' ' | '\t' | '\n' | '"') {
                continue;
            }
            // Record where this token began (post-whitespace) so error
            // formatters can produce zsh-style "at `<remaining>`" messages.
            m_tok_start_set(pre_pos);

            match c {
                '+' => {
                    if peek() == Some('+') {
                        advance();
                        return if m_unary() {
                            PREPLUS
                        } else {
                            POSTPLUS
                        };
                    }
                    if peek() == Some('=') {
                        advance();
                        return PLUSEQ;
                    }
                    return if m_unary() {
                        UPLUS
                    } else {
                        PLUS
                    };
                }

                '-' => {
                    if peek() == Some('-') {
                        advance();
                        return if m_unary() {
                            PREMINUS
                        } else {
                            POSTMINUS
                        };
                    }
                    if peek() == Some('=') {
                        advance();
                        return MINUSEQ;
                    }
                    if m_unary() {
                        // Check if followed by digit for negative number
                        if let Some(next) = peek() {
                            if is_digit(next) || next == '.' {
                                m_pos_sub(1); // Put back the -
                                return lexconstant();
                            }
                        }
                        return UMINUS;
                    }
                    return MINUS;
                }

                '(' => return M_INPAR,
                ')' => return M_OUTPAR,

                '!' => {
                    if peek() == Some('=') {
                        advance();
                        return NEQ;
                    }
                    return NOT;
                }

                '~' => return COMP,

                '&' => {
                    if peek() == Some('&') {
                        advance();
                        if peek() == Some('=') {
                            advance();
                            return DANDEQ;
                        }
                        return DAND;
                    }
                    if peek() == Some('=') {
                        advance();
                        return ANDEQ;
                    }
                    return AND;
                }

                '|' => {
                    if peek() == Some('|') {
                        advance();
                        if peek() == Some('=') {
                            advance();
                            return DOREQ;
                        }
                        return DOR;
                    }
                    if peek() == Some('=') {
                        advance();
                        return OREQ;
                    }
                    return OR;
                }

                '^' => {
                    if peek() == Some('^') {
                        advance();
                        if peek() == Some('=') {
                            advance();
                            return DXOREQ;
                        }
                        return DXOR;
                    }
                    if peek() == Some('=') {
                        advance();
                        return XOREQ;
                    }
                    return XOR;
                }

                '*' => {
                    if peek() == Some('*') {
                        advance();
                        if peek() == Some('=') {
                            advance();
                            return POWEREQ;
                        }
                        return POWER;
                    }
                    if peek() == Some('=') {
                        advance();
                        return MULEQ;
                    }
                    return MUL;
                }

                '/' => {
                    if peek() == Some('=') {
                        advance();
                        return DIVEQ;
                    }
                    return DIV;
                }

                '%' => {
                    if peek() == Some('=') {
                        advance();
                        return MODEQ;
                    }
                    return MOD;
                }

                '<' => {
                    if peek() == Some('<') {
                        advance();
                        if peek() == Some('=') {
                            advance();
                            return SHLEFTEQ;
                        }
                        return SHLEFT;
                    }
                    if peek() == Some('=') {
                        advance();
                        return LEQ;
                    }
                    return LES;
                }

                '>' => {
                    if peek() == Some('>') {
                        advance();
                        if peek() == Some('=') {
                            advance();
                            return SHRIGHTEQ;
                        }
                        return SHRIGHT;
                    }
                    if peek() == Some('=') {
                        advance();
                        return GEQ;
                    }
                    return GRE;
                }

                '=' => {
                    if peek() == Some('=') {
                        advance();
                        return DEQ;
                    }
                    return EQ;
                }

                '$' => {
                    // $$ = pid
                    m_yyval_set(Mnumber { l: m_pid(), d: 0.0, type_: MN_INTEGER });
                    return NUM;
                }

                '?' => {
                    if m_unary() {
                        // $? = lastval
                        m_yyval_set(Mnumber { l: m_lastval() as i64, d: 0.0, type_: MN_INTEGER });
                        return NUM;
                    }
                    return QUEST;
                }

                ':' => return COLON,
                ',' => return COMMA,

                '[' => {
                    // [base]value or output format [#base]
                    if is_digit(peek().unwrap_or('\0')) {
                        // [base]value
                        let base_start = m_pos();
                        while let Some(c) = peek() {
                            if is_digit(c) {
                                advance();
                            } else {
                                break;
                            }
                        }
                        if peek() != Some(']') {
                            m_error_set("bad base syntax".to_string());
                            return EOI;
                        }
                        let base_str: String = m_input_clone()[base_start..m_pos()].to_string();
                        let base: u32 = base_str.parse().unwrap_or(10);
                        advance(); // skip ]

                        if !is_digit(peek().unwrap_or('\0'))
                            && !is_ident_start(peek().unwrap_or('\0'))
                        {
                            m_error_set("bad base syntax".to_string());
                            return EOI;
                        }
                        // Reject out-of-range bases; from_str_radix panics
                        // on bases outside [2, 36].
                        if !(2..=36).contains(&base) {
                            m_error_set(format!(
                                "invalid base (must be 2 to 36 inclusive): {}",
                                base
                            ));
                            m_yyval_set(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER });
                            return NUM;
                        }

                        let val_start = m_pos();
                        while let Some(c) = peek() {
                            if c.is_ascii_alphanumeric() {
                                advance();
                            } else {
                                break;
                            }
                        }
                        let val_str = &m_input_clone()[val_start..m_pos()];
                        let val = i64::from_str_radix(val_str, base).unwrap_or(0);
                        m_lastbase_set(base as i32);
                        m_yyval_set(Mnumber { l: val, d: 0.0, type_: MN_INTEGER });
                        return NUM;
                    }
                    // Output format specifier [#base] - skip for now
                    if peek() == Some('#') {
                        while let Some(c) = peek() {
                            if c == ']' {
                                advance();
                                break;
                            }
                            advance();
                        }
                        continue;
                    }
                    m_error_set("bad output format specification".to_string());
                    return EOI;
                }

                '#' => {
                    // Character code: #\x or ##string
                    if peek() == Some('\\') || peek() == Some('#') {
                        advance();
                        if let Some(ch) = advance() {
                            m_yyval_set(Mnumber { l: ch as i64, d: 0.0, type_: MN_INTEGER });
                            return NUM;
                        }
                    }
                    // #varname - get first char value
                    let id_start = m_pos();
                    while let Some(c) = peek() {
                        if is_ident(c) {
                            advance();
                        } else {
                            break;
                        }
                    }
                    if m_pos() > id_start {
                        m_yylval_set(m_input_clone()[id_start..m_pos()].to_string());
                        return CID;
                    }
                    continue;
                }

                _ => {
                    if is_digit(c)
                        || (c == '.' && is_digit(peek().unwrap_or('\0')))
                    {
                        m_pos_sub(c.len_utf8());
                        return lexconstant();
                    }

                    if is_ident_start(c) {
                        let id_start = m_pos() - c.len_utf8();
                        while let Some(c) = peek() {
                            if is_ident(c) {
                                advance();
                            } else {
                                break;
                            }
                        }

                        let id = &m_input_clone()[id_start..m_pos()];

                        // Check for Inf/NaN
                        let id_lower = id.to_lowercase();
                        if id_lower == "nan" {
                            m_yyval_set(Mnumber { l: 0, d: f64::NAN, type_: MN_FLOAT });
                            return NUM;
                        }
                        if id_lower == "inf" {
                            m_yyval_set(Mnumber { l: 0, d: f64::INFINITY, type_: MN_FLOAT });
                            return NUM;
                        }

                        // Check for function call
                        if peek() == Some('(') {
                            // Skip to closing paren
                            let func_start = id_start;
                            advance(); // (
                            let mut depth = 1;
                            while let Some(c) = peek() {
                                advance();
                                if c == '(' {
                                    depth += 1;
                                } else if c == ')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                            m_yylval_set(m_input_clone()[func_start..m_pos()].to_string());
                            return FUNC;
                        }

                        // Check for array subscript
                        if peek() == Some('[') {
                            advance(); // [
                            let mut depth = 1;
                            while let Some(c) = peek() {
                                advance();
                                if c == '[' {
                                    depth += 1;
                                } else if c == ']' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                        }

                        m_yylval_set(m_input_clone()[id_start..m_pos()].to_string());
                        return ID;
                    }

                    return EOI;
                }
            }
        }
    }

/// Port of `push(val, lval, getme)` from `Src/math.c:916`.
///
/// Push a value onto the evaluator's operand stack, with the
/// optional lvalue name (set when the value came from a variable
/// reference; needed for `++`/`--`/assignment-op write-back).
pub(crate) fn push(val: Mnumber, lval: Option<String>) {
    m_stack_push(MathValue { val, lval, pval: () });
}

/// Port of `pop(noget)` from `Src/math.c:931`.
///
/// Pop the top operand from the stack, resolving any deferred
/// variable read (`Mnumber { l: 0, d: 0.0, type_: MN_UNSET }` + lval set). The C source
/// passes a `noget` flag to skip the resolution; the Rust port
/// always resolves since callers that want the raw lvalue use
/// `pop_with_lval` instead.
pub(crate) fn pop() -> Mnumber {
    if let Some(mv) = m_stack_pop() {
        if (mv.val.type_ == MN_UNSET) {
            if let Some(ref name) = mv.lval {
                return getmathparam(name);
            }
        }
        mv.val
    } else {
        m_error_set("stack underflow".to_string());
        Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
    }
    }

    // WARNING: NOT IN MATH.C — Rust-only stack helper. C inlines
    // this inside `pop()` (math.c:931) — its `noget` flag controls
    // whether to resolve the deferred Unset+lval read; zshrs splits
    // the two paths into separate fns so the resolved-vs-raw choice
    // is at the call site.
    pub(crate) fn pop_with_lval() -> MathValue {
        m_stack_pop().unwrap_or_default()
    }

    // WARNING: NOT IN MATH.C — Rust-only value-resolver. C inlines
    // the deferred-variable-read pattern inside `pop()` and `op()`
    // (math.c:931, 1154); the Rust port factors it out for `bop`
    // and `mathparse` to inspect-without-consuming.
    pub(crate) fn get_value(mv: &MathValue) -> Mnumber {
        if (mv.val.type_ == MN_UNSET) {
            if let Some(ref name) = mv.lval {
                return getmathparam(name);
            }
        }
        mv.val
    }

/// Port of `getmathparam(mptr)` from `Src/math.c:337`.
///
/// Look up a parameter by name from inside math context. zsh
/// auto-typesets a missing-but-referenced name (its mathparam
/// flag), but the Rust port keeps the variables map separate from
/// the param table so a miss returns `Integer(0)` and skips the
/// type-coercion. Indirect-string mode (`a="3+2"; $((a))`) is
/// handled by recursively evaluating the string value.
pub(crate) fn getmathparam(name: &str) -> Mnumber {
    // Strip array subscript if present
        let base_name = if let Some(bracket) = name.find('[') {
            &name[..bracket]
        } else {
            name
        };
        if let Some(v) = m_variables_get(base_name) {
            return v;
        }
        // c:Src/math.c:337 getmathparam — falls back to `getvalue(s)`
        // which parses the full subscript syntax (params.c:2180).
        // The Rust port previously required callers to seed
        // `with_string_variables` (a pre-populate pattern that
        // diverged from C). Read paramtab + array subscripts here
        // so matheval works without seeding.
        if let Some(bracket) = name.find('[') {
            let close = name.rfind(']').unwrap_or(name.len());
            let arr_name = &name[..bracket];
            let idx_str = &name[bracket + 1..close];
            // Recursively eval the index (so a[i+1], h[$k], etc work).
            let idx_val = matheval(idx_str)
                .map(|n| if n.type_ == crate::ported::zsh_h::MN_FLOAT { n.d as i64 } else { n.l })
                .unwrap_or(0);
            // Read paramtab directly: PM_ARRAY → u_arr indexed by 1-based pos.
            if let Ok(tab) = crate::ported::params::paramtab().read() {
                if let Some(pm) = tab.get(arr_name) {
                    if let Some(arr) = &pm.u_arr {
                        let len = arr.len() as i64;
                        let pos = if idx_val < 0 { len + idx_val } else { idx_val - 1 };
                        if pos >= 0 && (pos as usize) < arr.len() {
                            let raw = &arr[pos as usize];
                            if let Ok(n) = raw.parse::<i64>() {
                                return Mnumber { l: n, d: 0.0, type_: crate::ported::zsh_h::MN_INTEGER };
                            }
                            if let Ok(f) = raw.parse::<f64>() {
                                return Mnumber { l: 0, d: f, type_: crate::ported::zsh_h::MN_FLOAT };
                            }
                        }
                    }
                }
            }
            // PM_HASHED via paramtab_hashed_storage.
            if let Ok(m) = crate::ported::params::paramtab_hashed_storage().lock() {
                if let Some(map) = m.get(arr_name) {
                    if let Some(v) = map.get(idx_str) {
                        if let Ok(n) = v.parse::<i64>() {
                            return Mnumber { l: n, d: 0.0, type_: crate::ported::zsh_h::MN_INTEGER };
                        }
                        if let Ok(f) = v.parse::<f64>() {
                            return Mnumber { l: 0, d: f, type_: crate::ported::zsh_h::MN_FLOAT };
                        }
                    }
                }
            }
            return Mnumber { l: 0, d: 0.0, type_: crate::ported::zsh_h::MN_INTEGER };
        }
        if let Some(raw) = crate::ported::params::getsparam(base_name) {
            if let Ok(n) = raw.parse::<i64>() {
                return Mnumber { l: n, d: 0.0, type_: crate::ported::zsh_h::MN_INTEGER };
            }
            if let Ok(f) = raw.parse::<f64>() {
                return Mnumber { l: 0, d: f, type_: crate::ported::zsh_h::MN_FLOAT };
            }
            // Non-numeric string: fall through to recursive-eval below.
        }
        // Recursive eval: if the var holds a non-numeric string, evaluate
        // it AS an arith expression. zsh: `a="3+2"; $((a))` → 5. Bound
        // to one level of indirection — fresh evaluator each call so we
        // don't accidentally pollute s.variables.
        if let Some(raw) = m_string_variables_get(base_name) {
            // Save parent's eval state — `new(&raw)` resets thread_locals
            // for the sub-eval, which would otherwise clobber the parent.
            // Mirrors C `mathevall()` xyy* save/restore pattern (math.c:367).
            let saved = save_state();
            // Inherit caller's variables/string_variables/prec into the
            // sub-eval, with `base_name` removed from the indirect map to
            // prevent infinite recursion on `a="$a"`-style cycles.
            let inherited_vars = saved.variables.clone();
            let mut inherited_strs = saved.string_variables.clone();
            inherited_strs.remove(base_name);
            let inherited_prec = saved.prec;
            let inherited_c_prec = saved.c_precedences;

            new(&raw);
            m_variables_set(inherited_vars);
            m_string_variables_set(inherited_strs);
            m_prec_set(inherited_prec);
            m_c_precedences_set(inherited_c_prec);

            let result = mathevall();
            restore_state(saved);
            if let Ok(r) = result {
                return r;
            }
        }
        Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
    }

/// Port of `setmathvar(mvp, v)` from `Src/math.c:972`.
///
/// Write `val` to the named parameter from inside math context.
/// Subscripted writes (`a[i] = …`) are pre-handled by the
/// SubscriptArith free fns higher up the call chain; this stub
/// only handles the scalar case. Returns the stored value so
/// `op` can leave it on the stack.
pub(crate) fn setmathvar(name: &str, val: Mnumber) -> Mnumber {
    let base_name = if let Some(bracket) = name.find('[') {
        &name[..bracket]
    } else {
        name
    };
    m_variables_insert(base_name.to_string(), val);
    val
}

/// Port of `op(what)` from `Src/math.c:1154`.
///
/// Apply a binary or unary operator to the operand stack. Pops
/// 1-2 values, applies the operation (with type coercion), and
/// pushes the result. Handles assignment (`OP_E2*` flag) by
/// writing through `setmathvar` and pushing the new value back
/// with the same lvalue so chained assigns work.
pub(crate) fn op(what: i32) {
        if m_error_some() {
            return;
        }

        let tp = OP_TYPE[what as usize];

        // Binary operators
        if (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0 {
            if m_stack_len() < 2 {
                // zsh's exact wording for the same condition is
                // `bad math expression: operand expected at end of
                // string`. Matching it here means `let "1+"` and
                // `$((5+))` produce the same diagnostic shape that
                // scripts grep for.
                m_error_set("bad math expression: operand expected at end of string".to_string());
                return;
            }

            let b = pop();
            let mv_a = pop_with_lval();
            let a = if (mv_a.val.type_ == MN_UNSET) {
                if let Some(ref name) = mv_a.lval {
                    getmathparam(name)
                } else {
                    Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
                }
            } else {
                mv_a.val
            };

            // Coerce types
            let (a, b) = if (tp & (OP_A2IO | OP_E2IO)) != 0 {
                // Must be integers
                (Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }), d: 0.0, type_: MN_INTEGER }, Mnumber { l: (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }), d: 0.0, type_: MN_INTEGER })
            } else if (a.type_ == MN_FLOAT) != (b.type_ == MN_FLOAT) && what != COMMA {
                // Different types, coerce to float
                (Mnumber { l: 0, d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }), type_: MN_FLOAT }, Mnumber { l: 0, d: (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }), type_: MN_FLOAT })
            } else {
                (a, b)
            };

            let result = if m_noeval() > 0 {
                Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
            } else {
                let is_float = (a.type_ == MN_FLOAT);
                match what {
                    AND | ANDEQ => Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) & (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }), d: 0.0, type_: MN_INTEGER },
                    XOR | XOREQ => Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) ^ (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }), d: 0.0, type_: MN_INTEGER },
                    OR | OREQ => Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) | (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }), d: 0.0, type_: MN_INTEGER },

                    MUL | MULEQ => {
                        if is_float {
                            Mnumber { l: 0, d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) * (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }), type_: MN_FLOAT }
                        } else {
                            Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }).wrapping_mul((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })), d: 0.0, type_: MN_INTEGER }
                        }
                    }

                    DIV | DIVEQ => {
                        // Float div-by-zero is NOT an error in zsh —
                        // it produces IEEE Inf/-Inf/NaN per IEEE 754.
                        // Only INTEGER div-by-zero raises the error.
                        // Without this gate `1/0.0` errored out instead
                        // of returning `Inf`.
                        if is_float {
                            // Let f64 semantics handle 0.0, -0.0, NaN.
                            Mnumber { l: 0, d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) / (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }), type_: MN_FLOAT }
                        } else {
                            if !notzero(b) {
                                m_error_set("division by zero".to_string());
                                return;
                            }
                            let bi = (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l });
                            if bi == -1 {
                                Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }).wrapping_neg(), d: 0.0, type_: MN_INTEGER }
                            } else {
                                Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) / bi, d: 0.0, type_: MN_INTEGER }
                            }
                        }
                    }

                    MOD | MODEQ => {
                        if is_float {
                            // float % 0.0 → NaN per IEEE; let it fall
                            // through to f64 semantics rather than
                            // raising the integer-only error.
                            Mnumber { l: 0, d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) % (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }), type_: MN_FLOAT }
                        } else if !notzero(b) {
                            m_error_set("division by zero".to_string());
                            return;
                        } else {
                            let bi = (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l });
                            if bi == -1 {
                                Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
                            } else {
                                Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) % bi, d: 0.0, type_: MN_INTEGER }
                            }
                        }
                    }

                    PLUS | PLUSEQ => {
                        if is_float {
                            Mnumber { l: 0, d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) + (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }), type_: MN_FLOAT }
                        } else {
                            Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }).wrapping_add((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })), d: 0.0, type_: MN_INTEGER }
                        }
                    }

                    MINUS | MINUSEQ => {
                        if is_float {
                            Mnumber { l: 0, d: (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) - (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 }), type_: MN_FLOAT }
                        } else {
                            Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }).wrapping_sub((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })), d: 0.0, type_: MN_INTEGER }
                        }
                    }

                    SHLEFT | SHLEFTEQ => {
                        Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) << ((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) as u32 & 63), d: 0.0, type_: MN_INTEGER }
                    }
                    SHRIGHT | SHRIGHTEQ => {
                        Mnumber { l: (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) >> ((if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) as u32 & 63), d: 0.0, type_: MN_INTEGER }
                    }

                    LES => Mnumber { l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) < (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 })) as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) < (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })) as i64
                    }, d: 0.0, type_: MN_INTEGER },
                    LEQ => Mnumber { l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) <= (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 })) as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) <= (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })) as i64
                    }, d: 0.0, type_: MN_INTEGER },
                    GRE => Mnumber { l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) > (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 })) as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) > (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })) as i64
                    }, d: 0.0, type_: MN_INTEGER },
                    GEQ => Mnumber { l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) >= (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 })) as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) >= (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })) as i64
                    }, d: 0.0, type_: MN_INTEGER },
                    DEQ => Mnumber { l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) == (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 })) as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) == (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })) as i64
                    }, d: 0.0, type_: MN_INTEGER },
                    NEQ => Mnumber { l: if is_float {
                        ((if a.type_ == MN_FLOAT { a.d } else { a.l as f64 }) != (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 })) as i64
                    } else {
                        ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) != (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l })) as i64
                    }, d: 0.0, type_: MN_INTEGER },

                    DAND | DANDEQ => {
                        Mnumber { l: ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) != 0 && (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) != 0) as i64, d: 0.0, type_: MN_INTEGER }
                    }
                    DOR | DOREQ => {
                        Mnumber { l: ((if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) != 0 || (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) != 0) as i64, d: 0.0, type_: MN_INTEGER }
                    }
                    DXOR | DXOREQ => {
                        let ai = (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l }) != 0;
                        let bi = (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l }) != 0;
                        Mnumber { l: (ai != bi) as i64, d: 0.0, type_: MN_INTEGER }
                    }

                    POWER | POWEREQ => {
                        let bi = (if b.type_ == MN_FLOAT { b.d as i64 } else { b.l });
                        if !is_float && bi >= 0 {
                            let mut result = 1i64;
                            let base = (if a.type_ == MN_FLOAT { a.d as i64 } else { a.l });
                            for _ in 0..bi {
                                result = result.wrapping_mul(base);
                            }
                            Mnumber { l: result, d: 0.0, type_: MN_INTEGER }
                        } else {
                            let af = (if a.type_ == MN_FLOAT { a.d } else { a.l as f64 });
                            let bf = (if b.type_ == MN_FLOAT { b.d } else { b.l as f64 });
                            if bf <= 0.0 && af == 0.0 {
                                m_error_set("division by zero".to_string());
                                return;
                            }
                            if af < 0.0 && bf != bf.trunc() {
                                m_error_set("imaginary power".to_string());
                                return;
                            }
                            Mnumber { l: 0, d: af.powf(bf), type_: MN_FLOAT }
                        }
                    }

                    COMMA => b,
                    EQ => b,

                    _ => Mnumber { l: 0, d: 0.0, type_: MN_INTEGER },
                }
            };

            // Handle assignment
            if (tp & (OP_E2 | OP_E2IO)) != 0 {
                if let Some(ref name) = mv_a.lval {
                    let final_val = setmathvar(name, result);
                    push(final_val, Some(name.clone()));
                } else {
                    m_error_set("lvalue required".to_string());
                    push(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }, None);
                }
            } else {
                push(result, None);
            }
            return;
        }

        // Unary operators
        if m_stack_is_empty() {
            // zsh: unary op with empty stack -> `bad math
            // expression: operand expected at end of string`.
            // zshrs's bare `stack empty` had no match for scripts
            // grepping zsh's canonical wording.
            m_error_set("bad math expression: operand expected at end of string".to_string());
            return;
        }

        let mv = pop_with_lval();
        let val = if (mv.val.type_ == MN_UNSET) {
            if let Some(ref name) = mv.lval {
                getmathparam(name)
            } else {
                Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
            }
        } else {
            mv.val
        };

        match what {
            NOT => {
                let result = Mnumber { l: if ((val.type_ == MN_INTEGER && val.l == 0) || (val.type_ == MN_FLOAT && val.d == 0.0) || val.type_ == MN_UNSET) { 1 } else { 0 }, d: 0.0, type_: MN_INTEGER };
                push(result, None);
            }
            COMP => {
                let result = Mnumber { l: !(if val.type_ == MN_FLOAT { val.d as i64 } else { val.l }), d: 0.0, type_: MN_INTEGER };
                push(result, None);
            }
            UPLUS => {
                push(val, None);
            }
            UMINUS => {
                let result = if (val.type_ == MN_FLOAT) {
                    Mnumber { l: 0, d: -(if val.type_ == MN_FLOAT { val.d } else { val.l as f64 }), type_: MN_FLOAT }
                } else {
                    Mnumber { l: -(if val.type_ == MN_FLOAT { val.d as i64 } else { val.l }), d: 0.0, type_: MN_INTEGER }
                };
                push(result, None);
            }
            POSTPLUS => {
                // ++/-- on a literal (`5++`, `--5`) is a zsh error:
                // "bad math expression: lvalue required". Without the
                // mv.lval guard, zshrs silently incremented the
                // literal value and returned it, masking the bug.
                if mv.lval.is_none() {
                    m_error_set("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if (val.type_ == MN_FLOAT) {
                    Mnumber { l: 0, d: (if val.type_ == MN_FLOAT { val.d } else { val.l as f64 }) + 1.0, type_: MN_FLOAT }
                } else {
                    Mnumber { l: (if val.type_ == MN_FLOAT { val.d as i64 } else { val.l }) + 1, d: 0.0, type_: MN_INTEGER }
                };
                setmathvar(name, new_val);
                push(val, None); // Return original value
            }
            POSTMINUS => {
                if mv.lval.is_none() {
                    m_error_set("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if (val.type_ == MN_FLOAT) {
                    Mnumber { l: 0, d: (if val.type_ == MN_FLOAT { val.d } else { val.l as f64 }) - 1.0, type_: MN_FLOAT }
                } else {
                    Mnumber { l: (if val.type_ == MN_FLOAT { val.d as i64 } else { val.l }) - 1, d: 0.0, type_: MN_INTEGER }
                };
                setmathvar(name, new_val);
                push(val, None);
            }
            PREPLUS => {
                if mv.lval.is_none() {
                    m_error_set("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if (val.type_ == MN_FLOAT) {
                    Mnumber { l: 0, d: (if val.type_ == MN_FLOAT { val.d } else { val.l as f64 }) + 1.0, type_: MN_FLOAT }
                } else {
                    Mnumber { l: (if val.type_ == MN_FLOAT { val.d as i64 } else { val.l }) + 1, d: 0.0, type_: MN_INTEGER }
                };
                setmathvar(name, new_val);
                push(new_val, mv.lval);
            }
            PREMINUS => {
                if mv.lval.is_none() {
                    m_error_set("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if (val.type_ == MN_FLOAT) {
                    Mnumber { l: 0, d: (if val.type_ == MN_FLOAT { val.d } else { val.l as f64 }) - 1.0, type_: MN_FLOAT }
                } else {
                    Mnumber { l: (if val.type_ == MN_FLOAT { val.d as i64 } else { val.l }) - 1, d: 0.0, type_: MN_INTEGER }
                };
                setmathvar(name, new_val);
                push(new_val, mv.lval);
            }
            QUEST => {
                // Ternary: stack has [cond, true_val, false_val]
                // val already popped = false_val
                // Need to pop true_val and cond
                if m_stack_len() < 2 {
                    m_error_set("?: needs 3 operands".to_string());
                    return;
                }
                let false_val = val;
                let true_val = pop();
                let cond = pop();
                let result = if !((cond.type_ == MN_INTEGER && cond.l == 0) || (cond.type_ == MN_FLOAT && cond.d == 0.0) || cond.type_ == MN_UNSET) { true_val } else { false_val };
                push(result, None);
            }
            COLON => {
                m_error_set("':' without '?'".to_string());
            }
            _ => {
                m_error_set("unknown operator".to_string());
            }
        }
    }

/// Port of `bop(tk)` from `Src/math.c:1454`.
///
/// Short-circuit boolean prologue. Inspects (without popping) the
/// top of stack and bumps `m_noeval()` for the parse-only side of
/// `&&` / `||` / their assignment forms. The matching decrement
/// happens after `mathparse` recurses for the RHS.
pub(crate) fn bop(tk: i32) {
        if m_stack_is_empty() {
            return;
        }
        let mv = m_stack_top_clone().unwrap();
        let val = if (mv.val.type_ == MN_UNSET) {
            if let Some(ref name) = mv.lval {
                getmathparam(name)
            } else {
                Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
            }
        } else {
            mv.val
        };

        let tst = !((val.type_ == MN_INTEGER && val.l == 0) || (val.type_ == MN_FLOAT && val.d == 0.0) || val.type_ == MN_UNSET);
        match tk {
            DAND | DANDEQ if !tst => {
                m_noeval_inc();
            }
            DOR | DOREQ if tst => {
                m_noeval_inc();
            }
            _ => {}
        }
    }

    // WARNING: NOT IN MATH.C — Rust-only helper. C inlines the
    // expression `prec[COMMA] + 1` directly in mathparse() and
    // mathevall() everywhere it's needed (math.c:1594, 367).
    pub(crate) fn top_prec() -> u8 {
        m_prec()[COMMA as usize] + 1
    }

/// Port of `checkunary(mtokc, mptr)` from `Src/math.c:1548`.
///
/// Two roles. (1) Validate that the just-lexed token (`m_mtok()`)
/// matches the parser's expectation: an operand was wanted but an
/// operator (`OP_*` flags) showed up, or vice versa. Mismatch
/// emits zsh's `bad math expression: <kind> expected at <ctx>`
/// with `<kind>` being `operator` or `operand` and `<ctx>` taken
/// from the input pointer at the start of the bad token. (2)
/// Update `m_unary()` for the next iteration based on `OP_OPF`.
pub(crate) fn checkunary() {
    // Direct port of zsh math.c checkunary() (line 1548).
        // Two roles:
        //   1. Validate that the just-lexed token (`m_mtok()`)
        //      matches the parser's expectation (operator vs
        //      operand). Mismatch emits zsh's
        //      "bad math expression: <kind> expected at <ctx>"
        //      with `<kind>` = `operator` (errmsg=2) or `operand`
        //      (errmsg=1). zshrs previously only did step 2,
        //      which left e.g. `let "5 5"` and `$((2#1011x))`
        //      silently accepting bogus input.
        //   2. Update `m_unary()` for the next iteration.
        let tp = OP_TYPE[m_mtok() as usize];
        let is_op_token = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO | OP_OP)) != 0;
        let errmsg = if is_op_token {
            if m_unary() {
                1
            } else {
                0
            }
        } else if !m_unary() {
            2
        } else {
            0
        };
        if errmsg != 0 && !m_error_some() {
            let errtype = if errmsg == 2 { "operator" } else { "operand" };
            // zsh's `mptr` is the input position BEFORE zzlex
            // consumed the bad token. We track the same via
            // `tok_start` which zzlex updates after whitespace
            // skip. Walk forward past whitespace (mirrors zsh's
            // `inblank` skip) so the error context starts at
            // the first visible char.
            let input_owned = m_input_clone();
            let bytes = input_owned.as_bytes();
            let mut start = m_tok_start();
            while start < bytes.len() && matches!(bytes[start], b' ' | b'\t' | b'\n') {
                start += 1;
            }
            // zsh truncates after 10 chars and appends `...` if
            // there's more remaining (the over flag in the C
            // source). Mirror that to keep error messages
            // bounded for long bogus expressions.
            let remaining = m_input_slice_from(start);
            let (ctx, over) = if remaining.chars().count() > 10 {
                let truncated: String = remaining.chars().take(10).collect();
                (truncated, true)
            } else {
                (remaining.to_string(), false)
            };
            if ctx.is_empty() {
                m_error_set(format!(
                    "bad math expression: {} expected at end of string",
                    errtype
                ));
            } else {
                m_error_set(format!(
                    "bad math expression: {} expected at `{}{}'",
                    errtype,
                    ctx,
                    if over { "..." } else { "" }
                ));
            }
        }
        m_unary_set((tp & OP_OPF) == 0);
    }

    /// Operator-precedence parser - closely follows zsh math.c mathparse()
/// Port of `mathparse(pc)` from `Src/math.c:1594`.
    pub(crate) fn mathparse(pc: u8) {
        if m_error_some() {
            return;
        }

        m_mtok_set(zzlex());

        // Handle empty input
        if pc == top_prec() && m_mtok() == EOI {
            return;
        }

        checkunary();

        while m_prec()[m_mtok() as usize] <= pc {
            if m_error_some() {
                return;
            }

            match m_mtok() {
                NUM => {
                    push(m_yyval(), None);
                }
                ID => {
                    let lval = m_yylval_clone();
                    if m_noeval() > 0 {
                        push(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }, Some(lval));
                    } else {
                        push(Mnumber { l: 0, d: 0.0, type_: MN_UNSET }, Some(lval));
                    }
                }
                CID => {
                    let lval = m_yylval_clone();
                    let val = if m_noeval() > 0 {
                        Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
                    } else {
                        getcvar(&lval)
                    };
                    push(val, Some(lval));
                }
                FUNC => {
                    let func_call = m_yylval_clone();
                    let val = if m_noeval() > 0 {
                        Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
                    } else {
                        callmathfunc(&func_call)
                    };
                    push(val, None);
                }
                M_INPAR => {
                    mathparse(top_prec());
                    if m_mtok() != M_OUTPAR {
                        if !m_error_some() {
                            // Match zsh's `bad math expression: ')'
                            // expected` so error diagnostics align.
                            m_error_set("bad math expression: ')' expected".to_string());
                        }
                        return;
                    }
                }
                QUEST => {
                    // Ternary operator
                    if m_stack_is_empty() {
                        m_error_set("bad math expression".to_string());
                        return;
                    }
                    let mv = m_stack_top_clone().unwrap();
                    let cond = get_value(&mv);

                    let q = !((cond.type_ == MN_INTEGER && cond.l == 0) || (cond.type_ == MN_FLOAT && cond.d == 0.0) || cond.type_ == MN_UNSET);
                    if !q {
                        m_noeval_inc();
                    }
                    let colon_prec = m_prec()[COLON as usize];
                    let stack_before = m_stack_len();
                    mathparse(colon_prec - 1);
                    if !q {
                        m_noeval_dec();
                    }

                    if m_mtok() != COLON {
                        if !m_error_some() {
                            // Distinguish whether the inner parse
                            // produced an operand: stack grew →
                            // colon expected; stack same → operand
                            // missing (input ran out at end of
                            // string after `?`).
                            if m_stack_len() > stack_before {
                                m_error_set("bad math expression: ':' expected".to_string());
                            } else {
                                m_error_set(
                                    "bad math expression: operand expected at end of string"
                                        .to_string(),
                                );
                            }
                        }
                        return;
                    }

                    if q {
                        m_noeval_inc();
                    }
                    let quest_prec = m_prec()[QUEST as usize];
                    mathparse(quest_prec);
                    if q {
                        m_noeval_dec();
                    }

                    op(QUEST);
                    continue;
                }
                _ => {
                    // Binary/unary operator
                    let otok = m_mtok();
                    let onoeval = m_noeval();
                    let tp = OP_TYPE[otok as usize];
                    // Orphan binary at start: `let "*"`, `let "*5"`,
                    // `let "/"`. zsh keeps its input pointer at the
                    // start of the bad operator and emits `operand
                    // expected at \`<remaining>'`. zshrs previously
                    // collapsed every operand-missing case into "at
                    // end of string" which lost the operator
                    // location for orphan-at-start expressions.
                    let is_binary = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0;
                    if m_stack_is_empty() && is_binary {
                        let remaining = m_input_slice_from(m_tok_start());
                        m_error_set(format!(
                            "bad math expression: operand expected at `{}'",
                            remaining
                        ));
                        return;
                    }
                    if (tp & 0x03) == BOOL {
                        bop(otok);
                    }
                    let otok_prec = m_prec()[otok as usize];
                    // Right-to-left gets same prec, left-to-right gets prec-1
                    let adjust = if (tp & 0x01) != RL { 1 } else { 0 };
                    mathparse(otok_prec - adjust);
                    m_noeval_set(onoeval);
                    op(otok);
                    continue;
                }
            }

            // After operand (Num, Id, Func, InPar), get next token
            m_mtok_set(zzlex());
            checkunary();
        }
    }

    /// Call a math function
/// Port of `callmathfunc(o)` from `Src/math.c:1037`.
    pub(crate) fn callmathfunc(call: &str) -> Mnumber {
        // Parse function name and args
        let paren = call.find('(').unwrap_or(call.len());
        let name = &call[..paren];
        let args_str = if paren < call.len() {
            &call[paren + 1..call.len() - 1]
        } else {
            ""
        };

        // Parse arguments. Keep both the float view (for trig) and the
        // original Mnumber so int-preserving functions (abs/min/max/
        // int/floor/ceil/trunc) can return integer when all inputs
        // were integer.
        let arg_nums: Vec<Mnumber> = if args_str.is_empty() {
            vec![]
        } else {
            args_str
                .split(',')
                .filter_map(|arg| {
                    // Save caller's eval state, sub-eval each arg in a
                    // fresh state inheriting caller's variables, restore.
                    // C `mathevall()` xyy* save/restore (math.c:367).
                    let saved = save_state();
                    let inherited_vars = saved.variables.clone();
                    new(arg.trim());
                    m_variables_set(inherited_vars);
                    let result = mathevall().ok();
                    restore_state(saved);
                    result
                })
                .collect()
        };
        let args: Vec<f64> = arg_nums.iter().map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).collect();
        let all_int =
            !arg_nums.is_empty() && arg_nums.iter().all(|n| (n.type_ == MN_INTEGER));

        // Functions that preserve int-ness: when all args are int,
        // return Mnumber::Integer instead of Float to avoid the
        // trailing "." in the string output ("5." instead of "5").
        //
        // `int`/`floor`/`ceil`/`trunc` ALWAYS return Integer per zsh
        // (mathfunc.c:bin_zmathfn) — `$(( int(2.7) ))` prints "2",
        // not "2.". The truncation to int happens regardless of
        // whether the input was already an integer. `abs`/`min`/`max`
        // preserve the input type (int args → int result, float arg
        // anywhere → float result) since their semantics don't
        // inherently change the value's representation.
        let always_int = matches!(name, "int" | "floor" | "ceil" | "trunc");
        if always_int {
            let i = match name {
                "int" | "trunc" => arg_nums.first().map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l })).unwrap_or(0),
                "floor" => args.first().map(|x| x.floor() as i64).unwrap_or(0),
                "ceil" => args.first().map(|x| x.ceil() as i64).unwrap_or(0),
                _ => 0,
            };
            return Mnumber { l: i, d: 0.0, type_: MN_INTEGER };
        }
        let int_preserving = matches!(name, "abs" | "min" | "max");
        if all_int && int_preserving {
            let i = match name {
                "abs" => arg_nums.first().map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l }).abs()).unwrap_or(0),
                "min" => arg_nums.iter().map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l })).min().unwrap_or(0),
                "max" => arg_nums.iter().map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l })).max().unwrap_or(0),
                _ => 0,
            };
            return Mnumber { l: i, d: 0.0, type_: MN_INTEGER };
        }

        // Built-in math functions
        let result = match name {
            "abs" => args.first().map(|x| x.abs()).unwrap_or(0.0),
            "acos" => args.first().map(|x| x.acos()).unwrap_or(0.0),
            "asin" => args.first().map(|x| x.asin()).unwrap_or(0.0),
            "atan" => args.first().map(|x| x.atan()).unwrap_or(0.0),
            "atan2" => {
                let y = args.first().copied().unwrap_or(0.0);
                let x = args.get(1).copied().unwrap_or(1.0);
                y.atan2(x)
            }
            "ceil" => args.first().map(|x| x.ceil()).unwrap_or(0.0),
            "cos" => args.first().map(|x| x.cos()).unwrap_or(1.0),
            "cosh" => args.first().map(|x| x.cosh()).unwrap_or(1.0),
            "exp" => args.first().map(|x| x.exp()).unwrap_or(1.0),
            "floor" => args.first().map(|x| x.floor()).unwrap_or(0.0),
            "hypot" => {
                let x = args.first().copied().unwrap_or(0.0);
                let y = args.get(1).copied().unwrap_or(0.0);
                x.hypot(y)
            }
            "int" => args.first().map(|x| x.trunc()).unwrap_or(0.0),
            "log" => args.first().map(|x| x.ln()).unwrap_or(0.0),
            "log10" => args.first().map(|x| x.log10()).unwrap_or(0.0),
            "log2" => args.first().map(|x| x.log2()).unwrap_or(0.0),
            "max" => args.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            "min" => args.iter().copied().fold(f64::INFINITY, f64::min),
            "pow" => {
                let base = args.first().copied().unwrap_or(0.0);
                let exp = args.get(1).copied().unwrap_or(1.0);
                base.powf(exp)
            }
            "rand" => rand::random::<f64>(),
            "round" => args.first().map(|x| x.round()).unwrap_or(0.0),
            "sin" => args.first().map(|x| x.sin()).unwrap_or(0.0),
            "sinh" => args.first().map(|x| x.sinh()).unwrap_or(0.0),
            "sqrt" => args.first().map(|x| x.sqrt()).unwrap_or(0.0),
            "tan" => args.first().map(|x| x.tan()).unwrap_or(0.0),
            "tanh" => args.first().map(|x| x.tanh()).unwrap_or(0.0),
            "trunc" => args.first().map(|x| x.trunc()).unwrap_or(0.0),
            // `float(x)` — widen int/float to float. Identity on
            // floats; on ints, returns same value tagged as float so
            // `printf "%.4f"` prints "3.0000" instead of "3". Direct
            // port of mathfunc.c's `to_float()`.
            "float" => args.first().copied().unwrap_or(0.0),
            _ => {
                m_error_set(format!("unknown function: {}", name));
                0.0
            }
        };

        Mnumber { l: 0, d: result, type_: MN_FLOAT }
    }

    /// Evaluate the expression
/// Port of `mathevall(s, prec_tp, ep)` from `Src/math.c:367`.
    pub(crate) fn mathevall() -> Result<Mnumber, String> {
        m_prec_set(if m_c_precedences() { &C_PREC } else { &Z_PREC });

        // Skip leading whitespace and Nularg
        while let Some(c) = peek() {
            if c.is_whitespace() || c == '\u{a1}' {
                advance();
            } else {
                break;
            }
        }

        if m_pos() >= m_input_len() {
            return Ok(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER });
        }

        mathparse(top_prec());

        if let Some(err) = m_error_take() {
            return Err(err);
        }

        // Check for trailing characters
        while let Some(c) = peek() {
            if c.is_whitespace() {
                advance();
            } else if c == ')' {
                // zsh's specific wording for the unmatched-close
                // case: `bad math expression: unexpected ')'`.
                return Err("bad math expression: unexpected ')'".to_string());
            } else {
                return Err(format!("illegal character: {}", c));
            }
        }

        if m_stack_is_empty() {
            return Ok(Mnumber { l: 0, d: 0.0, type_: MN_INTEGER });
        }

        let mv = m_stack_pop().unwrap();
        let result = if (mv.val.type_ == MN_UNSET) {
            if let Some(ref name) = mv.lval {
                getmathparam(name)
            } else {
                Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
            }
        } else {
            mv.val
        };

        Ok(result)
    }

// WARNING: NOT IN MATH.C — Rust-only accessor (note plural — singular
// `getmathparam` IS in math.c:337). zsh C's caller reads the param
// table directly post-eval; this returns a snapshot of the in-memory
// variables map for ShellExecutor integration.
/// Get updated variables after evaluation
pub(crate) fn getmathparams() -> HashMap<String, Mnumber> {
    m_variables_clone()
}

/// Convenience function to evaluate a math expression
/// Top-level math-expression evaluator.
/// Port of `matheval(s)` from Src/math.c:1480 — wraps `mathevall()`\n/// (line 367) with the C source's standard error-message\n/// formatting.
pub fn matheval(s: &str) -> Result<Mnumber, String> {                     // c:1480
    new(s);
    mathevall()
}

/// Evaluate and return integer
/// Math evaluator that coerces the result to integer.
/// Port of `mathevali(s)` from Src/math.c:1505.
pub fn mathevali(s: &str) -> Result<i64, String> {                        // c:1505
    matheval(s).map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_arithmetic() {
        assert_eq!(mathevali("1 + 2").unwrap(), 3);
        assert_eq!(mathevali("10 - 3").unwrap(), 7);
        assert_eq!(mathevali("4 * 5").unwrap(), 20);
        assert_eq!(mathevali("20 / 4").unwrap(), 5);
        assert_eq!(mathevali("17 % 5").unwrap(), 2);
    }

    #[test]
    fn test_precedence() {
        assert_eq!(mathevali("2 + 3 * 4").unwrap(), 14);
        assert_eq!(mathevali("(2 + 3) * 4").unwrap(), 20);
        assert_eq!(mathevali("2 ** 3 ** 2").unwrap(), 512); // Right associative
    }

    #[test]
    fn test_comparison() {
        assert_eq!(mathevali("5 > 3").unwrap(), 1);
        assert_eq!(mathevali("5 < 3").unwrap(), 0);
        assert_eq!(mathevali("5 == 5").unwrap(), 1);
        assert_eq!(mathevali("5 != 3").unwrap(), 1);
        assert_eq!(mathevali("5 >= 5").unwrap(), 1);
        assert_eq!(mathevali("5 <= 5").unwrap(), 1);
    }

    #[test]
    fn test_logical() {
        assert_eq!(mathevali("1 && 1").unwrap(), 1);
        assert_eq!(mathevali("1 && 0").unwrap(), 0);
        assert_eq!(mathevali("1 || 0").unwrap(), 1);
        assert_eq!(mathevali("0 || 0").unwrap(), 0);
        assert_eq!(mathevali("!0").unwrap(), 1);
        assert_eq!(mathevali("!1").unwrap(), 0);
    }

    #[test]
    fn test_bitwise() {
        assert_eq!(mathevali("5 & 3").unwrap(), 1);
        assert_eq!(mathevali("5 | 3").unwrap(), 7);
        assert_eq!(mathevali("5 ^ 3").unwrap(), 6);
        assert_eq!(mathevali("~0").unwrap(), -1);
        assert_eq!(mathevali("1 << 4").unwrap(), 16);
        assert_eq!(mathevali("16 >> 2").unwrap(), 4);
    }

    #[test]
    fn test_ternary() {
        assert_eq!(mathevali("1 ? 10 : 20").unwrap(), 10);
        assert_eq!(mathevali("0 ? 10 : 20").unwrap(), 20);
        assert_eq!(mathevali("(5 > 3) ? 100 : 200").unwrap(), 100);
    }

    #[test]
    fn test_power() {
        assert_eq!(mathevali("2 ** 10").unwrap(), 1024);
        assert_eq!(mathevali("3 ** 3").unwrap(), 27);
        assert!((matheval("2.0 ** 0.5").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - std::f64::consts::SQRT_2).abs() < 0.0001);
    }

    #[test]
    fn test_float() {
        assert!((matheval("3.14 + 0.01").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - 3.15).abs() < 0.0001);
        assert!((matheval("1.5 * 2.0").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - 3.0).abs() < 0.0001);
    }

    #[test]
    fn test_unary() {
        assert_eq!(mathevali("-5").unwrap(), -5);
        assert_eq!(mathevali("- -5").unwrap(), 5); // space needed to avoid --
        assert_eq!(mathevali("+5").unwrap(), 5);
        assert_eq!(mathevali("-(-5)").unwrap(), 5);
    }

    #[test]
    fn test_base() {
        assert_eq!(mathevali("0xFF").unwrap(), 255);
        assert_eq!(mathevali("0b1010").unwrap(), 10);
        assert_eq!(mathevali("16#FF").unwrap(), 255);
        assert_eq!(mathevali("2#1010").unwrap(), 10);
        assert_eq!(mathevali("[16]FF").unwrap(), 255);
    }

    #[test]
    fn test_variables() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Mnumber { l: 10, d: 0.0, type_: MN_INTEGER });
        vars.insert("y".to_string(), Mnumber { l: 20, d: 0.0, type_: MN_INTEGER });

        new("x + y");
        with_variables(vars);
        assert_eq!(({ let __m = mathevall().unwrap(); if __m.type_ == MN_FLOAT { __m.d as i64 } else { __m.l } }), 30);
    }

    #[test]
    fn test_assignment() {
        new("x = 5");
        mathevall().unwrap();
        assert_eq!(({ let __m = m_variables_get("x").unwrap(); if __m.type_ == MN_FLOAT { __m.d as i64 } else { __m.l } }), 5);

        new("x = 5, x += 3");
        let result = mathevall().unwrap();
        assert_eq!((if result.type_ == MN_FLOAT { result.d as i64 } else { result.l }), 8);
    }

    #[test]
    fn test_increment() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Mnumber { l: 5, d: 0.0, type_: MN_INTEGER });

        new("++x");
        with_variables(vars.clone());
        assert_eq!(({ let __m = mathevall().unwrap(); if __m.type_ == MN_FLOAT { __m.d as i64 } else { __m.l } }), 6);
        assert_eq!(({ let __m = m_variables_get("x").unwrap(); if __m.type_ == MN_FLOAT { __m.d as i64 } else { __m.l } }), 6);

        new("x++");
        with_variables(vars.clone());
        assert_eq!(({ let __m = mathevall().unwrap(); if __m.type_ == MN_FLOAT { __m.d as i64 } else { __m.l } }), 5);
        assert_eq!(({ let __m = m_variables_get("x").unwrap(); if __m.type_ == MN_FLOAT { __m.d as i64 } else { __m.l } }), 6);
    }

    #[test]
    fn test_functions() {
        assert!((matheval("sqrt(4)").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - 2.0).abs() < 0.0001);
        assert!((matheval("sin(0)").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap()).abs() < 0.0001);
        assert!((matheval("cos(0)").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - 1.0).abs() < 0.0001);
        assert!((matheval("abs(-5)").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - 5.0).abs() < 0.0001);
        assert!((matheval("floor(3.7)").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - 3.0).abs() < 0.0001);
        assert!((matheval("ceil(3.2)").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap() - 4.0).abs() < 0.0001);
    }

    #[test]
    fn test_special_values() {
        assert!(matheval("Inf").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap().is_infinite());
        assert!(matheval("NaN").map(|n| (if n.type_ == MN_FLOAT { n.d } else { n.l as f64 })).unwrap().is_nan());
    }

    #[test]
    fn test_errors() {
        assert!(matheval("1 / 0").is_err());
        assert!(matheval("1 +").is_err());
        // Empty arith expression is a parse error in zsh:
        //   $ zsh -c '(( ))'; echo $?   →   1
        // The previous comment claimed "Empty parens are valid" — that
        // was wrong. Real zsh aborts with `bad math expression: empty
        // parentheses`; our matheval matches.
        assert!(matheval("()").is_err());
    }

    #[test]
    fn test_underscore_in_numbers() {
        assert_eq!(mathevali("1_000_000").unwrap(), 1000000);
        assert_eq!(mathevali("0xFF_FF").unwrap(), 65535);
    }

    #[test]
    fn test_comma_operator() {
        assert_eq!(mathevali("1, 2, 3").unwrap(), 3);
        assert_eq!(mathevali("(x = 1, y = 2, x + y)").unwrap(), 3);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: math
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Free fns moved verbatim from src/ported/exec.rs.
// ===========================================================
// BEGIN moved-from-exec-rs (free fns)
/// Pop argc arguments from the VM stack into a Vec<String>.
///
/// `Value::Array` entries (produced by `${arr[@]}`, glob expansion, brace
/// expansion, etc.) splice into multiple argv-style args — same flattening
/// rule as fusevm's `Op::Exec`. Without this, a builtin like `echo
/// ${arr[@]}` with `arr=(x y z)` would receive a single space-joined arg
/// `"x y z"` instead of three separate args.
/// Subscript-arith parser namespace. Holds the three pre-resolve parsers
/// `eval_arith_expr` runs against an expression before substituting array
/// references — the C source's `mathexpr()` (Src/math.c) inlines this work
/// inside the lexer, but Rust splits it out so the assignment-target arms
/// don't get confused with read sites.
// WARNING: NOT IN MATH.C — Rust-only string parser. C `setmathvar`
// (math.c:972) walks the lvalue pointer left in place by zzlex,
// so subscripted compound assigns fall out of the lexer for free.
// zshrs sees `((a[i]+=v))` as raw text and must split it before
// pre_resolve_array_subscripts substitutes the read value in place.
#[inline]
/// Detect `name[idx]=rhs` (or `name[idx]+=rhs`, etc.) at the start of
/// an arith expression. Returns (name, idx_expr, rhs). Used by
/// `eval_arith_expr` to handle `((a[i]=expr))` — the regular pre-
/// resolve pass would substitute a[i] with its current value first,
/// turning the expression into `0=42` which is invalid.
/// Parse `name[idx]OP rhs?` where OP is `++`, `--`, `+=`, `-=`, etc.
/// Returns (name, idx_expr, op, rhs). For `++`/`--`, rhs is empty.
pub(crate) fn parse_compound(expr: &str) -> Option<(String, String, String, String)> {
    let trimmed = expr.trim();
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = trimmed[..i].to_string();
    if i >= bytes.len() || bytes[i] != b'[' {
        return None;
    }
    let idx_start = i + 1;
    let mut depth = 1;
    let mut j = idx_start;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let idx_expr = trimmed[idx_start..j].to_string();
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() {
        return None;
    }
    let rest = &bytes[k..];
    // Try 3-char operators first (`<<=`, `>>=`, `**=`), then 2-char
    // (`++`, `--`, `+=`, `-=`, `*=`, `/=`, `%=`, `&=`, `|=`, `^=`).
    let (op, op_len) = match rest {
        [b'<', b'<', b'=', ..] => ("<<=", 3),
        [b'>', b'>', b'=', ..] => (">>=", 3),
        [b'*', b'*', b'=', ..] => ("**=", 3),
        [b'+', b'+', ..] => ("++", 2),
        [b'-', b'-', ..] => ("--", 2),
        [b'+', b'=', ..] => ("+=", 2),
        [b'-', b'=', ..] => ("-=", 2),
        [b'*', b'=', ..] => ("*=", 2),
        [b'/', b'=', ..] => ("/=", 2),
        [b'%', b'=', ..] => ("%=", 2),
        [b'&', b'=', ..] => ("&=", 2),
        [b'|', b'=', ..] => ("|=", 2),
        [b'^', b'=', ..] => ("^=", 2),
        _ => return None,
    };
    let rhs = trimmed[k + op_len..].trim().to_string();
    // For `++` / `--`, the rhs MUST be empty (anything else would be
    // a parse error). For `+=` etc., rhs is the value expression.
    if (op == "++" || op == "--") && !rhs.is_empty() {
        return None;
    }
    Some((name, idx_expr, op.to_string(), rhs))
}
// WARNING: NOT IN MATH.C — Rust-only string parser. C handles
// `++NAME[IDX]` via the lexer leaving the lvalue pointer set; the
// Rust port pre-parses the text. See parse_compound above.
/// Pre-increment/decrement on subscript: `++NAME[IDX]` / `--NAME[IDX]`.
/// Returns (name, idx_expr, op) where op is "++" or "--".
pub(crate) fn parse_pre_inc(expr: &str) -> Option<(String, String, String)> {
    let trimmed = expr.trim();
    let (after_op, pre_op) = if let Some(s) = trimmed.strip_prefix("++") {
        (s, "++")
    } else if let Some(s) = trimmed.strip_prefix("--") {
        (s, "--")
    } else {
        return None;
    };
    let after_op = after_op.trim_start();
    let bytes = after_op.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = after_op[..i].to_string();
    if i >= bytes.len() || bytes[i] != b'[' {
        return None;
    }
    let idx_start = i + 1;
    let mut depth = 1;
    let mut j = idx_start;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let idx_expr = after_op[idx_start..j].to_string();
    // After ], must be end of input (or whitespace).
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k != bytes.len() {
        return None;
    }
    Some((name, idx_expr, pre_op.to_string()))
}
// WARNING: NOT IN MATH.C — Rust-only string parser for `NAME[IDX]=v`.
// See parse_compound above for the rationale.
pub(crate) fn parse_assign(expr: &str) -> Option<(String, String, String)> {
    let trimmed = expr.trim();
    let bytes = trimmed.as_bytes();
    if bytes.is_empty() || !(bytes[0] == b'_' || bytes[0].is_ascii_alphabetic()) {
        return None;
    }
    let mut i = 1;
    while i < bytes.len() && (bytes[i] == b'_' || bytes[i].is_ascii_alphanumeric()) {
        i += 1;
    }
    let name = trimmed[..i].to_string();
    if i >= bytes.len() || bytes[i] != b'[' {
        return None;
    }
    let idx_start = i + 1;
    let mut depth = 1;
    let mut j = idx_start;
    while j < bytes.len() && depth > 0 {
        match bytes[j] {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if j >= bytes.len() {
        return None;
    }
    let idx_expr = trimmed[idx_start..j].to_string();
    // Skip ]
    let mut k = j + 1;
    while k < bytes.len() && bytes[k].is_ascii_whitespace() {
        k += 1;
    }
    if k >= bytes.len() || bytes[k] != b'=' {
        return None;
    }
    // Reject `==` and `=~` (comparison/regex, not assignment).
    if k + 1 < bytes.len() && (bytes[k + 1] == b'=' || bytes[k + 1] == b'~') {
        return None;
    }
    let rhs = trimmed[k + 1..].trim().to_string();
    Some((name, idx_expr, rhs))
}

// END moved-from-exec-rs (free fns)

// ===========================================================
// Numeric formatting helpers moved from src/ported/exec.rs.
// Mirror Src/math.c / Src/utils.c base+digit-grouping logic.
// ===========================================================


// WARNING: NOT IN MATH.C — `convbase` lives in `Src/params.c:5632`
// (called from math.c:1089). This file holds a duplicate that
// predates the params.rs port; canonical home is
// `crate::ported::params::convbase`. This entry is drift pending
// cleanup; do not add new callers — use `crate::ported::params::convbase`.
/// Format an integer in the given base (2-36) using zsh's
/// `BASE#DIGITS` form.
/// Port of `convbase(s, v, base)` from Src/utils.c (also called from
/// Src/math.c:1089). Bases 2-9 are unsigned-style; uppercase
/// A-Z are used for digits >= 10. A negative value is output
/// as `-BASE#DIGITS`.
pub fn convbase(n: i64, base: u32) -> String {                               // c:params.c:5632
    if !(2..=36).contains(&base) {
        return n.to_string();
    }
    if n == 0 {
        return format!("{}#0", base);
    }
    let neg = n < 0;
    let mut v: u64 = n.unsigned_abs();
    let mut digits = Vec::new();
    while v > 0 {
        let d = (v % base as u64) as u32;
        let ch = if d < 10 {
            (b'0' + d as u8) as char
        } else {
            (b'A' + (d - 10) as u8) as char
        };
        digits.push(ch);
        v /= base as u64;
    }
    digits.reverse();
    let body: String = digits.into_iter().collect();
    if neg {
        format!("-{}#{}", base, body)
    } else {
        format!("{}#{}", base, body)
    }
}

// ===========================================================
// Remaining stubs from Src/math.c that don't yet have a faithful
// implementation in the migrated free-fn evaluator. The
// in-place implementations (mathevall, getmathparam, lexconstant,
// setmathvar, callmathfunc, checkunary) replaced their stubs;
// the names below correspond to C helpers the evaluator uses
// internally below — bodies wire to existing Rust idioms while
// preserving the C name + citation.
// ===========================================================

/// Port of `isinf(x)` from Src/math.c:588 — IEEE +/-Infinity test.
/// Wraps Rust's `f64::is_infinite`.
pub(crate) fn isinf(x: f64) -> bool { x.is_infinite() }

/// Port of `isnan(x)` from Src/math.c:608 — IEEE NaN test. C
/// implements it as `store(&x) != store(&x)` to defeat compiler
/// folding of the canonical `x != x` NaN test; we route through
/// `store` for parity, but Rust's `f64::is_nan` is the
/// correctness path.
pub(crate) fn isnan(x: f64) -> bool { store(x) != store(x) || x.is_nan() }

/// Port of `notzero(a)` from Src/math.c:1142 — error-on-zero check
/// used by `/` and `%` operators. Returns true when `a` is non-
/// zero (caller continues), false when zero (caller raises
/// "division by zero"). Float zero is treated as non-zero per
/// IEEE 754 (1/0.0 → Inf, not an error) — only integer zero
/// trips the check, matching math.c's `if (!a.u.l) zerr(…)`.
pub(crate) fn notzero(a: Mnumber) -> bool {
    if (a.type_ == MN_UNSET) {
        return false;
    }
    if (a.type_ == MN_INTEGER) {
        return a.l != 0;
    }
    true
}

/// Port of `store(x)` from Src/math.c:601 — load/store a double
/// via a pointer to defeat compilers that mis-optimize the
/// canonical `x != x` NaN test. zsh only compiles this path when
/// `HAVE_ISNAN` is undefined; we keep it as a name-parity shim
/// so `isnan()` can route through it (matching the C source's
/// `store(&x) != store(&x)` idiom).
pub(crate) fn store(x: f64) -> f64 { x }

/// Port of `getcvar(s)` from Src/math.c:943 — character-constant
/// lookup. Reads the named shell variable and returns the
/// codepoint of its first character. Used for `#varname` token
/// (CId): `x="hello"; (( y = #x ))` puts 104 (`'h'`) into y.
/// On miss or empty value, returns 0 (matches zsh's `*s ? *s : 0`).
pub(crate) fn getcvar(name: &str) -> Mnumber {
    if let Some(raw) = m_string_variables_get(name) {
        return Mnumber { l: raw.chars().next().map(|c| c as i64).unwrap_or(0), d: 0.0, type_: MN_INTEGER };
    }
    if let Some(v) = m_variables_get(name) {
        let s = match v.type_ {
            MN_INTEGER => v.l.to_string(),
            MN_FLOAT => {
                let f = v.d;
                if isnan(f) { "NaN".to_string() }
                else if isinf(f) { if f > 0.0 { "Inf".to_string() } else { "-Inf".to_string() } }
                else { format!("{:.10}", f) }
            }
            _ => "0".to_string(),
        };
        return Mnumber { l: s.chars().next().map(|c| c as i64).unwrap_or(0), d: 0.0, type_: MN_INTEGER };
    }
    Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
}

/// Port of `mathevalarg(s, ss)` from Src/math.c:1514 — evaluate one
/// arg expression and return as integer. Used by `let` builtin
/// and others that take an arith-expr argument.
pub(crate) fn mathevalarg(expr: &str) -> i64 {
    matheval(expr).map(|n| (if n.type_ == MN_FLOAT { n.d as i64 } else { n.l })).unwrap_or(0)
}
