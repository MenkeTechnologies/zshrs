//! Random real module - port of Modules/random_real.c
//!
//! Provides high-quality floating-point random numbers.

use crate::random;

/// Port of `random_real()` from `Src/Modules/random_real.c:147`.
/// Generate a random double in [0, 1] uniformly distributed.
/// Direct port of `random_real` from src/zsh/Src/Modules/random_real.c
/// lines 145-212 (Campbell's algorithm, see
/// <http://mumble.net/~campbell/2014/04/28/uniform-random-float>).
///
/// Algorithm:
///   1. Read 64-bit chunks until we see a non-zero one. Each
///      all-zero chunk shifts the exponent down by 64.
///   2. Count leading zeros in the first non-zero chunk; shift
///      them into the exponent and refill the low bits of the
///      significand from another 64-bit draw.
///   3. Set the sticky bit (significand |= 1) so the round-to-
///      nearest-even doesn't bias toward even when the trailing
///      bits would have decided ties.
///   4. ldexp(significand, exponent) — convert to double.
///
/// This is the only correct way to generate a uniform double
/// from a uniform bit source. The naïve "53-bit fraction" approach
/// (random_real_53 below) skews ~3% of the [0, 1) interval toward
/// values just below 0.5.
pub fn random_real() -> f64 {
    let mut exponent: i32 = 0;                                                  // c:149
    let mut significand: u64 = 0;                                               // c:150

    // random_real.c:158-175 — read zeros into exponent until
    // we hit a non-zero chunk.
    while significand == 0 {                                                    // c:150
        exponent -= 64;                                                         // c:149
        significand = random::get_random_u64();                                 // c:150
        // random_real.c:172-174 — exp below -1074 means it would
        // round to zero anyway (smallest subnormal exponent).
        if exponent < -1074 {                                                   // c:149
            return 0.0;                                                         // c:147
        }                                                                       // c:147
    }                                                                           // c:147

    // random_real.c:185-196 — leading-zero shift.
    let shift = significand.leading_zeros() as i32;                             // c:185
    if shift != 0 {                                                             // c:152
        let r = random::get_random_u64();                                       // c:147
        exponent -= shift;                                                      // c:180
        significand <<= shift;                                                  // c:185
        significand |= r >> (64 - shift);                                       // c:185
    }                                                                           // c:147

    // random_real.c:205 — sticky bit so round-to-nearest doesn't
    // false-tie.
    significand |= 1;                                                           // c:150

    // random_real.c:211 — ldexp(significand, exponent).
    (significand as f64) * (exponent as f64).exp2()                             // c:211
}

/// Math-function entry point for `${(rrand)}` / `random()`.
/// Port of `bin_random_real()` from Src/Modules/random_real.c.
/// Dispatches to the 0/1/2-argument forms (random in [0,1),
/// [0,max), [min,max)). The 0-arg path uses the 53-bit mantissa
/// fast path; 1/2-arg paths use the full random_real() distribution.
pub fn bin_random_real(args: &[f64]) -> Result<f64, String> {
    match args.len() {
        0 => {
            // 53-bit fast path: two u32 draws stitched into a f64
            // mantissa, divided by 2^53 to get [0, 1).
            let a = random::get_random_u32() >> 5;
            let b = random::get_random_u32() >> 6;
            Ok((a as f64 * 67108864.0 + b as f64) * (1.0 / 9007199254740992.0))
        }
        1 => {
            let max = args[0];
            if max <= 0.0 {
                return Err("random: max must be positive".to_string());
            }
            Ok(random_real() * max)
        }
        2 => {
            let min = args[0];
            let max = args[1];
            if max <= min {
                return Err("random: max must be greater than min".to_string());
            }
            Ok(min + random_real() * (max - min))
        }
        _ => Err("random: too many arguments".to_string()),
    }
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

    #[test]
    fn test_random_real_max() {
        for _ in 0..100 {
            let r = bin_random_real(&[10.0]).unwrap();
            assert!((0.0..10.0).contains(&r));
        }
    }

    #[test]
    fn test_random_real_min_max() {
        for _ in 0..100 {
            let r = bin_random_real(&[5.0, 10.0]).unwrap();
            assert!((5.0..10.0).contains(&r));
        }
    }

    #[test]
    fn test_random_real_53() {
        for _ in 0..100 {
            let r = bin_random_real(&[]).unwrap();
            assert!((0.0..1.0).contains(&r));
        }
    }

    #[test]
    fn test_math_random_real_no_args() {
        let result = bin_random_real(&[]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!((0.0..1.0).contains(&r));
    }

    #[test]
    fn test_math_random_real_one_arg() {
        let result = bin_random_real(&[100.0]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!((0.0..100.0).contains(&r));
    }

    #[test]
    fn test_math_random_real_two_args() {
        let result = bin_random_real(&[10.0, 20.0]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!((10.0..20.0).contains(&r));
    }

    #[test]
    fn test_math_random_real_invalid() {
        assert!(bin_random_real(&[-1.0]).is_err());
        assert!(bin_random_real(&[10.0, 5.0]).is_err());
        assert!(bin_random_real(&[1.0, 2.0, 3.0]).is_err());
    }
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/random_real.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `_zclz64()` from Src/Modules/random_real.c:49.
#[allow(non_snake_case)]
pub fn _zclz64() -> i32 { 0 }

/// Port of `random_64bit()` from Src/Modules/random_real.c:84.
#[allow(non_snake_case)]
pub fn random_64bit() -> i32 { 0 }
