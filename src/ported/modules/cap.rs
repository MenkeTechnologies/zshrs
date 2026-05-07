//! Capabilities module - port of Modules/cap.c
//!
//! Provides POSIX.1e capability manipulation via cap, getcap, setcap builtins.
//! Requires the `libcap` feature and libcap on Linux
//! (`apt install libcap-dev` / `dnf install libcap-devel`).

use std::io;

// libcap FFI — these live in libcap (-lcap), not in libc.
#[cfg(all(target_os = "linux", feature = "libcap"))]
mod ffi {
    use libc::{c_char, c_int, c_void, ssize_t};

    /// Opaque capability state (cap_t is a pointer to this).
    pub type CapT = *mut c_void;

    #[link(name = "cap")]
    extern "C" {
        pub fn cap_get_proc() -> CapT;
        pub fn cap_set_proc(cap_p: CapT) -> c_int;
        pub fn cap_get_file(path: *const c_char) -> CapT;
        pub fn cap_set_file(path: *const c_char, cap_p: CapT) -> c_int;
        pub fn cap_from_text(buf: *const c_char) -> CapT;
        pub fn cap_to_text(caps: CapT, length: *mut ssize_t) -> *mut c_char;
        pub fn cap_free(obj: *mut c_void) -> c_int;
    }
}

/// Get the calling process's POSIX.1e capability set as a text string.
/// Port of the `cap_get_proc()` + `cap_to_text()` pair the C source's
/// `bin_cap()` (Src/Modules/cap.c:36) calls when invoked with no
/// arguments — backs `cap` with no args.
#[cfg(all(target_os = "linux", feature = "libcap"))]
pub fn get_proc_caps() -> io::Result<String> {
    use std::ffi::CStr;

    unsafe {
        let caps = ffi::cap_get_proc();
        if caps.is_null() {
            return Err(io::Error::last_os_error());
        }

        let text = ffi::cap_to_text(caps, std::ptr::null_mut());
        if text.is_null() {
            ffi::cap_free(caps);
            return Err(io::Error::last_os_error());
        }

        let result = CStr::from_ptr(text).to_string_lossy().into_owned();
        ffi::cap_free(text as *mut libc::c_void);
        ffi::cap_free(caps);

        Ok(result)
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/cap.c`.
#[cfg(not(all(target_os = "linux", feature = "libcap")))]
pub fn get_proc_caps() -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "capabilities not supported (build with --features libcap on Linux)",
    ))
}

/// Set the calling process's POSIX.1e capability set from a text
/// representation.
/// Port of the `cap_from_text()` + `cap_set_proc()` pair the C
/// source's `bin_cap()` (Src/Modules/cap.c:36) calls when invoked
/// with one argument — backs `cap STRING`.
#[cfg(all(target_os = "linux", feature = "libcap"))]
pub fn set_proc_caps(cap_string: &str) -> io::Result<()> {
    use std::ffi::CString;

    let cap_c = CString::new(cap_string)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid capability string"))?;

    unsafe {
        let caps = ffi::cap_from_text(cap_c.as_ptr());
        if caps.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid capability string",
            ));
        }

        let result = ffi::cap_set_proc(caps);
        ffi::cap_free(caps);

        if result != 0 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/cap.c`.
#[cfg(not(all(target_os = "linux", feature = "libcap")))]
pub fn set_proc_caps(_cap_string: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "capabilities not supported (build with --features libcap on Linux)",
    ))
}

