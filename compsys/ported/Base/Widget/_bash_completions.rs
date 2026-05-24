//! Port of `_bash_completions` — bash compspec compatibility.
//!
//! Local shell reference: `compsys/functions/Base/Widget/_bash_completions`
//! (system copy `/opt/homebrew/share/zsh/functions/_bash_completions`).
//!
//! Upstream `_bash_completions` is a `complete-word` widget. The
//! shell-side implementation is small (~50 lines) and primarily
//! delegates to bash's `complete -p` output parsing.
//!
//! Simplified Rust port: parses a single bash compspec string
//! directly (no shell-out to bash) and implements the most common
//! `-W wordlist`, `-G glob`, `-d` (dirs), `-f` (files), `-c`
//! (commands from PATH), `-u` (usernames from /etc/passwd) actions.
//! `-F function` / `-C command` (callbacks into shell) are noted
//! and skipped — they'd require a shell-eval substrate this layer
//! doesn't have.

use crate::base::{CompleterResult, MainCompleteState};

/// _bash_completions - Compatibility with bash completion specs
///
/// This function provides interoperability with bash's programmable completion.
/// It parses bash compspec strings and generates completions accordingly.
///
/// Compspec format: -F function | -C command | -G globpat | -W wordlist | -X filterpat
pub fn _bash_completions(state: &mut MainCompleteState, compspec: &str) -> CompleterResult {
    let prefix = &state.comp.params.prefix.clone();
    let mut matches = Vec::new();

    // Parse compspec arguments. Use a quote-aware tokenizer so
    // `-W "yes no maybe"` survives as a single arg rather than
    // splitting on the inner spaces.
    let args: Vec<String> = tokenize_quoted(compspec);
    let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let mut i = 0;

    while i < args.len() {
        match args[i] {
            "-W" if i + 1 < args.len() => {
                // Word list: -W "word1 word2 word3"
                i += 1;
                let wordlist = args[i].trim_matches('"').trim_matches('\'');
                for word in wordlist.split_whitespace() {
                    if word.to_lowercase().starts_with(&prefix.to_lowercase()) {
                        matches.push(crate::Completion::new(word));
                    }
                }
            }
            "-G" if i + 1 < args.len() => {
                // Glob pattern: -G "*.txt"
                i += 1;
                let pattern = args[i].trim_matches('"').trim_matches('\'');
                if let Ok(entries) = std::fs::read_dir(".") {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if glob_match_simple(pattern, name)
                                && name.to_lowercase().starts_with(&prefix.to_lowercase())
                            {
                                matches.push(crate::Completion::new(name));
                            }
                        }
                    }
                }
            }
            "-d" => {
                // Directories only
                if let Ok(entries) = std::fs::read_dir(".") {
                    for entry in entries.flatten() {
                        if let Ok(ft) = entry.file_type() {
                            if ft.is_dir() {
                                if let Some(name) = entry.file_name().to_str() {
                                    if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                        let mut c = crate::Completion::new(format!("{}/", name));
                                        c.flags |= crate::CompletionFlags::FILE;
                                        matches.push(c);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "-f" => {
                // Files
                if let Ok(entries) = std::fs::read_dir(".") {
                    for entry in entries.flatten() {
                        if let Some(name) = entry.file_name().to_str() {
                            if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                let mut c = crate::Completion::new(name);
                                c.flags |= crate::CompletionFlags::FILE;
                                matches.push(c);
                            }
                        }
                    }
                }
            }
            "-c" => {
                // Commands from PATH
                if let Ok(path) = std::env::var("PATH") {
                    for dir in path.split(':') {
                        if let Ok(entries) = std::fs::read_dir(dir) {
                            for entry in entries.flatten() {
                                if let Some(name) = entry.file_name().to_str() {
                                    if name.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                        matches.push(crate::Completion::new(name));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            "-u" => {
                // Usernames
                if let Ok(content) = std::fs::read_to_string("/etc/passwd") {
                    for line in content.lines() {
                        if let Some(user) = line.split(':').next() {
                            if user.to_lowercase().starts_with(&prefix.to_lowercase()) {
                                matches.push(crate::Completion::new(user));
                            }
                        }
                    }
                }
            }
            "-F" | "-C" => {
                // Function or command - would need to invoke bash, skip
                i += 1;
            }
            "-X" | "-P" | "-S" if i + 1 < args.len() => {
                // Filter/prefix/suffix - skip
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    if !matches.is_empty() {
        // Add matches to a group
        let mut group = crate::CompletionGroup::new("bash-completion");
        group.matches = matches;
        state.comp.groups.push(group);
        state.comp.nmatches += state
            .comp
            .groups
            .last()
            .map(|g| g.matches.len())
            .unwrap_or(0);
        CompleterResult::Matched
    } else {
        CompleterResult::NoMatch
    }
}

/// Simple glob matching for bash compatibility
fn glob_match_simple(pattern: &str, text: &str) -> bool {
    let pattern = pattern.as_bytes();
    let text = text.as_bytes();

    fn match_impl(p: &[u8], t: &[u8]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some(b'*'), _) => match_impl(&p[1..], t) || (!t.is_empty() && match_impl(p, &t[1..])),
            (Some(b'?'), Some(_)) => match_impl(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => match_impl(&p[1..], &t[1..]),
            _ => false,
        }
    }

    match_impl(pattern, text)
}

/// Quote-aware whitespace tokenizer. Handles `"..."` and `'...'`
/// as single tokens (strips the outer quotes). Required so the
/// compspec `-W "yes no maybe"` survives as ONE arg through to the
/// `-W` handler, which then splits the wordlist on inner spaces.
fn tokenize_quoted(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_quote: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, in_quote) {
            ('\\', _) => {
                // Backslash escapes the next char.
                if let Some(nc) = chars.next() {
                    cur.push(nc);
                }
            }
            ('"' | '\'', None) => {
                in_quote = Some(c);
            }
            (q, Some(open)) if q == open => {
                in_quote = None;
            }
            (c, None) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (c, _) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dash_w_quoted_wordlist_splits_and_filters() {
        // Quote-aware tokenizer keeps `-W "yes no maybe yesterday"`
        // as a single arg, then -W handler splits on inner spaces.
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ye".into();
        let result = _bash_completions(&mut state, "-W \"yes no maybe yesterday\"");
        assert!(matches!(result, CompleterResult::Matched));
        let names: Vec<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains(&"yes".to_string()));
        assert!(names.contains(&"yesterday".to_string()));
        assert!(!names.contains(&"no".to_string()));
        assert!(!names.contains(&"maybe".to_string()));
    }

    #[test]
    fn dash_w_single_quoted_wordlist_works() {
        let mut state = MainCompleteState::new("", 0);
        let result = _bash_completions(&mut state, "-W 'alpha beta gamma'");
        assert!(matches!(result, CompleterResult::Matched));
        let names: std::collections::HashSet<String> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(names.contains("alpha"));
        assert!(names.contains("beta"));
        assert!(names.contains("gamma"));
    }

    #[test]
    fn dash_w_unquoted_single_word_still_works() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ye".into();
        let result = _bash_completions(&mut state, "-W yes");
        assert!(matches!(result, CompleterResult::Matched));
    }

    #[test]
    fn empty_compspec_returns_no_match() {
        let mut state = MainCompleteState::new("", 0);
        assert!(matches!(
            _bash_completions(&mut state, ""),
            CompleterResult::NoMatch
        ));
    }

    #[test]
    fn glob_match_simple_handles_star() {
        assert!(glob_match_simple("*.rs", "foo.rs"));
        assert!(!glob_match_simple("*.rs", "foo.txt"));
    }

    #[test]
    fn glob_match_simple_handles_question() {
        assert!(glob_match_simple("?oo", "foo"));
        assert!(!glob_match_simple("?oo", "fooo"));
    }

    #[test]
    fn tokenize_quoted_handles_mixed_quotes() {
        let toks = tokenize_quoted("-W \"foo bar\" -G '*.txt'");
        assert_eq!(toks, vec!["-W", "foo bar", "-G", "*.txt"]);
    }

    #[test]
    fn tokenize_quoted_handles_backslash_escape() {
        let toks = tokenize_quoted(r"-W foo\ bar baz");
        assert_eq!(toks, vec!["-W", "foo bar", "baz"]);
    }

    #[test]
    fn unknown_flags_silently_ignored() {
        // -F, -C, -X, -P, -S are documented as skipped (need bash
        // shell eval substrate). Pin they don't produce matches but
        // also don't crash.
        let mut state = MainCompleteState::new("", 0);
        let r = _bash_completions(&mut state, "-F my_fn -X !*.bak");
        assert!(matches!(r, CompleterResult::NoMatch));
    }
}
