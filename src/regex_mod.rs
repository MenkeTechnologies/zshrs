//! Regex module - port of Modules/regex.c
//!
//! Provides regex matching conditional for the =~ operator.

use regex::Regex;
use std::collections::HashMap;

/// Result of a regex match operation
#[derive(Debug, Clone)]
pub struct RegexMatch {
    pub matched: bool,
    pub full_match: Option<String>,
    pub captures: Vec<Option<String>>,
    pub match_start: Option<usize>,
    pub match_end: Option<usize>,
    pub capture_starts: Vec<Option<usize>>,
    pub capture_ends: Vec<Option<usize>>,
}

impl RegexMatch {
    pub fn no_match() -> Self {
        Self {
            matched: false,
            full_match: None,
            captures: Vec::new(),
            match_start: None,
            match_end: None,
            capture_starts: Vec::new(),
            capture_ends: Vec::new(),
        }
    }
}

/// Options for regex matching
#[derive(Debug, Clone, Default)]
pub struct RegexOptions {
    pub case_insensitive: bool,
    pub bash_rematch: bool,
    pub ksh_arrays: bool,
}

/// Perform a regex match
pub fn regex_match(
    text: &str,
    pattern: &str,
    options: &RegexOptions,
) -> Result<RegexMatch, String> {
    let re = if options.case_insensitive {
        Regex::new(&format!("(?i){}", pattern))
    } else {
        Regex::new(pattern)
    }
    .map_err(|e| format!("failed to compile regex: {}", e))?;

    let caps = match re.captures(text) {
        Some(c) => c,
        None => return Ok(RegexMatch::no_match()),
    };

    let full_match = caps.get(0).map(|m| m.as_str().to_string());
    let match_start = caps.get(0).map(|m| m.start());
    let match_end = caps.get(0).map(|m| m.end());

    let mut captures = Vec::new();
    let mut capture_starts = Vec::new();
    let mut capture_ends = Vec::new();

    for i in 1..caps.len() {
        if let Some(m) = caps.get(i) {
            captures.push(Some(m.as_str().to_string()));
            capture_starts.push(Some(m.start()));
            capture_ends.push(Some(m.end()));
        } else {
            captures.push(None);
            capture_starts.push(None);
            capture_ends.push(None);
        }
    }

    Ok(RegexMatch {
        matched: true,
        full_match,
        captures,
        match_start,
        match_end,
        capture_starts,
        capture_ends,
    })
}

/// Convert byte offsets to character offsets
fn byte_to_char_offset(s: &str, byte_offset: usize) -> usize {
    s[..byte_offset].chars().count()
}

/// Get match variables in zsh format
pub fn get_match_variables(
    result: &RegexMatch,
    text: &str,
    options: &RegexOptions,
) -> HashMap<String, String> {
    let mut vars = HashMap::new();

    if !result.matched {
        return vars;
    }

    if options.bash_rematch {
        if let Some(ref full) = result.full_match {
            vars.insert("BASH_REMATCH[0]".to_string(), full.clone());
        }
        for (i, cap) in result.captures.iter().enumerate() {
            if let Some(c) = cap {
                vars.insert(format!("BASH_REMATCH[{}]", i + 1), c.clone());
            }
        }
    } else {
        if let Some(ref full) = result.full_match {
            vars.insert("MATCH".to_string(), full.clone());
        }

        let base = if options.ksh_arrays { 0 } else { 1 };

        if let Some(start) = result.match_start {
            let char_start = byte_to_char_offset(text, start);
            vars.insert("MBEGIN".to_string(), (char_start + base).to_string());
        }

        if let Some(end) = result.match_end {
            let char_end = byte_to_char_offset(text, end);
            vars.insert("MEND".to_string(), (char_end + base - 1).to_string());
        }

        for (i, cap) in result.captures.iter().enumerate() {
            if let Some(c) = cap {
                vars.insert(format!("match[{}]", i + base), c.clone());
            }
        }

        for (i, start) in result.capture_starts.iter().enumerate() {
            if let Some(s) = start {
                let char_start = byte_to_char_offset(text, *s);
                vars.insert(
                    format!("mbegin[{}]", i + base),
                    (char_start + base).to_string(),
                );
            } else {
                vars.insert(format!("mbegin[{}]", i + base), "-1".to_string());
            }
        }

        for (i, end) in result.capture_ends.iter().enumerate() {
            if let Some(e) = end {
                let char_end = byte_to_char_offset(text, *e);
                vars.insert(
                    format!("mend[{}]", i + base),
                    (char_end + base - 1).to_string(),
                );
            } else {
                vars.insert(format!("mend[{}]", i + base), "-1".to_string());
            }
        }
    }

    vars
}

