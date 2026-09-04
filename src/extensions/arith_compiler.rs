//! ArithCompiler — lowers zsh arithmetic expressions
//! (`$((...))`) into fusevm bytecodes. Used by `ZshCompiler` (in
//! `compile_zsh.rs`).
//!
//! **zshrs-original infrastructure with C-zsh-derived semantics.**
//! C zsh has no arithmetic compiler — `Src/math.c::matheval()`
//! tokenizes and evaluates in one pass via `getmathparam()` /
//! `mathevall()`. zshrs splits compilation from evaluation: the
//! tokenizer here matches `zzlex()` / `mathlex()` from
//! Src/math.c, but instead of pushing onto the math eval stack
//! we emit fusevm Ops which the JIT can specialize.

use fusevm::{ChunkBuilder, Op, Value};
use std::collections::HashMap;

// ═══════════════════════════════════════════════════════════════════════════
// ArithCompiler — lowers arithmetic expressions → fusevm bytecodes
// ═══════════════════════════════════════════════════════════════════════════

/// Arithmetic expression compiler.
///
/// Takes a zsh arithmetic expression (the content inside
/// `$((...))`) and emits fusevm bytecodes that compute the result.
///
/// **Tokenizer port**: same lexer shape as `zzlex()` /
/// `mathlex()` from Src/math.c. **Emit step**: zshrs-original —
/// C zsh evaluates inline via `mathevall()` (Src/math.c) and has
/// no compile-then-run path.
pub struct ArithCompiler<'a> {
    /// `input` field.
    pub input: &'a str,
    /// `pos` field.
    pub pos: usize,
    /// `builder` field.
    pub builder: ChunkBuilder,
    /// Variable name → slot index
    pub slots: HashMap<String, u16>,
    /// `next_slot` field.
    pub next_slot: u16,
    /// Names this expression ASSIGNS to (`x = …`, `x += …`, `x++`, `--x`).
    ///
    /// c:Src/math.c:1364-1372 — only an `OP_E2`/`OP_E2IO` operator reaches
    /// `setmathvar`; a name that is merely READ never gets written back.
    /// `compile_arith_str` uses this to write back exactly the names C
    /// would, instead of every identifier the expression mentions.
    pub assigned: std::collections::HashSet<String>,
}

// Token types matching the `MTYPE_*` enum from Src/math.c.
// Each variant corresponds to one of the operators / operand
// kinds the C source's `zzlex()` produces.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tok {
    Num(i64),
    Float(f64),
    Ident,
    Plus,
    Minus,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,
    LogAnd,
    LogOr,
    LogNot,
    Eq,
    Neq,
    Lt,
    Gt,
    Leq,
    Geq,
    Assign,
    PlusAssign,
    MinusAssign,
    MulAssign,
    DivAssign,
    ModAssign,
    // c:Src/math.c:135-142 ANDEQ/XOREQ/OREQ/SHLEFTEQ/SHRIGHTEQ and
    // c:Src/math.c:51/40-42 POWEREQ/DANDEQ/DOREQ/DXOREQ — the compound
    // assignments the old tokenizer had no variants for, so
    // `compile_arith` had to route every expression containing them
    // to the runtime evaluator.
    AndAssign,
    XorAssign,
    OrAssign,
    ShlAssign,
    ShrAssign,
    PowAssign,
    LogAndAssign,
    LogOrAssign,
    LogXorAssign,
    /// c:Src/math.c:126 `DXOR` — `^^`, logical xor. Not a C operator.
    LogXor,
    PreInc,
    PreDec,
    PostInc,
    PostDec,
    LParen,
    RParen,
    Comma,
    Quest,
    Colon,
    Eoi,
}

