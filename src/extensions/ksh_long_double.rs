//! ksh93's `Sfdouble_t` for the `--ksh` drop-in: the host's long double,
//! computed in software.
//!
//! !!! RUST-ONLY MODULE — no zsh C counterpart !!!
//!
//! ksh93 evaluates every arithmetic value as `Sfdouble_t`, C's `long double`
//! (ksh93u+m streval.c), whose width is the host's: a 64-bit significand on
//! x86 (x87), 113 bits on aarch64/riscv64/s390x Linux (binary128), 53 bits
//! on Apple arm64 where long double is double. zsh's `mnumber` holds an
//! f64, so the 11 or 60 extra bits of a ksh93 result had nowhere to live and
//! `print $((1.0/3))` on x86_64 printed f64's digits past the sixteenth.
//! This module is that home.
//!
//! A [`LongDouble`] is an exact `mant × 2^exp` with [`LDBL_MANT_DIG`]
//! significand bits. Every operation is computed exactly in a `BigUint` and
//! rounded once, half to even, which is what the hardware does. `pow` and the
//! math functions stay in double, as ksh93's own calls to `pow()` do
//! (`print $((2**0.5))` is the double's digits on every host).
//!
//! A result travels in an `mnumber` through the arena below: when `type_` is
//! `MN_FLOAT` the C union's `l` is dead, so it carries a handle into a
//! per-evaluation arena ([`store_float`], [`load`]), and `d` keeps the f64
//! rounding for every reader that does not know about the arena. The arena
//! is cleared when a top-level `matheval` begins ([`begin_evaluation`]); a
//! handle from an older evaluation reads as its `d`.

use crate::dash_mode::{LDBL_DIG, LDBL_MANT_DIG};
use crate::ported::zsh_h::{mnumber, MN_FLOAT, MN_INTEGER};
use num_bigint::BigUint;
use num_traits::{One, ToPrimitive, Zero};
use std::cell::RefCell;
use std::cmp::Ordering;

/// A long double value. `Finite` is exact `mant × 2^exp` with `mant` holding
/// exactly the format's significand bits (the top bit set).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LongDouble {
    negative: bool,
    kind: Kind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Zero,
    Finite { mant: u128, exp: i64 },
    Inf,
    NaN,
}

/// The exponent a format cannot reach: a value of 2^max_exp or more is an
/// infinity. 1024 for a double, 16384 for x87 and binary128.
fn max_exp(prec: u32) -> i64 {
    if prec == 53 {
        1024
    } else {
        16384
    }
}

/// The exponent of the smallest normal value: 2^-1022 for a double, 2^-16382
/// for x87 and binary128. ksh93 flushes anything below it to zero, constants
/// and results alike (`$((1e-310))`, `$((1e-300 * 1e-10))` and
/// `$((2.2e-308 / 4))` all print `0` on ksh93u+m 1.0.10).
fn min_exp(prec: u32) -> i64 {
    if prec == 53 {
        -1022
    } else {
        -16382
    }
}

/// Round an exact `big × 2^exp` to `prec` significand bits, half to even.
fn round_to(prec: u32, negative: bool, big: BigUint, exp: i64) -> LongDouble {
    if big.is_zero() {
        return LongDouble { negative, kind: Kind::Zero };
    }
    let bits = big.bits() as i64;
    let (mut mant, mut exp) = if bits > i64::from(prec) {
        let shift = (bits - i64::from(prec)) as u64;
        let dropped = &big & ((BigUint::one() << shift) - BigUint::one());
        let mut kept = &big >> shift;
        let half = BigUint::one() << (shift - 1);
        if dropped > half || (dropped == half && kept.bit(0)) {
            kept += BigUint::one();
        }
        (kept, exp + shift as i64)
    } else {
        let shift = (i64::from(prec) - bits) as u64;
        (big << shift, exp - shift as i64)
    };
    // A carry out of 1.111… makes 10.000…: one more bit, one less in the mantissa.
    if mant.bits() > u64::from(prec) {
        mant >>= 1u32;
        exp += 1;
    }
    if exp + i64::from(prec) > max_exp(prec) {
        return LongDouble { negative, kind: Kind::Inf };
    }
    if exp + i64::from(prec) - 1 < min_exp(prec) {
        return LongDouble { negative, kind: Kind::Zero };
    }
    let mant = mant.to_u128().expect("a significand of at most 113 bits fits u128");
    LongDouble { negative, kind: Kind::Finite { mant, exp } }
}

