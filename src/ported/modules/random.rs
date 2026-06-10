//! Random number module - port of Modules/random.c
//!
//! Provides access to kernel random sources for cryptographically secure
//! random number generation.

use std::fs::metadata;
use std::io;
use std::io::Read;
use std::os::fd::IntoRawFd;
use std::os::unix::fs::FileTypeExt;
use std::sync::atomic::Ordering;

use crate::ported::utils::zwarn;
use crate::random_real::random_real;
use crate::zsh_h::{features, module};
use std::sync::{Mutex, OnceLock};

/// Fill a buffer with cryptographically random bytes.
/// Port of `getrandom_buffer(void *buf, size_t len)` from Src/Modules/random.c:62 — the
/// C source dispatches to `getentropy(3)` on BSD, `getrandom(2)` on
/// Linux, or `/dev/urandom` as a portable fallback. We map onto
/// `arc4random_buf(3)` for macOS (BSD-derived), `getrandom(2)` on
/// Linux, and `/dev/urandom` everywhere else.
#[cfg(target_os = "macos")]
/// WARNING: param names don't match C — Rust=(buf) vs C=(buf, len)
pub fn getrandom_buffer(buf: &mut [u8]) -> io::Result<()> {
    // c:62
    unsafe {
        libc::arc4random_buf(buf.as_mut_ptr() as *mut libc::c_void, buf.len());
    }
    Ok(())
}

// Per-evaluator random-buffer state — bucket-1 dissolution per
// C source has TWO file-statics at Src/Modules/random.c:50-51:
//
//     static uint32_t rand_buff[8];
//     static int      buf_cnt = -1;
//
// Mirrored as two `thread_local!`s — each worker thread owns its
// own buffer (file-static semantics preserve under threading per
// PORT_PLAN bucket-1 rule).

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

