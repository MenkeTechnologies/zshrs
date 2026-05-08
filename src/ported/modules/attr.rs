//! Extended attributes (xattr) module - port of Modules/attr.c
//!
//! Provides zgetattr, zsetattr, zdelattr, zlistattr builtins for
//! manipulating extended file attributes.

use std::ffi::CString;
use std::io;

/// Options for xattr operations
#[derive(Debug, Default, Clone)]
pub struct XattrOptions {
    pub no_dereference: bool,
}

/// Read an extended attribute value.
/// Port of `xgetxattr()` from Src/Modules/attr.c:37 — the C source
/// abstracts the macOS / Linux / FreeBSD `xgetxattr(2)` ABI
/// differences behind a single helper. The `symlink` flag in the C
/// source maps onto our `options.no_dereference` (macOS:
/// `XATTR_NOFOLLOW`, Linux: `lgetxattr`).
#[cfg(target_os = "macos")]
pub fn xgetxattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<Vec<u8>> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let name_c = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid attr name"))?;

    let flags = if options.no_dereference {
        libc::XATTR_NOFOLLOW
    } else {
        0
    };

    let size = unsafe {
        libc::getxattr(
            path_c.as_ptr(),
            name_c.as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            flags,
        )
    };

    if size < 0 {
        return Err(io::Error::last_os_error());
    }

    if size == 0 {
        return Ok(Vec::new());
    }

    let mut buf = vec![0u8; size as usize];

    let result = unsafe {
        libc::getxattr(
            path_c.as_ptr(),
            name_c.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_void,
            size as usize,
            0,
            flags,
        )
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    buf.truncate(result as usize);
    Ok(buf)
}

/// Port of `xgetxattr()` from `Src/Modules/attr.c:37`.
#[cfg(target_os = "linux")]
pub fn xgetxattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<Vec<u8>> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let name_c = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid attr name"))?;

    let size = if options.no_dereference {
        unsafe { libc::lgetxattr(path_c.as_ptr(), name_c.as_ptr(), std::ptr::null_mut(), 0) }
    } else {
        unsafe { libc::getxattr(path_c.as_ptr(), name_c.as_ptr(), std::ptr::null_mut(), 0) }
    };

    if size < 0 {
        return Err(io::Error::last_os_error());
    }

    if size == 0 {
        return Ok(Vec::new());
    }

    let mut buf = vec![0u8; size as usize];

    let result = if options.no_dereference {
        unsafe {
            libc::lgetxattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                size as usize,
            )
        }
    } else {
        unsafe {
            libc::getxattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_void,
                size as usize,
            )
        }
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    buf.truncate(result as usize);
    Ok(buf)
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/attr.c`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xgetxattr(_path: &str, _name: &str, _options: &XattrOptions) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "xattr not supported",
    ))
}

/// Write an extended attribute value.
/// Port of `xsetxattr()` from Src/Modules/attr.c:67 — the C
/// source's wrapper over `xsetxattr(2)` / `lsetxattr(2)`.
#[cfg(target_os = "macos")]
pub fn xsetxattr(path: &str, name: &str, value: &[u8], options: &XattrOptions) -> io::Result<()> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let name_c = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid attr name"))?;

    let flags = if options.no_dereference {
        libc::XATTR_NOFOLLOW
    } else {
        0
    };

    let result = unsafe {
        libc::setxattr(
            path_c.as_ptr(),
            name_c.as_ptr(),
            value.as_ptr() as *const libc::c_void,
            value.len(),
            0,
            flags,
        )
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

/// Port of `xsetxattr()` from `Src/Modules/attr.c:67`.
#[cfg(target_os = "linux")]
pub fn xsetxattr(path: &str, name: &str, value: &[u8], options: &XattrOptions) -> io::Result<()> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let name_c = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid attr name"))?;

    let result = if options.no_dereference {
        unsafe {
            libc::lsetxattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                value.as_ptr() as *const libc::c_void,
                value.len(),
                0,
            )
        }
    } else {
        unsafe {
            libc::setxattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                value.as_ptr() as *const libc::c_void,
                value.len(),
                0,
            )
        }
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/attr.c`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xsetxattr(
    _path: &str,
    _name: &str,
    _value: &[u8],
    _options: &XattrOptions,
) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "xattr not supported",
    ))
}

/// Remove an extended attribute.
/// Port of `xremovexattr()` from Src/Modules/attr.c:83 — wrapper
/// over `xremovexattr(2)` / `lremovexattr(2)`.
#[cfg(target_os = "macos")]
pub fn xremovexattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<()> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let name_c = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid attr name"))?;

    let flags = if options.no_dereference {
        libc::XATTR_NOFOLLOW
    } else {
        0
    };

    let result = unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr(), flags) };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/attr.c`.
#[cfg(target_os = "linux")]
pub fn xremovexattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<()> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;
    let name_c = CString::new(name)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid attr name"))?;

    let result = if options.no_dereference {
        unsafe { libc::lremovexattr(path_c.as_ptr(), name_c.as_ptr()) }
    } else {
        unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr()) }
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/attr.c`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xremovexattr(_path: &str, _name: &str, _options: &XattrOptions) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "xattr not supported",
    ))
}

