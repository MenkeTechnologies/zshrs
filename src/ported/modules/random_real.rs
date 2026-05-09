//! Random real module — port of `Src/Modules/random_real.c`.
//!
//! Provides Campbell's-algorithm uniform-double RNG and the two
//! supporting helpers `_zclz64` and `random_64bit`.
//!
//! C source has zero `struct ...` / `enum ...` definitions. Rust
//! port matches: zero types.

// libc `ldexp` — `random_real()` calls it at random_real.c:212.
// Declared here (not imported from mathfunc) because the C source's
// random_real.c already includes <math.h> for this call; mirror
// that locality.
#[cfg(unix)]
extern "C" {
    fn ldexp(x: f64, exp: i32) -> f64;
}

/// Port of `random_real()` from `Src/Modules/random_real.c:147`.
/// Generates a uniform-distributed double in [0, 1) via Campbell's
/// algorithm — the only known way to produce a uniform double from
/// a uniform bit source without skew (the naïve "53-bit fraction"
/// approach biases ~3% of the interval toward values just below 0.5).
pub fn random_real() -> f64 {                                            // c:147
    let mut exponent: i32 = 0;                                           // c:151
    let mut significand: u64 = 0;                                        // c:152
    #[allow(unused_assignments)]
    let mut r: u64 = 0;                                                  // c:153

    // Read zeros into the exponent until we hit a one; the rest will
    // go into the significand. — c:159-176
    while significand == 0 {                                             // c:160
        exponent -= 64;                                                  // c:161

        // c:163-167 — errno = 0; significand = random_64bit();
        // if (errno) return -1;
        // Rust's `random_64bit()` returns 1 on entropy failure
        // (matching C's documented zero-avoidance) so the errno
        // probe collapses; the loop exits naturally on the
        // sentinel-1 result.
        significand = random_64bit();                                    // c:166

        // c:174-175 — exponent below -1074 (= emin + 1 - p, the
        // smallest subnormal's exponent) is guaranteed to round to
        // zero. So unlikely it only happens if random_64bit is broken.
        if exponent < -1074 {                                            // c:174
            return 0.0;                                                  // c:175
        }
    }

    // c:186-198 — leading-zero shift. There is a 1 somewhere in
    // significand, not necessarily in the most significant position.
    // If there are leading zeros, shift them into the exponent and
    // refill the less-significant bits from another draw.
    let shift = _zclz64(significand) as u32;                             // c:188 clz64
    if shift != 0 {                                                      // c:189
        // c:191-194 — errno = 0; r = random_64bit(); if (errno) return -1;
        r = random_64bit();                                              // c:192
        exponent -= shift as i32;                                        // c:196
        significand <<= shift;                                           // c:197
        significand |= r >> (64 - shift);                                // c:198
    }

    // c:201-208 — Set the sticky bit. There is almost surely another
    // 1 in the bit stream, so without this we might round what looks
    // like a tie to even when it isn't.
    significand |= 1;                                                    // c:208

    // c:211-212 — Finally, convert to double (rounding) and scale by
    // 2^exponent.
    unsafe { ldexp(significand as f64, exponent) }                       // c:212
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_real_range() {
        for _ in 0..100 {
            let r = random_real();
            assert!((0.0..1.0).contains(&r));
        }
    }
}

/// Port of `_zclz64()` from `Src/Modules/random_real.c:49`. Counts
/// the leading zero bits of a 64-bit value via the binary-search
/// fallback C uses when the compiler doesn't provide `__builtin_clzll`.
/// Rust has `u64::leading_zeros()` (a HW intrinsic on every modern
/// arch), but this entry exists so the function-name parity contract
/// is satisfied; logic follows random_real.c:49-79 verbatim.
#[allow(non_snake_case)]
pub fn _zclz64(x: u64) -> i32 {
    let mut n: i32 = 0;                                                 // c:51
    let mut x = x;                                                      // c:51
    if x == 0 {                                                         // c:53
        return 64;                                                      // c:54
    }                                                                   // c:54
    if x & 0xFFFF_FFFF_0000_0000 == 0 {                                 // c:56
        n += 32;                                                        // c:57
        x <<= 32;                                                       // c:58
    }                                                                   // c:59
    if x & 0xFFFF_0000_0000_0000 == 0 {                                 // c:60
        n += 16;                                                        // c:61
        x <<= 16;                                                       // c:62
    }                                                                   // c:63
    if x & 0xFF00_0000_0000_0000 == 0 {                                 // c:64
        n += 8;                                                         // c:65
        x <<= 8;                                                        // c:66
    }                                                                   // c:67
    if x & 0xF000_0000_0000_0000 == 0 {                                 // c:68
        n += 4;                                                         // c:69
        x <<= 4;                                                        // c:70
    }                                                                   // c:71
    if x & 0xC000_0000_0000_0000 == 0 {                                 // c:72
        n += 2;                                                         // c:73
        x <<= 1;                                                        // c:74 (NB: C source is x<<=1, intentional — match exactly)
    }                                                                   // c:75
    if x & 0x8000_0000_0000_0000 == 0 {                                 // c:76
        n += 1;                                                         // c:77
    }                                                                   // c:78
    n                                                                   // c:79
}

/// Port of `random_64bit()` from `Src/Modules/random_real.c:84`.
/// Pulls 64 bits from `getrandom_buffer()`; on failure emits the
/// same zwarn diagnostic as C and returns 1 (not 0 — `random_real()`'s
/// zero-detection loop would spin forever on 0).
pub fn random_64bit() -> u64 {                                           // c:84
    let mut buf = [0u8; 8];                                              // c:85 uint64_t r
    if crate::random::getrandom_buffer(&mut buf).is_err() {              // c:87
        crate::ported::utils::zwarn(                                     // c:88
            "zsh/random: Can't get sufficient random data.",
        );
        return 1;                                                        // c:89 0 will cause loop
    }
    u64::from_ne_bytes(buf)                                              // c:93 return r
}