/// Port of `getrandom_buffer(void *buf, size_t len)` from `Src/Modules/random.c:62`,
/// `#elif defined(HAVE_GETRANDOM)` branch (c:75-76):
/// `ret = getrandom(bufptr, (len - val), 0);`
/// with the C EINTR-retry loop at c:80-85.
#[cfg(target_os = "linux")]
/// WARNING: param names don't match C — Rust=(buf) vs C=(buf, len)
pub fn getrandom_buffer(buf: &mut [u8]) -> io::Result<()> {
    // c:62
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
pub fn getrandom_buffer(m: &mut [u8]) -> io::Result<()> {
    // c:62

    let mut file = File::open("/dev/urandom")?;
    file.read_exact(m)?;
    Ok(())
}

/// Port of `void get_bound_random_buffer(uint32_t *buffer, size_t count,
/// uint32_t max)` from `Src/Modules/random.c:104`. Lemire (2016)
/// fast-random-shuffling: multiply, threshold = -max % max, rejection-
/// sample only the rare `leftover < max` slot. `count` is folded into
/// `buffer.len()` per Rust idiom.
/// WARNING: param names don't match C — Rust=(buffer, max) vs C=(buffer, count, max)
pub fn get_bound_random_buffer(buffer: &mut [u32], max: u32) {
    // c:104
    // c:112 getrandom_buffer(buffer, count*sizeof(uint32_t)) — fill u32s.
    let mut bytes: Vec<u8> = vec![0u8; buffer.len() * 4];
    let _ = getrandom_buffer(&mut bytes);
    for (i, chunk) in bytes.chunks_exact(4).enumerate() {
        buffer[i] = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
    }
    if max == u32::MAX {
        // c:113 UINT32_MAX
        return; // c:114
    }
    for i in 0..buffer.len() {
        // c:116
        let mut multi_result: u64 = (buffer[i] as u64) * (max as u64); // c:117
        let mut leftover: u32 = multi_result as u32; // c:118
        if leftover < max {
            // c:124
            let threshold: u32 = (max.wrapping_neg()) % max; // c:125 -max % max
            while leftover < threshold {
                // c:126
                let j: u32 = get_srandom(); // c:127 get_srandom(NULL)
                multi_result = (j as u64) * (max as u64); // c:128
                leftover = multi_result as u32; // c:129
            }
        }
        buffer[i] = (multi_result >> 32) as u32; // c:132
    }
}

/// Port of `get_srandom(UNUSED(Param pm))` from `Src/Modules/random.c:58`. The
/// `getfn` slot the C source wires for the `$SRANDOM` special
/// parameter. Refills `rand_buff` via `getrandom_buffer()` when
/// drained, then returns the next pre-loaded u32.
/// WARNING: param names don't match C — Rust=() vs C=(pm)
pub fn get_srandom() -> u32 {
    // c:58
    let cnt = BUF_CNT.with(|c| c.get());
    if cnt == 0 {
        // c:145
        let mut bytes = [0u8; RAND_BUFF_SIZE * 4]; // c:143
        if getrandom_buffer(&mut bytes).is_ok() {
            // c:143
            RAND_BUFF.with(|r| {
                let mut buf = r.borrow_mut();
                for (i, chunk) in bytes.chunks_exact(4).enumerate() {
                    // c:143
                    buf[i] = u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                }
            });
        }
        BUF_CNT.with(|c| c.set(RAND_BUFF_SIZE)); // c:145
    }
    let new_cnt = BUF_CNT.with(|c| c.get()) - 1; // c:145
    BUF_CNT.with(|c| c.set(new_cnt));
    RAND_BUFF.with(|r| r.borrow()[new_cnt]) // c:145
}

/// `math_zrand_int(upper, lower, inclusive)` math function.
/// Port of `math_zrand_int(UNUSED(char *name), int argc, mnumber *argv, UNUSED(int id))` from Src/Modules/random.c:161 — the
/// C source's math-function entry point exposed to `${(( ... ))}`.
/// All three arguments are optional; behaviour matches the C
/// source's bound-checks (`lower < 0`, `upper < lower`, etc.).
/// WARNING: param names don't match C — Rust=(upper, lower, inclusive) vs C=(name, argc, argv, id)
pub fn math_zrand_int(
    upper: Option<i64>,
    lower: Option<i64>,
    inclusive: bool,
) -> Result<i64, String> {
    // c:161
    let lower = lower.unwrap_or(0);
    let upper = upper.unwrap_or(u32::MAX as i64);

    // c:179-185 — `lower < 0 || lower >= UINT32_MAX` — note `>=` not `>`.
    // C's check disallows lower=UINT32_MAX (4294967295) because the
    // subsequent `diff = upper - lower + incl` calc would push diff
    // into the carry-bit and wrap to 0 under uint32 arithmetic — the
    // `if (diff == 0)` branch at c:187 would then return upper as a
    // non-random fixed value. Prior Rust port used `> u32::MAX as i64`
    // (a `>` not `>=`), allowing the out-of-range case and silently
    // returning `upper` (non-random) when inclusive=true and upper
    // pegged at UINT32_MAX.
    if lower < 0 || lower >= u32::MAX as i64 {
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

    // c:183 — `upper < 0 || upper >= UINT32_MAX` — same `>=` semantic as
    // the lower check at c:179, for the same overflow-on-diff reason.
    if upper < 0 || upper >= u32::MAX as i64 {
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
///
/// C body (verbatim):
///   r = random_real();
///   if (r < 0) {
///       zwarnnam(name, "Failed to get sufficient random data.");
///   }
///   ret.type = MN_FLOAT;
///   ret.u.d = r;
///   return ret;
///
/// `random_real` returns -1 (via the thread_local sentinel set in
/// random_real.rs) when the entropy syscall fails. C warns then still
/// returns -1 as the math-function result; prior Rust port skipped
/// the warn so callers had no signal that `[[ $((zrand_float())) ]]`
/// produced a sentinel instead of a real probability.
/// WARNING: param names don't match C — Rust=() vs C=(name, argc, argv, id)
pub fn math_zrand_float() -> f64 {
    // c:204
    let r = random_real(); // c:210
    if r < 0.0 {
        // c:211
        crate::ported::utils::zwarnnam(
            "zrand_float", // c:212 — C uses `name`, the math-function name.
            "Failed to get sufficient random data.",
        );
    }
    r // c:215
}

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/random.c:243`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:243
    // c:243-261 — USE_URANDOM block: stat /dev/urandom; verify
    //              S_ISCHR. We probe via std::fs::metadata + file_type().
    match metadata("/dev/urandom") {
        // c:251
        Ok(md) => {
            if !md.file_type().is_char_device() {
                // c:256
                // c:257 — `zwarn("Error getting kernel random pool: %m");`
                zwarn("Error getting kernel random pool: not a char device");
                return 1;
            }
        }
        Err(e) => {
            zwarn(&format!("Error getting kernel random pool: {}", e));
            return 1;
        }
    }
    0 // c:275
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/random.c:267`.
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    // c:267
    *features = featuresarray(m, module_features());
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/random.c:275`.
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    // c:275
    handlefeatures(m, module_features(), enables)
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/random.c:282-308`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:282
    // c:296 — `if ((tmpfd = open("/dev/urandom", O_RDONLY)) < 0)`
    let f = match std::fs::OpenOptions::new().read(true).open("/dev/urandom") {
        Ok(f) => f,
        Err(e) => {
            // c:297 — `zwarn("Could not access kernel random pool: %e.", errno);`
            zwarn(&format!("Could not access kernel random pool: {}", e));
            return 1; // c:298
        }
    };
    // c:300 — `randfd = movefd(tmpfd);` — relocate to a high fd so the
    // urandom handle doesn't clash with shell-side fd 3-9 use. Prior
    // port skipped movefd, so randfd typically landed at fd 3 or 4 —
    // colliding with redirect-save slots from `exec 3>file` style
    // user commands. The shell's redirect machinery would then
    // dup-over the urandom fd, leaving random_real reading from
    // whatever the user redirected.
    let tmpfd = f.into_raw_fd(); // c:293 `int tmpfd = -1`
    let fd = crate::ported::utils::movefd(tmpfd);
    // c:301 — `addmodulefd(randfd, FDT_MODULE);` — register the urandom
    // fd in the global fdtable as FDT_MODULE so closem and exec
    // redirect-save recognize it as module-owned. Same fix as the
    // db_gdbm addmodulefd port (c0dad2eb83) but with FDT_MODULE
    // (matches C: random.c uses FDT_MODULE, db_gdbm.c uses FDT_INTERNAL).
    crate::ported::utils::addmodulefd(fd, crate::ported::zsh_h::FDT_MODULE);
    // c:302-305 — `if (randfd < 0) { zwarn(...); return 1; }` — movefd
    // failure (out of fd slots). Rust movefd returns -1 on failure
    // matching C.
    if fd < 0 {
        zwarn("Could not access kernel random pool.");
        return 1;
    }
    RANDFD.store(fd, Ordering::SeqCst);
    0 // c:307
}

/// Re-export of the canonical `random_real()` from
/// `Src/Modules/random_real.c:147` — Campbell's algorithm for
/// distribution-correct uniform doubles in `[0, 1)`. The simpler
/// "53-bit mantissa" approximation that previously lived here was
/// removed because it biases ~3% of the interval; the C author
/// (Taylor R. Campbell) explicitly warns against it in the random_real.c
/// header comment.

/// Generate a random integer in `[min, max]`.
// =====================================================================
// static struct features module_features                            c:255 (random.c)
// =====================================================================

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/random.c:312`.
pub fn cleanup_(m: *const module) -> i32 {
    // c:312
    setfeatureenables(m, module_features(), None)
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/random.c:319-326`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:319
    // c:322-323 — `if (randfd >= 0) zclose(randfd);`
    let fd = RANDFD.swap(-1, Ordering::SeqCst);
    if fd >= 0 {
        // Clear the fdtable entry BEFORE close so the post-close
        // kernel-reuse of this fd number doesn't inherit the
        // FDT_MODULE marker we set in boot_ (port c0dad2eb83 +
        // c3a5125d9f pattern). C's zclose internally calls
        // fdtable_set(fd, FDT_UNUSED) so the canonical path is safe;
        // the raw libc::close below skips that step.
        crate::ported::utils::fdtable_set(fd, crate::ported::zsh_h::FDT_UNUSED);
        unsafe { libc::close(fd) }; // c:323 zclose
    }
    0 // c:325
}

/// Buffer size for pre-loading random integers
// buffer to pre-load integers for SRANDOM to lessen the context switches  // c:49
const RAND_BUFF_SIZE: usize = 8;

// `mftab` — port of `static struct mathfunc mftab[]` (random.c).

// `patab` — port of `static struct paramdef patab[]` (random.c).

// `module_features` — port of `static struct features module_features`
// from random.c:255.

/// `RANDFD` — port of the file-static `int randfd` in
/// `Src/Modules/random.c:243`. Holds the open fd for `/dev/urandom`.
/// Set in `boot_()`, closed in `finish_()`.
pub static RANDFD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:34

// WARNING: NOT IN RANDOM.C — Rust-only convenience helpers.
// C inlines the equivalent logic inside `get_bound_random_buffer()`
// (Src/Modules/random.c:104) and the math-fn wrappers; the Rust
// port factors them out because callers in this same file (the
// Fisher-Yates shuffle in `bin_zshuffle`, the math ported
// `math_zrand_int`/`math_zrand_real`, the `get_bound_random_buffer`
// loop) would each inline identical 4-line getrandom-and-decode
// blocks. Names are Rust-original; renaming to match a C name
// would mislead.

/// WARNING: NOT IN RANDOM.C — one-shot u32 helper; C inlines the 4-byte getrandom_buffer read
/// (equivalent C logic at Src/Modules/random.c:79).
/// One-shot u32 read. C inlines the equivalent at random.c:79
/// (4-byte getrandom_buffer + decode).
pub fn random_u32() -> u32 {
    let mut buf = [0u8; 4];
    let _ = getrandom_buffer(&mut buf);
    u32::from_ne_bytes(buf)
}

/// WARNING: NOT IN RANDOM.C — two-word helper for random_real(); C inlines the 8-byte read
/// (equivalent C logic at Src/Modules/random_real.c:158).
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

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN RANDOM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "f:zrand_float".to_string(),
        "f:zrand_int".to_string(),
        "p:SRANDOM".to_string(),
    ]
}

