//! The platform libm's `cbrt`, not Rust's.
//!
//! zsh/mathfunc's `cbrt()` is the C library's (`c:Src/Modules/mathfunc.c:237`
//! `retd = cbrt(argd);`). A Rust binary on Linux also links
//! `compiler_builtins`, whose own `cbrt` (a port of musl's, correctly
//! rounded) is a local symbol the static link resolves first, so an
//! `extern "C" fn cbrt` never reaches glibc's. The two disagree:
//! glibc's `cbrt(27.0)` is `3.0000000000000004`, which zsh prints, while
//! the builtin gives `3`.
//!
//! The builtin copy is not in the dynamic symbol table, so looking the
//! name up in the global scope at runtime finds libm's. macOS links no
//! such builtin: its `cbrt` already is the system one.

#[cfg(target_os = "linux")]
use std::sync::OnceLock;

extern "C" {
    #[link_name = "cbrt"]
    fn linked_cbrt(x: f64) -> f64;
}

/// `cbrt(x)` from the C library zsh links against.
pub fn cbrt(x: f64) -> f64 {
    #[cfg(target_os = "linux")]
    {
        type CbrtFn = unsafe extern "C" fn(f64) -> f64;
        static LIBM_CBRT: OnceLock<Option<CbrtFn>> = OnceLock::new();
        let f = LIBM_CBRT.get_or_init(|| {
            // SAFETY: dlsym with a NUL-terminated name; a non-null result
            // is libm's `double cbrt(double)`.
            let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"cbrt".as_ptr()) };
            if p.is_null() {
                None
            } else {
                Some(unsafe { std::mem::transmute::<*mut libc::c_void, CbrtFn>(p) })
            }
        });
        if let Some(f) = f {
            return unsafe { f(x) };
        }
    }
    unsafe { linked_cbrt(x) }
}
