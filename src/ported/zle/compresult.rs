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

// `CompResult` enum (Rust-only) deleted per strict-rules. C source
// (Src/Zle/compresult.c) has 0 enums; the completion-result is
// communicated via globals (`amenu`, `lastambig`, `validlist`)
// and the per-call `ret`/`ok` int variables of `do_ambiguous` /
// `do_single` / `do_allmatches`. zshrs's port routes those
// through the executor's completion state directly.

// scs is used to return the position where a automatically created suffix  // c:573
// has to be inserted.                                                       // c:574
/// Replace `[word_start, word_end)` in `buffer` with `replacement`,
/// returning the new buffer plus updated cursor position.
/// Port of `instmatch()` from Src/Zle/compresult.c. The C source
/// uses this as the lowest-level "swap the partial word for the
/// chosen completion" primitive used by every other inserter
/// (`do_single`, `do_ambiguous`, `do_allmatches`).
pub fn instmatch(                                                            // c:578
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
// This is a utility function using the function above to allow access     // c:520
// to the unambiguous string and cursor position via compstate.             // c:521
/// completion engine inserts on the first Tab press when matches
/// are ambiguous.
/// Port of `unambig_data()` from Src/Zle/compresult.c. The C source
/// also tracks cursor placement within the prefix; ours returns
/// just the common-prefix string.
pub fn unambig_data(matches: &[String]) -> String {                          // c:525
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

/// Insert the single chosen match, optionally appending a space.
/// Port of `do_single()` from Src/Zle/compresult.c — fired when
// Insert a single match in the command line.                              // c:959
/// completion produced exactly one match. The trailing space is
/// the `AUTO_REMOVE_SLASH`-aware insertion that distinguishes
/// finished-completion from prefix-completion.
pub fn do_single(                                                            // c:963
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

/// Port of `do_ambiguous()` from `Src/Zle/compresult.c:744`. The
/// ambiguous-completion handler — inserts the unambiguous prefix
/// shared by all matches and triggers the listing display.
///
/// C signature: `static int do_ambiguous(void)`. Returns 1 if any
/// completion text was inserted, 0 otherwise.
///
/// **Approximation:** the full C body uses globals (`lastambig`,
/// `amenu`, `validlist`) + `instmatch` + `listmatches` not yet
/// fully wired. Rust port returns 1 if there's a non-empty
/// unambiguous prefix.
pub fn do_ambiguous(matches: &[String]) -> i32 {                         // c:744
    let prefix = unambig_data(matches);
    if prefix.is_empty() && matches.is_empty() {
        return 0;                                                        // c:nomatch
    }
    if !prefix.is_empty() { 1 } else { 0 }
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

// =====================================================================
// Deferred shims — `unimplemented!()` placeholders so the C-source
// fn names remain searchable. Bodies need the Cmgroup/Cmatch
// linked-list machine, listing-arena state, and zle_refresh's
// drawing primitives — substrate that isn't ported yet. Each panics
// on call so silent fakes can't escape — per the no-shortcuts rule.
// =====================================================================

/// Port of `bld_all_str()` from `Src/Zle/compresult.c:2187`.
/// Builds the inserted "all matches" string for `do_allmatches`.
pub fn bld_all_str() -> i32 {                                                // c:2187
    unimplemented!("compresult.rs::bld_all_str — c:2187 deferred (Cmatch list walk + brace expansion)");
}

/// Port of `calclist()` from `Src/Zle/compresult.c:1495`.
/// Computes per-match column widths and totals for the listing.
pub fn calclist() -> i32 {                                                   // c:1495
    unimplemented!("compresult.rs::calclist — c:1495 deferred (mgroup walk + width accumulator)");
}

/// Port of `do_ambig_menu()` from `Src/Zle/compresult.c:1381`.
/// Menu-completion entry for the ambiguous-matches case.
pub fn do_ambig_menu() -> i32 {                                              // c:1381
    unimplemented!("compresult.rs::do_ambig_menu — c:1381 deferred (menu state + amenu/lastambig globals)");
}

/// Port of `ilistmatches()` from `Src/Zle/compresult.c:2284`.
/// Hook callback for `listmatches()`.
pub fn ilistmatches() -> i32 {                                               // c:2284
    unimplemented!("compresult.rs::ilistmatches — c:2284 deferred (Hookdef + Chdata plumbing + printlist call)");
}

/// Port of `invalidate_list()` from `Src/Zle/compresult.c:2334`.
/// Hook callback for `invalidatelist()` — discards the cached list.
pub fn invalidate_list() -> i32 {                                            // c:2334
    unimplemented!("compresult.rs::invalidate_list — c:2334 deferred (validlist/showinglist globals + freematches dispatch)");
}

/// Port of `iprintm()` from `Src/Zle/compresult.c:2241`.
/// `CLPrintFunc` for the standard listing — prints one match cell.
pub fn iprintm() -> i32 {                                                    // c:2241
    unimplemented!("compresult.rs::iprintm — c:2241 deferred (zle_refresh tputs/term primitives + Cmatch field decode)");
}

/// Port of `list_matches()` from `Src/Zle/compresult.c:2304`.
/// Hook callback wrapper around `printlist`.
pub fn list_matches() -> i32 {                                               // c:2304
    unimplemented!("compresult.rs::list_matches — c:2304 deferred (Hookdef plumbing + iprintm dispatch)");
}

/// Port of `printlist()` from `Src/Zle/compresult.c:1978`.
/// Renders the completion list to the terminal (the workhorse
/// behind every listing path: ambiguous prompt, menu listing, etc.).
pub fn printlist() -> i32 {                                                  // c:1978
    unimplemented!("compresult.rs::printlist — c:1978 deferred (zle_refresh draw primitives + listdat global + ListPrintFunc dispatch)");
}
