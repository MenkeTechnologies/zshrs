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

/// Get an extended attribute value
#[cfg(target_os = "macos")]
pub fn getxattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<Vec<u8>> {
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

#[cfg(target_os = "linux")]
pub fn getxattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<Vec<u8>> {
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn getxattr(_path: &str, _name: &str, _options: &XattrOptions) -> io::Result<Vec<u8>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "xattr not supported",
    ))
}

/// Set an extended attribute value
#[cfg(target_os = "macos")]
pub fn setxattr(path: &str, name: &str, value: &[u8], options: &XattrOptions) -> io::Result<()> {
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

#[cfg(target_os = "linux")]
pub fn setxattr(path: &str, name: &str, value: &[u8], options: &XattrOptions) -> io::Result<()> {
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn setxattr(
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

/// Remove an extended attribute
#[cfg(target_os = "macos")]
pub fn removexattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<()> {
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

#[cfg(target_os = "linux")]
pub fn removexattr(path: &str, name: &str, options: &XattrOptions) -> io::Result<()> {
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

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn removexattr(_path: &str, _name: &str, _options: &XattrOptions) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "xattr not supported",
    ))
}

/// List extended attributes
#[cfg(target_os = "macos")]
pub fn listxattr(path: &str, options: &XattrOptions) -> io::Result<Vec<String>> {
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
    parse_xattr_list(&buf)
}

#[cfg(target_os = "linux")]
pub fn listxattr(path: &str, options: &XattrOptions) -> io::Result<Vec<String>> {
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
    parse_xattr_list(&buf)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn listxattr(_path: &str, _options: &XattrOptions) -> io::Result<Vec<String>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "xattr not supported",
    ))
}

fn parse_xattr_list(buf: &[u8]) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    let mut start = 0;

    for (i, &byte) in buf.iter().enumerate() {
        if byte == 0 {
            if i > start {
                let name = String::from_utf8_lossy(&buf[start..i]).into_owned();
                names.push(name);
            }
            start = i + 1;
        }
    }

    Ok(names)
}

/// Execute zgetattr builtin
pub fn builtin_zgetattr(file: &str, attr: &str, options: &XattrOptions) -> (i32, Option<String>) {
    match getxattr(file, attr, options) {
        Ok(value) => {
            let s = String::from_utf8_lossy(&value).into_owned();
            (0, Some(s))
        }
        Err(e) => (1, Some(format!("zgetattr: {}: {}\n", file, e))),
    }
}

/// Execute zsetattr builtin
pub fn builtin_zsetattr(
    file: &str,
    attr: &str,
    value: &str,
    options: &XattrOptions,
) -> (i32, String) {
    match setxattr(file, attr, value.as_bytes(), options) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("zsetattr: {}: {}\n", file, e)),
    }
}

/// Execute zdelattr builtin
pub fn builtin_zdelattr(file: &str, attrs: &[&str], options: &XattrOptions) -> (i32, String) {
    for attr in attrs {
        if let Err(e) = removexattr(file, attr, options) {
            return (1, format!("zdelattr: {}: {}\n", file, e));
        }
    }
    (0, String::new())
}

/// Execute zlistattr builtin
pub fn builtin_zlistattr(file: &str, options: &XattrOptions) -> (i32, Vec<String>, String) {
    match listxattr(file, options) {
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
    fn test_parse_xattr_list_empty() {
        let buf: &[u8] = &[];
        let result = parse_xattr_list(buf).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_xattr_list_single() {
        let buf = b"user.test\0";
        let result = parse_xattr_list(buf).unwrap();
        assert_eq!(result, vec!["user.test"]);
    }

    #[test]
    fn test_parse_xattr_list_multiple() {
        let buf = b"user.test1\0user.test2\0user.test3\0";
        let result = parse_xattr_list(buf).unwrap();
        assert_eq!(result, vec!["user.test1", "user.test2", "user.test3"]);
    }

    #[test]
    fn test_builtin_zgetattr_nonexistent() {
        let opts = XattrOptions::default();
        let (status, _) = builtin_zgetattr("/nonexistent/path", "user.test", &opts);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_zsetattr_nonexistent() {
        let opts = XattrOptions::default();
        let (status, _) = builtin_zsetattr("/nonexistent/path", "user.test", "value", &opts);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_zlistattr_nonexistent() {
        let opts = XattrOptions::default();
        let (status, _, _) = builtin_zlistattr("/nonexistent/path", &opts);
        assert_eq!(status, 1);
    }
}