impl<'a> ArithCompiler<'a> {
    /// `new` — see implementation.
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            builder: ChunkBuilder::new(),
            slots: HashMap::new(),
            next_slot: 0,
            assigned: std::collections::HashSet::new(),
        }
    }

    /// Compile the arithmetic expression to fusevm bytecodes.
    /// Returns the compiled chunk.
    pub fn compile(mut self) -> fusevm::Chunk {
        self.builder.set_source("$((...))");
        self.builder.emit(Op::PushFrame, 0);
        self.expr();
        self.builder.emit(Op::ReturnValue, 0);
        self.builder.build()
    }

    /// Get or allocate a slot for a variable name.
    pub fn slot_for(&mut self, name: &str) -> u16 {
        if let Some(&slot) = self.slots.get(name) {
            return slot;
        }
        let slot = self.next_slot;
        self.next_slot += 1;
        self.slots.insert(name.to_string(), slot);
        slot
    }

    /// Walk the input string and collect all identifier names that appear.
    /// Used by `compile_arith_inline` to pre-load values from
    /// `executor.variables` and to know which slots to write back after.
    /// Excludes language keywords and numeric literals.
    pub fn collect_identifiers(&self, expr: &str) -> Vec<String> {
        let bytes = expr.as_bytes();
        let mut names: Vec<String> = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            // Strip an optional leading `$` (and `${...}` braces) — `(( $1 ))`,
            // `(( $x ))`, `(( ${count} ))` should all pre-load just like
            // `(( x ))`.
            let with_dollar = b == b'$';
            if with_dollar {
                if i + 1 >= bytes.len() {
                    i += 1;
                    continue;
                }
                if bytes[i + 1] == b'{' {
                    i += 2;
                    let start = i;
                    while i < bytes.len() && bytes[i] != b'}' {
                        i += 1;
                    }
                    let name = expr[start..i].to_string();
                    if !name.is_empty() && !names.contains(&name) {
                        names.push(name);
                    }
                    if i < bytes.len() {
                        i += 1; // skip `}`
                    }
                    continue;
                }
                i += 1;
                let start = i;
                if bytes
                    .get(i)
                    .copied()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
                {
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                    let name = expr[start..i].to_string();
                    if !names.contains(&name) {
                        names.push(name);
                    }
                    continue;
                }
            }
            if b.is_ascii_alphabetic() || b == b'_' || (with_dollar && i < bytes.len()) {
                let start = i;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                let name = expr[start..i].to_string();
                if !name.is_empty() && !names.contains(&name) {
                    names.push(name);
                }
            } else {
                i += 1;
            }
        }
        names
    }

    // ── Tokenizer ──

    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            let b = self.input.as_bytes()[self.pos];
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn peek_char(&self) -> Option<u8> {
        self.input.as_bytes().get(self.pos).copied()
    }

    fn next_char(&mut self) -> Option<u8> {
        let c = self.input.as_bytes().get(self.pos).copied();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    fn read_ident(&mut self) -> String {
        let start = self.pos;
        while self.pos < self.input.len() {
            let b = self.input.as_bytes()[self.pos];
            if b.is_ascii_alphanumeric() || b == b'_' {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.input[start..self.pos].to_string()
    }

    fn read_number(&mut self) -> Tok {
        let start = self.pos;

        // Handle hex: 0x... (c:Src/math.c lexconstant — 0x/0X prefix
        // always parses as base 16 regardless of OCTALZEROES.)
        if self.pos + 1 < self.input.len()
            && self.input.as_bytes()[self.pos] == b'0'
            && (self.input.as_bytes()[self.pos + 1] == b'x'
                || self.input.as_bytes()[self.pos + 1] == b'X')
        {
            self.pos += 2;
            while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_hexdigit()
            {
                self.pos += 1;
            }
            let val = i64::from_str_radix(&self.input[start + 2..self.pos], 16).unwrap_or(0);
            // c:Src/math.c lexconstant — set `lastbase = 16` for the
            // PM_INTEGER pm.base inheritance path in assignsparam.
            // Without this `(( X = 0xff )); echo \$X` printed 255
            // instead of zsh's `16#FF`.
            crate::ported::math::set_lastbase(16);
            return Tok::Num(val);
        }

        // Decimal integer (and possibly base-N or octal). Greedy: walk
        // all digits, then check for `#` (base-N) or `.` (float).
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        // c:Src/math.c — `N#digits` base-N literal. The leading
        // number is the base (2..=36); the digits after `#` use that
        // base. zsh accepts `#` (signed result) and `##` (unsigned),
        // but for the arith_compiler scope just `#` is sufficient.
        //
        // !!! DASH-STRICT GATE (no C counterpart) !!! real dash/ash have POSIX
        // arithmetic ONLY (decimal, `0` octal, `0x` hex) — `base#num` is a
        // zsh/bash/ksh extension they REJECT ("expecting EOF: 16#ff"). Under
        // `zshrs --dash`/`--ash`, do NOT consume the `#` as a base separator;
        // leave the decimal parsed and let the stray `#` surface as an
        // unexpected token, matching the real shell's error. bash-family --sh
        // and --ksh keep accepting base#num (their real shells do too).
        if self.pos < self.input.len()
            && self.input.as_bytes()[self.pos] == b'#'
            && !crate::dash_mode::dash_strict()
        {
            let base_str = &self.input[start..self.pos];
            if let Ok(base) = base_str.parse::<u32>() {
                if (2..=36).contains(&base) {
                    self.pos += 1; // skip `#`
                    let digit_start = self.pos;
                    while self.pos < self.input.len() {
                        let b = self.input.as_bytes()[self.pos];
                        let in_base = if base <= 10 {
                            b.is_ascii_digit() && (b - b'0') < base as u8
                        } else {
                            b.is_ascii_digit()
                                || (b.is_ascii_alphabetic() && {
                                    let v = if b.is_ascii_lowercase() {
                                        b - b'a' + 10
                                    } else {
                                        b - b'A' + 10
                                    };
                                    (v as u32) < base
                                })
                        };
                        if in_base {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    let digits = &self.input[digit_start..self.pos];
                    let val = i64::from_str_radix(digits, base).unwrap_or(0);
                    // c:Src/math.c lexconstant — record the source
                    // base so PM_INTEGER assignment can inherit it
                    // for display formatting (`(( X = 2#1010 ));
                    // echo \$X` → `2#1010`).
                    crate::ported::math::set_lastbase(base as i32);
                    return Tok::Num(val);
                }
            }
        }

        // c:Src/math.c — `010` (leading zero) is octal ONLY when
        // OCTALZEROES is set; default off, so a leading-zero literal
        // is decimal. Honour the option here.
        let lex_octal = self.pos > start + 1
            && self.input.as_bytes()[start] == b'0'
            && self.input.as_bytes()[start + 1].is_ascii_digit()
            && crate::ported::zsh_h::isset(crate::ported::zsh_h::OCTALZEROES);
        if lex_octal {
            let val = i64::from_str_radix(&self.input[start + 1..self.pos], 8).unwrap_or(0);
            return Tok::Num(val);
        }

        // Float (after decimal-integer scan).
        if self.pos < self.input.len() && self.input.as_bytes()[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let val: f64 = self.input[start..self.pos].parse().unwrap_or(0.0);
            return Tok::Float(val);
        }

        let val: i64 = self.input[start..self.pos].parse().unwrap_or(0);
        Tok::Num(val)
    }

    fn next_tok(&mut self) -> (Tok, String) {
        self.skip_whitespace();

        let Some(c) = self.peek_char() else {
            return (Tok::Eoi, String::new());
        };

        match c {
            b'0'..=b'9' => {
                let tok = self.read_number();
                (tok, String::new())
            }
            b'a'..=b'z' | b'A'..=b'Z' | b'_' => {
                let name = self.read_ident();
                (Tok::Ident, name)
            }
            b'$' => {
                // `$NAME` / `${NAME}` / `$N` in arithmetic: consume the `$`
                // and read the var name as a normal identifier. zsh
                // accepts both `$x` and `x` in `(( ))`; the value is loaded
                // from the variable table either way (positional `$1`,
                // `$2` are stored under those names too).
                self.pos += 1;
                if self.peek_char() == Some(b'{') {
                    self.pos += 1;
                    let mut name = String::new();
                    while let Some(b) = self.peek_char() {
                        if b == b'}' {
                            self.pos += 1;
                            break;
                        }
                        name.push(b as char);
                        self.pos += 1;
                    }
                    (Tok::Ident, name)
                } else if let Some(b) = self.peek_char() {
                    if b.is_ascii_digit() {
                        // Positional `$N` — read all digits as the name.
                        let mut name = String::new();
                        while let Some(d) = self.peek_char() {
                            if !d.is_ascii_digit() {
                                break;
                            }
                            name.push(d as char);
                            self.pos += 1;
                        }
                        (Tok::Ident, name)
                    } else if b.is_ascii_alphabetic() || b == b'_' {
                        let name = self.read_ident();
                        (Tok::Ident, name)
                    } else {
                        // `$` followed by special char — not a meaningful
                        // arith form; emit zero.
                        (Tok::Num(0), String::new())
                    }
                } else {
                    (Tok::Num(0), String::new())
                }
            }
            b'+' => {
                self.pos += 1;
                match self.peek_char() {
                    // !!! DASH-STRICT GATE !!! `++`/`--` are C inc/dec operators
                    // that POSIX arithmetic (dash/ash) lacks — they reject
                    // `$((a++))`. Don't consume the second `+`: it becomes a
                    // separate unary `+` with no operand → parse error, like
                    // dash. `+=` is POSIX-valid and stays.
                    Some(b'+') if !crate::dash_mode::dash_strict() => {
                        self.pos += 1;
                        (Tok::PreInc, String::new())
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::PlusAssign, String::new())
                    }
                    _ => (Tok::Plus, String::new()),
                }
            }
            b'-' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'-') if !crate::dash_mode::dash_strict() => {
                        self.pos += 1;
                        (Tok::PreDec, String::new())
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::MinusAssign, String::new())
                    }
                    _ => (Tok::Minus, String::new()),
                }
            }
            b'*' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'*') => {
                        self.pos += 1;
                        if self.peek_char() == Some(b'=') {
                            self.pos += 1;
                            // c:Src/math.c:51 POWEREQ — `a **= b` is
                            // `a = a ** b`. Lexing it as MulAssign made
                            // `a=2; (( a **= 3 ))` yield 6 instead of 8.
                            (Tok::PowAssign, String::new())
                        } else {
                            (Tok::Pow, String::new())
                        }
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::MulAssign, String::new())
                    }
                    _ => (Tok::Mul, String::new()),
                }
            }
            b'/' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::DivAssign, String::new())
                } else {
                    (Tok::Div, String::new())
                }
            }
            b'%' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::ModAssign, String::new())
                } else {
                    (Tok::Mod, String::new())
                }
            }
            b'&' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'&') => {
                        self.pos += 1;
                        // c:Src/math.c:40 DANDEQ — `&&=`.
                        if self.peek_char() == Some(b'=') {
                            self.pos += 1;
                            (Tok::LogAndAssign, String::new())
                        } else {
                            (Tok::LogAnd, String::new())
                        }
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::AndAssign, String::new()) // c:35 ANDEQ
                    }
                    _ => (Tok::BitAnd, String::new()),
                }
            }
            b'|' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'|') => {
                        self.pos += 1;
                        // c:Src/math.c:41 DOREQ — `||=`.
                        if self.peek_char() == Some(b'=') {
                            self.pos += 1;
                            (Tok::LogOrAssign, String::new())
                        } else {
                            (Tok::LogOr, String::new())
                        }
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::OrAssign, String::new()) // c:37 OREQ
                    }
                    _ => (Tok::BitOr, String::new()),
                }
            }
            b'^' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'^') => {
                        self.pos += 1;
                        // c:Src/math.c:42 DXOREQ — `^^=`.
                        if self.peek_char() == Some(b'=') {
                            self.pos += 1;
                            (Tok::LogXorAssign, String::new())
                        } else {
                            (Tok::LogXor, String::new()) // c:26 DXOR
                        }
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::XorAssign, String::new()) // c:36 XOREQ
                    }
                    _ => (Tok::BitXor, String::new()),
                }
            }
            b'~' => {
                self.pos += 1;
                (Tok::BitNot, String::new())
            }
            b'!' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::Neq, String::new())
                } else {
                    (Tok::LogNot, String::new())
                }
            }
            b'<' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'<') => {
                        self.pos += 1;
                        // c:Src/math.c:38 SHLEFTEQ — `<<=`.
                        if self.peek_char() == Some(b'=') {
                            self.pos += 1;
                            (Tok::ShlAssign, String::new())
                        } else {
                            (Tok::Shl, String::new())
                        }
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::Leq, String::new())
                    }
                    _ => (Tok::Lt, String::new()),
                }
            }
            b'>' => {
                self.pos += 1;
                match self.peek_char() {
                    Some(b'>') => {
                        self.pos += 1;
                        // c:Src/math.c:39 SHRIGHTEQ — `>>=`.
                        if self.peek_char() == Some(b'=') {
                            self.pos += 1;
                            (Tok::ShrAssign, String::new())
                        } else {
                            (Tok::Shr, String::new())
                        }
                    }
                    Some(b'=') => {
                        self.pos += 1;
                        (Tok::Geq, String::new())
                    }
                    _ => (Tok::Gt, String::new()),
                }
            }
            b'=' => {
                self.pos += 1;
                if self.peek_char() == Some(b'=') {
                    self.pos += 1;
                    (Tok::Eq, String::new())
                } else {
                    (Tok::Assign, String::new())
                }
            }
            b'(' => {
                self.pos += 1;
                (Tok::LParen, String::new())
            }
            b')' => {
                self.pos += 1;
                (Tok::RParen, String::new())
            }
            b',' => {
                self.pos += 1;
                (Tok::Comma, String::new())
            }
            b'?' => {
                self.pos += 1;
                (Tok::Quest, String::new())
            }
            b':' => {
                self.pos += 1;
                (Tok::Colon, String::new())
            }
            _ => {
                self.pos += 1;
                (Tok::Eoi, String::new())
            }
        }
    }

    // ── Recursive descent → emit ops ──
    //
    // c:Src/math.c:220-273 `z_prec` — zsh's DEFAULT operator precedence,
    // which is NOT C's. Reading the table tightest-binding first:
    //
    //   2  unary  ++ -- ! ~ +x -x
    //   3  << >>          <- shifts bind TIGHTER than everything below
    //   4  &
    //   5  ^
    //   6  |
    //   7  **
    //   8  * / %
    //   9  + -
    //  10  < > <= >=
    //  11  == !=
    //  12  &&
    //  13  || ^^
    //  14  ?     15  :
    //  16  = += -= *= /= %= &= ^= |= <<= >>= &&= ||= ^^= **=
    //  17  ,
    //
    // The cascade below is one function per level in that order. It used
    // to be C's order (bitwise loosest, shifts between `+` and the
    // comparisons), which is why `compile_arith` had to route every
    // expression containing `& | ^ < > **` to the runtime evaluator: the
    // compiled answer was wrong. c:Src/math.c:190 `c_prec` is the table
    // zsh switches to under `setopt c_precedences`; `prec_tables_agree`
    // below is what keeps that option honest.
    /// `expr` — see implementation.
    pub fn expr(&mut self) {
        self.comma_expr();
    }

    /// c:Src/math.c z_prec[COMMA] = 17 — loosest binding. Each operand is
    /// evaluated in order and all but the last discarded (c:1373-1377
    /// `case COMMA: c = b`).
    fn comma_expr(&mut self) {
        self.assign_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::Comma {
                let _ = self.next_tok();
                self.builder.emit(Op::Pop, 0);
                self.assign_expr();
            } else {
                break;
            }
        }
    }

    /// c:Src/math.c z_prec[EQ..POWEREQ] = 16, `type[] = RL` — right-
    /// associative, so the RHS recurses back into this level.
    fn assign_expr(&mut self) {
        let save_pos = self.pos;

        self.skip_whitespace();
        if let Some(c) = self.peek_char() {
            if c.is_ascii_alphabetic() || c == b'_' {
                let name = self.read_ident();
                self.skip_whitespace();
                let (tok, _) = self.peek_tok();
                if tok == Tok::Assign {
                    let _ = self.next_tok(); // consume =
                    let slot = self.slot_for(&name);
                    self.assigned.insert(name.clone());
                    self.assign_expr();
                    self.builder.emit(Op::Dup, 0);
                    self.builder.emit(Op::SetSlot(slot), 0);
                    return;
                }
                if let Some(binop) = compound_assign_op(tok) {
                    let _ = self.next_tok(); // consume op=
                    let slot = self.slot_for(&name);
                    self.assigned.insert(name.clone());
                    self.builder.emit(Op::GetSlot(slot), 0);
                    self.assign_expr();
                    self.emit_binop(binop);
                    self.builder.emit(Op::Dup, 0);
                    self.builder.emit(Op::SetSlot(slot), 0);
                    return;
                }
                // Not an assignment — rewind and re-parse as a value.
                self.pos = save_pos;
            }
        }

        self.ternary_expr();
    }

    fn peek_tok(&mut self) -> (Tok, String) {
        let save = self.pos;
        let tok = self.next_tok();
        self.pos = save;
        tok
    }

    /// c:Src/math.c z_prec[QUEST] = 14, z_prec[COLON] = 15.
    fn ternary_expr(&mut self) {
        self.logor_expr();
        let (tok, _) = self.peek_tok();
        if tok == Tok::Quest {
            let _ = self.next_tok(); // consume ?
            let else_jump = self.builder.emit(Op::JumpIfFalse(0), 0);
            self.assign_expr(); // true branch
            let (colon, _) = self.peek_tok();
            let end_jump = self.builder.emit(Op::Jump(0), 0);
            let else_target = self.builder.current_pos();
            self.builder.patch_jump(else_jump, else_target);
            if colon == Tok::Colon {
                let _ = self.next_tok(); // consume :
            }
            self.assign_expr(); // false branch
            let end_target = self.builder.current_pos();
            self.builder.patch_jump(end_jump, end_target);
        }
    }

    /// c:Src/math.c z_prec[DOR] = z_prec[DXOR] = 13 — `||` and `^^` share
    /// a level. `||` short-circuits (c:1200 `BOOL`); `^^` cannot, since
    /// both sides decide the answer.
    fn logor_expr(&mut self) {
        self.logand_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::LogOr => {
                    let _ = self.next_tok();
                    let skip = self.builder.emit(Op::JumpIfTrueKeep(0), 0);
                    self.builder.emit(Op::Pop, 0);
                    self.logand_expr();
                    self.builder.patch_jump(skip, self.builder.current_pos());
                }
                Tok::LogXor => {
                    let _ = self.next_tok();
                    self.logand_expr();
                    self.builder.emit(
                        Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_LOGXOR, 2),
                        0,
                    );
                }
                _ => break,
            }
        }
    }

    /// c:Src/math.c z_prec[DAND] = 12.
    fn logand_expr(&mut self) {
        self.equality_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::LogAnd {
                let _ = self.next_tok();
                let skip = self.builder.emit(Op::JumpIfFalseKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.equality_expr();
                self.builder.patch_jump(skip, self.builder.current_pos());
            } else {
                break;
            }
        }
    }

    /// c:Src/math.c z_prec[DEQ] = z_prec[NEQ] = 11.
    fn equality_expr(&mut self) {
        self.comparison_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Eq => {
                    let _ = self.next_tok();
                    self.comparison_expr();
                    self.builder.emit(Op::NumEq, 0);
                }
                Tok::Neq => {
                    let _ = self.next_tok();
                    self.comparison_expr();
                    self.builder.emit(Op::NumNe, 0);
                }
                _ => break,
            }
        }
    }

    /// c:Src/math.c z_prec[LES..GEQ] = 10.
    fn comparison_expr(&mut self) {
        self.additive_expr();
        loop {
            let (tok, _) = self.peek_tok();
            let op = match tok {
                Tok::Lt => Op::NumLt,
                Tok::Gt => Op::NumGt,
                Tok::Leq => Op::NumLe,
                Tok::Geq => Op::NumGe,
                _ => break,
            };
            let _ = self.next_tok();
            self.additive_expr();
            self.builder.emit(op, 0);
        }
    }

    /// c:Src/math.c z_prec[PLUS] = z_prec[MINUS] = 9.
    fn additive_expr(&mut self) {
        self.multiplicative_expr();
        loop {
            let (tok, _) = self.peek_tok();
            let op = match tok {
                Tok::Plus => Op::Add,
                Tok::Minus => Op::Sub,
                _ => break,
            };
            let _ = self.next_tok();
            self.multiplicative_expr();
            self.builder.emit(op, 0);
        }
    }

    /// c:Src/math.c z_prec[MUL] = z_prec[DIV] = z_prec[MOD] = 8.
    fn multiplicative_expr(&mut self) {
        self.pow_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Mul => {
                    let _ = self.next_tok();
                    self.pow_expr();
                    self.builder.emit(Op::Mul, 0);
                }
                Tok::Div => {
                    let _ = self.next_tok();
                    self.pow_expr();
                    self.emit_binop(BinOp::Div);
                }
                Tok::Mod => {
                    let _ = self.next_tok();
                    self.pow_expr();
                    self.emit_binop(BinOp::Mod);
                }
                _ => break,
            }
        }
    }

    /// c:Src/math.c z_prec[POWER] = 7, `type[POWER] = RL|OP_A2` — right-
    /// associative, and it binds LOOSER than `| ^ &` in zsh (tighter in C).
    fn pow_expr(&mut self) {
        self.bitor_expr();
        let (tok, _) = self.peek_tok();
        if tok == Tok::Pow {
            let _ = self.next_tok();
            self.pow_expr(); // right-associative
            self.emit_binop(BinOp::Pow);
        }
    }

    /// c:Src/math.c z_prec[OR] = 6.
    fn bitor_expr(&mut self) {
        self.bitxor_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::BitOr {
                let _ = self.next_tok();
                self.bitxor_expr();
                self.builder.emit(Op::BitOr, 0);
            } else {
                break;
            }
        }
    }

    /// c:Src/math.c z_prec[XOR] = 5.
    fn bitxor_expr(&mut self) {
        self.bitand_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::BitXor {
                let _ = self.next_tok();
                self.bitand_expr();
                self.builder.emit(Op::BitXor, 0);
            } else {
                break;
            }
        }
    }

    /// c:Src/math.c z_prec[AND] = 4.
    fn bitand_expr(&mut self) {
        self.shift_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::BitAnd {
                let _ = self.next_tok();
                self.shift_expr();
                self.builder.emit(Op::BitAnd, 0);
            } else {
                break;
            }
        }
    }

    /// c:Src/math.c z_prec[SHLEFT] = z_prec[SHRIGHT] = 3 — in zsh the
    /// shifts bind tighter than every other binary operator.
    fn shift_expr(&mut self) {
        self.unary_expr();
        loop {
            let (tok, _) = self.peek_tok();
            let op = match tok {
                Tok::Shl => Op::Shl,
                Tok::Shr => Op::Shr,
                _ => break,
            };
            let _ = self.next_tok();
            self.unary_expr();
            self.builder.emit(op, 0);
        }
    }

    /// c:Src/math.c z_prec[NOT..UMINUS] = 2, `type[] = RL`.
    fn unary_expr(&mut self) {
        let (tok, _) = self.peek_tok();
        match tok {
            Tok::Minus => {
                let _ = self.next_tok();
                self.unary_expr();
                self.builder.emit(Op::Negate, 0);
            }
            Tok::Plus => {
                let _ = self.next_tok();
                self.unary_expr();
                // c:Src/math.c:1189 UPLUS — `c = b`, a no-op on numbers.
            }
            Tok::LogNot => {
                let _ = self.next_tok();
                self.unary_expr();
                self.builder.emit(Op::LogNot, 0);
            }
            Tok::BitNot => {
                let _ = self.next_tok();
                self.unary_expr();
                self.builder.emit(Op::BitNot, 0);
            }
            Tok::PreInc => {
                let _ = self.next_tok();
                let (_, var_name) = self.next_tok();
                let slot = self.slot_for(&var_name);
                self.assigned.insert(var_name);
                self.builder.emit(Op::PreIncSlot(slot), 0);
            }
            Tok::PreDec => {
                let _ = self.next_tok();
                let (_, var_name) = self.next_tok();
                let slot = self.slot_for(&var_name);
                self.assigned.insert(var_name);
                self.builder.emit(Op::GetSlot(slot), 0);
                self.builder.emit(Op::Dec, 0);
                self.builder.emit(Op::Dup, 0);
                self.builder.emit(Op::SetSlot(slot), 0);
            }
            _ => self.primary_expr(),
        }
    }

    fn primary_expr(&mut self) {
        let (tok, name) = self.next_tok();
        match tok {
            Tok::Num(n) => {
                self.builder.emit(Op::LoadInt(n), 0);
            }
            Tok::Float(f) => {
                self.builder.emit(Op::LoadFloat(f), 0);
            }
            Tok::Ident => {
                let slot = self.slot_for(&name);
                self.builder.emit(Op::GetSlot(slot), 0);

                // c:Src/math.c:112-113 POSTPLUS / POSTMINUS — the value of
                // the expression is the OLD one; the variable keeps the new.
                let (post_tok, _) = self.peek_tok();
                match post_tok {
                    Tok::PreInc => {
                        let _ = self.next_tok();
                        self.assigned.insert(name.clone());
                        self.builder.emit(Op::Dup, 0); // keep old value
                        self.builder.emit(Op::Inc, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                    }
                    Tok::PreDec => {
                        let _ = self.next_tok();
                        self.assigned.insert(name.clone());
                        self.builder.emit(Op::Dup, 0);
                        self.builder.emit(Op::Dec, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                    }
                    _ => {}
                }
            }
            Tok::LParen => {
                self.expr();
                let _ = self.next_tok(); // consume RParen
            }
            _ => {
                // Unexpected token — push 0
                self.builder.emit(Op::LoadInt(0), 0);
            }
        }
    }

    /// Emit one binary operator.
    ///
    /// `+ - * & | ^ << >>` are single fusevm ops whose int/float behaviour
    /// already matches `Src/math.c` (int result when both operands are
    /// integers, float otherwise; the bitwise/shift set coerces to int,
    /// which is C's `OP_A2IO`).
    ///
    /// `/ % **` are NOT: `Op::Div` always produces a float (zsh does
    /// integer division when both operands are integers, c:1243-1256),
    /// `Op::Mod` returns 0 for a zero divisor instead of raising
    /// "division by zero" (c:1260), and `Op::Pow` is `powf` rather than
    /// c:1343-1344's integer power loop. Those three route to a builtin
    /// that ports the C arm exactly — one indirect call per OCCURRENCE of
    /// the operator, versus the whole-expression re-lex the runtime
    /// evaluator costs on every evaluation.
    fn emit_binop(&mut self, op: BinOp) {
        match op {
            BinOp::Add => self.builder.emit(Op::Add, 0),
            BinOp::Sub => self.builder.emit(Op::Sub, 0),
            BinOp::Mul => self.builder.emit(Op::Mul, 0),
            BinOp::BitAnd => self.builder.emit(Op::BitAnd, 0),
            BinOp::BitOr => self.builder.emit(Op::BitOr, 0),
            BinOp::BitXor => self.builder.emit(Op::BitXor, 0),
            BinOp::Shl => self.builder.emit(Op::Shl, 0),
            BinOp::Shr => self.builder.emit(Op::Shr, 0),
            BinOp::Div => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_DIV, 2), 0),
            BinOp::Mod => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_MOD, 2), 0),
            BinOp::Pow => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_POW, 2), 0),
            BinOp::LogAnd => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_LOGAND, 2), 0),
            BinOp::LogOr => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_LOGOR, 2), 0),
            BinOp::LogXor => self
                .builder
                .emit(Op::CallBuiltin(crate::vm_helper::BUILTIN_ARITH_LOGXOR, 2), 0),
        };
    }
}

