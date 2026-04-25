//! Completion core for ZLE
//!
//! Port from zsh/Src/Zle/compcore.c (3,638 lines)
//!
//! The full completion engine is implemented in the `compsys` crate
//! (compsys/compcore.rs, 644 lines). This module provides the ZLE-side
//! interface that connects the editor to the completion system.
//!
//! Key C functions and their Rust locations:
//! - do_completion     → compsys::compcore::do_completion()
//! - before_complete   → compsys::compcore::before_complete()
//! - after_complete    → compsys::compcore::after_complete()
//! - callcompfunc      → compsys::shell_runner (completion function eval)
//! - makecomplist      → compsys::compcore::make_comp_list()
//! - addmatch          → compsys::compadd::add_match()
//! - addmatches        → compsys::compadd::add_matches()
//! - comp_str          → compsys::compset (word extraction)
//! - set_comp_sep      → compsys::compset::set_comp_sep()
//! - check_param       → compsys::base (parameter completion)
//! - multiquote        → compsys::base::multiquote()
//! - tildequote        → compsys::base::tildequote()
//! - ctokenize         → compsys::base::ctokenize()

/// Completion state passed between ZLE and the completion system
#[derive(Debug, Clone, Default)]
pub struct CompState {
    /// Current word being completed
    pub current_word: String,
    /// Words on the command line
    pub words: Vec<String>,
    /// Index of current word (1-based, zsh style)
    pub current: usize,
    /// Cursor position within current word
    pub cursor_pos: usize,
    /// Prefix before cursor in current word
    pub prefix: String,
    /// Suffix after cursor in current word
    pub suffix: String,
    /// The complete command line
    pub buffer: String,
    /// Whether we're in a special context (redirect, assignment, etc.)
    pub context: CompContext,
    /// Matches found
    pub matches: Vec<CompMatch>,
    /// Whether completion is active
    pub active: bool,
    /// Whether to show listing
    pub list: bool,
    /// Whether to insert immediately
    pub insert: bool,
    /// Number of matches
    pub nmatches: usize,
}

/// Completion context
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CompContext {
    #[default]
    Command,
    Argument,
    Redirect,
    Assignment,
    Subscript,
    Math,
    Condition,
    Array,
    Brace,
}

/// A completion match
#[derive(Debug, Clone)]
pub struct CompMatch {
    pub word: String,
    pub description: Option<String>,
    pub group: Option<String>,
    pub prefix: String,
    pub suffix: String,
    pub display: Option<String>,
}

impl CompMatch {
    pub fn new(word: &str) -> Self {
        CompMatch {
            word: word.to_string(),
            description: None,
            group: None,
            prefix: String::new(),
            suffix: String::new(),
            display: None,
        }
    }

    pub fn with_description(mut self, desc: &str) -> Self {
        self.description = Some(desc.to_string());
        self
    }
}

/// Initialize completion for a line (from compcore.c do_completion)
pub fn init_completion(buffer: &str, cursor: usize) -> CompState {
    let mut state = CompState::default();
    state.buffer = buffer.to_string();
    state.active = true;

    // Split into words
    let mut words = Vec::new();
    let mut current = 0;
    let mut word_start = 0;
    let mut in_word = false;
    let mut in_quote = false;
    let mut quote_char = '\0';

    for (i, c) in buffer.char_indices() {
        if in_quote {
            if c == quote_char {
                in_quote = false;
            }
            continue;
        }
        if c == '\'' || c == '"' {
            in_quote = true;
            quote_char = c;
            if !in_word {
                word_start = i;
                in_word = true;
            }
            continue;
        }
        if c.is_whitespace() {
            if in_word {
                words.push(buffer[word_start..i].to_string());
                if cursor >= word_start && cursor <= i {
                    current = words.len();
                }
                in_word = false;
            }
        } else if !in_word {
            word_start = i;
            in_word = true;
        }
    }
    if in_word {
        words.push(buffer[word_start..].to_string());
        if cursor >= word_start {
            current = words.len();
        }
    }
    if words.is_empty() || cursor >= buffer.len() {
        words.push(String::new());
        current = words.len();
    }

    state.words = words;
    state.current = current;
    if current > 0 && current <= state.words.len() {
        state.current_word = state.words[current - 1].clone();
    }

    state
}

/// Add a match to the completion state (from compcore.c addmatch/add_match_data)
pub fn addmatch(state: &mut CompState, m: CompMatch) {
    state.matches.push(m);
    state.nmatches = state.matches.len();
}

/// Get user variable for completion (from compcore.c get_user_var)
pub fn get_user_var(
    name: &str,
    vars: &std::collections::HashMap<String, String>,
) -> Option<String> {
    vars.get(name).cloned()
}

/// Quote a string for completion insertion (from compcore.c multiquote)
pub fn multiquote(s: &str, in_quotes: bool) -> String {
    if in_quotes {
        s.replace('\\', "\\\\").replace('\'', "\\'")
    } else {
        crate::utils::quote_string(s)
    }
}

/// Quote tilde in completion (from compcore.c tildequote)
pub fn tildequote(s: &str) -> String {
    if s.starts_with('~') {
        format!("\\{}", s)
    } else {
        s.to_string()
    }
}

/// Remove backslashes from completion word (from compcore.c rembslash)
pub fn rembslash(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut escape = false;
    for c in s.chars() {
        if escape {
            result.push(c);
            escape = false;
        } else if c == '\\' {
            escape = true;
        } else {
            result.push(c);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_completion() {
        let state = init_completion("git commit -m ", 14);
        assert_eq!(state.words, vec!["git", "commit", "-m", ""]);
        assert!(state.active);
    }

    #[test]
    fn test_addmatch() {
        let mut state = CompState::default();
        addmatch(&mut state, CompMatch::new("hello"));
        addmatch(&mut state, CompMatch::new("world"));
        assert_eq!(state.nmatches, 2);
    }

    #[test]
    fn test_multiquote() {
        assert_eq!(multiquote("it's", false), "'it'\\''s'");
    }

    #[test]
    fn test_tildequote() {
        assert_eq!(tildequote("~user"), "\\~user");
        assert_eq!(tildequote("/home"), "/home");
    }

    #[test]
    fn test_rembslash() {
        assert_eq!(rembslash("hello\\ world"), "hello world");
        assert_eq!(rembslash("no\\\\slash"), "no\\slash");
    }
}
