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

/// Replace `[word_start, word_end)` in `buffer` with `replacement`,
/// returning the new buffer plus updated cursor position.
/// Port of `instmatch()` from Src/Zle/compresult.c. The C source
/// uses this as the lowest-level "swap the partial word for the
/// chosen completion" primitive used by every other inserter
/// (`do_single`, `do_ambiguous`, `do_allmatches`).
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

/// Find the longest common prefix of every match — the substring the
/// completion engine inserts on the first Tab press when matches
/// are ambiguous.
/// Port of `unambig_data()` from Src/Zle/compresult.c. The C source
/// also tracks cursor placement within the prefix; ours returns
/// just the common-prefix string.
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

/// Case-insensitive variant of `unambig_data` — returns the common
/// prefix using the *first* match's casing for any case-folded
/// match.
/// Port of the case-insensitive branch of `unambig_data()` from
/// Src/Zle/compresult.c (the C source toggles based on the
/// `CASE_HACK` matcher flag).
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

/// Insert the single chosen match, optionally appending a space.
/// Port of `do_single()` from Src/Zle/compresult.c — fired when
/// completion produced exactly one match. The trailing space is
/// the `AUTO_REMOVE_SLASH`-aware insertion that distinguishes
/// finished-completion from prefix-completion.
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

/// Build the result for an ambiguous completion (multiple matches).
/// Port of `do_ambiguous()` from Src/Zle/compresult.c. The C source
/// inserts the unambiguous prefix into the buffer and triggers the
/// listing display; our Rust port returns a `CompResult` enum so
/// the caller decides whether to insert + list.
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

/// Insert every match into the buffer joined by `separator`.
/// Port of `do_allmatches()` from Src/Zle/compresult.c — fires for
/// the `all-matches` widget and for the implicit case when no
/// listing fits.
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

/// Step the menu cursor forward or backward, wrapping at the ends.
/// Port of `do_menucmp()` from Src/Zle/compresult.c. The C source
/// also handles per-group menu wrap; this Rust port treats the
/// match list as flat for the host's menu loop.
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

/// Accept the currently-selected menu match and finalise it into
/// the buffer.
/// Port of `accept_last()` from Src/Zle/compresult.c. Acts the same
/// as `do_single` with `add_space=true` since a confirmed selection
/// always wants a trailing space.
pub fn accept_last(
    buffer: &str,
    cursor: usize,
    word_start: usize,
    word_end: usize,
    selected: &str,
) -> (String, usize) {
    do_single(buffer, cursor, word_start, word_end, selected, true)
}

/// Test whether `word` satisfies the required prefix and suffix
/// constraints (the `compadd -P pre -S suf` requirements).
/// Port of `valid_match()` from Src/Zle/compresult.c.
pub fn valid_match(word: &str, prefix: &str, suffix: &str) -> bool {
    word.starts_with(prefix) && (suffix.is_empty() || word.ends_with(suffix))
}

/// Detect whether a string contains brace-expansion metacharacters
/// that would need quoting on insertion.
/// Port of `hasbrpsfx()` from Src/Zle/compresult.c — used by the
/// brace-suffix tracking that compsys keeps for menu completion.
pub fn hasbrpsfx(s: &str) -> bool {
    s.contains('{') || s.contains('}')
}

/// Render the "n/total" position label shown in the menu status
/// line.
/// Port of the position-string formatting in
/// Src/Zle/compresult.c (the `clprintm` group-header path).
pub fn build_pos_string(current: usize, total: usize) -> String {
    format!("{}/{}", current + 1, total)
}

/// Truncate a long completion line with `...` so it fits a column
/// budget.
/// Port of `cut_cline()` from Src/Zle/compresult.c. The C source
/// truncates the Cline's display field to `max_len`; ours emits
/// `…` (three ASCII dots) when truncation is needed.
pub fn cut_cline(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Concatenate the three text fields of a Cline back into a single
/// display string.
/// Port of `cline_str()` from Src/Zle/compresult.c. The C source
/// emits prefix + matched-region + suffix during list rendering;
/// the result here is what `compprintlist` writes to the screen.
pub fn cline_str(prefix: &str, line: &str, suffix: &str) -> String {
    format!("{}{}{}", prefix, line, suffix)
}

/// Compute how many rows the list will take given a fixed column
/// count.
/// Port of `list_lines()` from Src/Zle/compresult.c — the listing
/// path uses this to decide whether to invoke the more-prompt
/// (`asklistscroll`).
pub fn list_lines(matches: &[String], columns: usize) -> usize {
    if columns == 0 {
        return matches.len();
    }
    matches.len().div_ceil(columns)
}

/// Decide whether the listing exceeds `LISTMAX` and should be
/// suppressed.
/// Port of `skipnolist()` from Src/Zle/compresult.c. The C source
/// also consults `LISTMAX` in lines (negative LISTMAX); ours
/// honours just the "more than N matches" form.
pub fn skipnolist(matches: &[String], list_max: usize) -> bool {
    matches.len() > list_max && list_max > 0
}

/// Decide whether the match list fits on screen without scrolling.
/// Port of `comp_list()` from Src/Zle/compresult.c — the C source
/// is part of the "should we list inline or paginate?" branch in
/// `compprintlist()`.
pub fn comp_list(nmatches: usize, term_lines: usize) -> bool {
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

/// Port of `bld_all_str()` from Src/Zle/compresult.c:2187. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bld_all_str() -> i32 { 0 }

/// Port of `calclist()` from Src/Zle/compresult.c:1495. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn calclist() -> i32 { 0 }

/// Port of `do_ambig_menu()` from Src/Zle/compresult.c:1381. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn do_ambig_menu() -> i32 { 0 }

/// Port of `ilistmatches()` from Src/Zle/compresult.c:2284. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ilistmatches() -> i32 { 0 }

/// Port of `invalidate_list()` from Src/Zle/compresult.c:2334. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn invalidate_list() -> i32 { 0 }

/// Port of `iprintm()` from Src/Zle/compresult.c:2241. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn iprintm() -> i32 { 0 }

/// Port of `list_matches()` from Src/Zle/compresult.c:2304. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn list_matches() -> i32 { 0 }

/// Port of `printlist()` from Src/Zle/compresult.c:1978. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn printlist() -> i32 { 0 }
