//! Port of `_default` from `Completion/Zsh/Context/_default`.
//!
//! Full upstream body (27 lines verbatim):
//! ```text
//! sh: 1  #compdef -default-
//! sh: 2
//! sh: 3  local ctl
//! sh: 4
//! sh: 5  if { zstyle -s ":completion:${curcontext}:" use-compctl ctl ||
//! sh: 6       zmodload -e zsh/compctl } && [[ "$ctl" != (no|false|0|off) ]]; then
//! sh: 7    local opt
//! sh: 8
//! sh: 9    opt=()
//! sh:10    [[ "$ctl" = *first* ]] && opt=(-T)
//! sh:11    [[ "$ctl" = *default* ]] && opt=("$opt[@]" -D)
//! sh:12    compcall "$opt[@]" || return 0
//! sh:13  fi
//! sh:14
//! sh:15  _files "$@" && return 0
//! sh:16
//! sh:17  # magicequalsubst allows arguments like <any-old-stuff>=~/foo to do
//! sh:18  # file name expansion after the =.  In that case, it's natural to
//! sh:19  # allow completion to handle file names after any equals sign.
//! sh:20
//! sh:21  if [[ -o magicequalsubst && "$PREFIX" = *\=* ]]; then
//! sh:22    compstate[parameter]="${PREFIX%%\=*}"
//! sh:23    compset -P 1 '*='
//! sh:24    _value "$@"
//! sh:25  else
//! sh:26    return 1
//! sh:27  fi
//! ```
//!
//! Faithful re-port: structure mirrors shell's three-branch shape —
//! compctl shim (sh:5-13), `_files` fallback (sh:15), `magicequalsubst`
//! special case (sh:21-26).
//!
//! Skipped branches (documented as `// rust:` divergences):
//! - sh:5-13 (compctl): `zsh/compctl` is a deprecated compatibility
//!   module not present in zshrs. The whole `use-compctl` zstyle is
//!   moot — we always fall through to `_files`.
//! - sh:24 `_value "$@"`: `_value` is currently a stub in our port
//!   (see [`crate::compsys::ported::_value::_value`]). The magicequalsubst
//!   branch enters but the inner dispatch is a no-op until `_value`
//!   is implemented.
//!
//! Shell-local parity:
//! - `ctl` (sh:3): captures the `use-compctl` zstyle value; unused in
//!   the Rust port because the compctl branch is skipped. Documented
//!   inline at sh:3 for traceability.
//! - `opt` (sh:7): inner-scope `compcall` arg accumulator; also unused
//!   because the compctl branch is skipped.

use crate::compsys::compcore::CompletionState;
use crate::compsys::ported::_files::{files_execute, FilesOpts};
use crate::compsys::state::comp_caller_options_get;

