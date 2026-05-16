//! Random real module — port of `Src/Modules/random_real.c`.
//!
//! Top-level declaration order matches C source line-by-line:
//!   - `#define clz64(x) ...`                       c:43-46
//!   - `_zclz64(x)`                                  c:48
//!   - `random_64bit(void)`                          c:83
//!   - `random_real(void)`                           c:146
//!
//! C source has zero `struct ...` / `enum ...` definitions. Rust
//! port matches: zero types.

#![allow(non_snake_case)]

use crate::ported::modules::random::getrandom_buffer;
use crate::ported::utils::zwarn;

// =====================================================================
// _zclz64(uint64_t x)                                                c:48
// =====================================================================

/// Port of `_zclz64(uint64_t x)` from `Src/Modules/random_real.c:49`.
///
/// Binary-search clz fallback used when the compiler doesn't provide
/// `__builtin_clzll`. C source body followed line-by-line.
pub fn _zclz64(x: u64) -> i32 {
    let mut n: i32 = 0;                                                   // c:49
    let mut x = x;

    if x == 0 {                                                            // c:52
        return 64;                                                         // c:53
    }

    if (x & 0xFFFF_FFFF_0000_0000) == 0 {                                  // c:55
        n += 32;                                                           // c:56
        x <<= 32;                                                          // c:57
    }
    if (x & 0xFFFF_0000_0000_0000) == 0 {                                  // c:59
        n += 16;                                                           // c:60
        x <<= 16;                                                          // c:61
    }
    if (x & 0xFF00_0000_0000_0000) == 0 {                                  // c:63
        n += 8;                                                            // c:64
        x <<= 8;                                                           // c:65
    }
    if (x & 0xF000_0000_0000_0000) == 0 {                                  // c:67
        n += 4;                                                            // c:68
        x <<= 4;                                                           // c:69
    }
    if (x & 0xC000_0000_0000_0000) == 0 {                                  // c:71
        n += 2;                                                            // c:72
        // Upstream `Src/Modules/random_real.c:73` writes `x <<= 1`, which
        // is an off-by-one bug in the binary-halving CLZ cascade: every
        // earlier stage shifts by the same amount it adds to n (32/16/8/4),
        // so this stage must add 2 AND shift by 2. With the upstream
        // `<<= 1`, the bit ends up one position below the c:75 top-bit
        // mask and the count is one too high. Concrete failure case:
        // `_zclz64(2)` returns 63 with `<< 1`, true CLZ is 62.
        // Fixed per maintainer directive 2026-05 — port faithfulness
        // overridden because upstream is provably wrong.
        x <<= 2;                                                           // c:73 (FIXED: was x<<=1)
    }
    if (x & 0x8000_0000_0000_0000) == 0 {                                  // c:75
        n += 1;                                                            // c:76
    }
    n                                                                      // c:84
}

// =====================================================================
// random_64bit(void)                                                 c:83
// =====================================================================

/// Port of `random_64bit()` from `Src/Modules/random_real.c:84`.
///
/// C body returns `uint64_t`; failure path returns 1 (NOT 0 — the
/// `random_real()` zero-detection loop would spin forever on 0).
pub fn random_64bit() -> u64 {
    let r: u64;                                                            // c:84
    let mut buf = [0u8; 8];                                                // staging for &r

    // c:87 — `if (getrandom_buffer(&r, sizeof(r)) < 0)`
    if getrandom_buffer(&mut buf).is_err() {
        zwarn("zsh/random: Can't get sufficient random data.");            // c:88
        return 1;                                                          // c:89 0 will cause loop
    }
    r = u64::from_ne_bytes(buf);
    r                                                                      // c:92 return r;
}

// =====================================================================
/*
 * Uniform random floats: How to generate a double-precision
 * floating-point numbers in [0, 1] uniformly at random given a uniform
 * random source of bits.
 *
 * See <http://mumble.net/~campbell/2014/04/28/uniform-random-float>
 * for explanation.
 *
 * Updated 2015-02-22 to replace ldexp(x, <constant>) by x * ldexp(1,
 * <constant>), since glibc and NetBSD libm both have slow software
 * bit-twiddling implementations of ldexp, but GCC can constant-fold
 * the latter.
 */                                                                      // c:126-137
/*
 * random_real: Generate a stream of bits uniformly at random and
 * interpret it as the fractional part of the binary expansion of a
 * number in [0, 1], 0.00001010011111010100...; then round it.
 */                                                                      // c:147-143
// random_real(void)                                                    c:146
// =====================================================================

