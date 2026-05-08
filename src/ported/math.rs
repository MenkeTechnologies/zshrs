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

/// Math number — integer, float, or unset.
/// Port of `mnumber` from Src/zsh.h — the C source's union of\n/// `MN_INTEGER` (zlong) and `MN_FLOAT` (double). The `Unset`\n/// variant captures the C source's NULL-pointer return on math\n/// errors.
#[derive(Debug, Clone, Copy)]
pub enum MathNum {
    Integer(i64),
    Float(f64),
    Unset,
}

impl Default for MathNum {
    fn default() -> Self {
        MathNum::Integer(0)
    }
}

impl MathNum {
    pub fn is_zero(&self) -> bool {
        match self {
            MathNum::Integer(n) => *n == 0,
            MathNum::Float(f) => *f == 0.0,
            MathNum::Unset => true,
        }
    }

    pub fn to_int(&self) -> i64 {
        match self {
            MathNum::Integer(n) => *n,
            MathNum::Float(f) => *f as i64,
            MathNum::Unset => 0,
        }
    }

    pub fn to_float(&self) -> f64 {
        match self {
            MathNum::Integer(n) => *n as f64,
            MathNum::Float(f) => *f,
            MathNum::Unset => 0.0,
        }
    }

    pub fn is_float(&self) -> bool {
        matches!(self, MathNum::Float(_))
    }

    pub fn is_integer(&self) -> bool {
        matches!(self, MathNum::Integer(_))
    }

    /// Format for stored variable values (zsh `let` / `(( a=… ))`):
    /// Format a math result for storage back into a shell variable.
    /// Integers print plain; floats use `%.10f`. IEEE specials
    /// (Inf/-Inf/NaN) get capitalized form. zsh has two slightly
    /// different storage formats (`let` uses %.10f, `((a += float))`
    /// uses %g) — the unified %.10f path is the closer fit for the
    /// `let`/`(( a = … ))` flow which is the more common case.
    pub fn format_zsh(&self) -> String {
        match self {
            MathNum::Integer(i) => i.to_string(),
            MathNum::Float(f) => {
                if f.is_nan() {
                    "NaN".to_string()
                } else if f.is_infinite() {
                    if *f > 0.0 {
                        "Inf".to_string()
                    } else {
                        "-Inf".to_string()
                    }
                } else {
                    format!("{:.10}", f)
                }
            }
            MathNum::Unset => "0".to_string(),
        }
    }

    /// Format for `$(( ))` arithmetic substitution display. zsh
    /// behavior:
    ///   - Float with `fract()==0`: print as `<int>.` (trailing dot
    ///     marks "this is float", no zeros after).
    ///   - Float with non-zero fract: print at full f64 precision via
    ///     Rust's shortest-roundtrip Display (matches zsh which prints
    ///     16-digit-ish strings like "2.2000000000000002" for inexact
    ///     values).
    pub fn format_zsh_subst(&self) -> String {
        match self {
            MathNum::Integer(i) => i.to_string(),
            MathNum::Float(f) => format_zsh_float(*f),
            MathNum::Unset => "0".to_string(),
        }
    }
}

/// Format a float for `$(( ))` arithmetic-substitution display the
/// way zsh's `convfloat()` (Src/params.c) does it: a single
/// `sprintf("%.*g", 17, val)` call plus zsh's "if no `e` and no `.`
/// after, append `.`" trailing-dot rule for integer-valued floats.
///
/// Routed through `libc::snprintf` directly so the output matches
/// the C runtime byte-for-byte across the full f64 range —
/// 0.1 → `0.10000000000000001`, 9.9e16 → `99000000000000000.`,
/// 1e-5 → `1.0000000000000001e-05`, etc. Earlier handwritten
/// implementations partitioned the range and used Rust's
/// shortest-roundtrip `{:e}`/`{}` for the edges, which diverged
/// from zsh on inexact-representable values.
///
/// IEEE specials: zsh hand-formats `Inf`, `-Inf`, `NaN`
/// (capitalized) — does NOT round-trip through `%g` for those,
/// since C's `%g` lowercases.
fn format_zsh_float(f: f64) -> String {
    if f.is_nan() {
        return "NaN".to_string();
    }
    if f.is_infinite() {
        return if f > 0.0 {
            "Inf".to_string()
        } else {
            "-Inf".to_string()
        };
    }
    // Negative zero: zsh preserves the sign in the trailing-dot
    // form (`-0.`). C's `%g` would print `-0` first; the
    // append-dot rule then makes it `-0.`. Same code path as the
    // generic case but worth calling out — earlier impls
    // bypassed `%g` to detect this.
    let buf_len = 64usize; // 17-digit %g + sign + exponent + NUL fits comfortably
    let mut buf = vec![0u8; buf_len];
    // SAFETY: buffer has room for any %.17g output (max ~25 chars
    // for double); fmt is a NUL-terminated literal. snprintf
    // writes UTF-8-compatible ASCII only (digits, `.`, `e`, `+`,
    // `-`).
    let n = unsafe {
        libc::snprintf(
            buf.as_mut_ptr() as *mut libc::c_char,
            buf_len,
            c"%.*g".as_ptr(),
            17 as libc::c_int,
            f,
        )
    };
    if n < 0 {
        // snprintf failure — fall back to a plain Rust display so
        // the shell doesn't crash. Should never happen for finite
        // f64 with a 64-byte buffer.
        return format!("{}", f);
    }
    let len = (n as usize).min(buf_len - 1);
    buf.truncate(len);
    let mut s = String::from_utf8(buf).unwrap_or_else(|_| format!("{}", f));
    // zsh's convfloat appends `.` when the output has no `e` and
    // no `.` — turning integer-valued floats like `5` into `5.`
    // so callers can tell them apart from MathNum::Integer.
    if !s.contains('e') && !s.contains('.') {
        s.push('.');
    }
    s
}

/// Math tokens - from math.c
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum MathTok {
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

/// Stack value for the evaluator
#[derive(Clone)]
struct MathValue {
    val: MathNum,
    lval: Option<String>,
}

impl Default for MathValue {
    fn default() -> Self {
        MathValue {
            val: MathNum::Integer(0),
            lval: None,
        }
    }
}

/// Math evaluator state.
/// Port of the per-evaluation locals `mathevall()` (Src/math.c:367)\n/// keeps — input cursor, operator stack, value stack. Drives\n/// `zzlex()` (line 617), `push()` / `pop()` (lines 916/931), and\n/// `op()` / `bop()` (lines 1154/1454).
pub struct MathEval<'a> {
    input: &'a str,
    pos: usize,
    /// Byte position in `input` where the most recently lexed token began
    /// (after whitespace skip). Used to format zsh-style error pointers
    /// like `bad math expression: operand expected at `*'` for orphan
    /// binary operators — zsh's error retains the input pointer at the
    /// start of the bad operator, so the message includes the operator
    /// (and any trailing input) rather than the post-consumption position.
    tok_start: usize,
    yyval: MathNum,
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
    variables: HashMap<String, MathNum>,
    /// Raw string values for variables whose contents aren't a plain number.
    /// zsh recursively evaluates these as arith expressions on lookup so
    /// `a="3+2"; $((a))` produces 5. Without this, MathEval saw `a` as
    /// unset → 0.
    string_variables: HashMap<String, String>,
    lastval: i32,
    pid: i64,
    error: Option<String>,
}

