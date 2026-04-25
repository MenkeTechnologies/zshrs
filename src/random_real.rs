//! Random real module - port of Modules/random_real.c
//!
//! Provides high-quality floating-point random numbers.

use crate::random;

/// Generate a random double in [0, 1)
pub fn random_real() -> f64 {
    random::zrand_float()
}

/// Generate a random double in [0, max)
pub fn random_real_max(max: f64) -> f64 {
    random_real() * max
}

/// Generate a random double in [min, max)
pub fn random_real_range(min: f64, max: f64) -> f64 {
    min + random_real() * (max - min)
}

/// Generate high-precision random in [0, 1) using 53 bits
pub fn random_real_53() -> f64 {
    let a = random::get_random_u32() >> 5;
    let b = random::get_random_u32() >> 6;
    (a as f64 * 67108864.0 + b as f64) * (1.0 / 9007199254740992.0)
}

/// Math function for random real
pub fn math_random_real(args: &[f64]) -> Result<f64, String> {
    match args.len() {
        0 => Ok(random_real_53()),
        1 => {
            let max = args[0];
            if max <= 0.0 {
                return Err("random: max must be positive".to_string());
            }
            Ok(random_real_max(max))
        }
        2 => {
            let min = args[0];
            let max = args[1];
            if max <= min {
                return Err("random: max must be greater than min".to_string());
            }
            Ok(random_real_range(min, max))
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
            assert!(r >= 0.0 && r < 1.0);
        }
    }

    #[test]
    fn test_random_real_max() {
        for _ in 0..100 {
            let r = random_real_max(10.0);
            assert!(r >= 0.0 && r < 10.0);
        }
    }

    #[test]
    fn test_random_real_min_max() {
        for _ in 0..100 {
            let r = random_real_range(5.0, 10.0);
            assert!(r >= 5.0 && r < 10.0);
        }
    }

    #[test]
    fn test_random_real_53() {
        for _ in 0..100 {
            let r = random_real_53();
            assert!(r >= 0.0 && r < 1.0);
        }
    }

    #[test]
    fn test_math_random_real_no_args() {
        let result = math_random_real(&[]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r >= 0.0 && r < 1.0);
    }

    #[test]
    fn test_math_random_real_one_arg() {
        let result = math_random_real(&[100.0]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r >= 0.0 && r < 100.0);
    }

    #[test]
    fn test_math_random_real_two_args() {
        let result = math_random_real(&[10.0, 20.0]);
        assert!(result.is_ok());
        let r = result.unwrap();
        assert!(r >= 10.0 && r < 20.0);
    }

    #[test]
    fn test_math_random_real_invalid() {
        assert!(math_random_real(&[-1.0]).is_err());
        assert!(math_random_real(&[10.0, 5.0]).is_err());
        assert!(math_random_real(&[1.0, 2.0, 3.0]).is_err());
    }
}
