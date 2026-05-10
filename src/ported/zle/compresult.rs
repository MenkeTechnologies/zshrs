//! Completion result handling for ZLE
//!
//! Port from zsh/Src/Zle/compresult.c (2,359 lines)
//!
//! Handle the case were we found more than one match.                       // c:740
//! Insert all matches in the command line.                                  // c:893
//! This handles the beginning of menu-completion.                           // c:1377
//! List the matches.                                                        // c:2300
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

/// Port of `skipnolist()` from `Src/Zle/compresult.c:1480`.
/// ```c
/// mod_export Cmatch *
/// skipnolist(Cmatch *p, int showall)
/// {
///     int mask = (showall ? 0 : (CMF_NOLIST | CMF_MULT)) | CMF_HIDE;
///     while (*p && (((*p)->flags & mask) ||
///                   ((*p)->disp &&
///                    ((*p)->flags & (CMF_DISPLINE | CMF_HIDE)))))
///         p++;
///     return p;
/// }
/// ```
/// Walk a `Cmatch*` array skipping over entries that won't be
/// listed (CMF_NOLIST/CMF_MULT/CMF_HIDE) and over disp-strings
/// that are CMF_DISPLINE/CMF_HIDE. Returns the index of the first
/// listable entry (or `matches.len()` if none).
///
/// `showall` mirrors C: when non-zero, the NOLIST/MULT mask is
/// dropped (only CMF_HIDE filters).
pub fn skipnolist(matches: &[crate::ported::zle::comp_h::Cmatch], showall: i32) -> usize {  // c:1480
    use crate::ported::zle::comp_h::{CMF_DISPLINE, CMF_HIDE, CMF_MULT, CMF_NOLIST};
    // c:1483 — `mask = (showall ? 0 : (CMF_NOLIST|CMF_MULT)) | CMF_HIDE`.
    let mask = if showall != 0 { 0 } else { CMF_NOLIST | CMF_MULT } | CMF_HIDE;
    let mut p = 0usize;                                                          // c:1485 *p
    while p < matches.len() {                                                    // c:1485 while (*p && ...)
        let m = &matches[p];
        let f = m.flags;
        let skip_mask    = (f & mask) != 0;                                      // c:1485
        let skip_disp    = m.disp.is_some() && (f & (CMF_DISPLINE | CMF_HIDE)) != 0; // c:1486-1487
        if !(skip_mask || skip_disp) {
            break;
        }
        p += 1;                                                                  // c:1488 p++
    }
    p                                                                            // c:1490 return p
}

/// Port of `comp_list()` from `Src/Zle/compresult.c:1467`.
/// ```c
/// void
/// comp_list(char *v)
/// {
///     zsfree(complist);
///     complist = v;
///     onlyexpl = (v ? ((strstr(v, "expl") ? 1 : 0) |
///                      (strstr(v, "messages") ? 2 : 0)) : 0);
/// }
/// ```
/// Set the `complist` global and update `onlyexpl` per the
/// substring scan. Called from `bin_compset` to honour
/// `compstate[list]`.
pub fn comp_list(v: Option<&str>) {                                              // c:1467
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::ONLYEXPL;

    // c:1470-1471 — `zsfree(complist); complist = v`.
    let complist = crate::ported::zle::complete::COMPLIST
        .get_or_init(|| Mutex::new(String::new()));
    {
        let mut g = complist.lock().unwrap();
        g.clear();
        if let Some(s) = v {
            g.push_str(s);
        }
    }

    // c:1473-1474 — `onlyexpl = (v ? ((strstr(v,"expl")?1:0) |
    //                                 (strstr(v,"messages")?2:0)) : 0)`.
    let val = match v {
        None => 0,
        Some(s) => {
            (if s.contains("expl")     { 1 } else { 0 })
          | (if s.contains("messages") { 2 } else { 0 })
        }
    };
    ONLYEXPL.store(val, Ordering::SeqCst);
}

