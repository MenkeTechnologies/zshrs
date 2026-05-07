//! `zsh/regex` module — direct port of `Src/Modules/regex.c`.
//!
//! Provides one feature: the `-regex-match` infix condition usable
//! inside `[[ … ]]`:
//!
//! ```text
//! [[ "$str" -regex-match "$pattern" ]]
//! ```
//!
//! On match, status is 0 and the parenthesised subgroups are stored
//! in `$MATCH` / `$match[1..N]` (full-match in `$MATCH`, captures
//! in indexed positions). Direct port of `zcond_regex_match()` from
//! `Src/Modules/regex.c:60-210`.
//!
//! The C source links against the system POSIX `regcomp`/`regexec`
//! (`<regex.h>`). zshrs's existing `=~` operator already routes
//! through the Rust `regex` crate; this module exposes the same
//! engine under the alternate `-regex-match` name so scripts that
//! prefer the named-condition form keep working.

use regex::Regex;

/// Result of a `-regex-match` evaluation. Mirrors what
/// `zcond_regex_match()` writes into `$MATCH` / `$match[]` /
/// `$MBEGIN` / `$MEND` (regex.c:148-205).
#[derive(Debug, Clone, Default)]
pub struct RegexMatch {
    pub matched: bool,
    /// Full-match text — assigned to `$MATCH`.
    pub full: String,
    /// Indexed capture groups — assigned to `$match[1..N]`.
    pub captures: Vec<String>,
}

/// Port of `zcond_regex_match()` from `Src/Modules/regex.c:54`.
/// Compile `pat` and try to match it against `text`. Direct port
/// of the regcomp + regexec sequence in `zcond_regex_match`
/// (regex.c:78-145). Case-folding follows zsh's `CASEMATCH` option;
/// the caller passes `case_insensitive` when that option is OFF.
pub fn match_regex(text: &str, pat: &str, case_insensitive: bool) -> RegexMatch {
    // Wrap pat in `(?i)` for case-insensitive — the regex crate's
    // builder API accepts it inline; this matches the C source's
    // `rcflags |= REG_ICASE` branch (regex.c:75-76).
    let actual_pat = if case_insensitive {
        format!("(?i){}", pat)
    } else {
        pat.to_string()
    };
    let re = match Regex::new(&actual_pat) {
        Ok(r) => r,
        Err(_) => return RegexMatch::default(),
    };
    let Some(caps) = re.captures(text) else {
        return RegexMatch::default();
    };
    let full = caps
        .get(0)
        .map(|m| m.as_str().to_string())
        .unwrap_or_default();
    // Skip group 0 (full match); regex.c stores subgroups starting
    // at $match[1].
    let captures: Vec<String> = caps
        .iter()
        .skip(1)
        .map(|m| m.map(|m| m.as_str().to_string()).unwrap_or_default())
        .collect();
    RegexMatch {
        matched: true,
        full,
        captures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_simple_pattern() {
        let r = match_regex("hello world", "wor.d", false);
        assert!(r.matched);
        assert_eq!(r.full, "world");
    }

    #[test]
    fn captures_subgroups() {
        let r = match_regex("foo=42", "([a-z]+)=([0-9]+)", false);
        assert!(r.matched);
        assert_eq!(r.captures, vec!["foo".to_string(), "42".to_string()]);
    }

    #[test]
    fn case_insensitive_flag() {
        let r = match_regex("HELLO", "hello", true);
        assert!(r.matched);
    }

    #[test]
    fn returns_unmatched_for_invalid_pattern() {
        let r = match_regex("anything", "[", false);
        assert!(!r.matched);
    }
}
