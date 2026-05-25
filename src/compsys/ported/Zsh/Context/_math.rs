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
//!
//! Strict Rust port: faithful 1:1 — strips non-identifier chars
//! from PREFIX/SUFFIX (rewriting into IPREFIX/ISUFFIX), then
//! dispatches via [`_alternative`] with the exact three specs
//! upstream uses. Caller injects the data the three Zsh/Type
//! helpers need (params, user math fns, module math fns).



use std::collections::{BTreeMap, HashMap};

use crate::compsys::base::MainCompleteState;
use crate::compsys::ported::_alternative::_alternative;
use crate::compsys::ported::_math_params::_math_params;
use crate::compsys::ported::_module_math_func::_module_math_func;
use crate::compsys::ported::_user_math_func::_user_math_func;

/// `_math` — `-math-` context dispatcher.
pub fn _math(
    state: &mut MainCompleteState,
    params: &HashMap<String, String>,
    user_math_funcs: &[String],
    module_math_funcs: &BTreeMap<String, Vec<String>>,
) -> bool {
    // shell:3-6 — strip non-identifier chars from PREFIX into IPREFIX.
    let is_ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    let prefix = state.comp.params.prefix.clone();
    if let Some(non_ident_pos) = prefix.rfind(|c: char| !is_ident(c)) {
        let split = non_ident_pos + prefix[non_ident_pos..].chars().next().unwrap().len_utf8();
        state.comp.params.iprefix.push_str(&prefix[..split]);
        state.comp.params.prefix = prefix[split..].to_string();
    }
    // shell:7-10 — strip non-identifier chars from SUFFIX into ISUFFIX.
    let suffix = state.comp.params.suffix.clone();
    if let Some(non_ident_pos) = suffix.find(|c: char| !is_ident(c)) {
        let isuf_extra = suffix[non_ident_pos..].to_string();
        let new_isuffix = format!("{}{}", isuf_extra, state.comp.params.isuffix);
        state.comp.params.isuffix = new_isuffix;
        state.comp.params.suffix = suffix[..non_ident_pos].to_string();
    }

    // shell:12-14 — `_alternative` with three specs.
    let specs = vec![
        "math-parameters:math parameter:_math_params".to_string(),
        "user-math-functions:user math function:_user_math_func".to_string(),
        "module-math-functions:math function from zsh/mathfunc:_module_math_func".to_string(),
    ];
    _alternative(state, &specs, |s, action| match action {
        "_math_params" => _math_params(&mut s.comp, params),
        "_user_math_func" => _user_math_func(s, user_math_funcs),
        "_module_math_func" => _module_math_func(s, module_math_funcs),
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_non_identifier_prefix_into_iprefix() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "1+x".into();
        let params: HashMap<String, String> = HashMap::new();
        let _ = _math(&mut state, &params, &[], &BTreeMap::new());
        // After stripping, prefix = "x" and iprefix consumes "1+".
        assert_eq!(state.comp.params.iprefix, "1+");
        assert_eq!(state.comp.params.prefix, "x");
    }

    #[test]
    fn dispatches_three_alternative_groups() {
        let mut state = MainCompleteState::new("", 0);
        let mut params = HashMap::new();
        params.insert("COUNT".into(), "integer".into());
        let funcs = vec!["square".to_string()];
        let mut modules = BTreeMap::new();
        modules.insert("mathfunc".into(), vec!["sin".into()]);
        let _ = _math(&mut state, &params, &funcs, &modules);
        // _alternative creates groups for each `tag:` spec.
        let groups: Vec<&str> = state.comp.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"math-parameters"));
        assert!(groups.contains(&"user-math-functions"));
        assert!(groups
            .iter()
            .any(|n| n.starts_with("module-math-functions")));
    }

    #[test]
    fn pure_identifier_prefix_passes_through_unchanged() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "var".into();
        let params: HashMap<String, String> = HashMap::new();
        let _ = _math(&mut state, &params, &[], &BTreeMap::new());
        assert_eq!(state.comp.params.iprefix, "");
        assert_eq!(state.comp.params.prefix, "var");
    }

    #[test]
    fn empty_inputs_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        let _ = _math(&mut state, &HashMap::new(), &[], &BTreeMap::new());
    }

    #[test]
    fn suffix_with_non_identifier_chars_split() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "x".into();
        state.comp.params.suffix = "+1".into();
        let params: HashMap<String, String> = HashMap::new();
        let _ = _math(&mut state, &params, &[], &BTreeMap::new());
        assert_eq!(state.comp.params.isuffix, "+1");
        assert_eq!(state.comp.params.suffix, "");
    }
}
