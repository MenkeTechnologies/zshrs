//! Random number module - port of Modules/random.c
//!
//! Provides access to kernel random sources for cryptographically secure
//! random number generation.

use std::io;
use std::fs::File;
use std::io::Read;
use std::fs::metadata;
use std::os::unix::fs::FileTypeExt;
use std::os::fd::IntoRawFd;
use std::sync::atomic::Ordering;

/// Buffer size for pre-loading random integers
// buffer to pre-load integers for SRANDOM to lessen the context switches  // c:49
const RAND_BUFF_SIZE: usize = 8;

// Per-evaluator random-buffer state — bucket-1 dissolution per
// PORT_PLAN.md Phase 2. C source has TWO file-statics at
// Src/Modules/random.c:50-51:
//
//     static uint32_t rand_buff[8];
//     static int      buf_cnt = -1;
//
// Previous Rust port aggregated these into a `pub struct
// RandomState { buffer, buf_cnt }`, which is the bag-of-globals
// anti-pattern PORT_PLAN forbids. Dissolved into two
// `thread_local!`s mirroring the C declarations one-for-one;
// each worker thread owns its own buffer (file-static semantics
// preserve under threading per PORT_PLAN bucket-1 rule).

thread_local! {
    /// Port of file-static `static uint32_t rand_buff[8];` at
    /// `Src/Modules/random.c:50`. Pre-loaded buffer of u32s
    /// drained one entry at a time by `get_srandom()`.
    static RAND_BUFF: std::cell::RefCell<[u32; RAND_BUFF_SIZE]> = const {
        std::cell::RefCell::new([0; RAND_BUFF_SIZE])
    };
    /// Port of file-static `static int buf_cnt = -1;` at
    /// `Src/Modules/random.c:51`. Index of the next unread entry
    /// in `RAND_BUFF`; zero triggers a refill via
    /// `getrandom_buffer`.
    static BUF_CNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

/// Port of `get_srandom(UNUSED(Param pm))` from `Src/Modules/random.c:58`. The
/// `getfn` slot the C source wires for the `$SRANDOM` special
/// parameter. Refills `rand_buff` via `getrandom_buffer()` when
/// drained, then returns the next pre-loaded u32.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_srandom() -> u32 {                                                // c:58
    let cnt = BUF_CNT.with(|c| c.get());
    if cnt == 0 {                                                            // c:145
        let mut bytes = [0u8; RAND_BUFF_SIZE * 4];                           // c:143
        if getrandom_buffer(&mut bytes).is_ok() {                            // c:143
            RAND_BUFF.with(|r| {
                let mut buf = r.borrow_mut();
                for (i, chunk) in bytes.chunks_exact(4).enumerate() {        // c:143
                    buf[i] = u32::from_ne_bytes(
                        [chunk[0], chunk[1], chunk[2], chunk[3]],
                    );
                }
            });
        }
        BUF_CNT.with(|c| c.set(RAND_BUFF_SIZE));                             // c:145
    }
    let new_cnt = BUF_CNT.with(|c| c.get()) - 1;                             // c:145
    BUF_CNT.with(|c| c.set(new_cnt));
    RAND_BUFF.with(|r| r.borrow()[new_cnt])                                  // c:145
}

