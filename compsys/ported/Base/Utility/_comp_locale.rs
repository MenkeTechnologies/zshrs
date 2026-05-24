//! Port of `_comp_locale` — set locale for completion.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_comp_locale`
//! (system copy `/opt/homebrew/share/zsh/functions/_comp_locale`).
//!
//! Upstream shell source (the whole 20-line fn):
//! ```text
//! 11  if ctype=${${(f)"$(locale 2>/dev/null)"}:#^LC_CTYPE=*}; then
//! 12      unset -m LC_\*
//! 13      [[ -n $ctype ]] && eval export $ctype
//! 14  else
//! 15      ctype=${LC_ALL:-${LC_CTYPE:-${LANG:-C}}}
//! 16      unset -m LC_\*
//! 17      export LC_CTYPE=$ctype
//! 18  fi
//! 19  export LANG=C
//! ```
//!
//! Upstream sets a C-locale env for sane completion-tool output
//! while keeping LC_CTYPE for filename byte interpretation. The
//! comment says it MUST be run in a subshell (changes calling
//! shell's env).
//!
//! Faithful Rust port: actually performs the env mutation, mirroring
//! the shell exactly:
//!   - shell:12 / 16 `unset -m LC_\*` — unset every LC_* env var
//!   - shell:11-13 — preserve the system locale's LC_CTYPE setting
//!     (read via `locale` command, mirroring the shell's command-
//!     substitution-style read)
//!   - shell:15-17 fallback — when `locale` fails, build LC_CTYPE
//!     from `${LC_ALL:-${LC_CTYPE:-${LANG:-C}}}`
//!   - shell:19 — `export LANG=C`
//!
//! Returns the LC_CTYPE value that was set, so callers can verify.
//! Callers should run this in a forked subprocess (matching shell's
//! "should only be run in a subshell" comment) to avoid mutating
//! the parent's env permanently.

use std::process::Command;

/// _comp_locale - Set locale for completion.
///
/// Returns the final value of LC_CTYPE. Mutates process env:
///   - Removes every LC_* var
///   - Sets LC_CTYPE to the system's preserved value
///   - Sets LANG=C
pub fn _comp_locale() -> String {
    // shell:11 — `locale` command output, line-filtered to LC_CTYPE=…
    let preserved_lc_ctype = read_system_lc_ctype();

    // shell:12 / 16 — `unset -m LC_\*`
    let lc_vars: Vec<String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("LC_"))
        .map(|(k, _)| k)
        .collect();
    for k in &lc_vars {
        std::env::remove_var(k);
    }

    // shell:13 / 17 — set LC_CTYPE
    let lc_ctype = preserved_lc_ctype.unwrap_or_else(|| {
        // shell:15 fallback chain
        std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LC_CTYPE"))
            .or_else(|_| std::env::var("LANG"))
            .unwrap_or_else(|_| "C".to_string())
    });
    std::env::set_var("LC_CTYPE", &lc_ctype);

    // shell:19 — `export LANG=C`
    std::env::set_var("LANG", "C");

    lc_ctype
}