/// C's `ldexp(x, e)` for an exactly representable `x`.
fn ldexp(mut x: f64, mut e: i64) -> f64 {
    while e > 1000 {
        x *= 2f64.powi(1000);
        e -= 1000;
    }
    while e < -1000 {
        x *= 2f64.powi(-1000);
        e += 1000;
    }
    x * 2f64.powi(e as i32)
}

/// The host CPU's answer for a `(long long)` cast that does not fit:
/// x86's integer indefinite `INT64_MIN`; aarch64 saturates and maps NaN
/// to 0 (see `zlong_cast`).
fn cast_out_of_range(negative: bool, nan: bool) -> i64 {
    if cfg!(any(target_arch = "x86", target_arch = "x86_64")) || (negative && !nan) {
        i64::MIN
    } else if nan {
        0
    } else {
        i64::MAX
    }
}

impl LongDouble {
    pub const ZERO: LongDouble = LongDouble { negative: false, kind: Kind::Zero };

    pub fn from_i64(v: i64) -> Self {
        Self::from_i128(i128::from(v))
    }

    pub fn from_i128(v: i128) -> Self {
        Self::from_i128_p(v, LDBL_MANT_DIG)
    }

    fn from_i128_p(v: i128, prec: u32) -> Self {
        round_to(prec, v < 0, BigUint::from(v.unsigned_abs()), 0)
    }

    /// Widen a double: exact in every long double format.
    pub fn from_f64(x: f64) -> Self {
        Self::from_f64_p(x, LDBL_MANT_DIG)
    }

    fn from_f64_p(x: f64, prec: u32) -> Self {
        let negative = x.is_sign_negative();
        if x.is_nan() {
            return LongDouble { negative, kind: Kind::NaN };
        }
        if x.is_infinite() {
            return LongDouble { negative, kind: Kind::Inf };
        }
        let bits = x.to_bits();
        let exp_bits = ((bits >> 52) & 0x7ff) as i64;
        let frac = bits & ((1u64 << 52) - 1);
        let (mant, exp) = if exp_bits == 0 {
            (frac, -1074)
        } else {
            (frac | (1u64 << 52), exp_bits - 1075)
        };
        round_to(prec, negative, BigUint::from(mant), exp)
    }

    /// `strtold` of a decimal constant (`7.0`, `.5`, `1e20`, `-3`), correctly
    /// rounded; `None` for anything else.
    pub fn parse(text: &str) -> Option<Self> {
        Self::parse_p(text, LDBL_MANT_DIG)
    }

    fn parse_p(text: &str, prec: u32) -> Option<Self> {
        let (negative, rest) = match text.as_bytes().first() {
            Some(b'-') => (true, &text[1..]),
            Some(b'+') => (false, &text[1..]),
            _ => (false, text),
        };
        let (mantissa, exponent) = match rest.split_once(['e', 'E']) {
            Some((m, e)) => (m, e.parse::<i64>().ok()?),
            None => (rest, 0),
        };
        let (int_part, frac_part) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        if int_part.is_empty() && frac_part.is_empty() {
            return None;
        }
        let digits = format!("{int_part}{frac_part}");
        if !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let value = BigUint::parse_bytes(digits.as_bytes(), 10)?;
        let decimal_exp = exponent - frac_part.len() as i64;
        if value.is_zero() {
            return Some(LongDouble { negative, kind: Kind::Zero });
        }
        if decimal_exp >= 0 {
            let scaled = value * BigUint::from(10u32).pow(decimal_exp as u32);
            return Some(round_to(prec, negative, scaled, 0));
        }
        let divisor = BigUint::from(10u32).pow((-decimal_exp) as u32);
        Some(exact_quotient(prec, negative, value, &divisor, 0))
    }

