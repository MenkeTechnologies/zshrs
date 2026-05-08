//! Extended attributes (xattr) — port of `Src/Modules/attr.c`.
//!
//! Implements `zgetattr` / `zsetattr` / `zdelattr` / `zlistattr`.
//!
//! Structure mirrors the C source line-by-line:
//!   - `xgetxattr` / `xlistxattr` / `xsetxattr` / `xremovexattr`
//!     (attr.c:36/51/66/82) — thin wrappers over the macOS / Linux
//!     `xxxxattr(2)` ABI variants.
//!   - `bin_getattr` / `bin_setattr` / `bin_delattr` / `bin_listattr`
//!     (attr.c:97/132/149/168) — the four builtin entry points.
//!   - module entries: `setup_` / `features_` / `enables_` / `boot_`
//!     / `cleanup_` / `finish_` (attr.c:236+).

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::ffi::CString;

use crate::ported::exec::ShellExecutor;
use crate::ported::utils::zwarnnam;

#[cfg(target_os = "macos")]
const XATTR_NOFOLLOW: i32 = 0x0001;

// =====================================================================
// Port of `xgetxattr()` from Src/Modules/attr.c:36.
//
// C signature:
//   ssize_t xgetxattr(const char *path, const char *name,
//                     void *value, size_t size, int symlink);
// Rust port: same signature, returns ssize_t (`isize`). Caller
// passes a `&mut [u8]` slot for `value` and the buffer length for
// `size`. Pass `&mut []` (or any zero-length slice) to query the
// required size without filling — same as C's `value=NULL, size=0`
// idiom (attr.c:107).
// =====================================================================

/// Port of `xgetxattr()` from `Src/Modules/attr.c:36`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn xgetxattr(path: &str, name: &str, value: &mut [u8], symlink: i32) -> isize {
    let path_c = match CString::new(path) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let name_c = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let val_ptr = if value.is_empty() {
        std::ptr::null_mut()
    } else {
        value.as_mut_ptr() as *mut libc::c_void
    };
    #[cfg(target_os = "macos")]
    {
        let opts = if symlink != 0 { XATTR_NOFOLLOW } else { 0 };
        unsafe {
            libc::getxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len(), 0, opts)
        }
    }
    #[cfg(target_os = "linux")]
    {
        if symlink != 0 {
            unsafe { libc::lgetxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len()) }
        } else {
            unsafe { libc::getxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len()) }
        }
    }
}

/// Port of `xgetxattr()` from `Src/Modules/attr.c:36` — non-xattr stub.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn xgetxattr(_path: &str, _name: &str, _value: &mut [u8], _symlink: i32) -> isize {
    -1
}

// =====================================================================
// Port of `xlistxattr()` from Src/Modules/attr.c:51.
// =====================================================================

/// Port of `xlistxattr()` from `Src/Modules/attr.c:51`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn xlistxattr(path: &str, list: &mut [u8], symlink: i32) -> isize {
    let path_c = match CString::new(path) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let list_ptr = if list.is_empty() {
        std::ptr::null_mut()
    } else {
        list.as_mut_ptr() as *mut libc::c_char
    };
    #[cfg(target_os = "macos")]
    {
        let opts = if symlink != 0 { XATTR_NOFOLLOW } else { 0 };
        unsafe { libc::listxattr(path_c.as_ptr(), list_ptr, list.len(), opts) }
    }
    #[cfg(target_os = "linux")]
    {
        if symlink != 0 {
            unsafe { libc::llistxattr(path_c.as_ptr(), list_ptr, list.len()) }
        } else {
            unsafe { libc::listxattr(path_c.as_ptr(), list_ptr, list.len()) }
        }
    }
}

/// Port of `xlistxattr()` from `Src/Modules/attr.c:51` — non-xattr stub.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn xlistxattr(_path: &str, _list: &mut [u8], _symlink: i32) -> isize {
    -1
}

// =====================================================================
// Port of `xsetxattr()` from Src/Modules/attr.c:66.
// =====================================================================

