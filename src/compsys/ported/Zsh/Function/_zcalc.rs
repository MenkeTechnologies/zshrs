//! Port of `_zcalc` from `Completion/Zsh/Function/_zcalc`.
//!
//! Completion for the `zcalc` function. A thin `_arguments` wrapper: the
//! upstream body is one call with a fixed set of option specs and ignores
//! its own positional args.
//!
//! Full upstream body (9 lines, abridged — head is `#compdef zcalc`):
//! ```text
//! sh:3  _arguments -s -w -S : \
//! sh:4    '-#[specify default base]:base: ' \
//! sh:5    '-f[force floating point for all expressions]' \
//! sh:6    '-e[treat command line as expressions to be output immediately]' \
//! sh:7    '-r[enable Reverse Polish Notation]' \
//! sh:8    '*:expression: '
//! ```

use crate::compsys::ported::_arguments::_arguments;

/// sh:3-8 — the fixed `_arguments` invocation words: the `-s -w -S :`
/// leading flags followed by the five option/positional specs, verbatim.
fn zcalc_argv() -> Vec<String> {
    [
        "-s",
        "-w",
        "-S",
        ":",
        "-#[specify default base]:base: ",
        "-f[force floating point for all expressions]",
        "-e[treat command line as expressions to be output immediately]",
        "-r[enable Reverse Polish Notation]",
        "*:expression: ",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// `_zcalc` — completion for the `zcalc` function.
pub fn _zcalc(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_zcalc");
    // sh:3-8 — the upstream function ignores its positional args and calls
    // `_arguments` with a fixed spec list.
    _arguments(&zcalc_argv())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn argv_matches_upstream_specs() {
        let v = zcalc_argv();
        // sh:3 — leading `_arguments` flags then the terminating `:`.
        assert_eq!(&v[..4], &["-s", "-w", "-S", ":"]);
        // sh:4-8 — the five specs, in order, verbatim.
        assert_eq!(v[4], "-#[specify default base]:base: ");
        assert_eq!(v[5], "-f[force floating point for all expressions]");
        assert_eq!(
            v[6],
            "-e[treat command line as expressions to be output immediately]"
        );
        assert_eq!(v[7], "-r[enable Reverse Polish Notation]");
        assert_eq!(v[8], "*:expression: ");
        assert_eq!(v.len(), 9);
    }

    #[test]
    fn returns_one_without_completion_context() {
        // Mirror the `_arguments` unit tests: with a minimal words/CURRENT
        // state but no live completion context, comparguments fails and the
        // wrapper falls through to `return 1` (sh:588 in `_arguments`).
        use crate::ported::params::{setaparam, setiparam};
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["zcalc".to_string(), "".to_string()]);
        let _ = setiparam("CURRENT", 2);
        assert_eq!(_zcalc(&[]), 1);
    }
}