    pub fn is_zero(self) -> bool {
        self.kind == Kind::Zero
    }

    pub fn neg(mut self) -> Self {
        self.negative = !self.negative;
        self
    }

    pub fn add(self, other: Self) -> Self {
        self.add_p(other, LDBL_MANT_DIG)
    }

    fn add_p(self, other: Self, prec: u32) -> Self {
        use Kind::*;
        match (self.kind, other.kind) {
            (NaN, _) | (_, NaN) => LongDouble { negative: false, kind: NaN },
            (Inf, Inf) if self.negative != other.negative => LongDouble { negative: false, kind: NaN },
            (Inf, _) => self,
            (_, Inf) => other,
            // -0 + -0 is -0; any other zero sum is +0.
            (Zero, Zero) => LongDouble { negative: self.negative && other.negative, kind: Zero },
            (Zero, _) => other,
            (_, Zero) => self,
            (Finite { mant: ma, exp: ea }, Finite { mant: mb, exp: eb }) => {
                let exp = ea.min(eb);
                let a = BigUint::from(ma) << (ea - exp) as u64;
                let b = BigUint::from(mb) << (eb - exp) as u64;
                if self.negative == other.negative {
                    return round_to(prec, self.negative, a + b, exp);
                }
                match a.cmp(&b) {
                    Ordering::Equal => LongDouble::ZERO,
                    Ordering::Greater => round_to(prec, self.negative, a - b, exp),
                    Ordering::Less => round_to(prec, other.negative, b - a, exp),
                }
            }
        }
    }

    pub fn sub(self, other: Self) -> Self {
        self.add(other.neg())
    }

    pub fn mul(self, other: Self) -> Self {
        self.mul_p(other, LDBL_MANT_DIG)
    }

    fn mul_p(self, other: Self, prec: u32) -> Self {
        use Kind::*;
        let negative = self.negative != other.negative;
        match (self.kind, other.kind) {
            (NaN, _) | (_, NaN) | (Inf, Zero) | (Zero, Inf) => LongDouble { negative: false, kind: NaN },
            (Inf, _) | (_, Inf) => LongDouble { negative, kind: Inf },
            (Zero, _) | (_, Zero) => LongDouble { negative, kind: Zero },
            (Finite { mant: ma, exp: ea }, Finite { mant: mb, exp: eb }) => {
                round_to(prec, negative, BigUint::from(ma) * BigUint::from(mb), ea + eb)
            }
        }
    }

    pub fn div(self, other: Self) -> Self {
        self.div_p(other, LDBL_MANT_DIG)
    }

    fn div_p(self, other: Self, prec: u32) -> Self {
        use Kind::*;
        let negative = self.negative != other.negative;
        match (self.kind, other.kind) {
            (NaN, _) | (_, NaN) | (Inf, Inf) | (Zero, Zero) => LongDouble { negative: false, kind: NaN },
            (Inf, _) | (_, Zero) => LongDouble { negative, kind: Inf },
            (_, Inf) | (Zero, _) => LongDouble { negative, kind: Zero },
            (Finite { mant: ma, exp: ea }, Finite { mant: mb, exp: eb }) => {
                exact_quotient(prec, negative, BigUint::from(ma), &BigUint::from(mb), ea - eb)
            }
        }
    }

    /// Round to a double, half to even.
    pub fn to_f64(self) -> f64 {
        let sign = if self.negative { -1.0 } else { 1.0 };
        match self.kind {
            Kind::NaN => f64::NAN,
            Kind::Inf => sign * f64::INFINITY,
            Kind::Zero => sign * 0.0,
            Kind::Finite { mant, exp } => match round_to(53, self.negative, BigUint::from(mant), exp).kind {
                Kind::Finite { mant, exp } => sign * ldexp(mant as f64, exp),
                _ => sign * f64::INFINITY,
            },
        }
    }

