//! Port of `_subscript` — `-subscript-` context handler (inside
//! `$array[…]`).
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_subscript`.
//!
//! Upstream is ~130 lines, key dispatch:
//! ```text
//! #compdef -subscript-
//!
//! local expl ind osuf flags sep
//! [[ $ISUFFIX = *\]* ]] || osuf=\]
//! …
//! if [[ -n $compstate[parameter] ]]; then
//!   if [[ ${(Pt)compstate[parameter]} = assoc* ]]; then
//!     _wanted association-keys expl 'association key' compadd "$expls[@]" - "${(@k)${(P)compstate[parameter]}}"
//!   elif …
//! ```
//!
//! Strict Rust port: caller injects the parameter table (so we
//! can resolve `$compstate[parameter]` to its type). For assoc
//! arrays, emit the keys. For regular arrays, emit integer
//! indices `1..N`. For scalars, emit `1`.

use crate::base::MainCompleteState;
use crate::completion::Completion;
use crate::ported::_wanted::_wanted;

/// Per-parameter resolver — caller maps `$compstate[parameter]`
/// to its kind + key/index list.
pub enum SubscriptTarget {
    /// Associative array — emit the supplied keys.
    Association(Vec<String>),
    /// Regular array of length `n` — emit `1..=n`.
    Array(usize),
    /// Scalar — emit `1`.
    Scalar,
    /// No parameter resolved — emit nothing.
    None,
}

/// `_subscript` — `-subscript-` context dispatcher.
pub fn _subscript(state: &mut MainCompleteState, target: SubscriptTarget) -> bool {
    match target {
        SubscriptTarget::Association(keys) => {
            _wanted(state, "association-keys", "association key", |s| {
                let prefix = s.params.prefix.clone();
                let mut any = false;
                for k in &keys {
                    if k.starts_with(&prefix) {
                        s.add_match(Completion::new(k.clone()), Some("association-keys"));
                        any = true;
                    }
                }
                any
            })
        }
        SubscriptTarget::Array(n) => {
            _wanted(state, "indexes", "array index", |s| {
                let prefix = s.params.prefix.clone();
                let mut any = false;
                for i in 1..=n {
                    let idx = i.to_string();
                    if idx.starts_with(&prefix) {
                        s.add_match(Completion::new(idx), Some("indexes"));
                        any = true;
                    }
                }
                any
            })
        }
        SubscriptTarget::Scalar => _wanted(state, "indexes", "scalar index", |s| {
            s.add_match(Completion::new("1"), Some("indexes"));
            true
        }),
        SubscriptTarget::None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::TagManager;

    fn seed(state: &mut MainCompleteState, tag: &str) {
        state.tags = TagManager::new();
        state.tags.init(&[tag.to_string()]);
        state.tags.add_try(&[tag.to_string()]);
        let _ = state.tags.start();
    }

    #[test]
    fn association_keys_emitted() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state, "association-keys");
        let target =
            SubscriptTarget::Association(vec!["alpha".into(), "beta".into()]);
        let _ = _subscript(&mut state, target);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn array_indices_emitted() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state, "indexes");
        let _ = _subscript(&mut state, SubscriptTarget::Array(3));
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["1", "2", "3"]);
    }

    #[test]
    fn scalar_emits_index_one() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state, "indexes");
        let _ = _subscript(&mut state, SubscriptTarget::Scalar);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["1"]);
    }

    #[test]
    fn none_target_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        assert!(!_subscript(&mut state, SubscriptTarget::None));
    }

    #[test]
    fn prefix_filters_association_keys() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state, "association-keys");
        state.comp.params.prefix = "al".into();
        let target = SubscriptTarget::Association(vec![
            "alpha".into(),
            "beta".into(),
            "altered".into(),
        ]);
        let _ = _subscript(&mut state, target);
        let names: Vec<&str> = state.comp.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"altered"));
        assert!(!names.contains(&"beta"));
    }
}
