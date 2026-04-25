//! Completion result handling for ZLE
//!
//! Port from zsh/Src/Zle/compresult.c (2,359 lines)
//!
//! Handles insertion of completion results into the edit buffer:
//! unambiguous prefix insertion, menu cycling, single match auto-insert,
//! and ambiguous match handling.
//!
//! Key C functions and their Rust locations:
//! - do_single       → single unambiguous match insertion
//! - do_ambiguous     → handle multiple matches (list or menu)
//! - do_allmatches    → insert all matches
//! - do_menucmp       → menu completion cycling
//! - accept_last      → accept current menu selection
//! - instmatch        → insert a match into the buffer
//! - unambig_data     → compute unambiguous prefix
//! - build_pos_string → build position string for match

/// Result of completion attempt
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompResult {
    /// No matches found
    NoMatch,
    /// Single unambiguous match — insert it
    Single(String),
    /// Multiple matches — show list or enter menu
    Ambiguous {
        prefix: String,
        matches: Vec<String>,
    },
    /// Menu completion — cycling through matches
    Menu {
        current: usize,
        matches: Vec<String>,
    },
}

/// Insert a match into the buffer (from compresult.c instmatch)
pub fn instmatch(
    buffer: &str,
    cursor: usize,
    word_start: usize,
    word_end: usize,
    replacement: &str,
) -> (String, usize) {
    let mut result = String::with_capacity(buffer.len() + replacement.len());
    result.push_str(&buffer[..word_start]);
    result.push_str(replacement);
    result.push_str(&buffer[word_end..]);
    let new_cursor = word_start + replacement.len();
    (result, new_cursor)
}

/// Compute the longest unambiguous prefix of matches (from compresult.c unambig_data)
pub fn unambig_data(matches: &[String]) -> String {
    if matches.is_empty() {
        return String::new();
    }
    if matches.len() == 1 {
        return matches[0].clone();
    }

    let first = &matches[0];
    let mut prefix_len = first.len();

    for m in &matches[1..] {
        let common = first
            .chars()
            .zip(m.chars())
            .take_while(|(a, b)| a == b)
            .count();
        prefix_len = prefix_len.min(common);
    }

    first[..first
        .char_indices()
        .nth(prefix_len)
        .map(|(i, _)| i)
        .unwrap_or(first.len())]
        .to_string()
}

/// Case-insensitive unambiguous prefix
pub fn unambig_data_icase(matches: &[String]) -> String {
    if matches.is_empty() {
        return String::new();
    }
    if matches.len() == 1 {
        return matches[0].clone();
    }

    let first = matches[0].to_lowercase();
    let mut prefix_len = first.len();

    for m in &matches[1..] {
        let lower = m.to_lowercase();
        let common = first
            .chars()
            .zip(lower.chars())
            .take_while(|(a, b)| a == b)
            .count();
        prefix_len = prefix_len.min(common);
    }

    // Return using the case from the first match
    let first = &matches[0];
    first[..first
        .char_indices()
        .nth(prefix_len)
        .map(|(i, _)| i)
        .unwrap_or(first.len())]
        .to_string()
}

/// Handle a single unambiguous match (from compresult.c do_single)
pub fn do_single(
    buffer: &str,
    cursor: usize,
    word_start: usize,
    word_end: usize,
    the_match: &str,
    add_space: bool,
) -> (String, usize) {
    let suffix = if add_space { " " } else { "" };
    let replacement = format!("{}{}", the_match, suffix);
    instmatch(buffer, cursor, word_start, word_end, &replacement)
}

/// Handle ambiguous matches (from compresult.c do_ambiguous)
pub fn do_ambiguous(matches: &[String]) -> CompResult {
    let prefix = unambig_data(matches);
    if prefix.is_empty() && matches.is_empty() {
        CompResult::NoMatch
    } else {
        CompResult::Ambiguous {
            prefix,
            matches: matches.to_vec(),
        }
    }
}

/// Insert all matches (from compresult.c do_allmatches)
pub fn do_allmatches(
    buffer: &str,
    cursor: usize,
    word_start: usize,
    word_end: usize,
    matches: &[String],
    separator: &str,
) -> (String, usize) {
    let all = matches.join(separator);
    instmatch(buffer, cursor, word_start, word_end, &all)
}