    /// C's `(Sflong_t)` cast: truncation toward zero, with the host CPU's
    /// answer when the value does not fit.
    pub fn host_cast_i64(self) -> i64 {
        let Kind::Finite { mant, exp } = self.kind else {
            return match self.kind {
                Kind::Zero => 0,
                Kind::NaN => cast_out_of_range(self.negative, true),
                Kind::Inf | Kind::Finite { .. } => cast_out_of_range(self.negative, false),
            };
        };
        let magnitude: u128 = if exp >= 0 {
            if exp >= 64 || mant.leading_zeros() < exp as u32 + 1 {
                return cast_out_of_range(self.negative, false);
            }
            mant << exp
        } else if -exp >= 128 {
            0
        } else {
            mant >> (-exp)
        };
        let limit = 1u128 << 63;
        if magnitude < limit {
            let v = magnitude as i64;
            if self.negative {
                -v
            } else {
                v
            }
        } else if magnitude == limit && self.negative {
            i64::MIN
        } else {
            cast_out_of_range(self.negative, false)
        }
    }

    /// ksh93's test for printing a result as an integer: the cast
    /// round-trips (macro.c:2188 `(Sflong_t)num != num`).
    pub fn fits_host_cast(self) -> bool {
        Self::from_i64(self.host_cast_i64()).partial_cmp(&self) == Some(Ordering::Equal)
    }

    /// `%.*Lg` with `digits` significant digits: C's `%g` rules on the exact
    /// decimal expansion, rounded half to even, trailing zeros dropped.
    pub fn format_g(self, digits: usize) -> String {
        let sign = if self.negative { "-" } else { "" };
        let Kind::Finite { mant, exp } = self.kind else {
            return match self.kind {
                Kind::Zero => format!("{sign}0"),
                Kind::Inf => format!("{sign}inf"),
                Kind::NaN | Kind::Finite { .. } => "nan".to_string(),
            };
        };
        // Exact decimal digits and the number of them before the point.
        let (mut text, mut point) = if exp >= 0 {
            let n = BigUint::from(mant) << exp as u64;
            let s = n.to_string();
            let len = s.len() as i64;
            (s, len)
        } else {
            let n = BigUint::from(mant) * BigUint::from(5u32).pow((-exp) as u32);
            let s = n.to_string();
            let len = s.len() as i64;
            (s, len + exp)
        };
        if text.len() > digits {
            let rest = text.split_off(digits);
            let first = rest.as_bytes()[0];
            let tie = first == b'5' && rest[1..].bytes().all(|b| b == b'0');
            let last_odd = text.as_bytes()[digits - 1] % 2 == 1;
            if first > b'5' || (first == b'5' && (!tie || last_odd)) {
                text = increment_decimal(&text);
                if text.len() > digits {
                    text.pop();
                    point += 1;
                }
            }
        }
        while text.len() > 1 && text.ends_with('0') {
            text.pop();
        }
        let x = point - 1;
        let body = if x < -4 || x >= digits as i64 {
            let mantissa = if text.len() > 1 { format!("{}.{}", &text[..1], &text[1..]) } else { text };
            format!("{mantissa}e{}{:02}", if x < 0 { '-' } else { '+' }, x.abs())
        } else if point <= 0 {
            format!("0.{}{}", "0".repeat((-point) as usize), text)
        } else if point as usize >= text.len() {
            format!("{text}{}", "0".repeat(point as usize - text.len()))
        } else {
            format!("{}.{}", &text[..point as usize], &text[point as usize..])
        };
        format!("{sign}{body}")
    }
}

/// `value / divisor × 2^exp`, rounded to `prec` bits: the quotient is taken
/// with `prec + 3` bits and a sticky bit for any remainder.
fn exact_quotient(prec: u32, negative: bool, value: BigUint, divisor: &BigUint, exp: i64) -> LongDouble {
    let shift = u64::from(prec) + 3 + divisor.bits();
    let scaled = value << shift;
    let mut quotient = &scaled / divisor;
    let mut exp = exp - shift as i64;
    if !(&scaled % divisor).is_zero() {
        quotient = (quotient << 1u32) | BigUint::one();
        exp -= 1;
    }
    round_to(prec, negative, quotient, exp)
}

