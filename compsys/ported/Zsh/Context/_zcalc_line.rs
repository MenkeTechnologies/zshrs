//! Port of `_zcalc_line` — `-zcalc-line-` context (inside `zcalc`
//! command-line read via vared).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_zcalc_line`.
//!
//! Upstream shell source (~80 lines, key dispatch):
//! ```text
//! #compdef -zcalc-line-
//!
//! _zcalc_line_escapes() {
//!   local -a cmds
//!   cmds=(
//!     "!:shell escape" "q:quit" "norm:normal output format" …
//!   )
//!   cmds=("\:"${^cmds})
//!   _describe -t command-escapes "command escape" cmds -Q
//! }
//! …
//! ```
//!
//! Strict Rust port: detects the `:cmd` colon-command form and
//! emits the documented zcalc command escapes via `_describe`-
//! style emission. Math expressions fall through to [`_math`].

use std::collections::{BTreeMap, HashMap};

use crate::base::MainCompleteState;
use crate::completion::Completion;
use crate::ported::_math::_math;

/// Documented zcalc command-line escapes (from upstream).
pub const ZCALC_ESCAPES: &[(&str, &str)] = &[
    ("!", "shell escape"),
    ("q", "quit"),
    ("norm", "normal output format"),
    ("sci", "scientific output format"),
    ("fix", "fixed point output format"),
    ("eng", "engineering (power of 1000) output format"),
    ("raw", "raw output format"),
    ("local", "make variables local"),
    ("function", "define math function (also :func or :f)"),
];

/// `_zcalc_line` — `-zcalc-line-` context dispatcher.
pub fn _zcalc_line(
    state: &mut MainCompleteState,
    params: &HashMap<String, String>,
    user_math_funcs: &[String],
    module_math_funcs: &BTreeMap<String, Vec<String>>,
) -> bool {
    // shell: if PREFIX starts with `:`, emit command escapes.
    if state.comp.params.prefix.starts_with(':') {
        let prefix_after_colon = state.comp.params.prefix.trim_start_matches(':').to_string();
        state.comp.begin_group("command-escapes", true);
        let mut any = false;
        for (esc, desc) in ZCALC_ESCAPES {
            if !esc.starts_with(&prefix_after_colon) {
                continue;
            }
            let mut comp = Completion::new(*esc);
            comp.disp = Some(format!(":{} -- {}", esc, desc));
            comp.pre = Some(":".into());
            state.comp.add_match(comp, Some("command-escapes"));
            any = true;
        }
        state.comp.end_group();
        return any;
    }
    // Otherwise — math expression context. Dispatch to _math.
    _math(state, params, user_math_funcs, module_math_funcs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colon_prefix_emits_command_escapes() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = ":".into();
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"q"));
        assert!(names.contains(&"norm"));
        assert!(names.contains(&"function"));
    }

    #[test]
    fn colon_with_partial_filters_escapes() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = ":no".into();
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        // After stripping `:`, prefix is `no` → only `norm` survives.
        assert_eq!(names, vec!["norm"]);
    }

    #[test]
    fn no_colon_falls_through_to_math() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "1+x".into();
        let mut params = HashMap::new();
        params.insert("x".into(), "integer".into());
        let _ = _zcalc_line(&mut state, &params, &[], &BTreeMap::new());
        // _math strips non-ident chars; pin that the dispatch happened.
        assert_eq!(state.comp.params.iprefix, "1+");
        assert_eq!(state.comp.params.prefix, "x");
    }

    #[test]
    fn empty_prefix_falls_through_to_math() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        // No panic — _math handles empty prefix.
    }

    #[test]
    fn each_escape_has_descriptive_disp() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = ":".into();
        let _ = _zcalc_line(&mut state, &HashMap::new(), &[], &BTreeMap::new());
        for m in &state.comp.groups[0].matches {
            assert!(m.disp.as_deref().unwrap_or("").contains(" -- "));
        }
    }
}
