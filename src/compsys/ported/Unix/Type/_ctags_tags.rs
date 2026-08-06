//! Port of `_ctags_tags` from `Completion/Unix/Type/_ctags_tags`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh:1  #autoload
//! sh:3  local expl tags
//! sh:5  [[ -r tags ]] && tags=( ${${${(f)"$(< tags)"}:#!*}%%[[:blank:]]*} )
//! sh:7  _wanted ctags expl 'tag' compadd -M 'm:{a-zA-Z}={A-Za-z}' -a "$@" - tags
//! ```
//!
//! sh:5 — read the `tags` file (if readable), split on newlines, drop
//! lines beginning with `!` (ctags metadata), and keep each line's first
//! whitespace-delimited field (the tag name). The `-a tags` form of
//! compadd reads a NAMED array, so the port publishes `tags` as an array
//! parameter before dispatching `_wanted`.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::setaparam;

/// `_ctags_tags` — complete tag names from a ctags `tags` file in $PWD.
pub fn _ctags_tags(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ctags_tags");
    // sh:5  [[ -r tags ]] && tags=( ${${${(f)"$(< tags)"}:#!*}%%[[:blank:]]*} )
    let tags: Vec<String> = std::fs::read_to_string("tags")
        .map(|body| {
            body.lines()
                .filter(|l| !l.starts_with('!')) // :#!* — drop metadata lines
                .map(|l| {
                    // %%[[:blank:]]* — first blank-delimited field.
                    l.split([' ', '\t']).next().unwrap_or("").to_string()
                })
                .filter(|t| !t.is_empty())
                .collect()
        })
        .unwrap_or_default();
    setaparam("tags", tags);

    // sh:7  _wanted ctags expl 'tag' compadd -M 'm:{a-zA-Z}={A-Za-z}' -a "$@" - tags
    let mut wanted_argv: Vec<String> = vec![
        "ctags".to_string(),
        "expl".to_string(),
        "tag".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "m:{a-zA-Z}={A-Za-z}".to_string(),
        "-a".to_string(),
    ];
    wanted_argv.extend(args.iter().cloned());
    wanted_argv.push("-".to_string());
    wanted_argv.push("tags".to_string());
    _wanted(&wanted_argv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        // No `tags` file / no completion context → _wanted's tag guard fails.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(1, std::sync::atomic::Ordering::Relaxed);
        let r = _ctags_tags(&[]);
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(r, 1);
    }
}
