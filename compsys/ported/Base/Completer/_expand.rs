//! Port of `_expand` — expand special characters (`$`, `~`, `{}`).
//! Moved from `compsys/functions.rs`. Renamed from `expand` to mirror
//! zsh shell function name `_expand`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _expand - Expand special characters ($, ~, {})
pub fn _expand(state: &mut CompletionState) -> bool {
    let prefix = &state.params.prefix;
    let mut expanded = prefix.clone();
    let mut did_expand = false;

    // Tilde expansion
    if expanded.starts_with('~') {
        if let Ok(home) = std::env::var("HOME") {
            if expanded == "~" || expanded.starts_with("~/") {
                expanded = expanded.replacen("~", &home, 1);
                did_expand = true;
            }
        }
    }

    // Variable expansion
    while let Some(dollar_pos) = expanded.find('$') {
        let rest = &expanded[dollar_pos + 1..];
        let var_end = rest
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(rest.len());
        let var_name = &rest[..var_end];

        if let Ok(value) = std::env::var(var_name) {
            let before = &expanded[..dollar_pos];
            let after = &rest[var_end..];
            expanded = format!("{}{}{}", before, value, after);
            did_expand = true;
        } else {
            break;
        }
    }

    if did_expand && expanded != *prefix {
        state.add_match(Completion::new(&expanded), None);
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_expands_to_home() {
        let mut state = CompletionState::new();
        state.params.prefix = "~/projects".into();
        let home = std::env::var("HOME").expect("HOME set in test env");
        assert!(_expand(&mut state));
        let m = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .next()
            .expect("expansion emitted");
        assert_eq!(m.str_, format!("{}/projects", home));
    }

    #[test]
    fn variable_expands_when_set() {
        std::env::set_var("ZSHRS_TEST_VAR_777", "VALUE");
        let mut state = CompletionState::new();
        state.params.prefix = "$ZSHRS_TEST_VAR_777/sub".into();
        assert!(_expand(&mut state));
        let m = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .next()
            .expect("expansion emitted");
        assert_eq!(m.str_, "VALUE/sub");
        std::env::remove_var("ZSHRS_TEST_VAR_777");
    }

    #[test]
    fn no_expansion_chars_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "plain_word".into();
        assert!(!_expand(&mut state));
    }
}
