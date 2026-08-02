//! Port of `_options` from `Completion/Zsh/Type/_options`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:2
//! sh:3  # This should be used to complete all option names.
//! sh:4
//! sh:5  local expl
//! sh:6
//! sh:7  _wanted zsh-options expl 'zsh option' \
//! sh:8      compadd "$@" -M 'B:[nN][oO]= M:_= M:{A-Z}={a-z}' -k - options
//! ```
//!
//! `compadd -k options` reads keys of the shell-side `options`
//! associative array (the global option-name → on/off mapping).

use crate::compsys::ported::_wanted::_wanted;

/// `_options` — complete all zsh option names.
pub fn _options(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_options");
    // sh:7-8
    let mut wanted_argv: Vec<String> = vec![
        "zsh-options".to_string(),
        "expl".to_string(),
        "zsh option".to_string(),
        "compadd".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-M".to_string());
    wanted_argv.push("B:[nN][oO]= M:_= M:{A-Z}={a-z}".to_string());
    wanted_argv.push("-k".to_string());
    wanted_argv.push("-".to_string());
    wanted_argv.push("options".to_string());
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _options(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
