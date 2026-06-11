//! Port of `_guard` from `Completion/Base/Utility/_guard`.
//!
//! Full upstream body (12 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local garbage
//! sh: 4
//! sh: 5  zparseopts -K -D -a garbage M+: J+: V+: 1 2 o+: n F: X+:
//! sh: 6
//! sh: 7  [[ "$PREFIX$SUFFIX" != $~1 ]] && return 1
//! sh: 8
//! sh: 9  shift
//! sh:10  _message -e "$*"
//! sh:11
//! sh:12  [[ -n "$PREFIX$SUFFIX" ]]
//! ```
//!
//! `zparseopts` swallows a parade of compadd-passthrough flags into
//! `garbage` (we discard). Then matches `$PREFIX$SUFFIX` against
//! the first arg (interpreted as a glob via `$~1`); on mismatch
//! returns 1. Otherwise emits a description message and returns 0
//! iff anything is typed.

use crate::compsys::ported::_message::_message;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:5 — bridge to real `bin_zparseopts -K -D -a garbage M+: J+:
/// V+: 1 2 o+: n F: X+:` via `-v <name>`. Returns leftover argv.
fn run_zparseopts_guard(args: &[String]) -> Vec<String> {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("garbage", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-K".to_string(),
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "garbage".to_string(),
            "M+:".to_string(),
            "J+:".to_string(),
            "V+:".to_string(),
            "1".to_string(),
            "2".to_string(),
            "o+:".to_string(),
            "n".to_string(),
            "F:".to_string(),
            "X+:".to_string(),
        ],
        &make_ops(),
        0,
    );
    getaparam(src).unwrap_or_default()
}

/// `_guard` — accept completion iff `$PREFIX$SUFFIX` matches the
/// supplied pattern; returns 0 only when something is typed.
pub fn _guard(args: &[String]) -> i32 {
    // sh:5
    let mut argv = run_zparseopts_guard(args);

    // sh:7  [[ "$PREFIX$SUFFIX" != $~1 ]] && return 1
    let pattern = argv.first().cloned().unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let combined = format!("{}{}", prefix, suffix);
    let matched = if let Some(prog) = patcompile(&{ let mut __pat_tok = (&pattern).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, 0, None) {
        pattry(&prog, &combined)
    } else {
        pattern == combined
    };
    if !matched {
        return 1;
    }

    // sh:9-10
    if !argv.is_empty() {
        argv.remove(0); // shift
    }
    let msg = argv.join(" ");
    let _ = _message(&["-e".to_string(), msg]);

    // sh:12  return code = "$PREFIX$SUFFIX" non-empty
    if combined.is_empty() {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn empty_prefix_suffix_returns_one() {
        // sh:12 — empty `$PREFIX$SUFFIX` → return 1 (no input).
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let r = _guard(&["*".to_string(), "anything".to_string()]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn pattern_mismatch_returns_one() {
        // sh:7 — `$PREFIX$SUFFIX` doesn't match pattern → return 1.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = setsparam("PREFIX", "hello");
        let _ = setsparam("SUFFIX", "");
        let r = _guard(&["[0-9]*".to_string(), "number".to_string()]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 1);
    }

    #[test]
    fn pattern_match_returns_zero_when_input_present() {
        // sh:12 — matched pattern AND non-empty input → return 0.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _ = setsparam("PREFIX", "42");
        let _ = setsparam("SUFFIX", "");
        let r = _guard(&["[0-9]*".to_string(), "number".to_string()]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
        assert_eq!(r, 0);
    }
}
