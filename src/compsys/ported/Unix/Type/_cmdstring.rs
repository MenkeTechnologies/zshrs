//! Port of `_cmdstring` from `Completion/Unix/Type/_cmdstring`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is for a quoted argument that will be interpreted as a command.
//! sh: 4
//! sh: 5  compset -q
//! sh: 6  _normal
//! ```
//!
//! Upstream calls `compset -q` to treat the current word as a
//! quoted shell argument, then dispatches to `_normal` (which
//! handles command-vs-argument selection).
//!
//! Strict Rust port: applies `compset -q` semantics (strip
//! surrounding single OR double quotes from `prefix`/`suffix`)
//! BEFORE dispatching to `_command_names`. The shell `compset -q`
//! re-tokenizes the current word as a shell argument; for the
//! common "user typed `eval 'pre|fix'`" case, stripping the outer
//! quotes is the dominant effect.



use crate::compsys::compcore::CompletionState;

use super::_command_names::{_command_names, ShellInventory};

/// Apply `compset -q` to a slice — strip a matching open quote at
/// the start AND a matching close quote at the end. Returns
/// `(stripped_prefix, stripped_suffix, did_strip)`.
fn compset_q(prefix: &str, suffix: &str) -> (String, String, bool) {
    // The quote can be on prefix only, suffix only, or both. We
    // remove a quote character whenever it appears at the OUTER
    // boundary (prefix start / suffix end) AND mirrors a quote at
    // the other boundary OR there's no other quote yet (one-sided
    // case: user typed `'pref` and is mid-word).
    let p_first = prefix.chars().next();
    let s_last = suffix.chars().last();
    let stripped = match (p_first, s_last) {
        (Some(p), Some(s)) if (p == '\'' || p == '"') && p == s => {
            // Symmetric quoting around the word.
            (prefix[p.len_utf8()..].to_string(), suffix[..suffix.len() - s.len_utf8()].to_string(), true)
        }
        (Some(p), _) if (p == '\'' || p == '"') => {
            (prefix[p.len_utf8()..].to_string(), suffix.to_string(), true)
        }
        (_, Some(s)) if (s == '\'' || s == '"') => {
            (prefix.to_string(), suffix[..suffix.len() - s.len_utf8()].to_string(), true)
        }
        _ => (prefix.to_string(), suffix.to_string(), false),
    };
    stripped
}

/// _cmdstring - Complete a command string (for eval, etc.).
pub fn _cmdstring(state: &mut CompletionState, inv: &ShellInventory<'_>) -> bool {
    // shell:4 — compset -q
    let (new_p, new_s, _) =
        compset_q(&state.params.prefix, &state.params.suffix);
    state.params.prefix = new_p;
    state.params.suffix = new_s;
    // shell:5 — _normal. At our layer the leaf gateway is
    // _command_names (full mode = both builtins/funcs and external).
    _command_names(state, inv, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_command_names_full_mode() {
        let mut state = CompletionState::new();
        state.params.prefix = "t".into();
        let builtins = vec!["true".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdstring(&mut state, &inv);
        let groups: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        for tag in ["commands", "builtins", "functions", "aliases"] {
            assert!(
                groups.contains(&tag),
                "_cmdstring must delegate to full _command_names; missing tag {tag}"
            );
        }
    }

    #[test]
    fn emits_builtin_match_with_prefix_filter() {
        let mut state = CompletionState::new();
        state.params.prefix = "tr".into();
        let builtins = vec!["true".into(), "trap".into(), "exit".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdstring(&mut state, &inv);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"true".to_string()));
        assert!(names.contains(&"trap".to_string()));
        assert!(!names.contains(&"exit".to_string()));
    }

    #[test]
    fn empty_inventory_still_emits_groups() {
        let mut state = CompletionState::new();
        let inv = ShellInventory::default();
        let _ = _cmdstring(&mut state, &inv);
        // Full-mode _command_names creates the tag groups even when
        // empty.
        let groups: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        assert!(groups.contains(&"commands"));
    }

    #[test]
    fn aliases_appear_in_result() {
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let aliases = vec!["ll".into(), "la".into()];
        let inv = ShellInventory {
            aliases: &aliases,
            ..Default::default()
        };
        let _ = _cmdstring(&mut state, &inv);
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"ll".to_string()));
    }

    #[test]
    fn compset_q_strips_symmetric_double_quotes() {
        let (p, s, did) = compset_q("\"abc", "def\"");
        assert_eq!(p, "abc");
        assert_eq!(s, "def");
        assert!(did);
    }

    #[test]
    fn compset_q_strips_symmetric_single_quotes() {
        let (p, s, did) = compset_q("'abc", "def'");
        assert_eq!(p, "abc");
        assert_eq!(s, "def");
        assert!(did);
    }

    #[test]
    fn compset_q_strips_one_sided_open_quote() {
        let (p, s, did) = compset_q("\"abc", "");
        assert_eq!(p, "abc");
        assert_eq!(s, "");
        assert!(did);
    }

    #[test]
    fn compset_q_no_quotes_no_change() {
        let (p, s, did) = compset_q("abc", "def");
        assert_eq!(p, "abc");
        assert_eq!(s, "def");
        assert!(!did);
    }

    #[test]
    fn cmdstring_strips_quotes_before_dispatch() {
        // User typed `"tr|"` with cursor mid-word; after quote
        // stripping the inventory lookup should still pick up `trap`
        // / `true`.
        let mut state = CompletionState::new();
        state.params.prefix = "\"tr".into();
        state.params.suffix = "\"".into();
        let builtins = vec!["true".into(), "trap".into()];
        let inv = ShellInventory {
            builtins: &builtins,
            ..Default::default()
        };
        let _ = _cmdstring(&mut state, &inv);
        // Prefix should have been stripped of the opening quote so
        // the builtin lookup matches.
        assert_eq!(state.params.prefix, "tr");
        assert_eq!(state.params.suffix, "");
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"true".to_string()));
        assert!(names.contains(&"trap".to_string()));
    }
}
