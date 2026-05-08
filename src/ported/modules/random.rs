//! Random number module - port of Modules/random.c
//!
//! Provides access to kernel random sources for cryptographically secure
//! random number generation.

use std::io;

/// Buffer size for pre-loading random integers
const RAND_BUFF_SIZE: usize = 8;

/// Random number generator state.
/// Port of the file-static `rand_buff` / `buf_cnt` slot
/// Src/Modules/random.c keeps for batched kernel reads — refilling
/// 8 u32s at a time amortizes the `getrandom(2)` syscall cost
/// across `$SRANDOM` reads.
#[derive(Debug)]
pub struct RandomState {
    buffer: [u32; RAND_BUFF_SIZE],
    buf_cnt: usize,
}

impl Default for RandomState {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    fn default() -> Self {
        Self::new()
    }
}

impl RandomState {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    pub fn new() -> Self {
        Self {
            buffer: [0; RAND_BUFF_SIZE],
            buf_cnt: 0,
        }
    }

    /// Get a random u32 value.
    /// Port of `get_srandom()` from Src/Modules/random.c:143 — the
    /// `getfn` slot the C source wires for the `$SRANDOM` special
    /// parameter. Refills the buffer via `getrandom_buffer()` when
    /// drained.
    pub fn get_srandom(&mut self) -> u32 {
        if self.buf_cnt == 0 {                                                  // c:145
            let mut bytes = [0u8; RAND_BUFF_SIZE * 4];                          // c:143
            if getrandom_buffer(&mut bytes).is_ok() {                          // c:143
                for (i, chunk) in bytes.chunks_exact(4).enumerate() {           // c:143
                    self.buffer[i] = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);  // c:143
                }                                                               // c:143
            }                                                                   // c:143
            self.buf_cnt = RAND_BUFF_SIZE;                                      // c:145
        }                                                                       // c:143
        self.buf_cnt -= 1;                                                      // c:145
        self.buffer[self.buf_cnt]                                               // c:145
    }
}

/// Fill a buffer with cryptographically random bytes.
/// Port of `getrandom_buffer()` from Src/Modules/random.c:62 — the
/// C source dispatches to `getentropy(3)` on BSD, `getrandom(2)` on
/// Linux, or `/dev/urandom` as a portable fallback. We map onto
/// `arc4random_buf(3)` for macOS (BSD-derived), `getrandom(2)` on
/// Linux, and `/dev/urandom` everywhere else.
#[cfg(target_os = "macos")]
pub fn getrandom_buffer(buf: &mut [u8]) -> io::Result<()> {
    unsafe {
        libc::arc4random_buf(buf.as_mut_ptr() as *mut libc::c_void, buf.len());
    }
    Ok(())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
#[cfg(target_os = "linux")]
pub fn getrandom_buffer(buf: &mut [u8]) -> io::Result<()> {
    let mut filled = 0;

    while filled < buf.len() {
        let ret = unsafe {
            libc::getrandom(
                buf[filled..].as_mut_ptr() as *mut libc::c_void,
                buf.len() - filled,
                0,
            )
        };

        if ret < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }

        filled += ret as usize;
    }

    Ok(())
}

/// Port of `getrandom_buffer()` from `Src/Modules/random.c:282`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn getrandom_buffer(buf: &mut [u8]) -> io::Result<()> {
    use std::fs::File;
    use std::io::Read;

    let mut file = File::open("/dev/urandom")?;
    file.read_exact(buf)?;
    Ok(())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