/// Port of `comp_mod()` from `Src/Zle/compresult.c:1363`.
/// ```c
/// static int
/// comp_mod(int v, int m)
/// {
///     if (v >= 0)
///         v--;
///     if (v >= 0)
///         return v % m;
///     else {
///         while (v < 0)
///             v += m;
///         return v;
///     }
/// }
/// ```
/// Modular arithmetic helper: subtract one when `v >= 0`, then
/// take `v % m`; for negative `v` (after the decrement), wrap by
/// repeated addition until non-negative. Used to map menu-cycle
/// indices to match-array offsets (where `0` means "no match" and
/// `1..N` are the real matches, so the table is 1-indexed).
pub fn comp_mod(mut v: i32, m: i32) -> i32 {                                     // c:1363
    if v >= 0 {                                                                  // c:1366
        v -= 1;                                                                  // c:1367
    }
    if v >= 0 {                                                                  // c:1368
        v % m                                                                    // c:1369
    } else {                                                                     // c:1370
        while v < 0 {                                                            // c:1371
            v += m;                                                              // c:1372
        }
        v                                                                        // c:1373
    }
}

/// Port of `asklist()` from `Src/Zle/compresult.c:1925`.
pub fn asklist() -> i32 {                                                        // c:1925
    // C body c:1927-1976 — "Do you wish to see all N possibilities?"
    //                      prompt: trashzle, showinglist, listdat,
    //                      zterm_lines/zterm_columns clamping,
    //                      getzlequery for y/n. Without a live tty
    //                      reader the safe default is "yes" (return 0)
    //                      so completion proceeds. With listmaxlines
    //                      = 0 (default) C bypasses the prompt entirely.
    0
}

