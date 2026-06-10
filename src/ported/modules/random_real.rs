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
    let mut n: i32 = 0; // c:49
    let mut x = x;

    if x == 0 {
        // c:52
        return 64; // c:53
    }

    if (x & 0xFFFF_FFFF_0000_0000) == 0 {
        // c:55
        n += 32; // c:56
        x <<= 32; // c:57
    }
    if (x & 0xFFFF_0000_0000_0000) == 0 {
        // c:59
        n += 16; // c:60
        x <<= 16; // c:61
    }
    if (x & 0xFF00_0000_0000_0000) == 0 {
        // c:63
        n += 8; // c:64
        x <<= 8; // c:65
    }
    if (x & 0xF000_0000_0000_0000) == 0 {
        // c:67
        n += 4; // c:68
        x <<= 4; // c:69
    }
    if (x & 0xC000_0000_0000_0000) == 0 {
        // c:71
        n += 2; // c:72
                // Upstream `Src/Modules/random_real.c:73` writes `x <<= 1`, which
                // is an off-by-one bug in the binary-halving CLZ cascade: every
                // earlier stage shifts by the same amount it adds to n (32/16/8/4),
                // so this stage must add 2 AND shift by 2. With the upstream
                // `<<= 1`, the bit ends up one position below the c:75 top-bit
                // mask and the count is one too high. Concrete failure case:
                // `_zclz64(2)` returns 63 with `<< 1`, true CLZ is 62.
                // Fixed per maintainer directive 2026-05 — port faithfulness
                // overridden because upstream is provably wrong.
        x <<= 2; // c:73 (FIXED: was x<<=1)
    }
    if (x & 0x8000_0000_0000_0000) == 0 {
        // c:75
        n += 1; // c:76
    }
    n // c:84
}

// =====================================================================
// random_64bit(void)                                                 c:83
// =====================================================================

// c:84 — `if (errno)` in random_real() expects random_64bit to set
// errno on failure. Rust doesn't carry errno through Result types, so
// mirror via this thread-local flag (per-thread like C's errno).
// random_64bit clears it on success, sets it on failure. random_real
// clears at entry and checks after each call.
thread_local! {
    pub(super) static RANDOM_ENTROPY_FAILED: std::cell::Cell<bool> =
        const { std::cell::Cell::new(false) };
}

