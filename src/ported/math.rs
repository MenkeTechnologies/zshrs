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

use std::collections::HashMap;
use crate::ported::utils::zerr;
use std::env;
use crate::ported::exec::{
    self,
};

/// `MN_INTEGER` from Src/zsh.h — `mnumber.type` tag value.
pub const MN_INTEGER: u32 = 1;
/// `MN_FLOAT` from Src/zsh.h — `mnumber.type` tag value.
pub const MN_FLOAT: u32 = 2;
/// `MN_UNSET` — sentinel used for math errors / NULL-return paths.
/// Not a canonical zsh.h constant; the C source uses a NULL value
/// pointer to signal the same condition.
pub const MN_UNSET: u32 = 4;

/// Port of `mnumber` from `Src/zsh.h` — the math evaluator's
/// unified value type. C definition:
///
/// ```c
/// typedef struct mnumber {
///     union { zlong l; double d; } u;
///     int type;
/// } mnumber;
/// ```
///
/// Rust port flattens the union into both fields (8-byte cost,
/// safety win) — `type_` selects which side is valid.
#[derive(Debug, Clone, Copy)]
pub struct Mnumber {
    pub l: i64,
    pub d: f64,
    pub type_: u32,
}

impl Default for Mnumber {
    fn default() -> Self {
        Mnumber { l: 0, d: 0.0, type_: MN_INTEGER }
    }
}

impl Mnumber {
    pub const fn integer(i: i64) -> Self {
        Mnumber { l: i, d: 0.0, type_: MN_INTEGER }
    }
    pub const fn float(f: f64) -> Self {
        Mnumber { l: 0, d: f, type_: MN_FLOAT }
    }
    pub const fn unset() -> Self {
        Mnumber { l: 0, d: 0.0, type_: MN_UNSET }
    }

    pub fn is_zero(&self) -> bool {
        match self.type_ {
            MN_INTEGER => self.l == 0,
            MN_FLOAT => self.d == 0.0,
            _ => true,
        }
    }

    pub fn to_int(&self) -> i64 {
        match self.type_ {
            MN_INTEGER => self.l,
            MN_FLOAT => self.d as i64,
            _ => 0,
        }
    }

    pub fn to_float(&self) -> f64 {
        match self.type_ {
            MN_INTEGER => self.l as f64,
            MN_FLOAT => self.d,
            _ => 0.0,
        }
    }

    pub fn is_float(&self) -> bool { self.type_ == MN_FLOAT }
    pub fn is_integer(&self) -> bool { self.type_ == MN_INTEGER }
    pub fn is_unset(&self) -> bool { self.type_ == MN_UNSET }

    /// Format for stored variable values (zsh `let` / `(( a=… ))`):
    /// Integers print plain; floats use `%.10f`. IEEE specials
    /// (Inf/-Inf/NaN) get capitalized form.
    pub fn format_zsh(&self) -> String {
        match self.type_ {
            MN_INTEGER => self.l.to_string(),
            MN_FLOAT => {
                let f = self.d;
                if isnan(f) {
                    "NaN".to_string()
                } else if isinf(f) {
                    if f > 0.0 {
                        "Inf".to_string()
                    } else {
                        "-Inf".to_string()
                    }
                } else {
                    format!("{:.10}", f)
                }
            }
            _ => "0".to_string(),
        }
    }

    /// Format for `$(( ))` arithmetic substitution display.
    pub fn format_zsh_subst(&self) -> String {
        match self.type_ {
            MN_INTEGER => self.l.to_string(),
            MN_FLOAT => crate::ported::params::convfloat(self.d, 0, 0),
            _ => "0".to_string(),
        }
    }
}

/// Math tokens - from math.c
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum MathTok {
    InPar = 0,      // (
    OutPar = 1,     // )
    Not = 2,        // !
    Comp = 3,       // ~
    PostPlus = 4,   // x++
    PostMinus = 5,  // x--
    UPlus = 6,      // +x
    UMinus = 7,     // -x
    And = 8,        // &
    Xor = 9,        // ^
    Or = 10,        // |
    Mul = 11,       // *
    Div = 12,       // /
    Mod = 13,       // %
    Plus = 14,      // +
    Minus = 15,     // -
    ShLeft = 16,    // <<
    ShRight = 17,   // >>
    Les = 18,       // <
    Leq = 19,       // <=
    Gre = 20,       // >
    Geq = 21,       // >=
    Deq = 22,       // ==
    Neq = 23,       // !=
    DAnd = 24,      // &&
    DOr = 25,       // ||
    DXor = 26,      // ^^
    Quest = 27,     // ?
    Colon = 28,     // :
    Eq = 29,        // =
    PlusEq = 30,    // +=
    MinusEq = 31,   // -=
    MulEq = 32,     // *=
    DivEq = 33,     // /=
    ModEq = 34,     // %=
    AndEq = 35,     // &=
    XorEq = 36,     // ^=
    OrEq = 37,      // |=
    ShLeftEq = 38,  // <<=
    ShRightEq = 39, // >>=
    DAndEq = 40,    // &&=
    DOrEq = 41,     // ||=
    DXorEq = 42,    // ^^=
    Comma = 43,     // ,
    Eoi = 44,       // end of input
    PrePlus = 45,   // ++x
    PreMinus = 46,  // --x
    Num = 47,       // number literal
    Id = 48,        // identifier
    Power = 49,     // **
    CId = 50,       // #identifier (char value)
    PowerEq = 51,   // **=
    Func = 52,      // function call
}

const TOKCOUNT: usize = 53;

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
            val: Mnumber::integer(0),
            lval: None,
            pval: (),
        }
    }
}

/// Math evaluator state.
/// Port of the per-evaluation locals `mathevall()` (Src/math.c:367)\n/// keeps — input cursor, operator stack, value stack. Drives\n/// `zzlex()` (line 617), `push()` / `pop()` (lines 916/931), and\n/// `op()` / `bop()` (lines 1154/1454).
pub struct MathState<'a> {
    input: &'a str,
    pos: usize,
    /// Byte position in `input` where the most recently lexed token began
    /// (after whitespace skip). Used to format zsh-style error pointers
    /// like `bad math expression: operand expected at `*'` for orphan
    /// binary operators — zsh's error retains the input pointer at the
    /// start of the bad operator, so the message includes the operator
    /// (and any trailing input) rather than the post-consumption position.
    tok_start: usize,
    yyval: Mnumber,
    yylval: String,
    stack: Vec<MathValue>,
    mtok: MathTok,
    unary: bool,
    noeval: i32,
    lastbase: i32,
    prec: &'static [u8; TOKCOUNT],
    c_precedences: bool,
    force_float: bool,
    octal_zeroes: bool,
    variables: HashMap<String, Mnumber>,
    /// Raw string values for variables whose contents aren't a plain number.
    /// zsh recursively evaluates these as arith expressions on lookup so
    /// `a="3+2"; $((a))` produces 5. Without this, MathState saw `a` as
    /// unset → 0.
    string_variables: HashMap<String, String>,
    lastval: i32,
    pid: i64,
    error: Option<String>,
}

pub fn new<'a>(input: &'a str) -> MathState<'a> {
        MathState {
            input,
            pos: 0,
            tok_start: 0,
            yyval: Mnumber::integer(0),
            yylval: String::new(),
            stack: Vec::with_capacity(100),
            mtok: MathTok::Eoi,
            unary: true,
            noeval: 0,
            lastbase: -1,
            prec: &Z_PREC,
            c_precedences: false,
            force_float: false,
            octal_zeroes: false,
            variables: HashMap::new(),
            string_variables: HashMap::new(),
            lastval: 0,
            pid: std::process::id() as i64,
            error: None,
        }
    }

    pub fn with_variables<'a>(mut s: MathState<'a>, vars: HashMap<String, Mnumber>) -> MathState<'a> {
        s.variables = vars;
        s
    }

    /// Inject variables from string->string mapping (for shell integration)
    pub fn with_string_variables<'a>(mut s: MathState<'a>, vars: &HashMap<String, String>) -> MathState<'a> {
        for (k, v) in vars {
            if let Ok(i) = v.parse::<i64>() {
                s.variables.insert(k.clone(), Mnumber::integer(i));
            } else if let Ok(f) = v.parse::<f64>() {
                s.variables.insert(k.clone(), Mnumber::float(f));
            } else if !v.is_empty() {
                // Non-numeric string — keep raw so getmathparam can
                // recursively evaluate it as an arith expression.
                // zsh: `a="3+2"; $((a))` returns 5.
                s.string_variables.insert(k.clone(), v.clone());
            }
        }
        s
    }

    /// Extract modified variables as string->string mapping (for shell integration)
    pub fn extract_string_variables<'a>(s: &MathState<'a>) -> HashMap<String, String> {
        s.variables
            .iter()
            .map(|(k, v)| (k.clone(), v.format_zsh()))
            .collect()
    }

    pub fn with_c_precedences<'a>(mut s: MathState<'a>, enable: bool) -> MathState<'a> {
        s.c_precedences = enable;
        s.prec = if enable { &C_PREC } else { &Z_PREC };
        s
    }

    pub fn with_force_float<'a>(mut s: MathState<'a>, enable: bool) -> MathState<'a> {
        s.force_float = enable;
        s
    }

    pub fn with_octal_zeroes<'a>(mut s: MathState<'a>, enable: bool) -> MathState<'a> {
        s.octal_zeroes = enable;
        s
    }

    pub fn with_lastval<'a>(mut s: MathState<'a>, val: i32) -> MathState<'a> {
        s.lastval = val;
        s
    }

    pub fn peek(s: &MathState<'_>) -> Option<char> {
        s.input[s.pos..].chars().next()
    }

    pub fn advance(s: &mut MathState<'_>) -> Option<char> {
        let c = peek(s)?;
        s.pos += c.len_utf8();
        Some(c)
    }

    fn is_digit(c: char) -> bool {
        c.is_ascii_digit()
    }

    fn is_ident_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_'
    }

    fn is_ident(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_'
    }