/// Port of `ztat()` from `Src/Zle/compresult.c:869`.
/// `stat()` wrapper that follows symlinks unless `ls` is non-zero.
/// Returns `Option<Metadata>` mirroring C's `0`/`-1` return where
/// the metadata is filled into the supplied `struct stat *buf`.
pub fn ztat(path: &str, follow_symlink: bool) -> Option<std::fs::Metadata> {     // c:869
    if follow_symlink {                                                          // c:869 if (ls)
        // c:869 — `lstat(nam, buf)`. Don't follow symlinks.
        std::fs::symlink_metadata(path).ok()
    } else {                                                                     // c:869 else
        // c:869 — `stat(nam, buf)`. Follow symlinks.
        std::fs::metadata(path).ok()
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

    #[test]
    fn comp_mod_positive() {
        // c:1366-1369 — positive: decrement then % m.
        assert_eq!(comp_mod(1, 5), 0);   // (1-1) % 5 = 0
        assert_eq!(comp_mod(3, 5), 2);   // (3-1) % 5 = 2
        assert_eq!(comp_mod(5, 5), 4);   // (5-1) % 5 = 4
        assert_eq!(comp_mod(6, 5), 0);   // (6-1) % 5 = 0
    }

    #[test]
    fn comp_mod_zero_branches_negative() {
        // c:1366 — `if (v >= 0) v--;` so 0 → -1 → falls into else.
        // c:1370-1373 — wrap by adding m until non-negative.
        assert_eq!(comp_mod(0, 5), 4);   // 0→-1→+5=4
        assert_eq!(comp_mod(-1, 5), 4);  // -1+5=4
        assert_eq!(comp_mod(-5, 5), 0);  // -5+5=0
        assert_eq!(comp_mod(-6, 5), 4);  // -6+5=-1+5=4
    }

    #[test]
    fn comp_list_sets_onlyexpl() {
        use std::sync::atomic::Ordering;
        use crate::ported::zle::compcore::ONLYEXPL;
        // c:1473 — `(strstr(v,"expl")?1:0) | (strstr(v,"messages")?2:0)`.
        comp_list(Some("expl"));
        assert_eq!(ONLYEXPL.load(Ordering::SeqCst), 1);
        comp_list(Some("messages"));
        assert_eq!(ONLYEXPL.load(Ordering::SeqCst), 2);
        comp_list(Some("expl messages"));
        assert_eq!(ONLYEXPL.load(Ordering::SeqCst), 3);
        comp_list(Some("nothing"));
        assert_eq!(ONLYEXPL.load(Ordering::SeqCst), 0);
        comp_list(None);
        assert_eq!(ONLYEXPL.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn skipnolist_skips_hide_and_nolist() {
        use crate::ported::zle::comp_h::{Cmatch, CMF_HIDE, CMF_NOLIST};
        let mut a = Cmatch::default(); a.flags = CMF_NOLIST;
        let mut b = Cmatch::default(); b.flags = CMF_HIDE;
        let c = Cmatch::default();      // listable
        let v = vec![a, b, c];
        // c:1483 — mask = NOLIST|MULT|HIDE. First two skipped, third kept.
        assert_eq!(skipnolist(&v, 0), 2);
    }

    #[test]
    fn skipnolist_showall_keeps_nolist() {
        use crate::ported::zle::comp_h::{Cmatch, CMF_NOLIST};
        let mut a = Cmatch::default(); a.flags = CMF_NOLIST;
        let v = vec![a];
        // c:1483 — showall=1 drops NOLIST|MULT from mask, only HIDE filters.
        assert_eq!(skipnolist(&v, 1), 0);
    }

    #[test]
    fn skipnolist_skips_disp_displine() {
        use crate::ported::zle::comp_h::{Cmatch, CMF_DISPLINE};
        let mut a = Cmatch::default();
        a.disp = Some("display".into());
        a.flags = CMF_DISPLINE;
        let b = Cmatch::default();
        let v = vec![a, b];
        // c:1486-1487 — disp + (DISPLINE|HIDE) → skip.
        assert_eq!(skipnolist(&v, 0), 1);
    }
}

// =====================================================================
// Listing/menu helpers — the bodies depend on the full Cmgroup/Cmatch
// linked-list machine + listing arena + zle_refresh draw primitives.
// Until those land, these return empty/zero so callers don't blow up
// when no matches are available.
// =====================================================================

/// Port of `bld_all_str()` from `Src/Zle/compresult.c:2187`.
pub fn bld_all_str() -> String {                                             // c:2187
    // C body c:2189-2280 — walks Cmgroup list collecting every match
    //                     into a single quoted+space-joined string for
    //                     `do_allmatches`. Without Cmatch list: empty.
    String::new()
}

/// Port of `calclist()` from `Src/Zle/compresult.c:1495`.
pub fn calclist(_showall: i32) -> i32 {                                      // c:1495
    // C body c:1497-1976 — computes per-match column widths and totals
    //                      for the listing, populates listdat fields
    //                      (cols, lines, hidden, widthrest, etc.).
    //                      Without mgroup walk: 0 (no list).
    0
}

/// Port of `do_ambig_menu()` from `Src/Zle/compresult.c:1381`.
pub fn do_ambig_menu() -> i32 {                                              // c:1381
    // C body c:1383-1493 — menu-completion entry for the ambiguous-
    //                      matches case: sets amenu, lastambig, primes
    //                      domenuselect for next call. Substrate
    //                      (menu state) deferred; 0.
    0
}

/// Port of `ilistmatches()` from `Src/Zle/compresult.c:2284`.
pub fn ilistmatches() -> i32 {                                               // c:2284
    // C body c:2286-2302 — hook callback for `listmatches()` — calls
    //                      printlist with iprintm. Substrate deferred; 0.
    0
}

/// Port of `invalidate_list()` from `Src/Zle/compresult.c:2334`.
pub fn invalidate_list() -> i32 {                                            // c:2334
    // C body c:2336-2370 — discards cached match list: sets validlist=0,
    //                      showinglist=0, calls freematches. Substrate
    //                      deferred; 0.
    0
}

/// Port of `iprintm()` from `Src/Zle/compresult.c:2241`.
pub fn iprintm() -> i32 {                                                    // c:2241
    // C body c:2243-2282 — CLPrintFunc for the standard listing: prints
    //                      one match cell with proper column padding +
    //                      group separator. Substrate deferred; 0.
    0
}

/// Port of `list_matches()` from `Src/Zle/compresult.c:2304`.
pub fn list_matches() -> i32 {                                               // c:2304
    // C body c:2306-2332 — hook callback wrapper around printlist via
    //                      Hookdef registration. Substrate deferred; 0.
    0
}

/// Port of `printlist()` from `Src/Zle/compresult.c:1978`.
pub fn printlist() -> i32 {                                                  // c:1978
    // C body c:1980-2185 — workhorse listing renderer: emits each
    //                      match group through ListPrintFunc, handles
    //                      asklist prompt, scroll-paging, group sep.
    //                      Substrate deferred; 0 (nothing to print).
    0
}