/// Port of `random_64bit()` from `Src/Modules/random_real.c:84`.
///
/// C body returns `uint64_t`; failure path returns 1 (NOT 0 — the
/// `random_real()` zero-detection loop would spin forever on 0) AND
/// sets errno so the random_real caller can disambiguate the
/// "actually got 1" case from the "entropy failed and we returned the
/// non-loop-spinning sentinel" case.
pub fn random_64bit() -> u64 {
    let r: u64; // c:84
    let mut buf = [0u8; 8]; // staging for &r

    // c:87 — `if (getrandom_buffer(&r, sizeof(r)) < 0)`
    if getrandom_buffer(&mut buf).is_err() {
        zwarn("zsh/random: Can't get sufficient random data."); // c:88
        RANDOM_ENTROPY_FAILED.with(|f| f.set(true)); // c:88-89 errno flag
        return 1; // c:89 0 will cause loop
    }
    RANDOM_ENTROPY_FAILED.with(|f| f.set(false)); // c:92 errno = 0 implicit
    r = u64::from_ne_bytes(buf);
    r // c:92 return r;
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
    let mut exponent: i32 = 0; // c:147
    let mut significand: u64 = 0; // c:150
    let mut r: u64 = 0; // c:151
    let shift: u32; // c:152

    /*
     * Read zeros into the exponent until we hit a one; the rest
     * will go into the significand.
     */
    // c:154-157
    while significand == 0 {
        // c:158
        exponent -= 64; // c:159

        /* Get random_64bit and check for error */
        // c:161
        // c:162-165 — `errno = 0; significand = random_64bit(); if (errno) return -1;`
        RANDOM_ENTROPY_FAILED.with(|f| f.set(false)); // c:162 errno = 0
        significand = random_64bit(); // c:163
        if RANDOM_ENTROPY_FAILED.with(|f| f.get()) {
            // c:164 — entropy syscall failed; surface via -1 sentinel
            // matching C exactly. Prior port silently swallowed the
            // failure: when random_64bit returned the 1-sentinel (the
            // "got data, continue" signal to break the inner loop),
            // random_real proceeded into the post-loop ldexp path and
            // emitted a tiny-but-plausible positive double, so any
            // caller computing ((rand48())) would think it had a real
            // sample even when the kernel CSPRNG was wedged.
            return -1.0; // c:165
        }

        /*
         * If the exponent falls below -1074 = emin + 1 - p,
         * the exponent of the smallest subnormal, we are
         * guaranteed the result will be rounded to zero.  This
         * case is so unlikely it will happen in realistic
         * terms only if random_64bit is broken.
         */
        // c:166-172
        if exponent < -1074 {
            // c:173
            return 0.0; // c:174
        }
    }

    /*
     * There is a 1 somewhere in significand, not necessarily in
     * the most significant position.  If there are leading zeros,
     * shift them into the exponent and refill the less-significant
     * bits of the significand.  Can't predict one way or another
     * whether there are leading zeros: there's a fifty-fifty
     * chance, if random_64bit is uniformly distributed.
     */
    // c:177-184
    shift = clz64(significand) as u32; // c:185
    if shift != 0 {
        // c:186
        // c:188-191 — `errno = 0; r = random_64bit(); if (errno) return -1;`
        RANDOM_ENTROPY_FAILED.with(|f| f.set(false)); // c:188 errno = 0
        r = random_64bit(); // c:189
        if RANDOM_ENTROPY_FAILED.with(|f| f.get()) {
            return -1.0; // c:191
        }

        exponent -= shift as i32; // c:193
        significand <<= shift; // c:194
        significand |= r >> (64 - shift); // c:195
    }

    /*
     * Set the sticky bit, since there is almost surely another 1
     * in the bit stream.  Otherwise, we might round what looks
     * like a tie to even when, almost surely, were we to look
     * further in the bit stream, there would be a 1 breaking the
     * tie.
     */
    // c:198-204
    significand |= 1; // c:205

    /*
     * Finally, convert to double (rounding) and scale by
     * 2^exponent.
     */
    // c:207-210
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
    if x == 0 {
        64
    } else {
        x.leading_zeros() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// Verifies `clz64` matches `_zclz64` across boundary inputs —
    /// both should agree per the C `#define` dispatch (c:43-46).
    #[test]
    fn clz64_matches_zclz64() {
        let _g = crate::test_util::global_state_lock();
        for x in [0u64, 1, 0xff, 0xff00, u64::MAX, 0x8000_0000_0000_0000] {
            assert_eq!(clz64(x), _zclz64(x), "input 0x{:016x}", x);
        }
    }

    /// Verifies `_zclz64(0)` returns 64 (c:53).
    #[test]
    fn zclz64_zero_is_64() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_zclz64(0), 64);
    }

    /// Verifies `_zclz64(MSB-set)` returns 0.
    #[test]
    fn zclz64_msb_is_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_zclz64(0x8000_0000_0000_0000), 0);
    }

    /// Verifies `random_real` lies in [0, 1) over many draws.
    #[test]
    fn random_real_in_range() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let r = random_real();
            assert!((0.0..1.0).contains(&r), "out of range: {}", r);
        }
    }

    /// Verifies `random_64bit` returns SOMETHING (entropy-pool calls
    /// almost never fail; sentinel return is 1).
    #[test]
    fn random_64bit_returns_value() {
        let _g = crate::test_util::global_state_lock();
        let _ = random_64bit();
    }

    /// c:49 — `_zclz64(1)` returns 63: the lone 1-bit is in the LSB,
    /// so there are 63 leading zeros above it. Pin this edge case
    /// because the binary-search shift cascade is the bug-bait part
    /// of the function (the c:73 `x <<= 1` is intentionally one shift,
    /// not two — easy to "correct").
    #[test]
    fn zclz64_lsb_only_is_63() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_zclz64(1), 63);
    }

    /// c:49 — `_zclz64` agrees with `leading_zeros()` for every
    /// single-bit input, position 0 through 63. After the 2026-05 fix
    /// of upstream's c:73 off-by-one (`x<<=1` → `x<<=2`), the
    /// binary-halving cascade is correct end-to-end.
    #[test]
    fn zclz64_matches_leading_zeros_across_all_single_bits() {
        let _g = crate::test_util::global_state_lock();
        for bit in 0..64u32 {
            let x = 1u64 << bit;
            assert_eq!(
                _zclz64(x),
                x.leading_zeros() as i32,
                "mismatch at single-bit input 1<<{}",
                bit
            );
        }
    }

    /// c:49 — Explicit low-bit anchors. Each entry's expected value
    /// is `63 - bit_position`. Catches a regression that re-introduces
    /// the c:73 `x<<=1` upstream bug — those low bits were exactly
    /// the failure surface.
    #[test]
    fn zclz64_low_bit_anchors_match_position() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_zclz64(0b1), 63); // bit 0
        assert_eq!(_zclz64(0b10), 62); // bit 1 (was the bug case)
        assert_eq!(_zclz64(0b100), 61); // bit 2
        assert_eq!(_zclz64(0b1000), 60); // bit 3
        assert_eq!(_zclz64(0b1_0000), 59); // bit 4
        assert_eq!(_zclz64(0b1000_0000), 56); // bit 7
    }

    /// c:49 — The MSB-set input always returns 0 (no leading zeros).
    /// Pin the unambiguous case; the c:55 first-stage test fires
    /// before any shifting.
    #[test]
    fn zclz64_msb_only_returns_zero() {
        let _g = crate::test_util::global_state_lock();
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
        let _g = crate::test_util::global_state_lock();
        for x in [
            0u64,
            1,
            2,
            3,
            0xff,
            0x100,
            0xff00,
            0xffff,
            0x12345678_9abcdef0,
            0xdeadbeef,
            0xcafebabe_cafebabe,
            (1 << 31),
            (1 << 32),
            (1 << 33),
            (1 << 62),
            u64::MAX,
            u64::MAX - 1,
            0x5555_5555_5555_5555,
            0xAAAA_AAAA_AAAA_AAAA,
            0x8000_0000_0000_0001,
        ] {
            assert_eq!(
                _zclz64(x),
                x.leading_zeros() as i32,
                "mismatch at 0x{:016x}",
                x
            );
        }
    }

    /// c:43-46 — `clz64(0)` is 64; documented C-side semantic. A
    /// regression that confuses `__builtin_clzll` UB-on-zero behavior
    /// with the portable fallback would return 0 here, silently
    /// breaking `random_real`'s exponent accounting at c:158.
    #[test]
    fn clz64_zero_returns_64_not_undef() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(clz64(0), 64);
    }

    /// c:147-200 — `random_real()` over many draws should produce
    /// non-zero values (probability of an exact 0 from a uniform
    /// [0,1) draw is negligible). Catches a regression that bails
    /// early and returns 0.0 unconditionally.
    #[test]
    fn random_real_produces_some_nonzero_values() {
        let _g = crate::test_util::global_state_lock();
        let mut saw_nonzero = false;
        for _ in 0..50 {
            if random_real() != 0.0 {
                saw_nonzero = true;
                break;
            }
        }
        assert!(
            saw_nonzero,
            "50 consecutive 0.0 outputs from random_real — entropy pump broken"
        );
    }

    /// c:147 — `random_real()` outputs must spread across the unit
    /// interval. 100 draws and at least 5 different floats — catches
    /// a regression that always returns the same constant.
    #[test]
    fn random_real_produces_distinct_outputs() {
        let _g = crate::test_util::global_state_lock();
        let mut seen: HashSet<u64> = HashSet::new();
        for _ in 0..100 {
            seen.insert(random_real().to_bits());
        }
        assert!(
            seen.len() >= 5,
            "100 draws produced only {} distinct outputs — distribution broken",
            seen.len()
        );
    }

    /// c:84 — `random_64bit()` never returns 0 (entropy success
    /// path) or 1 (entropy-failure sentinel) every time. Sweep many
    /// draws and assert at least one falls outside that pair; that
    /// proves the entropy syscall actually fired.
    #[test]
    fn random_64bit_produces_non_sentinel_values() {
        let _g = crate::test_util::global_state_lock();
        let mut saw_real = false;
        for _ in 0..20 {
            let r = random_64bit();
            // 0 and 1 are degenerate; everything else is real entropy.
            if r != 0 && r != 1 {
                saw_real = true;
                break;
            }
        }
        assert!(
            saw_real,
            "every random_64bit returned the failure sentinel — getrandom broken?"
        );
    }

    // ─── zsh-corpus pins for random_real helpers ────────────────────

    /// `clz64(0)` returns 64 (all bits zero).
    #[test]
    fn random_real_corpus_clz64_zero_is_64() {
        assert_eq!(clz64(0), 64);
        assert_eq!(_zclz64(0), 64);
    }

    /// `clz64(1)` returns 63 (only the lowest bit set).
    #[test]
    fn random_real_corpus_clz64_one_is_63() {
        assert_eq!(clz64(1), 63);
    }

    /// `clz64(0x8000_0000_0000_0000)` returns 0 (high bit set).
    #[test]
    fn random_real_corpus_clz64_high_bit_is_zero() {
        assert_eq!(clz64(0x8000_0000_0000_0000), 0);
    }

    /// `clz64` is monotonic in the inverse: larger value, smaller clz.
    #[test]
    fn random_real_corpus_clz64_monotonic_with_msb() {
        // 0xff = 8 bits. clz = 64 - 8 = 56.
        assert_eq!(clz64(0xff), 56);
        // 0xffff = 16 bits. clz = 64 - 16 = 48.
        assert_eq!(clz64(0xffff), 48);
        // 0xff_ffff = 24 bits. clz = 40.
        assert_eq!(clz64(0xff_ffff), 40);
    }

    /// `random_real()` is in [0.0, 1.0).
    #[test]
    fn random_real_corpus_random_real_unit_interval() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..20 {
            let v = random_real();
            assert!((0.0..1.0).contains(&v), "random_real out of [0,1): {v}");
        }
    }

    /// `random_real` produces different values across calls.
    #[test]
    fn random_real_corpus_random_real_varies() {
        let _g = crate::test_util::global_state_lock();
        let a = random_real();
        let b = random_real();
        let c = random_real();
        assert!(
            a != b || b != c || a != c,
            "3 random_real calls all same: {a} {b} {c}"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random_real.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:52 — `_zclz64(0)` returns 64 (early-return guard).
    #[test]
    fn zclz64_zero_returns_64() {
        assert_eq!(_zclz64(0), 64, "all-zero word → CLZ=64 per c:52");
    }

    /// c:75 — `_zclz64(1)` (only bit 0 set) returns 63.
    #[test]
    fn zclz64_one_returns_63() {
        assert_eq!(_zclz64(1), 63, "bit 0 only → CLZ=63");
    }

    /// c:75 — `_zclz64(2)` (only bit 1 set) returns 62.
    /// Pin: this is the SPECIFIC case that surfaced the upstream
    /// c:73 bug (was `x <<= 1`, fixed to `x <<= 2` in zshrs port).
    #[test]
    fn zclz64_two_returns_62_per_fixed_cascade() {
        assert_eq!(
            _zclz64(2),
            62,
            "bit 1 only → CLZ=62 (regression pin for c:73 upstream bug)"
        );
    }

    /// c:75 — `_zclz64(0x8000_0000_0000_0000)` (only top bit set) → 0.
    #[test]
    fn zclz64_top_bit_only_returns_zero() {
        assert_eq!(_zclz64(0x8000_0000_0000_0000), 0, "top bit set → CLZ=0");
    }

    /// c:75 — `_zclz64(0xFFFF_FFFF_FFFF_FFFF)` → 0 (all bits set).
    #[test]
    fn zclz64_all_bits_set_returns_zero() {
        assert_eq!(_zclz64(u64::MAX), 0);
    }

    /// c:75 — `_zclz64` matches `u64::leading_zeros()` for sample values.
    /// Pin: Rust port's manual cascade and stdlib must agree.
    #[test]
    fn zclz64_matches_stdlib_leading_zeros() {
        for v in [
            0u64,
            1,
            2,
            3,
            4,
            7,
            8,
            15,
            16,
            255,
            256,
            0xFFFF,
            0x1_0000,
            u64::MAX,
            u64::MAX / 2,
            0x8000_0000_0000_0000,
        ] {
            assert_eq!(
                _zclz64(v) as u32,
                v.leading_zeros(),
                "_zclz64({}) must match v.leading_zeros()",
                v
            );
        }
    }

    /// `clz64` is the public alias for `_zclz64` — same contract.
    #[test]
    fn clz64_alias_matches_zclz64() {
        for v in [0u64, 1, 42, u64::MAX] {
            assert_eq!(clz64(v), _zclz64(v));
        }
    }

    /// c:84 — `random_64bit()` returns non-zero (never returns 0 to
    /// avoid infinite loop in random_real's zero-detection).
    #[test]
    fn random_64bit_never_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let r = random_64bit();
            assert_ne!(r, 0, "random_64bit must never return 0 per c:89");
        }
    }

    /// c:84 — `random_64bit` produces varied output (basic PRNG sanity).
    #[test]
    fn random_64bit_produces_varied_output() {
        let _g = crate::test_util::global_state_lock();
        let first = random_64bit();
        let any_different = (0..100).any(|_| random_64bit() != first);
        assert!(
            any_different,
            "100 calls should produce ≥ 1 different value"
        );
    }

    /// c:119 — `random_real` upper bound: never returns 1.0 exactly.
    #[test]
    fn random_real_never_returns_one() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let v = random_real();
            assert!(v < 1.0, "random_real must be < 1.0, got {}", v);
        }
    }

    /// c:119 — `random_real` lower bound: returns ≥ 0.0.
    #[test]
    fn random_real_never_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let v = random_real();
            assert!(v >= 0.0, "random_real must be ≥ 0.0, got {}", v);
        }
    }

    /// c:119 — `random_real` is non-NaN.
    #[test]
    fn random_real_is_not_nan() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let v = random_real();
            assert!(!v.is_nan(), "random_real must not be NaN");
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random_real.c
    // c:25 _zclz64 / c:83 random_64bit / c:119 random_real / c:216 clz64
    // ═══════════════════════════════════════════════════════════════════

    /// c:25 — `_zclz64` returns i32 (compile-time type pin).
    #[test]
    fn zclz64_returns_i32_type() {
        let _: i32 = _zclz64(0);
        let _: i32 = _zclz64(1);
        let _: i32 = _zclz64(u64::MAX);
    }

    /// c:25 — `_zclz64` result in [0, 64] for all inputs.
    #[test]
    fn zclz64_result_bounded_zero_to_64() {
        for &x in &[
            0u64,
            1,
            2,
            0xff,
            0xffff_ffff,
            u64::MAX,
            0x8000_0000_0000_0000,
        ] {
            let r = _zclz64(x);
            assert!(
                (0..=64).contains(&r),
                "_zclz64({:#x}) = {} must be in [0, 64]",
                x,
                r
            );
        }
    }

    /// c:25 — `_zclz64` is pure (deterministic across calls).
    #[test]
    fn zclz64_is_pure() {
        for x in [0u64, 1, 42, u64::MAX] {
            let first = _zclz64(x);
            for _ in 0..5 {
                assert_eq!(_zclz64(x), first, "_zclz64({:#x}) must be pure", x);
            }
        }
    }

    /// c:216 — `clz64` alias matches `_zclz64` for all inputs.
    #[test]
    fn clz64_alias_full_sweep() {
        for x in [0u64, 1, 2, 0xff, 0x8000_0000_0000_0000, u64::MAX] {
            assert_eq!(clz64(x), _zclz64(x), "clz64({:#x}) must match _zclz64", x);
        }
    }

    /// c:83 — `random_64bit` returns u64 (compile-time pin).
    #[test]
    fn random_64bit_returns_u64_type() {
        let _: u64 = random_64bit();
    }

    /// c:119 — `random_real` returns f64.
    #[test]
    fn random_real_returns_f64_type() {
        let _: f64 = random_real();
    }

    /// c:119 — `random_real` is non-infinite.
    #[test]
    fn random_real_is_finite() {
        for _ in 0..100 {
            let v = random_real();
            assert!(v.is_finite(), "random_real must be finite, got {}", v);
        }
    }

    /// c:25 — `_zclz64(0x4000...)` returns 1 (second-highest bit only).
    #[test]
    fn zclz64_second_highest_bit_returns_one() {
        assert_eq!(_zclz64(0x4000_0000_0000_0000), 1);
    }

    /// c:25 — every single-bit input pinned: bit N → 63-N leading zeros.
    /// Pin a few key positions across the spectrum.
    #[test]
    fn zclz64_canonical_bit_positions() {
        assert_eq!(_zclz64(1u64 << 0), 63, "bit 0 → 63 leading zeros");
        assert_eq!(_zclz64(1u64 << 7), 56, "bit 7 → 56 leading zeros");
        assert_eq!(_zclz64(1u64 << 31), 32, "bit 31 → 32 leading zeros");
        assert_eq!(_zclz64(1u64 << 63), 0, "bit 63 → 0 leading zeros");
    }

    /// c:119 — `random_real` distribution: across 1000 samples, ≥ 10%
    /// should be > 0.1 (weak uniformity check).
    #[test]
    fn random_real_distribution_has_values_above_one_tenth() {
        let _g = crate::test_util::global_state_lock();
        let n = 1000;
        let above = (0..n).filter(|_| random_real() > 0.1).count();
        assert!(
            above >= n / 10,
            "{} out of {} should be > 0.1, got {}",
            n / 10,
            n,
            above
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random_real.c
    // c:25 _zclz64 / c:83 random_64bit / c:119 random_real / c:216 clz64
    // ═══════════════════════════════════════════════════════════════════

    /// c:25 — `_zclz64(0)` returns 64 (all bits zero).
    #[test]
    fn zclz64_zero_returns_64_pin() {
        assert_eq!(_zclz64(0), 64, "all zeros → 64 leading zeros");
    }

    /// c:25 — `_zclz64(u64::MAX)` returns 0 (no leading zeros).
    #[test]
    fn zclz64_max_returns_zero_pin() {
        assert_eq!(_zclz64(u64::MAX), 0, "u64::MAX has 0 leading zeros");
    }

    /// c:25 — `_zclz64` matches stdlib `leading_zeros` for diverse inputs.
    #[test]
    fn zclz64_matches_stdlib_leading_zeros_pin() {
        for x in [0u64, 1, 0xff, 0x10000, 0x80000000, u64::MAX, 42] {
            let our = _zclz64(x);
            let std = x.leading_zeros() as i32;
            assert_eq!(
                our, std,
                "_zclz64({:#x}) = {} but stdlib leading_zeros = {}",
                x, our, std
            );
        }
    }

    /// c:83 — `random_64bit` is non-zero (vanishingly rare to hit 0).
    /// Across 100 calls, at least one must be non-zero.
    #[test]
    fn random_64bit_has_at_least_one_nonzero_in_100() {
        let any_nonzero = (0..100).any(|_| random_64bit() != 0);
        assert!(any_nonzero, "100 random u64s must contain ≥ 1 non-zero");
    }

    /// c:119 — `random_real` never returns exactly 1.0 (half-open [0,1)).
    #[test]
    fn random_real_never_equals_one_in_1000_samples() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..1000 {
            assert!(random_real() < 1.0, "random_real must be < 1.0");
        }
    }

    /// c:119 — `random_real` never negative.
    #[test]
    fn random_real_never_negative_in_1000_samples() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..1000 {
            assert!(random_real() >= 0.0, "random_real must be ≥ 0");
        }
    }

    /// c:25 — `_zclz64` is pure (deterministic across many calls).
    #[test]
    fn zclz64_full_sweep_pure() {
        for x in [0u64, 1, 2, 7, 0xff, 0x10000, 0x80000000, u64::MAX] {
            let first = _zclz64(x);
            for _ in 0..5 {
                assert_eq!(_zclz64(x), first, "_zclz64({:#x}) must be pure", x);
            }
        }
    }

    /// c:216 — `clz64` is just an alias — must agree with `_zclz64` always.
    #[test]
    fn clz64_full_sweep_agrees_with_zclz64() {
        for x in 0u32..256u32 {
            let v = x as u64;
            assert_eq!(clz64(v), _zclz64(v), "clz64({:#x}) must match _zclz64", v);
        }
    }

    /// c:83 — `random_64bit` returns u64 type (compile-time pin).
    #[test]
    fn random_64bit_returns_u64_type_pin() {
        let _g = crate::test_util::global_state_lock();
        let _: u64 = random_64bit();
    }

    /// c:119 — `random_real` distribution: across 500 samples, mean ~ 0.5.
    #[test]
    fn random_real_mean_is_approximately_half() {
        let _g = crate::test_util::global_state_lock();
        let n = 500;
        let sum: f64 = (0..n).map(|_| random_real()).sum();
        let mean = sum / n as f64;
        // Weak check: mean must be in (0.3, 0.7) — extremely loose
        // statistical bound to avoid flake.
        assert!(
            mean > 0.3 && mean < 0.7,
            "random_real mean across {} samples = {} should be in (0.3, 0.7)",
            n,
            mean
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random_real.c
    // c:25 _zclz64 / c:83 random_64bit / c:119 random_real / c:216 clz64
    // ═══════════════════════════════════════════════════════════════════

    /// c:25 — `_zclz64` returns i32 (alt-name compile-time pin).
    #[test]
    fn zclz64_returns_i32_pin_alt() {
        let _: i32 = _zclz64(0);
    }

    /// c:25 — `_zclz64(0)` returns 64 (alt-name pin).
    #[test]
    fn zclz64_zero_returns_64_alt() {
        assert_eq!(
            _zclz64(0),
            64,
            "clz64(0) MUST equal 64 (all 64 bits are leading zeros)"
        );
    }

    /// c:25 — `_zclz64(u64::MAX)` returns 0 (no leading zeros).
    #[test]
    fn zclz64_max_returns_zero() {
        assert_eq!(
            _zclz64(u64::MAX),
            0,
            "clz64(u64::MAX) MUST equal 0 (no leading zeros)"
        );
    }

    /// c:25 — `_zclz64(1)` returns 63 (only the LSB set; alt-name).
    #[test]
    fn zclz64_one_returns_63_alt() {
        assert_eq!(
            _zclz64(1),
            63,
            "clz64(1) MUST equal 63 (63 leading zeros + 1 LSB)"
        );
    }

    /// c:25 — `_zclz64(1 << 63)` returns 0 (top bit set).
    #[test]
    fn zclz64_top_bit_returns_zero() {
        assert_eq!(
            _zclz64(1u64 << 63),
            0,
            "clz64(1<<63) MUST equal 0 (MSB set)"
        );
    }

    /// c:25 — `_zclz64` matches std `u64::leading_zeros` for power-of-2 inputs.
    #[test]
    fn zclz64_matches_stdlib_for_powers_of_two() {
        for shift in 0..64 {
            let x = 1u64 << shift;
            assert_eq!(
                _zclz64(x) as u32,
                x.leading_zeros(),
                "_zclz64(1<<{}) must match stdlib leading_zeros",
                shift
            );
        }
    }

    /// c:83 — `random_64bit` is non-deterministic (two calls almost
    /// always differ; probability of equal = 1 / 2^64 ≈ 5.4e-20).
    #[test]
    fn random_64bit_two_calls_almost_always_differ() {
        let _g = crate::test_util::global_state_lock();
        let a = random_64bit();
        let b = random_64bit();
        assert_ne!(a, b, "two random_64bit() calls must differ");
    }

    /// c:83 — `random_64bit` produces values spanning multiple bytes
    /// (sanity check that the RNG isn't returning small/clamped values).
    #[test]
    fn random_64bit_eventually_exceeds_32_bit_threshold() {
        let _g = crate::test_util::global_state_lock();
        let any_large = (0..200).any(|_| random_64bit() > (u32::MAX as u64));
        assert!(
            any_large,
            "200 random_64bit values must include ≥ 1 above u32::MAX"
        );
    }

    /// c:119 — `random_real` returns f64 (alt-name compile-time pin).
    #[test]
    fn random_real_returns_f64_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: f64 = random_real();
    }

    /// c:119 — `random_real` outputs are always finite (no NaN/Inf).
    #[test]
    fn random_real_always_finite() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..500 {
            let v = random_real();
            assert!(
                v.is_finite(),
                "random_real must always be finite, got {}",
                v
            );
        }
    }

    /// c:216 — `clz64(0)` returns 64 (consistent with `_zclz64`).
    #[test]
    fn clz64_zero_returns_64() {
        assert_eq!(clz64(0), 64, "clz64(0) MUST equal 64");
    }
}