/// Conditional test for regex-match
pub fn cond_regex_match(lhs: &str, rhs: &str, options: &RegexOptions) -> (bool, RegexMatch) {
    match regex_match(lhs, rhs, options) {
        Ok(result) => (result.matched, result),
        Err(_) => (false, RegexMatch::no_match()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_regex_match_simple() {
        let opts = RegexOptions::default();
        let result = regex_match("hello world", "hello", &opts).unwrap();
        assert!(result.matched);
        assert_eq!(result.full_match, Some("hello".to_string()));
    }

    #[test]
    fn test_regex_match_no_match() {
        let opts = RegexOptions::default();
        let result = regex_match("hello world", "goodbye", &opts).unwrap();
        assert!(!result.matched);
    }

    #[test]
    fn test_regex_match_captures() {
        let opts = RegexOptions::default();
        let result = regex_match("hello world", "(hello) (world)", &opts).unwrap();
        assert!(result.matched);
        assert_eq!(result.full_match, Some("hello world".to_string()));
        assert_eq!(result.captures.len(), 2);
        assert_eq!(result.captures[0], Some("hello".to_string()));
        assert_eq!(result.captures[1], Some("world".to_string()));
    }

    #[test]
    fn test_regex_match_case_insensitive() {
        let opts = RegexOptions {
            case_insensitive: true,
            ..Default::default()
        };
        let result = regex_match("HELLO WORLD", "hello", &opts).unwrap();
        assert!(result.matched);
    }

    #[test]
    fn test_regex_match_case_sensitive() {
        let opts = RegexOptions::default();
        let result = regex_match("HELLO WORLD", "hello", &opts).unwrap();
        assert!(!result.matched);
    }

    #[test]
    fn test_regex_match_positions() {
        let opts = RegexOptions::default();
        let result = regex_match("foo bar baz", "bar", &opts).unwrap();
        assert!(result.matched);
        assert_eq!(result.match_start, Some(4));
        assert_eq!(result.match_end, Some(7));
    }

    #[test]
    fn test_regex_match_invalid_pattern() {
        let opts = RegexOptions::default();
        let result = regex_match("test", "[invalid", &opts);
        assert!(result.is_err());
    }

    #[test]
    fn test_get_match_variables_zsh() {
        let opts = RegexOptions::default();
        let result = regex_match("hello world", "(hello) (world)", &opts).unwrap();
        let vars = get_match_variables(&result, "hello world", &opts);

        assert_eq!(vars.get("MATCH"), Some(&"hello world".to_string()));
        assert_eq!(vars.get("MBEGIN"), Some(&"1".to_string()));
        assert_eq!(vars.get("MEND"), Some(&"11".to_string()));
    }

    #[test]
    fn test_get_match_variables_bash() {
        let opts = RegexOptions {
            bash_rematch: true,
            ..Default::default()
        };
        let result = regex_match("hello world", "(hello) (world)", &opts).unwrap();
        let vars = get_match_variables(&result, "hello world", &opts);

        assert_eq!(
            vars.get("BASH_REMATCH[0]"),
            Some(&"hello world".to_string())
        );
        assert_eq!(vars.get("BASH_REMATCH[1]"), Some(&"hello".to_string()));
        assert_eq!(vars.get("BASH_REMATCH[2]"), Some(&"world".to_string()));
    }

    #[test]
    fn test_cond_regex_match() {
        let opts = RegexOptions::default();
        let (matched, _) = cond_regex_match("hello world", "hello", &opts);
        assert!(matched);

        let (matched, _) = cond_regex_match("hello world", "goodbye", &opts);
        assert!(!matched);
    }

    #[test]
    fn test_byte_to_char_offset_ascii() {
        assert_eq!(byte_to_char_offset("hello", 0), 0);
        assert_eq!(byte_to_char_offset("hello", 5), 5);
    }

    #[test]
    fn test_byte_to_char_offset_unicode() {
        let s = "héllo";
        assert_eq!(byte_to_char_offset(s, 0), 0);
        assert_eq!(byte_to_char_offset(s, 1), 1);
        assert_eq!(byte_to_char_offset(s, 3), 2);
    }
}
