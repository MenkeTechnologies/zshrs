//! Port of `_brace_parameter` from
//! `Completion/Zsh/Context/_brace_parameter`.
//!
//! Full upstream body (214 lines, abridged):
//! ```text
//! sh:  1  #compdef -brace-parameter-
//! sh:  4  if compset -P '\(*\)*'; then … : prefix flag → _parameter ; return
//! sh: 14  elif compset -P '\(*'; then … parameter-flag catalog
//! sh: 80  elif compset -P '#%[#%]##'; then  … offset / length modifiers
//! sh:130  elif compset -P '\\/[^/]##'; then  … substitution
//! sh:140  else _parameter
//! sh:214  fi
//! ```
//!
//! `${…}` parameter-expansion completion. Dispatches to
//! `_parameter` for the trivial case; flag-letter catalog for
//! `${(F)…}` etc.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::setaparam;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_brace_parameter` — complete inside `${…}`.
pub fn _brace_parameter() -> i32 {
    // sh:4
    if bin_compset(
        "compset",
        &["-P".to_string(), "\\(*\\)*".to_string()],
        &make_ops(),
        0,
    ) == 0
    {
        return dispatch_function_call("_parameter", &[]).unwrap_or(1);
    }

    // sh:14  inside `(...)` — parameter flag catalog
    if bin_compset(
        "compset",
        &["-P".to_string(), "\\(*".to_string()],
        &make_ops(),
        0,
    ) == 0
    {
        let flags: Vec<String> = vec![
            "A:create assoc array from key/val pairs",
            "a:array (with @)",
            "C:capitalize words",
            "D:turn HOME prefix into ~",
            "e:treat $\\(…\\) as command substitution",
            "f:split on newlines",
            "F:join on newlines",
            "g:process escape sequences",
            "i:case-insensitive sort",
            "j:join with given string",
            "k:expand to keys of associative array",
            "L:lowercase result",
            "l:left-justify",
            "m:multibyte-aware width",
            "n:numerically-sort",
            "O:reverse-sort",
            "o:sort",
            "P:treat as parameter name; recursive lookup",
            "p:recognize escape sequences in next flag",
            "q:quote special chars",
            "s:split on given string",
            "t:type of param",
            "U:uppercase result",
            "u:unique",
            "V:visible (printable)",
            "v:return values not keys (with k)",
            "w:treat as words",
            "X:fail on bad pattern",
            "Z:split with shell syntax",
            "z:split on shell tokens",
        ]
        .into_iter()
        .map(|s| s.to_string())
        .collect();
        setaparam("flags", flags);
        return dispatch_function_call(
            "_describe",
            &[
                "-t".to_string(),
                "flags".to_string(),
                "parameter flag".to_string(),
                "flags".to_string(),
                "-Q".to_string(),
                "-S".to_string(),
                "".to_string(),
            ],
        )
        .unwrap_or(1);
    }

    // sh:214 — fallback: parameter-name completion
    dispatch_function_call("_parameter", &[]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "myvar");
        assert_eq!(_brace_parameter(), 1);
    }
}
