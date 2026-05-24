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

/// _complete_help_generic - Generic help completion. Parses
/// option lines of the form `--opt[=ARG]   description` or
/// `-x[, --long]   description` and emits them filtered by the
/// current prefix. Strips trailing commas so `-h,` lookup-keys as
/// `-h`. Supports lines with multiple options separated by commas
/// (e.g. `-h, --help`).
pub fn _complete_help_generic(state: &mut CompletionState, help_text: &str) -> bool {
    let prefix = state.params.prefix.clone();
    let mut options: Vec<(String, String)> = Vec::new();

    for line in help_text.lines() {
        let line = line.trim();
        if !line.starts_with('-') {
            continue;
        }
        // Find the split point between options block and description:
        // first run of >=2 spaces or a tab.
        let split_at = line
            .char_indices()
            .collect::<Vec<_>>()
            .windows(2)
            .find_map(|w| {
                let (i, a) = w[0];
                let (_, b) = w[1];
                if (a == ' ' && b == ' ') || a == '\t' {
                    Some(i)
                } else {
                    None
                }
            });
        let (opts_segment, desc) = match split_at {
            Some(i) => (&line[..i], line[i..].trim()),
            None => (line, ""),
        };
        // Each comma-separated entry is its own option.
        for raw_opt in opts_segment.split(',') {
            let opt = raw_opt.trim().trim_end_matches(',');
            // Trim the `[=ARG]` / `=ARG` / `<arg>` trailers from the
            // option-name lookup-key, but keep the desc intact.
            let key = opt
                .split(|c: char| c == '=' || c == '[' || c == ' ' || c == '<')
                .next()
                .unwrap_or("");
            if key.is_empty() || !key.starts_with('-') {
                continue;
            }
            if prefix.is_empty() || key.starts_with(&prefix) {
                options.push((key.to_string(), desc.to_string()));
            }
        }
    }

    if options.is_empty() {
        return false;
    }
    // Dedup (preserving first-seen order) — `-h, --help` line emits
    // both keys, but each key from later lines that repeats is
    // dropped.
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<(String, String)> = options
        .into_iter()
        .filter(|(k, _)| seen.insert(k.clone()))
        .collect();

    state.begin_group("options", true);
    for (opt, desc) in unique {
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

    #[test]
    fn comma_separated_options_on_one_line_both_emitted() {
        // `-h, --help   Show help` should produce both `-h` and
        // `--help` as separate completions sharing the same desc.
        let help = "-h, --help    Show this help";
        let mut state = CompletionState::new();
        assert!(_complete_help_generic(&mut state, help));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"-h"));
        assert!(names.contains(&"--help"));
    }

    #[test]
    fn option_with_arg_keyed_without_arg() {
        // `--output=FILE  Write here` — lookup key is `--output`,
        // not `--output=FILE`.
        let help = "  --output=FILE   Write output here";
        let mut state = CompletionState::new();
        assert!(_complete_help_generic(&mut state, help));
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names, vec!["--output"]);
    }

    #[test]
    fn duplicate_options_deduplicated() {
        let help = "
        --verbose    Be verbose
        --verbose    Also be verbose (duplicate)
        ";
        let mut state = CompletionState::new();
        assert!(_complete_help_generic(&mut state, help));
        let count = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .filter(|c| c.str_ == "--verbose")
            .count();
        assert_eq!(count, 1, "duplicate option lines should collapse to 1");
    }

    #[test]
    fn empty_prefix_emits_all_options() {
        let help = "
        --a    A
        --b    B
        --c    C
        ";
        let mut state = CompletionState::new();
        // prefix empty by default.
        let _ = _complete_help_generic(&mut state, help);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"--a"));
        assert!(names.contains(&"--b"));
        assert!(names.contains(&"--c"));
    }

    #[test]
    fn description_includes_dashes_intact() {
        // A description that itself contains `--` (e.g. "use -- as
        // sentinel") shouldn't get truncated.
        let help = "--sentinel    use -- as the sentinel argument";
        let mut state = CompletionState::new();
        assert!(_complete_help_generic(&mut state, help));
        let disp = state.groups[0].matches[0]
            .disp
            .as_deref()
            .unwrap_or("");
        assert!(disp.contains("use -- as the sentinel argument"));
    }
}
