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

use std::ffi::CString;

use crate::ported::utils::{metafy, unmetafy, zwarnnam};
use crate::ported::zsh_h::{module, options, OPT_ISSET};

#[cfg(target_os = "macos")]
const XATTR_NOFOLLOW: i32 = 0x0001;

// =====================================================================
// xgetxattr(const char *path, const char *name, void *value, size_t size, int symlink)  c:36
// =====================================================================

/// Port of `xgetxattr()` from `Src/Modules/attr.c:36`.
///
/// Caller passes a `&mut [u8]` slot for `value` and the buffer length
/// for `size`. Empty slice queries required size — same as C
/// `value=NULL, size=0` (attr.c:107).
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn xgetxattr(path: &str, name: &str, value: &mut [u8], symlink: i32) -> isize {
    let path_c = match CString::new(path) { Ok(c) => c, Err(_) => return -1 };
    let name_c = match CString::new(name) { Ok(c) => c, Err(_) => return -1 };
    let val_ptr = if value.is_empty() {
        std::ptr::null_mut()
    } else {
        value.as_mut_ptr() as *mut libc::c_void
    };
    #[cfg(target_os = "macos")]
    {
        // c:40 — `return getxattr(path, name, value, size, 0, symlink ? XATTR_NOFOLLOW: 0);`
        unsafe {
            libc::getxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len(), 0,
                           if symlink != 0 { XATTR_NOFOLLOW } else { 0 })
        }
    }
    #[cfg(target_os = "linux")]
    {
        // c:42-47 — switch (symlink) { case 0: getxattr; default: lgetxattr; }
        match symlink {
            0 => unsafe { libc::getxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len()) },
            _ => unsafe { libc::lgetxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len()) },
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xgetxattr(_path: &str, _name: &str, _value: &mut [u8], _symlink: i32) -> isize { -1 }

// =====================================================================
// xlistxattr(const char *path, char *list, size_t size, int symlink)  c:51
// =====================================================================

