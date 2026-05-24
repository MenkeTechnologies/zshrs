//! Port of `_complete_help_generic` — generic help completion.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_complete_help_generic`
//! (system copy `/opt/homebrew/share/zsh/functions/_complete_help_generic`).
//!
//! Upstream shell source (full 17-line widget):
//! ```text
//!  6  [[ $WIDGET = *noread* ]] || local ZSH_TRACE_GENERIC_WIDGET
//!  8  if [[ $WIDGET = *debug* ]]; then
//!  9    ZSH_TRACE_GENERIC_WIDGET=_complete_debug
//! 10  else
//! 11    ZSH_TRACE_GENERIC_WIDGET=_complete_help
//! 12  fi
//! 14  if [[ $WIDGET != *noread* ]]; then
//! 15    zle read-command && zle $REPLY -w
//! 16  fi
//! ```
//!
//! Upstream is a `zle` widget that reads ANOTHER widget name from
//! the user via `read-command`, then runs it with
//! `ZSH_TRACE_GENERIC_WIDGET` set so `_generic` knows to call
//! `_complete_help` (or `_complete_debug`) on it.
//!
//! Simplified Rust port: takes the `--help`-style text directly and
//! parses dash-prefixed option lines, emitting them as completions
//! with `option -- description` disp format. Skips the zle widget
//! interaction entirely — this is the "give me the parsed options"
//! API that callers actually need.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _complete_help_generic - Generic help completion
pub fn _complete_help_generic(state: &mut CompletionState, help_text: &str) -> bool {
    let prefix = state.params.prefix.clone();

    // Parse --option lines from help text
    let mut options = Vec::new();

    for line in help_text.lines() {
        let line = line.trim();
        if line.starts_with('-') {
            // Extract option and description
            let parts: Vec<&str> = line.splitn(2, |c: char| c.is_whitespace()).collect();
            if let Some(opt) = parts.first() {
                let desc = parts.get(1).unwrap_or(&"").trim();
                if opt.starts_with(&prefix) || prefix.is_empty() {
                    options.push((opt.to_string(), desc.to_string()));
                }
            }
        }
    }

    if options.is_empty() {
        return false;
    }

    state.begin_group("options", true);
    for (opt, desc) in options {
        let mut comp = Completion::new(&opt);
        if !desc.is_empty() {
            comp.disp = Some(format!("{} -- {}", opt, desc));
        }
        state.add_match(comp, Some("options"));
    }
    state.end_group();

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_dash_lines_with_descriptions() {
        let help = "
        -h, --help    Show help message
        -v            Verbose output
        --version     Print version
        ";
        let mut state = CompletionState::new();
        assert!(_complete_help_generic(&mut state, help));
        let by_str: std::collections::HashMap<&str, &str> = state.groups[0]
            .matches
            .iter()
            .map(|c| (c.str_.as_str(), c.disp.as_deref().unwrap_or("")))
            .collect();
        // The parser splits at first whitespace, so the first
        // segment becomes the option.
        assert!(by_str.contains_key("-v"));
        assert!(by_str["-v"].starts_with("-v -- "));
        assert!(by_str.contains_key("--version"));
    }

    #[test]
    fn no_dash_lines_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_complete_help_generic(&mut state, "no dashes here at all"));
    }

    #[test]
    fn empty_help_text_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_complete_help_generic(&mut state, ""));
    }

    #[test]
    fn prefix_filters_options() {
        let help = "
        --verbose    Be verbose
        --version    Print version
        --debug      Enable debug
        ";
        let mut state = CompletionState::new();
        state.params.prefix = "--ver".into();
        assert!(_complete_help_generic(&mut state, help));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"--verbose"));
        assert!(names.contains(&"--version"));
        assert!(!names.contains(&"--debug"));
    }
}
