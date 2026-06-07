//! `zsh/attr` module — port of `Src/Modules/attr.c`.
//!
//! Top-level declaration order matches C source line-by-line:
//!   - `xgetxattr(path, name, value, size, symlink)`  c:36
//!   - `xlistxattr(path, list, size, symlink)`        c:51
//!   - `xsetxattr(path, name, value, size, flags, symlink)` c:66
//!   - `xremovexattr(path, name, symlink)`            c:82
//!   - `bin_getattr(nam, argv, ops, func)`            c:97
//!   - `bin_setattr(nam, argv, ops, func)`            c:132
//!   - `bin_delattr(nam, argv, ops, func)`            c:149
//!   - `bin_listattr(nam, argv, ops, func)`           c:168
//!   - `static struct builtin bintab[]`               c:219
//!   - `static struct features module_features`       c:226
//!   - `setup_(m)` / `features_(m, features)` /
//!     `enables_(m, enables)` / `boot_(m)` /
//!     `cleanup_(m)` / `finish_(m)`                   c:235-275

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use crate::params::setsparam;
use crate::ported::utils::{metafy, unmetafy, zwarnnam};
use crate::ported::zsh_h::{features, module, options, MAX_OPS, OPT_ISSET};
use std::ffi::CString;
use std::sync::{Mutex, OnceLock};

// =====================================================================
// xgetxattr(const char *path, const char *name, void *value, size_t size, int symlink)  c:36
// =====================================================================

/// Port of `xgetxattr(const char *path, const char *name, void *value, size_t size, int symlink)` from `Src/Modules/attr.c:37`.
///
/// Caller passes a `&mut [u8]` slot for `value` and the buffer length
/// for `size`. Empty slice queries required size — same as C
/// `value=NULL, size=0` (attr.c:107).
#[cfg(any(target_os = "macos", target_os = "linux"))]
/// WARNING: param names don't match C — Rust=(path, name, value, symlink) vs C=(path, name, value, size, symlink)
pub fn xgetxattr(path: &str, name: &str, value: &mut [u8], symlink: i32) -> isize {
    // c:37
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
        // c:40 — `return getxattr(path, name, value, size, 0, symlink ? XATTR_NOFOLLOW: 0);`
        unsafe {
            libc::getxattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                val_ptr,
                value.len(),
                0,
                if symlink != 0 { XATTR_NOFOLLOW } else { 0 },
            )
        }
    }
    #[cfg(target_os = "linux")]
    {
        // c:37-47 — switch (symlink) { case 0: getxattr; default: lgetxattr; }
        match symlink {
            0 => unsafe { libc::getxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len()) },
            _ => unsafe { libc::lgetxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len()) },
        }
    }
}

/// Port of `xgetxattr(const char *path, const char *name, void *value, size_t size, int symlink)` from `Src/Modules/attr.c:37`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
/// WARNING: param names don't match C — Rust=(_path, _name, _value, _symlink) vs C=(path, name, value, size, symlink)
pub fn xgetxattr(_path: &str, _name: &str, _value: &mut [u8], _symlink: i32) -> isize {
    -1
}

// =====================================================================
// xlistxattr(const char *path, char *list, size_t size, int symlink)  c:51
// =====================================================================

/// Port of `xlistxattr(const char *path, char *list, size_t size, int symlink)` from `Src/Modules/attr.c:52`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
/// WARNING: param names don't match C — Rust=(path, list, symlink) vs C=(path, list, size, symlink)
pub fn xlistxattr(path: &str, list: &mut [u8], symlink: i32) -> isize {
    // c:52
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
        // c:55 — return listxattr(path, list, size, symlink ? XATTR_NOFOLLOW : 0);
        unsafe {
            libc::listxattr(
                path_c.as_ptr(),
                list_ptr,
                list.len(),
                if symlink != 0 { XATTR_NOFOLLOW } else { 0 },
            )
        }
    }
    #[cfg(target_os = "linux")]
    {
        // c:52-62 — switch (symlink) { case 0: listxattr; default: llistxattr; }
        match symlink {
            0 => unsafe { libc::listxattr(path_c.as_ptr(), list_ptr, list.len()) },
            _ => unsafe { libc::llistxattr(path_c.as_ptr(), list_ptr, list.len()) },
        }
    }
}

/// Port of `xlistxattr(const char *path, char *list, size_t size, int symlink)` from `Src/Modules/attr.c:52`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
/// WARNING: param names don't match C — Rust=(_path, _list, _symlink) vs C=(path, list, size, symlink)
pub fn xlistxattr(_path: &str, _list: &mut [u8], _symlink: i32) -> isize {
    -1
}

// =====================================================================
// xsetxattr(const char *path, const char *name, const void *value,
//           size_t size, int flags, int symlink)                       c:66
// =====================================================================

/// Port of `xsetxattr(const char *path, const char *name, const void *value, size_t size, int flags, int symlink)` from `Src/Modules/attr.c:67`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
/// WARNING: param names don't match C — Rust=(path, name, value, flags, symlink) vs C=(path, name, value, size, flags, symlink)
pub fn xsetxattr(path: &str, name: &str, value: &[u8], flags: i32, symlink: i32) -> i32 {
    // c:67
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
        // c:71 — `return setxattr(path, name, value, size, 0, flags | symlink ? XATTR_NOFOLLOW : 0);`
        // The C operator-precedence quirk: `(flags | symlink) ? ... : 0`.
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
        // c:67-78 — switch (symlink) { case 0: setxattr; default: lsetxattr; }
        match symlink {
            0 => unsafe {
                libc::setxattr(
                    path_c.as_ptr(),
                    name_c.as_ptr(),
                    val_ptr,
                    value.len(),
                    flags,
                )
            },
            _ => unsafe {
                libc::lsetxattr(
                    path_c.as_ptr(),
                    name_c.as_ptr(),
                    val_ptr,
                    value.len(),
                    flags,
                )
            },
        }
    }
}