// WARNING: NOT IN RANDOM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 3]);
    }
    0
}

// WARNING: NOT IN RANDOM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn setfeatureenables(_m: *const module, _f: &Mutex<features>, _e: Option<&[i32]>) -> i32 {
    0
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// WARNING: NOT IN RANDOM.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 0,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 2,
            pd_list: None,
            pd_size: 1,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_random_state() {
        let _g = crate::test_util::global_state_lock();

        let r1 = get_srandom();
        let r2 = get_srandom();
        let r3 = get_srandom();
        assert!(r1 != r2 || r2 != r3);
    }

    #[test]
    fn test_get_random_u32() {
        let _g = crate::test_util::global_state_lock();
        let r1 = random_u32();
        let r2 = random_u32();
        let r3 = random_u32();
        assert!(r1 != r2 || r2 != r3);
    }

    #[test]
    fn test_get_random_u64() {
        let _g = crate::test_util::global_state_lock();
        let r1 = random_u64();
        let r2 = random_u64();
        assert_ne!(r1, r2);
    }

    #[test]
    fn test_bounded_random() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let r = bounded(10);
            assert!(r < 10);
        }
    }

    #[test]
    fn test_bounded_random_one() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            let r = bounded(1);
            assert_eq!(r, 0);
        }
    }

    #[test]
    fn test_zrand_int() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(Some(100), Some(50), false).unwrap();
        assert!((50..100).contains(&r));

        let r = math_zrand_int(Some(100), Some(50), true).unwrap();
        assert!((50..=100).contains(&r));
    }

    #[test]
    fn test_zrand_int_no_args() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(None, None, false).unwrap();
        assert!(r >= 0);
    }

    #[test]
    fn test_zrand_int_errors() {
        let _g = crate::test_util::global_state_lock();
        assert!(math_zrand_int(Some(50), Some(100), false).is_err());
        assert!(math_zrand_int(Some(-1), None, false).is_err());
    }

    #[test]
    fn test_zrand_float() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let r = math_zrand_float();
            assert!((0.0..1.0).contains(&r));
        }
    }

    #[test]
    fn test_random_real() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..100 {
            let r = random_real();
            assert!((0.0..1.0).contains(&r));
        }
    }

    #[test]
    fn test_shuffle() {
        let _g = crate::test_util::global_state_lock();
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

    #[test]
    fn test_fill_random_bytes() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 32];
        getrandom_buffer(&mut buf).unwrap();
        assert!(!buf.iter().all(|&b| b == 0));
    }

    /// c:161 — `math_zrand_int(upper, lower, inclusive=true)` returns
    /// values in `[lower, upper]`. 50 iterations to verify EVERY
    /// returned value lies in range — regression returning out-of-
    /// bounds values would silently corrupt arithmetic-driven scripts.
    #[test]
    fn math_zrand_int_inclusive_range_respects_bounds() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let v = math_zrand_int(Some(10), Some(5), true).unwrap();
            assert!(
                (5..=10).contains(&v),
                "value {v} out of inclusive range [5, 10]"
            );
        }
    }

    /// c:161 — `inclusive=false` excludes upper bound. `random_int(0,10)`
    /// must NEVER return 10 in this mode.
    #[test]
    fn math_zrand_int_exclusive_excludes_upper_bound() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let v = math_zrand_int(Some(10), Some(5), false).unwrap();
            assert!(
                (5..10).contains(&v),
                "value {v} out of exclusive range [5, 10)"
            );
        }
    }

    /// c:204 — `math_zrand_float` returns a float in `[0.0, 1.0)`.
    /// Regression returning negative or >=1.0 would break PRNGs users
    /// layer on top.
    #[test]
    fn math_zrand_float_in_unit_interval() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let v = math_zrand_float();
            assert!(
                (0.0..1.0).contains(&v),
                "value {v} out of unit interval [0.0, 1.0)"
            );
        }
    }

    /// `getrandom_buffer` produces different output on successive
    /// calls. Catches a regression where the RNG is fixed-seeded.
    #[test]
    fn getrandom_buffer_two_calls_differ() {
        let _g = crate::test_util::global_state_lock();
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        getrandom_buffer(&mut a).unwrap();
        getrandom_buffer(&mut b).unwrap();
        assert_ne!(a, b, "two random reads must differ (or RNG is broken)");
    }

    // ─── zsh-corpus pins for random helpers ────────────────────────

    /// `get_srandom` returns u32 with variance across calls.
    #[test]
    fn random_corpus_get_srandom_varies() {
        let _g = crate::test_util::global_state_lock();
        let a = get_srandom();
        let b = get_srandom();
        let c = get_srandom();
        assert!(
            a != b || b != c || a != c,
            "3 srandom calls all returned same value: {a} {b} {c}"
        );
    }

    /// `math_zrand_float` always in [0.0, 1.0).
    #[test]
    fn random_corpus_math_zrand_float_unit_interval() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let v = math_zrand_float();
            assert!((0.0..1.0).contains(&v), "value {v} out of [0.0, 1.0)");
        }
    }

    /// `get_bound_random_buffer` with max=100 fills with values < 100.
    #[test]
    fn random_corpus_bound_random_under_max() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u32; 50];
        get_bound_random_buffer(&mut buf, 100);
        for &v in &buf {
            assert!(v < 100, "{v} should be < 100");
        }
    }

    /// `get_bound_random_buffer` with max=1 fills with all zeros.
    #[test]
    fn random_corpus_bound_random_max_one_all_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u32; 20];
        get_bound_random_buffer(&mut buf, 1);
        for &v in &buf {
            assert_eq!(v, 0, "max=1 → all values 0, got {v}");
        }
    }

    /// `getrandom_buffer` with empty slice doesn't panic.
    #[test]
    fn random_corpus_getrandom_empty_buffer_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut empty: [u8; 0] = [];
        getrandom_buffer(&mut empty).unwrap();
    }

    /// `getrandom_buffer` fills 1-byte buffer.
    #[test]
    fn random_corpus_getrandom_single_byte() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 1];
        getrandom_buffer(&mut buf).unwrap();
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random.c.
    // ═══════════════════════════════════════════════════════════════════

    /// c:161 — `math_zrand_int(None, None, false)` returns Ok value in
    /// [0, u32::MAX) range (defaults: lower=0, upper=u32::MAX, exclusive).
    #[test]
    fn math_zrand_int_default_bounds_returns_ok() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(None, None, false).expect("default bounds → Ok");
        assert!(r >= 0, "result must be ≥ 0");
        assert!(r <= u32::MAX as i64, "result must fit in u32 range");
    }

    /// c:161 — `lower < 0` returns Err.
    #[test]
    fn math_zrand_int_negative_lower_returns_err() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(Some(100), Some(-1), false);
        assert!(r.is_err(), "negative lower must error");
        let msg = r.unwrap_err();
        assert!(msg.contains("Lower bound"), "error mentions Lower bound");
    }

    /// c:161 — `upper > u32::MAX` returns Err on lower-check (since
    /// lower defaults check runs first, but upper validation is also
    /// pinned).
    #[test]
    fn math_zrand_int_lower_above_u32_max_returns_err() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(Some(100), Some((u32::MAX as i64) + 1), false);
        assert!(r.is_err(), "lower > u32::MAX must error");
    }

    /// c:161 — `upper < lower` returns Err with "must be greater" msg.
    #[test]
    fn math_zrand_int_upper_below_lower_returns_err() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(Some(5), Some(10), false);
        assert!(r.is_err());
        let msg = r.unwrap_err();
        assert!(
            msg.contains("greater"),
            "msg mentions 'greater', got: {}",
            msg
        );
    }

    /// c:161 — `upper == lower` with exclusive returns the bound
    /// (diff=0 short-circuit branch).
    #[test]
    fn math_zrand_int_upper_equals_lower_returns_bound() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(Some(7), Some(7), false).expect("equal bounds OK");
        assert_eq!(r, 7, "diff=0 → returns upper");
    }

    /// c:161 — `inclusive=true` with same lower=upper returns lower
    /// (diff = 0 - 0 + 1 = 1, range [lower, upper]).
    #[test]
    fn math_zrand_int_inclusive_single_point_returns_lower() {
        let _g = crate::test_util::global_state_lock();
        let r = math_zrand_int(Some(42), Some(42), true).expect("OK");
        assert_eq!(r, 42, "inclusive single-point → that point");
    }

    /// c:161 — result always within [lower, upper] (or upper-1 if exclusive).
    #[test]
    fn math_zrand_int_result_in_range() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let r = math_zrand_int(Some(10), Some(0), false).unwrap();
            assert!(r >= 0 && r < 10, "exclusive [0,10): got {}", r);
        }
        for _ in 0..50 {
            let r = math_zrand_int(Some(10), Some(0), true).unwrap();
            assert!(r >= 0 && r <= 10, "inclusive [0,10]: got {}", r);
        }
    }

    /// c:204 — `math_zrand_float()` returns value in [0.0, 1.0).
    #[test]
    fn math_zrand_float_in_zero_one_range() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let r = math_zrand_float();
            assert!(r >= 0.0 && r < 1.0, "must be in [0,1): got {}", r);
        }
    }

    /// c:204 — `math_zrand_float` is not always the same value
    /// (basic randomness sanity — 100 calls should produce ≥ 2
    /// distinct values).
    #[test]
    fn math_zrand_float_produces_varied_output() {
        let _g = crate::test_util::global_state_lock();
        let first = math_zrand_float();
        let any_different = (0..100).any(|_| math_zrand_float() != first);
        assert!(
            any_different,
            "100 calls should produce ≥ 1 different value"
        );
    }

    /// c:58 — `get_srandom()` returns u32, repeated calls produce
    /// varied values (PRNG behavior pin).
    #[test]
    fn get_srandom_produces_varied_values() {
        let _g = crate::test_util::global_state_lock();
        let first = get_srandom();
        let any_different = (0..100).any(|_| get_srandom() != first);
        assert!(
            any_different,
            "100 calls should produce ≥ 1 different value"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random.c
    // c:26 getrandom_buffer / c:109 get_bound_random_buffer / c:144 get_srandom
    // c:173 math_zrand_int / c:219 math_zrand_float / lifecycle
    // ═══════════════════════════════════════════════════════════════════

    /// c:26 — `getrandom_buffer(&mut [])` empty buf no-panic.
    #[test]
    fn getrandom_buffer_empty_buffer_returns_ok() {
        let _g = crate::test_util::global_state_lock();
        let mut buf: [u8; 0] = [];
        assert!(getrandom_buffer(&mut buf).is_ok());
    }

    /// c:26 — `getrandom_buffer` returns Result type.
    #[test]
    fn getrandom_buffer_returns_io_result_type() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 1];
        let _: io::Result<()> = getrandom_buffer(&mut buf);
    }

    /// c:26 — `getrandom_buffer` actually fills (high probability of
    /// at least one non-zero byte in 256 bytes).
    #[test]
    fn getrandom_buffer_fills_with_nonzero_bytes() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 256];
        getrandom_buffer(&mut buf).unwrap();
        assert!(
            buf.iter().any(|&b| b != 0),
            "256 random bytes should have ≥ 1 non-zero"
        );
    }

    /// c:109 — `get_bound_random_buffer(buf, 1)` fills all zeros (only
    /// value < 1 is 0).
    #[test]
    fn get_bound_random_buffer_max_one_all_zero() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u32; 100];
        get_bound_random_buffer(&mut buf, 1);
        for &v in &buf {
            assert_eq!(v, 0, "max=1 → all values must be 0");
        }
    }

    /// c:109 — `get_bound_random_buffer(buf, max)` all values < max.
    #[test]
    fn get_bound_random_buffer_respects_max_bound() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u32; 1000];
        let max = 100u32;
        get_bound_random_buffer(&mut buf, max);
        for &v in &buf {
            assert!(v < max, "value {} must be < max {}", v, max);
        }
    }

    /// c:144 — `get_srandom` returns u32 (compile-time type pin).
    #[test]
    fn get_srandom_returns_u32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: u32 = get_srandom();
    }

    /// c:219 — `math_zrand_float()` returns f64 strictly in [0.0, 1.0).
    #[test]
    fn math_zrand_float_strictly_in_half_open_unit() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let v = math_zrand_float();
            assert!(
                v >= 0.0 && v < 1.0,
                "math_zrand_float = {} must be in [0.0, 1.0)",
                v
            );
        }
    }

    /// c:226-303 — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn random_full_lifecycle_returns_zero_for_all() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        assert_eq!(setup_(null), 0);
        let mut feats = Vec::new();
        let _ = features_(null, &mut feats);
        let mut enables: Option<Vec<i32>> = None;
        let _ = enables_(null, &mut enables);
        assert_eq!(boot_(null), 0);
        assert_eq!(cleanup_(null), 0);
        assert_eq!(finish_(null), 0);
    }

    /// c:343 — `random_u32` produces varied values.
    #[test]
    fn random_u32_produces_varied_values() {
        let _g = crate::test_util::global_state_lock();
        let first = random_u32();
        let any_diff = (0..100).any(|_| random_u32() != first);
        assert!(any_diff, "100 u32 randoms should differ from first");
    }

    /// c:353 — `random_u64` produces varied values.
    #[test]
    fn random_u64_produces_varied_values() {
        let _g = crate::test_util::global_state_lock();
        let first = random_u64();
        let any_diff = (0..100).any(|_| random_u64() != first);
        assert!(any_diff, "100 u64 randoms should differ from first");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random.c
    // c:343 random_u32 / c:353 random_u64 / c:364 bounded /
    // c:144 get_srandom + lifecycle type pins
    // ═══════════════════════════════════════════════════════════════════

    /// c:343 — `random_u32` returns u32 (compile-time type pin).
    #[test]
    fn random_u32_returns_u32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: u32 = random_u32();
    }

    /// c:353 — `random_u64` returns u64 (compile-time type pin).
    #[test]
    fn random_u64_returns_u64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: u64 = random_u64();
    }

    /// c:364 — `bounded(0)` returns 0 (degenerate range).
    #[test]
    fn bounded_zero_max_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(bounded(0), 0, "max=0 always returns 0");
    }

    /// c:364 — `bounded(1)` always returns 0 (only value < 1).
    #[test]
    fn bounded_one_max_always_zero() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            assert_eq!(bounded(1), 0, "max=1 → result must be 0");
        }
    }

    /// c:364 — `bounded(max)` result always strictly less than max.
    #[test]
    fn bounded_result_strictly_less_than_max() {
        let _g = crate::test_util::global_state_lock();
        for &max in &[2u32, 10, 100, 1000, 1_000_000] {
            for _ in 0..20 {
                let v = bounded(max);
                assert!(v < max, "bounded({}) = {} must be < max", max, v);
            }
        }
    }

    /// c:364 — `bounded` returns u32 (compile-time type pin).
    #[test]
    fn bounded_returns_u32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: u32 = bounded(100);
    }

    /// c:364 — `bounded(u32::MAX)` short-circuit branch returns
    /// arbitrary u32 (degenerate special case at c:368).
    #[test]
    fn bounded_u32_max_returns_u32_range() {
        let _g = crate::test_util::global_state_lock();
        // No bound to check beyond "doesn't panic" + type.
        let _: u32 = bounded(u32::MAX);
    }

    /// c:226 — `setup_` returns i32 (compile-time type pin).
    #[test]
    fn random_setup_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = setup_(std::ptr::null());
    }

    /// c:249 — features list contains the canonical 3 entries
    /// (f:zrand_float, f:zrand_int, p:SRANDOM).
    #[test]
    fn random_features_canonical_three_entries() {
        let _g = crate::test_util::global_state_lock();
        let mut feats = Vec::new();
        features_(std::ptr::null(), &mut feats);
        assert_eq!(feats.len(), 3, "random advertises 3 features");
        assert!(
            feats.iter().any(|f| f == "f:zrand_float"),
            "must contain f:zrand_float"
        );
        assert!(
            feats.iter().any(|f| f == "f:zrand_int"),
            "must contain f:zrand_int"
        );
        assert!(
            feats.iter().any(|f| f == "p:SRANDOM"),
            "must contain p:SRANDOM"
        );
    }

    /// c:296 — `cleanup_` idempotent.
    #[test]
    fn random_cleanup_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(cleanup_(std::ptr::null()), 0);
        }
    }

    /// c:303 — `finish_` idempotent.
    #[test]
    fn random_finish_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(finish_(std::ptr::null()), 0);
        }
    }

    /// c:263 — `boot_` idempotent.
    #[test]
    fn random_boot_idempotent() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(boot_(std::ptr::null()), 0);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/random.c
    // c:144 get_srandom / c:219 math_zrand_float / c:343 random_u32 /
    // c:353 random_u64 / c:364 bounded / c:109 get_bound_random_buffer
    // ═══════════════════════════════════════════════════════════════════

    /// c:144 — `get_srandom` returns u32 (compile-time pin, alt).
    #[test]
    fn get_srandom_returns_u32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: u32 = get_srandom();
    }

    /// c:144 — `get_srandom` is non-deterministic across two calls
    /// (probability of equal = 1/2^32 ≈ 2.3e-10 — cosmically unlikely).
    #[test]
    fn get_srandom_two_calls_differ() {
        let _g = crate::test_util::global_state_lock();
        let a = get_srandom();
        let b = get_srandom();
        assert_ne!(a, b, "two get_srandom() calls must differ");
    }

    /// c:343 — `random_u32` returns u32 (compile-time pin, alt).
    #[test]
    fn random_u32_returns_u32_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: u32 = random_u32();
    }

    /// c:353 — `random_u64` returns u64 (compile-time pin, alt).
    #[test]
    fn random_u64_returns_u64_pin_alt() {
        let _g = crate::test_util::global_state_lock();
        let _: u64 = random_u64();
    }

    /// c:353 — `random_u64` eventually exceeds u32::MAX threshold
    /// (proves it's a full 64-bit value, not a u32 zero-extended).
    #[test]
    fn random_u64_eventually_exceeds_u32_max() {
        let _g = crate::test_util::global_state_lock();
        let any_large = (0..200).any(|_| random_u64() > (u32::MAX as u64));
        assert!(
            any_large,
            "200 random_u64 values must include ≥ 1 above u32::MAX"
        );
    }

    /// c:219 — `math_zrand_float` returns f64 (compile-time pin).
    #[test]
    fn math_zrand_float_returns_f64_type() {
        let _g = crate::test_util::global_state_lock();
        let _: f64 = math_zrand_float();
    }

    /// c:219 — `math_zrand_float` outputs always finite (no NaN/Inf).
    #[test]
    fn math_zrand_float_always_finite() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..500 {
            let v = math_zrand_float();
            assert!(
                v.is_finite(),
                "math_zrand_float must always be finite, got {}",
                v
            );
        }
    }

    /// c:364 — `bounded(1)` always returns 0 (only one valid value).
    #[test]
    fn bounded_one_always_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            assert_eq!(
                bounded(1),
                0,
                "bounded(1) must always return 0 (only valid value)"
            );
        }
    }

    /// c:364 — `bounded(2)` returns only 0 or 1.
    #[test]
    fn bounded_two_returns_zero_or_one() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..50 {
            let v = bounded(2);
            assert!(v < 2, "bounded(2) must be 0 or 1; got {}", v);
        }
    }

    /// c:109 — `get_bound_random_buffer` fills entire buffer with
    /// values strictly less than max.
    #[test]
    fn get_bound_random_buffer_all_under_max() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = vec![0u32; 100];
        get_bound_random_buffer(&mut buf, 10);
        for &v in &buf {
            assert!(v < 10, "buffer value {} must be < max=10", v);
        }
    }

    /// c:226/249/256/263/296/303 — each lifecycle hook returns 0 individually
    /// (tighter failure resolution).
    #[test]
    fn random_each_lifecycle_hook_returns_zero_individually() {
        let _g = crate::test_util::global_state_lock();
        let null = std::ptr::null();
        let mut v: Vec<String> = Vec::new();
        let mut e: Option<Vec<i32>> = None;
        assert_eq!(setup_(null), 0, "c:226 setup_");
        assert_eq!(features_(null, &mut v), 0, "c:249 features_");
        assert_eq!(enables_(null, &mut e), 0, "c:256 enables_");
        assert_eq!(boot_(null), 0, "c:263 boot_");
        assert_eq!(cleanup_(null), 0, "c:296 cleanup_");
        assert_eq!(finish_(null), 0, "c:303 finish_");
    }
}