/// c:Src/math.c:190 `c_prec` — the table `setopt c_precedences` selects.
/// Indexed by the same token numbering as `Z_PREC`.
///
/// The compiled path always parses with zsh's DEFAULT precedence. That is
/// only safe for an expression whose operators keep their relative order
/// in both tables, which `prec_tables_agree` decides; anything else goes
/// to the runtime evaluator, which reads `prec` live.
const C_PREC: &[(Tok, i32)] = &[
    (Tok::LogNot, 2),
    (Tok::BitNot, 2),
    (Tok::PreInc, 2),
    (Tok::PreDec, 2),
    (Tok::BitAnd, 9),
    (Tok::BitXor, 10),
    (Tok::BitOr, 11),
    (Tok::Mul, 4),
    (Tok::Div, 4),
    (Tok::Mod, 4),
    (Tok::Plus, 5),
    (Tok::Minus, 5),
    (Tok::Shl, 6),
    (Tok::Shr, 6),
    (Tok::Lt, 7),
    (Tok::Leq, 7),
    (Tok::Gt, 7),
    (Tok::Geq, 7),
    (Tok::Eq, 8),
    (Tok::Neq, 8),
    (Tok::LogAnd, 12),
    (Tok::LogOr, 14),
    (Tok::LogXor, 13),
    (Tok::Quest, 15),
    (Tok::Colon, 16),
    (Tok::Pow, 3),
    (Tok::Comma, 18),
];

