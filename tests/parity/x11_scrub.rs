//! Keeps the parity suite away from X11.
//!
//! Every parity script runs under the developer's own environment, and on
//! macOS that environment carries two ways to start XQuartz from a test:
//!
//!   * `$DISPLAY` is a launchd socket (`…/org.xquartz:0`), so any X client a
//!     script happens to run (`xset`, `xdpyinfo`, a completer probing the
//!     display) makes launchd start the X server and wait on it;
//!   * `/opt/X11/bin` is on `$PATH`, and the volume is case-insensitive, so a
//!     bare `x` resolves to `/opt/X11/bin/X` — the X server itself, which
//!     blocks for minutes in BOTH shells (the alias suite hit exactly that).
//!
//! The scrub runs before `main`, while the process is still single-threaded,
//! so no test thread ever races a `setenv`. Both shells under comparison
//! inherit the cleaned environment, so it cannot skew a result.

/// The `$PATH` entries an X11 install adds.
const X11_DIRS: [&str; 2] = ["/opt/X11/bin", "/usr/X11/bin"];

fn scrubbed_path(path: &str) -> String {
    path.split(':')
        .filter(|dir| !X11_DIRS.contains(dir))
        .collect::<Vec<_>>()
        .join(":")
}

extern "C" fn scrub() {
    if let Some(path) = std::env::var_os("PATH").and_then(|p| p.into_string().ok()) {
        let clean = scrubbed_path(&path);
        if clean != path {
            if let Ok(value) = std::ffi::CString::new(clean) {
                // SAFETY: runs from the loader's init table, before any thread
                // exists; both strings are NUL-terminated.
                unsafe { libc::setenv(c"PATH".as_ptr(), value.as_ptr(), 1) };
            }
        }
    }
    // SAFETY: as above.
    unsafe { libc::unsetenv(c"DISPLAY".as_ptr()) };
}

#[used]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
#[cfg_attr(not(target_os = "macos"), link_section = ".init_array")]
static SCRUB_AT_LOAD: extern "C" fn() = scrub;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_x11_dirs_are_dropped_and_the_rest_keeps_its_order() {
        assert_eq!(
            scrubbed_path("/a:/opt/X11/bin:/b:/usr/X11/bin:/c"),
            "/a:/b:/c"
        );
        assert_eq!(scrubbed_path("/a:/b"), "/a:/b");
    }

    /// The point of the module: a script run by a parity test sees neither
    /// the X server on `$PATH` nor a display to start one for.
    #[test]
    fn child_shells_see_no_x11_path_and_no_display() {
        let out = std::process::Command::new("/bin/sh")
            .args(["-c", r#"printf '%s|%s' "$PATH" "${DISPLAY-unset}""#])
            .output()
            .expect("spawn sh");
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        let (path, display) = text.split_once('|').expect("two fields");
        assert!(
            !path.split(':').any(|d| X11_DIRS.contains(&d)),
            "an X11 dir is still on PATH: {path}"
        );
        assert_eq!(display, "unset", "DISPLAY leaked into the parity suite");
    }
}