/// Port of `xsetxattr()` from `Src/Modules/attr.c:66`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn xsetxattr(
    path: &str,
    name: &str,
    value: &[u8],
    flags: i32,
    symlink: i32,
) -> i32 {
    let path_c = match CString::new(path) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let name_c = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let val_ptr = value.as_ptr() as *const libc::c_void;
    #[cfg(target_os = "macos")]
    {
        // C: `flags | symlink ? XATTR_NOFOLLOW : 0` — the cast is the
        // C source's well-known operator-precedence quirk; the
        // resulting expression is `(flags | symlink) ? XATTR_NOFOLLOW
        // : 0`. We mirror it byte-for-byte.
        let combined = if (flags | symlink) != 0 {
            XATTR_NOFOLLOW
        } else {
            0
        };
        unsafe {
            libc::setxattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                val_ptr,
                value.len(),
                0,
                combined,
            )
        }
    }
    #[cfg(target_os = "linux")]
    {
        if symlink != 0 {
            unsafe {
                libc::lsetxattr(
                    path_c.as_ptr(),
                    name_c.as_ptr(),
                    val_ptr,
                    value.len(),
                    flags,
                )
            }
        } else {
            unsafe {
                libc::setxattr(
                    path_c.as_ptr(),
                    name_c.as_ptr(),
                    val_ptr,
                    value.len(),
                    flags,
                )
            }
        }
    }
}

/// Port of `xsetxattr()` from `Src/Modules/attr.c:66` — non-xattr stub.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn xsetxattr(
    _path: &str,
    _name: &str,
    _value: &[u8],
    _flags: i32,
    _symlink: i32,
) -> i32 {
    -1
}

// =====================================================================
// Port of `xremovexattr()` from Src/Modules/attr.c:82.
// =====================================================================

/// Port of `xremovexattr()` from `Src/Modules/attr.c:82`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(crate) fn xremovexattr(path: &str, name: &str, symlink: i32) -> i32 {
    let path_c = match CString::new(path) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    let name_c = match CString::new(name) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    #[cfg(target_os = "macos")]
    {
        let opts = if symlink != 0 { XATTR_NOFOLLOW } else { 0 };
        unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr(), opts) }
    }
    #[cfg(target_os = "linux")]
    {
        if symlink != 0 {
            unsafe { libc::lremovexattr(path_c.as_ptr(), name_c.as_ptr()) }
        } else {
            unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr()) }
        }
    }
}

/// Port of `xremovexattr()` from `Src/Modules/attr.c:82` — non-xattr stub.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(crate) fn xremovexattr(_path: &str, _name: &str, _symlink: i32) -> i32 {
    -1
}

// =====================================================================
// Port of `bin_getattr()` from Src/Modules/attr.c:97.
//
// C signature:
//   bin_getattr(char *nam, char **argv, Options ops, int func)
// Rust port adds `&mut ShellExecutor` so `setsparam(param, ...)` /
// `unsetparam(param)` from C lines 110/119 can be expressed as
// `s.variables.insert(...)` / `s.variables.remove(...)`.
// =====================================================================

