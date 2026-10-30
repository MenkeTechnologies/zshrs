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
//! rounded once, half to even, which is what the hardware does. The `**`
//! operator stays in double, as ksh93 calls C's `pow()` for it (`print
//! $((2**0.5))` is the double's digits on every host); the built-in math
//! functions are the host's long double ones (`powl`, `atan2l`, …), below.
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
            (Finite { mant: ma, exp: ea }, Finite { mant: mb, exp: eb }) => exact_sum(
                prec,
                (self.negative, BigUint::from(ma), ea),
                (other.negative, BigUint::from(mb), eb),
            ),
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

/// The sum of two nonzero exact values `±big × 2^exp`, rounded once to `prec`
/// bits. An exact cancellation is `+0`.
fn exact_sum(prec: u32, a: (bool, BigUint, i64), b: (bool, BigUint, i64)) -> LongDouble {
    let exp = a.2.min(b.2);
    let x = a.1 << (a.2 - exp) as u64;
    let y = b.1 << (b.2 - exp) as u64;
    if a.0 == b.0 {
        return round_to(prec, a.0, x + y, exp);
    }
    match x.cmp(&y) {
        Ordering::Equal => LongDouble::ZERO,
        Ordering::Greater => round_to(prec, a.0, x - y, exp),
        Ordering::Less => round_to(prec, b.0, y - x, exp),
    }
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

// ─── ksh93's math functions in long double ─────────────────────────────────
//
// Where long double is wider than double, ksh93 calls the `l` variant of
// every function it builds in: features/math.sh links `${name}l` ahead of
// `${name}` for each data/math.tab entry, so `print $((pow(2,0.5)))` is
// `powl`'s 18 digits on x86_64 Linux. Below are those functions on a
// `LongDouble`: exact where the result is exact (fma, fdim, round, an
// integral power or factorial), else computed in a 256-bit working float and
// rounded once to the host's significand. C99 Annex F gives the special
// cases; fmin/fmax ties follow glibc (sysdeps/x86_64/fpu/s_fmaxl.S,
// math/s_fmax_template.c).

/// Significand bits of [`Big`]: well past binary128's 113.
const WORK_BITS: u64 = 256;

/// A working float `±m × 2^e`, `m` truncated to [`WORK_BITS`] bits.
#[derive(Clone, Debug)]
struct Big {
    negative: bool,
    m: BigUint,
    e: i64,
}

impl Big {
    fn new(negative: bool, m: BigUint, e: i64) -> Big {
        let bits = m.bits();
        if bits > WORK_BITS {
            let shift = bits - WORK_BITS;
            Big { negative, m: m >> shift, e: e + shift as i64 }
        } else {
            Big { negative, m, e }
        }
    }

    fn int(v: i64) -> Big {
        Big::new(v < 0, BigUint::from(v.unsigned_abs()), 0)
    }

    /// A finite long double, exactly; zero for anything else.
    fn of(x: LongDouble) -> Big {
        match x.kind {
            Kind::Finite { mant, exp } => Big::new(x.negative, BigUint::from(mant), exp),
            _ => Big { negative: x.negative, m: BigUint::zero(), e: 0 },
        }
    }

    fn is_zero(&self) -> bool {
        self.m.is_zero()
    }

    /// The exponent of the top bit: `|self|` is in `[2^top, 2^(top+1))`.
    fn top(&self) -> i64 {
        self.e + self.m.bits() as i64 - 1
    }

    fn neg(mut self) -> Big {
        self.negative = !self.negative;
        self
    }

    fn abs(mut self) -> Big {
        self.negative = false;
        self
    }

    /// `self × 2^k`.
    fn shl(mut self, k: i64) -> Big {
        self.e += k;
        self
    }

    fn add(&self, other: &Big) -> Big {
        if self.is_zero() {
            return other.clone();
        }
        if other.is_zero() {
            return self.clone();
        }
        // Align at a few bits under what the larger operand keeps.
        let floor = self.top().max(other.top()) - WORK_BITS as i64 - 4;
        let align = |b: &Big| {
            if b.e >= floor {
                b.m.clone() << (b.e - floor) as u64
            } else {
                b.m.clone() >> (floor - b.e) as u64
            }
        };
        let (a, b) = (align(self), align(other));
        if self.negative == other.negative {
            return Big::new(self.negative, a + b, floor);
        }
        match a.cmp(&b) {
            Ordering::Less => Big::new(other.negative, b - a, floor),
            _ => Big::new(self.negative, a - b, floor),
        }
    }

    fn sub(&self, other: &Big) -> Big {
        self.add(&other.clone().neg())
    }

    fn mul(&self, other: &Big) -> Big {
        Big::new(self.negative != other.negative, &self.m * &other.m, self.e + other.e)
    }

    fn div(&self, other: &Big) -> Big {
        let shift = WORK_BITS + other.m.bits();
        let q = (&self.m << shift) / &other.m;
        Big::new(self.negative != other.negative, q, self.e - shift as i64 - other.e)
    }

    fn sqrt(&self) -> Big {
        let mut shift = (2 * WORK_BITS + 2).saturating_sub(self.m.bits());
        if (self.e - shift as i64) % 2 != 0 {
            shift += 1;
        }
        Big::new(false, (&self.m << shift).sqrt(), (self.e - shift as i64) / 2)
    }

    fn cmp(&self, other: &Big) -> Ordering {
        let d = self.sub(other);
        if d.is_zero() {
            Ordering::Equal
        } else if d.negative {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }

    /// The nearest integer, for a value well inside i64.
    fn round_i64(&self) -> i64 {
        let magnitude = if self.e >= 0 {
            (&self.m << self.e as u64).to_i64().unwrap_or(i64::MAX)
        } else {
            let shift = (-self.e) as u64;
            ((&self.m + (BigUint::one() << (shift - 1))) >> shift).to_i64().unwrap_or(i64::MAX)
        };
        if self.negative {
            -magnitude
        } else {
            magnitude
        }
    }

    /// Integer part and fraction of a non-negative value.
    fn split(&self) -> (BigUint, Big) {
        if self.e >= 0 {
            return (&self.m << self.e as u64, Big::int(0));
        }
        let shift = (-self.e) as u64;
        let fraction = &self.m & ((BigUint::one() << shift) - BigUint::one());
        (&self.m >> shift, Big::new(false, fraction, self.e))
    }

    fn to_ld(&self, prec: u32) -> LongDouble {
        if self.is_zero() {
            return LongDouble { negative: self.negative, kind: Kind::Zero };
        }
        round_to(prec, self.negative, self.m.clone(), self.e)
    }
}

/// `Σ s^k z^(2k+1)/(2k+1)` for small `z`: atanh with `s = 1`, atan with `s = -1`.
fn odd_series(z: &Big, alternate: bool) -> Big {
    let z2 = z.mul(z);
    let mut power = z.clone();
    let mut sum = Big::int(0);
    for k in 0i64.. {
        let mut term = power.div(&Big::int(2 * k + 1));
        if term.is_zero() || (!sum.is_zero() && term.top() < sum.top() - WORK_BITS as i64 - 4) {
            break;
        }
        if alternate && k % 2 == 1 {
            term = term.neg();
        }
        sum = sum.add(&term);
        power = power.mul(&z2);
    }
    sum
}

fn ln2() -> Big {
    static LN2: std::sync::OnceLock<Big> = std::sync::OnceLock::new();
    // ln 2 = 2 atanh(1/3).
    LN2.get_or_init(|| odd_series(&Big::int(1).div(&Big::int(3)), false).shl(1)).clone()
}

fn pi() -> Big {
    static PI: std::sync::OnceLock<Big> = std::sync::OnceLock::new();
    // Machin: π = 16 atan(1/5) - 4 atan(1/239).
    PI.get_or_init(|| {
        let a = odd_series(&Big::int(1).div(&Big::int(5)), true).shl(4);
        a.sub(&odd_series(&Big::int(1).div(&Big::int(239)), true).shl(2))
    })
    .clone()
}

/// ln x for x > 0: x = 2^n f with f in [0.75, 1.5), ln f = 2 atanh((f-1)/(f+1)).
fn ln(x: &Big) -> Big {
    let mut n = x.top();
    let mut f = x.clone().shl(-n);
    if f.cmp(&Big::int(3).shl(-1)) != Ordering::Less {
        f = f.shl(-1);
        n += 1;
    }
    let one = Big::int(1);
    let z = f.sub(&one).div(&f.add(&one));
    Big::int(n).mul(&ln2()).add(&odd_series(&z, false).shl(1))
}

/// e^x for |x| below 2^16: x = k ln 2 + r, e^r by Taylor on r/2^12, squared back.
fn exp(x: &Big) -> Big {
    const HALVINGS: i64 = 12;
    let ln2 = ln2();
    let k = x.div(&ln2).round_i64();
    let r = x.sub(&Big::int(k).mul(&ln2)).shl(-HALVINGS);
    let mut sum = Big::int(1);
    let mut term = Big::int(1);
    for i in 1.. {
        term = term.mul(&r).div(&Big::int(i));
        if term.is_zero() || term.top() < -(WORK_BITS as i64) - 8 {
            break;
        }
        sum = sum.add(&term);
    }
    for _ in 0..HALVINGS {
        sum = sum.mul(&sum);
    }
    sum.shl(k)
}

/// atan t for t ≥ 0: past 1 as π/2 - atan(1/t); three angle halvings
/// `t / (1 + √(1+t²))` bring t under tan(π/32) for the series.
fn atan(t: &Big) -> Big {
    let one = Big::int(1);
    if t.cmp(&one) == Ordering::Greater {
        return pi().shl(-1).sub(&atan(&one.div(t)));
    }
    let mut t = t.clone();
    for _ in 0..3 {
        t = t.div(&one.add(&one.add(&t.mul(&t)).sqrt()));
    }
    odd_series(&t, true).shl(3)
}

/// sin(π f) for f in [0, 1].
fn sin_pi(f: &Big) -> Big {
    let half = Big::int(1).shl(-1);
    let f = if f.cmp(&half) == Ordering::Greater { Big::int(1).sub(f) } else { f.clone() };
    let theta = pi().mul(&f);
    let theta2 = theta.mul(&theta);
    let mut sum = theta.clone();
    let mut term = theta;
    for i in 1i64.. {
        term = term.mul(&theta2).div(&Big::int((2 * i) * (2 * i + 1))).neg();
        if term.is_zero() || term.top() < sum.top() - WORK_BITS as i64 - 4 {
            break;
        }
        sum = sum.add(&term);
    }
    sum
}

/// Γ(x) for x > 0. x = r + n with r in [1, 2) (or x < 1, Γ(x) = Γ(x+1)/x);
/// Γ(r) is the lower incomplete γ(r, N) = N^r e^-N Σ N^k / (r(r+1)…(r+k)),
/// whose missing tail Γ(r, N) < N e^-N is under 2^-269 at N = 192.
fn gamma(x: &Big) -> Big {
    const N: i64 = 192;
    let one = Big::int(1);
    let (whole, fraction) = x.split();
    let (r, scale) = if whole.is_zero() {
        (x.add(&one), one.div(x))
    } else {
        let r = one.add(&fraction);
        let mut product = one.clone();
        let mut factor = r.clone();
        let steps = whole.to_u64().unwrap_or(u64::MAX) - 1;
        for _ in 0..steps {
            product = product.mul(&factor);
            factor = factor.add(&one);
        }
        (r, product)
    };
    let n = Big::int(N);
    let mut term = one.div(&r);
    let mut sum = term.clone();
    let mut denominator = r.clone();
    loop {
        denominator = denominator.add(&one);
        term = term.mul(&n).div(&denominator);
        if term.top() < sum.top() - WORK_BITS as i64 - 8 {
            break;
        }
        sum = sum.add(&term);
    }
    let lower = exp(&r.mul(&ln(&n)).sub(&n)).mul(&sum);
    lower.mul(&scale)
}

fn nan() -> LongDouble {
    LongDouble { negative: false, kind: Kind::NaN }
}

fn inf(negative: bool) -> LongDouble {
    LongDouble { negative, kind: Kind::Inf }
}

fn zero(negative: bool) -> LongDouble {
    LongDouble { negative, kind: Kind::Zero }
}

fn is_nan(x: LongDouble) -> bool {
    x.kind == Kind::NaN
}

/// `Some(odd)` for an integer, `None` for anything else.
fn integer_parity(x: LongDouble) -> Option<bool> {
    match x.kind {
        Kind::Zero => Some(false),
        Kind::Finite { mant, exp } if exp >= 0 => Some(exp == 0 && mant & 1 == 1),
        Kind::Finite { mant, exp } => {
            let shift = (-exp) as u32;
            if shift >= 128 || mant & ((1u128 << shift) - 1) != 0 {
                None
            } else {
                Some((mant >> shift) & 1 == 1)
            }
        }
        _ => None,
    }
}

impl LongDouble {
    pub fn is_nan(self) -> bool {
        is_nan(self)
    }

    pub fn is_sign_negative(self) -> bool {
        self.negative
    }

    pub fn is_finite(self) -> bool {
        matches!(self.kind, Kind::Zero | Kind::Finite { .. })
    }

    /// Only [`nexttowardl`] makes a subnormal; every other result flushes.
    pub fn is_subnormal_p(self, prec: u32) -> bool {
        matches!(self.kind, Kind::Finite { mant, exp }
            if exp + i64::from(128 - mant.leading_zeros()) - 1 < min_exp(prec))
    }

    pub fn is_normal_p(self, prec: u32) -> bool {
        matches!(self.kind, Kind::Finite { .. }) && !self.is_subnormal_p(prec)
    }
}

pub fn atan2l(y: LongDouble, x: LongDouble, prec: u32) -> LongDouble {
    use Kind::*;
    if is_nan(y) || is_nan(x) {
        return nan();
    }
    let signed = |v: Big| (if y.negative { v.neg() } else { v }).to_ld(prec);
    match (y.kind, x.kind) {
        (Zero, _) if x.negative => signed(pi()),
        (Zero, _) => zero(y.negative),
        (Inf, Inf) if x.negative => signed(pi().mul(&Big::int(3)).shl(-2)),
        (Inf, Inf) => signed(pi().shl(-2)),
        (Inf, _) | (_, Zero) => signed(pi().shl(-1)),
        (_, Inf) if x.negative => signed(pi()),
        (_, Inf) => zero(y.negative),
        _ => {
            let angle = atan(&Big::of(y).abs().div(&Big::of(x).abs()));
            signed(if x.negative { pi().sub(&angle) } else { angle })
        }
    }
}

pub fn powl(x: LongDouble, y: LongDouble, prec: u32) -> LongDouble {
    use Kind::*;
    let one = LongDouble::from_i128_p(1, prec);
    if y.is_zero() || x == one {
        return one;
    }
    if is_nan(x) || is_nan(y) {
        return nan();
    }
    let parity = integer_parity(y);
    let odd = parity == Some(true);
    match x.kind {
        Zero if y.negative => return inf(x.negative && odd),
        Zero => return zero(x.negative && odd),
        Inf if y.negative => return zero(x.negative && odd),
        Inf => return inf(x.negative && odd),
        _ => {}
    }
    if y.kind == Inf {
        let magnitude = Big::of(x).abs().cmp(&Big::int(1));
        return match magnitude {
            Ordering::Equal => one,
            Ordering::Less if y.negative => inf(false),
            Ordering::Less => zero(false),
            _ if y.negative => zero(false),
            _ => inf(false),
        };
    }
    if x.negative && parity.is_none() {
        return nan();
    }
    let negative = x.negative && odd;
    let (Finite { mant, exp }, Some(_)) = (x.kind, parity) else {
        return general_pow(x, y, negative, prec);
    };
    // An integral power is exact before its one rounding.
    let n = y.abs_i64().filter(|n| *n <= 1024);
    let Some(n) = n else {
        return general_pow(x, y, negative, prec);
    };
    let power = BigUint::from(mant).pow(n as u32);
    if y.negative {
        exact_quotient(prec, negative, BigUint::one(), &power, -exp * n)
    } else {
        round_to(prec, negative, power, exp * n)
    }
}

impl LongDouble {
    /// `|self|` for an integer that fits i64.
    fn abs_i64(self) -> Option<i64> {
        let Kind::Finite { mant, exp } = self.kind else { return None };
        if exp >= 0 {
            mant.checked_shl(exp as u32).filter(|_| exp < 64).and_then(|m| i64::try_from(m).ok())
        } else {
            i64::try_from(mant >> (-exp).min(127) as u32).ok()
        }
    }
}

/// x^y = e^(y ln|x|) for finite nonzero operands.
fn general_pow(x: LongDouble, y: LongDouble, negative: bool, prec: u32) -> LongDouble {
    let t = Big::of(y).mul(&ln(&Big::of(x).abs()));
    if t.is_zero() {
        return LongDouble::from_i128_p(if negative { -1 } else { 1 }, prec);
    }
    // Past 2^16 the result is far outside every long double's range.
    if t.top() >= 16 {
        return if t.negative { zero(negative) } else { inf(negative) };
    }
    let r = exp(&t);
    (if negative { r.neg() } else { r }).to_ld(prec)
}

pub fn exp2l(x: LongDouble, prec: u32) -> LongDouble {
    powl(LongDouble::from_i128_p(2, prec), x, prec)
}

/// Γ(x). Its largest finite argument is about 1755.5 in x87 and binary128.
pub fn tgammal(x: LongDouble, prec: u32) -> LongDouble {
    use Kind::*;
    const OVERFLOW: i64 = 1800;
    match x.kind {
        NaN => return nan(),
        Zero => return inf(x.negative),
        Inf if x.negative => return nan(),
        Inf => return x,
        Finite { .. } => {}
    }
    let past_range = Big::of(x).abs().cmp(&Big::int(OVERFLOW)) == Ordering::Greater;
    if integer_parity(x).is_some() {
        if x.negative {
            return nan();
        }
        if past_range {
            return inf(false);
        }
        // (n-1)!, exact before its one rounding.
        let n = x.abs_i64().expect("an integer under 1800");
        let factorial = (1..n).fold(BigUint::one(), |f, k| f * BigUint::from(k as u64));
        return round_to(prec, false, factorial, 0);
    }
    if !x.negative {
        return if past_range { inf(false) } else { gamma(&Big::of(x)).to_ld(prec) };
    }
    // Reflection: Γ(x) = π / (sin(πx) Γ(1-x)); with |x| = k + f,
    // sin(πx) = -(-1)^k sin(πf).
    let (whole, fraction) = Big::of(x).abs().split();
    let mut sine = sin_pi(&fraction);
    if !whole.bit(0) {
        sine = sine.neg();
    }
    if past_range {
        return zero(sine.negative);
    }
    let reflected = Big::int(1).sub(&Big::of(x));
    pi().div(&sine.mul(&gamma(&reflected))).to_ld(prec)
}

/// `x × y + z` with one rounding.
pub fn fmal(x: LongDouble, y: LongDouble, z: LongDouble, prec: u32) -> LongDouble {
    use Kind::*;
    if is_nan(x) || is_nan(y) || is_nan(z) {
        return nan();
    }
    let product_negative = x.negative != y.negative;
    if x.kind == Inf || y.kind == Inf {
        if x.kind == Zero || y.kind == Zero || (z.kind == Inf && z.negative != product_negative) {
            return nan();
        }
        return inf(product_negative);
    }
    if z.kind == Inf {
        return z;
    }
    let (Finite { mant: ma, exp: ea }, Finite { mant: mb, exp: eb }) = (x.kind, y.kind) else {
        // A zero product.
        return if z.kind == Zero { zero(product_negative && z.negative) } else { z };
    };
    let product = BigUint::from(ma) * BigUint::from(mb);
    match z.kind {
        Finite { mant, exp } => exact_sum(
            prec,
            (product_negative, product, ea + eb),
            (z.negative, BigUint::from(mant), exp),
        ),
        _ => round_to(prec, product_negative, product, ea + eb),
    }
}

/// The positive difference: `x - y` when x > y, else `+0` (NaN for a NaN).
pub fn fdiml(x: LongDouble, y: LongDouble, prec: u32) -> LongDouble {
    match x.partial_cmp(&y) {
        Some(Ordering::Less | Ordering::Equal) => zero(false),
        _ => x.add_p(y.neg(), prec),
    }
}

/// The smaller operand; a NaN loses to a number; a tie answers `x`.
pub fn fminl(x: LongDouble, y: LongDouble) -> LongDouble {
    match x.partial_cmp(&y) {
        Some(Ordering::Greater) => y,
        Some(_) => x,
        None if is_nan(x) => y,
        None => x,
    }
}

/// The larger operand; a NaN loses to a number. A tie (`-0` and `0`) answers
/// `y` on x87, whose `fcmovb` keeps y unless y < x, and `x` in glibc's
/// generic `isgreaterequal (x, y) ? x : …`.
pub fn fmaxl(x: LongDouble, y: LongDouble, prec: u32) -> LongDouble {
    match x.partial_cmp(&y) {
        Some(Ordering::Less) => y,
        Some(Ordering::Equal) if prec == 64 => y,
        Some(_) => x,
        None if is_nan(x) => y,
        None => x,
    }
}

/// How [`to_integral`] treats a fraction.
#[derive(Clone, Copy, PartialEq)]
pub enum Rounding {
    /// `truncl`: toward zero.
    Trunc,
    /// `roundl`: half away from zero.
    Round,
    /// `nearbyintl` in the default mode: half to even.
    NearestEven,
}

/// `truncl`, `roundl` and `nearbyintl`. A zero result keeps x's sign.
pub fn to_integral(x: LongDouble, mode: Rounding, prec: u32) -> LongDouble {
    let Kind::Finite { mant, exp } = x.kind else { return x };
    if exp >= 0 {
        return x;
    }
    // A significand of at most 113 bits shifted past 114 is under one half.
    let shift = (-exp) as u32;
    if shift > 114 {
        return zero(x.negative);
    }
    let mut whole = mant >> shift;
    let fraction = mant & ((1u128 << shift) - 1);
    let half = 1u128 << (shift - 1);
    let up = match mode {
        Rounding::Trunc => false,
        Rounding::Round => fraction >= half,
        Rounding::NearestEven => fraction > half || (fraction == half && whole & 1 == 1),
    };
    if up {
        whole += 1;
    }
    if whole == 0 {
        return zero(x.negative);
    }
    round_to(prec, x.negative, BigUint::from(whole), 0)
}

/// The next long double after x toward y; y when they are equal.
pub fn nexttowardl(x: LongDouble, y: LongDouble, prec: u32) -> LongDouble {
    use Kind::*;
    let Some(order) = x.partial_cmp(&y) else { return nan() };
    if order == Ordering::Equal {
        return y;
    }
    let top = 1u128 << (prec - 1);
    let (negative, mant, exp) = match x.kind {
        // The smallest subnormal, which only this function produces.
        Zero => return LongDouble { negative: y.negative, kind: Finite { mant: top, exp: min_exp(prec) - 2 * i64::from(prec - 1) } },
        Inf => (x.negative, (top << 1) - 1, max_exp(prec) - i64::from(prec)),
        Finite { mant, exp } => {
            let away_from_zero = (order == Ordering::Less) != x.negative;
            if away_from_zero {
                (x.negative, mant + 1, exp)
            } else if mant == top {
                (x.negative, (top << 1) - 1, exp - 1)
            } else {
                (x.negative, mant - 1, exp)
            }
        }
        NaN => return nan(),
    };
    round_to(prec, negative, BigUint::from(mant), exp)
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

    /// The x86_64 Linux answers of ksh93u+m 1.0.10 (`powl`, `atan2l`, … on
    /// x87) the emulation matrix records, printed with LDBL_DIG digits.
    #[test]
    fn math_functions_match_x87_ksh93() {
        let p = |t: &str| parse(t, X87);
        let g = |v: LongDouble| v.format_g(18);
        assert_eq!(g(powl(p("2"), p("0.5"), X87)), "1.41421356237309505");
        assert_eq!(g(powl(p("2"), p("10"), X87)), "1024");
        assert_eq!(g(powl(p("2"), p("-1"), X87)), "0.5");
        assert_eq!(g(powl(p("10"), p("20"), X87)), "1e+20");
        assert_eq!(g(powl(p("0"), p("0"), X87)), "1");
        assert_eq!(g(atan2l(p("1"), p("1"), X87)), "0.78539816339744831");
        assert_eq!(g(atan2l(p("-0.0"), p("-1"), X87)), "-3.14159265358979324");
        assert_eq!(g(atan2l(p("1"), p("0"), X87)), "1.57079632679489662");
        assert_eq!(g(fminl(p("-0.0"), p("0.0"))), "-0");
        assert_eq!(g(fdiml(p("3"), p("5"), X87)), "0");
        assert_eq!(g(fmal(p("0.1"), p("10"), p("-1"), X87)), "1.35525271560688054e-20");
        assert_eq!(g(exp2l(p("0.5"), X87)), "1.41421356237309505");
        assert_eq!(g(exp2l(p("3"), X87)), "8");
        assert_eq!(g(tgammal(p("5"), X87)), "24");
        assert_eq!(g(tgammal(p("0.5"), X87)), "1.77245385090551603");
        assert_eq!(g(tgammal(p("172"), X87)), "1.24101807021766782e+309");
        assert_eq!(g(nexttowardl(p("1"), p("2"), X87)), "1");
    }

    /// Values that only a correct long double computation gets: binary128
    /// digits of π/4, Γ and a fractional power, and the C99 special cases.
    #[test]
    fn math_functions_are_correct_past_double() {
        let q = |t: &str| parse(t, QUAD);
        assert_eq!(atan2l(q("1"), q("1"), QUAD).format_g(33), "0.785398163397448309615660845819876");
        // Γ(1/2) = √π and Γ(5/2) = 3√π/4, both by bc -l.
        assert_eq!(tgammal(q("0.5"), QUAD).format_g(30), "1.77245385090551602729816748334");
        assert_eq!(tgammal(q("2.5"), QUAD).format_g(30), "1.32934038817913702047362561251");
        // Γ(-1/2) = -2√π = -3.54490770181103205459633496668229036…
        assert_eq!(tgammal(q("-0.5"), QUAD).format_g(30), "-3.54490770181103205459633496668");
        assert_eq!(powl(q("2.5"), q("1.5"), QUAD).format_g(30), "3.95284707521047416499861693054");
        let x = |t: &str| parse(t, X87);
        assert!(powl(x("-8"), x("0.5"), X87).is_nan());
        assert_eq!(powl(x("-2"), x("3"), X87).format_g(18), "-8");
        assert_eq!(powl(x("-0.0"), x("-3"), X87).format_g(18), "-inf");
        assert_eq!(powl(x("0.5"), LongDouble { negative: true, kind: Kind::Inf }, X87).format_g(18), "inf");
        assert!(tgammal(x("-2"), X87).is_nan());
        assert_eq!(tgammal(x("-0.0"), X87).format_g(18), "-inf");
        assert_eq!(tgammal(x("1800.5"), X87).format_g(18), "inf");
        assert_eq!(fmaxl(x("-0.0"), x("0.0"), X87).format_g(18), "0");
        assert_eq!(fmaxl(x("-0.0"), x("0.0"), QUAD).format_g(18), "-0");
        assert_eq!(to_integral(x("-2.5"), Rounding::Round, X87).format_g(18), "-3");
        assert_eq!(to_integral(x("2.5"), Rounding::NearestEven, X87).format_g(18), "2");
        assert_eq!(to_integral(x("-0.3"), Rounding::Trunc, X87).format_g(18), "-0");
        assert_eq!(to_integral(x("12345678901234567.5"), Rounding::Round, X87).format_g(18), "12345678901234568");
        assert_eq!(nexttowardl(x("1"), x("0"), X87), round_to(X87, false, BigUint::from(u64::MAX), -64));
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
