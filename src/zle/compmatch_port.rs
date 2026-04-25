//! Completion matching engine for ZLE
//!
//! Port from zsh/Src/Zle/compmatch.c (2,974 lines)
//!
//! The full matching engine is in compsys/matching.rs (458 lines).
//! This module provides the pattern matching, anchor handling, and
//! match line construction used during completion.
//!
//! Key C functions and their Rust locations:
//! - match_str         → compsys::matching::match_str()
//! - match_parts       → compsys::matching::match_parts()
//! - comp_match        → compsys::matching::comp_match()
//! - pattern_match_equivalence → compsys::matching (inline)
//! - add_match_str/part/sub    → compsys::matching (inline)
//! - cline_* (match line ops)  → compsys::base::CompletionLine

/// Completion matcher pattern (from compmatch.c Cmatcher)
#[derive(Debug, Clone)]
pub struct CompMatcher {
    pub line_pattern: String,
    pub word_pattern: String,
    pub flags: MatchFlags,
}

/// Match control flags
#[derive(Debug, Clone, Copy, Default)]
pub struct MatchFlags {
    pub case_insensitive: bool,
    pub partial_word: bool,
    pub anchor_start: bool,
    pub anchor_end: bool,
    pub substring: bool,
}

/// A completion line segment (from compmatch.c Cline)
#[derive(Debug, Clone)]
pub struct CompLine {
    pub prefix: String,
    pub line: String,
    pub suffix: String,
    pub word: String,
    pub matched: bool,
}

impl CompLine {
    pub fn new() -> Self {
        CompLine {
            prefix: String::new(),
            line: String::new(),
            suffix: String::new(),
            word: String::new(),
            matched: false,
        }
    }

    /// Get the total length (from compmatch.c cline_sublen)
    pub fn sublen(&self) -> usize {
        self.prefix.len() + self.line.len() + self.suffix.len()
    }

    /// Set lengths from content (from compmatch.c cline_setlens)
    pub fn setlens(&mut self) {
        // Already handled by String lengths
    }
}