/// Port of `lexconstant()` from `Src/math.c:462`.
///
/// Lex a numeric constant — decimal/hex/binary/octal integer or
/// floating-point literal. Sets `s.yyval` and returns
/// `MathTok::Num`. Recognises `0x`/`0b` prefixes, base-prefix
/// (`16#FF`), trailing-dot float, scientific notation, and zsh's
/// underscore digit-grouping. Mirrors C's `zstrtol_underscore()`
/// for greedy base parsing (consume valid digits only, leave the
/// rest as the next token).
pub(crate) fn lexconstant(s: &mut MathState<'_>) -> MathTok {
        let _start = s.pos;
        let mut is_neg = false;

        // Handle leading minus for unary context
        if peek(s) == Some('-') {
            is_neg = true;
            advance(s);
        }

        // Check for hex/binary/octal
        if peek(s) == Some('0') {
            advance(s);
            match peek(s).map(|c| c.to_ascii_lowercase()) {
                Some('x') => {
                    // Hex: 0xFF
                    advance(s);
                    let hex_start = s.pos;
                    while let Some(c) = peek(s) {
                        if c.is_ascii_hexdigit() || c == '_' {
                            advance(s);
                        } else {
                            break;
                        }
                    }
                    let hex_str: String = s.input[hex_start..s.pos]
                        .chars()
                        .filter(|&c| c != '_')
                        .collect();
                    let val = i64::from_str_radix(&hex_str, 16).unwrap_or(0);
                    s.lastbase = 16;
                    s.yyval = if s.force_float {
                        Mnumber::float(if is_neg { -(val as f64) } else { val as f64 })
                    } else {
                        Mnumber::integer(if is_neg { -val } else { val })
                    };
                    return MathTok::Num;
                }
                Some('b') => {
                    // Binary: 0b1010
                    advance(s);
                    let bin_start = s.pos;
                    while let Some(c) = peek(s) {
                        if c == '0' || c == '1' || c == '_' {
                            advance(s);
                        } else {
                            break;
                        }
                    }
                    let bin_str: String = s.input[bin_start..s.pos]
                        .chars()
                        .filter(|&c| c != '_')
                        .collect();
                    let val = i64::from_str_radix(&bin_str, 2).unwrap_or(0);
                    s.lastbase = 2;
                    s.yyval = if s.force_float {
                        Mnumber::float(if is_neg { -(val as f64) } else { val as f64 })
                    } else {
                        Mnumber::integer(if is_neg { -val } else { val })
                    };
                    return MathTok::Num;
                }
                Some('o') | Some('O') => {
                    // zsh rejects `0o…` octal-prefix (Rust/Python form).
                    // Only `0x` (hex), `0b` (binary), and bare-leading-0
                    // (with `setopt octalzeroes`) are recognized. Emit
                    // the same diagnostic zsh produces — set s.error
                    // and return a stub Num so the caller's
                    // error-propagation path picks up the failure.
                    s.error = Some(format!(
                        "bad math expression: operator expected at `{}'",
                        &s.input[s.pos..]
                    ));
                    s.yyval = Mnumber::integer(0);
                    return MathTok::Num;
                }
                _ => {
                    // Could be octal or just 0
                    if s.octal_zeroes {
                        // Check if this looks like octal
                        let oct_start = s.pos;
                        let mut is_octal = true;
                        while let Some(c) = peek(s) {
                            if c.is_ascii_digit() || c == '_' {
                                if ('8'..='9').contains(&c) {
                                    is_octal = false;
                                }
                                advance(s);
                            } else if c == '.' || c == 'e' || c == 'E' || c == '#' {
                                is_octal = false;
                                break;
                            } else {
                                break;
                            }
                        }
                        if is_octal && s.pos > oct_start {
                            let oct_str: String = s.input[oct_start..s.pos]
                                .chars()
                                .filter(|&c| c != '_')
                                .collect();
                            let val = i64::from_str_radix(&oct_str, 8).unwrap_or(0);
                            s.lastbase = 8;
                            s.yyval = if s.force_float {
                                Mnumber::float(if is_neg { -(val as f64) } else { val as f64 })
                            } else {
                                Mnumber::integer(if is_neg { -val } else { val })
                            };
                            return MathTok::Num;
                        }
                        s.pos = oct_start;
                    }
                    // Put back the 0
                    s.pos -= 1;
                }
            }
        }

        // Parse decimal integer or float
        let num_start = s.pos;
        while let Some(c) = peek(s) {
            if is_digit(c) || c == '_' {
                advance(s);
            } else {
                break;
            }
        }

        // Check for float
        if peek(s) == Some('.') || peek(s) == Some('e') || peek(s) == Some('E') {
            // Float
            if peek(s) == Some('.') {
                advance(s);
                while let Some(c) = peek(s) {
                    if is_digit(c) || c == '_' {
                        advance(s);
                    } else {
                        break;
                    }
                }
            }
            if peek(s) == Some('e') || peek(s) == Some('E') {
                advance(s);
                if peek(s) == Some('+') || peek(s) == Some('-') {
                    advance(s);
                }
                while let Some(c) = peek(s) {
                    if is_digit(c) || c == '_' {
                        advance(s);
                    } else {
                        break;
                    }
                }
            }
            let float_str: String = s.input[num_start..s.pos]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let val: f64 = float_str.parse().unwrap_or(0.0);
            s.yyval = Mnumber::float(if is_neg { -val } else { val });
            return MathTok::Num;
        }

        // Check for base#value syntax (e.g., 16#FF)
        if peek(s) == Some('#') {
            advance(s);
            let base_str: String = s.input[num_start..s.pos - 1]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let base: u32 = base_str.parse().unwrap_or(10);
            // zsh: `1#X` errors with "invalid base (must be 2 to 36 inclusive)".
            // i64::from_str_radix panics on out-of-range base; reject early.
            if !(2..=36).contains(&base) {
                s.error = Some(format!(
                    "invalid base (must be 2 to 36 inclusive): {}",
                    base
                ));
                s.yyval = Mnumber::integer(0);
                return MathTok::Num;
            }
            s.lastbase = base as i32;

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
            while let Some(c) = peek(s) {
                if c == '_' {
                    advance(s);
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
                advance(s);
            }
            s.yyval = if s.force_float {
                Mnumber::float(if is_neg { -(val as f64) } else { val as f64 })
            } else {
                Mnumber::integer(if is_neg { -val } else { val })
            };
            return MathTok::Num;
        }

        // Plain integer
        let int_str: String = s.input[num_start..s.pos]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let val: i64 = int_str.parse().unwrap_or(0);
        s.yyval = if s.force_float {
            Mnumber::float(if is_neg { -(val as f64) } else { val as f64 })
        } else {
            Mnumber::integer(if is_neg { -val } else { val })
        };
        MathTok::Num
    }

/// Port of `zzlex()` from `Src/math.c:617`.
///
/// Main math-expression lexer — returns the next token, advancing
/// `s.pos` and updating `s.yyval` / `s.yylval` as side-effects.
/// Handles all operators, ident lookahead for `Func` vs `Id`,
/// `[base]value` / `[#base]EXPR` output-radix prefixes, char
/// constants (`#x`, `##varname`), and dispatches numeric literals
/// to `lexconstant()`.
pub(crate) fn zzlex(s: &mut MathState<'_>) -> MathTok {
        s.yyval = Mnumber::integer(0);

        loop {
            let pre_pos = s.pos;
            let c = match advance(s) {
                Some(c) => c,
                None => {
                    s.tok_start = pre_pos;
                    return MathTok::Eoi;
                }
            };

            if matches!(c, ' ' | '\t' | '\n' | '"') {
                continue;
            }
            // Record where this token began (post-whitespace) so error
            // formatters can produce zsh-style "at `<remaining>`" messages.
            s.tok_start = pre_pos;

            match c {
                '+' => {
                    if peek(s) == Some('+') {
                        advance(s);
                        return if s.unary {
                            MathTok::PrePlus
                        } else {
                            MathTok::PostPlus
                        };
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::PlusEq;
                    }
                    return if s.unary {
                        MathTok::UPlus
                    } else {
                        MathTok::Plus
                    };
                }

                '-' => {
                    if peek(s) == Some('-') {
                        advance(s);
                        return if s.unary {
                            MathTok::PreMinus
                        } else {
                            MathTok::PostMinus
                        };
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::MinusEq;
                    }
                    if s.unary {
                        // Check if followed by digit for negative number
                        if let Some(next) = peek(s) {
                            if is_digit(next) || next == '.' {
                                s.pos -= 1; // Put back the -
                                return lexconstant(s);
                            }
                        }
                        return MathTok::UMinus;
                    }
                    return MathTok::Minus;
                }

                '(' => return MathTok::InPar,
                ')' => return MathTok::OutPar,

                '!' => {
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::Neq;
                    }
                    return MathTok::Not;
                }

                '~' => return MathTok::Comp,

                '&' => {
                    if peek(s) == Some('&') {
                        advance(s);
                        if peek(s) == Some('=') {
                            advance(s);
                            return MathTok::DAndEq;
                        }
                        return MathTok::DAnd;
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::AndEq;
                    }
                    return MathTok::And;
                }

                '|' => {
                    if peek(s) == Some('|') {
                        advance(s);
                        if peek(s) == Some('=') {
                            advance(s);
                            return MathTok::DOrEq;
                        }
                        return MathTok::DOr;
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::OrEq;
                    }
                    return MathTok::Or;
                }

                '^' => {
                    if peek(s) == Some('^') {
                        advance(s);
                        if peek(s) == Some('=') {
                            advance(s);
                            return MathTok::DXorEq;
                        }
                        return MathTok::DXor;
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::XorEq;
                    }
                    return MathTok::Xor;
                }

                '*' => {
                    if peek(s) == Some('*') {
                        advance(s);
                        if peek(s) == Some('=') {
                            advance(s);
                            return MathTok::PowerEq;
                        }
                        return MathTok::Power;
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::MulEq;
                    }
                    return MathTok::Mul;
                }

                '/' => {
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::DivEq;
                    }
                    return MathTok::Div;
                }

                '%' => {
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::ModEq;
                    }
                    return MathTok::Mod;
                }

                '<' => {
                    if peek(s) == Some('<') {
                        advance(s);
                        if peek(s) == Some('=') {
                            advance(s);
                            return MathTok::ShLeftEq;
                        }
                        return MathTok::ShLeft;
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::Leq;
                    }
                    return MathTok::Les;
                }

                '>' => {
                    if peek(s) == Some('>') {
                        advance(s);
                        if peek(s) == Some('=') {
                            advance(s);
                            return MathTok::ShRightEq;
                        }
                        return MathTok::ShRight;
                    }
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::Geq;
                    }
                    return MathTok::Gre;
                }

                '=' => {
                    if peek(s) == Some('=') {
                        advance(s);
                        return MathTok::Deq;
                    }
                    return MathTok::Eq;
                }

                '$' => {
                    // $$ = pid
                    s.yyval = Mnumber::integer(s.pid);
                    return MathTok::Num;
                }

                '?' => {
                    if s.unary {
                        // $? = lastval
                        s.yyval = Mnumber::integer(s.lastval as i64);
                        return MathTok::Num;
                    }
                    return MathTok::Quest;
                }

                ':' => return MathTok::Colon,
                ',' => return MathTok::Comma,

                '[' => {
                    // [base]value or output format [#base]
                    if is_digit(peek(s).unwrap_or('\0')) {
                        // [base]value
                        let base_start = s.pos;
                        while let Some(c) = peek(s) {
                            if is_digit(c) {
                                advance(s);
                            } else {
                                break;
                            }
                        }
                        if peek(s) != Some(']') {
                            s.error = Some("bad base syntax".to_string());
                            return MathTok::Eoi;
                        }
                        let base_str: String = s.input[base_start..s.pos].to_string();
                        let base: u32 = base_str.parse().unwrap_or(10);
                        advance(s); // skip ]

                        if !is_digit(peek(s).unwrap_or('\0'))
                            && !is_ident_start(peek(s).unwrap_or('\0'))
                        {
                            s.error = Some("bad base syntax".to_string());
                            return MathTok::Eoi;
                        }
                        // Reject out-of-range bases; from_str_radix panics
                        // on bases outside [2, 36].
                        if !(2..=36).contains(&base) {
                            s.error = Some(format!(
                                "invalid base (must be 2 to 36 inclusive): {}",
                                base
                            ));
                            s.yyval = Mnumber::integer(0);
                            return MathTok::Num;
                        }

                        let val_start = s.pos;
                        while let Some(c) = peek(s) {
                            if c.is_ascii_alphanumeric() {
                                advance(s);
                            } else {
                                break;
                            }
                        }
                        let val_str = &s.input[val_start..s.pos];
                        let val = i64::from_str_radix(val_str, base).unwrap_or(0);
                        s.lastbase = base as i32;
                        s.yyval = Mnumber::integer(val);
                        return MathTok::Num;
                    }
                    // Output format specifier [#base] - skip for now
                    if peek(s) == Some('#') {
                        while let Some(c) = peek(s) {
                            if c == ']' {
                                advance(s);
                                break;
                            }
                            advance(s);
                        }
                        continue;
                    }
                    s.error = Some("bad output format specification".to_string());
                    return MathTok::Eoi;
                }

                '#' => {
                    // Character code: #\x or ##string
                    if peek(s) == Some('\\') || peek(s) == Some('#') {
                        advance(s);
                        if let Some(ch) = advance(s) {
                            s.yyval = Mnumber::integer(ch as i64);
                            return MathTok::Num;
                        }
                    }
                    // #varname - get first char value
                    let id_start = s.pos;
                    while let Some(c) = peek(s) {
                        if is_ident(c) {
                            advance(s);
                        } else {
                            break;
                        }
                    }
                    if s.pos > id_start {
                        s.yylval = s.input[id_start..s.pos].to_string();
                        return MathTok::CId;
                    }
                    continue;
                }

                _ => {
                    if is_digit(c)
                        || (c == '.' && is_digit(peek(s).unwrap_or('\0')))
                    {
                        s.pos -= c.len_utf8();
                        return lexconstant(s);
                    }

                    if is_ident_start(c) {
                        let id_start = s.pos - c.len_utf8();
                        while let Some(c) = peek(s) {
                            if is_ident(c) {
                                advance(s);
                            } else {
                                break;
                            }
                        }

                        let id = &s.input[id_start..s.pos];

                        // Check for Inf/NaN
                        let id_lower = id.to_lowercase();
                        if id_lower == "nan" {
                            s.yyval = Mnumber::float(f64::NAN);
                            return MathTok::Num;
                        }
                        if id_lower == "inf" {
                            s.yyval = Mnumber::float(f64::INFINITY);
                            return MathTok::Num;
                        }

                        // Check for function call
                        if peek(s) == Some('(') {
                            // Skip to closing paren
                            let func_start = id_start;
                            advance(s); // (
                            let mut depth = 1;
                            while let Some(c) = peek(s) {
                                advance(s);
                                if c == '(' {
                                    depth += 1;
                                } else if c == ')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                            s.yylval = s.input[func_start..s.pos].to_string();
                            return MathTok::Func;
                        }

                        // Check for array subscript
                        if peek(s) == Some('[') {
                            advance(s); // [
                            let mut depth = 1;
                            while let Some(c) = peek(s) {
                                advance(s);
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

                        s.yylval = s.input[id_start..s.pos].to_string();
                        return MathTok::Id;
                    }

                    return MathTok::Eoi;
                }
            }
        }
    }

