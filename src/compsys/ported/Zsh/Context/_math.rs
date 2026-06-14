//! Port of `_math` from `Completion/Zsh/Context/_math`.
//!
//! Full upstream body (14 lines verbatim):
//! ```text
//! sh: 1  #compdef -math- let
//! sh: 2
//! sh: 3  if [[ "$PREFIX" = *[^a-zA-Z0-9_]* ]]; then
//! sh: 4    IPREFIX="$IPREFIX${PREFIX%%[a-zA-Z0-9_]#}"
//! sh: 5    PREFIX="${PREFIX##*[^a-zA-Z0-9_]}"
//! sh: 6  fi
//! sh: 7  if [[ "$SUFFIX" = *[^a-zA-Z0-9_]* ]]; then
//! sh: 8    ISUFFIX="${SUFFIX##[a-zA-Z0-9_]#}$ISUFFIX"
//! sh: 9    SUFFIX="${SUFFIX%%[^a-zA-Z0-9_]*}"
//! sh:10  fi
//! sh:11
//! sh:12  _alternative 'math-parameters:math parameter: _math_params' \
//! sh:13      'user-math-functions:user math function: _user_math_func' \
//! sh:14      'module-math-functions:math function from zsh/mathfunc: _module_math_func'
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setsparam};

fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `_math` — `(( … ))` math-context completion: trim non-ident
/// chars from PREFIX/SUFFIX into IPREFIX/ISUFFIX, then offer math
/// param + math-fn alternatives.
pub fn _math() -> i32 {
    // sh:3-6  PREFIX has non-ident → split off the trailing ident
    //         portion as the new PREFIX; the leading part joins
    //         IPREFIX.
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if prefix.chars().any(|c| !is_ident_char(c)) {
        let trail_start = prefix
            .rfind(|c: char| !is_ident_char(c))
            .map(|i| i + 1)
            .unwrap_or(0);
        let head = &prefix[..trail_start];
        let tail = &prefix[trail_start..];
        let ipref = getsparam("IPREFIX").unwrap_or_default();
        let _ = setsparam("IPREFIX", &format!("{}{}", ipref, head));
        let _ = setsparam("PREFIX", tail);
    }

    // sh:7-10  symmetric for SUFFIX
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    if suffix.chars().any(|c| !is_ident_char(c)) {
        let lead_end = suffix
            .find(|c: char| !is_ident_char(c))
            .unwrap_or(suffix.len());
        let head = &suffix[..lead_end];
        let tail = &suffix[lead_end..];
        let isuf = getsparam("ISUFFIX").unwrap_or_default();
        let _ = setsparam("ISUFFIX", &format!("{}{}", tail, isuf));
        let _ = setsparam("SUFFIX", head);
    }

    // sh:12-14
    dispatch_function_call(
        "_alternative",
        &[
            "math-parameters:math parameter: _math_params".to_string(),
            "user-math-functions:user math function: _user_math_func".to_string(),
            "module-math-functions:math function from zsh/mathfunc: _module_math_func".to_string(),
        ],
    )
    .unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "ident");
        let _ = setsparam("SUFFIX", "");
        assert_eq!(_math(), 1);
    }

    #[test]
    fn splits_prefix_non_ident_into_iprefix() {
        // sh:3-6 — PREFIX with leading non-ident chars: trailing
        //   ident portion stays as PREFIX, non-ident head moves to
        //   IPREFIX.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "a+b");
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _ = _math();
        assert_eq!(getsparam("PREFIX").as_deref(), Some("b"));
        assert_eq!(getsparam("IPREFIX").as_deref(), Some("a+"));
    }
}