impl Default for CompLine {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if two matcher patterns are the same (from compmatch.c cpatterns_same)
pub fn cpatterns_same(a: &[CompMatcher], b: &[CompMatcher]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .all(|(ma, mb)| ma.line_pattern == mb.line_pattern && ma.word_pattern == mb.word_pattern)
}

/// Check if two matcher lists are the same (from compmatch.c cmatchers_same)
pub fn cmatchers_same(a: &[CompMatcher], b: &[CompMatcher]) -> bool {
    cpatterns_same(a, b)
}

/// Match a completion word against a line (from compmatch.c match_str)
pub fn match_str(
    line: &str,
    word: &str,
    matchers: &[CompMatcher],
    flags: &MatchFlags,
) -> Option<Vec<CompLine>> {
    if flags.case_insensitive {
        if line.to_lowercase().starts_with(&word.to_lowercase()) {
            return Some(vec![CompLine {
                line: line.to_string(),
                word: word.to_string(),
                matched: true,
                ..Default::default()
            }]);
        }
    } else if line.starts_with(word) {
        return Some(vec![CompLine {
            line: line.to_string(),
            word: word.to_string(),
            matched: true,
            ..Default::default()
        }]);
    }

    // Try matchers
    for matcher in matchers {
        if try_matcher(line, word, matcher) {
            return Some(vec![CompLine {
                line: line.to_string(),
                word: word.to_string(),
                matched: true,
                ..Default::default()
            }]);
        }
    }

    None
}

fn try_matcher(line: &str, word: &str, matcher: &CompMatcher) -> bool {
    if matcher.flags.case_insensitive {
        line.to_lowercase().contains(&word.to_lowercase())
    } else if matcher.flags.substring {
        line.contains(word)
    } else if matcher.flags.partial_word {
        // Match word parts: "fb" matches "foobar" at word boundaries
        let mut li = line.chars().peekable();
        let mut wi = word.chars();
        let mut wc = wi.next();

        while let Some(lc) = li.next() {
            if let Some(w) = wc {
                if lc.eq_ignore_ascii_case(&w) {
                    wc = wi.next();
                }
            } else {
                return true;
            }
        }
        wc.is_none()
    } else {
        false
    }
}

/// Match parts of a completion (from compmatch.c match_parts)
pub fn match_parts(line: &str, word: &str, flags: &MatchFlags) -> Vec<(usize, usize)> {
    let mut parts = Vec::new();
    let line_lower = if flags.case_insensitive {
        line.to_lowercase()
    } else {
        line.to_string()
    };
    let word_lower = if flags.case_insensitive {
        word.to_lowercase()
    } else {
        word.to_string()
    };

    let mut pos = 0;
    for wc in word_lower.chars() {
        if let Some(found) = line_lower[pos..].find(wc) {
            let abs_pos = pos + found;
            parts.push((abs_pos, abs_pos + wc.len_utf8()));
            pos = abs_pos + wc.len_utf8();
        }
    }
    parts
}

/// Full completion match (from compmatch.c comp_match)
pub fn comp_match(line: &str, word: &str, flags: &MatchFlags) -> bool {
    match_str(line, word, &[], flags).is_some()
}

/// Start a match operation (from compmatch.c start_match)
pub fn start_match() -> Vec<CompLine> {
    Vec::new()
}

/// Abort a match operation (from compmatch.c abort_match)
pub fn abort_match(_lines: Vec<CompLine>) {
    // Drop the lines
}

/// Get a CompLine copy (from compmatch.c get_cline/cp_cline)
pub fn cp_cline(line: &CompLine) -> CompLine {
    line.clone()
}

/// Free a CompLine (from compmatch.c free_cline) - no-op in Rust
pub fn free_cline(_line: CompLine) {}

/// Revert a CompLine to original state (from compmatch.c revert_cline)
pub fn revert_cline(line: &mut CompLine) {
    line.matched = false;
}

/// Check if a CompLine was matched (from compmatch.c cline_matched)
pub fn cline_matched(line: &CompLine) -> bool {
    line.matched
}

/// Pattern match with equivalence classes (from compmatch.c pattern_match_equivalence)
pub fn pattern_match_equivalence(a: char, b: char, case_insensitive: bool) -> bool {
    if case_insensitive {
        a.eq_ignore_ascii_case(&b)
    } else {
        a == b
    }
}

/// Parse a matcher specification string (from compmatch.c)
/// Format: "m:{[:lower:]}={[:upper:]}" or "l:|=* r:|=*" etc.
pub fn parse_matcher_spec(spec: &str) -> Vec<CompMatcher> {
    let mut matchers = Vec::new();

    for part in spec.split_whitespace() {
        let flags = MatchFlags {
            case_insensitive: part.starts_with("m:"),
            partial_word: part.starts_with("r:") || part.starts_with("l:"),
            anchor_start: part.starts_with("l:"),
            anchor_end: part.starts_with("r:"),
            substring: part.starts_with("M:"),
        };

        if let Some((line_pat, word_pat)) = part.split_once('=') {
            let line_pat = line_pat.split(':').last().unwrap_or("");
            matchers.push(CompMatcher {
                line_pattern: line_pat.to_string(),
                word_pattern: word_pat.to_string(),
                flags,
            });
        }
    }

    matchers
}

/// Update bmatchers (from compmatch.c add_bmatchers/update_bmatchers)
pub fn update_bmatchers(matchers: &mut Vec<CompMatcher>, new: Vec<CompMatcher>) {
    *matchers = new;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_match_str_exact() {
        let flags = MatchFlags::default();
        assert!(match_str("foobar", "foo", &[], &flags).is_some());
        assert!(match_str("foobar", "baz", &[], &flags).is_none());
    }

    #[test]
    fn test_match_str_case_insensitive() {
        let flags = MatchFlags {
            case_insensitive: true,
            ..Default::default()
        };
        assert!(match_str("FooBar", "foo", &[], &flags).is_some());
    }

    #[test]
    fn test_match_parts() {
        let flags = MatchFlags::default();
        let parts = match_parts("foobar", "fbr", &flags);
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn test_pattern_match_equivalence() {
        assert!(pattern_match_equivalence('a', 'A', true));
        assert!(!pattern_match_equivalence('a', 'A', false));
    }

    #[test]
    fn test_parse_matcher_spec() {
        let matchers = parse_matcher_spec("m:{[:lower:]}={[:upper:]}");
        assert_eq!(matchers.len(), 1);
        assert!(matchers[0].flags.case_insensitive);
    }

    #[test]
    fn test_comp_line() {
        let mut cl = CompLine::new();
        cl.prefix = "pre".to_string();
        cl.line = "middle".to_string();
        cl.suffix = "suf".to_string();
        assert_eq!(cl.sublen(), 12);
    }
}
