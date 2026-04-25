//! Completion matching specifications (-M option)
//!
//! Matcher control in zsh is complex. This implements the basic forms:
//! - m:pattern=replacement - match pattern, allow replacement
//! - l:anchor|pattern=replacement - left-anchored
//! - r:pattern|anchor=replacement - right-anchored  
//! - b:anchor|pattern=replacement - both ends
//! - L, R, B, M - case variants (line vs word)

use std::fmt;

/// Type of matcher
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatcherType {
    /// m: simple match
    Simple,
    /// l: left-anchored
    Left,
    /// r: right-anchored
    Right,
    /// b: both ends (interleaved)
    Both,
}

/// Pattern element in a matcher
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternElement {
    /// Literal character
    Char(char),
    /// Any single character (?)
    Any,
    /// Character class [abc] or [^abc]
    Class { chars: String, negated: bool },
    /// Equivalence class {a-z}
    Equiv(String),
}

impl PatternElement {
    pub fn matches(&self, c: char) -> bool {
        match self {
            Self::Char(ch) => *ch == c,
            Self::Any => true,
            Self::Class { chars, negated } => {
                let found = chars.contains(c);
                if *negated {
                    !found
                } else {
                    found
                }
            }
            Self::Equiv(chars) => chars.contains(c),
        }
    }
}

/// A single matcher specification
#[derive(Clone, Debug)]
pub struct Matcher {
    pub typ: MatcherType,
    /// Line pattern (what to match in the command line)
    pub line_pattern: Vec<PatternElement>,
    /// Word pattern (what to match in completion candidates)
    pub word_pattern: Vec<PatternElement>,
    /// Left anchor pattern
    pub left_anchor: Vec<PatternElement>,
    /// Right anchor pattern
    pub right_anchor: Vec<PatternElement>,
    /// Match on line side (uppercase) vs word side (lowercase)
    pub line_side: bool,
}

impl Default for Matcher {
    fn default() -> Self {
        Self {
            typ: MatcherType::Simple,
            line_pattern: Vec::new(),
            word_pattern: Vec::new(),
            left_anchor: Vec::new(),
            right_anchor: Vec::new(),
            line_side: false,
        }
    }
}

/// A complete match specification (can have multiple matchers)
#[derive(Clone, Debug, Default)]
pub struct MatchSpec {
    pub matchers: Vec<Matcher>,
}

impl MatchSpec {
    /// Parse a match specification string
    /// Format: "m:pat=repl l:anch|pat=repl r:pat|anch=repl b:anch|pat|anch=repl"
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut matchers = Vec::new();

        for part in spec.split_whitespace() {
            if part.is_empty() {
                continue;
            }

            let matcher = parse_single_matcher(part)?;
            matchers.push(matcher);
        }

        Ok(Self { matchers })
    }

    /// Check if a word matches against a prefix using these matchers
    pub fn matches(&self, word: &str, prefix: &str) -> bool {
        if self.matchers.is_empty() {
            // Default: case-insensitive prefix match
            return word.to_lowercase().starts_with(&prefix.to_lowercase());
        }

        // Try each matcher
        for matcher in &self.matchers {
            if matcher_matches(matcher, word, prefix) {
                return true;
            }
        }

        // Also try default matching
        word.to_lowercase().starts_with(&prefix.to_lowercase())
    }

    /// Common matcher: case-insensitive
    pub fn case_insensitive() -> Self {
        // m:{a-zA-Z}={A-Za-z}
        Self {
            matchers: vec![Matcher {
                typ: MatcherType::Simple,
                line_pattern: vec![PatternElement::Equiv(
                    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ".to_string(),
                )],
                word_pattern: vec![PatternElement::Equiv(
                    "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz".to_string(),
                )],
                ..Default::default()
            }],
        }
    }

    /// Common matcher: partial word completion (l:|=*)
    pub fn partial_word() -> Self {
        // l:|=* r:|=*
        Self {
            matchers: vec![
                Matcher {
                    typ: MatcherType::Left,
                    word_pattern: vec![],
                    ..Default::default()
                },
                Matcher {
                    typ: MatcherType::Right,
                    word_pattern: vec![],
                    ..Default::default()
                },
            ],
        }
    }
}