/// Port of `xsetxattr(const char *path, const char *name, const void *value, size_t size, int flags, int symlink)` from `Src/Modules/attr.c:67`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
/// WARNING: param names don't match C — Rust=(_path, _name, _value, _flags, _symlink) vs C=(path, name, value, size, flags, symlink)
pub fn xsetxattr(_path: &str, _name: &str, _value: &[u8], _flags: i32, _symlink: i32) -> i32 {
    -1
}

// =====================================================================
// xremovexattr(const char *path, const char *name, int symlink)       c:82
// =====================================================================

/// Port of `xremovexattr(const char *path, const char *name, int symlink)` from `Src/Modules/attr.c:83`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn xremovexattr(path: &str, name: &str, symlink: i32) -> i32 {
    // c:83
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
        // c:86 — `return removexattr(path, name, symlink ? XATTR_NOFOLLOW : 0);`
        unsafe {
            libc::removexattr(
                path_c.as_ptr(),
                name_c.as_ptr(),
                if symlink != 0 { XATTR_NOFOLLOW } else { 0 },
            )
        }
    }
    #[cfg(target_os = "linux")]
    {
        // c:83-93 — switch (symlink) { case 0: removexattr; default: lremovexattr; }
        match symlink {
            0 => unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr()) },
            _ => unsafe { libc::lremovexattr(path_c.as_ptr(), name_c.as_ptr()) },
        }
    }
}

/// Port of `xremovexattr(const char *path, const char *name, int symlink)` from `Src/Modules/attr.c:83`.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
#[allow(unused_variables)]
pub fn xremovexattr(path: &str, name: &str, symlink: i32) -> i32 {
    -1
}

// =====================================================================
// bin_getattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:97
// =====================================================================

/// Port of `bin_getattr(char *nam, char **argv, Options ops, UNUSED(int func))` from `Src/Modules/attr.c:98`.
#[allow(unused_variables)]
pub fn bin_getattr(nam: &str, argv: &[String], ops: &options, func: i32) -> i32 {
    // c:98
    // c:98 — `int ret = 0;`
    let mut ret: i32 = 0;
    // c:101 — `int val_len = 0, attr_len = 0, slen;`
    let mut val_len: isize = 0;
    let mut attr_len: isize = 0;
    let _slen: usize;
    // c:102 — `char *value, *file = argv[0], *attr = argv[1], *param = argv[2];`
    let file_arg = argv.get(0).map(|s| s.as_str()).unwrap_or("");
    let attr_arg = argv.get(1).map(|s| s.as_str()).unwrap_or("");
    let param: Option<&str> = argv.get(2).map(|s| s.as_str());
    // c:103 — `int symlink = OPT_ISSET(ops, 'h');`
    let symlink: i32 = if OPT_ISSET(ops, b'h') { 1 } else { 0 };

    // c:105 — `unmetafy(file, &slen);`
    let mut file_bytes = file_arg.as_bytes().to_vec();
    _slen = unmetafy(&mut file_bytes);
    // c:106 — `unmetafy(attr, NULL);`
    let mut attr_bytes = attr_arg.as_bytes().to_vec();
    unmetafy(&mut attr_bytes);
    let file = std::str::from_utf8(&file_bytes).unwrap_or(file_arg);
    let attr = std::str::from_utf8(&attr_bytes).unwrap_or(attr_arg);

    // c:107 — `val_len = xgetxattr(file, attr, NULL, 0, symlink);`
    val_len = xgetxattr(file, attr, &mut [], symlink);
    if val_len == 0 {
        // c:108
        if let Some(p) = param {
            // c:109
            unsetparam(p); // c:110
        }
        return 0; // c:111
    }
    if val_len > 0 {
        // c:113
        // c:114 — value = (char *)zalloc(val_len+1);
        let mut value: Vec<u8> = vec![0u8; (val_len + 1) as usize];
        // c:115 — attr_len = xgetxattr(file, attr, value, val_len, symlink);
        attr_len = xgetxattr(file, attr, &mut value[..val_len as usize], symlink);
        if attr_len > 0 && attr_len <= val_len {
            // c:116
            value[attr_len as usize] = b'\0'; // c:117
            let val_plain = String::from_utf8_lossy(&value[..attr_len as usize]).into_owned();
            if let Some(p) = param {
                // c:118
                // c:119 — setsparam(param, metafy(value, attr_len, META_DUP));
                setsparam(p, &metafy(&val_plain));
            } else {
                println!("{}", val_plain); // c:121
            }
        }
        // c:123 — zfree(value, val_len+1); (Vec drop reclaims)
    }
    if val_len < 0 || attr_len < 0 || attr_len > val_len {
        // c:125
        // c:126 — zwarnnam(nam, "%s: %e", metafy(file, slen, META_NOALLOC), errno);
        zwarnnam(
            nam,
            &format!("{}: {}", metafy(file), std::io::Error::last_os_error()),
        );
        // c:133 — ret = 1 + ((val_len > 0 && attr_len > val_len) || attr_len < 0);
        ret = 1 + i32::from((val_len > 0 && attr_len > val_len) || attr_len < 0);
    }
    ret // c:133
}

// =====================================================================
// bin_setattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:132
// =====================================================================