// WARNING: NOT IN RANDOM.C — Rust-only convenience helpers.
// C inlines the equivalent logic inside `get_bound_random_buffer()`
// (Src/Modules/random.c:104) and the math-fn wrappers; the Rust
// port factors them out because callers in this same file (the
// Fisher-Yates shuffle in `bin_zshuffle`, the math fns
// `math_zrand_int`/`math_zrand_real`, the `get_bound_random_buffer`
// loop) would each inline identical 4-line getrandom-and-decode
// blocks. Names are Rust-original; renaming to match a C name
// would mislead.

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
/// One-shot u32 read. C inlines the equivalent at random.c:79
/// (4-byte getrandom_buffer + decode).
pub fn random_u32() -> u32 {
    let mut buf = [0u8; 4];
    let _ = getrandom_buffer(&mut buf);
    u32::from_ne_bytes(buf)
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
/// Two-word read used by `random_real()` at random_real.c:158-175
/// for uniform-real sampling. C reads 8 bytes inline.
pub fn random_u64() -> u64 {
    let mut buf = [0u8; 8];
    let _ = getrandom_buffer(&mut buf);
    u64::from_ne_bytes(buf)
}

/// Get a random integer in `[0, max)` using Lemire's unbiased
/// rejection. Port of the inline bound-rejection logic inside
/// `get_bound_random_buffer()` (random.c:104) — extracted as a
/// per-element scalar helper since multiple callers need the
/// single-value form.
pub fn bounded(max: u32) -> u32 {
    if max == 0 {
        return 0;
    }
    if max == u32::MAX {
        return random_u32();
    }
    let mut x = random_u32();
    let mut m = (x as u64) * (max as u64);
    let mut l = m as u32;
    if l < max {
        let threshold = (-(max as i64) as u64 % max as u64) as u32;
        while l < threshold {
            x = random_u32();
            m = (x as u64) * (max as u64);
            l = m as u32;
        }
    }
    (m >> 32) as u32
}

/// Fill a buffer with cryptographically random bytes.
/// Port of `getrandom_buffer(void *buf, size_t len)` from Src/Modules/random.c:62 — the
/// C source dispatches to `getentropy(3)` on BSD, `getrandom(2)` on
/// Linux, or `/dev/urandom` as a portable fallback. We map onto
/// `arc4random_buf(3)` for macOS (BSD-derived), `getrandom(2)` on
/// Linux, and `/dev/urandom` everywhere else.
#[cfg(target_os = "macos")]
/// WARNING: param names don't match C — Rust=(buf) vs C=(buf, len)
pub fn getrandom_buffer(buf: &mut [u8]) -> io::Result<()> {                  // c:62
    unsafe {
        libc::arc4random_buf(buf.as_mut_ptr() as *mut libc::c_void, buf.len());
    }
    Ok(())
}

/// Port of `getrandom_buffer(void *buf, size_t len)` from `Src/Modules/random.c:62`,
/// `#elif defined(HAVE_GETRANDOM)` branch (c:75-76):
/// `ret = getrandom(bufptr, (len - val), 0);`
/// with the C EINTR-retry loop at c:80-85.
#[cfg(target_os = "linux")]
/// WARNING: param names don't match C — Rust=(buf) vs C=(buf, len)
pub fn getrandom_buffer(buf: &mut [u8]) -> io::Result<()> {                  // c:62
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

/// Port of `getrandom_buffer(void *buf, size_t len)` from `Src/Modules/random.c:62`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn getrandom_buffer(m: &mut [u8]) -> io::Result<()> {                  // c:62

    let mut file = File::open("/dev/urandom")?;
    file.read_exact(m)?;
    Ok(())
}

/// Fill a buffer with bounded random integers.
/// Port of `get_bound_random_buffer(uint32_t *buffer, size_t count, uint32_t max)` from Src/Modules/random.c:104
/// — repeatedly pulls from the kernel and rejection-samples each
/// slot until the entire buffer is filled with values in `[0, max)`.
/// WARNING: param names don't match C — Rust=(buffer, max) vs C=(buffer, count, max)
pub fn get_bound_random_buffer(buffer: &mut [u32], max: u32) {               // c:104
    for item in buffer.iter_mut() {
        *item = bounded(max);
    }
}

/// `math_zrand_int(upper, lower, inclusive)` math function.
/// Port of `math_zrand_int(UNUSED(char *name), int argc, mnumber *argv, UNUSED(int id))` from Src/Modules/random.c:161 — the
/// C source's math-function entry point exposed to `${(( ... ))}`.
/// All three arguments are optional; behaviour matches the C
/// source's bound-checks (`lower < 0`, `upper < lower`, etc.).
/// WARNING: param names don't match C — Rust=(upper, lower, inclusive) vs C=(name, argc, argv, id)
pub fn math_zrand_int(upper: Option<i64>, lower: Option<i64>, inclusive: bool) -> Result<i64, String> { // c:161
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

    let r = bounded(diff);
    Ok(r as i64 + lower)
}

/// `math_zrand_float()` math function.
/// Port of `math_zrand_float(UNUSED(char *name), UNUSED(int argc), UNUSED(mnumber *argv), UNUSED(int id))` from Src/Modules/random.c:204 —
/// the C source's math-function entry point that returns a
/// uniform double in `[0, 1)`.
/// WARNING: param names don't match C — Rust=() vs C=(name, argc, argv, id)
pub fn math_zrand_float() -> f64 {                                           // c:204
    random_real()
}

/// Re-export of the canonical `random_real()` from
/// `Src/Modules/random_real.c:147` — Campbell's algorithm for
/// distribution-correct uniform doubles in `[0, 1)`. The simpler
/// "53-bit mantissa" approximation that previously lived here was
/// removed because it biases ~3% of the interval; the C author
/// (Taylor R. Campbell) explicitly warns against it in the random_real.c
/// header comment.
pub use crate::ported::modules::random_real::random_real;

/// Generate a random integer in `[min, max]`.
#[cfg(test)]
mod tests {
    use super::*;

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_random_state() {
        
        let r1 = get_srandom();
        let r2 = get_srandom();
        let r3 = get_srandom();
        assert!(r1 != r2 || r2 != r3);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_get_random_u32() {
        let r1 = random_u32();
        let r2 = random_u32();
        let r3 = random_u32();
        assert!(r1 != r2 || r2 != r3);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_get_random_u64() {
        let r1 = random_u64();
        let r2 = random_u64();
        assert_ne!(r1, r2);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_bounded_random() {
        for _ in 0..100 {
            let r = bounded(10);
            assert!(r < 10);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_bounded_random_one() {
        for _ in 0..10 {
            let r = bounded(1);
            assert_eq!(r, 0);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_zrand_int() {
        let r = math_zrand_int(Some(100), Some(50), false).unwrap();
        assert!((50..100).contains(&r));

        let r = math_zrand_int(Some(100), Some(50), true).unwrap();
        assert!((50..=100).contains(&r));
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_zrand_int_no_args() {
        let r = math_zrand_int(None, None, false).unwrap();
        assert!(r >= 0);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_zrand_int_errors() {
        assert!(math_zrand_int(Some(50), Some(100), false).is_err());
        assert!(math_zrand_int(Some(-1), None, false).is_err());
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_zrand_float() {
        for _ in 0..100 {
            let r = math_zrand_float();
            assert!((0.0..1.0).contains(&r));
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_random_real() {
        for _ in 0..100 {
            let r = random_real();
            assert!((0.0..1.0).contains(&r));
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_shuffle() {
        // Fisher–Yates shuffle, inlined here since the helper is gone.
        let mut arr = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let original = arr.clone();
        let n = arr.len();
        for i in (1..n).rev() {
            let j = bounded((i + 1) as u32) as usize;
            arr.swap(i, j);
        }
        arr.sort();
        assert_eq!(arr, original.to_vec());
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/random.c`.
    #[test]
    fn test_fill_random_bytes() {
        let mut buf = [0u8; 32];
        getrandom_buffer(&mut buf).unwrap();
        assert!(!buf.iter().all(|&b| b == 0));
    }
}

// =====================================================================
// static struct features module_features                            c:255 (random.c)
// =====================================================================

use crate::ported::zsh_h::module;

// `mftab` — port of `static struct mathfunc mftab[]` (random.c).


// `patab` — port of `static struct paramdef patab[]` (random.c).


// `module_features` — port of `static struct features module_features`
// from random.c:255.



/// `RANDFD` — port of the file-static `int randfd` in
/// `Src/Modules/random.c:243`. Holds the open fd for `/dev/urandom`.
/// Set in `boot_()`, closed in `finish_()`.
pub static RANDFD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:34

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/random.c:243`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {                                    // c:243
    // c:243-261 — USE_URANDOM block: stat /dev/urandom; verify
    //              S_ISCHR. We probe via std::fs::metadata + file_type().
    match metadata("/dev/urandom") {                                         // c:251
        Ok(md) => {
            if !md.file_type().is_char_device() {                            // c:256
                tracing::warn!(target: "random", "Error getting kernel random pool: not a char device");
                return 1;
            }
        }
        Err(e) => {
            tracing::warn!(target: "random", "Error getting kernel random pool: {}", e);
            return 1;
        }
    }
    0                                                                        // c:275
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/random.c:267`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {     // c:267
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/random.c:275`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {  // c:275
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/random.c:282`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {                                     // c:282
    // c:282-308 — USE_URANDOM block: open(/dev/urandom, O_RDONLY),
    //              movefd, addmodulefd to track the fd.
    match std::fs::OpenOptions::new().read(true).open("/dev/urandom") {      // c:295
        Ok(f) => {
            let fd = f.into_raw_fd();                                        // c:312
            RANDFD.store(fd, Ordering::SeqCst);
            0
        }
        Err(e) => {
            tracing::warn!(target: "random", "Could not access kernel random pool: {}", e);
            1                                                                // c:319
        }
    }
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/random.c:312`.
pub fn cleanup_(m: *const module) -> i32 {                                  // c:312
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/random.c:319`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {                                   // c:319
    // c:319-324 — USE_URANDOM block: `if (randfd >= 0) zclose(randfd)`.
    let fd = RANDFD.swap(-1, Ordering::SeqCst);
    if fd >= 0 {
        unsafe { libc::close(fd) };                                          // c:323 zclose
    }
    0
}

use crate::ported::zsh_h::features as features_t;
use std::sync::{Mutex, OnceLock};

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,
        bn_size: 0,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 2,
        pd_list: None,
        pd_size: 1,
        n_abstract: 0,
    }))
}

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec!["f:zrand_float".to_string(), "f:zrand_int".to_string(), "p:SRANDOM".to_string()]
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
fn handlefeatures(
    _m: *const module,
    _f: &Mutex<features_t>,
    enables: &mut Option<Vec<i32>>,
) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 3]);
    }
    0
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/random.c`.
fn setfeatureenables(
    _m: *const module,
    _f: &Mutex<features_t>,
    _e: Option<&[i32]>,
) -> i32 {
    0
}