/// Port of `push()` from `Src/math.c:916`.
///
/// Push a value onto the evaluator's operand stack, with the
/// optional lvalue name (set when the value came from a variable
/// reference; needed for `++`/`--`/assignment-op write-back).
pub fn push(s: &mut MathState<'_>, val: Mnumber, lval: Option<String>) {
    s.stack.push(MathValue { val, lval, pval: () });
}

/// Port of `pop()` from `Src/math.c:931`.
///
/// Pop the top operand from the stack, resolving any deferred
/// variable read (`Mnumber::unset()` + lval set). The C source
/// passes a `noget` flag to skip the resolution; the Rust port
/// always resolves since callers that want the raw lvalue use
/// `pop_with_lval` instead.
pub fn pop(s: &mut MathState<'_>) -> Mnumber {
    if let Some(mv) = s.stack.pop() {
        if mv.val.is_unset() {
            if let Some(ref name) = mv.lval {
                return getmathparam(s, name);
            }
        }
        mv.val
    } else {
        s.error = Some("stack underflow".to_string());
        Mnumber::integer(0)
    }
    }

    pub(crate) fn pop_with_lval(s: &mut MathState<'_>) -> MathValue {
        s.stack.pop().unwrap_or_default()
    }

    pub(crate) fn get_value(s: &MathState<'_>, mv: &MathValue) -> Mnumber {
        if mv.val.is_unset() {
            if let Some(ref name) = mv.lval {
                return getmathparam(s, name);
            }
        }
        mv.val
    }

/// Port of `getmathparam()` from `Src/math.c:337`.
///
/// Look up a parameter by name from inside math context. zsh
/// auto-typesets a missing-but-referenced name (its mathparam
/// flag), but the Rust port keeps the variables map separate from
/// the param table so a miss returns `Integer(0)` and skips the
/// type-coercion. Indirect-string mode (`a="3+2"; $((a))`) is
/// handled by recursively evaluating the string value.
pub fn getmathparam(s: &MathState<'_>, name: &str) -> Mnumber {
    // Strip array subscript if present
        let base_name = if let Some(bracket) = name.find('[') {
            &name[..bracket]
        } else {
            name
        };
        if let Some(v) = s.variables.get(base_name).copied() {
            return v;
        }
        // Recursive eval: if the var holds a non-numeric string, evaluate
        // it AS an arith expression. zsh: `a="3+2"; $((a))` → 5. Bound
        // to one level of indirection — fresh evaluator each call so we
        // don't accidentally pollute s.variables.
        if let Some(raw) = s.string_variables.get(base_name) {
            let mut sub = new(raw);
            sub.variables = s.variables.clone();
            sub.string_variables = s.string_variables.clone();
            // Avoid infinite recursion: drop our own entry so a self-
            // referential `a=a` short-circuits to 0 rather than looping.
            sub.string_variables.remove(base_name);
            sub.prec = s.prec;
            sub.c_precedences = s.c_precedences;
            if let Ok(result) = mathevall(&mut sub) {
                return result;
            }
        }
        Mnumber::integer(0)
    }

/// Port of `setmathvar()` from `Src/math.c:972`.
///
/// Write `val` to the named parameter from inside math context.
/// Subscripted writes (`a[i] = …`) are pre-handled by the
/// SubscriptArith free fns higher up the call chain; this stub
/// only handles the scalar case. Returns the stored value so
/// `op` can leave it on the stack.
pub fn setmathvar(s: &mut MathState<'_>, name: &str, val: Mnumber) -> Mnumber {
    let base_name = if let Some(bracket) = name.find('[') {
        &name[..bracket]
    } else {
        name
    };
    s.variables.insert(base_name.to_string(), val);
    val
}