/// Port of `bin_setattr(char *nam, char **argv, Options ops, UNUSED(int func))` from `Src/Modules/attr.c:133`.
#[allow(unused_variables)]
pub fn bin_setattr(nam: &str, argv: &[String], ops: &options, func: i32) -> i32 {
    // c:133
    // c:133 — `int ret = 0, slen, vlen;`
    let _slen: usize;
    let vlen: usize;
    // c:136 — `int symlink = OPT_ISSET(ops, 'h');`
    let symlink: i32 = if OPT_ISSET(ops, b'h') { 1 } else { 0 };
    // c:137 — `char *file = argv[0], *attr = argv[1], *value = argv[2];`
    let file_arg = argv.get(0).map(|s| s.as_str()).unwrap_or("");
    let attr_arg = argv.get(1).map(|s| s.as_str()).unwrap_or("");
    let value_arg = argv.get(2).map(|s| s.as_str()).unwrap_or("");

    // c:139-141 — unmetafy each.
    let mut file_bytes = file_arg.as_bytes().to_vec();
    _slen = unmetafy(&mut file_bytes);
    let mut attr_bytes = attr_arg.as_bytes().to_vec();
    unmetafy(&mut attr_bytes);
    let mut value_bytes = value_arg.as_bytes().to_vec();
    vlen = unmetafy(&mut value_bytes);
    let file = std::str::from_utf8(&file_bytes).unwrap_or(file_arg);
    let attr = std::str::from_utf8(&attr_bytes).unwrap_or(attr_arg);

    // c:142 — `if (xsetxattr(file, attr, value, vlen, 0, symlink))`
    if xsetxattr(file, attr, &value_bytes[..vlen], 0, symlink) != 0 {
        // c:143 — zwarnnam(nam, "%s: %e", metafy(file, slen, META_NOALLOC), errno);
        zwarnnam(
            nam,
            &format!("{}: {}", metafy(file), std::io::Error::last_os_error()),
        );
        return 1; // c:150 ret = 1;
    }
    0 // c:150
}

// =====================================================================
// bin_delattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:149
// =====================================================================

/// Port of `bin_delattr(char *nam, char **argv, Options ops, UNUSED(int func))` from `Src/Modules/attr.c:150`.
///
/// Body assumes `argv.len() >= 2` (file + ≥1 attr) per C body c:154-157
/// which dereferences `argv[0]` and `argv[1..]` directly. The
/// `zdelattr` BUILTIN spec at builtin.rs:10876 declares
/// `min_args=2`; the dispatcher at builtin.rs:591 rejects shorter
/// argv before reaching this body, matching C
/// `Src/builtin.c:432`. Tests that exercise the body MUST go through
/// the dispatcher (or supply at least 2 args) to honor that contract.
#[allow(unused_variables)]
pub fn bin_delattr(nam: &str, argv: &[String], ops: &options, func: i32) -> i32 {
    // c:150 — `int ret = 0, slen;`
    let _slen: usize;
    // c:Src/Modules/attr.c — C's bin_delattr is dispatched by execbuiltin
    // with `min_args=1` (BUILTIN spec at c:469), so argv[0] is always
    // present when the C body runs. The Rust port calls this fn
    // directly from tests / future code paths without that gate, so
    // indexing `argv[0]` on an empty slice panics. Mirror C's
    // dispatcher-level usage error here.
    if argv.is_empty() {
        zwarnnam(nam, "not enough arguments");
        return 1;
    }
    // c:153 — `int symlink = OPT_ISSET(ops, 'h');`
    let symlink: i32 = if OPT_ISSET(ops, b'h') { 1 } else { 0 };
    // c:154 — `char *file = argv[0], **attr = argv;`
    let file_arg = argv[0].as_str();

    // c:156 — `unmetafy(file, &slen);`
    let mut file_bytes = file_arg.as_bytes().to_vec();
    _slen = unmetafy(&mut file_bytes);
    let file = std::str::from_utf8(&file_bytes)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| file_arg.to_string());

    // c:157 — `while (*++attr)` — iterate argv[1..]
    for attr_arg in &argv[1..] {
        // c:158 — `unmetafy(*attr, NULL);`
        let mut attr_bytes = attr_arg.as_bytes().to_vec();
        unmetafy(&mut attr_bytes);
        let attr = std::str::from_utf8(&attr_bytes).unwrap_or(attr_arg);
        if xremovexattr(&file, attr, symlink) != 0 {
            // c:159
            // c:160 — zwarnnam(nam, "%s: %e", metafy(file, slen, META_NOALLOC), errno);
            zwarnnam(
                nam,
                &format!("{}: {}", metafy(&file), std::io::Error::last_os_error()),
            );
            return 1; // c:169-162 ret=1; break;
        }
    }
    0 // c:169
}

// =====================================================================
// bin_listattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:168
// =====================================================================