/// Port of `bin_getattr()` from `Src/Modules/attr.c:97`.
///
/// `zgetattr [-h] file attr [param]`: read the named xattr from
/// `file`. With `param`, write the value into the named shell
/// parameter; without, print to stdout.
pub(crate) fn bin_getattr(s: &mut ShellExecutor, nam: &str, argv: &[String], symlink: i32) -> i32 {
    if argv.len() < 2 {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    let file = argv[0].as_str();
    let attr = argv[1].as_str();
    let param = argv.get(2).map(|s| s.as_str());
    let mut ret = 0;
    // C: val_len = xgetxattr(file, attr, NULL, 0, symlink);
    let val_len = xgetxattr(file, attr, &mut [], symlink);
    if val_len == 0 {
        // attr.c:108-112 — empty xattr; unset param if given.
        if let Some(p) = param {
            s.variables.remove(p);
            s.arrays.remove(p);
        }
        return 0;
    }
    let mut attr_len: isize = 0;
    if val_len > 0 {
        // C: value = zalloc(val_len+1); attr_len = xgetxattr(...);
        let mut value = vec![0u8; val_len as usize];
        attr_len = xgetxattr(file, attr, &mut value, symlink);
        if attr_len > 0 && attr_len <= val_len {
            value.truncate(attr_len as usize);
            // C: setsparam(param, metafy(...)) or printf("%s\n", value);
            let val = String::from_utf8_lossy(&value).into_owned();
            if let Some(p) = param {
                s.variables.insert(p.to_string(), val);
            } else {
                println!("{}", val);
            }
        }
    }
    if val_len < 0 || attr_len < 0 || attr_len > val_len {
        zwarnnam(nam, &format!("{}: {}", file, std::io::Error::last_os_error()));
        // C: ret = 1 + ((val_len > 0 && attr_len > val_len) || attr_len < 0);
        ret = 1 + i32::from((val_len > 0 && attr_len > val_len) || attr_len < 0);
    }
    ret
}

// =====================================================================
// Port of `bin_setattr()` from Src/Modules/attr.c:132.
// =====================================================================

/// Port of `bin_setattr()` from `Src/Modules/attr.c:132`.
///
/// `zsetattr [-h] file attr value`: write `value` to the named
/// xattr.
pub(crate) fn bin_setattr(_s: &mut ShellExecutor, nam: &str, argv: &[String], symlink: i32) -> i32 {
    if argv.len() < 3 {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    let file = argv[0].as_str();
    let attr = argv[1].as_str();
    let value = argv[2].as_bytes();
    if xsetxattr(file, attr, value, 0, symlink) != 0 {
        zwarnnam(nam, &format!("{}: {}", file, std::io::Error::last_os_error()));
        return 1;
    }
    0
}

// =====================================================================
// Port of `bin_delattr()` from Src/Modules/attr.c:149.
// =====================================================================

/// Port of `bin_delattr()` from `Src/Modules/attr.c:149`.
///
/// `zdelattr [-h] file attr...`: remove each named xattr; bail on
/// the first error.
pub(crate) fn bin_delattr(_s: &mut ShellExecutor, nam: &str, argv: &[String], symlink: i32) -> i32 {
    if argv.len() < 2 {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    let file = argv[0].as_str();
    // C: while (*++attr) — iterate argv[1..].
    for attr in &argv[1..] {
        if xremovexattr(file, attr, symlink) != 0 {
            zwarnnam(nam, &format!("{}: {}", file, std::io::Error::last_os_error()));
            return 1;
        }
    }
    0
}

// =====================================================================
// Port of `bin_listattr()` from Src/Modules/attr.c:168.
// =====================================================================

/// Port of `bin_listattr()` from `Src/Modules/attr.c:168`.
///
/// `zlistattr [-h] file [param]`: list xattr names. With `param`,
/// write the array; without, print one per line.
pub(crate) fn bin_listattr(s: &mut ShellExecutor, nam: &str, argv: &[String], symlink: i32) -> i32 {
    if argv.is_empty() {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    let file = argv[0].as_str();
    let param = argv.get(1).map(|s| s.as_str());
    let mut ret = 0;
    let val_len = xlistxattr(file, &mut [], symlink);
    if val_len == 0 {
        if let Some(p) = param {
            s.variables.remove(p);
            s.arrays.remove(p);
        }
        return 0;
    }
    let mut list_len: isize = 0;
    if val_len > 0 {
        let mut value = vec![0u8; val_len as usize];
        list_len = xlistxattr(file, &mut value, symlink);
        if list_len > 0 && list_len <= val_len {
            value.truncate(list_len as usize);
            // C walks the NUL-terminated name list (attr.c:192-205).
            let names: Vec<String> = value
                .split(|&b| b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect();
            if let Some(p) = param {
                // C: setaparam(param, array)
                s.arrays.insert(p.to_string(), names);
            } else {
                // C: while (p < &value[list_len]) printf("%s\n", p);
                for n in &names {
                    println!("{}", n);
                }
            }
        }
    }
    if val_len < 0 || list_len < 0 || list_len > val_len {
        zwarnnam(nam, &format!("{}: {}", file, std::io::Error::last_os_error()));
        // C: ret = 1 + (list_len > val_len || list_len < 0);
        ret = 1 + i32::from(list_len > val_len || list_len < 0);
    }
    ret
}

// =====================================================================
// Module entry points (attr.c:236-275).
// =====================================================================

/// Port of `setup_()` from `Src/Modules/attr.c:236`.
pub fn setup_() -> i32 {
    0
}

/// Port of `features_()` from `Src/Modules/attr.c:243`.
pub fn features_() -> i32 {
    0
}

/// Port of `enables_()` from `Src/Modules/attr.c:251`.
pub fn enables_() -> i32 {
    0
}

/// Port of `boot_()` from `Src/Modules/attr.c:258`.
pub fn boot_() -> i32 {
    0
}

/// Port of `cleanup_()` from `Src/Modules/attr.c:265`.
pub fn cleanup_() -> i32 {
    0
}

/// Port of `finish_()` from `Src/Modules/attr.c:272`.
pub fn finish_() -> i32 {
    0
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xgetxattr_nonexistent() {
        let mut buf = [0u8; 0];
        let r = xgetxattr("/nonexistent/path", "user.test", &mut buf, 0);
        assert!(r < 0);
    }

    #[test]
    fn test_xsetxattr_nonexistent() {
        let r = xsetxattr("/nonexistent/path", "user.test", b"value", 0, 0);
        assert!(r < 0);
    }

    #[test]
    fn test_xlistxattr_nonexistent() {
        let mut buf = [0u8; 0];
        let r = xlistxattr("/nonexistent/path", &mut buf, 0);
        assert!(r < 0);
    }

    #[test]
    fn test_xremovexattr_nonexistent() {
        let r = xremovexattr("/nonexistent/path", "user.test", 0);
        assert!(r < 0);
    }
}