/// Port of `op()` from `Src/math.c:1154`.
///
/// Apply a binary or unary operator to the operand stack. Pops
/// 1-2 values, applies the operation (with type coercion), and
/// pushes the result. Handles assignment (`OP_E2*` flag) by
/// writing through `setmathvar` and pushing the new value back
/// with the same lvalue so chained assigns work.
pub(crate) fn op(s: &mut MathState<'_>, what: MathTok) {
        if s.error.is_some() {
            return;
        }

        let tp = OP_TYPE[what as usize];

        // Binary operators
        if (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0 {
            if s.stack.len() < 2 {
                // zsh's exact wording for the same condition is
                // `bad math expression: operand expected at end of
                // string`. Matching it here means `let "1+"` and
                // `$((5+))` produce the same diagnostic shape that
                // scripts grep for.
                s.error =
                    Some("bad math expression: operand expected at end of string".to_string());
                return;
            }

            let b = pop(s);
            let mv_a = pop_with_lval(s);
            let a = if mv_a.val.is_unset() {
                if let Some(ref name) = mv_a.lval {
                    getmathparam(s, name)
                } else {
                    Mnumber::integer(0)
                }
            } else {
                mv_a.val
            };

            // Coerce types
            let (a, b) = if (tp & (OP_A2IO | OP_E2IO)) != 0 {
                // Must be integers
                (Mnumber::integer(a.to_int()), Mnumber::integer(b.to_int()))
            } else if a.is_float() != b.is_float() && what != MathTok::Comma {
                // Different types, coerce to float
                (Mnumber::float(a.to_float()), Mnumber::float(b.to_float()))
            } else {
                (a, b)
            };

            let result = if s.noeval > 0 {
                Mnumber::integer(0)
            } else {
                let is_float = a.is_float();
                match what {
                    MathTok::And | MathTok::AndEq => Mnumber::integer(a.to_int() & b.to_int()),
                    MathTok::Xor | MathTok::XorEq => Mnumber::integer(a.to_int() ^ b.to_int()),
                    MathTok::Or | MathTok::OrEq => Mnumber::integer(a.to_int() | b.to_int()),

                    MathTok::Mul | MathTok::MulEq => {
                        if is_float {
                            Mnumber::float(a.to_float() * b.to_float())
                        } else {
                            Mnumber::integer(a.to_int().wrapping_mul(b.to_int()))
                        }
                    }

                    MathTok::Div | MathTok::DivEq => {
                        // Float div-by-zero is NOT an error in zsh —
                        // it produces IEEE Inf/-Inf/NaN per IEEE 754.
                        // Only INTEGER div-by-zero raises the error.
                        // Without this gate `1/0.0` errored out instead
                        // of returning `Inf`.
                        if is_float {
                            // Let f64 semantics handle 0.0, -0.0, NaN.
                            Mnumber::float(a.to_float() / b.to_float())
                        } else {
                            if !notzero(b) {
                                s.error = Some("division by zero".to_string());
                                return;
                            }
                            let bi = b.to_int();
                            if bi == -1 {
                                Mnumber::integer(a.to_int().wrapping_neg())
                            } else {
                                Mnumber::integer(a.to_int() / bi)
                            }
                        }
                    }

                    MathTok::Mod | MathTok::ModEq => {
                        if is_float {
                            // float % 0.0 → NaN per IEEE; let it fall
                            // through to f64 semantics rather than
                            // raising the integer-only error.
                            Mnumber::float(a.to_float() % b.to_float())
                        } else if !notzero(b) {
                            s.error = Some("division by zero".to_string());
                            return;
                        } else {
                            let bi = b.to_int();
                            if bi == -1 {
                                Mnumber::integer(0)
                            } else {
                                Mnumber::integer(a.to_int() % bi)
                            }
                        }
                    }

                    MathTok::Plus | MathTok::PlusEq => {
                        if is_float {
                            Mnumber::float(a.to_float() + b.to_float())
                        } else {
                            Mnumber::integer(a.to_int().wrapping_add(b.to_int()))
                        }
                    }

                    MathTok::Minus | MathTok::MinusEq => {
                        if is_float {
                            Mnumber::float(a.to_float() - b.to_float())
                        } else {
                            Mnumber::integer(a.to_int().wrapping_sub(b.to_int()))
                        }
                    }

                    MathTok::ShLeft | MathTok::ShLeftEq => {
                        Mnumber::integer(a.to_int() << (b.to_int() as u32 & 63))
                    }
                    MathTok::ShRight | MathTok::ShRightEq => {
                        Mnumber::integer(a.to_int() >> (b.to_int() as u32 & 63))
                    }

                    MathTok::Les => Mnumber::integer(if is_float {
                        (a.to_float() < b.to_float()) as i64
                    } else {
                        (a.to_int() < b.to_int()) as i64
                    }),
                    MathTok::Leq => Mnumber::integer(if is_float {
                        (a.to_float() <= b.to_float()) as i64
                    } else {
                        (a.to_int() <= b.to_int()) as i64
                    }),
                    MathTok::Gre => Mnumber::integer(if is_float {
                        (a.to_float() > b.to_float()) as i64
                    } else {
                        (a.to_int() > b.to_int()) as i64
                    }),
                    MathTok::Geq => Mnumber::integer(if is_float {
                        (a.to_float() >= b.to_float()) as i64
                    } else {
                        (a.to_int() >= b.to_int()) as i64
                    }),
                    MathTok::Deq => Mnumber::integer(if is_float {
                        (a.to_float() == b.to_float()) as i64
                    } else {
                        (a.to_int() == b.to_int()) as i64
                    }),
                    MathTok::Neq => Mnumber::integer(if is_float {
                        (a.to_float() != b.to_float()) as i64
                    } else {
                        (a.to_int() != b.to_int()) as i64
                    }),

                    MathTok::DAnd | MathTok::DAndEq => {
                        Mnumber::integer((a.to_int() != 0 && b.to_int() != 0) as i64)
                    }
                    MathTok::DOr | MathTok::DOrEq => {
                        Mnumber::integer((a.to_int() != 0 || b.to_int() != 0) as i64)
                    }
                    MathTok::DXor | MathTok::DXorEq => {
                        let ai = a.to_int() != 0;
                        let bi = b.to_int() != 0;
                        Mnumber::integer((ai != bi) as i64)
                    }

                    MathTok::Power | MathTok::PowerEq => {
                        let bi = b.to_int();
                        if !is_float && bi >= 0 {
                            let mut result = 1i64;
                            let base = a.to_int();
                            for _ in 0..bi {
                                result = result.wrapping_mul(base);
                            }
                            Mnumber::integer(result)
                        } else {
                            let af = a.to_float();
                            let bf = b.to_float();
                            if bf <= 0.0 && af == 0.0 {
                                s.error = Some("division by zero".to_string());
                                return;
                            }
                            if af < 0.0 && bf != bf.trunc() {
                                s.error = Some("imaginary power".to_string());
                                return;
                            }
                            Mnumber::float(af.powf(bf))
                        }
                    }

                    MathTok::Comma => b,
                    MathTok::Eq => b,

                    _ => Mnumber::integer(0),
                }
            };

            // Handle assignment
            if (tp & (OP_E2 | OP_E2IO)) != 0 {
                if let Some(ref name) = mv_a.lval {
                    let final_val = setmathvar(s, name, result);
                    push(s, final_val, Some(name.clone()));
                } else {
                    s.error = Some("lvalue required".to_string());
                    push(s, Mnumber::integer(0), None);
                }
            } else {
                push(s, result, None);
            }
            return;
        }

        // Unary operators
        if s.stack.is_empty() {
            // zsh: unary op with empty stack -> `bad math
            // expression: operand expected at end of string`.
            // zshrs's bare `stack empty` had no match for scripts
            // grepping zsh's canonical wording.
            s.error = Some("bad math expression: operand expected at end of string".to_string());
            return;
        }

        let mv = pop_with_lval(s);
        let val = if mv.val.is_unset() {
            if let Some(ref name) = mv.lval {
                getmathparam(s, name)
            } else {
                Mnumber::integer(0)
            }
        } else {
            mv.val
        };

        match what {
            MathTok::Not => {
                let result = Mnumber::integer(if val.is_zero() { 1 } else { 0 });
                push(s, result, None);
            }
            MathTok::Comp => {
                let result = Mnumber::integer(!val.to_int());
                push(s, result, None);
            }
            MathTok::UPlus => {
                push(s, val, None);
            }
            MathTok::UMinus => {
                let result = if val.is_float() {
                    Mnumber::float(-val.to_float())
                } else {
                    Mnumber::integer(-val.to_int())
                };
                push(s, result, None);
            }
            MathTok::PostPlus => {
                // ++/-- on a literal (`5++`, `--5`) is a zsh error:
                // "bad math expression: lvalue required". Without the
                // mv.lval guard, zshrs silently incremented the
                // literal value and returned it, masking the bug.
                if mv.lval.is_none() {
                    s.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    Mnumber::float(val.to_float() + 1.0)
                } else {
                    Mnumber::integer(val.to_int() + 1)
                };
                setmathvar(s, name, new_val);
                push(s, val, None); // Return original value
            }
            MathTok::PostMinus => {
                if mv.lval.is_none() {
                    s.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    Mnumber::float(val.to_float() - 1.0)
                } else {
                    Mnumber::integer(val.to_int() - 1)
                };
                setmathvar(s, name, new_val);
                push(s, val, None);
            }
            MathTok::PrePlus => {
                if mv.lval.is_none() {
                    s.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    Mnumber::float(val.to_float() + 1.0)
                } else {
                    Mnumber::integer(val.to_int() + 1)
                };
                setmathvar(s, name, new_val);
                push(s, new_val, mv.lval);
            }
            MathTok::PreMinus => {
                if mv.lval.is_none() {
                    s.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    Mnumber::float(val.to_float() - 1.0)
                } else {
                    Mnumber::integer(val.to_int() - 1)
                };
                setmathvar(s, name, new_val);
                push(s, new_val, mv.lval);
            }
            MathTok::Quest => {
                // Ternary: stack has [cond, true_val, false_val]
                // val already popped = false_val
                // Need to pop true_val and cond
                if s.stack.len() < 2 {
                    s.error = Some("?: needs 3 operands".to_string());
                    return;
                }
                let false_val = val;
                let true_val = pop(s);
                let cond = pop(s);
                let result = if !cond.is_zero() { true_val } else { false_val };
                push(s, result, None);
            }
            MathTok::Colon => {
                s.error = Some("':' without '?'".to_string());
            }
            _ => {
                s.error = Some("unknown operator".to_string());
            }
        }
    }

/// Port of `bop()` from `Src/math.c:1454`.
///
/// Short-circuit boolean prologue. Inspects (without popping) the
/// top of stack and bumps `s.noeval` for the parse-only side of
/// `&&` / `||` / their assignment forms. The matching decrement
/// happens after `mathparse` recurses for the RHS.
pub(crate) fn bop(s: &mut MathState<'_>, tk: MathTok) {
        if s.stack.is_empty() {
            return;
        }
        let mv = &s.stack[s.stack.len() - 1];
        let val = if mv.val.is_unset() {
            if let Some(ref name) = mv.lval {
                getmathparam(s, name)
            } else {
                Mnumber::integer(0)
            }
        } else {
            mv.val
        };

        let tst = !val.is_zero();
        match tk {
            MathTok::DAnd | MathTok::DAndEq if !tst => {
                s.noeval += 1;
            }
            MathTok::DOr | MathTok::DOrEq if tst => {
                s.noeval += 1;
            }
            _ => {}
        }
    }

    pub fn top_prec(s: &MathState<'_>) -> u8 {
        s.prec[MathTok::Comma as usize] + 1
    }

/// Port of `checkunary()` from `Src/math.c:1548`.
///
/// Two roles. (1) Validate that the just-lexed token (`s.mtok`)
/// matches the parser's expectation: an operand was wanted but an
/// operator (`OP_*` flags) showed up, or vice versa. Mismatch
/// emits zsh's `bad math expression: <kind> expected at <ctx>`
/// with `<kind>` being `operator` or `operand` and `<ctx>` taken
/// from the input pointer at the start of the bad token. (2)
/// Update `s.unary` for the next iteration based on `OP_OPF`.
pub fn checkunary(s: &mut MathState<'_>) {
    // Direct port of zsh math.c checkunary() (line 1548).
        // Two roles:
        //   1. Validate that the just-lexed token (`s.mtok`)
        //      matches the parser's expectation (operator vs
        //      operand). Mismatch emits zsh's
        //      "bad math expression: <kind> expected at <ctx>"
        //      with `<kind>` = `operator` (errmsg=2) or `operand`
        //      (errmsg=1). zshrs previously only did step 2,
        //      which left e.g. `let "5 5"` and `$((2#1011x))`
        //      silently accepting bogus input.
        //   2. Update `s.unary` for the next iteration.
        let tp = OP_TYPE[s.mtok as usize];
        let is_op_token = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO | OP_OP)) != 0;
        let errmsg = if is_op_token {
            if s.unary {
                1
            } else {
                0
            }
        } else if !s.unary {
            2
        } else {
            0
        };
        if errmsg != 0 && s.error.is_none() {
            let errtype = if errmsg == 2 { "operator" } else { "operand" };
            // zsh's `mptr` is the input position BEFORE zzlex
            // consumed the bad token. We track the same via
            // `tok_start` which zzlex updates after whitespace
            // skip. Walk forward past whitespace (mirrors zsh's
            // `inblank` skip) so the error context starts at
            // the first visible char.
            let bytes = s.input.as_bytes();
            let mut start = s.tok_start;
            while start < bytes.len() && matches!(bytes[start], b' ' | b'\t' | b'\n') {
                start += 1;
            }
            // zsh truncates after 10 chars and appends `...` if
            // there's more remaining (the over flag in the C
            // source). Mirror that to keep error messages
            // bounded for long bogus expressions.
            let remaining = &s.input[start..];
            let (ctx, over) = if remaining.chars().count() > 10 {
                let truncated: String = remaining.chars().take(10).collect();
                (truncated, true)
            } else {
                (remaining.to_string(), false)
            };
            if ctx.is_empty() {
                s.error = Some(format!(
                    "bad math expression: {} expected at end of string",
                    errtype
                ));
            } else {
                s.error = Some(format!(
                    "bad math expression: {} expected at `{}{}'",
                    errtype,
                    ctx,
                    if over { "..." } else { "" }
                ));
            }
        }
        s.unary = (tp & OP_OPF) == 0;
    }

    /// Operator-precedence parser - closely follows zsh math.c mathparse()
    pub fn mathparse(s: &mut MathState<'_>, pc: u8) {
        if s.error.is_some() {
            return;
        }

        s.mtok = zzlex(s);

        // Handle empty input
        if pc == top_prec(s) && s.mtok == MathTok::Eoi {
            return;
        }

        checkunary(s);

        while s.prec[s.mtok as usize] <= pc {
            if s.error.is_some() {
                return;
            }

            match s.mtok {
                MathTok::Num => {
                    push(s, s.yyval, None);
                }
                MathTok::Id => {
                    let lval = s.yylval.clone();
                    if s.noeval > 0 {
                        push(s, Mnumber::integer(0), Some(lval));
                    } else {
                        push(s, Mnumber::unset(), Some(lval));
                    }
                }
                MathTok::CId => {
                    let lval = s.yylval.clone();
                    let val = if s.noeval > 0 {
                        Mnumber::integer(0)
                    } else {
                        getcvar(s, &lval)
                    };
                    push(s, val, Some(lval));
                }
                MathTok::Func => {
                    let func_call = s.yylval.clone();
                    let val = if s.noeval > 0 {
                        Mnumber::integer(0)
                    } else {
                        callmathfunc(s, &func_call)
                    };
                    push(s, val, None);
                }
                MathTok::InPar => {
                    mathparse(s, top_prec(s));
                    if s.mtok != MathTok::OutPar {
                        if s.error.is_none() {
                            // Match zsh's `bad math expression: ')'
                            // expected` so error diagnostics align.
                            s.error = Some("bad math expression: ')' expected".to_string());
                        }
                        return;
                    }
                }
                MathTok::Quest => {
                    // Ternary operator
                    if s.stack.is_empty() {
                        s.error = Some("bad math expression".to_string());
                        return;
                    }
                    let mv = &s.stack[s.stack.len() - 1];
                    let cond = get_value(s, mv);

                    let q = !cond.is_zero();
                    if !q {
                        s.noeval += 1;
                    }
                    let colon_prec = s.prec[MathTok::Colon as usize];
                    let stack_before = s.stack.len();
                    mathparse(s, colon_prec - 1);
                    if !q {
                        s.noeval -= 1;
                    }

                    if s.mtok != MathTok::Colon {
                        if s.error.is_none() {
                            // Distinguish whether the inner parse
                            // produced an operand: stack grew →
                            // colon expected; stack same → operand
                            // missing (input ran out at end of
                            // string after `?`).
                            if s.stack.len() > stack_before {
                                s.error = Some("bad math expression: ':' expected".to_string());
                            } else {
                                s.error = Some(
                                    "bad math expression: operand expected at end of string"
                                        .to_string(),
                                );
                            }
                        }
                        return;
                    }

                    if q {
                        s.noeval += 1;
                    }
                    let quest_prec = s.prec[MathTok::Quest as usize];
                    mathparse(s, quest_prec);
                    if q {
                        s.noeval -= 1;
                    }

                    op(s, MathTok::Quest);
                    continue;
                }
                _ => {
                    // Binary/unary operator
                    let otok = s.mtok;
                    let onoeval = s.noeval;
                    let tp = OP_TYPE[otok as usize];
                    // Orphan binary at start: `let "*"`, `let "*5"`,
                    // `let "/"`. zsh keeps its input pointer at the
                    // start of the bad operator and emits `operand
                    // expected at \`<remaining>'`. zshrs previously
                    // collapsed every operand-missing case into "at
                    // end of string" which lost the operator
                    // location for orphan-at-start expressions.
                    let is_binary = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0;
                    if s.stack.is_empty() && is_binary {
                        let remaining = &s.input[s.tok_start..];
                        s.error = Some(format!(
                            "bad math expression: operand expected at `{}'",
                            remaining
                        ));
                        return;
                    }
                    if (tp & 0x03) == BOOL {
                        bop(s, otok);
                    }
                    let otok_prec = s.prec[otok as usize];
                    // Right-to-left gets same prec, left-to-right gets prec-1
                    let adjust = if (tp & 0x01) != RL { 1 } else { 0 };
                    mathparse(s, otok_prec - adjust);
                    s.noeval = onoeval;
                    op(s, otok);
                    continue;
                }
            }

            // After operand (Num, Id, Func, InPar), get next token
            s.mtok = zzlex(s);
            checkunary(s);
        }
    }

    /// Call a math function
    pub fn callmathfunc(s: &mut MathState<'_>, call: &str) -> Mnumber {
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
                    let mut eval = new(arg.trim());
                    eval.variables = s.variables.clone();
                    mathevall(&mut eval).ok()
                })
                .collect()
        };
        let args: Vec<f64> = arg_nums.iter().map(|n| n.to_float()).collect();
        let all_int =
            !arg_nums.is_empty() && arg_nums.iter().all(|n| n.is_integer());

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
                "int" | "trunc" => arg_nums.first().map(|n| n.to_int()).unwrap_or(0),
                "floor" => args.first().map(|x| x.floor() as i64).unwrap_or(0),
                "ceil" => args.first().map(|x| x.ceil() as i64).unwrap_or(0),
                _ => 0,
            };
            return Mnumber::integer(i);
        }
        let int_preserving = matches!(name, "abs" | "min" | "max");
        if all_int && int_preserving {
            let i = match name {
                "abs" => arg_nums.first().map(|n| n.to_int().abs()).unwrap_or(0),
                "min" => arg_nums.iter().map(|n| n.to_int()).min().unwrap_or(0),
                "max" => arg_nums.iter().map(|n| n.to_int()).max().unwrap_or(0),
                _ => 0,
            };
            return Mnumber::integer(i);
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
                s.error = Some(format!("unknown function: {}", name));
                0.0
            }
        };

        Mnumber::float(result)
    }

    /// Evaluate the expression
    pub fn mathevall(s: &mut MathState<'_>) -> Result<Mnumber, String> {
        s.prec = if s.c_precedences { &C_PREC } else { &Z_PREC };

        // Skip leading whitespace and Nularg
        while let Some(c) = peek(s) {
            if c.is_whitespace() || c == '\u{a1}' {
                advance(s);
            } else {
                break;
            }
        }

        if s.pos >= s.input.len() {
            return Ok(Mnumber::integer(0));
        }

        mathparse(s, top_prec(s));

        if let Some(ref err) = s.error {
            return Err(err.clone());
        }

        // Check for trailing characters
        while let Some(c) = peek(s) {
            if c.is_whitespace() {
                advance(s);
            } else if c == ')' {
                // zsh's specific wording for the unmatched-close
                // case: `bad math expression: unexpected ')'`.
                return Err("bad math expression: unexpected ')'".to_string());
            } else {
                return Err(format!("illegal character: {}", c));
            }
        }

        if s.stack.is_empty() {
            return Ok(Mnumber::integer(0));
        }

        let mv = s.stack.pop().unwrap();
        let result = if mv.val.is_unset() {
            if let Some(ref name) = mv.lval {
                getmathparam(s, name)
            } else {
                Mnumber::integer(0)
            }
        } else {
            mv.val
        };

        Ok(result)
    }