/// `"1299"` → `"1300"`, `"999"` → `"1000"`.
fn increment_decimal(digits: &str) -> String {
    let mut bytes = digits.as_bytes().to_vec();
    for b in bytes.iter_mut().rev() {
        if *b == b'9' {
            *b = b'0';
        } else {
            *b += 1;
            return String::from_utf8(bytes).expect("ascii digits");
        }
    }
    bytes.insert(0, b'1');
    String::from_utf8(bytes).expect("ascii digits")
}

impl PartialOrd for LongDouble {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        use Kind::*;
        let sign = |v: &LongDouble| if v.negative { -1 } else { 1 };
        match (self.kind, other.kind) {
            (NaN, _) | (_, NaN) => None,
            (Zero, Zero) => Some(Ordering::Equal),
            (Zero, _) => Some(if other.negative { Ordering::Greater } else { Ordering::Less }),
            (_, Zero) => Some(if self.negative { Ordering::Less } else { Ordering::Greater }),
            _ if self.negative != other.negative => Some(sign(self).cmp(&sign(other))),
            (Inf, Inf) => Some(Ordering::Equal),
            (Inf, _) => Some(if self.negative { Ordering::Less } else { Ordering::Greater }),
            (_, Inf) => Some(if self.negative { Ordering::Greater } else { Ordering::Less }),
            (Finite { mant: ma, exp: ea }, Finite { mant: mb, exp: eb }) => {
                // Order by the exponent of the top bit, then by the significands
                // left-aligned, so values of different precisions compare too.
                let top = |m: u128, e: i64| e + i64::from(128 - m.leading_zeros());
                let aligned = |m: u128| m << m.leading_zeros();
                let magnitude = top(ma, ea).cmp(&top(mb, eb)).then(aligned(ma).cmp(&aligned(mb)));
                Some(if self.negative { magnitude.reverse() } else { magnitude })
            }
        }
    }
}

// ─── The per-evaluation arena behind `mnumber` ─────────────────────────────

struct Arena {
    generation: u32,
    values: Vec<LongDouble>,
}

thread_local! {
    static ARENA: RefCell<Arena> = RefCell::new(Arena { generation: 1, values: Vec::new() });
}

/// Start a top-level evaluation: handles of the previous one go stale.
pub fn begin_evaluation() {
    ARENA.with(|arena| {
        let mut arena = arena.borrow_mut();
        arena.generation = arena.generation.wrapping_add(1).max(1);
        arena.values.clear();
    });
}

fn handle(generation: u32, index: usize) -> i64 {
    (i64::from(generation) << 32) | (index as i64 + 1)
}

fn lookup(handle: i64) -> Option<LongDouble> {
    if handle <= 0 {
        return None;
    }
    let generation = (handle >> 32) as u32;
    let index = ((handle & 0xffff_ffff) - 1) as usize;
    ARENA.with(|arena| {
        let arena = arena.borrow();
        (arena.generation == generation).then(|| arena.values.get(index).copied()).flatten()
    })
}

/// A float-typed result: `MN_FLOAT`, the long double in the arena, `d` its
/// double rounding.
pub fn store_float(value: LongDouble) -> mnumber {
    let l = ARENA.with(|arena| {
        let mut arena = arena.borrow_mut();
        arena.values.push(value);
        handle(arena.generation, arena.values.len() - 1)
    });
    mnumber { l, d: value.to_f64(), type_: MN_FLOAT }
}