/// Parse a single matcher like "m:pat=repl"
fn parse_single_matcher(s: &str) -> Result<Matcher, String> {
    let mut matcher = Matcher::default();

    // Check for type prefix
    let (typ_char, rest) = if s.len() >= 2 && s.chars().nth(1) == Some(':') {
        (s.chars().next().unwrap(), &s[2..])
    } else {
        return Err(format!("invalid matcher format: {}", s));
    };

    matcher.line_side = typ_char.is_uppercase();

    matcher.typ = match typ_char.to_ascii_lowercase() {
        'm' => MatcherType::Simple,
        'l' => MatcherType::Left,
        'r' => MatcherType::Right,
        'b' => MatcherType::Both,
        'e' => MatcherType::Both, // 'e' is interleaved like 'b'
        'x' => {
            // x: terminates matching
            return Ok(matcher);
        }
        _ => return Err(format!("unknown matcher type: {}", typ_char)),
    };

    // Parse the rest based on type
    match matcher.typ {
        MatcherType::Simple => {
            // m:line=word
            let parts: Vec<&str> = rest.splitn(2, '=').collect();
            if parts.len() == 2 {
                matcher.line_pattern = parse_pattern(parts[0])?;
                matcher.word_pattern = parse_pattern(parts[1])?;
            } else {
                matcher.line_pattern = parse_pattern(rest)?;
                matcher.word_pattern = matcher.line_pattern.clone();
            }
        }
        MatcherType::Left => {
            // l:anchor|line=word
            if let Some(pipe_pos) = rest.find('|') {
                matcher.left_anchor = parse_pattern(&rest[..pipe_pos])?;
                let after_anchor = &rest[pipe_pos + 1..];
                let parts: Vec<&str> = after_anchor.splitn(2, '=').collect();
                if parts.len() == 2 {
                    matcher.line_pattern = parse_pattern(parts[0])?;
                    matcher.word_pattern = parse_pattern(parts[1])?;
                } else {
                    matcher.line_pattern = parse_pattern(after_anchor)?;
                }
            } else {
                let parts: Vec<&str> = rest.splitn(2, '=').collect();
                if parts.len() == 2 {
                    matcher.line_pattern = parse_pattern(parts[0])?;
                    matcher.word_pattern = parse_pattern(parts[1])?;
                }
            }
        }
        MatcherType::Right => {
            // r:line|anchor=word
            let parts: Vec<&str> = rest.splitn(2, '=').collect();
            let main = parts[0];

            if let Some(pipe_pos) = main.find('|') {
                matcher.line_pattern = parse_pattern(&main[..pipe_pos])?;
                matcher.right_anchor = parse_pattern(&main[pipe_pos + 1..])?;
            } else {
                matcher.line_pattern = parse_pattern(main)?;
            }

            if parts.len() == 2 {
                matcher.word_pattern = parse_pattern(parts[1])?;
            }
        }
        MatcherType::Both => {
            // b:left|line|right=word or just b:left|line=word
            let parts: Vec<&str> = rest.splitn(2, '=').collect();
            let main = parts[0];
            let anchors: Vec<&str> = main.split('|').collect();

            match anchors.len() {
                1 => {
                    matcher.line_pattern = parse_pattern(anchors[0])?;
                }
                2 => {
                    matcher.left_anchor = parse_pattern(anchors[0])?;
                    matcher.line_pattern = parse_pattern(anchors[1])?;
                }
                3 => {
                    matcher.left_anchor = parse_pattern(anchors[0])?;
                    matcher.line_pattern = parse_pattern(anchors[1])?;
                    matcher.right_anchor = parse_pattern(anchors[2])?;
                }
                _ => return Err("too many | in matcher".to_string()),
            }

            if parts.len() == 2 {
                matcher.word_pattern = parse_pattern(parts[1])?;
            }
        }
    }

    Ok(matcher)
}