/// List a file's extended-attribute names.
/// Port of `xlistxattr()` from Src/Modules/attr.c:52 — wrapper
/// over `listxattr(2)` / `llistxattr(2)`. The C source returns the
/// raw NUL-terminated buffer; we parse it into a `Vec<String>`.
#[cfg(target_os = "macos")]
pub fn xlistxattr(path: &str, options: &XattrOptions) -> io::Result<Vec<String>> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;

    let flags = if options.no_dereference {
        libc::XATTR_NOFOLLOW
    } else {
        0
    };

    let size = unsafe { libc::listxattr(path_c.as_ptr(), std::ptr::null_mut(), 0, flags) };

    if size < 0 {
        return Err(io::Error::last_os_error());
    }

    if size == 0 {
        return Ok(Vec::new());
    }

    let mut buf = vec![0u8; size as usize];

    let result = unsafe {
        libc::listxattr(
            path_c.as_ptr(),
            buf.as_mut_ptr() as *mut libc::c_char,
            size as usize,
            flags,
        )
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    buf.truncate(result as usize);
    // Walk the NUL-terminated name list inline — direct port of the
    // C loop in bin_listattr (Src/Modules/attr.c:169).
    let mut names = Vec::new();
    let mut start = 0;
    for (i, &byte) in buf.iter().enumerate() {
        if byte == 0 {
            if i > start {
                names.push(String::from_utf8_lossy(&buf[start..i]).into_owned());
            }
            start = i + 1;
        }
    }
    Ok(names)
}

/// Port of `xlistxattr()` from `Src/Modules/attr.c:52`.
#[cfg(target_os = "linux")]
pub fn xlistxattr(path: &str, options: &XattrOptions) -> io::Result<Vec<String>> {
    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;

    let size = if options.no_dereference {
        unsafe { libc::llistxattr(path_c.as_ptr(), std::ptr::null_mut(), 0) }
    } else {
        unsafe { libc::listxattr(path_c.as_ptr(), std::ptr::null_mut(), 0) }
    };

    if size < 0 {
        return Err(io::Error::last_os_error());
    }

    if size == 0 {
        return Ok(Vec::new());
    }

    let mut buf = vec![0u8; size as usize];

    let result = if options.no_dereference {
        unsafe {
            libc::llistxattr(
                path_c.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_char,
                size as usize,
            )
        }
    } else {
        unsafe {
            libc::listxattr(
                path_c.as_ptr(),
                buf.as_mut_ptr() as *mut libc::c_char,
                size as usize,
            )
        }
    };

    if result < 0 {
        return Err(io::Error::last_os_error());
    }

    buf.truncate(result as usize);
    // Walk the NUL-terminated name list inline — direct port of the
    // C loop in bin_listattr (Src/Modules/attr.c:169).
    let mut names = Vec::new();
    let mut start = 0;
    for (i, &byte) in buf.iter().enumerate() {
        if byte == 0 {
            if i > start {
                names.push(String::from_utf8_lossy(&buf[start..i]).into_owned());
            }
            start = i + 1;
        }
    }
    Ok(names)
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/attr.c`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xlistxattr(_path: &str, _options: &XattrOptions) -> io::Result<Vec<String>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "xattr not supported",
    ))
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/attr.c`.

/// `zgetattr` builtin entry point.
/// Port of `bin_getattr()` from Src/Modules/attr.c:98 — calls
/// `xgetxattr()` and surfaces the value as a string. Honours
/// `-h` (no-dereference) the same way the C source does.
pub fn bin_getattr(file: &str, attr: &str, options: &XattrOptions) -> (i32, Option<String>) {
    match xgetxattr(file, attr, options) {
        Ok(value) => {
            let s = String::from_utf8_lossy(&value).into_owned();
            (0, Some(s))
        }
        Err(e) => (1, Some(format!("zgetattr: {}: {}\n", file, e))),
    }
}

/// `zsetattr` builtin entry point.
/// Port of `bin_setattr()` from Src/Modules/attr.c:133.
pub fn bin_setattr(
    file: &str,
    attr: &str,
    value: &str,
    options: &XattrOptions,
) -> (i32, String) {
    match xsetxattr(file, attr, value.as_bytes(), options) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("zsetattr: {}: {}\n", file, e)),
    }
}

/// `zdelattr` builtin entry point.
/// Port of `bin_delattr()` from Src/Modules/attr.c:150 — removes
/// each named xattr and bails on the first error, matching the C
/// source's loop.
pub fn bin_delattr(file: &str, attrs: &[&str], options: &XattrOptions) -> (i32, String) {
    for attr in attrs {
        if let Err(e) = xremovexattr(file, attr, options) {
            return (1, format!("zdelattr: {}: {}\n", file, e));
        }
    }
    (0, String::new())
}

/// `zlistattr` builtin entry point.
/// Port of `bin_listattr()` from Src/Modules/attr.c:169.
pub fn bin_listattr(file: &str, options: &XattrOptions) -> (i32, Vec<String>, String) {
    match xlistxattr(file, options) {
        Ok(attrs) => (0, attrs, String::new()),
        Err(e) => (1, Vec::new(), format!("zlistattr: {}: {}\n", file, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xattr_options_default() {
        let opts = XattrOptions::default();
        assert!(!opts.no_dereference);
    }

    #[test]
    fn test_builtin_zgetattr_nonexistent() {
        let opts = XattrOptions::default();
        let (status, _) = bin_getattr("/nonexistent/path", "user.test", &opts);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_zsetattr_nonexistent() {
        let opts = XattrOptions::default();
        let (status, _) = bin_setattr("/nonexistent/path", "user.test", "value", &opts);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_zlistattr_nonexistent() {
        let opts = XattrOptions::default();
        let (status, _, _) = bin_listattr("/nonexistent/path", &opts);
        assert_eq!(status, 1);
    }
}

/// Module loader entry — port of `setup_()` from Src/Modules/attr.c:236.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/attr.c:243.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/attr.c:251.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/attr.c:258.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/attr.c:265.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/attr.c:272.
pub fn finish_() -> i32 {
    0
}
