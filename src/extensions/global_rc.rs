//! Runtime resolution of zsh's system-wide startup-file directory.
//!
//! **zshrs-original infrastructure — no C source counterpart, by
//! construction.** In C zsh the directory is not resolved at all: autoconf
//! substitutes `${sysconfdir}` into `GLOBAL_ZSHENV` / `GLOBAL_ZPROFILE` /
//! `GLOBAL_ZSHRC` / `GLOBAL_ZLOGIN` / `GLOBAL_ZLOGOUT` at *build* time
//! (`Src/init.c` then uses the resulting string literals verbatim), so each
//! zsh binary only ever knows the one path its packager configured. zshrs
//! ships a single binary to every platform and therefore has to make that
//! decision at run time — a question upstream never has to ask.
//!
//! The two layouts in the wild:
//!
//!   * `/etc/zshenv` — upstream's `--sysconfdir=/etc` default; macOS and
//!     the RPM distros.
//!   * `/etc/zsh/zshenv` — Debian, Ubuntu and Arch, which configure
//!     `--sysconfdir=/etc/zsh`.
//!
//! Hardcoding the upstream path meant zshrs silently skipped the
//! system-wide configuration on every Debian-family machine. It showed up
//! as an xtrace divergence on the ubuntu-latest CI runner, where
//! `zsh -fxc 'true'` traces
//! `+/etc/zsh/zshenv:15> [[ -z $PATH || $PATH == /bin:/usr/bin ]]`
//! (Debian's system zshenv seeding `$PATH`) and zshrs traced nothing at
//! all — it had never opened the file.

use crate::ported::config_h::{
    GLOBAL_ZLOGIN, GLOBAL_ZLOGOUT, GLOBAL_ZPROFILE, GLOBAL_ZSHENV, GLOBAL_ZSHRC,
};

/// The Debian/Ubuntu/Arch `--sysconfdir=/etc/zsh` directory.
const DEBIAN_SYSCONFDIR: &str = "/etc/zsh";

/// Map one compile-time `GLOBAL_Z*` default onto the path the running
/// platform's zsh would actually read.
///
/// The Debian directory wins when the file is present there, otherwise the
/// compiled-in `/etc` path stands. A real system carries exactly one of the
/// two — no zsh package installs both — so the probe is unambiguous, and a
/// machine with neither keeps the upstream path (which is what the caller
/// then finds missing and skips, as zsh does).
///
/// Paths that do not start with `/etc/` are returned unchanged: the caller
/// is naming something that is not a sysconfdir-relative startup file.
pub fn global_rc_path(default: &str) -> String {
    let Some(base) = default.strip_prefix("/etc/") else {
        return default.to_string();
    };
    let debian = format!("{DEBIAN_SYSCONFDIR}/{base}");
    if std::path::Path::new(&debian).exists() {
        debian
    } else {
        default.to_string()
    }
}

/// The five system-wide startup files in `Src/init.c` source order,
/// resolved for this platform: zshenv, zprofile, zshrc, zlogin, zlogout.
pub fn global_rc_chain() -> [String; 5] {
    [
        global_rc_path(GLOBAL_ZSHENV),
        global_rc_path(GLOBAL_ZPROFILE),
        global_rc_path(GLOBAL_ZSHRC),
        global_rc_path(GLOBAL_ZLOGIN),
        global_rc_path(GLOBAL_ZLOGOUT),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A non-sysconfdir path is not rewritten — the resolver only ever
    /// relocates `/etc/<file>`, never an arbitrary caller-supplied path.
    #[test]
    fn non_etc_paths_pass_through_untouched() {
        assert_eq!(
            global_rc_path("/usr/local/etc/zshenv"),
            "/usr/local/etc/zshenv"
        );
        assert_eq!(global_rc_path("zshenv"), "zshenv");
        assert_eq!(global_rc_path(""), "");
    }

    /// Whichever branch this platform takes, the result must name the same
    /// file — only the directory may differ. A resolver that returned a
    /// different stem would silently disable system-wide configuration,
    /// which is the exact bug this module exists to fix.
    #[test]
    fn resolution_preserves_the_file_name() {
        for default in global_rc_chain() {
            let stem = default.rsplit('/').next().expect("non-empty path");
            assert!(
                default == format!("/etc/{stem}")
                    || default == format!("{DEBIAN_SYSCONFDIR}/{stem}"),
                "{default:?} is neither the upstream nor the Debian location"
            );
        }
    }

    /// The chain must stay in `Src/init.c`'s source order — zshenv before
    /// zprofile before zshrc before zlogin before zlogout — because
    /// callers source it positionally.
    #[test]
    fn chain_is_in_init_c_source_order() {
        let chain = global_rc_chain();
        let names: Vec<&str> = chain
            .iter()
            .map(|p| p.rsplit('/').next().expect("non-empty path"))
            .collect();
        assert_eq!(names, ["zshenv", "zprofile", "zshrc", "zlogin", "zlogout"]);
    }
}