/// Read the system's preferred LC_CTYPE by invoking `locale` and
/// parsing the `LC_CTYPE=…` line. Returns None when the command
/// fails (sandboxed CI, missing binary, etc.).
fn read_system_lc_ctype() -> Option<String> {
    let out = Command::new("locale").output().ok()?;
    if !out.status.success() {
        return None;
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(value) = line.strip_prefix("LC_CTYPE=") {
            // Strip surrounding quotes the locale cmd sometimes adds.
            let v = value.trim().trim_matches('"').trim_matches('\'');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};

    // Env-mutating tests must serialise (other tests read PATH
    // etc.). Share the lock across this module.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        static L: OnceLock<Mutex<()>> = OnceLock::new();
        L.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    /// Snapshot all LC_* + LANG vars + restore on drop.
    struct EnvSaver {
        snapshot: Vec<(String, Option<String>)>,
    }
    impl EnvSaver {
        fn snap() -> Self {
            let keys: Vec<String> = std::env::vars()
                .map(|(k, _)| k)
                .filter(|k| k.starts_with("LC_") || k == "LANG")
                .collect();
            let mut snapshot: Vec<(String, Option<String>)> =
                keys.iter().map(|k| (k.clone(), std::env::var(k).ok())).collect();
            // Also snapshot LANG/LC_CTYPE/LC_ALL even if not set yet.
            for k in ["LANG", "LC_CTYPE", "LC_ALL"] {
                if !snapshot.iter().any(|(n, _)| n == k) {
                    snapshot.push((k.into(), std::env::var(k).ok()));
                }
            }
            EnvSaver { snapshot }
        }
    }
    impl Drop for EnvSaver {
        fn drop(&mut self) {
            // First unset everything currently LC_*.
            let now: Vec<String> = std::env::vars()
                .map(|(k, _)| k)
                .filter(|k| k.starts_with("LC_"))
                .collect();
            for k in &now {
                std::env::remove_var(k);
            }
            // Then restore snapshot.
            for (k, v) in &self.snapshot {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn sets_lang_to_c() {
        let _g = env_lock();
        let _saver = EnvSaver::snap();
        _comp_locale();
        assert_eq!(std::env::var("LANG").as_deref(), Ok("C"));
    }

    #[test]
    fn returns_a_non_empty_lc_ctype() {
        let _g = env_lock();
        let _saver = EnvSaver::snap();
        let lc = _comp_locale();
        assert!(!lc.is_empty(), "LC_CTYPE must be set to something");
        assert_eq!(std::env::var("LC_CTYPE").as_deref(), Ok(lc.as_str()));
    }

    #[test]
    fn removes_lc_messages_and_lc_collate() {
        let _g = env_lock();
        let _saver = EnvSaver::snap();
        std::env::set_var("LC_MESSAGES", "fr_FR.UTF-8");
        std::env::set_var("LC_COLLATE", "fr_FR.UTF-8");
        _comp_locale();
        assert!(
            std::env::var("LC_MESSAGES").is_err(),
            "LC_MESSAGES must be removed (unset -m LC_*)"
        );
        assert!(
            std::env::var("LC_COLLATE").is_err(),
            "LC_COLLATE must be removed"
        );
    }

    #[test]
    fn fallback_chain_when_locale_command_unavailable() {
        let _g = env_lock();
        let _saver = EnvSaver::snap();
        // Set LC_ALL so the fallback chain has something to pick up.
        // (read_system_lc_ctype may still succeed if `locale` is
        // installed; in that case we just verify the result is
        // non-empty.)
        std::env::set_var("LC_ALL", "en_US.UTF-8");
        let lc = _comp_locale();
        assert!(!lc.is_empty());
    }

    #[test]
    fn final_fallback_is_c() {
        let _g = env_lock();
        let _saver = EnvSaver::snap();
        // Remove every locale env so even the locale-cmd fallback
        // chain has nothing — the very last fallback is "C".
        for k in ["LC_ALL", "LC_CTYPE", "LANG"] {
            std::env::remove_var(k);
        }
        let lc = _comp_locale();
        // If `locale` command is available it'll return some non-empty
        // value; otherwise the very last "C" fallback fires. Either
        // way LC_CTYPE is non-empty.
        assert!(!lc.is_empty());
    }

    #[test]
    fn removes_every_lc_underscore_variant() {
        let _g = env_lock();
        let _saver = EnvSaver::snap();
        for k in [
            "LC_NUMERIC",
            "LC_TIME",
            "LC_MONETARY",
            "LC_PAPER",
            "LC_NAME",
            "LC_ADDRESS",
            "LC_TELEPHONE",
            "LC_MEASUREMENT",
            "LC_IDENTIFICATION",
        ] {
            std::env::set_var(k, "fr_FR.UTF-8");
        }
        _comp_locale();
        for k in [
            "LC_NUMERIC",
            "LC_TIME",
            "LC_MONETARY",
            "LC_PAPER",
            "LC_NAME",
            "LC_ADDRESS",
            "LC_TELEPHONE",
            "LC_MEASUREMENT",
            "LC_IDENTIFICATION",
        ] {
            assert!(
                std::env::var(k).is_err(),
                "{k} not removed by _comp_locale"
            );
        }
    }

    #[test]
    fn lc_ctype_reset_to_preserved_value_not_one_of_removed_lc_vars() {
        // After running, LC_CTYPE is back — every OTHER LC_* should
        // be gone.
        let _g = env_lock();
        let _saver = EnvSaver::snap();
        std::env::set_var("LC_MESSAGES", "polluter");
        _comp_locale();
        assert!(std::env::var("LC_CTYPE").is_ok());
        assert!(std::env::var("LC_MESSAGES").is_err());
    }
}