/// Parse a pattern string into elements
fn parse_pattern(s: &str) -> Result<Vec<PatternElement>, String> {
    let mut elements = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];

        match c {
            '?' => {
                elements.push(PatternElement::Any);
            }
            '*' => {
                // * in matcher means "any number of any char" - represented as empty
                // This is handled specially in matching
            }
            '[' => {
                // Character class
                let mut negated = false;
                i += 1;
                if i < chars.len() && (chars[i] == '!' || chars[i] == '^') {
                    negated = true;
                    i += 1;
                }
                let mut class_chars = String::new();
                while i < chars.len() && chars[i] != ']' {
                    if chars[i] == '-'
                        && !class_chars.is_empty()
                        && i + 1 < chars.len()
                        && chars[i + 1] != ']'
                    {
                        // Range
                        let start = class_chars.pop().unwrap();
                        i += 1;
                        let end = chars[i];
                        for c in start..=end {
                            class_chars.push(c);
                        }
                    } else {
                        class_chars.push(chars[i]);
                    }
                    i += 1;
                }
                elements.push(PatternElement::Class {
                    chars: class_chars,
                    negated,
                });
            }
            '{' => {
                // Equivalence class
                i += 1;
                let mut equiv_chars = String::new();
                while i < chars.len() && chars[i] != '}' {
                    if chars[i] == '-'
                        && !equiv_chars.is_empty()
                        && i + 1 < chars.len()
                        && chars[i + 1] != '}'
                    {
                        let start = equiv_chars.pop().unwrap();
                        i += 1;
                        let end = chars[i];
                        for c in start..=end {
                            equiv_chars.push(c);
                        }
                    } else {
                        equiv_chars.push(chars[i]);
                    }
                    i += 1;
                }
                elements.push(PatternElement::Equiv(equiv_chars));
            }
            '\\' if i + 1 < chars.len() => {
                i += 1;
                elements.push(PatternElement::Char(chars[i]));
            }
            _ => {
                elements.push(PatternElement::Char(c));
            }
        }

        i += 1;
    }

    Ok(elements)
}

/// Check if a word matches a prefix using a specific matcher
fn matcher_matches(matcher: &Matcher, word: &str, prefix: &str) -> bool {
    // For empty patterns, this is anchor-only matching
    if matcher.line_pattern.is_empty() && matcher.word_pattern.is_empty() {
        return true;
    }

    let word_chars: Vec<char> = word.chars().collect();
    let prefix_chars: Vec<char> = prefix.chars().collect();

    // Simple case: pattern-based character equivalence
    let mut wi = 0;
    let mut pi = 0;

    while pi < prefix_chars.len() && wi < word_chars.len() {
        let pc = prefix_chars[pi];
        let wc = word_chars[wi];

        // Check if characters match via the matcher patterns
        let matches = if pc == wc {
            true
        } else {
            // Check equivalence via patterns
            let line_matches = matcher.line_pattern.iter().any(|p| p.matches(pc));
            let word_matches = matcher.word_pattern.iter().any(|p| p.matches(wc));
            line_matches && word_matches
        };

        if matches {
            pi += 1;
            wi += 1;
        } else {
            return false;
        }
    }

    // All prefix characters must be consumed
    pi == prefix_chars.len()
}

impl fmt::Display for MatchSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.matchers.iter().map(|m| format!("{:?}", m)).collect();
        write!(f, "{}", parts.join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let spec = MatchSpec::parse("m:a=b").unwrap();
        assert_eq!(spec.matchers.len(), 1);
        assert_eq!(spec.matchers[0].typ, MatcherType::Simple);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let spec = MatchSpec::parse("m:{a-z}={A-Z}").unwrap();
        assert_eq!(spec.matchers.len(), 1);
    }

    #[test]
    fn test_case_insensitive_matching() {
        let spec = MatchSpec::case_insensitive();
        assert!(spec.matches("Foo", "foo"));
        assert!(spec.matches("FOO", "foo"));
        assert!(spec.matches("foo", "FOO"));
    }

    #[test]
    fn test_default_matching() {
        let spec = MatchSpec::default();
        assert!(spec.matches("foobar", "foo"));
        assert!(spec.matches("Foobar", "foo")); // case insensitive by default
        assert!(!spec.matches("barfoo", "foo"));
    }

    #[test]
    fn test_pattern_element() {
        assert!(PatternElement::Char('a').matches('a'));
        assert!(!PatternElement::Char('a').matches('b'));

        assert!(PatternElement::Any.matches('x'));

        let class = PatternElement::Class {
            chars: "abc".to_string(),
            negated: false,
        };
        assert!(class.matches('a'));
        assert!(!class.matches('d'));

        let neg_class = PatternElement::Class {
            chars: "abc".to_string(),
            negated: true,
        };
        assert!(!neg_class.matches('a'));
        assert!(neg_class.matches('d'));
    }
}
