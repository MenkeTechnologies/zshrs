//! PCRE module - port of Modules/pcre.c
//!
//! Provides PCRE regex matching through pcre_compile, pcre_match, pcre_study builtins.
//! Uses the Rust `regex` crate which provides Perl-compatible regex syntax.

use regex::Regex;
use std::collections::HashMap;

/// Compiled PCRE pattern state
#[derive(Debug)]
pub struct PcreState {
    pattern: Option<Regex>,
    pattern_str: Option<String>,
}

impl Default for PcreState {
    fn default() -> Self {
        Self::new()
    }
}

impl PcreState {
    pub fn new() -> Self {
        Self {
            pattern: None,
            pattern_str: None,
        }
    }

    pub fn has_pattern(&self) -> bool {
        self.pattern.is_some()
    }

    pub fn clear(&mut self) {
        self.pattern = None;
        self.pattern_str = None;
    }
}

/// Options for pcre_compile
#[derive(Debug, Default, Clone)]
pub struct PcreCompileOptions {
    pub anchored: bool,
    pub caseless: bool,
    pub multiline: bool,
    pub extended: bool,
    pub dotall: bool,
}

/// Options for pcre_match
#[derive(Debug, Default, Clone)]
pub struct PcreMatchOptions {
    pub match_var: Option<String>,
    pub array_var: Option<String>,
    pub assoc_var: Option<String>,
    pub offset: usize,
    pub return_offsets: bool,
    pub use_dfa: bool,
}

/// Result of a PCRE match
#[derive(Debug, Clone)]
pub struct PcreMatchResult {
    pub matched: bool,
    pub full_match: Option<String>,
    pub captures: Vec<Option<String>>,
    pub named_captures: HashMap<String, String>,
    pub match_start: Option<usize>,
    pub match_end: Option<usize>,
}

impl PcreMatchResult {
    pub fn no_match() -> Self {
        Self {
            matched: false,
            full_match: None,
            captures: Vec::new(),
            named_captures: HashMap::new(),
            match_start: None,
            match_end: None,
        }
    }
}

/// Compile a PCRE pattern
pub fn pcre_compile(
    pattern: &str,
    options: &PcreCompileOptions,
    state: &mut PcreState,
) -> Result<(), String> {
    state.clear();

    let mut pattern_str = String::new();

    if options.caseless {
        pattern_str.push_str("(?i)");
    }
    if options.multiline {
        pattern_str.push_str("(?m)");
    }
    if options.dotall {
        pattern_str.push_str("(?s)");
    }
    if options.extended {
        pattern_str.push_str("(?x)");
    }
    if options.anchored {
        pattern_str.push('^');
    }

    pattern_str.push_str(pattern);

    match Regex::new(&pattern_str) {
        Ok(re) => {
            state.pattern = Some(re);
            state.pattern_str = Some(pattern_str);
            Ok(())
        }
        Err(e) => Err(format!("error in regex: {}", e)),
    }
}

/// Study a compiled pattern (no-op with Rust regex, but kept for API compat)
pub fn pcre_study(state: &PcreState) -> Result<(), String> {
    if state.pattern.is_none() {
        return Err("no pattern has been compiled for study".to_string());
    }
    Ok(())
}

/// Match a string against the compiled pattern
pub fn pcre_match(
    text: &str,
    options: &PcreMatchOptions,
    state: &PcreState,
) -> Result<PcreMatchResult, String> {
    let re = state
        .pattern
        .as_ref()
        .ok_or_else(|| "no pattern has been compiled".to_string())?;

    let search_text = if options.offset > 0 && options.offset < text.len() {
        &text[options.offset..]
    } else if options.offset >= text.len() {
        return Ok(PcreMatchResult::no_match());
    } else {
        text
    };

    let caps = match re.captures(search_text) {
        Some(c) => c,
        None => return Ok(PcreMatchResult::no_match()),
    };

    let full_match = caps.get(0).map(|m| m.as_str().to_string());
    let match_start = caps.get(0).map(|m| m.start() + options.offset);
    let match_end = caps.get(0).map(|m| m.end() + options.offset);

    let mut captures = Vec::new();
    for i in 1..caps.len() {
        captures.push(caps.get(i).map(|m| m.as_str().to_string()));
    }

    let mut named_captures = HashMap::new();
    for name in re.capture_names().flatten() {
        if let Some(m) = caps.name(name) {
            named_captures.insert(name.to_string(), m.as_str().to_string());
        }
    }

    Ok(PcreMatchResult {
        matched: true,
        full_match,
        captures,
        named_captures,
        match_start,
        match_end,
    })
}