/// Get a file's POSIX.1e capability set as a text string.
/// Port of the `cap_get_file()` + `cap_to_text()` pair the C
/// source's `bin_getcap()` (Src/Modules/cap.c:68) calls per file
/// argument — backs `getcap FILE...`.
#[cfg(all(target_os = "linux", feature = "libcap"))]
pub fn get_file_caps(path: &str) -> io::Result<String> {
    use std::ffi::{CStr, CString};

    let path_c = CString::new(path)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid path"))?;

    unsafe {
        let caps = ffi::cap_get_file(path_c.as_ptr());
        if caps.is_null() {
            return Err(io::Error::last_os_error());
        }

        let text = ffi::cap_to_text(caps, std::ptr::null_mut());
        if text.is_null() {
            ffi::cap_free(caps);
            return Err(io::Error::last_os_error());
        }

        let result = CStr::from_ptr(text).to_string_lossy().into_owned();
        ffi::cap_free(text as *mut libc::c_void);
        ffi::cap_free(caps);

        Ok(result)
    }
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/cap.c`.
#[cfg(not(all(target_os = "linux", feature = "libcap")))]
pub fn get_file_caps(_path: &str) -> io::Result<String> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "capabilities not supported (build with --features libcap on Linux)",
    ))
}

/// `cap` builtin entry point.
/// Port of `bin_cap()` from Src/Modules/cap.c:36. With no args
/// prints `cap_get_proc()`; with one arg calls `cap_set_proc()` on
/// the parsed capability string.
pub fn bin_cap(args: &[&str]) -> (i32, String) {
    if args.is_empty() {
        match get_proc_caps() {
            Ok(caps) => (0, format!("{}\n", caps)),
            Err(e) => (1, format!("cap: {}\n", e)),
        }
    } else {
        match set_proc_caps(args[0]) {
            Ok(()) => (0, String::new()),
            Err(e) => (1, format!("cap: {}\n", e)),
        }
    }
}

/// `getcap` builtin entry point.
/// Port of `bin_getcap()` from Src/Modules/cap.c:68. Reports each
/// argument's file capabilities; missing-args case matches the C
/// source's "file required" error.
pub fn bin_getcap(args: &[&str]) -> (i32, String) {
    if args.is_empty() {
        return (1, "getcap: file required\n".to_string());
    }

    let mut output = String::new();
    let mut status = 0;

    for file in args {
        match get_file_caps(file) {
            Ok(caps) => output.push_str(&format!("{} {}\n", file, caps)),
            Err(e) => {
                output.push_str(&format!("getcap: {}: {}\n", file, e));
                status = 1;
            }
        }
    }

    (status, output)
}

/// `setcap` builtin entry point.
/// Port of `bin_setcap()` from Src/Modules/cap.c:91. Applies the
/// shared capability string (first arg) to every remaining file
/// argument. The per-file `cap_from_text()` + `cap_set_file()` +
/// `cap_free()` triple is inlined per the C source's loop body —
/// no helper function in C, no helper function here.
pub fn bin_setcap(args: &[&str]) -> (i32, String) {
    if args.len() < 2 {
        return (
            1,
            "setcap: capability string and file required\n".to_string(),
        );
    }

    let cap_string = args[0];
    let mut status = 0;
    let mut output = String::new();

    for file in &args[1..] {
        // Per-file body is the inlined `cap_from_text` /
        // `cap_set_file` / `cap_free` triple from the C source's
        // loop at Src/Modules/cap.c:91. The Linux+libcap path
        // calls real libcap; everything else returns Unsupported.
        let result: io::Result<()> = {
            #[cfg(all(target_os = "linux", feature = "libcap"))]
            {
                use std::ffi::CString;
                let cap_c = CString::new(cap_string).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid capability string")
                });
                let path_c = CString::new(*file).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidInput, "invalid path")
                });
                match (cap_c, path_c) {
                    (Ok(cap_c), Ok(path_c)) => unsafe {
                        let caps = ffi::cap_from_text(cap_c.as_ptr());
                        if caps.is_null() {
                            Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                "invalid capability string",
                            ))
                        } else {
                            let rc = ffi::cap_set_file(path_c.as_ptr(), caps);
                            ffi::cap_free(caps);
                            if rc != 0 {
                                Err(io::Error::last_os_error())
                            } else {
                                Ok(())
                            }
                        }
                    },
                    (Err(e), _) | (_, Err(e)) => Err(e),
                }
            }
            #[cfg(not(all(target_os = "linux", feature = "libcap")))]
            {
                Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "capabilities not supported (build with --features libcap on Linux)",
                ))
            }
        };

        if let Err(e) = result {
            output.push_str(&format!("setcap: {}: {}\n", file, e));
            status = 1;
        }
    }

    (status, output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_cap_no_args() {
        let (status, _) = bin_cap(&[]);
        #[cfg(not(all(target_os = "linux", feature = "libcap")))]
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_getcap_no_args() {
        let (status, _) = bin_getcap(&[]);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_setcap_no_args() {
        let (status, _) = bin_setcap(&[]);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_setcap_missing_file() {
        let (status, _) = bin_setcap(&["cap_net_admin+ep"]);
        assert_eq!(status, 1);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// cap / getcap / setcap — Linux capabilities (zsh/Src/Modules/cap.c).
    /// Routes through src/cap.rs which exposes get_proc_caps,
    /// set_proc_caps, get_file_caps, set_file_caps. On macOS or
    /// without the libcap feature, the underlying calls return
    /// io::Error(Unsupported).
    /// `cap` builtin — delegates to canonical port at
    /// `src/ported/modules/cap.rs:197` (`bin_cap()` from
    /// `Src/Modules/cap.c:36`). The duplicate body that lived here
    /// previously has been removed; this shim is the only entry
    /// point exec.rs exposes.
    pub(crate) fn bin_cap(&self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let (status, output) = crate::cap::bin_cap(&argv);
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
    /// `getcap` builtin — delegates to canonical port at
    /// `src/ported/modules/cap.rs:215` (`bin_getcap()` from
    /// `Src/Modules/cap.c:68`).
    pub(crate) fn bin_getcap(&self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let (status, output) = crate::cap::bin_getcap(&argv);
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
    /// `setcap` builtin — delegates to canonical port at
    /// `src/ported/modules/cap.rs:240` (`bin_setcap()` from
    /// `Src/Modules/cap.c:91`).
    pub(crate) fn bin_setcap(&self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let (status, output) = crate::cap::bin_setcap(&argv);
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
}
// END moved-from-exec-rs
