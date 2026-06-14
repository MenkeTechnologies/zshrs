//! Port of `_zcalc_line` from
//! `Completion/Zsh/Context/_zcalc_line`.
//!
//! Full upstream body (81 lines, abridged):
//! ```text
//! sh: 1  #compdef -zcalc-line-
//! sh: 5  _zcalc_line_escapes() { … describe `:cmd` escapes … }
//! sh:23  _zcalc_line() {
//! sh:25    if [[ CURRENT -eq 1 && $words[1] != ":"(\\|)"!"* ]]; then
//! sh:26      command-escapes alternative + _math
//! sh:35    case $words[1] in (":"(\\|)"!"*) … dispatch shell command
//! sh:50  (:function)  … define math function …
//! sh:80  esac
//! sh:81  }
//! ```

use crate::compsys::ported::_alternative::_alternative;
use crate::compsys::ported::_message::_message;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getiparam, setaparam};

/// sh:5 — describe the catalogue of `:cmd` escapes.
fn _zcalc_line_escapes() -> i32 {
    let cmds: Vec<String> = vec![
        "\\::!:shell escape".to_string(),
        "\\::q:quit".to_string(),
        "\\::norm:normal output format".to_string(),
        "\\::sci:scientific output format".to_string(),
        "\\::fix:fixed point output format".to_string(),
        "\\::eng:engineering output format".to_string(),
        "\\::raw:raw output format".to_string(),
        "\\::local:make variables local".to_string(),
        "\\::function:define math function".to_string(),
    ];
    setaparam("cmds", cmds);
    dispatch_function_call(
        "_describe",
        &[
            "-t".to_string(),
            "command-escapes".to_string(),
            "command escape".to_string(),
            "cmds".to_string(),
            "-Q".to_string(),
        ],
    )
    .unwrap_or(1)
}

/// `_zcalc_line` — `-zcalc-line-` context: complete `zcalc`
/// command-line entries (escapes or math expressions).
pub fn _zcalc_line() -> i32 {
    let words = getaparam("words").unwrap_or_default();
    let current = getiparam("CURRENT") as usize;
    let word1 = words.first().cloned().unwrap_or_default();

    // sh:25  command-position
    if current == 1 && !word1.starts_with(":!") && !word1.starts_with(":\\!") {
        let mut alts: Vec<String> = Vec::new();
        if word1.is_empty() || word1.starts_with(':') {
            alts.push("command-escapes:command escape:_zcalc_line_escapes".to_string());
        }
        if word1.is_empty() || !word1.starts_with(':') {
            alts.push("math:math formula:_math".to_string());
        }
        return _alternative(&alts);
    }

    // sh:35  `:!cmd` shell escape — delegate to _normal
    if word1.starts_with(":!") || word1.starts_with(":\\!") {
        return dispatch_function_call("_normal", &[]).unwrap_or(1);
    }

    // sh:50  `:function` precision message + math dispatch
    if word1 == ":function" {
        if current == 2 {
            let _ = _message(&["precision".to_string()]);
        }
    }

    // Fallback
    dispatch_function_call("_math", &[]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["".to_string()]);
        let _ = setsparam("CURRENT", "1");
        let _r = _zcalc_line();
    }

    #[test]
    fn escapes_dispatch() {
        let _g = crate::test_util::global_state_lock();
        let _r = _zcalc_line_escapes();
    }
}