/// An integer-typed result, as ksh93 reads one: the integer when the host
/// cast round-trips (`INT64_MAX + 1000` is `INT64_MAX` on aarch64), else a
/// float (`9.22337203685477581e+18` on x86).
///
/// An integer's `d` is otherwise unused, so it carries the sign of a zero
/// result: ksh93 holds `-1*0` as a long double `-0`, and `signbit` sees it.
pub fn store(value: LongDouble) -> mnumber {
    if value.fits_host_cast() {
        let d = if value.is_zero() && value.negative { -0.0 } else { 0.0 };
        mnumber { l: value.host_cast_i64(), d, type_: MN_INTEGER }
    } else {
        store_float(value)
    }
}

/// True for the integer-typed `-0` [`store`] produces.
pub fn is_negative_zero(n: &mnumber) -> bool {
    n.type_ == MN_INTEGER && n.l == 0 && n.d.is_sign_negative()
}

/// The long double an `mnumber` denotes: an integer widened, a float read
/// from the arena, or its `d` when the handle is not this evaluation's.
pub fn load(n: &mnumber) -> LongDouble {
    if n.type_ == MN_FLOAT {
        lookup(n.l).unwrap_or_else(|| LongDouble::from_f64(n.d))
    } else {
        LongDouble::from_i64(n.l)
    }
}

// ─── What `math.rs` calls at its ksh93 gates ───────────────────────────────

/// An integer operand rounded through the long double, as ksh93 holds it:
/// exact on x86 and binary128 hosts, 53 bits on Apple arm64 (where
/// `9007199254740993` is 9007199254740992).
pub fn round_int(n: mnumber) -> mnumber {
    if n.type_ == MN_INTEGER {
        store(LongDouble::from_i64(n.l))
    } else {
        n
    }
}

pub fn int_add(a: i64, b: i64) -> mnumber {
    store(LongDouble::from_i128(i128::from(a) + i128::from(b)))
}

pub fn int_sub(a: i64, b: i64) -> mnumber {
    store(LongDouble::from_i128(i128::from(a) - i128::from(b)))
}

pub fn int_mul(a: i64, b: i64) -> mnumber {
    store(LongDouble::from_i128(i128::from(a) * i128::from(b)))
}

/// `a ** b` on integers: ksh93 calls C's `pow()`, so the result is a double
/// widened (streval.c:338 `num = pow(sp[-1],num)`).
pub fn int_pow(a: i64, b: i64) -> mnumber {
    store(LongDouble::from_f64((a as f64).powf(b as f64)))
}

pub fn float_add(a: &mnumber, b: &mnumber) -> mnumber {
    store_float(load(a).add(load(b)))
}

pub fn float_sub(a: &mnumber, b: &mnumber) -> mnumber {
    store_float(load(a).sub(load(b)))
}

pub fn float_mul(a: &mnumber, b: &mnumber) -> mnumber {
    store_float(load(a).mul(load(b)))
}

pub fn float_div(a: &mnumber, b: &mnumber) -> mnumber {
    store_float(load(a).div(load(b)))
}

pub fn float_neg(n: &mnumber) -> mnumber {
    store_float(load(n).neg())
}

/// Order two operands as long doubles; `None` when either is NaN.
pub fn float_cmp(a: &mnumber, b: &mnumber) -> Option<Ordering> {
    load(a).partial_cmp(&load(b))
}

/// A decimal integer constant too large for i64 (`99999999999999999999`),
/// read as ksh93 reads every constant: through `strtold`.
pub fn integer_constant(text: &str, negative: bool) -> Option<mnumber> {
    LongDouble::parse(text).map(|v| store(if negative { v.neg() } else { v }))
}

/// A float constant (`7.0`, `1e20`), exact in the host's long double.
pub fn float_constant(text: &str, negative: bool) -> Option<mnumber> {
    LongDouble::parse(text).map(|v| store_float(if negative { v.neg() } else { v }))
}