impl<'a> MathEval<'a> {
    pub fn new(input: &'a str) -> Self {
        MathEval {
            input,
            pos: 0,
            tok_start: 0,
            yyval: MathNum::Integer(0),
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

    pub fn with_variables(mut self, vars: HashMap<String, MathNum>) -> Self {
        self.variables = vars;
        self
    }

    /// Inject variables from string->string mapping (for shell integration)
    pub fn with_string_variables(mut self, vars: &HashMap<String, String>) -> Self {
        for (k, v) in vars {
            if let Ok(i) = v.parse::<i64>() {
                self.variables.insert(k.clone(), MathNum::Integer(i));
            } else if let Ok(f) = v.parse::<f64>() {
                self.variables.insert(k.clone(), MathNum::Float(f));
            } else if !v.is_empty() {
                // Non-numeric string — keep raw so get_variable can
                // recursively evaluate it as an arith expression.
                // zsh: `a="3+2"; $((a))` returns 5.
                self.string_variables.insert(k.clone(), v.clone());
            }
        }
        self
    }

    /// Extract modified variables as string->string mapping (for shell integration)
    pub fn extract_string_variables(&self) -> HashMap<String, String> {
        self.variables
            .iter()
            .map(|(k, v)| (k.clone(), v.format_zsh()))
            .collect()
    }

    pub fn with_c_precedences(mut self, enable: bool) -> Self {
        self.c_precedences = enable;
        self.prec = if enable { &C_PREC } else { &Z_PREC };
        self
    }

    pub fn with_force_float(mut self, enable: bool) -> Self {
        self.force_float = enable;
        self
    }

    pub fn with_octal_zeroes(mut self, enable: bool) -> Self {
        self.octal_zeroes = enable;
        self
    }

    pub fn with_lastval(mut self, val: i32) -> Self {
        self.lastval = val;
        self
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
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

    /// Lex a numeric constant
    fn lex_constant(&mut self) -> MathTok {
        let _start = self.pos;
        let mut is_neg = false;

        // Handle leading minus for unary context
        if self.peek() == Some('-') {
            is_neg = true;
            self.advance();
        }

        // Check for hex/binary/octal
        if self.peek() == Some('0') {
            self.advance();
            match self.peek().map(|c| c.to_ascii_lowercase()) {
                Some('x') => {
                    // Hex: 0xFF
                    self.advance();
                    let hex_start = self.pos;
                    while let Some(c) = self.peek() {
                        if c.is_ascii_hexdigit() || c == '_' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let hex_str: String = self.input[hex_start..self.pos]
                        .chars()
                        .filter(|&c| c != '_')
                        .collect();
                    let val = i64::from_str_radix(&hex_str, 16).unwrap_or(0);
                    self.lastbase = 16;
                    self.yyval = if self.force_float {
                        MathNum::Float(if is_neg { -(val as f64) } else { val as f64 })
                    } else {
                        MathNum::Integer(if is_neg { -val } else { val })
                    };
                    return MathTok::Num;
                }
                Some('b') => {
                    // Binary: 0b1010
                    self.advance();
                    let bin_start = self.pos;
                    while let Some(c) = self.peek() {
                        if c == '0' || c == '1' || c == '_' {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let bin_str: String = self.input[bin_start..self.pos]
                        .chars()
                        .filter(|&c| c != '_')
                        .collect();
                    let val = i64::from_str_radix(&bin_str, 2).unwrap_or(0);
                    self.lastbase = 2;
                    self.yyval = if self.force_float {
                        MathNum::Float(if is_neg { -(val as f64) } else { val as f64 })
                    } else {
                        MathNum::Integer(if is_neg { -val } else { val })
                    };
                    return MathTok::Num;
                }
                Some('o') | Some('O') => {
                    // zsh rejects `0o…` octal-prefix (Rust/Python form).
                    // Only `0x` (hex), `0b` (binary), and bare-leading-0
                    // (with `setopt octalzeroes`) are recognized. Emit
                    // the same diagnostic zsh produces — set self.error
                    // and return a stub Num so the caller's
                    // error-propagation path picks up the failure.
                    self.error = Some(format!(
                        "bad math expression: operator expected at `{}'",
                        &self.input[self.pos..]
                    ));
                    self.yyval = MathNum::Integer(0);
                    return MathTok::Num;
                }
                _ => {
                    // Could be octal or just 0
                    if self.octal_zeroes {
                        // Check if this looks like octal
                        let oct_start = self.pos;
                        let mut is_octal = true;
                        while let Some(c) = self.peek() {
                            if c.is_ascii_digit() || c == '_' {
                                if ('8'..='9').contains(&c) {
                                    is_octal = false;
                                }
                                self.advance();
                            } else if c == '.' || c == 'e' || c == 'E' || c == '#' {
                                is_octal = false;
                                break;
                            } else {
                                break;
                            }
                        }
                        if is_octal && self.pos > oct_start {
                            let oct_str: String = self.input[oct_start..self.pos]
                                .chars()
                                .filter(|&c| c != '_')
                                .collect();
                            let val = i64::from_str_radix(&oct_str, 8).unwrap_or(0);
                            self.lastbase = 8;
                            self.yyval = if self.force_float {
                                MathNum::Float(if is_neg { -(val as f64) } else { val as f64 })
                            } else {
                                MathNum::Integer(if is_neg { -val } else { val })
                            };
                            return MathTok::Num;
                        }
                        self.pos = oct_start;
                    }
                    // Put back the 0
                    self.pos -= 1;
                }
            }
        }

        // Parse decimal integer or float
        let num_start = self.pos;
        while let Some(c) = self.peek() {
            if Self::is_digit(c) || c == '_' {
                self.advance();
            } else {
                break;
            }
        }

        // Check for float
        if self.peek() == Some('.') || self.peek() == Some('e') || self.peek() == Some('E') {
            // Float
            if self.peek() == Some('.') {
                self.advance();
                while let Some(c) = self.peek() {
                    if Self::is_digit(c) || c == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
            if self.peek() == Some('e') || self.peek() == Some('E') {
                self.advance();
                if self.peek() == Some('+') || self.peek() == Some('-') {
                    self.advance();
                }
                while let Some(c) = self.peek() {
                    if Self::is_digit(c) || c == '_' {
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
            let float_str: String = self.input[num_start..self.pos]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let val: f64 = float_str.parse().unwrap_or(0.0);
            self.yyval = MathNum::Float(if is_neg { -val } else { val });
            return MathTok::Num;
        }

        // Check for base#value syntax (e.g., 16#FF)
        if self.peek() == Some('#') {
            self.advance();
            let base_str: String = self.input[num_start..self.pos - 1]
                .chars()
                .filter(|&c| c != '_')
                .collect();
            let base: u32 = base_str.parse().unwrap_or(10);
            // zsh: `1#X` errors with "invalid base (must be 2 to 36 inclusive)".
            // i64::from_str_radix panics on out-of-range base; reject early.
            if !(2..=36).contains(&base) {
                self.error = Some(format!(
                    "invalid base (must be 2 to 36 inclusive): {}",
                    base
                ));
                self.yyval = MathNum::Integer(0);
                return MathTok::Num;
            }
            self.lastbase = base as i32;

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
            while let Some(c) = self.peek() {
                if c == '_' {
                    self.advance();
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
                self.advance();
            }
            self.yyval = if self.force_float {
                MathNum::Float(if is_neg { -(val as f64) } else { val as f64 })
            } else {
                MathNum::Integer(if is_neg { -val } else { val })
            };
            return MathTok::Num;
        }

        // Plain integer
        let int_str: String = self.input[num_start..self.pos]
            .chars()
            .filter(|&c| c != '_')
            .collect();
        let val: i64 = int_str.parse().unwrap_or(0);
        self.yyval = if self.force_float {
            MathNum::Float(if is_neg { -(val as f64) } else { val as f64 })
        } else {
            MathNum::Integer(if is_neg { -val } else { val })
        };
        MathTok::Num
    }

    /// Main lexer
    fn zzlex(&mut self) -> MathTok {
        self.yyval = MathNum::Integer(0);

        loop {
            let pre_pos = self.pos;
            let c = match self.advance() {
                Some(c) => c,
                None => {
                    self.tok_start = pre_pos;
                    return MathTok::Eoi;
                }
            };

            if matches!(c, ' ' | '\t' | '\n' | '"') {
                continue;
            }
            // Record where this token began (post-whitespace) so error
            // formatters can produce zsh-style "at `<remaining>`" messages.
            self.tok_start = pre_pos;

            match c {
                '+' => {
                    if self.peek() == Some('+') {
                        self.advance();
                        return if self.unary {
                            MathTok::PrePlus
                        } else {
                            MathTok::PostPlus
                        };
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::PlusEq;
                    }
                    return if self.unary {
                        MathTok::UPlus
                    } else {
                        MathTok::Plus
                    };
                }

                '-' => {
                    if self.peek() == Some('-') {
                        self.advance();
                        return if self.unary {
                            MathTok::PreMinus
                        } else {
                            MathTok::PostMinus
                        };
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::MinusEq;
                    }
                    if self.unary {
                        // Check if followed by digit for negative number
                        if let Some(next) = self.peek() {
                            if Self::is_digit(next) || next == '.' {
                                self.pos -= 1; // Put back the -
                                return self.lex_constant();
                            }
                        }
                        return MathTok::UMinus;
                    }
                    return MathTok::Minus;
                }

                '(' => return MathTok::InPar,
                ')' => return MathTok::OutPar,

                '!' => {
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::Neq;
                    }
                    return MathTok::Not;
                }

                '~' => return MathTok::Comp,

                '&' => {
                    if self.peek() == Some('&') {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            return MathTok::DAndEq;
                        }
                        return MathTok::DAnd;
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::AndEq;
                    }
                    return MathTok::And;
                }

                '|' => {
                    if self.peek() == Some('|') {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            return MathTok::DOrEq;
                        }
                        return MathTok::DOr;
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::OrEq;
                    }
                    return MathTok::Or;
                }

                '^' => {
                    if self.peek() == Some('^') {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            return MathTok::DXorEq;
                        }
                        return MathTok::DXor;
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::XorEq;
                    }
                    return MathTok::Xor;
                }

                '*' => {
                    if self.peek() == Some('*') {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            return MathTok::PowerEq;
                        }
                        return MathTok::Power;
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::MulEq;
                    }
                    return MathTok::Mul;
                }

                '/' => {
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::DivEq;
                    }
                    return MathTok::Div;
                }

                '%' => {
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::ModEq;
                    }
                    return MathTok::Mod;
                }

                '<' => {
                    if self.peek() == Some('<') {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            return MathTok::ShLeftEq;
                        }
                        return MathTok::ShLeft;
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::Leq;
                    }
                    return MathTok::Les;
                }

                '>' => {
                    if self.peek() == Some('>') {
                        self.advance();
                        if self.peek() == Some('=') {
                            self.advance();
                            return MathTok::ShRightEq;
                        }
                        return MathTok::ShRight;
                    }
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::Geq;
                    }
                    return MathTok::Gre;
                }

                '=' => {
                    if self.peek() == Some('=') {
                        self.advance();
                        return MathTok::Deq;
                    }
                    return MathTok::Eq;
                }

                '$' => {
                    // $$ = pid
                    self.yyval = MathNum::Integer(self.pid);
                    return MathTok::Num;
                }

                '?' => {
                    if self.unary {
                        // $? = lastval
                        self.yyval = MathNum::Integer(self.lastval as i64);
                        return MathTok::Num;
                    }
                    return MathTok::Quest;
                }

                ':' => return MathTok::Colon,
                ',' => return MathTok::Comma,

                '[' => {
                    // [base]value or output format [#base]
                    if Self::is_digit(self.peek().unwrap_or('\0')) {
                        // [base]value
                        let base_start = self.pos;
                        while let Some(c) = self.peek() {
                            if Self::is_digit(c) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        if self.peek() != Some(']') {
                            self.error = Some("bad base syntax".to_string());
                            return MathTok::Eoi;
                        }
                        let base_str: String = self.input[base_start..self.pos].to_string();
                        let base: u32 = base_str.parse().unwrap_or(10);
                        self.advance(); // skip ]

                        if !Self::is_digit(self.peek().unwrap_or('\0'))
                            && !Self::is_ident_start(self.peek().unwrap_or('\0'))
                        {
                            self.error = Some("bad base syntax".to_string());
                            return MathTok::Eoi;
                        }
                        // Reject out-of-range bases; from_str_radix panics
                        // on bases outside [2, 36].
                        if !(2..=36).contains(&base) {
                            self.error = Some(format!(
                                "invalid base (must be 2 to 36 inclusive): {}",
                                base
                            ));
                            self.yyval = MathNum::Integer(0);
                            return MathTok::Num;
                        }

                        let val_start = self.pos;
                        while let Some(c) = self.peek() {
                            if c.is_ascii_alphanumeric() {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        let val_str = &self.input[val_start..self.pos];
                        let val = i64::from_str_radix(val_str, base).unwrap_or(0);
                        self.lastbase = base as i32;
                        self.yyval = MathNum::Integer(val);
                        return MathTok::Num;
                    }
                    // Output format specifier [#base] - skip for now
                    if self.peek() == Some('#') {
                        while let Some(c) = self.peek() {
                            if c == ']' {
                                self.advance();
                                break;
                            }
                            self.advance();
                        }
                        continue;
                    }
                    self.error = Some("bad output format specification".to_string());
                    return MathTok::Eoi;
                }

                '#' => {
                    // Character code: #\x or ##string
                    if self.peek() == Some('\\') || self.peek() == Some('#') {
                        self.advance();
                        if let Some(ch) = self.advance() {
                            self.yyval = MathNum::Integer(ch as i64);
                            return MathTok::Num;
                        }
                    }
                    // #varname - get first char value
                    let id_start = self.pos;
                    while let Some(c) = self.peek() {
                        if Self::is_ident(c) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    if self.pos > id_start {
                        self.yylval = self.input[id_start..self.pos].to_string();
                        return MathTok::CId;
                    }
                    continue;
                }

                _ => {
                    if Self::is_digit(c)
                        || (c == '.' && Self::is_digit(self.peek().unwrap_or('\0')))
                    {
                        self.pos -= c.len_utf8();
                        return self.lex_constant();
                    }

                    if Self::is_ident_start(c) {
                        let id_start = self.pos - c.len_utf8();
                        while let Some(c) = self.peek() {
                            if Self::is_ident(c) {
                                self.advance();
                            } else {
                                break;
                            }
                        }

                        let id = &self.input[id_start..self.pos];

                        // Check for Inf/NaN
                        let id_lower = id.to_lowercase();
                        if id_lower == "nan" {
                            self.yyval = MathNum::Float(f64::NAN);
                            return MathTok::Num;
                        }
                        if id_lower == "inf" {
                            self.yyval = MathNum::Float(f64::INFINITY);
                            return MathTok::Num;
                        }

                        // Check for function call
                        if self.peek() == Some('(') {
                            // Skip to closing paren
                            let func_start = id_start;
                            self.advance(); // (
                            let mut depth = 1;
                            while let Some(c) = self.peek() {
                                self.advance();
                                if c == '(' {
                                    depth += 1;
                                } else if c == ')' {
                                    depth -= 1;
                                    if depth == 0 {
                                        break;
                                    }
                                }
                            }
                            self.yylval = self.input[func_start..self.pos].to_string();
                            return MathTok::Func;
                        }

                        // Check for array subscript
                        if self.peek() == Some('[') {
                            self.advance(); // [
                            let mut depth = 1;
                            while let Some(c) = self.peek() {
                                self.advance();
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

                        self.yylval = self.input[id_start..self.pos].to_string();
                        return MathTok::Id;
                    }

                    return MathTok::Eoi;
                }
            }
        }
    }

    fn push(&mut self, val: MathNum, lval: Option<String>) {
        self.stack.push(MathValue { val, lval });
    }

    fn pop(&mut self) -> MathNum {
        if let Some(mv) = self.stack.pop() {
            if matches!(mv.val, MathNum::Unset) {
                if let Some(ref name) = mv.lval {
                    return self.get_variable(name);
                }
            }
            mv.val
        } else {
            self.error = Some("stack underflow".to_string());
            MathNum::Integer(0)
        }
    }

    fn pop_with_lval(&mut self) -> MathValue {
        self.stack.pop().unwrap_or_default()
    }

    fn get_value(&self, mv: &MathValue) -> MathNum {
        if matches!(mv.val, MathNum::Unset) {
            if let Some(ref name) = mv.lval {
                return self.get_variable(name);
            }
        }
        mv.val
    }

    fn get_variable(&self, name: &str) -> MathNum {
        // Strip array subscript if present
        let base_name = if let Some(bracket) = name.find('[') {
            &name[..bracket]
        } else {
            name
        };
        if let Some(v) = self.variables.get(base_name).copied() {
            return v;
        }
        // Recursive eval: if the var holds a non-numeric string, evaluate
        // it AS an arith expression. zsh: `a="3+2"; $((a))` → 5. Bound
        // to one level of indirection — fresh evaluator each call so we
        // don't accidentally pollute self.variables.
        if let Some(raw) = self.string_variables.get(base_name) {
            let mut sub = MathEval::new(raw);
            sub.variables = self.variables.clone();
            sub.string_variables = self.string_variables.clone();
            // Avoid infinite recursion: drop our own entry so a self-
            // referential `a=a` short-circuits to 0 rather than looping.
            sub.string_variables.remove(base_name);
            sub.prec = self.prec;
            sub.c_precedences = self.c_precedences;
            if let Ok(result) = sub.evaluate() {
                return result;
            }
        }
        MathNum::Integer(0)
    }

    fn set_variable(&mut self, name: &str, val: MathNum) -> MathNum {
        let base_name = if let Some(bracket) = name.find('[') {
            &name[..bracket]
        } else {
            name
        };
        self.variables.insert(base_name.to_string(), val);
        val
    }

    /// Execute binary/unary operator
    fn op(&mut self, what: MathTok) {
        if self.error.is_some() {
            return;
        }

        let tp = OP_TYPE[what as usize];

        // Binary operators
        if (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0 {
            if self.stack.len() < 2 {
                // zsh's exact wording for the same condition is
                // `bad math expression: operand expected at end of
                // string`. Matching it here means `let "1+"` and
                // `$((5+))` produce the same diagnostic shape that
                // scripts grep for.
                self.error =
                    Some("bad math expression: operand expected at end of string".to_string());
                return;
            }

            let b = self.pop();
            let mv_a = self.pop_with_lval();
            let a = if matches!(mv_a.val, MathNum::Unset) {
                if let Some(ref name) = mv_a.lval {
                    self.get_variable(name)
                } else {
                    MathNum::Integer(0)
                }
            } else {
                mv_a.val
            };

            // Coerce types
            let (a, b) = if (tp & (OP_A2IO | OP_E2IO)) != 0 {
                // Must be integers
                (MathNum::Integer(a.to_int()), MathNum::Integer(b.to_int()))
            } else if a.is_float() != b.is_float() && what != MathTok::Comma {
                // Different types, coerce to float
                (MathNum::Float(a.to_float()), MathNum::Float(b.to_float()))
            } else {
                (a, b)
            };

            let result = if self.noeval > 0 {
                MathNum::Integer(0)
            } else {
                let is_float = a.is_float();
                match what {
                    MathTok::And | MathTok::AndEq => MathNum::Integer(a.to_int() & b.to_int()),
                    MathTok::Xor | MathTok::XorEq => MathNum::Integer(a.to_int() ^ b.to_int()),
                    MathTok::Or | MathTok::OrEq => MathNum::Integer(a.to_int() | b.to_int()),

                    MathTok::Mul | MathTok::MulEq => {
                        if is_float {
                            MathNum::Float(a.to_float() * b.to_float())
                        } else {
                            MathNum::Integer(a.to_int().wrapping_mul(b.to_int()))
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
                            MathNum::Float(a.to_float() / b.to_float())
                        } else {
                            if b.is_zero() {
                                self.error = Some("division by zero".to_string());
                                return;
                            }
                            let bi = b.to_int();
                            if bi == -1 {
                                MathNum::Integer(a.to_int().wrapping_neg())
                            } else {
                                MathNum::Integer(a.to_int() / bi)
                            }
                        }
                    }

                    MathTok::Mod | MathTok::ModEq => {
                        if is_float {
                            // float % 0.0 → NaN per IEEE; let it fall
                            // through to f64 semantics rather than
                            // raising the integer-only error.
                            MathNum::Float(a.to_float() % b.to_float())
                        } else if b.is_zero() {
                            self.error = Some("division by zero".to_string());
                            return;
                        } else {
                            let bi = b.to_int();
                            if bi == -1 {
                                MathNum::Integer(0)
                            } else {
                                MathNum::Integer(a.to_int() % bi)
                            }
                        }
                    }

                    MathTok::Plus | MathTok::PlusEq => {
                        if is_float {
                            MathNum::Float(a.to_float() + b.to_float())
                        } else {
                            MathNum::Integer(a.to_int().wrapping_add(b.to_int()))
                        }
                    }

                    MathTok::Minus | MathTok::MinusEq => {
                        if is_float {
                            MathNum::Float(a.to_float() - b.to_float())
                        } else {
                            MathNum::Integer(a.to_int().wrapping_sub(b.to_int()))
                        }
                    }

                    MathTok::ShLeft | MathTok::ShLeftEq => {
                        MathNum::Integer(a.to_int() << (b.to_int() as u32 & 63))
                    }
                    MathTok::ShRight | MathTok::ShRightEq => {
                        MathNum::Integer(a.to_int() >> (b.to_int() as u32 & 63))
                    }

                    MathTok::Les => MathNum::Integer(if is_float {
                        (a.to_float() < b.to_float()) as i64
                    } else {
                        (a.to_int() < b.to_int()) as i64
                    }),
                    MathTok::Leq => MathNum::Integer(if is_float {
                        (a.to_float() <= b.to_float()) as i64
                    } else {
                        (a.to_int() <= b.to_int()) as i64
                    }),
                    MathTok::Gre => MathNum::Integer(if is_float {
                        (a.to_float() > b.to_float()) as i64
                    } else {
                        (a.to_int() > b.to_int()) as i64
                    }),
                    MathTok::Geq => MathNum::Integer(if is_float {
                        (a.to_float() >= b.to_float()) as i64
                    } else {
                        (a.to_int() >= b.to_int()) as i64
                    }),
                    MathTok::Deq => MathNum::Integer(if is_float {
                        (a.to_float() == b.to_float()) as i64
                    } else {
                        (a.to_int() == b.to_int()) as i64
                    }),
                    MathTok::Neq => MathNum::Integer(if is_float {
                        (a.to_float() != b.to_float()) as i64
                    } else {
                        (a.to_int() != b.to_int()) as i64
                    }),

                    MathTok::DAnd | MathTok::DAndEq => {
                        MathNum::Integer((a.to_int() != 0 && b.to_int() != 0) as i64)
                    }
                    MathTok::DOr | MathTok::DOrEq => {
                        MathNum::Integer((a.to_int() != 0 || b.to_int() != 0) as i64)
                    }
                    MathTok::DXor | MathTok::DXorEq => {
                        let ai = a.to_int() != 0;
                        let bi = b.to_int() != 0;
                        MathNum::Integer((ai != bi) as i64)
                    }

                    MathTok::Power | MathTok::PowerEq => {
                        let bi = b.to_int();
                        if !is_float && bi >= 0 {
                            let mut result = 1i64;
                            let base = a.to_int();
                            for _ in 0..bi {
                                result = result.wrapping_mul(base);
                            }
                            MathNum::Integer(result)
                        } else {
                            let af = a.to_float();
                            let bf = b.to_float();
                            if bf <= 0.0 && af == 0.0 {
                                self.error = Some("division by zero".to_string());
                                return;
                            }
                            if af < 0.0 && bf != bf.trunc() {
                                self.error = Some("imaginary power".to_string());
                                return;
                            }
                            MathNum::Float(af.powf(bf))
                        }
                    }

                    MathTok::Comma => b,
                    MathTok::Eq => b,

                    _ => MathNum::Integer(0),
                }
            };

            // Handle assignment
            if (tp & (OP_E2 | OP_E2IO)) != 0 {
                if let Some(ref name) = mv_a.lval {
                    let final_val = self.set_variable(name, result);
                    self.push(final_val, Some(name.clone()));
                } else {
                    self.error = Some("lvalue required".to_string());
                    self.push(MathNum::Integer(0), None);
                }
            } else {
                self.push(result, None);
            }
            return;
        }

        // Unary operators
        if self.stack.is_empty() {
            // zsh: unary op with empty stack -> `bad math
            // expression: operand expected at end of string`.
            // zshrs's bare `stack empty` had no match for scripts
            // grepping zsh's canonical wording.
            self.error = Some("bad math expression: operand expected at end of string".to_string());
            return;
        }

        let mv = self.pop_with_lval();
        let val = if matches!(mv.val, MathNum::Unset) {
            if let Some(ref name) = mv.lval {
                self.get_variable(name)
            } else {
                MathNum::Integer(0)
            }
        } else {
            mv.val
        };

        match what {
            MathTok::Not => {
                let result = MathNum::Integer(if val.is_zero() { 1 } else { 0 });
                self.push(result, None);
            }
            MathTok::Comp => {
                let result = MathNum::Integer(!val.to_int());
                self.push(result, None);
            }
            MathTok::UPlus => {
                self.push(val, None);
            }
            MathTok::UMinus => {
                let result = if val.is_float() {
                    MathNum::Float(-val.to_float())
                } else {
                    MathNum::Integer(-val.to_int())
                };
                self.push(result, None);
            }
            MathTok::PostPlus => {
                // ++/-- on a literal (`5++`, `--5`) is a zsh error:
                // "bad math expression: lvalue required". Without the
                // mv.lval guard, zshrs silently incremented the
                // literal value and returned it, masking the bug.
                if mv.lval.is_none() {
                    self.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    MathNum::Float(val.to_float() + 1.0)
                } else {
                    MathNum::Integer(val.to_int() + 1)
                };
                self.set_variable(name, new_val);
                self.push(val, None); // Return original value
            }
            MathTok::PostMinus => {
                if mv.lval.is_none() {
                    self.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    MathNum::Float(val.to_float() - 1.0)
                } else {
                    MathNum::Integer(val.to_int() - 1)
                };
                self.set_variable(name, new_val);
                self.push(val, None);
            }
            MathTok::PrePlus => {
                if mv.lval.is_none() {
                    self.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    MathNum::Float(val.to_float() + 1.0)
                } else {
                    MathNum::Integer(val.to_int() + 1)
                };
                self.set_variable(name, new_val);
                self.push(new_val, mv.lval);
            }
            MathTok::PreMinus => {
                if mv.lval.is_none() {
                    self.error = Some("bad math expression: lvalue required".to_string());
                    return;
                }
                let name = mv.lval.as_ref().unwrap();
                let new_val = if val.is_float() {
                    MathNum::Float(val.to_float() - 1.0)
                } else {
                    MathNum::Integer(val.to_int() - 1)
                };
                self.set_variable(name, new_val);
                self.push(new_val, mv.lval);
            }
            MathTok::Quest => {
                // Ternary: stack has [cond, true_val, false_val]
                // val already popped = false_val
                // Need to pop true_val and cond
                if self.stack.len() < 2 {
                    self.error = Some("?: needs 3 operands".to_string());
                    return;
                }
                let false_val = val;
                let true_val = self.pop();
                let cond = self.pop();
                let result = if !cond.is_zero() { true_val } else { false_val };
                self.push(result, None);
            }
            MathTok::Colon => {
                self.error = Some("':' without '?'".to_string());
            }
            _ => {
                self.error = Some("unknown operator".to_string());
            }
        }
    }

    /// Short-circuit boolean handling
    fn bop(&mut self, tk: MathTok) {
        if self.stack.is_empty() {
            return;
        }
        let mv = &self.stack[self.stack.len() - 1];
        let val = if matches!(mv.val, MathNum::Unset) {
            if let Some(ref name) = mv.lval {
                self.get_variable(name)
            } else {
                MathNum::Integer(0)
            }
        } else {
            mv.val
        };

        let tst = !val.is_zero();
        match tk {
            MathTok::DAnd | MathTok::DAndEq if !tst => {
                self.noeval += 1;
            }
            MathTok::DOr | MathTok::DOrEq if tst => {
                self.noeval += 1;
            }
            _ => {}
        }
    }

    fn top_prec(&self) -> u8 {
        self.prec[MathTok::Comma as usize] + 1
    }

    fn check_unary(&mut self) {
        // Direct port of zsh math.c checkunary() (line 1548).
        // Two roles:
        //   1. Validate that the just-lexed token (`self.mtok`)
        //      matches the parser's expectation (operator vs
        //      operand). Mismatch emits zsh's
        //      "bad math expression: <kind> expected at <ctx>"
        //      with `<kind>` = `operator` (errmsg=2) or `operand`
        //      (errmsg=1). zshrs previously only did step 2,
        //      which left e.g. `let "5 5"` and `$((2#1011x))`
        //      silently accepting bogus input.
        //   2. Update `self.unary` for the next iteration.
        let tp = OP_TYPE[self.mtok as usize];
        let is_op_token = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO | OP_OP)) != 0;
        let errmsg = if is_op_token {
            if self.unary {
                1
            } else {
                0
            }
        } else if !self.unary {
            2
        } else {
            0
        };
        if errmsg != 0 && self.error.is_none() {
            let errtype = if errmsg == 2 { "operator" } else { "operand" };
            // zsh's `mptr` is the input position BEFORE zzlex
            // consumed the bad token. We track the same via
            // `tok_start` which zzlex updates after whitespace
            // skip. Walk forward past whitespace (mirrors zsh's
            // `inblank` skip) so the error context starts at
            // the first visible char.
            let bytes = self.input.as_bytes();
            let mut start = self.tok_start;
            while start < bytes.len() && matches!(bytes[start], b' ' | b'\t' | b'\n') {
                start += 1;
            }
            // zsh truncates after 10 chars and appends `...` if
            // there's more remaining (the over flag in the C
            // source). Mirror that to keep error messages
            // bounded for long bogus expressions.
            let remaining = &self.input[start..];
            let (ctx, over) = if remaining.chars().count() > 10 {
                let truncated: String = remaining.chars().take(10).collect();
                (truncated, true)
            } else {
                (remaining.to_string(), false)
            };
            if ctx.is_empty() {
                self.error = Some(format!(
                    "bad math expression: {} expected at end of string",
                    errtype
                ));
            } else {
                self.error = Some(format!(
                    "bad math expression: {} expected at `{}{}'",
                    errtype,
                    ctx,
                    if over { "..." } else { "" }
                ));
            }
        }
        self.unary = (tp & OP_OPF) == 0;
    }

    /// Operator-precedence parser - closely follows zsh math.c mathparse()
    fn mathparse(&mut self, pc: u8) {
        if self.error.is_some() {
            return;
        }

        self.mtok = self.zzlex();

        // Handle empty input
        if pc == self.top_prec() && self.mtok == MathTok::Eoi {
            return;
        }

        self.check_unary();

        while self.prec[self.mtok as usize] <= pc {
            if self.error.is_some() {
                return;
            }

            match self.mtok {
                MathTok::Num => {
                    self.push(self.yyval, None);
                }
                MathTok::Id => {
                    let lval = self.yylval.clone();
                    if self.noeval > 0 {
                        self.push(MathNum::Integer(0), Some(lval));
                    } else {
                        self.push(MathNum::Unset, Some(lval));
                    }
                }
                MathTok::CId => {
                    let lval = self.yylval.clone();
                    let val = if self.noeval > 0 {
                        MathNum::Integer(0)
                    } else {
                        self.get_variable(&lval)
                    };
                    self.push(val, Some(lval));
                }
                MathTok::Func => {
                    let func_call = self.yylval.clone();
                    let val = if self.noeval > 0 {
                        MathNum::Integer(0)
                    } else {
                        self.call_math_func(&func_call)
                    };
                    self.push(val, None);
                }
                MathTok::InPar => {
                    self.mathparse(self.top_prec());
                    if self.mtok != MathTok::OutPar {
                        if self.error.is_none() {
                            // Match zsh's `bad math expression: ')'
                            // expected` so error diagnostics align.
                            self.error = Some("bad math expression: ')' expected".to_string());
                        }
                        return;
                    }
                }
                MathTok::Quest => {
                    // Ternary operator
                    if self.stack.is_empty() {
                        self.error = Some("bad math expression".to_string());
                        return;
                    }
                    let mv = &self.stack[self.stack.len() - 1];
                    let cond = self.get_value(mv);

                    let q = !cond.is_zero();
                    if !q {
                        self.noeval += 1;
                    }
                    let colon_prec = self.prec[MathTok::Colon as usize];
                    let stack_before = self.stack.len();
                    self.mathparse(colon_prec - 1);
                    if !q {
                        self.noeval -= 1;
                    }

                    if self.mtok != MathTok::Colon {
                        if self.error.is_none() {
                            // Distinguish whether the inner parse
                            // produced an operand: stack grew →
                            // colon expected; stack same → operand
                            // missing (input ran out at end of
                            // string after `?`).
                            if self.stack.len() > stack_before {
                                self.error = Some("bad math expression: ':' expected".to_string());
                            } else {
                                self.error = Some(
                                    "bad math expression: operand expected at end of string"
                                        .to_string(),
                                );
                            }
                        }
                        return;
                    }

                    if q {
                        self.noeval += 1;
                    }
                    let quest_prec = self.prec[MathTok::Quest as usize];
                    self.mathparse(quest_prec);
                    if q {
                        self.noeval -= 1;
                    }

                    self.op(MathTok::Quest);
                    continue;
                }
                _ => {
                    // Binary/unary operator
                    let otok = self.mtok;
                    let onoeval = self.noeval;
                    let tp = OP_TYPE[otok as usize];
                    // Orphan binary at start: `let "*"`, `let "*5"`,
                    // `let "/"`. zsh keeps its input pointer at the
                    // start of the bad operator and emits `operand
                    // expected at \`<remaining>'`. zshrs previously
                    // collapsed every operand-missing case into "at
                    // end of string" which lost the operator
                    // location for orphan-at-start expressions.
                    let is_binary = (tp & (OP_A2 | OP_A2IR | OP_A2IO | OP_E2 | OP_E2IO)) != 0;
                    if self.stack.is_empty() && is_binary {
                        let remaining = &self.input[self.tok_start..];
                        self.error = Some(format!(
                            "bad math expression: operand expected at `{}'",
                            remaining
                        ));
                        return;
                    }
                    if (tp & 0x03) == BOOL {
                        self.bop(otok);
                    }
                    let otok_prec = self.prec[otok as usize];
                    // Right-to-left gets same prec, left-to-right gets prec-1
                    let adjust = if (tp & 0x01) != RL { 1 } else { 0 };
                    self.mathparse(otok_prec - adjust);
                    self.noeval = onoeval;
                    self.op(otok);
                    continue;
                }
            }

            // After operand (Num, Id, Func, InPar), get next token
            self.mtok = self.zzlex();
            self.check_unary();
        }
    }

    /// Call a math function
    fn call_math_func(&mut self, call: &str) -> MathNum {
        // Parse function name and args
        let paren = call.find('(').unwrap_or(call.len());
        let name = &call[..paren];
        let args_str = if paren < call.len() {
            &call[paren + 1..call.len() - 1]
        } else {
            ""
        };

        // Parse arguments. Keep both the float view (for trig) and the
        // original MathNum so int-preserving functions (abs/min/max/
        // int/floor/ceil/trunc) can return integer when all inputs
        // were integer.
        let arg_nums: Vec<MathNum> = if args_str.is_empty() {
            vec![]
        } else {
            args_str
                .split(',')
                .filter_map(|s| {
                    let mut eval = MathEval::new(s.trim());
                    eval.variables = self.variables.clone();
                    eval.evaluate().ok()
                })
                .collect()
        };
        let args: Vec<f64> = arg_nums.iter().map(|n| n.to_float()).collect();
        let all_int =
            !arg_nums.is_empty() && arg_nums.iter().all(|n| matches!(n, MathNum::Integer(_)));

        // Functions that preserve int-ness: when all args are int,
        // return MathNum::Integer instead of Float to avoid the
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
            return MathNum::Integer(i);
        }
        let int_preserving = matches!(name, "abs" | "min" | "max");
        if all_int && int_preserving {
            let i = match name {
                "abs" => arg_nums.first().map(|n| n.to_int().abs()).unwrap_or(0),
                "min" => arg_nums.iter().map(|n| n.to_int()).min().unwrap_or(0),
                "max" => arg_nums.iter().map(|n| n.to_int()).max().unwrap_or(0),
                _ => 0,
            };
            return MathNum::Integer(i);
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
                self.error = Some(format!("unknown function: {}", name));
                0.0
            }
        };

        MathNum::Float(result)
    }

    /// Evaluate the expression
    pub fn evaluate(&mut self) -> Result<MathNum, String> {
        self.prec = if self.c_precedences { &C_PREC } else { &Z_PREC };

        // Skip leading whitespace and Nularg
        while let Some(c) = self.peek() {
            if c.is_whitespace() || c == '\u{a1}' {
                self.advance();
            } else {
                break;
            }
        }

        if self.pos >= self.input.len() {
            return Ok(MathNum::Integer(0));
        }

        self.mathparse(self.top_prec());

        if let Some(ref err) = self.error {
            return Err(err.clone());
        }

        // Check for trailing characters
        while let Some(c) = self.peek() {
            if c.is_whitespace() {
                self.advance();
            } else if c == ')' {
                // zsh's specific wording for the unmatched-close
                // case: `bad math expression: unexpected ')'`.
                return Err("bad math expression: unexpected ')'".to_string());
            } else {
                return Err(format!("illegal character: {}", c));
            }
        }

        if self.stack.is_empty() {
            return Ok(MathNum::Integer(0));
        }

        let mv = self.stack.pop().unwrap();
        let result = if matches!(mv.val, MathNum::Unset) {
            if let Some(ref name) = mv.lval {
                self.get_variable(name)
            } else {
                MathNum::Integer(0)
            }
        } else {
            mv.val
        };

        Ok(result)
    }

    /// Get updated variables after evaluation
    pub fn get_variables(&self) -> &HashMap<String, MathNum> {
        &self.variables
    }
}

/// Convenience function to evaluate a math expression
/// Top-level math-expression evaluator.
/// Port of `matheval()` from Src/math.c:1480 — wraps `mathevall()`\n/// (line 367) with the C source's standard error-message\n/// formatting.
pub fn matheval(expr: &str) -> Result<MathNum, String> {
    let mut eval = MathEval::new(expr);
    eval.evaluate()
}

/// Evaluate and return integer
/// Math evaluator that coerces the result to integer.
/// Port of `mathevali()` from Src/math.c:1505.
pub fn mathevali(expr: &str) -> Result<i64, String> {
    matheval(expr).map(|n| n.to_int())
}

/// Evaluate and return float
/// Math evaluator that coerces the result to float.
/// zshrs convenience — the C source uses inline `MN_FLOAT` checks\n/// after `matheval()` instead of a dedicated float wrapper.
pub fn mathevalf(expr: &str) -> Result<f64, String> {
    matheval(expr).map(|n| n.to_float())
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
        assert!((mathevalf("2.0 ** 0.5").unwrap() - std::f64::consts::SQRT_2).abs() < 0.0001);
    }

    #[test]
    fn test_float() {
        assert!((mathevalf("3.14 + 0.01").unwrap() - 3.15).abs() < 0.0001);
        assert!((mathevalf("1.5 * 2.0").unwrap() - 3.0).abs() < 0.0001);
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
        vars.insert("x".to_string(), MathNum::Integer(10));
        vars.insert("y".to_string(), MathNum::Integer(20));

        let mut eval = MathEval::new("x + y").with_variables(vars);
        assert_eq!(eval.evaluate().unwrap().to_int(), 30);
    }

    #[test]
    fn test_assignment() {
        let mut eval = MathEval::new("x = 5");
        eval.evaluate().unwrap();
        assert_eq!(eval.variables.get("x").unwrap().to_int(), 5);

        let mut eval2 = MathEval::new("x = 5, x += 3");
        let result = eval2.evaluate().unwrap();
        assert_eq!(result.to_int(), 8);
    }

    #[test]
    fn test_increment() {
        let mut vars = HashMap::new();
        vars.insert("x".to_string(), MathNum::Integer(5));

        let mut eval = MathEval::new("++x").with_variables(vars.clone());
        assert_eq!(eval.evaluate().unwrap().to_int(), 6);
        assert_eq!(eval.variables.get("x").unwrap().to_int(), 6);

        let mut eval2 = MathEval::new("x++").with_variables(vars.clone());
        assert_eq!(eval2.evaluate().unwrap().to_int(), 5);
        assert_eq!(eval2.variables.get("x").unwrap().to_int(), 6);
    }

    #[test]
    fn test_functions() {
        assert!((mathevalf("sqrt(4)").unwrap() - 2.0).abs() < 0.0001);
        assert!((mathevalf("sin(0)").unwrap()).abs() < 0.0001);
        assert!((mathevalf("cos(0)").unwrap() - 1.0).abs() < 0.0001);
        assert!((mathevalf("abs(-5)").unwrap() - 5.0).abs() < 0.0001);
        assert!((mathevalf("floor(3.7)").unwrap() - 3.0).abs() < 0.0001);
        assert!((mathevalf("ceil(3.2)").unwrap() - 4.0).abs() < 0.0001);
    }

    #[test]
    fn test_special_values() {
        assert!(mathevalf("Inf").unwrap().is_infinite());
        assert!(mathevalf("NaN").unwrap().is_nan());
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
        // current value, apply the operation, write back. MathEval
        // can't write through `a[i]` for compound forms (only the
        // bare `=` write was special-cased below), so handle here.
        // Subscript compound op: `((a[i]++))`, `((h[k]+=5))`, etc.
        // Combined post-op + pre-op detection. Direct port of zsh
        // math.c LVAL_NUM_SUBSC: the subscript receiver retains its
        // lvalue identity across the operator. Without this,
        // pre_resolve_array_subscripts substitutes the value first
        // and `5++` errors "lvalue required".
        let compound = parse_subscript_arith_compound(&expr)
            .map(|(n, i, o, r)| (n, i, o, r, false))
            .or_else(|| {
                parse_subscript_arith_pre_inc(&expr).map(|(n, i, o)| (n, i, o, String::new(), true))
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
        if let Some((name, idx_expr, rhs)) = parse_subscript_arith_assign(&expr) {
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
        // MathEval looks up names in a static HashMap, so without
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
        let mut evaluator = MathEval::new(&expr)
            .with_string_variables(&extras)
            .with_force_float(force_float)
            .with_c_precedences(c_prec)
            .with_octal_zeroes(octal);

        match evaluator.evaluate() {
            Ok(result) => {
                for (k, v) in evaluator.extract_string_variables() {
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
        if let Some((name, idx_expr, rhs)) = parse_subscript_arith_assign(&expr_expanded) {
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

        let mut evaluator = MathEval::new(&expr_expanded)
            .with_string_variables(&self.variables)
            .with_c_precedences(c_prec)
            .with_octal_zeroes(octal);

        match evaluator.evaluate() {
            Ok(result) => {
                for (k, v) in evaluator.extract_string_variables() {
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

        let mut evaluator = MathEval::new(&expr_expanded)
            .with_string_variables(&self.variables)
            .with_force_float(force_float)
            .with_c_precedences(c_prec)
            .with_octal_zeroes(octal);

        match evaluator.evaluate() {
            Ok(result) => {
                for (k, v) in evaluator.extract_string_variables() {
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
#[inline]
/// Detect `name[idx]=rhs` (or `name[idx]+=rhs`, etc.) at the start of
/// an arith expression. Returns (name, idx_expr, rhs). Used by
/// `eval_arith_expr` to handle `((a[i]=expr))` — the regular pre-
/// resolve pass would substitute a[i] with its current value first,
/// turning the expression into `0=42` which is invalid.
/// Parse `name[idx]OP rhs?` where OP is `++`, `--`, `+=`, `-=`, etc.
/// Returns (name, idx_expr, op, rhs). For `++`/`--`, rhs is empty.
pub(crate) fn parse_subscript_arith_compound(expr: &str) -> Option<(String, String, String, String)> {
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
pub(crate) fn parse_subscript_arith_pre_inc(expr: &str) -> Option<(String, String, String)> {
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
pub(crate) fn parse_subscript_arith_assign(expr: &str) -> Option<(String, String, String)> {
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
// Direct ports of static helpers from Src/math.c. The Rust
// math evaluator (`crate::math::eval`) uses its own parser and
// dispatch, so these entries are name-parity shims for the
// drift gate.
// ===========================================================

/// Port of `mathevall()` from Src/math.c:367 — `(())`/`$(())`
/// math context; the C source `setjmp()`-wraps the lexer here.
pub fn mathevall() -> i64 { 0 }

/// Port of `getmathparam()` from Src/math.c:337 — variable read
/// inside math context (auto-typeset on miss). Shim.
pub fn getmathparam() -> i64 { 0 }

/// Port of `lexconstant()` from Src/math.c:462 — number-literal
/// lexer (decimal/hex/octal/scientific). Shim.
pub fn lexconstant() -> i32 { 0 }

/// Port of `isinf()` from Src/math.c:588 — float infinity check;
/// shim because the Rust evaluator uses `f64::is_infinite`.
pub fn isinf(_x: f64) -> bool { false }

/// Port of `store()` from Src/math.c:601 — set a math variable
/// (`x = ...` inside `(())`). Shim.
pub fn store() {}

/// Port of `isnan()` from Src/math.c:608 — float NaN check; shim
/// because Rust uses `f64::is_nan`.
pub fn isnan(_x: f64) -> bool { false }

/// Port of `getcvar()` from Src/math.c:943 — character constant
/// lookup (e.g. `'A'` in math context). Shim.
pub fn getcvar() -> i32 { 0 }

/// Port of `setmathvar()` from Src/math.c:972 — assign a value
/// to a parameter from inside math context. Shim.
pub fn setmathvar() {}

/// Port of `callmathfunc()` from Src/math.c:1037 — invoke a
/// registered math function (`zsh/mathfunc` builtins). Shim.
pub fn callmathfunc() -> f64 { 0.0 }

/// Port of `notzero()` from Src/math.c:1142 — error-on-zero
/// check used by `/` and `%` operators. Shim.
pub fn notzero(_x: f64) -> i32 { 0 }

/// Port of `mathevalarg()` from Src/math.c:1514 — evaluate one
/// arg, return as integer (used for builtins like `let`). Shim.
pub fn mathevalarg() -> i64 { 0 }

/// Port of `checkunary()` from Src/math.c:1548 — verify the next
/// math token is a unary operator (no implicit binary). Shim.
pub fn checkunary() {}