/// Get a single random u32.
/// One-shot variant of `getrandom_buffer()` from Src/Modules/
/// random.c:62 — convenience for callers that don't need the
/// 8-element batching `RandomState` provides.
pub fn get_random_u32() -> u32 {
    let mut buf = [0u8; 4];
    let _ = getrandom_buffer(&mut buf);
    u32::from_ne_bytes(buf)
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
/// Get a single random u64.
/// Two-word variant of `get_random_u32` — used by the
/// `random_real()` path in Src/Modules/random_real.c:158-175 which
/// reads 64 bits at a time when building uniform-real samples.
pub fn get_random_u64() -> u64 {
    let mut buf = [0u8; 8];
    let _ = getrandom_buffer(&mut buf);
    u64::from_ne_bytes(buf)
}

/// Get a random integer in `[0, max)` using Lemire's unbiased
/// rejection algorithm.
/// Port of the inline bound-rejection logic inside
/// `get_bound_random_buffer()` from Src/Modules/random.c:104. The C
/// source uses the same `(x * max) >> 32` reduction with rejection
/// when the low 32 bits fall under the bias threshold.
pub fn get_bounded_random(max: u32) -> u32 {
    if max == 0 {
        return 0;
    }

    if max == u32::MAX {
        return get_random_u32();
    }

    let mut x = get_random_u32();
    let mut m = (x as u64) * (max as u64);
    let mut l = m as u32;

    if l < max {
        let threshold = (-(max as i64) as u64 % max as u64) as u32;
        while l < threshold {
            x = get_random_u32();
            m = (x as u64) * (max as u64);
            l = m as u32;
        }
    }

    (m >> 32) as u32
}

/// Fill a buffer with bounded random integers.
/// Port of `get_bound_random_buffer()` from Src/Modules/random.c:104
/// — repeatedly pulls from the kernel and rejection-samples each
/// slot until the entire buffer is filled with values in `[0, max)`.
pub fn get_bound_random_buffer(buffer: &mut [u32], max: u32) {
    for item in buffer.iter_mut() {
        *item = get_bounded_random(max);
    }
}

/// `math_zrand_int(upper, lower, inclusive)` math function.
/// Port of `math_zrand_int()` from Src/Modules/random.c:161 — the
/// C source's math-function entry point exposed to `${(( ... ))}`.
/// All three arguments are optional; behaviour matches the C
/// source's bound-checks (`lower < 0`, `upper < lower`, etc.).
pub fn math_zrand_int(upper: Option<i64>, lower: Option<i64>, inclusive: bool) -> Result<i64, String> {
    let lower = lower.unwrap_or(0);
    let upper = upper.unwrap_or(u32::MAX as i64);

    if lower < 0 || lower > u32::MAX as i64 {
        return Err(format!(
            "Lower bound ({}) out of range: 0-4294967295",
            lower
        ));
    }

    if upper < lower {
        return Err(format!(
            "Upper bound ({}) must be greater than Lower Bound ({})",
            upper, lower
        ));
    }

    if upper < 0 || upper > u32::MAX as i64 {
        return Err(format!(
            "Upper bound ({}) out of range: 0-4294967295",
            upper
        ));
    }

    let incl = if inclusive { 1 } else { 0 };
    let diff = (upper - lower + incl) as u32;

    if diff == 0 {
        return Ok(upper);
    }

    let r = get_bounded_random(diff);
    Ok(r as i64 + lower)
}

/// `math_zrand_float()` math function.
/// Port of `math_zrand_float()` from Src/Modules/random.c:204 —
/// the C source's math-function entry point that returns a
/// uniform double in `[0, 1)`.
pub fn math_zrand_float() -> f64 {
    random_real()
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
/// Generate a random real in `[0, 1)` using 53 bits of randomness.
/// Equivalent to the simpler "53-bit mantissa" variant
/// Src/Modules/random_real.c documents at line 145; for the
/// distribution-correct path see `crate::random_real::random_real`.
pub fn random_real() -> f64 {
    let x = get_random_u64();
    (x >> 11) as f64 * (1.0 / (1u64 << 53) as f64)
}

/// Generate a random integer in `[min, max]`.
/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
/// Shuffle a slice in place using Fisher–Yates.
/// zshrs-original convenience — closest C zsh analog is the
/// `${(o)array}` / `${(O)array}` deterministic-sort code in
/// Src/subst.c. C zsh doesn't ship a shuffle primitive; this is
/// added for parameter-flag and stryke completeness.
pub fn shuffle<T>(slice: &mut [T]) {
    let n = slice.len();
    if n <= 1 {
        return;
    }

    for i in (1..n).rev() {
        let j = get_bounded_random((i + 1) as u32) as usize;
        slice.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_state() {
        let mut state = RandomState::new();
        let r1 = state.get_srandom();
        let r2 = state.get_srandom();
        let r3 = state.get_srandom();
        assert!(r1 != r2 || r2 != r3);
    }

    #[test]
    fn test_get_random_u32() {
        let r1 = get_random_u32();
        let r2 = get_random_u32();
        let r3 = get_random_u32();
        assert!(r1 != r2 || r2 != r3);
    }

    #[test]
    fn test_get_random_u64() {
        let r1 = get_random_u64();
        let r2 = get_random_u64();
        assert_ne!(r1, r2);
    }

    #[test]
    fn test_bounded_random() {
        for _ in 0..100 {
            let r = get_bounded_random(10);
            assert!(r < 10);
        }
    }

    #[test]
    fn test_bounded_random_one() {
        for _ in 0..10 {
            let r = get_bounded_random(1);
            assert_eq!(r, 0);
        }
    }

    #[test]
    fn test_zrand_int() {
        let r = math_zrand_int(Some(100), Some(50), false).unwrap();
        assert!((50..100).contains(&r));

        let r = math_zrand_int(Some(100), Some(50), true).unwrap();
        assert!((50..=100).contains(&r));
    }

    #[test]
    fn test_zrand_int_no_args() {
        let r = math_zrand_int(None, None, false).unwrap();
        assert!(r >= 0);
    }

    #[test]
    fn test_zrand_int_errors() {
        assert!(math_zrand_int(Some(50), Some(100), false).is_err());
        assert!(math_zrand_int(Some(-1), None, false).is_err());
    }

    #[test]
    fn test_zrand_float() {
        for _ in 0..100 {
            let r = math_zrand_float();
            assert!((0.0..1.0).contains(&r));
        }
    }

    #[test]
    fn test_random_real() {
        for _ in 0..100 {
            let r = random_real();
            assert!((0.0..1.0).contains(&r));
        }
    }

    #[test]
    fn test_shuffle() {
        let mut arr = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let original = arr.clone();
        shuffle(&mut arr);
        arr.sort();
        assert_eq!(arr, original.to_vec());
    }

    #[test]
    fn test_fill_random_bytes() {
        let mut buf = [0u8; 32];
        getrandom_buffer(&mut buf).unwrap();
        assert!(!buf.iter().all(|&b| b == 0));
    }
}

/// Module loader entry — port of `setup_()` from Src/Modules/random.c:243.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/random.c:267.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/random.c:275.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/random.c:282.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/random.c:312.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/random.c:319.
pub fn finish_() -> i32 {
    0
}