/// c:Src/math.c:250 `z_prec` — zsh's default table, the one the cascade
/// in this file implements.
const Z_PREC: &[(Tok, i32)] = &[
    (Tok::LogNot, 2),
    (Tok::BitNot, 2),
    (Tok::PreInc, 2),
    (Tok::PreDec, 2),
    (Tok::Shl, 3),
    (Tok::Shr, 3),
    (Tok::BitAnd, 4),
    (Tok::BitXor, 5),
    (Tok::BitOr, 6),
    (Tok::Pow, 7),
    (Tok::Mul, 8),
    (Tok::Div, 8),
    (Tok::Mod, 8),
    (Tok::Plus, 9),
    (Tok::Minus, 9),
    (Tok::Lt, 10),
    (Tok::Leq, 10),
    (Tok::Gt, 10),
    (Tok::Geq, 10),
    (Tok::Eq, 11),
    (Tok::Neq, 11),
    (Tok::LogAnd, 12),
    (Tok::LogOr, 13),
    (Tok::LogXor, 13),
    (Tok::Quest, 14),
    (Tok::Colon, 15),
    (Tok::Comma, 17),
];

fn prec_of(table: &[(Tok, i32)], tok: Tok) -> Option<i32> {
    table.iter().find(|(t, _)| *t == tok).map(|(_, p)| *p)
}

