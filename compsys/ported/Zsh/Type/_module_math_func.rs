//! Port of `_module_math_func` — complete math functions from zsh
//! modules (`zsh/mathfunc`, `zsh/example`, `zsh/system`).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_module_math_func`.
//!
//! Upstream shell source (8 lines):
//! ```text
//! local -a modules=( example mathfunc system )
//! for mod in $modules; do
//!   funcs=( ${${${(f)"$(zmodload -Fl zsh/$mod 2>/dev/null)"}:#^+f:*}##+f:} )
//!   alts+=( "module-math-functions.${mod}:math function from zsh/${mod}:compadd -S '(' $funcs" )
//! done
//! _alternative $alts
//! ```
//!
//! Strict Rust port: faithful 1:1 — builds the alternatives list
//! per upstream verbatim, then dispatches via [`_alternative`].
//! The action string carries the literal `compadd -S '(' fn1 fn2
//! …` invocation; the handler closure parses it and emits each
//! `fn` with `(` suffix + NOSPACE (the `-S '('` semantic).

use std::collections::BTreeMap;

use crate::base::MainCompleteState;
use crate::completion::{Completion, CompletionFlags};
use crate::ported::_alternative::_alternative;

/// `_module_math_func` — dispatch via `_alternative` with one spec
/// per module.
pub fn _module_math_func(
    state: &mut MainCompleteState,
    modules: &BTreeMap<String, Vec<String>>,
) -> bool {
    // Build alternatives list verbatim per upstream shell:5.
    let mut alts: Vec<String> = Vec::new();
    for (modname, fns) in modules {
        let joined = fns.join(" ");
        alts.push(format!(
            "module-math-functions.{}:math function from zsh/{}:compadd -S '(' {}",
            modname, modname, joined
        ));
    }

    // shell:8 — `_alternative $alts`. Action handler parses
    // `compadd -S '(' fn1 fn2 …` and emits each fn with `(`
    // suffix + NOSPACE.
    _alternative(state, &alts, |s, action| {
        // Strip the `compadd -S '(' ` prefix; remainder is the
        // space-separated fn list.
        let body = action
            .strip_prefix("compadd -S '(' ")
            .unwrap_or(action);
        let prefix = s.comp.params.prefix.clone();
        let mut any = false;
        for f in body.split_whitespace() {
            if !f.starts_with(&prefix) {
                continue;
            }
            let mut comp = Completion::new(f);
            comp.suf = Some("(".into());
            comp.flags |= CompletionFlags::NOSPACE;
            s.comp.add_match(comp, None);
            any = true;
        }
        any
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_group_per_module_with_paren_suffix() {
        let mut state = MainCompleteState::new("", 0);
        let mut mods = BTreeMap::new();
        mods.insert("mathfunc".into(), vec!["sin".into(), "cos".into()]);
        mods.insert("system".into(), vec!["zsystem_supports".into()]);
        let _ = _module_math_func(&mut state, &mods);
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"module-math-functions.mathfunc"));
        assert!(groups.contains(&"module-math-functions.system"));
        for g in &state.comp.groups {
            for c in &g.matches {
                assert_eq!(c.suf.as_deref(), Some("("));
                assert!(c.flags.contains(CompletionFlags::NOSPACE));
            }
        }
    }

    #[test]
    fn prefix_filter_per_module() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "si".into();
        let mut mods = BTreeMap::new();
        mods.insert(
            "mathfunc".into(),
            vec!["sin".into(), "cos".into(), "sinh".into()],
        );
        let _ = _module_math_func(&mut state, &mods);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"sin"));
        assert!(names.contains(&"sinh"));
        assert!(!names.contains(&"cos"));
    }

    #[test]
    fn empty_modules_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_module_math_func(&mut state, &BTreeMap::new()));
    }

    #[test]
    fn module_with_empty_fn_list_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        let mut mods = BTreeMap::new();
        mods.insert("empty".into(), vec![]);
        assert!(!_module_math_func(&mut state, &mods));
    }

    #[test]
    fn no_matching_prefix_across_all_modules_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "zzzzz".into();
        let mut mods = BTreeMap::new();
        mods.insert("mathfunc".into(), vec!["sin".into()]);
        assert!(!_module_math_func(&mut state, &mods));
    }
}