/// Port of `xlistxattr()` from `Src/Modules/attr.c:51`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn xlistxattr(path: &str, list: &mut [u8], symlink: i32) -> isize {
    let path_c = match CString::new(path) { Ok(c) => c, Err(_) => return -1 };
    let list_ptr = if list.is_empty() {
        std::ptr::null_mut()
    } else {
        list.as_mut_ptr() as *mut libc::c_char
    };
    #[cfg(target_os = "macos")]
    {
        // c:55 — return listxattr(path, list, size, symlink ? XATTR_NOFOLLOW : 0);
        unsafe {
            libc::listxattr(path_c.as_ptr(), list_ptr, list.len(),
                            if symlink != 0 { XATTR_NOFOLLOW } else { 0 })
        }
    }
    #[cfg(target_os = "linux")]
    {
        // c:57-62 — switch (symlink) { case 0: listxattr; default: llistxattr; }
        match symlink {
            0 => unsafe { libc::listxattr(path_c.as_ptr(), list_ptr, list.len()) },
            _ => unsafe { libc::llistxattr(path_c.as_ptr(), list_ptr, list.len()) },
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xlistxattr(_path: &str, _list: &mut [u8], _symlink: i32) -> isize { -1 }

// =====================================================================
// xsetxattr(const char *path, const char *name, const void *value,
//           size_t size, int flags, int symlink)                       c:66
// =====================================================================

/// Port of `xsetxattr()` from `Src/Modules/attr.c:66`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn xsetxattr(path: &str, name: &str, value: &[u8], flags: i32, symlink: i32) -> i32 {
    let path_c = match CString::new(path) { Ok(c) => c, Err(_) => return -1 };
    let name_c = match CString::new(name) { Ok(c) => c, Err(_) => return -1 };
    let val_ptr = value.as_ptr() as *const libc::c_void;
    #[cfg(target_os = "macos")]
    {
        // c:71 — `return setxattr(path, name, value, size, 0, flags | symlink ? XATTR_NOFOLLOW : 0);`
        // The C operator-precedence quirk: `(flags | symlink) ? ... : 0`.
        let combined = if (flags | symlink) != 0 { XATTR_NOFOLLOW } else { 0 };
        unsafe {
            libc::setxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len(), 0, combined)
        }
    }
    #[cfg(target_os = "linux")]
    {
        // c:73-78 — switch (symlink) { case 0: setxattr; default: lsetxattr; }
        match symlink {
            0 => unsafe { libc::setxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len(), flags) },
            _ => unsafe { libc::lsetxattr(path_c.as_ptr(), name_c.as_ptr(), val_ptr, value.len(), flags) },
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xsetxattr(_path: &str, _name: &str, _value: &[u8], _flags: i32, _symlink: i32) -> i32 { -1 }

// =====================================================================
// xremovexattr(const char *path, const char *name, int symlink)       c:82
// =====================================================================

/// Port of `xremovexattr()` from `Src/Modules/attr.c:82`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn xremovexattr(path: &str, name: &str, symlink: i32) -> i32 {
    let path_c = match CString::new(path) { Ok(c) => c, Err(_) => return -1 };
    let name_c = match CString::new(name) { Ok(c) => c, Err(_) => return -1 };
    #[cfg(target_os = "macos")]
    {
        // c:86 — `return removexattr(path, name, symlink ? XATTR_NOFOLLOW : 0);`
        unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr(),
                                   if symlink != 0 { XATTR_NOFOLLOW } else { 0 }) }
    }
    #[cfg(target_os = "linux")]
    {
        // c:88-93 — switch (symlink) { case 0: removexattr; default: lremovexattr; }
        match symlink {
            0 => unsafe { libc::removexattr(path_c.as_ptr(), name_c.as_ptr()) },
            _ => unsafe { libc::lremovexattr(path_c.as_ptr(), name_c.as_ptr()) },
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn xremovexattr(_path: &str, _name: &str, _symlink: i32) -> i32 { -1 }

// =====================================================================
// bin_getattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:97
// =====================================================================

/// Port of `bin_getattr()` from `Src/Modules/attr.c:97`.
pub fn bin_getattr(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 {
    // c:100 — `int ret = 0;`
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
    if val_len == 0 {                                                    // c:108
        if let Some(p) = param {                                         // c:109
            unsetparam(p);                                                // c:110
        }
        return 0;                                                         // c:111
    }
    if val_len > 0 {                                                     // c:113
        // c:114 — value = (char *)zalloc(val_len+1);
        let mut value: Vec<u8> = vec![0u8; (val_len + 1) as usize];
        // c:115 — attr_len = xgetxattr(file, attr, value, val_len, symlink);
        attr_len = xgetxattr(file, attr, &mut value[..val_len as usize], symlink);
        if attr_len > 0 && attr_len <= val_len {                         // c:116
            value[attr_len as usize] = b'\0';                            // c:117
            let val_plain = String::from_utf8_lossy(&value[..attr_len as usize]).into_owned();
            if let Some(p) = param {                                     // c:118
                // c:119 — setsparam(param, metafy(value, attr_len, META_DUP));
                setsparam(p, &metafy(&val_plain));
            } else {
                println!("{}", val_plain);                                // c:121
            }
        }
        // c:123 — zfree(value, val_len+1); (Vec drop reclaims)
    }
    if val_len < 0 || attr_len < 0 || attr_len > val_len {               // c:125
        // c:126 — zwarnnam(nam, "%s: %e", metafy(file, slen, META_NOALLOC), errno);
        zwarnnam(nam, &format!("{}: {}", metafy(file), std::io::Error::last_os_error()));
        // c:127 — ret = 1 + ((val_len > 0 && attr_len > val_len) || attr_len < 0);
        ret = 1 + i32::from((val_len > 0 && attr_len > val_len) || attr_len < 0);
    }
    ret                                                                   // c:129
}

// =====================================================================
// bin_setattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:132
// =====================================================================

/// Port of `bin_setattr()` from `Src/Modules/attr.c:132`.
pub fn bin_setattr(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 {
    // c:135 — `int ret = 0, slen, vlen;`
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
        zwarnnam(nam, &format!("{}: {}", metafy(file), std::io::Error::last_os_error()));
        return 1;                                                         // c:144 ret = 1;
    }
    0                                                                     // c:146
}

// =====================================================================
// bin_delattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:149
// =====================================================================

/// Port of `bin_delattr()` from `Src/Modules/attr.c:149`.
pub fn bin_delattr(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 {
    // c:152 — `int ret = 0, slen;`
    let _slen: usize;
    // c:153 — `int symlink = OPT_ISSET(ops, 'h');`
    let symlink: i32 = if OPT_ISSET(ops, b'h') { 1 } else { 0 };
    // c:154 — `char *file = argv[0], **attr = argv;`
    let file_arg = argv.get(0).map(|s| s.as_str()).unwrap_or("");

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
        if xremovexattr(&file, attr, symlink) != 0 {                     // c:159
            // c:160 — zwarnnam(nam, "%s: %e", metafy(file, slen, META_NOALLOC), errno);
            zwarnnam(nam, &format!("{}: {}", metafy(&file), std::io::Error::last_os_error()));
            return 1;                                                     // c:161-162 ret=1; break;
        }
    }
    0                                                                     // c:165
}

// =====================================================================
// bin_listattr(char *nam, char **argv, Options ops, UNUSED(int func))  c:168
// =====================================================================

/// Port of `bin_listattr()` from `Src/Modules/attr.c:168`.
pub fn bin_listattr(nam: &str, argv: &[String], ops: &options, _func: i32) -> i32 {
    // c:171 — `int ret = 0;`
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
    if val_len == 0 {                                                    // c:178
        if let Some(p) = param {                                         // c:179
            unsetparam(p);                                                // c:180
        }
        return 0;                                                         // c:181
    }
    if val_len > 0 {                                                     // c:183
        // c:184 — value = (char *)zalloc(val_len+1);
        let mut value: Vec<u8> = vec![0u8; (val_len + 1) as usize];
        // c:185 — list_len = xlistxattr(file, value, val_len, symlink);
        list_len = xlistxattr(file, &mut value[..val_len as usize], symlink);
        if list_len > 0 && list_len <= val_len {                         // c:186
            // c:187 — `char *p = value;` — walk the NUL-separated names list.
            let names_bytes = &value[..list_len as usize];
            let raw_names: Vec<&[u8]> = names_bytes
                .split(|&b| b == 0)
                .filter(|s| !s.is_empty())
                .collect();
            if let Some(p) = param {                                     // c:188
                // c:189-202 — build metafied char-array, setaparam(param, array)
                let metafied_names: Vec<String> = raw_names
                    .iter()
                    .map(|n| metafy(&String::from_utf8_lossy(n)))
                    .collect();
                setaparam(p, metafied_names);                            // c:202
            } else {
                // c:203-206 — printf("%s\n", p) per name.
                for n in &raw_names {
                    println!("{}", String::from_utf8_lossy(n));         // c:204
                }
            }
        }
    }
    if val_len < 0 || list_len < 0 || list_len > val_len {               // c:210
        // c:211 — zwarnnam(nam, "%s: %e", metafy(file, slen, META_NOALLOC), errno);
        zwarnnam(nam, &format!("{}: {}", metafy(file), std::io::Error::last_os_error()));
        // c:212 — ret = 1 + (list_len > val_len || list_len < 0);
        ret = 1 + i32::from(list_len > val_len || list_len < 0);
    }
    ret                                                                   // c:214
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

/// Port of `setup_()` from `Src/Modules/attr.c:236`.
pub fn setup_(_m: *const module) -> i32 { 0 }                          // c:238

/// Port of `features_()` from `Src/Modules/attr.c:243`.
/// C body: `*features = featuresarray(m, &module_features); return 0;`
pub fn features_(m: *const module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, module_features());                   // c:245
    0                                                                  // c:246
}

/// Port of `enables_()` from `Src/Modules/attr.c:251`.
/// C body: `return handlefeatures(m, &module_features, enables);`
pub fn enables_(m: *const module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, module_features(), enables)                      // c:253
}

/// Port of `boot_()` from `Src/Modules/attr.c:258`.
pub fn boot_(_m: *const module) -> i32 { 0 }                           // c:260

/// Port of `cleanup_()` from `Src/Modules/attr.c:265`.
/// C body: `return setfeatureenables(m, &module_features, NULL);`
pub fn cleanup_(m: *const module) -> i32 {
    setfeatureenables(m, module_features(), None)                      // c:267
}

/// Port of `finish_()` from `Src/Modules/attr.c:272`.
pub fn finish_(_m: *const module) -> i32 { 0 }                         // c:274

// =====================================================================
// External fns + tables. `static struct features module_features` from
// attr.c:226, plus Src/module.c stubs.
// =====================================================================

use std::sync::{Mutex, OnceLock};
use crate::ported::zsh_h::features as features_t;

static MODULE_FEATURES: OnceLock<Mutex<features_t>> = OnceLock::new();

fn module_features() -> &'static Mutex<features_t> {
    MODULE_FEATURES.get_or_init(|| Mutex::new(features_t {
        bn_list: None,                                                 // c:227 bintab[4]
        bn_size: 4,
        cd_list: None,
        cd_size: 0,
        mf_list: None,
        mf_size: 0,
        pd_list: None,
        pd_size: 0,
        n_abstract: 0,
    }))
}

// `featuresarray` — Src/module.c:3275.
fn featuresarray(_m: *const module, _f: &Mutex<features_t>) -> Vec<String> {
    vec![
        "b:zgetattr".to_string(),
        "b:zsetattr".to_string(),
        "b:zdelattr".to_string(),
        "b:zlistattr".to_string(),
    ]
}

// `handlefeatures` — Src/module.c:3370.
fn handlefeatures(m: *const module, f: &Mutex<features_t>, enables: &mut Option<Vec<i32>>) -> i32 {
    if enables.is_none() {
        *enables = Some(getfeatureenables(m, f));
    } else if let Some(e) = enables.as_ref() {
        return setfeatureenables(m, f, Some(e));
    }
    0
}

fn getfeatureenables(_m: *const module, f: &Mutex<features_t>) -> Vec<i32> {
    let g = f.lock().unwrap();
    let total = g.bn_size + g.cd_size + g.mf_size + g.pd_size + g.n_abstract;
    vec![0; total as usize]
}

// `setfeatureenables` — Src/module.c:3445.
fn setfeatureenables(_m: *const module, _f: &Mutex<features_t>, _e: Option<&Vec<i32>>) -> i32 {
    0
}

// =====================================================================
// External fns from other Src/*.c files — routed through the canonical
// 2-arg variants that match the C signatures.
// =====================================================================

/// Port of `setsparam()` from `Src/params.c:3380` — delegates to
/// `ksh93::setsparam(name, val)` which provides the env-var-shim
/// implementation matching the C signature.
fn setsparam(name: &str, value: &str) {
    crate::ported::modules::ksh93::setsparam(name, value);
}

/// Port of `setaparam()` from `Src/params.c:3357` — delegates to
/// `ksh93::setsparam` with the value colon-joined (PATH-style array
/// shape that the env-var bridge unpacks at read time).
fn setaparam(name: &str, value: Vec<String>) {
    crate::ported::modules::ksh93::setsparam(name, &value.join(":"));
}

/// Port of `unsetparam()` from `Src/params.c:3690` — env::remove_var
/// is the static-link equivalent of paramtab->removenode +
/// freeparamnode for scalar params.
fn unsetparam(name: &str) {
    std::env::remove_var(name);
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zsh_h::MAX_OPS;

    fn empty_ops() -> options {
        options { ind: [0u8; MAX_OPS], args: Vec::new(), argscount: 0, argsalloc: 0 }
    }

    #[test]
    fn xgetxattr_nonexistent_returns_negative() {
        let mut buf = [0u8; 0];
        let r = xgetxattr("/nonexistent/path", "user.test", &mut buf, 0);
        assert!(r < 0);
    }

    #[test]
    fn xsetxattr_nonexistent_returns_negative() {
        let r = xsetxattr("/nonexistent/path", "user.test", b"value", 0, 0);
        assert!(r < 0);
    }

    #[test]
    fn xlistxattr_nonexistent_returns_negative() {
        let mut buf = [0u8; 0];
        let r = xlistxattr("/nonexistent/path", &mut buf, 0);
        assert!(r < 0);
    }

    #[test]
    fn xremovexattr_nonexistent_returns_negative() {
        let r = xremovexattr("/nonexistent/path", "user.test", 0);
        assert!(r < 0);
    }

    #[test]
    fn bin_getattr_nonexistent_path_returns_nonzero() {
        let ops = empty_ops();
        let argv: Vec<String> = vec!["/nonexistent/path".into(), "user.test".into()];
        let rc = bin_getattr("zgetattr", &argv, &ops, 0);
        assert_ne!(rc, 0);
    }

    #[test]
    fn bin_setattr_nonexistent_path_returns_one() {
        let ops = empty_ops();
        let argv: Vec<String> = vec!["/nonexistent/path".into(), "user.test".into(), "value".into()];
        let rc = bin_setattr("zsetattr", &argv, &ops, 0);
        assert_eq!(rc, 1);
    }

    #[test]
    fn module_loaders_return_zero() {
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
}