/// Whether `Z_PREC` and `C_PREC` bracket this expression identically.
///
/// Only the RELATIVE order of the operators actually present matters to a
/// precedence-climbing parse, and associativity is the same in both
/// tables, so it is enough to check every pair of operators in the
/// expression for an order inversion between the two tables. `(( a << b
/// ))` is safe (one operator); `(( a << b & c ))` is not (`<<` is tighter
/// than `&` in zsh, looser in C).
fn prec_tables_agree(ops: &[Tok]) -> bool {
    for (i, x) in ops.iter().enumerate() {
        for y in &ops[i + 1..] {
            let (Some(zx), Some(zy)) = (prec_of(Z_PREC, *x), prec_of(Z_PREC, *y)) else {
                continue;
            };
            let (Some(cx), Some(cy)) = (prec_of(C_PREC, *x), prec_of(C_PREC, *y)) else {
                continue;
            };
            if (zx - zy).signum() != (cx - cy).signum() {
                return false;
            }
        }
    }
    true
}

/// Why an expression cannot be compiled, or `None` when it can.
///
/// Everything listed here needs state the compiled path does not carry:
/// a subscript needs `getvalue`'s full parser, a base-tagged literal sets
/// `lastbase` for the ASSIGNMENT that follows it (c:Src/math.c
/// `lexconstant`), a math function needs `callmathfunc`, and a
/// `${…}` form other than a bare name needs word expansion first.
/// Each falls back to `BUILTIN_ARITH_EVAL`, which is what every
/// expression used to do.
pub fn arith_uncompilable_reason(expr: &str) -> Option<&'static str> {
    let b = expr.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        match c {
            // Array subscript — `getmathparam` parses the whole
            // `name[...]` form including subscript flags.
            b'[' => return Some("array subscript"),
            // Quoted identifier — c:Src/math.c treats `"abc"` as a name.
            b'"' | 0x9e => return Some("quoted identifier"),
            // `n#ddd` base literal / `#\c` char constant: both set
            // `lastbase`, which the following assignment inherits.
            b'#' => return Some("base literal or char constant"),
            b'$' => {
                // `$name` and `${name}` name a parameter, which is what a
                // bare `name` means in arithmetic anyway, so both lex as
                // plain identifiers and read through `getmathparam`.
                //
                // A POSITIONAL (`$1`, `${2}`) does not: c:Src/params.c
                // keeps the positionals in `pparams`, not in `paramtab`,
                // so `getmathparam("1")` finds nothing and reads 0 —
                // `(( $1 % 15 == 0 ))` was true for every argument. Those,
                // and every other `${…}` form, need word expansion first
                // and go to the runtime evaluator.
                let rest = &b[i + 1..];
                let mut name = rest.iter().copied();
                let first = match name.next() {
                    Some(b'{') => {
                        let body: Vec<u8> =
                            rest.iter().skip(1).take_while(|&&x| x != b'}').copied().collect();
                        if body.is_empty()
                            || !body[1..].iter().all(|&x| x.is_ascii_alphanumeric() || x == b'_')
                        {
                            return Some("parameter expansion");
                        }
                        body[0]
                    }
                    Some(x) => x,
                    None => return Some("parameter expansion"),
                };
                if !(first.is_ascii_alphabetic() || first == b'_') {
                    return Some("parameter expansion");
                }
            }
            _ => {}
        }
    }
    // Base-tagged literals — see `#` above; these are the 0x/0b spellings.
    let lower = expr.to_ascii_lowercase();
    if lower.contains("0x") || lower.contains("0b") {
        return Some("base-tagged literal");
    }
    // Exponent form (`1e3`) — `read_number` stops at the `e`, so the
    // exponent would be parsed as a separate identifier.
    if b.windows(2)
        .any(|w| w[0].is_ascii_digit() && (w[1] | 0x20) == b'e')
    {
        return Some("float exponent");
    }
    // A math function call — `name(` with a name in front of the paren.
    let mut prev_ident = false;
    for &c in b {
        if c == b'(' && prev_ident {
            return Some("math function call");
        }
        prev_ident = c.is_ascii_alphanumeric() || c == b'_';
    }
    // Precedence-table divergence under `setopt c_precedences`.
    let mut ac = ArithCompiler::new(expr);
    let mut ops: Vec<Tok> = Vec::new();
    loop {
        let (tok, _) = ac.next_tok();
        if tok == Tok::Eoi {
            break;
        }
        if prec_of(Z_PREC, tok).is_some() {
            ops.push(tok);
        }
    }
    if !prec_tables_agree(&ops) {
        return Some("c_precedences-sensitive operator mix");
    }
    None
}