/// Menu completion: get next match (from compresult.c do_menucmp)
pub fn do_menucmp(matches: &[String], current: usize, forward: bool) -> (usize, &str) {
    if matches.is_empty() {
        return (0, "");
    }
    let next = if forward {
        (current + 1) % matches.len()
    } else {
        if current == 0 {
            matches.len() - 1
        } else {
            current - 1
        }
    };
    (next, &matches[next])
}

/// Accept current menu selection (from compresult.c accept_last)
pub fn accept_last(
    buffer: &str,
    cursor: usize,
    word_start: usize,
    word_end: usize,
    selected: &str,
) -> (String, usize) {
    do_single(buffer, cursor, word_start, word_end, selected, true)
}

/// Check if a match is valid (has required prefix/suffix) (from compresult.c valid_match)
pub fn valid_match(word: &str, prefix: &str, suffix: &str) -> bool {
    word.starts_with(prefix) && (suffix.is_empty() || word.ends_with(suffix))
}

/// Check if match has a brace prefix/suffix (from compresult.c hasbrpsfx)
pub fn hasbrpsfx(s: &str) -> bool {
    s.contains('{') || s.contains('}')
}

/// Build position string for display (from compresult.c build_pos_string)
pub fn build_pos_string(current: usize, total: usize) -> String {
    format!("{}/{}", current + 1, total)
}

/// Cut completion line for insertion (from compresult.c cut_cline)
pub fn cut_cline(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Build string from completion line (from compresult.c cline_str)
pub fn cline_str(prefix: &str, line: &str, suffix: &str) -> String {
    format!("{}{}{}", prefix, line, suffix)
}

/// Determine number of lines needed to display list (from compresult.c list_lines)
pub fn list_lines(matches: &[String], columns: usize) -> usize {
    if columns == 0 {
        return matches.len();
    }
    (matches.len() + columns - 1) / columns
}

/// Check if listing should be skipped (from compresult.c skipnolist)
pub fn skipnolist(matches: &[String], list_max: usize) -> bool {
    matches.len() > list_max && list_max > 0
}

/// Determine completion list layout (from compresult.c comp_list)
pub fn comp_list(nmatches: usize, term_lines: usize) -> bool {
    // Return true if list fits on screen
    nmatches < term_lines
}

/// Ask whether to show list (from compresult.c asklist)
pub fn asklist(nmatches: usize) -> String {
    format!("zsh: do you wish to see all {} possibilities? ", nmatches)
}

/// Get file status for completion coloring (from compresult.c ztat)
pub fn ztat(path: &str) -> Option<std::fs::Metadata> {
    std::fs::metadata(path).ok()
}

/// Modify completion result (from compresult.c comp_mod)
pub fn comp_mod(result: &str, to_end: bool) -> String {
    if to_end {
        format!("{} ", result) // add trailing space
    } else {
        result.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unambig_data() {
        assert_eq!(unambig_data(&["foobar".into(), "foobaz".into()]), "fooba");
        assert_eq!(unambig_data(&["abc".into()]), "abc");
        assert_eq!(unambig_data(&[]), "");
    }

    #[test]
    fn test_instmatch() {
        let (result, cursor) = instmatch("git co", 6, 4, 6, "commit");
        assert_eq!(result, "git commit");
        assert_eq!(cursor, 10);
    }

    #[test]
    fn test_do_single() {
        let (result, cursor) = do_single("git co", 6, 4, 6, "commit", true);
        assert_eq!(result, "git commit ");
        assert_eq!(cursor, 11);
    }

    #[test]
    fn test_do_menucmp() {
        let matches = vec!["commit".into(), "checkout".into(), "cherry-pick".into()];
        let (next, word) = do_menucmp(&matches, 0, true);
        assert_eq!(next, 1);
        assert_eq!(word, "checkout");

        let (next, word) = do_menucmp(&matches, 2, true);
        assert_eq!(next, 0);
        assert_eq!(word, "commit");
    }

    #[test]
    fn test_valid_match() {
        assert!(valid_match("foobar", "foo", ""));
        assert!(valid_match("foobar", "foo", "bar"));
        assert!(!valid_match("foobar", "baz", ""));
    }

    #[test]
    fn test_build_pos_string() {
        assert_eq!(build_pos_string(0, 10), "1/10");
        assert_eq!(build_pos_string(9, 10), "10/10");
    }

    #[test]
    fn test_list_lines() {
        assert_eq!(list_lines(&vec!["a".into(); 10], 3), 4);
        assert_eq!(list_lines(&vec!["a".into(); 6], 3), 2);
    }
}