/// ksh93's text for an arithmetic result (macro.c:2188-2193): the integer
/// when the cast round-trips and the value is nonzero, `%Lg` for a zero
/// (`-0` keeps its sign), else `%.*Lg` with `LDBL_DIG`.
pub fn format_result(n: &mnumber) -> String {
    let value = load(n);
    if value.is_zero() {
        value.format_g(6)
    } else if value.fits_host_cast() {
        value.host_cast_i64().to_string()
    } else {
        value.format_g(LDBL_DIG as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const X87: u32 = 64;
    const QUAD: u32 = 113;
    const DOUBLE: u32 = 53;

    fn parse(text: &str, prec: u32) -> LongDouble {
        LongDouble::parse_p(text, prec).expect("a decimal constant")
    }

    /// The digits ksh93u+m 1.0.10 prints for `print $((1.0/3))` and
    /// `$((2**0.5))`: x86_64 Linux (18 digits of an x87 quotient) and Apple
    /// arm64 (15 digits of a double), as measured by the emulation matrix.
    #[test]
    fn division_rounds_to_the_hosts_significand() {
        let third = parse("1.0", X87).div_p(parse("3", X87), X87);
        assert_eq!(third.format_g(18), "0.333333333333333333");
        let third = parse("1.0", DOUBLE).div_p(parse("3", DOUBLE), DOUBLE);
        assert_eq!(third.format_g(15), "0.333333333333333");
        let third = parse("1.0", QUAD).div_p(parse("3", QUAD), QUAD);
        assert_eq!(third.format_g(33), "0.333333333333333333333333333333333");
        let sqrt2 = LongDouble::from_f64_p(2f64.powf(0.5), X87);
        assert_eq!(sqrt2.format_g(18), "1.41421356237309515");
        assert_eq!(parse("7.0", X87).div_p(parse("2", X87), X87).format_g(18), "3.5");
    }

    /// `%g` formatting: integral values, the exponent threshold at the
    /// precision, small values, zero's sign, and half-to-even rounding.
    #[test]
    fn format_g_follows_printf() {
        assert_eq!(parse("1e20", X87).format_g(18), "1e+20");
        assert_eq!(parse("5.0", X87).format_g(18), "5");
        assert_eq!(parse("123456789012345678", X87).format_g(18), "123456789012345678");
        assert_eq!(parse("1234567890123456789", X87).format_g(18), "1.23456789012345679e+18");
        assert_eq!(parse("0.0001", X87).format_g(18), "0.0001");
        assert_eq!(parse("0.00001", DOUBLE).format_g(15), "1e-05");
        assert_eq!(parse("-0", X87).format_g(18), "-0");
        // Subnormals flush to zero, as ksh93 prints them.
        assert_eq!(parse("-1e-320", DOUBLE).format_g(15), "-0");
        assert_eq!(parse("2.2e-308", DOUBLE).format_g(15), "0");
        assert_eq!(parse("2.3e-308", DOUBLE).format_g(15), "2.3e-308");
        assert_eq!(parse("1e-310", X87).format_g(18), "1e-310");
        assert_eq!(LongDouble::from_i128_p(1_844_674_407_370_955_160, X87).format_g(18), "1.84467440737095516e+18");
        assert_eq!(parse("0.125", DOUBLE).format_g(2), "0.12");
        assert_eq!(parse("0.375", DOUBLE).format_g(2), "0.38");
    }

    /// Integer `+ - *` overflow as the emulation matrix measures it on both
    /// hosts: aarch64 saturates (`INT64_MAX + 1000` is `INT64_MAX`), x86's
    /// cast gives INT64_MIN so the sum stays a float.
    #[test]
    fn integer_overflow_follows_the_host_cast() {
        let max = LongDouble::from_i128_p(i128::from(i64::MAX), DOUBLE);
        let sum = max.add_p(LongDouble::from_i128_p(1000, DOUBLE), DOUBLE);
        assert_eq!(sum.format_g(15), "9.22337203685478e+18");
        let big = LongDouble::from_i128_p(i128::from(i64::MAX) + 1_000_000, DOUBLE);
        assert_eq!(big.format_g(15), "9.22337203685578e+18");
        let two63 = LongDouble::from_i128_p(1i128 << 63, X87);
        assert_eq!(two63.format_g(18), "9.22337203685477581e+18");
        let two64 = LongDouble::from_i128_p(1i128 << 64, X87);
        assert_eq!(two64.format_g(18), "1.84467440737095516e+19");
        let three40 = LongDouble::from_i128_p(3i128.pow(40), X87);
        assert_eq!(three40.format_g(18), "1.21576654590569288e+19");
        // Exact on x87, rounded to the nearest even double on Apple arm64.
        assert_eq!(LongDouble::from_i128_p(9007199254740993, X87).format_g(18), "9007199254740993");
        assert_eq!(LongDouble::from_i128_p(9007199254740993, DOUBLE).host_cast_i64(), 9007199254740992);
    }

    /// The host cast: in-range values truncate, `-2^63` fits, `2^63` and
    /// above take the CPU's out-of-range answer.
    #[test]
    fn host_cast_truncates_and_handles_the_edges() {
        assert_eq!(parse("3.9", X87).host_cast_i64(), 3);
        assert_eq!(parse("-3.9", X87).host_cast_i64(), -3);
        assert_eq!(parse("-9223372036854775808", X87).host_cast_i64(), i64::MIN);
        assert!(parse("-9223372036854775808", X87).fits_host_cast());
        let two63 = parse("9223372036854775808", X87);
        assert_eq!(two63.host_cast_i64(), cast_out_of_range(false, false));
        // The host cast of 2^63 is INT64_MIN on x86 and INT64_MAX elsewhere; it
        // round-trips only where INT64_MAX itself rounds to 2^63, a 53-bit long
        // double (Apple arm64). On aarch64 Linux the 113-bit value stays 2^63 - 1,
        // so ksh93 prints `$((2**63))` as a float there.
        let saturates = cfg!(not(any(target_arch = "x86", target_arch = "x86_64")));
        assert_eq!(two63.fits_host_cast(), saturates && LDBL_MANT_DIG == 53);
        // `fits_host_cast` rounds through the HOST precision, so test it there.
        assert!(parse("9223372036854775807", LDBL_MANT_DIG).fits_host_cast());
        assert!(!parse("0.5", LDBL_MANT_DIG).fits_host_cast());
    }

    /// Round trips and ordering through the arena, and a stale handle falls
    /// back to the double.
    #[test]
    fn arena_keeps_a_result_for_one_evaluation() {
        begin_evaluation();
        let third = store_float(LongDouble::parse("1.0").unwrap().div(LongDouble::parse("3").unwrap()));
        assert_eq!(third.type_, MN_FLOAT);
        assert_eq!(load(&third), LongDouble::parse("1.0").unwrap().div(LongDouble::parse("3").unwrap()));
        assert_eq!(format_result(&third), LongDouble::from_f64(1.0).div(LongDouble::from_f64(3.0)).format_g(LDBL_DIG as usize));
        assert_eq!(float_cmp(&third, &mnumber { l: 1, d: 0.0, type_: MN_INTEGER }), Some(Ordering::Less));
        assert_eq!(format_result(&store(LongDouble::from_i64(7))), "7");
        assert_eq!(format_result(&store_float(LongDouble::parse("-0").unwrap())), "-0");
        begin_evaluation();
        assert_eq!(load(&third), LongDouble::from_f64(third.d));
        assert_eq!(load(&mnumber { l: 0, d: 2.5, type_: MN_FLOAT }), LongDouble::from_f64(2.5));
    }

    #[test]
    fn parse_reads_strtold_forms() {
        assert_eq!(parse(".5", X87), parse("0.5", X87));
        assert_eq!(parse("1e2", X87), parse("100", X87));
        assert_eq!(parse("2.5e-1", X87), parse("0.25", X87));
        assert!(LongDouble::parse("abc").is_none());
        assert!(LongDouble::parse("").is_none());
        // 0.1 is not exact: the x87 and double roundings differ past bit 53.
        assert_ne!(parse("0.1", X87), LongDouble::from_f64_p(0.1, X87));
        assert_eq!(parse("0.1", DOUBLE), LongDouble::from_f64_p(0.1, DOUBLE));
    }
}