/// Get updated variables after evaluation
pub fn getmathparams<'a>(s: &'a MathState<'_>) -> &'a HashMap<String, Mnumber> {
    &s.variables
}

/// Convenience function to evaluate a math expression
/// Top-level math-expression evaluator.
/// Port of `matheval()` from Src/math.c:1480 — wraps `mathevall()`\n/// (line 367) with the C source's standard error-message\n/// formatting.
pub fn matheval(expr: &str) -> Result<Mnumber, String> {
    let mut eval = new(expr);
    mathevall(&mut eval)
}

/// Evaluate and return integer
/// Math evaluator that coerces the result to integer.
/// Port of `mathevali()` from Src/math.c:1505.
pub fn mathevali(expr: &str) -> Result<i64, String> {
    matheval(expr).map(|n| n.to_int())
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
        assert!((matheval("2.0 ** 0.5").map(|n| n.to_float()).unwrap() - std::f64::consts::SQRT_2).abs() < 0.0001);
    }

    #[test]
    fn test_float() {
        assert!((matheval("3.14 + 0.01").map(|n| n.to_float()).unwrap() - 3.15).abs() < 0.0001);
        assert!((matheval("1.5 * 2.0").map(|n| n.to_float()).unwrap() - 3.0).abs() < 0.0001);
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
        vars.insert("x".to_string(), Mnumber::integer(10));
        vars.insert("y".to_string(), Mnumber::integer(20));

        let mut eval = with_variables(new("x + y"), vars);
        assert_eq!(mathevall(&mut eval).unwrap().to_int(), 30);
    }

    #[test]
    fn test_assignment() {
        let mut eval = new("x = 5");
        mathevall(&mut eval).unwrap();
        assert_eq!(eval.variables.get("x").unwrap().to_int(), 5);

        let mut eval2 = new("x = 5, x += 3");
        let result = mathevall(&mut eval2).unwrap();
        assert_eq!(result.to_int(), 8);
    }

    #[test]
    fn test_increment() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), Mnumber::integer(5));

        let mut eval = with_variables(new("++x"), vars.clone());
        assert_eq!(mathevall(&mut eval).unwrap().to_int(), 6);
        assert_eq!(eval.variables.get("x").unwrap().to_int(), 6);

        let mut eval2 = with_variables(new("x++"), vars.clone());
        assert_eq!(mathevall(&mut eval2).unwrap().to_int(), 5);
        assert_eq!(eval2.variables.get("x").unwrap().to_int(), 6);
    }

    #[test]
    fn test_functions() {
        assert!((matheval("sqrt(4)").map(|n| n.to_float()).unwrap() - 2.0).abs() < 0.0001);
        assert!((matheval("sin(0)").map(|n| n.to_float()).unwrap()).abs() < 0.0001);
        assert!((matheval("cos(0)").map(|n| n.to_float()).unwrap() - 1.0).abs() < 0.0001);
        assert!((matheval("abs(-5)").map(|n| n.to_float()).unwrap() - 5.0).abs() < 0.0001);
        assert!((matheval("floor(3.7)").map(|n| n.to_float()).unwrap() - 3.0).abs() < 0.0001);
        assert!((matheval("ceil(3.2)").map(|n| n.to_float()).unwrap() - 4.0).abs() < 0.0001);
    }

    #[test]
    fn test_special_values() {
        assert!(matheval("Inf").map(|n| n.to_float()).unwrap().is_infinite());
        assert!(matheval("NaN").map(|n| n.to_float()).unwrap().is_nan());
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
impl crate::ported::exec::ShellExecutor {
    pub fn evaluate_arithmetic(&mut self, expr: &str) -> String {
        // First, resolve `$NAME[(flags)pat]` / `$@[(flags)pat]`
        // before expand_string — otherwise `$@` gets joined into
        // a scalar (`a b c`) and the trailing `[…]` becomes
        // ambiguous text. zinit relies on `(( $@[(I)-*] ))`.
        let expr_pre = if expr.contains('$') {
            self.pre_resolve_dollar_subscripts(expr)
        } else {
            expr.to_string()
        };
        // Only run expand_string when the expression has `$` (for
        // var/cmd-subst/nested-arith). Otherwise pass through —
        // expand_string would tilde-expand `~` (bitwise NOT in arith
        // context) into "no such user" errors.
        let expr = if expr_pre.contains('$') || expr_pre.contains('`') {
            self.singsub(&expr_pre)
        } else {
            expr_pre
        };
        // Subscripted-array compound-assign / increment / decrement:
        // `((a[i]++))`, `((a[i]+=v))`, `((a[i]-=v))`, etc. Read the
        // current value, apply the operation, write back. MathState
        // can't write through `a[i]` for compound forms (only the
        // bare `=` write was special-cased below), so handle here.
        // Subscript compound op: `((a[i]++))`, `((h[k]+=5))`, etc.
        // Combined post-op + pre-op detection. Direct port of zsh
        // math.c LVAL_NUM_SUBSC: the subscript receiver retains its
        // lvalue identity across the operator. Without this,
        // pre_resolve_array_subscripts substitutes the value first
        // and `5++` errors "lvalue required".
        let compound = parse_compound(&expr)
            .map(|(n, i, o, r)| (n, i, o, r, false))
            .or_else(|| {
                parse_pre_inc(&expr).map(|(n, i, o)| (n, i, o, String::new(), true))
            });
        if let Some((name, idx_expr, op, rhs, is_pre)) = compound {
            let is_assoc = self.assoc_arrays.contains_key(&name);
            let idx_val = if is_assoc {
                0
            } else {
                self.eval_arith_expr(&idx_expr)
            };
            let key_str = if is_assoc {
                let s = idx_expr.trim();
                if (s.starts_with('"') && s.ends_with('"'))
                    || (s.starts_with('\'') && s.ends_with('\''))
                {
                    s[1..s.len() - 1].to_string()
                } else {
                    s.to_string()
                }
            } else {
                String::new()
            };
            let rhs_val = if rhs.is_empty() {
                1
            } else {
                self.eval_arith_expr(&rhs)
            };
            let cur: i64 = if is_assoc {
                self.assoc_arrays
                    .get(&name)
                    .and_then(|m| m.get(&key_str))
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0)
            } else if let Some(arr) = self.arrays.get(&name) {
                let len = arr.len() as i64;
                let pos = if idx_val < 0 {
                    len + idx_val
                } else {
                    idx_val - 1
                };
                if pos >= 0 && (pos as usize) < arr.len() {
                    arr[pos as usize].parse().unwrap_or(0)
                } else {
                    0
                }
            } else {
                0
            };
            let new_val: i64 = match op.as_str() {
                "++" => cur + 1,
                "--" => cur - 1,
                "+=" => cur + rhs_val,
                "-=" => cur - rhs_val,
                "*=" => cur * rhs_val,
                "/=" => {
                    if rhs_val == 0 {
                        zerr("division by zero");
                        return cur.to_string();
                    }
                    cur / rhs_val
                }
                "%=" => {
                    if rhs_val == 0 {
                        zerr("division by zero");
                        return cur.to_string();
                    }
                    cur % rhs_val
                }
                "&=" => cur & rhs_val,
                "|=" => cur | rhs_val,
                "^=" => cur ^ rhs_val,
                "<<=" => cur << rhs_val,
                ">>=" => cur >> rhs_val,
                "**=" => (cur as f64).powi(rhs_val as i32) as i64,
                _ => cur,
            };
            // Write back.
            if is_assoc {
                if let Some(map) = self.assoc_arrays.get_mut(&name) {
                    map.insert(key_str, new_val.to_string());
                }
            } else if let Some(arr) = self.arrays.get_mut(&name) {
                let len = arr.len() as i64;
                let pos = if idx_val < 0 {
                    len + idx_val
                } else {
                    idx_val - 1
                };
                if pos >= 0 {
                    let p = pos as usize;
                    if p >= arr.len() {
                        arr.resize(p + 1, "0".to_string());
                    }
                    arr[p] = new_val.to_string();
                }
            } else {
                // Auto-create indexed array.
                let mut arr: Vec<String> = Vec::new();
                let pos = (idx_val - 1).max(0) as usize;
                arr.resize(pos + 1, "0".to_string());
                arr[pos] = new_val.to_string();
                self.arrays.insert(name, arr);
            }
            // Post `++`/`--` returns OLD value; pre-op + compound
            // assigns return NEW value.
            let result = if !is_pre && (op == "++" || op == "--") {
                cur
            } else {
                new_val
            };
            return result.to_string();
        }
        // Subscripted-array arith assignment: `((a[i]=expr))`. Without
        // this special case, pre_resolve_array_subscripts would
        // substitute a[i] with its current value (`0=42` → invalid).
        if let Some((name, idx_expr, rhs)) = parse_assign(&expr) {
            let idx_val = self.eval_arith_expr(&idx_expr);
            let rhs_val = self.eval_arith_expr(&rhs);
            if let Some(arr) = self.arrays.get_mut(&name) {
                let i_pos = if idx_val < 0 {
                    arr.len() as i64 + idx_val
                } else {
                    idx_val - 1
                };
                if i_pos >= 0 {
                    let pos = i_pos as usize;
                    if pos >= arr.len() {
                        arr.resize(pos + 1, "0".to_string());
                    }
                    arr[pos] = rhs_val.to_string();
                }
            } else if let Some(map) = self.assoc_arrays.get_mut(&name) {
                map.insert(idx_val.to_string(), rhs_val.to_string());
            } else {
                let mut arr: Vec<String> = Vec::new();
                let i_pos = if idx_val < 0 {
                    0
                } else {
                    (idx_val - 1).max(0) as usize
                };
                arr.resize(i_pos + 1, "0".to_string());
                arr[i_pos] = rhs_val.to_string();
                self.arrays.insert(name, arr);
            }
            return rhs_val.to_string();
        }
        let expr = self.pre_resolve_array_subscripts(&expr);
        // Output radix prefix `[#N]EXPR` (with `N#` prefix) and
        // `[##N]EXPR` (without). Direct port of zsh's math.c
        // (line 786 onward in patcompswitch's `[` case): `n=1`
        // for single-`#` (prefix kept), `n=-1` for double-`##`
        // (prefix dropped). The base must be 2..=36. Strip the
        // prefix from `expr`, store the radix for post-eval
        // formatting, then continue with the inner expression.
        let mut output_radix: Option<(u32, bool)> = None;
        let mut output_underscore: Option<u32> = None;
        let expr = {
            // Direct port of zsh src/zsh/Src/math.c:786-833. Handles:
            //   [N]NUM       (base-N literal, processed elsewhere)
            //   [#N]EXPR     (output radix N, prefixed `N#`)
            //   [##N]EXPR    (output radix N, no prefix)
            //   [#N_M]EXPR   (output radix N, group every M digits with `_`)
            //   [##N_]EXPR   (output radix, group default 3 digits)
            // Allow leading whitespace before `[#`; trim again after `]`.
            let mut e = expr.as_str().trim_start();
            if let Some(rest) = e.strip_prefix("[#") {
                let (no_prefix_form, body) = if let Some(r2) = rest.strip_prefix('#') {
                    (true, r2)
                } else {
                    (false, rest)
                };
                if let Some(close_idx) = body.find(']') {
                    // Split radix and optional `_GROUP` per math.c:810-815:
                    //   if (*ptr == '_') { ptr++; if (idigit(*ptr))
                    //     outputunderscore=zstrtol(ptr,...); else outputunderscore=3; }
                    let inside = &body[..close_idx];
                    let (n_str, under_part) = match inside.find('_') {
                        Some(p) => (&inside[..p], Some(&inside[p + 1..])),
                        None => (inside, None),
                    };
                    if let Ok(n) = n_str.parse::<u32>() {
                        if (2..=36).contains(&n) {
                            output_radix = Some((n, no_prefix_form));
                            // Underscore digit-group size. Empty
                            // suffix means default 3 (matches zsh's
                            // `else outputunderscore = 3`).
                            output_underscore = under_part.map(|s| {
                                if s.is_empty() {
                                    3
                                } else {
                                    s.parse::<u32>().unwrap_or(3)
                                }
                            });
                            e = body[close_idx + 1..].trim_start();
                        }
                    }
                }
            }
            e.to_string()
        };
        let force_float = self.options.get("forcefloat").copied().unwrap_or(false);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        // Pre-resolve dynamic special parameters that aren't in the
        // variables map: $RANDOM, $SECONDS, $EPOCHSECONDS,
        // $EPOCHREALTIME, $LINENO, $PPID, $UID, $EUID, $GID, $EGID.
        // MathState looks up names in a static HashMap, so without
        // substitution these would resolve to 0. Inject the current
        // value into a fresh extras HashMap.
        let mut extras = self.variables.clone();
        for special in [
            "RANDOM",
            "SECONDS",
            "EPOCHSECONDS",
            "EPOCHREALTIME",
            "LINENO",
            "PPID",
            "UID",
            "EUID",
            "GID",
            "EGID",
        ] {
            if !extras.contains_key(special) || special == "RANDOM" {
                let v = self.get_variable(special);
                extras.insert(special.to_string(), v);
            }
        }
        let evaluator = new(&expr);
        let evaluator = with_string_variables(evaluator, &extras);
        let evaluator = with_force_float(evaluator, force_float);
        let evaluator = with_c_precedences(evaluator, c_prec);
        let mut evaluator = with_octal_zeroes(evaluator, octal);

        match mathevall(&mut evaluator) {
            Ok(result) => {
                for (k, v) in extract_string_variables(&evaluator) {
                    let formatted = self.format_for_var_attr(&k, &v);
                    // Only mirror to env when the variable is
                    // explicitly exported (typeset -x or env::var
                    // already has it from a prior export). zshrs
                    // previously env::set_var-d every arith write-
                    // back, which leaked `local -i x=0; ((x=5))`
                    // values into the process env and survived the
                    // fn-exit local_save_stack unwind — variables
                    // got restored but env::var() lookup-fallback
                    // still saw the leaked value, so `${x:-unset}`
                    // post-fn returned the stale leaked value.
                    let is_exported = self
                        .var_attrs
                        .get(&k)
                        .map(|a| a.export)
                        .unwrap_or(false);
                    self.variables.insert(k.clone(), formatted.clone());
                    if is_exported {
                        env::set_var(&k, &formatted);
                    }
                }
                // If the expression had a `[#N]` / `[##N]` prefix,
                // format the integer result in base N. zsh's
                // single-`#` form prefixes `N#`; double-`##` drops
                // the prefix (math.c: `outputradix < 0` means
                // no-prefix). Floats fall back to the default %g
                // format (zsh: same thing — radix only affects
                // integer results).
                if let Some((base, no_prefix)) = output_radix {
                    let n = result.to_int();
                    // Direct port of convbase_underscore at
                    // Src/params.c:5645 — handles `[#N_M]` underscore
                    // grouping (no-op when group is None / 0).
                    let body = crate::ported::params::convbase_underscore(
                        n,
                        base,
                        output_underscore.map(|g| g as i32).unwrap_or(0),
                    );
                    // Direct port of convbase_ptr at
                    // src/zsh/Src/params.c:5596-5604:
                    //   isset(CBASES) && base == 16              → "0x"
                    //   isset(CBASES) && base == 8 && OCTALZEROES → "0"
                    //   base != 10                                → "N#"
                    //   else                                      → ""
                    // The double-`##` form (`[##N]`) drops the prefix
                    // entirely (math.c outputradix < 0 → params.c:5606
                    // takes the else branch with negated base, no prefix).
                    let cbases = self.options.get("cbases").copied().unwrap_or(false);
                    let octalzeroes = self.options.get("octalzeroes").copied().unwrap_or(false);
                    // body currently has "N#DIGITS" (or "-N#DIGITS").
                    // Strip the "N#" so we can prepend whichever prefix
                    // the option-set demands.
                    let (sign, raw_digits) = if let Some(stripped) = body.strip_prefix('-') {
                        ("-", stripped)
                    } else {
                        ("", body.as_str())
                    };
                    let digits = match raw_digits.find('#') {
                        Some(idx) => &raw_digits[idx + 1..],
                        None => raw_digits,
                    };
                    let prefix = if no_prefix {
                        ""
                    } else if cbases && base == 16 {
                        "0x"
                    } else if cbases && base == 8 && octalzeroes {
                        "0"
                    } else if base != 10 {
                        // Will format below with `N#` prefix.
                        return format!("{}{}#{}", sign, base, digits);
                    } else {
                        ""
                    };
                    return format!("{}{}{}", sign, prefix, digits);
                }
                // zsh splits formatting between the two contexts that
                // share this code path:
                //   - `$(())` arithmetic substitution → `%g`-ish: 4.0
                //     prints as "4." (zsh quirk — keeps the dot to
                //     mark "this is float", drops trailing zeros)
                //   - storage from `let`/`(( a=… ))` → `%.10f`
                // extract_string_variables (storage) already uses
                // %.10f via format_zsh; here for the substitution
                // return value emulate zsh's %g style.
                result.format_zsh_subst()
            }
            Err(msg) => {
                // zsh writes arith errors to stderr in `zsh:LINE: <msg>`
                // form. Status conventions differ by context but both
                // paths call this method — emit the diagnostic and
                // return "0"; the calling site decides whether to abort
                // (substitution: zsh aborts the whole command) or
                // continue (arith command: status 1-or-2 from the
                // StrEq-to-"0" check). Avoid touching `last_status`
                // here — the SetStatus op emitted by callers wins
                // anyway, AND a stray `last_status=2` clobbers the
                // status of unrelated paths that share evaluate_arith
                // (e.g. `a+=y` where the value parses as a non-arith
                // string then errors silently).
                zerr(&format!("{}", msg));
                // zsh aborts the surrounding command on arith
                // errors — `echo $((2#5))` emits the diagnostic
                // but does NOT print `0`. Match common error
                // shapes — "bad math expression" is the canonical
                // give-up signal; "invalid base" is a separate
                // diagnostic from numeric base parsing. Without
                // this, zshrs printed the diagnostic THEN the
                // bogus `0` value.
                if msg.starts_with("bad math expression") || msg.starts_with("invalid base") {
                    std::process::exit(1);
                }
                // NOTE: NOT aborting on "division by zero" — `((1/0))`
                // arith COMMAND continues with non-zero status (zsh
                // sets 2). Only `$((1/0))` substitution should abort,
                // but both share this evaluator and we lack a context
                // signal to distinguish. Keeping continue-with-"0"
                // for now; substitution callers see the diagnostic.
                "0".to_string()
            }
        }
    }
    pub(crate) fn eval_arith_expr(&mut self, expr: &str) -> i64 {
        let expr_expanded = if expr.contains('$') || expr.contains('`') {
            self.singsub(expr)
        } else {
            expr.to_string()
        };
        // Subscripted-array arith assignment: `((a[i]=expr))`. The
        // pre_resolve_array_subscripts pass below substitutes a[i]
        // with the current value (e.g. 0=42 → invalid). Detect the
        // assignment LHS first, evaluate the RHS, write to arrays.
        if let Some((name, idx_expr, rhs)) = parse_assign(&expr_expanded) {
            // Evaluate the index (could itself be an expression).
            let idx_val = self.eval_arith_expr(&idx_expr);
            // Evaluate the RHS.
            let rhs_val = self.eval_arith_expr(&rhs);
            // Write back: arrays for numeric idx, assoc otherwise.
            if let Some(arr) = self.arrays.get_mut(&name) {
                let i_pos = if idx_val < 0 {
                    arr.len() as i64 + idx_val
                } else {
                    idx_val - 1
                };
                if i_pos >= 0 {
                    let pos = i_pos as usize;
                    if pos >= arr.len() {
                        arr.resize(pos + 1, "0".to_string());
                    }
                    arr[pos] = rhs_val.to_string();
                }
            } else if let Some(map) = self.assoc_arrays.get_mut(&name) {
                map.insert(idx_val.to_string(), rhs_val.to_string());
            } else {
                // Auto-create indexed array.
                let mut arr: Vec<String> = Vec::new();
                let i_pos = if idx_val < 0 {
                    0
                } else {
                    (idx_val - 1).max(0) as usize
                };
                arr.resize(i_pos + 1, "0".to_string());
                arr[i_pos] = rhs_val.to_string();
                self.arrays.insert(name, arr);
            }
            return rhs_val;
        }
        let expr_expanded = self.pre_resolve_array_subscripts(&expr_expanded);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        let evaluator = new(&expr_expanded);
        let evaluator = with_string_variables(evaluator, &self.variables);
        let evaluator = with_c_precedences(evaluator, c_prec);
        let mut evaluator = with_octal_zeroes(evaluator, octal);

        match mathevall(&mut evaluator) {
            Ok(result) => {
                for (k, v) in extract_string_variables(&evaluator) {
                    let formatted = self.format_for_var_attr(&k, &v);
                    // Only mirror to env when the variable is
                    // explicitly exported (typeset -x or env::var
                    // already has it from a prior export). zshrs
                    // previously env::set_var-d every arith write-
                    // back, which leaked `local -i x=0; ((x=5))`
                    // values into the process env and survived the
                    // fn-exit local_save_stack unwind — variables
                    // got restored but env::var() lookup-fallback
                    // still saw the leaked value, so `${x:-unset}`
                    // post-fn returned the stale leaked value.
                    let is_exported = self
                        .var_attrs
                        .get(&k)
                        .map(|a| a.export)
                        .unwrap_or(false);
                    self.variables.insert(k.clone(), formatted.clone());
                    if is_exported {
                        env::set_var(&k, &formatted);
                    }
                }
                result.to_int()
            }
            Err(msg) => {
                // zsh writes arith errors (div-by-zero, bad expr, etc.) to
                // stderr in the form `zshrs:LINE: <message>`. Without this
                // gate, `$((10/0))` returned 0 silently — masking real bugs
                // in user scripts.
                zerr(&format!("{}", msg));
                0
            }
        }
    }
    pub(crate) fn eval_arith_expr_float(&mut self, expr: &str) -> f64 {
        let expr_expanded = if expr.contains('$') || expr.contains('`') {
            self.singsub(expr)
        } else {
            expr.to_string()
        };
        let expr_expanded = self.pre_resolve_array_subscripts(&expr_expanded);
        let force_float = self.options.get("forcefloat").copied().unwrap_or(false);
        let c_prec = self.options.get("cprecedences").copied().unwrap_or(false);
        let octal = self.options.get("octalzeroes").copied().unwrap_or(false);

        let evaluator = new(&expr_expanded);
        let evaluator = with_string_variables(evaluator, &self.variables);
        let evaluator = with_force_float(evaluator, force_float);
        let evaluator = with_c_precedences(evaluator, c_prec);
        let mut evaluator = with_octal_zeroes(evaluator, octal);

        match mathevall(&mut evaluator) {
            Ok(result) => {
                for (k, v) in extract_string_variables(&evaluator) {
                    let formatted = self.format_for_var_attr(&k, &v);
                    // Only mirror to env when the variable is
                    // explicitly exported (typeset -x or env::var
                    // already has it from a prior export). zshrs
                    // previously env::set_var-d every arith write-
                    // back, which leaked `local -i x=0; ((x=5))`
                    // values into the process env and survived the
                    // fn-exit local_save_stack unwind — variables
                    // got restored but env::var() lookup-fallback
                    // still saw the leaked value, so `${x:-unset}`
                    // post-fn returned the stale leaked value.
                    let is_exported = self
                        .var_attrs
                        .get(&k)
                        .map(|a| a.export)
                        .unwrap_or(false);
                    self.variables.insert(k.clone(), formatted.clone());
                    if is_exported {
                        env::set_var(&k, &formatted);
                    }
                }
                result.to_float()
            }
            Err(_) => 0.0,
        }
    }
    pub(crate) fn evaluate_arithmetic_expr(&mut self, expr: &str) -> i64 {
        self.eval_arith_expr(expr)
    }
    /// Execute arithmetic expression
    /// Port of execarith() from exec.c
    pub fn execarith(&mut self, expr: &str) -> i32 {
        let result = self.eval_arith_expr(expr);
        if result == 0 {
            1
        } else {
            0
        }
    }
}
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


