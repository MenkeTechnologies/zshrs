//! Port of `_options_set` from `Completion/Zsh/Type/_options_set`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete all set options. This relies on `_main_complete' to store the
//! sh: 4  # names of the options that were set when it was called in the array
//! sh: 5  # `_options_set'.
//! sh: 6
//! sh: 7  local expl
//! sh: 8
//! sh: 9  _wanted zsh-options expl 'set zsh option' \
//! sh:10      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -a - _options_set
//! ```
//!
//! Calls real `_wanted` (in `compsys::ported::_wanted`). The
//! `-a _options_set` flag tells `compadd` to read matches from the
//! shell array named `_options_set` (populated by `_main_complete`
//! at engine startup).

use crate::compsys::ported::_wanted::wanted_byname;

/// `_options_set` — complete names of zsh options that are currently
/// set. Returns `_wanted`'s exit code.
pub fn _options_set(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_options_set");
    // sh:9-10
    let mut wanted_argv: Vec<String> = vec![
        "zsh-options".to_string(),
        "expl".to_string(),
        "set zsh option".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-M".to_string());
    wanted_argv.push("B:[nN][oO]= M:_= M:{A-Z}={a-z}".to_string());
    wanted_argv.push("-a".to_string());
    wanted_argv.push("-".to_string());
    wanted_argv.push("_options_set".to_string());
    wanted_byname(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_nonzero_with_no_matches_registered() {
        // sh:9 — without any tag/spec pre-registered by _main_complete
        //   AND no shell-side _options_set array, _wanted returns 1.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _options_set(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
