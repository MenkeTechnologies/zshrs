//! Port of `_delimiters` from `Completion/Zsh/Type/_delimiters`.
//!
//! Full upstream body (16 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # Simple function to offer delimiters for modifiers and qualifiers.
//! sh: 4  # Single argument is tag to use.
//! sh: 5
//! sh: 6  local expl
//! sh: 7  local -a list
//! sh: 8
//! sh: 9  zstyle -a ":completion:${curcontext}:$1" delimiters list ||
//! sh:10    list=(: + / - %)
//! sh:11
//! sh:12  if (( ${#list} )); then
//! sh:13    _wanted delimiters expl delimiter compadd -S '' -a list
//! sh:14  else
//! sh:15    _message delimiter
//! sh:16  fi
//! ```

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getsparam, setaparam};

/// `_delimiters` — offer the delimiter chars used in modifiers /
/// qualifiers. Reads the `delimiters` style for the caller's tag
/// (arg 0); falls back to `: + / - %`.
pub fn _delimiters(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_delimiters");
    // sh:6-7  locals
    let tag = args.first().cloned().unwrap_or_default();
    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:9-10
    let ctx = format!(":completion:{}:{}", curcontext, tag);
    let mut list = lookupstyle(&ctx, "delimiters");
    if list.is_empty() {
        list = vec![
            ":".to_string(),
            "+".to_string(),
            "/".to_string(),
            "-".to_string(),
            "%".to_string(),
        ];
    }

    // sh:12-16
    if !list.is_empty() {
        // sh:13  _wanted delimiters expl delimiter compadd -S '' -a list
        //   The `-a list` flag tells compadd to read from a shell
        //   array named `list`; publish ours under that name.
        setaparam("list", list);
        _wanted(&[
            "delimiters".to_string(),
            "expl".to_string(),
            "delimiter".to_string(),
            "compadd".to_string(),
            "-S".to_string(),
            "".to_string(),
            "-a".to_string(),
            "list".to_string(),
        ])
    } else {
        // sh:15  _message delimiter
        _message(&["delimiter".to_string()])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn falls_back_to_default_list_when_style_unset() {
        // sh:10 — when delimiters style is unset, falls back to
        //   `: + / - %`. We can't observe the list contents from
        //   here (it's published into `list` shell array), but we
        //   can verify the function publishes a non-empty array and
        //   takes the _wanted path (which returns 1 when no tags
        //   are registered, matching shell semantics).
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = _delimiters(&["mytag".to_string()]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
        // Verify the default list was published.
        let list = crate::ported::params::getaparam("list").unwrap_or_default();
        assert_eq!(list, vec![":", "+", "/", "-", "%"]);
    }
}