/// Conditional test for pcre-match
pub fn cond_pcre_match(lhs: &str, rhs: &str, caseless: bool) -> (bool, PcreMatchResult) {
    let options = PcreCompileOptions {
        caseless,
        ..Default::default()
    };

    let mut state = PcreState::new();

    if pcre_compile(rhs, &options, &mut state).is_err() {
        return (false, PcreMatchResult::no_match());
    }

    let match_options = PcreMatchOptions::default();

    match pcre_match(lhs, &match_options, &state) {
        Ok(result) => (result.matched, result),
        Err(_) => (false, PcreMatchResult::no_match()),
    }
}

/// Execute pcre_compile builtin
pub fn builtin_pcre_compile(
    args: &[&str],
    options: &PcreCompileOptions,
    state: &mut PcreState,
) -> (i32, String) {
    if args.is_empty() {
        return (1, "pcre_compile: pattern required\n".to_string());
    }

    match pcre_compile(args[0], options, state) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("pcre_compile: {}\n", e)),
    }
}

/// Execute pcre_study builtin
pub fn builtin_pcre_study(state: &PcreState) -> (i32, String) {
    match pcre_study(state) {
        Ok(()) => (0, String::new()),
        Err(e) => (1, format!("pcre_study: {}\n", e)),
    }
}

/// Execute pcre_match builtin
pub fn builtin_pcre_match(
    args: &[&str],
    options: &PcreMatchOptions,
    state: &PcreState,
) -> (i32, PcreMatchResult) {
    if args.is_empty() {
        return (1, PcreMatchResult::no_match());
    }

    match pcre_match(args[0], options, state) {
        Ok(result) => {
            if result.matched {
                (0, result)
            } else {
                (1, result)
            }
        }
        Err(_) => (1, PcreMatchResult::no_match()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pcre_state_new() {
        let state = PcreState::new();
        assert!(!state.has_pattern());
    }

    #[test]
    fn test_pcre_compile_simple() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();

        let result = pcre_compile("hello", &options, &mut state);
        assert!(result.is_ok());
        assert!(state.has_pattern());
    }

    #[test]
    fn test_pcre_compile_invalid() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();

        let result = pcre_compile("[invalid", &options, &mut state);
        assert!(result.is_err());
    }

    #[test]
    fn test_pcre_compile_caseless() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions {
            caseless: true,
            ..Default::default()
        };

        let result = pcre_compile("hello", &options, &mut state);
        assert!(result.is_ok());

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("HELLO WORLD", &match_opts, &state).unwrap();
        assert!(result.matched);
    }

    #[test]
    fn test_pcre_study_no_pattern() {
        let state = PcreState::new();
        let result = pcre_study(&state);
        assert!(result.is_err());
    }

    #[test]
    fn test_pcre_study_with_pattern() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("hello", &options, &mut state).unwrap();

        let result = pcre_study(&state);
        assert!(result.is_ok());
    }

    #[test]
    fn test_pcre_match_simple() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("hello", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
        assert!(result.matched);
        assert_eq!(result.full_match, Some("hello".to_string()));
    }

    #[test]
    fn test_pcre_match_no_match() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("hello", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("goodbye world", &match_opts, &state).unwrap();
        assert!(!result.matched);
    }

    #[test]
    fn test_pcre_match_captures() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile(r"(\w+) (\w+)", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
        assert!(result.matched);
        assert_eq!(result.captures.len(), 2);
        assert_eq!(result.captures[0], Some("hello".to_string()));
        assert_eq!(result.captures[1], Some("world".to_string()));
    }

    #[test]
    fn test_pcre_match_named_captures() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile(r"(?P<first>\w+) (?P<second>\w+)", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions::default();
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
        assert!(result.matched);
        assert_eq!(
            result.named_captures.get("first"),
            Some(&"hello".to_string())
        );
        assert_eq!(
            result.named_captures.get("second"),
            Some(&"world".to_string())
        );
    }

    #[test]
    fn test_pcre_match_with_offset() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        pcre_compile("world", &options, &mut state).unwrap();

        let match_opts = PcreMatchOptions {
            offset: 6,
            ..Default::default()
        };
        let result = pcre_match("hello world", &match_opts, &state).unwrap();
        assert!(result.matched);
        assert_eq!(result.match_start, Some(6));
    }

    #[test]
    fn test_cond_pcre_match() {
        let (matched, _) = cond_pcre_match("hello world", "hello", false);
        assert!(matched);

        let (matched, _) = cond_pcre_match("hello world", "HELLO", true);
        assert!(matched);

        let (matched, _) = cond_pcre_match("hello world", "HELLO", false);
        assert!(!matched);
    }

    #[test]
    fn test_builtin_pcre_compile_no_args() {
        let mut state = PcreState::new();
        let options = PcreCompileOptions::default();
        let (status, _) = builtin_pcre_compile(&[], &options, &mut state);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_pcre_match_no_pattern() {
        let state = PcreState::new();
        let options = PcreMatchOptions::default();
        let (status, _) = builtin_pcre_match(&["test"], &options, &state);
        assert_eq!(status, 1);
    }
}
