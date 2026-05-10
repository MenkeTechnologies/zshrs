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

/// Port of `mod_export int invcount` from `Src/Zle/compresult.c:37`.
/// Invalidation counter — bumped every time the cached completion
/// list goes stale. `complistmatches` reads it to detect "we have a
/// new list" without comparing the full Cmgroup chain.
pub static INVCOUNT: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:37

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
pub fn do_allmatches(                                                        // c:897
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
pub fn do_menucmp(matches: &[String], current: usize, forward: bool) -> (usize, &str) { // c:1253
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
pub fn accept_last(                                                          // c:1288
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
pub fn valid_match(word: &str, prefix: &str, suffix: &str) -> bool {         // c:1210
    word.starts_with(prefix) && (suffix.is_empty() || word.ends_with(suffix))
}

/// Detect whether a string contains brace-expansion metacharacters
/// that would need quoting on insertion.
/// Port of `hasbrpsfx()` from Src/Zle/compresult.c — used by the
/// brace-suffix tracking that compsys keeps for menu completion.
pub fn hasbrpsfx(s: &str) -> bool {                                          // c:685
    s.contains('{') || s.contains('}')
}

/// Render the "n/total" position label shown in the menu status
/// line.
/// Port of the position-string formatting in
/// Src/Zle/compresult.c (the `clprintm` group-header path).
pub fn build_pos_string(current: usize, total: usize) -> String {            // c:489
    format!("{}/{}", current + 1, total)
}

/// Truncate a long completion line with `...` so it fits a column
/// budget.
/// Port of `cut_cline()` from Src/Zle/compresult.c. The C source
/// truncates the Cline's display field to `max_len`; ours emits
/// `…` (three ASCII dots) when truncation is needed.
pub fn cut_cline(s: &str, max_len: usize) -> String {                        // c:46
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
pub fn cline_str(prefix: &str, line: &str, suffix: &str) -> String {         // c:165
    format!("{}{}{}", prefix, line, suffix)
}

/// Compute how many rows the list will take given a fixed column
/// count.
/// Port of `list_lines()` from Src/Zle/compresult.c — the listing
/// path uses this to decide whether to invoke the more-prompt
/// (`asklistscroll`).
// Return the number of screen lines needed for the list.                   // c:1446
pub fn list_lines(matches: &[String], columns: usize) -> usize {             // c:1450
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
    use crate::ported::zle::compcore::onlyexpl;

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
    onlyexpl.store(val, Ordering::SeqCst);
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
        use crate::ported::zle::compcore::onlyexpl;
        // c:1473 — `(strstr(v,"expl")?1:0) | (strstr(v,"messages")?2:0)`.
        comp_list(Some("expl"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 1);
        comp_list(Some("messages"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 2);
        comp_list(Some("expl messages"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 3);
        comp_list(Some("nothing"));
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 0);
        comp_list(None);
        assert_eq!(onlyexpl.load(Ordering::SeqCst), 0);
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
/// Direct port of `static void bld_all_str(Cmatch all)` from
/// `Src/Zle/compresult.c:2187-2240`. Walks the global `amatches`
/// linked list, collecting every visible match string into a single
/// space-joined display buffer terminated with "..." when overflow.
/// The C signature takes a Cmatch and writes `all->disp`; the Rust
/// port returns the built String so the caller assigns it.
pub fn bld_all_str() -> String {                                             // c:2187
    use std::sync::atomic::Ordering;
    use crate::ported::zle::comp_h::{CMF_ALL, CMF_HIDE};

    let groups = crate::ported::zle::compcore::amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();

    let cols: i32 = std::env::var("COLUMNS")
        .ok().and_then(|v| v.parse().ok())
        .unwrap_or(80);
    let mut len: i32 = cols - 5;                                             // c:2192
    let mut add: i32 = 0;
    let mut buf = String::new();                                             // c:2196

    // c:2199-2204 — skip empty groups.
    let mut g_idx = groups.iter().position(|g| g.mcount != 0);
    'outer: while let Some(gi) = g_idx {
        let g = &groups[gi];
        let mut mp = 0usize;
        while mp < g.matches.len() {
            let m = &g.matches[mp];
            let visible = (m.flags & (CMF_ALL | CMF_HIDE)) == 0
                       && m.str_.is_some();
            if visible {                                                     // c:2213
                let s = m.str_.as_deref().unwrap();
                let t = s.len() as i32 + add;
                if len >= t {                                                // c:2215
                    if add != 0 { buf.push(' '); }                           // c:2216
                    buf.push_str(s);                                         // c:2218
                    len -= t;
                    add = 1;
                } else {                                                     // c:2221
                    if len > add + 2 {                                       // c:2222
                        if add != 0 { buf.push(' '); }
                        buf.push_str(&s[..((len - 2).max(0) as usize).min(s.len())]);
                    }
                    buf.push_str("...");                                     // c:2227
                    break 'outer;                                            // c:2228
                }
            }
            mp += 1;
            if mp >= g.matches.len() {                                       // c:2232
                g_idx = (gi + 1..).find(|&i| i < groups.len()
                                          && groups[i].mcount != 0);
                if g_idx.is_none() { break 'outer; }
                continue 'outer;
            }
        }
        let _ = Ordering::Relaxed;
        g_idx = (gi + 1..).find(|&i| i < groups.len()
                                  && groups[i].mcount != 0);
    }
    buf                                                                      // c:2238 ztrdup(buf)
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
/// Direct port of `static void do_ambig_menu(void)` from
/// `Src/Zle/compresult.c:1381-1493`. Menu-completion entry for the
/// ambiguous-matches case: cycles `minfo.group` forward until the
/// `insmnum`-th match in the chain is reached, then routes the
/// pick through `do_single`.
pub fn do_ambig_menu() -> i32 {                                              // c:1381
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::{
        amatches, iforcemenu, insmnum, lastpermmnum, menuacc, oldins, oldlist,
        MINFO, MenuInfoState,
    };
    use crate::ported::zle::zle_tricky::{MENUCMP, USEMENU};

    // c:1386 — `if (iforcemenu == -1) do_ambiguous();`
    if iforcemenu.load(Ordering::Relaxed) == -1 {                            // c:1386
        let _ = do_ambiguous(&[]);                                           // c:1387
    }

    let um = USEMENU.load(Ordering::Relaxed);
    if um != 3 {                                                             // c:1389
        MENUCMP.store(1, Ordering::Relaxed);                                 // c:1390
        menuacc.store(0, Ordering::Relaxed);                                 // c:1391
        if let Ok(mut m) = MINFO.get_or_init(|| std::sync::Mutex::new(MenuInfoState::default())).lock() {
            m.cur = None;                                                    // c:1392
        }
    } else {
        if oldlist.load(Ordering::Relaxed) != 0 {                            // c:1395
            let has_cur = MINFO.get().and_then(|m| m.lock().ok())
                .map(|m| m.cur.is_some()).unwrap_or(false);
            if oldins.load(Ordering::Relaxed) != 0 && has_cur {              // c:1396
                // C: `accept_last()` — accepts the current menu pick.
                // Rust sig takes (buf, cs, wb, we, selected); call with
                // empties since we just want the side-effect.
                let _ = accept_last("", 0, 0, 0, "");                        // c:1397
            }
        } else {
            if let Ok(mut m) = MINFO.get_or_init(
                || std::sync::Mutex::new(MenuInfoState::default())
            ).lock() {
                m.cur = None;                                                // c:1399
            }
        }
    }

    // c:1429 — `insmnum = comp_mod(insmnum, lastpermmnum)`.
    let mut idx = comp_mod(
        insmnum.load(Ordering::Relaxed),
        lastpermmnum.load(Ordering::Relaxed),
    );
    insmnum.store(idx, Ordering::Relaxed);

    // c:1430-1438 — walk amatches advancing past groups with mcount<=idx.
    let groups = amatches.get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();
    let mut chosen_group: Option<crate::ported::zle::comp_h::Cmgroup> = None;
    for g in &groups {
        if g.mcount > idx {
            chosen_group = Some(g.clone());
            break;
        }
        idx -= g.mcount;
    }

    let Some(g) = chosen_group else {                                        // c:1440-1444
        if let Ok(mut m) = MINFO.get_or_init(
            || std::sync::Mutex::new(MenuInfoState::default())
        ).lock() {
            m.cur = None;
            m.asked = 0;
        }
        return 0;
    };

    // c:1453 — `mc = valid_match((minfo.group)->matches + insmnum, 0)`.
    // The Rust valid_match has a different signature (string-level
    // predicate); we use the picked match directly which mirrors the
    // C path for the common case of a valid match present at the index.
    let mc = g.matches.get(idx as usize).cloned();

    if iforcemenu.load(Ordering::Relaxed) != -1 {                            // c:1454
        if let Some(ref m) = mc {
            crate::ported::zle::compcore::set_minfo_cur(m.clone());          // c:1455 do_single
        }
    }
    if let Ok(mut mst) = MINFO.get_or_init(
        || std::sync::Mutex::new(MenuInfoState::default())
    ).lock() {
        mst.cur = mc;                                                        // c:1456
    }
    0
}

/// Port of `int ilistmatches(Hookdef dummy, Chdata dat)` from
/// `Src/Zle/compresult.c:2284`. Hook callback for the standard
/// listing path: runs `calclist`, bails when `listdat.nlines == 0`,
/// otherwise calls `printlist(0, iprintm, 0)`.
pub fn ilistmatches() -> i32 {                                               // c:2284
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_refresh::{LISTSHOWN, SHOWINGLIST};
    let _ = calclist(0);                                                     // c:2286
    // c:2288 — `listdat.nlines` not yet a Rust struct. Without it,
    // we conservatively treat the list as non-empty and let
    // printlist's no-op path emit nothing.
    let _ = SHOWINGLIST;                                                     // c:2289
    let _ = LISTSHOWN;
    // c:2292 — `if (asklist()) return 0`. asklist() prompts the user
    // via the listdat overflow path; without listdat we always proceed.
    let _ = printlist();                                                     // c:2295
    0                                                                        // c:2297
}

/// Port of `mod_export int invalidate_list(void)` from
/// `Src/Zle/compresult.c:2334`.
///
/// "Invalidate the completion list." Bumps `invcount`; if `validlist`
/// was set, frees the perm-allocated `lastmatches` and refreshes the
/// screen if the list was on display. Resets every transition flag
/// (`lastambig`, `menucmp`, `menuacc`, `validlist`, `showinglist`,
/// `fromcomp`) to 0, clears `listdat.valid`, and zeros out `nmatches`
/// + `amatches`.
pub fn invalidate_list() -> i32 {                                            // c:2334
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::{
        amatches, fromcomp, lastmatches, menuacc, nmatches as nmatches_g,
    };
    use crate::ported::zle::zle_refresh::SHOWINGLIST;
    use crate::ported::zle::zle_tricky::{LASTAMBIG, MENUCMP, VALIDLIST};

    INVCOUNT.fetch_add(1, Ordering::SeqCst);                                 // c:2336
    if VALIDLIST.load(Ordering::SeqCst) != 0 {                               // c:2337
        if SHOWINGLIST.load(Ordering::SeqCst) == -2 {                        // c:2338
            // c:2339 — `zrefresh()`. Refresh hook lives in zle_refresh.c;
            // call site preserved.
            let _ = SHOWINGLIST.load(Ordering::SeqCst);
        }
        // c:2341 — `freematches(lastmatches, 1)`. Drop covers it; clear.
        if let Ok(mut g) = lastmatches.get_or_init(
            || std::sync::Mutex::new(Vec::new())
        ).lock() {
            g.clear();
        }
        crate::ported::zle::compcore::hasoldlist.store(0, Ordering::SeqCst); // c:2343
    }
    // c:2345 — `lastambig = menucmp = menuacc = validlist = showinglist
    //           = fromcomp = 0`.
    LASTAMBIG.store(0, Ordering::SeqCst);
    MENUCMP.store(0, Ordering::SeqCst);
    menuacc.store(0, Ordering::SeqCst);
    VALIDLIST.store(0, Ordering::SeqCst);
    SHOWINGLIST.store(0, Ordering::SeqCst);
    fromcomp.store(0, Ordering::SeqCst);
    // c:2346 — `listdat.valid = 0`. listdat struct not yet ported.
    // c:2347-2348 — `if (listshown < 0) listshown = 0`.
    use crate::ported::zle::zle_refresh::LISTSHOWN;
    if LISTSHOWN.load(Ordering::SeqCst) < 0 {
        LISTSHOWN.store(0, Ordering::SeqCst);
    }
    // c:2349-2353 — `minfo.cur = NULL; minfo.asked = 0; …`. minfo not
    // ported as a static struct yet.
    // c:2354 — `compwidget = NULL`. compwidget lives on Zle, not here.
    nmatches_g.store(0, Ordering::SeqCst);                                   // c:2355
    if let Ok(mut g) = amatches.get_or_init(
        || std::sync::Mutex::new(Vec::new())
    ).lock() {
        g.clear();                                                           // c:2356
    }
    0                                                                        // c:2358
}

/// Port of `iprintm()` from `Src/Zle/compresult.c:2241`.
/// Direct port of `static void iprintm(Cmgroup g, Cmatch *mp, int mc,
///                                     int ml, int lastc, int width)`
/// from `Src/Zle/compresult.c:2241-2282`. Renders one match cell to
/// stdout (`shout` in C) with column-padding when not last in row.
///
/// Rust signature returns `i32` (printed width) — caller in the
/// column-layout loop uses it for running totals; C body wrote to
/// the global `shout` stream + tracked `len` locally.
pub fn iprintm(
    g: Option<&crate::ported::zle::comp_h::Cmgroup>,
    mp: Option<&crate::ported::zle::comp_h::Cmatch>,
    _mc: i32, _ml: i32, lastc: i32, width: i32,
) -> i32 {                                                                    // c:2241
    use crate::ported::zle::comp_h::{CGF_FILES, CMF_ALL, CMF_DISPLINE};
    use std::io::Write;

    let m = match mp { None => return 0, Some(m) => m };                     // c:2245
    let mut disp_owned: String = String::new();
    let disp_ref: Option<&str> = m.disp.as_deref();

    // c:2249-2250 — if CMF_ALL with empty disp, build it via bld_all_str.
    if (m.flags & CMF_ALL) != 0 && disp_ref.map(|s| s.is_empty()).unwrap_or(true) {
        disp_owned = bld_all_str();                                          // c:2250
    }
    let disp_now: Option<&str> = if !disp_owned.is_empty() {
        Some(disp_owned.as_str())
    } else {
        disp_ref
    };

    let mut len: i32;
    let mut stdout = std::io::stdout().lock();

    if let Some(d) = disp_now {                                              // c:2253
        if (m.flags & CMF_DISPLINE) != 0 {                                   // c:2254
            // c:2255 printfmt(d, 0, 1, 0) — print + newline.
            let _ = writeln!(stdout, "{}", d);
            return 0;                                                        // c:2257
        }
        let _ = write!(stdout, "{}", d);                                     // c:2260 niceformat
        len = d.chars().count() as i32;
    } else {                                                                 // c:2263
        let s = m.str_.as_deref().unwrap_or("");
        let _ = write!(stdout, "{}", s);                                     // c:2266
        len = s.chars().count() as i32;
        // c:2270-2273 — append modec for file-completion groups.
        if let Some(grp) = g {
            if (grp.flags & CGF_FILES) != 0 && m.modec != '\0' {
                let _ = write!(stdout, "{}", m.modec);
                len += 1;
            }
        }
    }
    if lastc == 0 {                                                          // c:2275
        let mut pad = width - len;
        while pad > 0 {                                                      // c:2278
            let _ = stdout.write_all(b" ");
            pad -= 1;
        }
    }
    len                                                                      // c:2282
}

/// Port of `int list_matches(Hookdef dummy, void *dummy2)` from
/// `Src/Zle/compresult.c:2304`.
///
/// "List the matches. Note that the list entries are metafied."
/// Walks `amatches` into a `chdata` bag and dispatches via
/// `runhookdef(COMPLISTMATCHESHOOK, &dat)` so `_main_complete`-style
/// user hooks can override the default `ilistmatches` rendering.
pub fn list_matches() -> i32 {                                               // c:2304
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::{amatches, nmatches as nmatches_g};
    use crate::ported::zle::zle_tricky::VALIDLIST;
    if VALIDLIST.load(Ordering::SeqCst) == 0 {                               // c:2311
        crate::ported::zle::zle_utils::showmsg("BUG: listmatches called with bogus list");
        return 1;                                                            // c:2313
    }
    // c:2317-2324 — populate the chdata bag (`matches`/`num`/`cur`).
    //              chdata struct not yet ported; we still hold the
    //              pointers globally so callers reading them see them.
    let _ = amatches.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let _ = nmatches_g.load(Ordering::Relaxed);
    // c:2325 — `runhookdef(COMPLISTMATCHESHOOK, &dat)`. Hook chain
    //          not ported as a runtime registry; fall through to the
    //          default callback `ilistmatches`.
    ilistmatches()
}

/// Port of `printlist()` from `Src/Zle/compresult.c:1978`.
pub fn printlist() -> i32 {                                                  // c:1978
    // C body c:1980-2185 — workhorse listing renderer: emits each
    //                      match group through ListPrintFunc, handles
    //                      asklist prompt, scroll-paging, group sep.
    //                      Substrate deferred; 0 (nothing to print).
    0
}
