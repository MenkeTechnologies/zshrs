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
    pub input: &'a str,
    pub pos: usize,
    pub builder: ChunkBuilder,
    /// Variable name → slot index
    pub slots: HashMap<String, u16>,
    pub next_slot: u16,
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
    pub fn new(input: &'a str) -> Self {
        Self {
            input,
            pos: 0,
            builder: ChunkBuilder::new(),
            slots: HashMap::new(),
            next_slot: 0,
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

        // Handle hex: 0x...
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
            return Tok::Num(val);
        }

        // Handle octal: 0...
        if self.pos + 1 < self.input.len()
            && self.input.as_bytes()[self.pos] == b'0'
            && self.input.as_bytes()[self.pos + 1].is_ascii_digit()
        {
            while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            let val = i64::from_str_radix(&self.input[start + 1..self.pos], 8).unwrap_or(0);
            return Tok::Num(val);
        }

        // Decimal integer or float
        while self.pos < self.input.len() && self.input.as_bytes()[self.pos].is_ascii_digit() {
            self.pos += 1;
        }

        // Check for float
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
                    Some(b'+') => {
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
                    Some(b'-') => {
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
                            (Tok::MulAssign, String::new()) // **= as mul assign for now
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
                if self.peek_char() == Some(b'&') {
                    self.pos += 1;
                    (Tok::LogAnd, String::new())
                } else {
                    (Tok::BitAnd, String::new())
                }
            }
            b'|' => {
                self.pos += 1;
                if self.peek_char() == Some(b'|') {
                    self.pos += 1;
                    (Tok::LogOr, String::new())
                } else {
                    (Tok::BitOr, String::new())
                }
            }
            b'^' => {
                self.pos += 1;
                (Tok::BitXor, String::new())
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
                        (Tok::Shl, String::new())
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
                        (Tok::Shr, String::new())
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
    // Precedence climbing: comma < assign < ternary < logor < logand <
    // bitor < bitxor < bitand < eq < cmp < shift < add < mul < pow < unary

    pub fn expr(&mut self) {
        self.assign_expr();
    }

    fn assign_expr(&mut self) {
        let save_pos = self.pos;

        // Check for assignment: ident = expr
        self.skip_whitespace();
        if let Some(c) = self.peek_char() {
            if c.is_ascii_alphabetic() || c == b'_' {
                let name = self.read_ident();
                self.skip_whitespace();
                let (tok, _) = self.peek_tok();
                match tok {
                    Tok::Assign => {
                        let _ = self.next_tok(); // consume =
                        let slot = self.slot_for(&name);
                        self.assign_expr();
                        self.builder.emit(Op::Dup, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                        return;
                    }
                    Tok::PlusAssign
                    | Tok::MinusAssign
                    | Tok::MulAssign
                    | Tok::DivAssign
                    | Tok::ModAssign => {
                        let _ = self.next_tok(); // consume op=
                        let slot = self.slot_for(&name);
                        self.builder.emit(Op::GetSlot(slot), 0);
                        self.assign_expr();
                        match tok {
                            Tok::PlusAssign => self.builder.emit(Op::Add, 0),
                            Tok::MinusAssign => self.builder.emit(Op::Sub, 0),
                            Tok::MulAssign => self.builder.emit(Op::Mul, 0),
                            Tok::DivAssign => self.builder.emit(Op::Div, 0),
                            Tok::ModAssign => self.builder.emit(Op::Mod, 0),
                            _ => unreachable!(),
                        };
                        self.builder.emit(Op::Dup, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                        return;
                    }
                    _ => {}
                }
                // Not assignment — rewind
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

    fn ternary_expr(&mut self) {
        self.logor_expr();
        let (tok, _) = self.peek_tok();
        if tok == Tok::Quest {
            let _ = self.next_tok(); // consume ?
            let else_jump = self.builder.emit(Op::JumpIfFalse(0), 0);
            self.expr(); // true branch
            let (colon, _) = self.peek_tok();
            let end_jump = self.builder.emit(Op::Jump(0), 0);
            let else_target = self.builder.current_pos();
            self.builder.patch_jump(else_jump, else_target);
            if colon == Tok::Colon {
                let _ = self.next_tok(); // consume :
            }
            self.expr(); // false branch
            let end_target = self.builder.current_pos();
            self.builder.patch_jump(end_jump, end_target);
        }
    }

    fn logor_expr(&mut self) {
        self.logand_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::LogOr {
                let _ = self.next_tok();
                let skip = self.builder.emit(Op::JumpIfTrueKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.logand_expr();
                self.builder.patch_jump(skip, self.builder.current_pos());
            } else {
                break;
            }
        }
    }

    fn logand_expr(&mut self) {
        self.bitor_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::LogAnd {
                let _ = self.next_tok();
                let skip = self.builder.emit(Op::JumpIfFalseKeep(0), 0);
                self.builder.emit(Op::Pop, 0);
                self.bitor_expr();
                self.builder.patch_jump(skip, self.builder.current_pos());
            } else {
                break;
            }
        }
    }

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

    fn bitand_expr(&mut self) {
        self.equality_expr();
        loop {
            let (tok, _) = self.peek_tok();
            if tok == Tok::BitAnd {
                let _ = self.next_tok();
                self.equality_expr();
                self.builder.emit(Op::BitAnd, 0);
            } else {
                break;
            }
        }
    }

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

    fn comparison_expr(&mut self) {
        self.shift_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Lt => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumLt, 0);
                }
                Tok::Gt => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumGt, 0);
                }
                Tok::Leq => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumLe, 0);
                }
                Tok::Geq => {
                    let _ = self.next_tok();
                    self.shift_expr();
                    self.builder.emit(Op::NumGe, 0);
                }
                _ => break,
            }
        }
    }

    fn shift_expr(&mut self) {
        self.add_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Shl => {
                    let _ = self.next_tok();
                    self.add_expr();
                    self.builder.emit(Op::Shl, 0);
                }
                Tok::Shr => {
                    let _ = self.next_tok();
                    self.add_expr();
                    self.builder.emit(Op::Shr, 0);
                }
                _ => break,
            }
        }
    }

    fn add_expr(&mut self) {
        self.mul_expr();
        loop {
            let (tok, _) = self.peek_tok();
            match tok {
                Tok::Plus => {
                    let _ = self.next_tok();
                    self.mul_expr();
                    self.builder.emit(Op::Add, 0);
                }
                Tok::Minus => {
                    let _ = self.next_tok();
                    self.mul_expr();
                    self.builder.emit(Op::Sub, 0);
                }
                _ => break,
            }
        }
    }

    fn mul_expr(&mut self) {
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
                    self.builder.emit(Op::Div, 0);
                }
                Tok::Mod => {
                    let _ = self.next_tok();
                    self.pow_expr();
                    self.builder.emit(Op::Mod, 0);
                }
                _ => break,
            }
        }
    }

    fn pow_expr(&mut self) {
        self.unary_expr();
        let (tok, _) = self.peek_tok();
        if tok == Tok::Pow {
            let _ = self.next_tok();
            self.pow_expr(); // right-associative
            self.builder.emit(Op::Pow, 0);
        }
    }

    fn unary_expr(&mut self) {
        let (tok, name) = self.peek_tok();
        match tok {
            Tok::Minus => {
                let _ = self.next_tok();
                self.unary_expr();
                self.builder.emit(Op::Negate, 0);
            }
            Tok::Plus => {
                let _ = self.next_tok();
                self.unary_expr();
                // unary + is a no-op on numbers
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
                // Next token must be identifier
                let (_, var_name) = self.next_tok();
                let slot = self.slot_for(&var_name);
                self.builder.emit(Op::PreIncSlot(slot), 0);
            }
            Tok::PreDec => {
                let _ = self.next_tok();
                let (_, var_name) = self.next_tok();
                let slot = self.slot_for(&var_name);
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

                // Check for postfix ++ / --
                let (post_tok, _) = self.peek_tok();
                match post_tok {
                    Tok::PreInc => {
                        // Reused as PostInc here
                        let _ = self.next_tok();
                        self.builder.emit(Op::Dup, 0); // keep old value
                        self.builder.emit(Op::Inc, 0);
                        self.builder.emit(Op::SetSlot(slot), 0);
                        // old value remains on stack (postfix semantics)
                    }
                    Tok::PreDec => {
                        let _ = self.next_tok();
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
}
