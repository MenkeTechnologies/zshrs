//! One C/POSIX-locale codec on every platform.
//!
//! zsh hands every multibyte decision to libc's `mbrtowc`/`wcrtomb`, and
//! the platforms disagree about the C locale, whose codeset is ASCII
//! (`nl_langinfo(CODESET)` is `ANSI_X3.4-1968` on glibc and `US-ASCII` on
//! macOS):
//!
//! * glibc rejects every byte above 0x7f with `EILSEQ`, so zsh takes its
//!   `MB_INVALID` arms and prints such a byte as `\M-c`
//!   (`c:Src/utils.c:5396-5403`);
//! * macOS returns 1 and hands back the byte value as the wide character,
//!   so the same zsh prints 0xe3 raw.
//!
//! zsh's own test suite expects the glibc output (`E02xtrace.ztst`:
//! `LC_ALL=C which ヌ` prints `$'\M-c\M-\C-C\M-\C-L'`), and zshrs has to
//! print the same thing on every platform. So under an ASCII codeset these
//! wrappers reject what ASCII cannot hold: a byte above 0x7f does not decode
//! and a wide character above 0x7f does not encode. Every other locale
//! (UTF-8, ISO-8859-*, …) goes to libc untouched.
//!
//! `crate::ported::utils` re-exports both functions under the libc names,
//! so every ported call site gets the same answer without restating it.

use std::ffi::CStr;

extern "C" {
    #[link_name = "mbrtowc"]
    fn libc_mbrtowc(
        pwc: *mut libc::wchar_t,
        s: *const libc::c_char,
        n: libc::size_t,
        ps: *mut libc::c_void,
    ) -> libc::size_t;

    #[link_name = "wcrtomb"]
    fn libc_wcrtomb(s: *mut libc::c_char, wc: libc::wchar_t, ps: *mut libc::c_void)
        -> libc::size_t;
}

/// `(size_t)-1` — libc's "invalid sequence" result (zsh's `MB_INVALID`).
const MB_INVALID: libc::size_t = libc::size_t::MAX;

/// libc sets `errno = EILSEQ` alongside `(size_t)-1`; do the same.
fn set_eilseq() {
    // SAFETY: the errno accessors return this thread's errno slot.
    #[cfg(target_vendor = "apple")]
    unsafe {
        *libc::__error() = libc::EILSEQ;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location() = libc::EILSEQ;
    }
}

/// True when the current `LC_CTYPE` codeset is ASCII — the C/POSIX locale.
/// Asked per call, because `LC_ALL=C cmd` and `LC_CTYPE=…` assignments
/// change the locale at run time.
pub fn codeset_is_ascii() -> bool {
    // SAFETY: nl_langinfo returns a pointer to a NUL-terminated static
    // string (or NULL), valid until the next setlocale on this thread.
    let cs = unsafe { libc::nl_langinfo(libc::CODESET) };
    if cs.is_null() {
        return false;
    }
    let cs = unsafe { CStr::from_ptr(cs) }.to_bytes();
    cs.eq_ignore_ascii_case(b"ANSI_X3.4-1968")
        || cs.eq_ignore_ascii_case(b"US-ASCII")
        || cs.eq_ignore_ascii_case(b"ASCII")
}

/// `mbrtowc(3)`, with a byte above 0x7f rejected under an ASCII codeset.
///
/// # Safety
/// Same contract as libc `mbrtowc`: `s` readable for `n` bytes (or NULL),
/// `ps` a valid `mbstate_t` buffer.
pub unsafe fn mbrtowc(
    pwc: *mut libc::wchar_t,
    s: *const libc::c_char,
    n: libc::size_t,
    ps: *mut libc::c_void,
) -> libc::size_t {
    if !s.is_null() && n > 0 && *s as u8 > 0x7f && codeset_is_ascii() {
        set_eilseq();
        return MB_INVALID;
    }
    libc_mbrtowc(pwc, s, n, ps)
}

/// `wcrtomb(3)`, with a wide character above 0x7f rejected under an ASCII
/// codeset.
///
/// # Safety
/// Same contract as libc `wcrtomb`: `s` writable for `MB_CUR_MAX` bytes
/// (or NULL), `ps` a valid `mbstate_t` buffer.
pub unsafe fn wcrtomb(
    s: *mut libc::c_char,
    wc: libc::wchar_t,
    ps: *mut libc::c_void,
) -> libc::size_t {
    if !s.is_null() && wc as u32 > 0x7f && codeset_is_ascii() {
        set_eilseq();
        return MB_INVALID;
    }
    libc_wcrtomb(s, wc, ps)
}

/// libc `wint_t`: `unsigned int` on glibc/musl, `__int32_t` on Darwin.
#[cfg(target_vendor = "apple")]
type WintT = libc::c_int;
#[cfg(not(target_vendor = "apple"))]
type WintT = libc::c_uint;

extern "C" {
    #[link_name = "iswalnum"]
    fn libc_iswalnum(wc: WintT) -> libc::c_int;
    #[link_name = "iswalpha"]
    fn libc_iswalpha(wc: WintT) -> libc::c_int;
    #[link_name = "iswgraph"]
    fn libc_iswgraph(wc: WintT) -> libc::c_int;
}

/// `iswalnum(3)` for a non-ASCII character, the test zsh's `wcsitype` makes
/// (c:Src/utils.c:4350, 4353, 4370). Under an ASCII codeset no character above
/// 0x7f decodes, so none is alphanumeric: `LC_ALL=C; é=x` is a command word,
/// not an assignment. In every other locale the C library decides, and its
/// class is narrower than Unicode's: macOS rejects `Ⅻ`, `½`, `²` and `Ⓐ`.
pub fn iswalnum(c: char) -> bool {
    if c as u32 > 0x7f && codeset_is_ascii() {
        return false;
    }
    std::sync::LazyLock::force(&crate::ported::utils::MB_LOCALE_READY);
    // SAFETY: iswalnum reads only its argument and the thread's locale.
    unsafe { libc_iswalnum(c as WintT) != 0 }
}

/// `iswalpha(3)`, with every character above 0x7f rejected under an ASCII
/// codeset (where none of them decodes), like [`iswalnum`].
pub fn iswalpha(c: char) -> bool {
    if c as u32 > 0x7f && codeset_is_ascii() {
        return false;
    }
    std::sync::LazyLock::force(&crate::ported::utils::MB_LOCALE_READY);
    // SAFETY: iswalpha reads only its argument and the thread's locale.
    unsafe { libc_iswalpha(c as WintT) != 0 }
}

/// `iswgraph(3)`, with every character above 0x7f rejected under an ASCII
/// codeset, like [`iswalnum`].
pub fn iswgraph(c: char) -> bool {
    if c as u32 > 0x7f && codeset_is_ascii() {
        return false;
    }
    std::sync::LazyLock::force(&crate::ported::utils::MB_LOCALE_READY);
    // SAFETY: iswgraph reads only its argument and the thread's locale.
    unsafe { libc_iswgraph(c as WintT) != 0 }
}