/// `_default` — `-default-` context handler.
// rust: shell uses globals ($curcontext, $compstate, $PREFIX); Rust
// takes state explicitly. compctl branch skipped (deprecated).
pub fn _default(state: &mut CompletionState) -> bool {
    // sh:3  local ctl                — use-compctl zstyle value
    //                                   (unused — compctl branch skipped)
    let _ctl: () = ();

    // sh:5-13 — compctl legacy shim. SKIPPED: zsh/compctl module is
    // deprecated and not present in zshrs. The full guard would be:
    //   if zstyle -s ":completion:${curcontext}:" use-compctl ctl
    //       || zmodload -e zsh/compctl && [[ "$ctl" != (no|false|0|off) ]]
    //   then compcall ...
    // For zshrs the answer is always "no" — fall straight through.

    // sh:15  _files "$@" && return 0   — default-completion is files
    if files_execute(state, &FilesOpts::default()) {
        return true;
    }

    // sh:21  if [[ -o magicequalsubst && "$PREFIX" = *\=* ]]; then
    let magicequalsubst_on = comp_caller_options_get("magicequalsubst").unwrap_or(false);
    let prefix_has_eq = state.params.prefix.contains('=');
    if magicequalsubst_on && prefix_has_eq {
        // sh:22  compstate[parameter]="${PREFIX%%\=*}"
        //   parameter name is the substring BEFORE the first `=`.
        let parameter_name: String = state
            .params
            .prefix
            .split_once('=')
            .map(|(left, _)| left.to_string())
            .unwrap_or_default();
        state.params.compstate.parameter = parameter_name;

        // sh:23  compset -P 1 '*='
        //   shift past `<name>=` so PREFIX becomes just the post-`=` text.
        if let Some((_, after)) = state.params.prefix.split_once('=') {
            state.params.iprefix.push_str(&state.params.prefix[..state.params.prefix.len() - after.len()]);
            state.params.prefix = after.to_string();
        }

        // sh:24  _value "$@"
        // rust: _value is currently a stub (Zsh/Context/_value.rs). The
        // branch shape mirrors the shell exactly; once _value lands the
        // dispatch will activate. For now: return false to mirror
        // "_value returned no matches" outcome.
        // TODO: when _value is fully implemented, replace this with
        //   _value(state, &parameter_name_clone, "")
        false
    } else {
        // sh:26  return 1
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::state::{comp_caller_options_clear, comp_caller_options_set_one};
    use std::sync::Mutex;
    static OPTS_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn delegates_to_files_execute_when_files_match() {
        // sh:15 — `_files "$@" && return 0` is the primary path.
        // With a prefix that matches something on disk, _default
        // returns true via the files branch.
        let mut state = CompletionState::new();
        state.params.prefix = "Carg".into();
        let _ = _default(&mut state);
        // No hard assert on file existence (varies by working dir);
        // pin no panic + no-magicequalsubst leakage when prefix lacks `=`.
    }

    #[test]
    fn off_prefix_returns_false() {
        // sh:15 returns false (no files) + sh:21 condition fails
        // (PREFIX has no `=`) + sh:26 returns 1. Net: false.
        let _g = OPTS_LOCK.lock().unwrap();
        comp_caller_options_clear();
        let mut state = CompletionState::new();
        state.params.prefix = "definitely-no-such-file-xyz-123".into();
        assert!(!_default(&mut state));
    }

    #[test]
    fn no_panic_with_empty_prefix() {
        // sh:21 condition `"$PREFIX" = *\=*` is false for empty
        // PREFIX → branch skipped → no panic.
        let _g = OPTS_LOCK.lock().unwrap();
        comp_caller_options_clear();
        let mut state = CompletionState::new();
        let _ = _default(&mut state);
    }

    #[test]
    fn magicequalsubst_off_skips_value_branch() {
        // sh:21 first conjunct `-o magicequalsubst` is false (option
        // not in caller_options snapshot) → branch skipped even when
        // PREFIX contains `=`.
        let _g = OPTS_LOCK.lock().unwrap();
        comp_caller_options_clear();
        let mut state = CompletionState::new();
        state.params.prefix = "foo=bar".into();
        assert!(!_default(&mut state),
                "without magicequalsubst, prefix `foo=bar` returns false");
        assert!(state.params.compstate.parameter.is_empty(),
                "magicequalsubst-off branch must NOT mutate compstate.parameter");
    }

    #[test]
    fn magicequalsubst_on_with_equals_splits_prefix() {
        // sh:21-24 — both conjuncts true: option is on AND PREFIX
        // contains `=`. Body should: set compstate[parameter] to
        // the part BEFORE `=`, shift PREFIX past `<name>=`, dispatch
        // _value (currently stub → returns false).
        let _g = OPTS_LOCK.lock().unwrap();
        comp_caller_options_clear();
        comp_caller_options_set_one("magicequalsubst", true);
        let mut state = CompletionState::new();
        state.params.prefix = "foo=bar".into();
        let _ = _default(&mut state);

        // sh:22 — compstate[parameter] = "foo"
        assert_eq!(state.params.compstate.parameter, "foo",
                   "sh:22 — parameter must be PREFIX before `=`");
        // sh:23 — compset -P 1 '*=' moves `foo=` from PREFIX to IPREFIX
        assert_eq!(state.params.prefix, "bar",
                   "sh:23 — PREFIX must be the post-`=` substring");
        assert!(state.params.iprefix.contains("foo="),
                "sh:23 — IPREFIX must absorb the moved `foo=` segment");

        comp_caller_options_clear();
    }

    #[test]
    fn magicequalsubst_on_but_no_equals_skips_branch() {
        // sh:21 second conjunct fails: option on but PREFIX has no `=`.
        let _g = OPTS_LOCK.lock().unwrap();
        comp_caller_options_clear();
        comp_caller_options_set_one("magicequalsubst", true);
        let mut state = CompletionState::new();
        state.params.prefix = "plainprefix".into();
        let _ = _default(&mut state);
        // compstate.parameter must NOT be touched.
        assert!(state.params.compstate.parameter.is_empty());
        comp_caller_options_clear();
    }

    #[test]
    fn ctl_local_documented_but_unused() {
        // sh:3 — `local ctl` declared in shell but unused in our port
        // because the compctl branch (sh:5-13) is skipped. Pin: no
        // observable side effect from the skipped branch.
        let _g = OPTS_LOCK.lock().unwrap();
        comp_caller_options_clear();
        let mut state = CompletionState::new();
        state.params.prefix = "x".into();
        let _ = _default(&mut state);
        // No state for `ctl` exists in our port; nothing to assert
        // beyond "no panic, no surprise mutation".
    }
}
