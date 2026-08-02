//! Port of `_functions` from `Completion/Zsh/Type/_functions`.
//!
//! Full upstream body (8 lines verbatim):
//! ```text
//! sh:1  #compdef unfunction
//! sh:2
//! sh:3  local expl ffilt
//! sh:4
//! sh:5  zstyle -t ":completion:${curcontext}:functions" prefix-needed && \
//! sh:6   [[ $PREFIX != [_.]* ]] && \
//! sh:7   ffilt='[(I)[^_.]*]'
//! sh:8
//! sh:9  _wanted functions expl 'shell function' compadd -k "$@" - "functions$ffilt"
//! ```
//!
//! `compadd -k functions$ffilt` — the `-k` flag tells compadd to
//! read keys from the named associative array; with a subscript
//! filter `[(I)[^_.]*]` it keeps only names not starting with `_`
//! or `.`. The full subscripted name is passed as a literal arg.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::getsparam;

/// `_functions` — `unfunction` completion: emit shell function
/// names. When the `prefix-needed` style is true AND `$PREFIX`
/// doesn't already start with `_` or `.`, filter out functions
/// starting with those chars.
pub fn _functions(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_functions");
    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:5-7
    let style_ctx = format!(":completion:{}:functions", curcontext);
    let prefix_needed = testforstyle(&style_ctx, "prefix-needed") == 0;
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let prefix_filtered = prefix_needed && !prefix.starts_with('_') && !prefix.starts_with('.');
    let ffilt = if prefix_filtered { "[(I)[^_.]*]" } else { "" };

    // sh:9
    let mut wanted_argv: Vec<String> = vec![
        "functions".to_string(),
        "expl".to_string(),
        "shell function".to_string(),
        "compadd".to_string(),
        "-k".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    wanted_argv.push(format!("functions{}", ffilt));
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
        let r = _functions(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
