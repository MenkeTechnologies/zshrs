//! Rust-only utility (NOT a port — lives outside `src/ported/` by design).
//!
//! C's `(zlong) d` for a `double` that is NaN, infinite or outside the
//! `zlong` range is undefined behaviour, so the answer is whatever the CPU's
//! convert instruction produces. zsh therefore prints different numbers on
//! different hosts for the same script (`integer i; i=$(( 1.0/0 ))`):
//!
//!   * x86 / x86_64 (`cvttsd2si`): the "integer indefinite" value, `i64::MIN`,
//!     for NaN, ±inf and every out-of-range magnitude;
//!   * aarch64 (`fcvtzs`): saturates (`i64::MAX` / `i64::MIN`) and maps NaN
//!     to 0 — the same answer Rust's `as i64` gives.
//!
//! zshrs follows the host it runs on, exactly as the C build would, so a
//! script gives the same number under zsh and zshrs on the same machine.

/// C's `(zlong) d`: truncate toward zero, with the host CPU's answer for
/// values that do not fit.
#[inline]
pub fn zlong_from_double(d: f64) -> i64 {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        // 2^63 is exactly representable; the valid range is [-2^63, 2^63).
        if !(-9_223_372_036_854_775_808.0..9_223_372_036_854_775_808.0).contains(&d) {
            return i64::MIN;
        }
    }
    d as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_range_values_truncate_toward_zero() {
        assert_eq!(zlong_from_double(3.9), 3);
        assert_eq!(zlong_from_double(-3.9), -3);
        assert_eq!(zlong_from_double(-9_223_372_036_854_775_808.0), i64::MIN);
    }

    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    #[test]
    fn x86_answers_integer_indefinite_for_unrepresentable() {
        for d in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 9.3e18, -9.3e18] {
            assert_eq!(zlong_from_double(d), i64::MIN, "{d}");
        }
    }

    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    #[test]
    fn saturating_hosts_clamp_and_zero_nan() {
        assert_eq!(zlong_from_double(f64::INFINITY), i64::MAX);
        assert_eq!(zlong_from_double(f64::NEG_INFINITY), i64::MIN);
        assert_eq!(zlong_from_double(f64::NAN), 0);
    }
}