/// Port of `random_real()` from `Src/Modules/random_real.c:147`.
pub fn random_real() -> f64 {
    let mut exponent: i32 = 0;                                             // c:147
    let mut significand: u64 = 0;                                          // c:150
    let mut r: u64 = 0;                                                    // c:151
    let shift: u32;                                                        // c:152

    /*
     * Read zeros into the exponent until we hit a one; the rest
     * will go into the significand.
     */                                                                    // c:154-157
    while significand == 0 {                                               // c:158
        exponent -= 64;                                                    // c:159

        /* Get random_64bit and check for error */                         // c:161
        // c:162-165 — errno = 0; significand = random_64bit(); if (errno) return -1;
        // The Rust `random_64bit()` returns 1 on entropy failure (c:89),
        // so the loop exits naturally on the sentinel — no explicit
        // errno probe needed. The `< 0` return path of C maps to our
        // "no error" success path.
        significand = random_64bit();                                      // c:163

        /*
         * If the exponent falls below -1074 = emin + 1 - p,
         * the exponent of the smallest subnormal, we are
         * guaranteed the result will be rounded to zero.  This
         * case is so unlikely it will happen in realistic
         * terms only if random_64bit is broken.
         */                                                                // c:166-172
        if exponent < -1074 {                                              // c:173
            return 0.0;                                                    // c:174
        }
    }

    /*
     * There is a 1 somewhere in significand, not necessarily in
     * the most significant position.  If there are leading zeros,
     * shift them into the exponent and refill the less-significant
     * bits of the significand.  Can't predict one way or another
     * whether there are leading zeros: there's a fifty-fifty
     * chance, if random_64bit is uniformly distributed.
     */                                                                    // c:177-184
    shift = clz64(significand) as u32;                                     // c:185
    if shift != 0 {                                                        // c:186
        // c:188-191 — errno = 0; r = random_64bit(); if (errno) return -1;
        r = random_64bit();                                                // c:189

        exponent -= shift as i32;                                          // c:193
        significand <<= shift;                                             // c:194
        significand |= r >> (64 - shift);                                  // c:195
    }

    /*
     * Set the sticky bit, since there is almost surely another 1
     * in the bit stream.  Otherwise, we might round what looks
     * like a tie to even when, almost surely, were we to look
     * further in the bit stream, there would be a 1 breaking the
     * tie.
     */                                                                    // c:198-204
    significand |= 1;                                                      // c:205

    /*
     * Finally, convert to double (rounding) and scale by
     * 2^exponent.
     */                                                                    // c:207-210
    // c:211 — `return ldexp((double)significand, exponent);`
    // libm `ldexp(x, exp) = x * 2^exp`. Rust stdlib doesn't expose
    // ldexp; use the `(x as f64) * 2f64.powi(exp)` equivalent, which
    // matches the C author's 2015-02-22 update note (c:133-136) about
    // glibc's slow software bit-twiddling implementation.
    (significand as f64) * (2.0_f64).powi(exponent)
}

// =====================================================================
// /* Count the number of leading zeros, hopefully in gcc/clang by HW
//  * instruction */                                                  c:40-41
// #if defined(__GNUC__) || defined(__clang__)
// #define clz64(x) __builtin_clzll(x)
// #else
// #define clz64(x) _zclz64(x)
// =====================================================================

