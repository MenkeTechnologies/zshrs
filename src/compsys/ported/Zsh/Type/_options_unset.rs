//! Port of `_options_unset` from `Completion/Zsh/Type/_options_unset`.
//!
//! Full upstream body (10 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Complete all unset options. This relies on `_main_complete' to store the
//! sh: 4  # names of the options that were unset when it was called in the array
//! sh: 5  # `_options_unset'.
//! sh: 6
//! sh: 7  local expl
//! sh: 8
//! sh: 9  _wanted zsh-options expl 'unset zsh option' \
//! sh:10      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -a - _options_unset
//! ```

use crate::compsys::ported::_wanted::wanted_byname;

/// `_options_unset` — complete names of zsh options currently unset.
/// Returns `_wanted`'s exit code.
pub fn _options_unset(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_options_unset");
    // sh:9-10
    let mut wanted_argv: Vec<String> = vec![
        "zsh-options".to_string(),
        "expl".to_string(),
        "unset zsh option".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-M".to_string());
    wanted_argv.push("B:[nN][oO]= M:_= M:{A-Z}={a-z}".to_string());
    wanted_argv.push("-a".to_string());
    wanted_argv.push("-".to_string());
    wanted_argv.push("_options_unset".to_string());
    wanted_byname(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_nonzero_with_no_matches_registered() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _options_unset(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