/// Format an integer in the given base (2-36) using zsh's
/// `BASE#DIGITS` form.
/// Port of `convbase()` from Src/utils.c (also called from
/// Src/math.c:1089). Bases 2-9 are unsigned-style; uppercase
/// A-Z are used for digits >= 10. A negative value is output
/// as `-BASE#DIGITS`.
pub fn convbase(n: i64, base: u32) -> String {
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

/// Port of `isinf()` from Src/math.c:588 — IEEE +/-Infinity test.
/// Wraps Rust's `f64::is_infinite`.
pub fn isinf(x: f64) -> bool { x.is_infinite() }

/// Port of `isnan()` from Src/math.c:608 — IEEE NaN test. C
/// implements it as `store(&x) != store(&x)` to defeat compiler
/// folding of the canonical `x != x` NaN test; we route through
/// `store` for parity, but Rust's `f64::is_nan` is the
/// correctness path.
pub fn isnan(x: f64) -> bool { store(x) != store(x) || x.is_nan() }

/// Port of `notzero()` from Src/math.c:1142 — error-on-zero check
/// used by `/` and `%` operators. Returns true when `a` is non-
/// zero (caller continues), false when zero (caller raises
/// "division by zero"). Float zero is treated as non-zero per
/// IEEE 754 (1/0.0 → Inf, not an error) — only integer zero
/// trips the check, matching math.c's `if (!a.u.l) zerr(…)`.
pub fn notzero(a: Mnumber) -> bool {
    if a.is_unset() {
        return false;
    }
    if a.is_integer() {
        return a.l != 0;
    }
    true
}

/// Port of `store()` from Src/math.c:601 — load/store a double
/// via a pointer to defeat compilers that mis-optimize the
/// canonical `x != x` NaN test. zsh only compiles this path when
/// `HAVE_ISNAN` is undefined; we keep it as a name-parity shim
/// so `isnan()` can route through it (matching the C source's
/// `store(&x) != store(&x)` idiom).
pub fn store(x: f64) -> f64 { x }

/// Port of `getcvar()` from Src/math.c:943 — character-constant
/// lookup. Reads the named shell variable and returns the
/// codepoint of its first character. Used for `#varname` token
/// (CId): `x="hello"; (( y = #x ))` puts 104 (`'h'`) into y.
/// On miss or empty value, returns 0 (matches zsh's `*s ? *s : 0`).
pub fn getcvar(s: &MathState<'_>, name: &str) -> Mnumber {
    if let Some(raw) = s.string_variables.get(name) {
        return Mnumber::integer(raw.chars().next().map(|c| c as i64).unwrap_or(0));
    }
    if let Some(v) = s.variables.get(name) {
        return Mnumber::integer(v.format_zsh().chars().next().map(|c| c as i64).unwrap_or(0));
    }
    Mnumber::integer(0)
}

/// Port of `mathevalarg()` from Src/math.c:1514 — evaluate one
/// arg expression and return as integer. Used by `let` builtin
/// and others that take an arith-expr argument.
pub fn mathevalarg(expr: &str) -> i64 {
    matheval(expr).map(|n| n.to_int()).unwrap_or(0)
}