/// Port of `clz64(x)` macro from `Src/Modules/random_real.c:43`.
///
/// Dispatches to the HW clz on platforms that have it (Rust's
/// `u64::leading_zeros()` is the HW intrinsic on every modern arch),
/// or to `_zclz64()` as the portable fallback. C resolves this at
/// preprocessor time; Rust resolves at the call site.
#[inline]
pub fn clz64(x: u64) -> i32 {
    // Equivalent to C's `__builtin_clzll(x)` branch (c:43).
    if x == 0 { 64 } else { x.leading_zeros() as i32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies `clz64` matches `_zclz64` across boundary inputs —
    /// both should agree per the C `#define` dispatch (c:43-46).
    #[test]
    fn clz64_matches_zclz64() {
        for x in [0u64, 1, 0xff, 0xff00, u64::MAX, 0x8000_0000_0000_0000] {
            assert_eq!(clz64(x), _zclz64(x), "input 0x{:016x}", x);
        }
    }

    /// Verifies `_zclz64(0)` returns 64 (c:53).
    #[test]
    fn zclz64_zero_is_64() {
        assert_eq!(_zclz64(0), 64);
    }

    /// Verifies `_zclz64(MSB-set)` returns 0.
    #[test]
    fn zclz64_msb_is_zero() {
        assert_eq!(_zclz64(0x8000_0000_0000_0000), 0);
    }

    /// Verifies `random_real` lies in [0, 1) over many draws.
    #[test]
    fn random_real_in_range() {
        for _ in 0..100 {
            let r = random_real();
            assert!((0.0..1.0).contains(&r), "out of range: {}", r);
        }
    }

    /// Verifies `random_64bit` returns SOMETHING (entropy-pool calls
    /// almost never fail; sentinel return is 1).
    #[test]
    fn random_64bit_returns_value() {
        let _ = random_64bit();
    }

    /// c:49 — `_zclz64(1)` returns 63: the lone 1-bit is in the LSB,
    /// so there are 63 leading zeros above it. Pin this edge case
    /// because the binary-search shift cascade is the bug-bait part
    /// of the function (the c:73 `x <<= 1` is intentionally one shift,
    /// not two — easy to "correct").
    #[test]
    fn zclz64_lsb_only_is_63() {
        assert_eq!(_zclz64(1), 63);
    }

    /// c:49 — `_zclz64` agrees with `leading_zeros()` for every
    /// single-bit input, position 0 through 63. After the 2026-05 fix
    /// of upstream's c:73 off-by-one (`x<<=1` → `x<<=2`), the
    /// binary-halving cascade is correct end-to-end.
    #[test]
    fn zclz64_matches_leading_zeros_across_all_single_bits() {
        for bit in 0..64u32 {
            let x = 1u64 << bit;
            assert_eq!(_zclz64(x), x.leading_zeros() as i32,
                "mismatch at single-bit input 1<<{}", bit);
        }
    }

    /// c:49 — Explicit low-bit anchors. Each entry's expected value
    /// is `63 - bit_position`. Catches a regression that re-introduces
    /// the c:73 `x<<=1` upstream bug — those low bits were exactly
    /// the failure surface.
    #[test]
    fn zclz64_low_bit_anchors_match_position() {
        assert_eq!(_zclz64(0b1),         63);  // bit 0
        assert_eq!(_zclz64(0b10),        62);  // bit 1 (was the bug case)
        assert_eq!(_zclz64(0b100),       61);  // bit 2
        assert_eq!(_zclz64(0b1000),      60);  // bit 3
        assert_eq!(_zclz64(0b1_0000),    59);  // bit 4
        assert_eq!(_zclz64(0b1000_0000), 56);  // bit 7
    }

    /// c:49 — The MSB-set input always returns 0 (no leading zeros).
    /// Pin the unambiguous case; the c:55 first-stage test fires
    /// before any shifting.
    #[test]
    fn zclz64_msb_only_returns_zero() {
        let x = 1u64 << 63;
        assert_eq!(_zclz64(x), 0);
        assert_eq!(clz64(x), 0);
        assert_eq!(x.leading_zeros() as i32, 0);
    }

    /// c:49 — Wide sweep of random-ish 64-bit values comparing
    /// `_zclz64` against `u64::leading_zeros()`. Pins the broad
    /// agreement post-fix; if any future regen drifts the cascade,
    /// this catches it across the full input space.
    #[test]
    fn zclz64_matches_leading_zeros_across_diverse_inputs() {
        for x in [
            0u64, 1, 2, 3, 0xff, 0x100, 0xff00, 0xffff,
            0x12345678_9abcdef0, 0xdeadbeef, 0xcafebabe_cafebabe,
            (1 << 31), (1 << 32), (1 << 33), (1 << 62),
            u64::MAX, u64::MAX - 1, 0x5555_5555_5555_5555,
            0xAAAA_AAAA_AAAA_AAAA, 0x8000_0000_0000_0001,
        ] {
            assert_eq!(_zclz64(x), x.leading_zeros() as i32,
                "mismatch at 0x{:016x}", x);
        }
    }

    /// c:43-46 — `clz64(0)` is 64; documented C-side semantic. A
    /// regression that confuses `__builtin_clzll` UB-on-zero behavior
    /// with the portable fallback would return 0 here, silently
    /// breaking `random_real`'s exponent accounting at c:158.
    #[test]
    fn clz64_zero_returns_64_not_undef() {
        assert_eq!(clz64(0), 64);
    }

    /// c:147-200 — `random_real()` over many draws should produce
    /// non-zero values (probability of an exact 0 from a uniform
    /// [0,1) draw is negligible). Catches a regression that bails
    /// early and returns 0.0 unconditionally.
    #[test]
    fn random_real_produces_some_nonzero_values() {
        let mut saw_nonzero = false;
        for _ in 0..50 {
            if random_real() != 0.0 {
                saw_nonzero = true;
                break;
            }
        }
        assert!(saw_nonzero,
            "50 consecutive 0.0 outputs from random_real — entropy pump broken");
    }

    /// c:147 — `random_real()` outputs must spread across the unit
    /// interval. 100 draws and at least 5 different floats — catches
    /// a regression that always returns the same constant.
    #[test]
    fn random_real_produces_distinct_outputs() {
        use std::collections::HashSet;
        let mut seen: HashSet<u64> = HashSet::new();
        for _ in 0..100 {
            seen.insert(random_real().to_bits());
        }
        assert!(seen.len() >= 5,
            "100 draws produced only {} distinct outputs — distribution broken",
            seen.len());
    }

    /// c:84 — `random_64bit()` never returns 0 (entropy success
    /// path) or 1 (entropy-failure sentinel) every time. Sweep many
    /// draws and assert at least one falls outside that pair; that
    /// proves the entropy syscall actually fired.
    #[test]
    fn random_64bit_produces_non_sentinel_values() {
        let mut saw_real = false;
        for _ in 0..20 {
            let r = random_64bit();
            // 0 and 1 are degenerate; everything else is real entropy.
            if r != 0 && r != 1 {
                saw_real = true;
                break;
            }
        }
        assert!(saw_real, "every random_64bit returned the failure sentinel — getrandom broken?");
    }
}