/// Port of `bin_listattr(char *nam, char **argv, Options ops, UNUSED(int func))` from `Src/Modules/attr.c:169`.
#[allow(unused_variables)]
pub fn bin_listattr(nam: &str, argv: &[String], ops: &options, func: i32) -> i32 {
    // c:169 — `int ret = 0;`
    let mut ret: i32 = 0;
    // c:172 — `int val_len, list_len = 0, slen;`
    let val_len: isize;
    let mut list_len: isize = 0;
    let _slen: usize;
    // c:173 — `char *value, *file = argv[0], *param = argv[1];`
    let file_arg = argv.get(0).map(|s| s.as_str()).unwrap_or("");
    let param: Option<&str> = argv.get(1).map(|s| s.as_str());
    // c:174 — `int symlink = OPT_ISSET(ops, 'h');`
    let symlink: i32 = if OPT_ISSET(ops, b'h') { 1 } else { 0 };

    // c:176 — `unmetafy(file, &slen);`
    let mut file_bytes = file_arg.as_bytes().to_vec();
    _slen = unmetafy(&mut file_bytes);
    let file_owned = std::str::from_utf8(&file_bytes)
        .map(|s| s.to_string())
        .unwrap_or_else(|_| file_arg.to_string());
    let file = file_owned.as_str();

    // c:177 — `val_len = xlistxattr(file, NULL, 0, symlink);`
    val_len = xlistxattr(file, &mut [], symlink);
    if val_len == 0 {
        // c:178
        if let Some(p) = param {
            // c:179
            unsetparam(p); // c:180
        }
        return 0; // c:181
    }
    if val_len > 0 {
        // c:183
        // c:184 — value = (char *)zalloc(val_len+1);
        let mut value: Vec<u8> = vec![0u8; (val_len + 1) as usize];
        // c:185 — list_len = xlistxattr(file, value, val_len, symlink);
        list_len = xlistxattr(file, &mut value[..val_len as usize], symlink);
        if list_len > 0 && list_len <= val_len {
            // c:186
            // c:187 — `char *p = value;` — walk the NUL-separated names list.
            let names_bytes = &value[..list_len as usize];
            let raw_names: Vec<&[u8]> = names_bytes
                .split(|&b| b == 0)
                .filter(|s| !s.is_empty())
                .collect();
            if let Some(p) = param {
                // c:188
                // c:189-202 — build metafied char-array, setaparam(param, array)
                let metafied_names: Vec<String> = raw_names
                    .iter()
                    .map(|n| metafy(&String::from_utf8_lossy(n)))
                    .collect();
                setaparam(p, metafied_names); // c:202
            } else {
                // c:203-206 — printf("%s\n", p) per name.
                for n in &raw_names {
                    println!("{}", String::from_utf8_lossy(n)); // c:204
                }
            }
        }
    }
    if val_len < 0 || list_len < 0 || list_len > val_len {
        // c:210
        // c:211 — zwarnnam(nam, "%s: %e", metafy(file, slen, META_NOALLOC), errno);
        zwarnnam(
            nam,
            &format!("{}: {}", metafy(file), std::io::Error::last_os_error()),
        );
        // c:212 — ret = 1 + (list_len > val_len || list_len < 0);
        ret = 1 + i32::from(list_len > val_len || list_len < 0);
    }
    ret // c:214
}

// =====================================================================
// /* module paraphernalia */                                          c:217
// static struct builtin bintab[]                                     c:219
// static struct features module_features                             c:226
//
// Static dispatch tables consumed by C module loader. Static-link
// path: dispatcher in `src/extensions/` invokes bin_* directly.
// Tables omitted from Rust port pending module-loader.
// =====================================================================

// =====================================================================
// setup_(UNUSED(Module m))                                           c:235
// =====================================================================

// =====================================================================
// External ported + tables. `static struct features module_features` from
// attr.c:226. Dispatch through canonical `module::featuresarray`.
// =====================================================================

// `bintab` — port of `static struct builtin bintab[]` (attr.c).

// `module_features` — port of `static struct features module_features`
// from attr.c:226.

/// Port of `setup_(UNUSED(Module m))` from `Src/Modules/attr.c:236`.
#[allow(unused_variables)]
pub fn setup_(m: *const module) -> i32 {
    // c:236
    // C body c:238-239 — `return 0`. Faithful empty-body port.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from `Src/Modules/attr.c:243`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());
    0 // c:258
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from `Src/Modules/attr.c:251`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables) // c:258
}

/// Port of `boot_(UNUSED(Module m))` from `Src/Modules/attr.c:258`.
#[allow(unused_variables)]
pub fn boot_(m: *const module) -> i32 {
    // c:258
    // C body c:260-261 — `return 0`. Faithful empty-body port; the
    //                    zgetattr/zsetattr/zdelattr/zlistattr builtins
    //                    register via the bn_list feature dispatch.
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from `Src/Modules/attr.c:265`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None) // c:272
}

/// Port of `finish_(UNUSED(Module m))` from `Src/Modules/attr.c:272`.
#[allow(unused_variables)]
pub fn finish_(m: *const module) -> i32 {
    // c:272
    // C body c:274-275 — `return 0`. Faithful empty-body port; the
    //                    builtins unregister via cleanup_'s setfeatureenables.
    0
}

#[cfg(target_os = "macos")]
const XATTR_NOFOLLOW: i32 = 0x0001;

// =====================================================================
// External ported from other Src/*.c files — routed through the canonical
// 2-arg variants that match the C signatures.
// =====================================================================

/// Port of `setsparam(char *s, char *val)` from `Src/params.c:3350` — delegates to
/// `ksh93::setsparam(name, val)` which provides the env-var-shim
/// implementation matching the C signature.
/// WARNING: param names don't match C — Rust=(name, value) vs C=(PM_HASHED)

/// Port of `setaparam(char *s, char **aval)` from `Src/params.c:3595` — delegates to
/// `ksh93::setsparam` with the value colon-joined (PATH-style array
/// shape that the env-var bridge unpacks at read time).
/// WARNING: param names don't match C — Rust=(name, value) vs C=(s, val, flags)
fn setaparam(name: &str, value: Vec<String>) {
    setsparam(name, &value.join(":"));
}

/// Port of `unsetparam(char *s)` from `Src/params.c:3819` — env::remove_var
/// is the static-link equivalent of paramtab->removenode +
/// freeparamnode for scalar params.
fn unsetparam(v: &str) {
    std::env::remove_var(v);
}

// =====================================================================
// Tests
// =====================================================================

static MODULE_FEATURES: OnceLock<Mutex<features>> = OnceLock::new();

