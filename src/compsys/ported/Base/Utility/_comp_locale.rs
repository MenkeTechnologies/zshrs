//! Port of `_comp_locale` from `Completion/Base/Utility/_comp_locale`.
//!
//! Full upstream body (20 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # Arrange that LC_CTYPE retains the current setting so characters in
//! sh: 4  # file names are handled properly, but other locales are set to C
//! sh: 7  # This exports new locale settings, so should only
//! sh: 8  # be run in a subshell.  A typical use is in a $(...).
//! sh:10  local ctype
//! sh:12  if ctype=${${(f)"$(locale 2>/dev/null)"}:#^LC_CTYPE=*}; then
//! sh:13      unset -m LC_\*
//! sh:14      [[ -n $ctype ]] && eval export $ctype
//! sh:15  else
//! sh:16      ctype=${LC_ALL:-${LC_CTYPE:-${LANG:-C}}}
//! sh:17      unset -m LC_\*
//! sh:18      export LC_CTYPE=$ctype
//! sh:19  fi
//! sh:20  export LANG=C
//! ```
//!
//! sh:8 doc: "should only be run in a subshell". The fork happens in
//! `$(...)` substitution. In the Rust port we manipulate the
//! process-level env directly; callers MUST run this in a
//! short-lived child (e.g. via command substitution) to avoid
//! polluting the parent shell's locale.

use std::env;

/// `_comp_locale` — reset all LC_* vars to C except LC_CTYPE
/// (preserved from the current process env), and set LANG=C.
pub fn _comp_locale() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_comp_locale");
    // sh:12 — derive LC_CTYPE preference (process-level env, since
    //   `$(locale)` would just dump our own env).
    let ctype = env::var("LC_ALL")
        .ok()
        .filter(|s| !s.is_empty())
        .or_else(|| env::var("LC_CTYPE").ok().filter(|s| !s.is_empty()))
        .or_else(|| env::var("LANG").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "C".to_string());

    // sh:13 / sh:17  unset -m LC_*
    let lc_vars: Vec<String> = env::vars()
        .filter_map(|(k, _)| if k.starts_with("LC_") { Some(k) } else { None })
        .collect();
    for k in lc_vars {
        env::remove_var(&k);
    }

    // sh:18
    env::set_var("LC_CTYPE", ctype);
    // sh:20
    env::set_var("LANG", "C");

    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `_comp_locale` exports into the process environment, as C's does into
    /// its subshell. A test that left `LANG=C` behind changed the collation of
    /// every later `setlocale(LC_ALL, "")` in the binary (each executor init
    /// runs one), so an unrelated sort test saw `C` order half-way through.
    /// Put every variable the call can touch back on drop.
    struct LocaleEnv(Vec<(String, Option<String>)>);

    impl LocaleEnv {
        fn save() -> Self {
            let mut saved: Vec<(String, Option<String>)> = env::vars()
                .filter(|(k, _)| k.starts_with("LC_"))
                .map(|(k, v)| (k, Some(v)))
                .collect();
            for k in ["LANG", "LC_ALL", "LC_CTYPE"] {
                if !saved.iter().any(|(s, _)| s == k) {
                    saved.push((k.to_string(), env::var(k).ok()));
                }
            }
            LocaleEnv(saved)
        }
    }

    impl Drop for LocaleEnv {
        fn drop(&mut self) {
            for (k, _) in env::vars().filter(|(k, _)| k.starts_with("LC_")) {
                env::remove_var(k);
            }
            for (k, v) in &self.0 {
                match v {
                    Some(v) => env::set_var(k, v),
                    None => env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn sets_lang_to_c() {
        let _g = crate::test_util::global_state_lock();
        let _env = LocaleEnv::save();
        env::set_var("LANG", "en_US.UTF-8");
        let _ = _comp_locale();
        assert_eq!(env::var("LANG").as_deref(), Ok("C"));
    }

    #[test]
    fn preserves_lc_ctype_from_pre_existing_var() {
        let _g = crate::test_util::global_state_lock();
        let _env = LocaleEnv::save();
        env::set_var("LC_CTYPE", "en_US.UTF-8");
        env::remove_var("LC_ALL");
        let _ = _comp_locale();
        assert_eq!(env::var("LC_CTYPE").as_deref(), Ok("en_US.UTF-8"));
    }
}
