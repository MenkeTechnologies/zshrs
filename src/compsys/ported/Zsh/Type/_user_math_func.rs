//! Port of `_user_math_func` from `Completion/Zsh/Type/_user_math_func`.
//!
//! Full upstream body (9 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl
//! sh: 4  local -a funcs
//! sh: 5
//! sh: 6  funcs=(${${${(f)"$(functions -M)"}##functions -M }%% *})
//! sh: 7
//! sh: 8  _wanted user-math-functions expl 'user math function' \
//! sh: 9      compadd -S '(' -q "$@" -a funcs
//! ```
//!
//! `functions -M` lists user-defined math functions (zsh's
//! `zmathfuncdef`-style functions). Each emitted name gets the `(`
//! suffix + NOSPACE so the user keeps typing the argument list.
//!
//! Strict Rust port: caller injects the function-name list.



use crate::compsys::base::MainCompleteState;
use crate::compsys::completion::{Completion, CompletionFlags};
use crate::compsys::ported::_wanted::_wanted;

/// `_user_math_func` — emit user math functions with `(` suffix.
pub fn _user_math_func(state: &mut MainCompleteState, funcs: &[String]) -> bool {
    // shell: `_wanted user-math-functions expl 'user math function' compadd -S '(' -q -a funcs`
    let prefix = state.comp.params.prefix.clone();
    _wanted(state, "user-math-functions", "user math function", |s| {
        let mut any = false;
        for f in funcs {
            if !f.starts_with(&prefix) {
                continue;
            }
            let mut comp = Completion::new(f.clone());
            comp.suf = Some("(".into());
            comp.flags |= CompletionFlags::NOSPACE;
            s.add_match(comp, Some("user-math-functions"));
            any = true;
        }
        any
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["user-math-functions".into()]);
        state.tags.add_try(&["user-math-functions".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn emits_with_paren_suffix() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let funcs = vec!["square".to_string(), "cube".to_string()];
        assert!(_user_math_func(&mut state, &funcs));
        for m in &state.comp.groups[0].matches {
            assert_eq!(m.suf.as_deref(), Some("("));
            assert!(m.flags.contains(CompletionFlags::NOSPACE));
        }
    }

    #[test]
    fn untagged_call_skips_emission() {
        let mut state = MainCompleteState::new("", 0);
        let funcs = vec!["square".to_string()];
        assert!(!_user_math_func(&mut state, &funcs));
    }

    #[test]
    fn prefix_filter() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "sq".into();
        let funcs = vec![
            "square".to_string(),
            "cube".to_string(),
            "sqrt".to_string(),
        ];
        let _ = _user_math_func(&mut state, &funcs);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"square"));
        assert!(names.contains(&"sqrt"));
        assert!(!names.contains(&"cube"));
    }

    #[test]
    fn empty_funcs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        assert!(!_user_math_func(&mut state, &[]));
    }

    #[test]
    fn group_named_user_math_functions() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        let funcs = vec!["x".to_string()];
        let _ = _user_math_func(&mut state, &funcs);
        assert!(state.comp.groups.iter().any(|g| g.name == "user-math-functions"));
    }

    #[test]
    fn off_prefix_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "zzz".into();
        let funcs = vec!["square".to_string()];
        assert!(!_user_math_func(&mut state, &funcs));
    }
}