// Local stubs for the per-module entry points. C uses generic
// `featuresarray`/`handlefeatures`/`setfeatureenables` (module.c:
// 3275/3370/3445) but those take `Builtin` + `Features` pointer
// fields the Rust port doesn't carry. The hardcoded descriptor
// list mirrors the C bintab/conddefs/mathfuncs/paramdefs.
// WARNING: NOT IN ATTR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn featuresarray(_m: *const module, _f: &Mutex<features>) -> Vec<String> {
    vec![
        "b:zgetattr".to_string(),
        "b:zsetattr".to_string(),
        "b:zdelattr".to_string(),
        "b:zlistattr".to_string(),
    ]
}

// WARNING: NOT IN ATTR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn handlefeatures(_m: *const module, _f: &Mutex<features>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(vec![1; 4]);
    }
    0
}

// WARNING: NOT IN ATTR.C — Rust-only module-framework shim.
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

// WARNING: NOT IN ATTR.C — Rust-only module-framework shim.
// C uses generic featuresarray/handlefeatures/setfeatureenables from
// Src/module.c:3275/3370/3445 with C-side Builtin/Features pointers;
// Rust per-module shims hardcode the bintab/conddefs/mathfuncs/paramdefs.
fn module_features() -> &'static Mutex<features> {
    MODULE_FEATURES.get_or_init(|| {
        Mutex::new(features {
            bn_list: None,
            bn_size: 4,
            cd_list: None,
            cd_size: 0,
            mf_list: None,
            mf_size: 0,
            pd_list: None,
            pd_size: 0,
            n_abstract: 0,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_ops() -> options {
        options {
            ind: [0u8; MAX_OPS],
            args: Vec::new(),
            argscount: 0,
            argsalloc: 0,
        }
    }

    #[test]
    fn xgetxattr_nonexistent_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 0];
        let r = xgetxattr("/nonexistent/path", "user.test", &mut buf, 0);
        assert!(r < 0);
    }

    #[test]
    fn xsetxattr_nonexistent_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let r = xsetxattr("/nonexistent/path", "user.test", b"value", 0, 0);
        assert!(r < 0);
    }

    #[test]
    fn xlistxattr_nonexistent_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 0];
        let r = xlistxattr("/nonexistent/path", &mut buf, 0);
        assert!(r < 0);
    }

    #[test]
    fn xremovexattr_nonexistent_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let r = xremovexattr("/nonexistent/path", "user.test", 0);
        assert!(r < 0);
    }

    #[test]
    fn bin_getattr_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let argv: Vec<String> = vec!["/nonexistent/path".into(), "user.test".into()];
        let rc = bin_getattr("zgetattr", &argv, &ops, 0);
        assert_ne!(rc, 0);
    }

    #[test]
    fn bin_setattr_nonexistent_path_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let argv: Vec<String> = vec![
            "/nonexistent/path".into(),
            "user.test".into(),
            "value".into(),
        ];
        let rc = bin_setattr("zsetattr", &argv, &ops, 0);
        assert_eq!(rc, 1);
    }

    #[test]
    fn module_loaders_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let m: *const module = std::ptr::null();
        let mut features: Vec<String> = Vec::new();
        let mut enables: Option<Vec<i32>> = None;
        assert_eq!(setup_(m), 0);
        assert_eq!(features_(m, &mut features), 0);
        assert_eq!(features.len(), 4);
        assert_eq!(enables_(m, &mut enables), 0);
        assert_eq!(boot_(m), 0);
        assert_eq!(cleanup_(m), 0);
        assert_eq!(finish_(m), 0);
    }

    /// c:98 — `bin_getattr` with NO args returns nonzero (needs at
    /// least path + attr name). Pin the usage-error gate.
    #[test]
    fn bin_getattr_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let rc = bin_getattr("zgetattr", &[], &ops, 0);
        assert_ne!(rc, 0, "bin_getattr without args must error");
    }

    /// c:98 — `bin_getattr` with ONE arg (path only, no attr name)
    /// returns nonzero. Pin the second-arg-required gate.
    #[test]
    fn bin_getattr_one_arg_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let argv = vec!["/tmp".to_string()];
        let rc = bin_getattr("zgetattr", &argv, &ops, 0);
        assert_ne!(rc, 0, "bin_getattr with only path must error");
    }

    /// c:133 — `bin_setattr` body (c:135-146) has NO internal arity
    /// check. Upstream dispatcher (BUILTIN min_args=3 at attr.c:300)
    /// enforces it before calling the body. Calling the body
    /// directly with <3 args reads `argv[2]` as empty (Rust port via
    /// `argv.get(2).unwrap_or("")`; C reads OOB as UB).
    ///
    /// Pin the body's actual behavior: empty value + bogus path →
    /// xsetxattr fails → return 1.
    #[test]
    fn bin_setattr_with_bogus_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let argv = vec![
            "/__definitely_not_a_path__".to_string(),
            "user.test".to_string(),
            "value".to_string(),
        ];
        let rc = bin_setattr("zsetattr", &argv, &ops, 0);
        assert_eq!(rc, 1, "bin_setattr on bogus path must return 1 per c:144");
    }

    /// c:175 — `bin_delattr` on a nonexistent path returns nonzero.
    /// Pin the error-passthrough from removexattr's ENOENT.
    #[test]
    fn bin_delattr_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let argv = vec![
            "/__definitely_not_a_path__".to_string(),
            "user.test".to_string(),
        ];
        let rc = bin_delattr("zdelattr", &argv, &ops, 0);
        assert_ne!(rc, 0, "delattr on bogus path must error");
    }

    /// c:222 — `bin_listattr` on a nonexistent path returns nonzero.
    /// Pin the listxattr error-surface contract.
    #[test]
    fn bin_listattr_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let argv = vec!["/__definitely_not_a_path__".to_string()];
        let rc = bin_listattr("zlistattr", &argv, &ops, 0);
        assert_ne!(rc, 0, "listattr on bogus path must error");
    }

    /// c:37 — `xgetxattr` on a nonexistent path returns negative
    /// (errno = ENOENT). Pin the negative-return signal.
    #[test]
    fn xgetxattr_nonexistent_path_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 64];
        let r = xgetxattr("/__nonexistent__", "user.foo", &mut buf, 0);
        assert!(r < 0, "xgetxattr on bogus path must surface error");
    }

    /// c:67 — `xsetxattr` on a nonexistent path returns nonzero.
    #[test]
    fn xsetxattr_nonexistent_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = xsetxattr("/__nonexistent__", "user.foo", b"v", 0, 0);
        assert_ne!(r, 0, "xsetxattr on bogus path must surface error");
    }

    // ─── zsh-corpus pins for xlistxattr / xremovexattr / builtins ────

    /// `xlistxattr` on nonexistent path returns negative.
    #[test]
    fn attr_corpus_xlistxattr_nonexistent_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 128];
        let r = xlistxattr("/__never_exists_zshrs_xyz__", &mut buf, 0);
        assert!(r < 0, "missing path → negative, got {r}");
    }

    /// `xremovexattr` on nonexistent path returns nonzero error.
    #[test]
    fn attr_corpus_xremovexattr_nonexistent_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = xremovexattr("/__never_exists_zshrs_xyz__", "user.foo", 0);
        assert_ne!(r, 0, "missing path → error");
    }

    /// `bin_getattr` with no args returns nonzero (usage error).
    #[test]
    fn attr_corpus_bin_getattr_no_args_errors() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_getattr("zgetattr", &[], &ops, 0);
        assert_ne!(r, 0);
    }

    /// `bin_setattr` with no args returns nonzero.
    #[test]
    fn attr_corpus_bin_setattr_no_args_errors() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_setattr("zsetattr", &[], &ops, 0);
        assert_ne!(r, 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/attr.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `bin_delattr` no-args usage error — routed through the
    /// dispatcher (`execbuiltin`) so the c:222 BUILTIN min_args=2 gate
    /// at builtin.rs:591 fires before reaching the body. Calling
    /// `bin_delattr(..., &[], ...)` directly would OOB-panic on
    /// `argv[0]` — that's the dispatcher's job to prevent, not the
    /// body's. This matches the C contract at Src/builtin.c:432 (gate)
    /// + Src/Modules/attr.c:154 (body assumes argv[0]).
    #[test]
    fn bin_delattr_no_args_errors_via_dispatcher() {
        let _g = crate::test_util::global_state_lock();
        let table = crate::ported::builtin::createbuiltintable();
        let bn = table.get("zdelattr").expect("zdelattr registered");
        let bn_ptr = (*bn as *const _) as *mut crate::ported::zsh_h::builtin;
        let r = crate::ported::builtin::execbuiltin(
            vec!["zdelattr".to_string()], // argv[0] = command name only
            Vec::new(),
            bn_ptr,
        );
        assert_ne!(r, 0, "min_args=2 unmet → dispatcher returns nonzero");
    }

    /// `bin_delattr` with only file path routed through dispatcher —
    /// argc=1 still fails min_args=2 gate (file + ≥1 attr required).
    #[test]
    fn bin_delattr_one_arg_errors_via_dispatcher() {
        let _g = crate::test_util::global_state_lock();
        let table = crate::ported::builtin::createbuiltintable();
        let bn = table.get("zdelattr").expect("zdelattr registered");
        let bn_ptr = (*bn as *const _) as *mut crate::ported::zsh_h::builtin;
        let r = crate::ported::builtin::execbuiltin(
            vec!["zdelattr".to_string(), "/tmp".to_string()], // file only
            Vec::new(),
            bn_ptr,
        );
        assert_ne!(r, 0, "file alone without attr name → usage error");
    }

    /// `bin_listattr` with no args returns nonzero (usage error).
    #[test]
    fn bin_listattr_no_args_errors() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_listattr("zlistattr", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// `xgetxattr` on nonexistent path returns negative (libc errno).
    #[test]
    fn xgetxattr_nonexistent_path_returns_negative_pin() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = vec![0u8; 256];
        let r = xgetxattr("/__never_exists_zshrs_xyz__", "user.foo", &mut buf, 0);
        assert!(r < 0, "missing path → negative, got {}", r);
    }

    /// `xlistxattr` on nonexistent path returns negative.
    #[test]
    fn xlistxattr_nonexistent_path_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = vec![0u8; 1024];
        let r = xlistxattr("/__never_exists_zshrs_xyz__", &mut buf, 0);
        assert!(r < 0, "missing path → negative, got {}", r);
    }

    /// `xsetxattr` on nonexistent path returns nonzero.
    #[test]
    fn xsetxattr_nonexistent_path_returns_nonzero_pin() {
        let _g = crate::test_util::global_state_lock();
        let r = xsetxattr("/__never_exists_zshrs_xyz__", "user.foo", b"v", 0, 0);
        assert_ne!(r, 0, "missing path → nonzero error");
    }

    /// `xremovexattr` with symlink=1 on nonexistent path returns nonzero.
    #[test]
    fn xremovexattr_nonexistent_with_symlink_flag_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let r = xremovexattr("/__never_exists_zshrs_xyz__", "user.foo", 1);
        assert_ne!(r, 0, "missing path with symlink=1 → nonzero");
    }

    /// `xgetxattr` with empty buffer is safe (no panic).
    #[test]
    fn xgetxattr_empty_buf_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut buf: [u8; 0] = [];
        let _ = xgetxattr("/__never_exists_zshrs_xyz__", "user.foo", &mut buf, 0);
    }

    /// `xlistxattr` with empty buffer no panic.
    #[test]
    fn xlistxattr_empty_buf_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut buf: [u8; 0] = [];
        let _ = xlistxattr("/__never_exists_zshrs_xyz__", &mut buf, 0);
    }

    /// Module lifecycle: setup_ returns 0.
    #[test]
    fn attr_setup_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(setup_(std::ptr::null()), 0);
    }

    /// Boot/cleanup/finish all 0.
    #[test]
    fn attr_boot_cleanup_finish_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(boot_(std::ptr::null()), 0);
        assert_eq!(cleanup_(std::ptr::null()), 0);
        assert_eq!(finish_(std::ptr::null()), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/attr.c
    // c:39 xgetxattr / c:92 xlistxattr / c:140 xsetxattr / c:210 xremovexattr
    // c:254 bin_getattr / c:327 bin_setattr / c:367 bin_delattr / c:407 bin_listattr
    // ═══════════════════════════════════════════════════════════════════

    /// c:39 — `xgetxattr` is deterministic for same input.
    #[test]
    fn xgetxattr_is_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        let first = xgetxattr("/nonexistent_path", "user.x", &mut buf, 0);
        for _ in 0..3 {
            let mut buf2 = [0u8; 4];
            let r = xgetxattr("/nonexistent_path", "user.x", &mut buf2, 0);
            assert_eq!(
                r.signum(),
                first.signum(),
                "xgetxattr must be deterministic in sign"
            );
        }
    }

    /// c:39 — `xgetxattr` with symlink flag = 1 also returns negative for
    /// nonexistent path.
    #[test]
    fn xgetxattr_with_symlink_flag_nonexistent_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        let r = xgetxattr("/nonexistent_path", "user.x", &mut buf, 1);
        assert!(r < 0, "symlink mode on nonexistent path → negative");
    }

    /// c:92 — `xlistxattr` with symlink flag also returns negative for
    /// nonexistent path.
    #[test]
    fn xlistxattr_with_symlink_flag_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 4];
        assert!(xlistxattr("/nonexistent_path", &mut buf, 1) < 0);
    }

    /// c:140 — `xsetxattr` with all-zero buf works (empty value set).
    /// On nonexistent path still fails.
    #[test]
    fn xsetxattr_empty_value_on_nonexistent_returns_negative() {
        let _g = crate::test_util::global_state_lock();
        let r = xsetxattr("/nonexistent_path", "user.empty", &[], 0, 0);
        assert!(r < 0, "nonexistent path → negative regardless of value");
    }

    /// c:140 — `xsetxattr` with various flags doesn't panic on bad path.
    #[test]
    fn xsetxattr_various_flags_no_panic() {
        let _g = crate::test_util::global_state_lock();
        for flag in [0i32, 1, 2, 0xff] {
            let _ = xsetxattr("/nonexistent_path", "user.x", b"val", flag, 0);
        }
    }

    /// c:210 — `xremovexattr` with both symlink flag values doesn't panic.
    #[test]
    fn xremovexattr_both_symlink_modes_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = xremovexattr("/nonexistent_path", "user.x", 0);
        let _ = xremovexattr("/nonexistent_path", "user.x", 1);
    }

    /// c:254 — `bin_getattr` empty path arg → nonzero.
    #[test]
    fn bin_getattr_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_getattr("getattr", &["".into(), "user.x".into()], &ops, 0);
        assert_ne!(r, 0, "empty path → error");
    }

    /// c:407 — `bin_listattr` empty path returns nonzero.
    #[test]
    fn bin_listattr_empty_path_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_listattr("listattr", &["".into()], &ops, 0);
        assert_ne!(r, 0, "empty path → error");
    }

    /// c:254-407 — all four attr builtin return values fit in u8 exit-code
    /// range (0..256).
    #[test]
    fn attr_builtins_return_in_exit_code_range() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for r in [
            bin_getattr("getattr", &["/tmp".into(), "user.x".into()], &ops, 0),
            bin_setattr(
                "setattr",
                &["/tmp".into(), "user.x".into(), "v".into()],
                &ops,
                0,
            ),
            bin_listattr("listattr", &["/tmp".into()], &ops, 0),
        ] {
            assert!(
                (0..256).contains(&r),
                "exit code must fit in u8 range, got {}",
                r
            );
        }
    }

    /// c:507-... — full lifecycle setup→features→enables→boot→cleanup→finish.
    #[test]
    fn attr_full_lifecycle_returns_zero_for_all() {
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

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Modules/attr.c
    // c:39 xgetxattr / c:92 xlistxattr / c:140 xsetxattr / c:210 xremovexattr
    // c:254 bin_getattr / c:327 bin_setattr / c:375 bin_delattr /
    // c:415 bin_listattr
    // ═══════════════════════════════════════════════════════════════════

    /// c:39 — `xgetxattr` returns isize (compile-time pin).
    #[test]
    fn xgetxattr_returns_isize_type() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 8];
        let _: isize = xgetxattr("/nonexistent", "user.x", &mut buf, 0);
    }

    /// c:92 — `xlistxattr` returns isize (compile-time pin).
    #[test]
    fn xlistxattr_returns_isize_type() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 8];
        let _: isize = xlistxattr("/nonexistent", &mut buf, 0);
    }

    /// c:140 — `xsetxattr` returns i32 (compile-time pin).
    #[test]
    fn xsetxattr_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = xsetxattr("/nonexistent", "user.x", b"v", 0, 0);
    }

    /// c:210 — `xremovexattr` returns i32 (compile-time pin).
    #[test]
    fn xremovexattr_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = xremovexattr("/nonexistent", "user.x", 0);
    }

    /// c:39 — `xgetxattr` deterministic for nonexistent path.
    #[test]
    fn xgetxattr_nonexistent_deterministic() {
        let _g = crate::test_util::global_state_lock();
        let mut a = [0u8; 8];
        let first = xgetxattr("/__definitely_no_such_xyz__", "user.x", &mut a, 0);
        for _ in 0..3 {
            let mut b = [0u8; 8];
            assert_eq!(
                xgetxattr("/__definitely_no_such_xyz__", "user.x", &mut b, 0),
                first,
                "xgetxattr must be pure across calls for missing path"
            );
        }
    }

    /// c:254/327/375/415 — every bin_*attr returns i32 (compile-time pin).
    #[test]
    fn attr_builtins_all_return_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let _: i32 = bin_getattr("getattr", &[], &ops, 0);
        let _: i32 = bin_setattr("setattr", &[], &ops, 0);
        let _: i32 = bin_delattr("delattr", &[], &ops, 0);
        let _: i32 = bin_listattr("listattr", &[], &ops, 0);
    }

    /// c:254 — `bin_getattr` no args returns nonzero (alt-name dup pin).
    #[test]
    fn bin_getattr_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_getattr("getattr", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:327 — `bin_setattr` no args returns nonzero (alt name).
    #[test]
    fn bin_setattr_no_args_usage_error_alt() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_setattr("setattr", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:375 — `bin_delattr` no args MUST return nonzero (C: usage error).
    /// In zshrs the port panics with index OOB instead.
    #[test]
    fn bin_delattr_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_delattr("delattr", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:415 — `bin_listattr` no args returns nonzero.
    #[test]
    fn bin_listattr_no_args_returns_nonzero() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_listattr("listattr", &[], &ops, 0);
        assert_ne!(r, 0, "no args → usage error");
    }

    /// c:254/327/375/415 — all builtin exit codes non-negative.
    #[test]
    fn attr_builtins_exit_codes_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for r in [
            bin_getattr("getattr", &[], &ops, 0),
            bin_setattr("setattr", &[], &ops, 0),
            bin_delattr("delattr", &[], &ops, 0),
            bin_listattr("listattr", &[], &ops, 0),
        ] {
            assert!(r >= 0, "exit code must be non-negative, got {}", r);
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity pins for Src/Modules/attr.c
    // c:39 xgetxattr / c:92 xlistxattr / c:140 xsetxattr / c:210 xremovexattr /
    // c:254-415 bin_* / c:515 setup_
    // ═══════════════════════════════════════════════════════════════════

    /// c:515 — `setup_` is idempotent.
    #[test]
    fn attr_setup_idempotent_alt_pin() {
        let _g = crate::test_util::global_state_lock();
        for _ in 0..10 {
            assert_eq!(setup_(std::ptr::null()), 0);
        }
    }

    /// c:39 — `xgetxattr` for nonexistent path doesn't panic.
    #[test]
    fn xgetxattr_nonexistent_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 64];
        let _ = xgetxattr("/__never_exists_xyz__", "user.test", &mut buf, 0);
    }

    /// c:39 — `xgetxattr` empty path doesn't panic.
    #[test]
    fn xgetxattr_empty_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 64];
        let _ = xgetxattr("", "", &mut buf, 0);
    }

    /// c:92 — `xlistxattr` returns isize (alt).
    #[test]
    fn xlistxattr_returns_isize_type_alt() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 64];
        let _: isize = xlistxattr("/", &mut buf, 0);
    }

    /// c:92 — `xlistxattr` for nonexistent path doesn't panic.
    #[test]
    fn xlistxattr_nonexistent_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let mut buf = [0u8; 64];
        let _ = xlistxattr("/__never_exists_xyz__", &mut buf, 0);
    }

    /// c:140 — `xsetxattr` for nonexistent path doesn't panic.
    #[test]
    fn xsetxattr_nonexistent_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = xsetxattr("/__never_exists__", "user.test", b"v", 0, 0);
    }

    /// c:210 — `xremovexattr` for nonexistent path doesn't panic.
    #[test]
    fn xremovexattr_nonexistent_path_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _ = xremovexattr("/__never_exists__", "user.test", 0);
    }

    /// c:254 — `bin_getattr` various func values don't panic.
    #[test]
    fn bin_getattr_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_getattr("getattr", &[], &ops, func);
        }
    }

    /// c:327 — `bin_setattr` various func values don't panic.
    #[test]
    fn bin_setattr_various_func_values_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        for func in [-1, 0, 1, 100, i32::MAX] {
            let _ = bin_setattr("setattr", &[], &ops, func);
        }
    }

    /// c:254 — `bin_getattr` empty args non-negative.
    #[test]
    fn bin_getattr_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_getattr("getattr", &[], &ops, 0);
        assert!(r >= 0, "bin_getattr empty must be ≥ 0, got {}", r);
    }

    /// c:327 — `bin_setattr` empty args non-negative.
    #[test]
    fn bin_setattr_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_setattr("setattr", &[], &ops, 0);
        assert!(r >= 0, "bin_setattr empty must be ≥ 0, got {}", r);
    }

    /// c:415 — `bin_listattr` empty args non-negative.
    #[test]
    fn bin_listattr_empty_args_non_negative() {
        let _g = crate::test_util::global_state_lock();
        let ops = empty_ops();
        let r = bin_listattr("listattr", &[], &ops, 0);
        assert!(r >= 0, "bin_listattr empty must be ≥ 0, got {}", r);
    }
}