/// The binary operators `emit_binop` can lower. Separate from `Tok` because
/// `x op= y` and `x op y` share the operator but not the token.
#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    LogAnd,
    LogOr,
    LogXor,
}

/// The operator a compound-assignment token applies —
/// c:Src/math.c:1364-1372, where `PLUSEQ` runs the `PLUS` arm and then
/// assigns. `None` for a token that is not a compound assignment.
fn compound_assign_op(tok: Tok) -> Option<BinOp> {
    Some(match tok {
        Tok::PlusAssign => BinOp::Add,
        Tok::MinusAssign => BinOp::Sub,
        Tok::MulAssign => BinOp::Mul,
        Tok::DivAssign => BinOp::Div,
        Tok::ModAssign => BinOp::Mod,
        Tok::AndAssign => BinOp::BitAnd,
        Tok::OrAssign => BinOp::BitOr,
        Tok::XorAssign => BinOp::BitXor,
        Tok::ShlAssign => BinOp::Shl,
        Tok::ShrAssign => BinOp::Shr,
        Tok::PowAssign => BinOp::Pow,
        Tok::LogAndAssign => BinOp::LogAnd,
        Tok::LogOrAssign => BinOp::LogOr,
        Tok::LogXorAssign => BinOp::LogXor,
        _ => return None,
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests — pure-expression evaluation via ArithCompiler → fusevm::VM.
//
// These tests pin the compiler's emitted bytecode by *running* the result and
// asserting the numeric output. Variable / identifier paths need executor
// pre-loading and are exercised by integration tests in tests/zshrs_shell.rs
// — the unit tests below stay literal-only so they need no shell context.
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use fusevm::{VMResult, Value};

    /// Compile + run an arithmetic expression. Panics on VM error so a
    /// regression surfaces with the expression text.
    fn eval(expr: &str) -> Value {
        let chunk = ArithCompiler::new(expr).compile();
        let mut vm = fusevm::VM::new(chunk);
        // `/ % **` lower to builtin calls because no fusevm op has
        // `Src/math.c`'s int/float semantics for them (see `emit_binop`).
        // Register the same three arms the shell registers so these
        // literal-only tests exercise the real operator bodies.
        vm.register_builtin(crate::vm_helper::BUILTIN_ARITH_DIV, |vm, _argc| {
            let b = crate::fusevm_bridge::arith_pop_mnumber(vm);
            let a = crate::fusevm_bridge::arith_pop_mnumber(vm);
            crate::fusevm_bridge::arith_push(crate::fusevm_bridge::arith_div(a, b))
        });
        vm.register_builtin(crate::vm_helper::BUILTIN_ARITH_MOD, |vm, _argc| {
            let b = crate::fusevm_bridge::arith_pop_mnumber(vm);
            let a = crate::fusevm_bridge::arith_pop_mnumber(vm);
            crate::fusevm_bridge::arith_push(crate::fusevm_bridge::arith_mod(a, b))
        });
        vm.register_builtin(crate::vm_helper::BUILTIN_ARITH_POW, |vm, _argc| {
            let b = crate::fusevm_bridge::arith_pop_mnumber(vm);
            let a = crate::fusevm_bridge::arith_pop_mnumber(vm);
            crate::fusevm_bridge::arith_push(crate::fusevm_bridge::arith_pow(a, b))
        });
        match vm.run() {
            VMResult::Ok(v) => v,
            VMResult::Halted => Value::Undef,
            VMResult::Error(e) => panic!("VM error evaluating {expr:?}: {e}"),
        }
    }

    fn eval_int(expr: &str) -> i64 {
        eval(expr).to_int()
    }

    fn eval_float(expr: &str) -> f64 {
        eval(expr).to_float()
    }

    // ── Integer literals ─────────────────────────────────────────────────
    #[test]
    fn literal_zero() {
        assert_eq!(eval_int("0"), 0);
    }

    #[test]
    fn literal_small_positive() {
        assert_eq!(eval_int("42"), 42);
    }

    #[test]
    fn literal_large() {
        assert_eq!(eval_int("1000000000"), 1_000_000_000);
    }

    #[test]
    fn literal_hex_lowercase() {
        assert_eq!(eval_int("0xff"), 255);
    }

    #[test]
    fn literal_hex_uppercase() {
        assert_eq!(eval_int("0XDEAD"), 0xDEAD);
    }

    #[test]
    fn literal_hex_mixed() {
        assert_eq!(eval_int("0xCaFe"), 0xCAFE);
    }

    #[test]
    fn literal_octal() {
        // zsh DEFAULT: `017` is decimal 17, NOT octal — `setopt
        // OCTAL_ZEROES` is required to enable C-style 0NNN octals
        // (per Src/options.c default_opts[]). Verified against
        // `zsh -fc 'let x=017; print $x'` → "17".
        assert_eq!(eval_int("017"), 17);
    }

    #[test]
    fn literal_octal_zero_prefix_only() {
        // `0` alone is decimal zero; the octal path requires a digit AFTER `0`.
        assert_eq!(eval_int("0"), 0);
    }

    // ── Float literals ───────────────────────────────────────────────────
    #[test]
    fn literal_float_simple() {
        assert!((eval_float("3.14") - 3.14).abs() < 1e-9);
    }

    #[test]
    fn literal_float_no_fractional_digits() {
        // `5.` parses as 5.0 per read_number()'s `.` handling.
        assert!((eval_float("5.") - 5.0).abs() < 1e-9);
    }

    // ── Addition / subtraction ───────────────────────────────────────────
    #[test]
    fn add_two_ints() {
        assert_eq!(eval_int("40 + 2"), 42);
    }

    #[test]
    fn sub_two_ints() {
        assert_eq!(eval_int("50 - 8"), 42);
    }

    #[test]
    fn sub_into_negative() {
        assert_eq!(eval_int("5 - 10"), -5);
    }

    #[test]
    fn add_chain_left_associative() {
        assert_eq!(eval_int("1 + 2 + 3 + 4"), 10);
    }

    #[test]
    fn sub_chain_left_associative() {
        // (((100 - 1) - 2) - 3) = 94, NOT 100 - (1 - 2 - 3) = 104
        assert_eq!(eval_int("100 - 1 - 2 - 3"), 94);
    }

    // ── Multiplication / division / modulo ──────────────────────────────
    #[test]
    fn mul_two_ints() {
        assert_eq!(eval_int("6 * 7"), 42);
    }

    #[test]
    fn div_two_ints() {
        assert_eq!(eval_int("84 / 2"), 42);
    }

    #[test]
    fn mod_two_ints() {
        assert_eq!(eval_int("17 % 5"), 2);
    }

    #[test]
    fn mod_evenly_divides() {
        assert_eq!(eval_int("10 % 5"), 0);
    }

    // ── Precedence ───────────────────────────────────────────────────────
    #[test]
    fn precedence_mul_over_add() {
        assert_eq!(eval_int("2 + 3 * 4"), 14);
    }

    #[test]
    fn precedence_div_over_sub() {
        assert_eq!(eval_int("20 - 10 / 2"), 15);
    }

    #[test]
    fn precedence_parens_override() {
        assert_eq!(eval_int("(2 + 3) * 4"), 20);
    }

    #[test]
    fn precedence_nested_parens() {
        assert_eq!(eval_int("((1 + 2) * (3 + 4))"), 21);
    }

    #[test]
    fn precedence_pow_over_mul() {
        // 2 * 3 ** 2 = 2 * 9 = 18, not (2*3)**2 = 36
        assert_eq!(eval_int("2 * 3 ** 2"), 18);
    }

    #[test]
    fn pow_right_associative() {
        // 2 ** 3 ** 2 = 2 ** 9 = 512, not (2**3)**2 = 64
        assert_eq!(eval_int("2 ** 3 ** 2"), 512);
    }

    // ── Unary operators ──────────────────────────────────────────────────
    #[test]
    fn unary_minus_literal() {
        assert_eq!(eval_int("-5"), -5);
    }

    #[test]
    fn unary_minus_expr() {
        assert_eq!(eval_int("-(3 + 4)"), -7);
    }

    #[test]
    fn unary_plus_is_noop() {
        assert_eq!(eval_int("+42"), 42);
    }

    #[test]
    fn double_negation_requires_separator() {
        // `--5` parses as pre-decrement (the `--` token), NOT two unary minuses.
        // Real double negation needs a space or parens between the two `-`s.
        assert_eq!(eval_int("- -5"), 5);
        assert_eq!(eval_int("-(-5)"), 5);
    }

    #[test]
    fn unary_minus_binds_tighter_than_pow() {
        // Our grammar: unary_expr → unary_expr (right-recursive), then primary.
        // `-2 ** 2` parses as `(-2) ** 2` = 4 with this layout (pow is below
        // unary). Pinning current behavior — if it changes, this test surfaces
        // the change explicitly.
        assert_eq!(eval_int("-2 ** 2"), 4);
    }

    // ── Bitwise ──────────────────────────────────────────────────────────
    #[test]
    fn bitand_basic() {
        assert_eq!(eval_int("0xFF & 0x0F"), 0x0F);
    }

    #[test]
    fn bitor_basic() {
        assert_eq!(eval_int("0x10 | 0x01"), 0x11);
    }

    #[test]
    fn bitxor_basic() {
        assert_eq!(eval_int("0xFF ^ 0x0F"), 0xF0);
    }

    #[test]
    fn bitnot_zero_is_minus_one() {
        assert_eq!(eval_int("~0"), -1);
    }

    #[test]
    fn bitnot_one() {
        // ~1 = -2 (two's-complement)
        assert_eq!(eval_int("~1"), -2);
    }

    #[test]
    fn bitwise_precedence_and_over_or() {
        // & binds tighter than | → 1 | (2 & 0) = 1 | 0 = 1
        assert_eq!(eval_int("1 | 2 & 0"), 1);
    }

    // ── Shifts ───────────────────────────────────────────────────────────
    #[test]
    fn shl_basic() {
        assert_eq!(eval_int("1 << 4"), 16);
    }

    #[test]
    fn shr_basic() {
        assert_eq!(eval_int("16 >> 2"), 4);
    }

    #[test]
    fn shl_chain() {
        assert_eq!(eval_int("1 << 1 << 2"), 8);
    }

    // ── Comparison ───────────────────────────────────────────────────────
    #[test]
    fn cmp_eq_true() {
        assert_eq!(eval_int("5 == 5"), 1);
    }

    #[test]
    fn cmp_eq_false() {
        assert_eq!(eval_int("5 == 6"), 0);
    }

    #[test]
    fn cmp_ne_true() {
        assert_eq!(eval_int("5 != 6"), 1);
    }

    #[test]
    fn cmp_lt_true() {
        assert_eq!(eval_int("3 < 5"), 1);
    }

    #[test]
    fn cmp_lt_false_on_equal() {
        assert_eq!(eval_int("5 < 5"), 0);
    }

    #[test]
    fn cmp_le_true_on_equal() {
        assert_eq!(eval_int("5 <= 5"), 1);
    }

    #[test]
    fn cmp_gt_true() {
        assert_eq!(eval_int("5 > 3"), 1);
    }

    #[test]
    fn cmp_ge_true_on_equal() {
        assert_eq!(eval_int("5 >= 5"), 1);
    }

    // ── Logical ──────────────────────────────────────────────────────────
    #[test]
    fn logand_true_true() {
        assert_eq!(eval_int("1 && 1"), 1);
    }

    #[test]
    fn logand_short_circuits_on_false() {
        // 0 && X — short-circuit means RHS doesn't matter.
        assert_eq!(eval_int("0 && 99"), 0);
    }

    #[test]
    fn logor_true_short_circuits() {
        // 1 || X — short-circuit yields 1 regardless of RHS.
        assert_eq!(eval_int("1 || 0"), 1);
    }

    #[test]
    fn logor_both_false() {
        assert_eq!(eval_int("0 || 0"), 0);
    }

    #[test]
    fn lognot_true() {
        assert_eq!(eval_int("!0"), 1);
    }

    #[test]
    fn lognot_false() {
        assert_eq!(eval_int("!1"), 0);
    }

    #[test]
    fn lognot_double() {
        // !!5 == truthy(5) == 1
        assert_eq!(eval_int("!!5"), 1);
    }

    // ── Ternary ──────────────────────────────────────────────────────────
    #[test]
    fn ternary_true_branch() {
        assert_eq!(eval_int("1 ? 10 : 20"), 10);
    }

    #[test]
    fn ternary_false_branch() {
        assert_eq!(eval_int("0 ? 10 : 20"), 20);
    }

    #[test]
    fn ternary_condition_is_expression() {
        assert_eq!(eval_int("(3 < 5) ? 100 : 200"), 100);
    }

    #[test]
    fn ternary_nested_in_true_branch() {
        assert_eq!(eval_int("1 ? (0 ? 1 : 2) : 3"), 2);
    }

    // ── Float arithmetic ─────────────────────────────────────────────────
    #[test]
    fn float_add() {
        assert!((eval_float("1.5 + 2.5") - 4.0).abs() < 1e-9);
    }

    #[test]
    fn float_div_does_not_truncate() {
        // 1.0 / 4.0 keeps fractional part. C zsh: `(( 1.0 / 4.0 ))` = 0.25.
        assert!((eval_float("1.0 / 4.0") - 0.25).abs() < 1e-9);
    }

    // ── collect_identifiers (pure helper) ────────────────────────────────
    #[test]
    fn collect_identifiers_bare() {
        let c = ArithCompiler::new("");
        let names = c.collect_identifiers("a + b * c");
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn collect_identifiers_dollar_prefixed() {
        let c = ArithCompiler::new("");
        let names = c.collect_identifiers("$x + 1");
        assert_eq!(names, vec!["x"]);
    }

    #[test]
    fn collect_identifiers_braced() {
        let c = ArithCompiler::new("");
        let names = c.collect_identifiers("${count} * 2");
        assert_eq!(names, vec!["count"]);
    }

    #[test]
    fn collect_identifiers_positional() {
        let c = ArithCompiler::new("");
        let names = c.collect_identifiers("$1 + $2");
        assert_eq!(names, vec!["1", "2"]);
    }

    #[test]
    fn collect_identifiers_dedups() {
        let c = ArithCompiler::new("");
        let names = c.collect_identifiers("a + a + b + a");
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn collect_identifiers_ignores_pure_numeric() {
        let c = ArithCompiler::new("");
        let names = c.collect_identifiers("42 + 7 * 3");
        assert!(names.is_empty(), "got names: {names:?}");
    }

    // ── slot_for (variable slot allocator) ───────────────────────────────
    #[test]
    fn slot_for_first_var_gets_slot_zero() {
        let mut c = ArithCompiler::new("");
        assert_eq!(c.slot_for("x"), 0);
    }

    #[test]
    fn slot_for_second_var_gets_slot_one() {
        let mut c = ArithCompiler::new("");
        let _ = c.slot_for("x");
        assert_eq!(c.slot_for("y"), 1);
    }

    #[test]
    fn slot_for_repeated_name_returns_same_slot() {
        let mut c = ArithCompiler::new("");
        let s1 = c.slot_for("a");
        let s2 = c.slot_for("a");
        assert_eq!(s1, s2);
    }

    // ── Chunk shape ──────────────────────────────────────────────────────
    #[test]
    fn compile_emits_pushframe_and_returnvalue_brackets() {
        let chunk = ArithCompiler::new("1").compile();
        assert!(
            matches!(chunk.ops.first(), Some(Op::PushFrame)),
            "first op should be PushFrame, got {:?}",
            chunk.ops.first()
        );
        assert!(
            matches!(chunk.ops.last(), Some(Op::ReturnValue)),
            "last op should be ReturnValue, got {:?}",
            chunk.ops.last()
        );
    }

    #[test]
    fn compile_sets_source_marker() {
        let chunk = ArithCompiler::new("1").compile();
        assert_eq!(chunk.source, "$((...))");
    }
}
