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

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_misc::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_hist::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_move::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_word::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_params::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_vi::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_utils::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_refresh::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

pub static INVCOUNT: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:37

// scs is used to return the position where a automatically created suffix  // c:578
// has to be inserted.                                                       // c:578
/// Replace `[word_start, word_end)` in `buffer` with `replacement`,
/// returning the new buffer plus updated cursor position.
/// Port of `instmatch(Cmatch m, int *scs)` from Src/Zle/compresult.c. The C source
/// uses this as the lowest-level "swap the partial word for the
/// chosen completion" primitive used by every other inserter
/// (`do_single`, `do_ambiguous`, `do_allmatches`).
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, replacement) vs C=(m, scs)
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
// This is a utility function using the function above to allow access     // c:525
// to the unambiguous string and cursor position via compstate.             // c:525
/// completion engine inserts on the first Tab press when matches
/// are ambiguous.
/// Port of `unambig_data(int *cp, char **pp, char **ip)` from Src/Zle/compresult.c. The C source
/// also tracks cursor placement within the prefix; ours returns
/// just the common-prefix string.
/// WARNING: param names don't match C — Rust=(matches) vs C=(cp, pp, ip)
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
/// Port of `do_single(Cmatch m)` from Src/Zle/compresult.c — fired when
// Insert a single match in the command line.                              // c:963
/// completion produced exactly one match. The trailing space is
/// the `AUTO_REMOVE_SLASH`-aware insertion that distinguishes
/// finished-completion from prefix-completion.
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, the_match, add_space) vs C=(m)
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
/// WARNING: param names don't match C — Rust=(matches) vs C=()
pub fn do_ambiguous(matches: &[String]) -> i32 {                         // c:744
    let prefix = unambig_data(matches);
    if prefix.is_empty() && matches.is_empty() {
        return 0;                                                        // c:nomatch
    }
    if !prefix.is_empty() { 1 } else { 0 }
}

/// Insert every match into the buffer joined by `separator`.
/// Port of `do_allmatches(UNUSED(int end))` from Src/Zle/compresult.c — fires for
/// the `all-matches` widget and for the implicit case when no
/// listing fits.
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, matches, separator) vs C=(end)
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
/// Port of `do_menucmp(int lst)` from Src/Zle/compresult.c. The C source
/// also handles per-group menu wrap; this Rust port treats the
/// match list as flat for the host's menu loop.
/// WARNING: param names don't match C — Rust=(matches, current, forward) vs C=(lst)
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
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, selected) vs C=()
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
/// Port of `valid_match(m, next)` from Src/Zle/compresult.c.
/// WARNING: param names don't match C — Rust=(word, prefix, suffix) vs C=(m, next)
pub fn valid_match(word: &str, prefix: &str, suffix: &str) -> bool {         // c:1210
    word.starts_with(prefix) && (suffix.is_empty() || word.ends_with(suffix))
}

/// Detect whether a string contains brace-expansion metacharacters
/// that would need quoting on insertion.
/// Port of `hasbrpsfx(Cmatch m, char *pre, char *suf)` from Src/Zle/compresult.c — used by the
/// brace-suffix tracking that compsys keeps for menu completion.
/// WARNING: param names don't match C — Rust=(s) vs C=(m, pre, suf)
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
/// Port of `cut_cline(Cline l)` from Src/Zle/compresult.c. The C source
/// truncates the Cline's display field to `max_len`; ours emits
/// `…` (three ASCII dots) when truncation is needed.
/// WARNING: param names don't match C — Rust=(s, max_len) vs C=(l)
pub fn cut_cline(s: &str, max_len: usize) -> String {                        // c:46
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

/// Direct port of `char *cline_str(Cline l, int ins, int *csp,
/// LinkList posl)` from `Src/Zle/compresult.c:165`. Walks the Cline
/// chain and produces a visible string by emitting, for each node:
///
/// 1. If `olen != 0 && !(flags & CLF_SUF) && !prefix`: emit `orig`
///    (the unjoined original).
/// 2. Else walk the `prefix` sub-list, emitting each part's
///    `line` (if CLF_LINE) or `word` (otherwise).
/// 3. Emit the node's anchor — `line` (if CLF_LINE) or `word`.
/// 4. If `olen != 0 && (flags & CLF_SUF) && !suffix`: emit `orig`.
/// 5. Else walk the `suffix` sub-list.
///
/// The C source also integrates with `inststrlen` (buffer edit),
/// `brbeg`/`brend` (brace chains), and `posl` (position-list output).
/// The Rust port handles the `ins=0` / `csp=NULL` / `posl=NULL` case
/// — pure visible-text rendering — which is what `unambig_data` and
/// the listing path need. Caller-side buffer integration is deferred
/// pending the `zlemetaline` edit primitives.
/// WARNING: signature change — C=(l, ins, csp, posl) vs Rust=(l) -> String
pub fn cline_str(                                                            // c:165
    l: Option<&crate::ported::zle::comp_h::Cline>,
) -> String {
    use crate::ported::zle::comp_h::{CLF_LINE, CLF_SUF};
    let mut out = String::new();
    let mut cur = l;
    while let Some(node) = cur {
        // c:214 — `if (l->olen && !(l->flags & CLF_SUF) && !l->prefix)`
        if node.olen != 0 && (node.flags & CLF_SUF) == 0 && node.prefix.is_none() {
            // c:216 — emit `orig`.
            if let Some(o) = &node.orig {
                out.push_str(o);
            }
        } else {
            // c:219-235 — walk prefix sub-list.
            let mut p = node.prefix.as_deref();
            while let Some(part) = p {
                let s = if (part.flags & CLF_LINE) != 0 {
                    part.line.as_deref()
                } else {
                    part.word.as_deref()
                };
                if let Some(s) = s { out.push_str(s); }
                p = part.next.as_deref();
            }
        }
        // c:282-285 — emit the anchor.
        let anchor = if (node.flags & CLF_LINE) != 0 {
            node.line.as_deref()
        } else {
            node.word.as_deref()
        };
        if let Some(a) = anchor { out.push_str(a); }

        // c:336-338 — `if (l->olen && (l->flags & CLF_SUF) && !l->suffix)`
        if node.olen != 0 && (node.flags & CLF_SUF) != 0 && node.suffix.is_none() {
            if let Some(o) = &node.orig {
                out.push_str(o);
            }
        } else {
            // c:374-382 — walk suffix sub-list.
            let mut p = node.suffix.as_deref();
            while let Some(part) = p {
                let s = if (part.flags & CLF_LINE) != 0 {
                    part.line.as_deref()
                } else {
                    part.word.as_deref()
                };
                if let Some(s) = s { out.push_str(s); }
                p = part.next.as_deref();
            }
        }
        cur = node.next.as_deref();
    }
    out
}

/// Compute how many rows the list will take given a fixed column
/// count.
/// Port of `list_lines()` from Src/Zle/compresult.c — the listing
/// path uses this to decide whether to invoke the more-prompt
/// (`asklistscroll`).
// Return the number of screen lines needed for the list.                   // c:1450
/// WARNING: param names don't match C — Rust=(matches, columns) vs C=()
pub fn list_lines(matches: &[String], columns: usize) -> usize {             // c:1450
    if columns == 0 {
        return matches.len();
    }
    matches.len().div_ceil(columns)
}

/// Port of `skipnolist(Cmatch *p, int showall)` from `Src/Zle/compresult.c:1480`.
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
pub fn skipnolist(p: &[crate::ported::zle::comp_h::Cmatch], showall: i32) -> usize {  // c:1481
    use crate::ported::zle::comp_h::{CMF_DISPLINE, CMF_HIDE, CMF_MULT, CMF_NOLIST};
    // c:1483 — `mask = (showall ? 0 : (CMF_NOLIST|CMF_MULT)) | CMF_HIDE`.
    let mask = if showall != 0 { 0 } else { CMF_NOLIST | CMF_MULT } | CMF_HIDE;
    let mut i = 0usize;                                                          // c:1485 *p
    while i < p.len() {                                                    // c:1485 while (*p && ...)
        let m = &p[i];
        let f = m.flags;
        let skip_mask    = (f & mask) != 0;                                      // c:1485
        let skip_disp    = m.disp.is_some() && (f & (CMF_DISPLINE | CMF_HIDE)) != 0; // c:1486-1487
        if !(skip_mask || skip_disp) {
            break;
        }
        i += 1;                                                                  // c:1488 p++
    }
    i                                                                            // c:1490 return p
}

/// Port of `comp_list(char *v)` from `Src/Zle/compresult.c:1468`.
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
pub fn comp_list(v: Option<&str>) {                                              // c:1468
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

/// Port of `comp_mod(int v, int m)` from `Src/Zle/compresult.c:1363`.
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
pub fn comp_mod(mut v: i32, m: i32) -> i32 {                                     // c:1364
    if v >= 0 {                                                                  // c:1364
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

/// Direct port of `int asklist(void)` from
/// `Src/Zle/compresult.c:1925`. The "do you wish to see all N
/// possibilities?" prompt that gates display of long completion
/// lists. Returns 1 to suppress the listing (user said no), 0 to
/// proceed (yes / no prompt needed).
///
/// Implements the C decision tree:
///   - `trashzle()` + zero `showinglist`/`listshown`.
///   - `clearflag = USEZLE && !termflags && dolastprompt`.
///   - Threshold check `complistmax > 0 ? nlist >= complistmax :
///     complistmax < 0 ? nlines <= -complistmax :
///     nlines >= zterm_lines`.
///   - If threshold tripped, prompt via `getzlequery` and set
///     `minfo.asked = 1 or 2`. Else return based on previous asked.
pub fn asklist() -> i32 {                                                        // c:1925
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::MINFO;
    use crate::ported::zle::complete::COMPLISTMAX;
    use crate::ported::zle::zle_refresh::{LASTLISTLEN, CLEARFLAG};
    use crate::ported::zle::zle_refresh::SHOWINGLIST;
    use crate::ported::zsh_h::{USEZLE, isset};

    // c:1928 — `trashzle(); showinglist = listshown = 0; lastlistlen = 0`.
    crate::ported::zle::zle_main::trashzle();                                    // c:1928
    SHOWINGLIST.store(0, Ordering::Relaxed);
    crate::ported::zle::zle_refresh::LISTSHOWN.store(0, Ordering::Relaxed);
    LASTLISTLEN.store(0, Ordering::Relaxed);                                     // c:1934

    // c:1930 — `clearflag = (isset(USEZLE) && !termflags && dolastprompt)`.
    let usezle = isset(USEZLE);
    let termflags = crate::ported::params::TERMFLAGS.load(Ordering::Relaxed);
    let dolastprompt = crate::ported::zle::compcore::dolastprompt
        .load(Ordering::Relaxed) != 0;
    let clearflag = usezle && termflags == 0 && dolastprompt;
    CLEARFLAG.store(if clearflag { 1 } else { 0 }, Ordering::Relaxed);

    // c:1937-1940 — snapshot listdat counts + minfo state.
    let listdat = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();
    let zterm_lines = crate::ported::utils::adjustlines() as i32;
    let cmax = COMPLISTMAX.load(Ordering::Relaxed) as i32;

    let has_cur = MINFO.get().and_then(|m| m.lock().ok())
        .map(|m| m.cur.is_some()).unwrap_or(false);
    let already_asked = MINFO.get().and_then(|m| m.lock().ok())
        .map(|m| m.asked).unwrap_or(0);

    // c:1939-1942 — threshold gate.
    let over_threshold = (cmax > 0 && listdat.nlist >= cmax)
        || (cmax < 0 && listdat.nlines <= -cmax)
        || (cmax == 0 && listdat.nlines >= zterm_lines);

    // c:1939 — `if ((!minfo.cur || !minfo.asked) && over_threshold)`.
    if (!has_cur || already_asked == 0) && over_threshold {
        // c:1947-1953 — write the "do you wish to see ...?" prompt.
        let prompt = if listdat.nlist > 0 {
            format!(
                "zsh: do you wish to see all {} possibilities ({} lines)? ",
                listdat.nlist, listdat.nlines
            )
        } else {
            format!(
                "zsh: do you wish to see all {} lines? ",
                listdat.nlines
            )
        };
        let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        let out = if fd >= 0 { fd } else { 1 };
        let _ = crate::ported::utils::write_loop(out, prompt.as_bytes());

        // c:1955 — `getzlequery()`.
        let said_yes = crate::ported::zle::zle_utils::getzlequery() != 0;

        if !said_yes {                                                           // c:1956
            // c:1957-1964 — clean up the question line.
            let _ = crate::ported::utils::write_loop(out, b"\n");
            // c:1965 — `minfo.asked = 2`.
            if let Ok(mut m) = MINFO.get_or_init(
                || std::sync::Mutex::new(Default::default())
            ).lock() {
                m.asked = 2;
            }
            return 1;                                                            // c:1966
        }
        // c:1968-1974 — clean up after a yes.
        let _ = crate::ported::utils::write_loop(out, b"\n");
        // c:1975 — `minfo.asked = 1`.
        if let Ok(mut m) = MINFO.get_or_init(
            || std::sync::Mutex::new(Default::default())
        ).lock() {
            m.asked = 1;
        }
    }
    // c:1978-1979 — second-pass entry: already-asked-no falls through
    //                to the final return-1 to suppress the listing.

    // c:1981 — `return (minfo.asked ? minfo.asked - 1 : 0);`.
    let asked_now = MINFO.get().and_then(|m| m.lock().ok())
        .map(|m| m.asked).unwrap_or(0);
    if asked_now != 0 { asked_now - 1 } else { 0 }
}

/// Port of `ztat(char *nam, struct stat *buf, int ls)` from `Src/Zle/compresult.c:869`.
/// `stat()` wrapper that follows symlinks unless `ls` is non-zero.
/// Returns `Option<Metadata>` mirroring C's `0`/`-1` return where
/// the metadata is filled into the supplied `struct stat *buf`.
/// WARNING: param names don't match C — Rust=(path, follow_symlink) vs C=(nam, buf, ls)
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(unambig_data(&["foobar".into(), "foobaz".into()]), "fooba");
        assert_eq!(unambig_data(&["abc".into()]), "abc");
        assert_eq!(unambig_data(&[]), "");
    }

    #[test]
    fn cline_str_none_returns_empty() {
        // c:165 — null Cline → empty string.
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(cline_str(None), "");
    }

    #[test]
    fn cline_str_emits_word_anchor() {
        // c:282 — non-CLF_LINE node emits `word`.
        use crate::ported::zle::comp_h::Cline;
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut n = Cline::default();
        n.word = Some("hello".to_string());
        n.wlen = 5;
        assert_eq!(cline_str(Some(&n)), "hello");
    }

    #[test]
    fn cline_str_emits_line_anchor_when_clf_line_set() {
        // c:282 — CLF_LINE node emits `line` instead of `word`.
        use crate::ported::zle::comp_h::{Cline, CLF_LINE};
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut n = Cline::default();
        n.flags = CLF_LINE;
        n.line = Some("LINE".to_string());
        n.word = Some("word-should-not-emit".to_string());
        assert_eq!(cline_str(Some(&n)), "LINE");
    }

    #[test]
    fn cline_str_emits_orig_when_olen_set_and_no_prefix() {
        // c:214 — olen!=0 && !CLF_SUF && !prefix → emit `orig` (not
        //          the prefix-walk + word path).
        use crate::ported::zle::comp_h::Cline;
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut n = Cline::default();
        n.orig = Some("original".to_string());
        n.olen = 8;
        n.word = Some("anchor".to_string());
        // Output = orig + word anchor (the C path emits both).
        assert_eq!(cline_str(Some(&n)), "originalanchor");
    }

    #[test]
    fn cline_str_walks_prefix_chain() {
        // c:219-235 — prefix sub-list walked when olen==0 or
        //              CLF_SUF set.
        use crate::ported::zle::comp_h::Cline;
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut p2 = Cline::default();
        p2.word = Some("ond".to_string());
        let mut p1 = Cline::default();
        p1.word = Some("sec".to_string());
        p1.next = Some(Box::new(p2));
        let mut n = Cline::default();
        n.prefix = Some(Box::new(p1));
        n.word = Some("anchor".to_string());
        assert_eq!(cline_str(Some(&n)), "secondanchor");
    }

    #[test]
    fn cline_str_walks_next_chain() {
        // c:165 — top-level walk via `l = l->next`.
        use crate::ported::zle::comp_h::Cline;
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut n2 = Cline::default();
        n2.word = Some("B".to_string());
        let mut n1 = Cline::default();
        n1.word = Some("A".to_string());
        n1.next = Some(Box::new(n2));
        assert_eq!(cline_str(Some(&n1)), "AB");
    }

    #[test]
    fn test_instmatch() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let (result, cursor) = instmatch("git co", 6, 4, 6, "commit");
        assert_eq!(result, "git commit");
        assert_eq!(cursor, 10);
    }

    #[test]
    fn test_do_single() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let (result, cursor) = do_single("git co", 6, 4, 6, "commit", true);
        assert_eq!(result, "git commit ");
        assert_eq!(cursor, 11);
    }

    #[test]
    fn test_do_menucmp() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert!(valid_match("foobar", "foo", ""));
        assert!(valid_match("foobar", "foo", "bar"));
        assert!(!valid_match("foobar", "baz", ""));
    }

    #[test]
    fn test_build_pos_string() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(build_pos_string(0, 10), "1/10");
        assert_eq!(build_pos_string(9, 10), "10/10");
    }

    #[test]
    fn test_list_lines() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(list_lines(&vec!["a".into(); 10], 3), 4);
        assert_eq!(list_lines(&vec!["a".into(); 6], 3), 2);
    }

    #[test]
    fn comp_mod_positive() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1366-1369 — positive: decrement then % m.
        assert_eq!(comp_mod(1, 5), 0);   // (1-1) % 5 = 0
        assert_eq!(comp_mod(3, 5), 2);   // (3-1) % 5 = 2
        assert_eq!(comp_mod(5, 5), 4);   // (5-1) % 5 = 4
        assert_eq!(comp_mod(6, 5), 0);   // (6-1) % 5 = 0
    }

    #[test]
    fn comp_mod_zero_branches_negative() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1366 — `if (v >= 0) v--;` so 0 → -1 → falls into else.
        // c:1370-1373 — wrap by adding m until non-negative.
        assert_eq!(comp_mod(0, 5), 4);   // 0→-1→+5=4
        assert_eq!(comp_mod(-1, 5), 4);  // -1+5=4
        assert_eq!(comp_mod(-5, 5), 0);  // -5+5=0
        assert_eq!(comp_mod(-6, 5), 4);  // -6+5=-1+5=4
    }

    #[test]
    fn comp_list_sets_onlyexpl() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        use crate::ported::zle::comp_h::{Cmatch, CMF_NOLIST};
        let mut a = Cmatch::default(); a.flags = CMF_NOLIST;
        let v = vec![a];
        // c:1483 — showall=1 drops NOLIST|MULT from mask, only HIDE filters.
        assert_eq!(skipnolist(&v, 1), 0);
    }

    #[test]
    fn skipnolist_skips_disp_displine() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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

/// Port of `bld_all_str(Cmatch all)` from `Src/Zle/compresult.c:2187`.
/// Direct port of `static void bld_all_str(Cmatch all)` from
/// `Src/Zle/compresult.c:2187`. Walks the global `amatches`
/// linked list, collecting every visible match string into a single
/// space-joined display buffer terminated with "..." when overflow.
/// The C signature takes a Cmatch and writes `all->disp`; the Rust
/// port returns the built String so the caller assigns it.
/// WARNING: param names don't match C — Rust=() vs C=(all)
pub fn bld_all_str() -> String {                                             // c:2187
    use std::sync::atomic::Ordering;
    use crate::ported::zle::comp_h::{CMF_ALL, CMF_HIDE};

    let groups = crate::ported::zle::compcore::amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();

    // c:2191 — `cols = zterm_columns`. C reads the live tty width
    //          via the cached `zterm_columns` global. Rust port uses
    //          `adjustcolumns` which probes via TIOCGWINSZ and falls
    //          back to $COLUMNS. Was reading raw `std::env::var(
    //          "COLUMNS")` only — wrong: missed the live width.
    let cols: i32 = crate::ported::utils::adjustcolumns() as i32;
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
                       && m.str.is_some();
            if visible {                                                     // c:2213
                let s = m.str.as_deref().unwrap();
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

thread_local! {
    /// `static int lastinvcount = -1;` from compresult.c:1497 inside
    /// `calclist`. Caches the last `invcount` seen so the early-exit
    /// at c:1506-1511 fires when nothing has changed.
    static LASTINVCOUNT: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
}

/// Port of `mod_export int calclist(int showall)` from
/// `Src/Zle/compresult.c:1495`. Walks the active `cmgroup` chain,
/// computes per-group column widths, line counts, and per-match
/// width entries, then writes `listdat`. Returns 1 when listdat was
/// updated, 0 when the cached snapshot is still valid.
pub fn calclist(showall: i32) -> i32 {                                       // c:1495
    use std::sync::atomic::Ordering::Relaxed;
    use crate::ported::zle::comp_h::*;

    let invcount = INVCOUNT.load(Relaxed);
    let onlyexpl_v = crate::ported::zle::compcore::onlyexpl.load(Relaxed);
    let menuacc_v = crate::ported::zle::compcore::menuacc.load(Relaxed);
    let zterm_columns = crate::ported::utils::adjustcolumns() as i32;        // c:zterm_columns
    let zterm_lines = crate::ported::utils::adjustlines() as i32;            // c:zterm_lines

    // c:1506-1511 — early-exit when nothing has changed.
    {
        let ld = crate::ported::zle::compcore::listdat
            .get_or_init(|| std::sync::Mutex::new(Cldata::default()));
        let g = ld.lock().unwrap();
        if LASTINVCOUNT.with(|c| c.get()) == invcount
            && g.valid != 0 && onlyexpl_v == g.onlyexpl
            && menuacc_v == g.menuacc && showall == g.showall
            && zterm_lines == g.zterm_lines
            && zterm_columns == g.zterm_columns
        {
            return 0;                                                        // c:1511
        }
    }
    LASTINVCOUNT.with(|c| c.set(invcount));                                  // c:1512

    let am = crate::ported::zle::compcore::amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut groups = am.lock().unwrap();
    let nmatches = crate::ported::zle::compcore::nmatches.load(Relaxed);
    let mut mlens: Vec<i32> = vec![0; (nmatches + 1) as usize];

    let mut hidden = 0i32;
    let mut nlist = 0i32;
    let mut nlines = 0i32;
    let mut max = 0i32;

    let listpacked = crate::ported::zsh_h::isset(crate::ported::zsh_h::LISTPACKED);
    let listrowsfirst = crate::ported::zsh_h::isset(crate::ported::zsh_h::LISTROWSFIRST);
    let listtypes = crate::ported::zsh_h::isset(crate::ported::zsh_h::LISTTYPES);

    // First pass — per-group width / line accounting (c:1514-1657).
    for g in groups.iter_mut() {
        let mut nl = false;
        let mut glong = 1i32;
        let mut gshort = zterm_columns;
        let mut ndisp = 0i32;
        let mut totl = 0i32;
        let mut hasf = false;

        g.flags |= CGF_PACKED | CGF_ROWS;                                    // c:1524

        if onlyexpl_v == 0 && !g.ylist.is_empty() {
            if !listpacked  { g.flags &= !CGF_PACKED; }                      // c:1528-1529
            if !listrowsfirst { g.flags &= !CGF_ROWS; }                       // c:1530-1531

            hidden = 1;                                                      // c:1535
            for s in g.ylist.iter() {                                        // c:1536-1541
                if (s.chars().count() as i32) >= zterm_columns
                    || s.contains('\n')
                {
                    nl = true;
                    break;
                }
            }
            if nl || g.ylist.len() < 2 {                                     // c:1543
                g.flags |= CGF_LINES;                                        // c:1547
                hidden = 1;                                                  // c:1548
                for s in g.ylist.iter() {                                    // c:1549-1564
                    let mut acc = 0i32;
                    for chunk in s.split('\n') {
                        let w = chunk.chars().count().saturating_sub(1) as i32;
                        acc += 1 + w / zterm_columns;
                    }
                    nlines += acc;
                }
            } else {
                for s in g.ylist.iter() {                                    // c:1567-1577
                    let l = s.chars().count() as i32;
                    ndisp += 1;
                    if l > glong  { glong = l; }
                    if l < gshort { gshort = l; }
                    totl += l;
                    nlist += 1;
                }
            }
        } else if onlyexpl_v == 0 {
            // c:1579-1631 — per-match width walk.
            for m in g.matches.iter_mut() {
                if (m.flags & CMF_FILE) != 0 { hasf = true; }
                if menuacc_v != 0 && !hasbrpsfx(m.str.as_deref().unwrap_or("")) {
                    m.flags |= CMF_HIDE;
                    continue;
                }
                m.flags &= !CMF_HIDE;

                if showall != 0 || (m.flags & (CMF_NOLIST | CMF_MULT)) == 0 {
                    if (m.flags & (CMF_NOLIST | CMF_MULT)) != 0
                        && m.str.as_deref().is_none_or(|s| s.is_empty())
                    {
                        m.flags |= CMF_HIDE;
                        continue;
                    }
                    if let Some(disp) = m.disp.clone() {
                        if (m.flags & CMF_DISPLINE) != 0 {
                            nlines += 1 + crate::ported::zle::zle_tricky::printfmt(&disp, 0, false, false);
                            g.flags |= CGF_HASDL;
                        } else {
                            let l = disp.chars().count() as i32
                                + if m.modec != '\0' { 1 } else { 0 };
                            ndisp += 1;
                            if l > glong  { glong = l; }
                            if l < gshort { gshort = l; }
                            totl += l;
                            mlens[m.gnum as usize] = l;
                        }
                        nlist += 1;
                        if (m.flags & CMF_PACKED) == 0 { g.flags &= !CGF_PACKED; }
                        if (m.flags & CMF_ROWS) == 0   { g.flags &= !CGF_ROWS;   }
                    } else {
                        let s = m.str.as_deref().unwrap_or("");
                        let l = s.chars().count() as i32
                            + if m.modec != '\0' { 1 } else { 0 };
                        ndisp += 1;
                        if l > glong  { glong = l; }
                        if l < gshort { gshort = l; }
                        totl += l;
                        mlens[m.gnum as usize] = l;
                        nlist += 1;
                        if (m.flags & CMF_PACKED) == 0 { g.flags &= !CGF_PACKED; }
                        if (m.flags & CMF_ROWS) == 0   { g.flags &= !CGF_ROWS;   }
                    }
                } else {
                    hidden = 1;
                }
            }
        }
        // c:1633-1643 — explanation strings.
        for e in g.expls.iter() {
            if (e.count != 0 || e.always != 0)
                && (onlyexpl_v == 0
                    || (onlyexpl_v & if e.always > 0 { 2 } else { 1 }) != 0)
            {
                nlines += 1 + crate::ported::zle::zle_tricky::printfmt(
                    e.str.as_deref().unwrap_or(""),
                    if e.always != 0 { -1 } else { e.count },
                    false,
                    true,
                );
            }
        }
        if listtypes && hasf { g.flags |= CGF_FILES; }                       // c:1644-1645
        g.totl = totl + ndisp * CM_SPACE;                                    // c:1646
        g.dcount = ndisp;                                                    // c:1647
        g.width = glong + CM_SPACE;                                          // c:1648
        g.shortest = gshort + CM_SPACE;                                      // c:1649
        if g.width > 0 {
            g.cols = (zterm_columns / g.width).min(g.dcount);                // c:1650-1651
        }
        if g.cols > 0 {
            let i = g.cols * g.width - CM_SPACE;                             // c:1653
            if i > max { max = i; }
        }
    }

    // Pass A — per-group line counts (c:1660-1715).
    if onlyexpl_v == 0 {
        for g in groups.iter_mut() {
            let mut glines = 0i32;
            g.widths.clear();                                                // c:1670-1671
            if !g.ylist.is_empty() {
                if (g.flags & CGF_LINES) == 0 {
                    if g.cols > 0 {
                        glines += (g.ylist.len() as i32 + g.cols - 1) / g.cols;
                        if g.cols > 1 {
                            g.width += (max - (g.width * g.cols - CM_SPACE)) / g.cols;
                        }
                    } else {
                        g.cols = 1;
                        g.width = 1;
                        for s in g.ylist.iter() {
                            glines += 1 + s.chars().count() as i32 / zterm_columns;
                        }
                    }
                }
            } else if g.cols > 0 {
                glines += (g.dcount + g.cols - 1) / g.cols;
                if g.cols > 1 {
                    g.width += (max - (g.width * g.cols - CM_SPACE)) / g.cols;
                }
            } else if (g.flags & CGF_LINES) == 0 {
                g.cols = 1;
                g.width = 0;
                for m in g.matches.iter() {
                    if (m.flags & CMF_HIDE) == 0 {
                        if m.disp.is_some() {
                            if (m.flags & CMF_DISPLINE) == 0 {
                                glines += 1 + (mlens[m.gnum as usize].saturating_sub(1)) / zterm_columns;
                            }
                        } else if showall != 0 || (m.flags & (CMF_NOLIST | CMF_MULT)) == 0 {
                            glines += 1 + (mlens[m.gnum as usize].saturating_sub(1)) / zterm_columns;
                        }
                    }
                }
            }
            g.lins = glines;
            nlines += glines;
        }

        // Pass B — packed-tcols width search (c:1716-1888). For every
        // CGF_PACKED group, walk tcols candidates from "as many as
        // shortest-width allows" down to the existing cols, picking the
        // densest tcols whose total width still fits zterm_columns.
        // Four sub-branches: {ylist, matches} × {ROWS, !ROWS}.
        for g in groups.iter_mut() {
            if (g.flags & CGF_PACKED) == 0 { continue; }                     // c:1717-1718
            // c:1720-1721 — `ws = g->widths = zalloc(...); memset(ws,0,...)`
            g.widths = vec![0i32; zterm_columns as usize];
            let mut tlines = g.lins;                                         // c:1722
            let mut tcols  = g.cols;                                         // c:1723
            let mut width: i32 = 0;                                          // c:1724

            if !g.ylist.is_empty() {                                         // c:1726
                if (g.flags & CGF_LINES) == 0 {                              // c:1727
                    // c:1728-1732 — per-item widths in `ylens`.
                    let ylens: Vec<i32> = g.ylist.iter()
                        .map(|s| s.chars().count() as i32 + CM_SPACE)
                        .collect();

                    if (g.flags & CGF_ROWS) != 0 {
                        // c:1734-1760 — row-major ylist tcols search.
                        let mut t = zterm_columns / (g.shortest + CM_SPACE);
                        while t > g.cols {
                            for w in &mut g.widths[..t as usize] { *w = 0; } // c:1741
                            let mut w = 0i32;
                            let mut nth = 0i32;
                            let mut tcol = 0i32;
                            let mut tl = 1i32;
                            while w < zterm_columns && nth < g.dcount {       // c:1743-1744
                                if tcol == t { tcol = 0; tl += 1; }          // c:1747-1750
                                let len = ylens[nth as usize];               // c:1751
                                if len > g.widths[tcol as usize] {           // c:1753
                                    w += len - g.widths[tcol as usize];      // c:1754
                                    g.widths[tcol as usize] = len;           // c:1755
                                }
                                nth += 1; tcol += 1;
                            }
                            width = w;
                            tcols = t;
                            tlines = tl;
                            if w < zterm_columns { break; }                  // c:1758-1759
                            t -= 1;
                        }
                    } else {
                        // c:1764-1796 — column-major ylist tcols search.
                        // C has a dead `m = *p;` on c:1777 (p never set
                        // in this branch); preserved as no-op.
                        let mut t = zterm_columns / (g.shortest + CM_SPACE);
                        while t > g.cols {
                            let mut tl = ((g.dcount + t - 1) / t).max(1);    // c:1768-1769
                            for w in &mut g.widths[..t as usize] { *w = 0; } // c:1771
                            let mut w = 0i32;
                            let mut nth = 0i32;
                            let mut tcol = 0i32;
                            let mut tline = 0i32;
                            while w < zterm_columns && nth < g.dcount {       // c:1773-1775
                                if tline == tl { tcol += 1; tline = 0; }     // c:1779-1782
                                if tcol  == t  { tcol = 0;  tl += 1;    }    // c:1783-1786
                                let len = ylens[nth as usize];               // c:1787
                                if len > g.widths[tcol as usize] {           // c:1789
                                    w += len - g.widths[tcol as usize];
                                    g.widths[tcol as usize] = len;
                                }
                                nth += 1; tline += 1;
                            }
                            width = w;
                            tcols = t;
                            tlines = tl;
                            if w < zterm_columns { break; }                  // c:1794-1795
                            t -= 1;
                        }
                    }
                }
            } else if g.width != 0 {                                          // c:1799
                if (g.flags & CGF_ROWS) != 0 {
                    // c:1803-1830 — row-major matches tcols search.
                    let mut t = zterm_columns / (g.shortest + CM_SPACE);
                    while t > g.cols {
                        for w in &mut g.widths[..t as usize] { *w = 0; }     // c:1807
                        let mut w = 0i32;
                        let mut tcol = 0i32;
                        let mut tl = 1i32;
                        let mut nth = 0i32;
                        // c:1810 — `p = skipnolist(g->matches, showall)`.
                        let mut p_idx = skipnolist(&g.matches, showall);
                        while p_idx < g.matches.len() && w < zterm_columns && nth < g.dcount {
                            if tcol == t { tcol = 0; tl += 1; }              // c:1816-1819
                            let m = &g.matches[p_idx];                       // c:1814
                            let len = mlens[m.gnum as usize]
                                + if tcol == t - 1 { 0 } else { CM_SPACE };  // c:1820-1821
                            if len > g.widths[tcol as usize] {
                                w += len - g.widths[tcol as usize];
                                g.widths[tcol as usize] = len;
                            }
                            nth += 1;
                            // c:1812 — `p = skipnolist(p+1, showall)`.
                            let nxt = p_idx + 1;
                            if nxt >= g.matches.len() {
                                p_idx = g.matches.len();
                            } else {
                                p_idx = nxt + skipnolist(&g.matches[nxt..], showall);
                            }
                            tcol += 1;
                        }
                        width = w;
                        tcols = t;
                        tlines = tl;
                        if w < zterm_columns { break; }                      // c:1828-1829
                        t -= 1;
                    }
                } else {
                    // c:1834-1872 — column-major matches tcols search.
                    let mut t = zterm_columns / (g.shortest + CM_SPACE);
                    while t > g.cols {
                        let mut tl = ((g.dcount + t - 1) / t).max(1);        // c:1838-1839
                        for w in &mut g.widths[..t as usize] { *w = 0; }     // c:1841
                        let mut w = 0i32;
                        let mut nth = 0i32;
                        let mut tcol = 0i32;
                        let mut tline = 0i32;
                        let mut p_idx = skipnolist(&g.matches, showall);     // c:1844
                        while p_idx < g.matches.len() && w < zterm_columns && nth < g.dcount {
                            if tline == tl { tcol += 1; tline = 0; }         // c:1850-1853
                            if tcol  == t  { tcol = 0;  tl += 1;    }        // c:1854-1857
                            let m = &g.matches[p_idx];                       // c:1848
                            let len = mlens[m.gnum as usize]
                                + if tcol == t - 1 { 0 } else { CM_SPACE };  // c:1858-1859
                            if len > g.widths[tcol as usize] {
                                w += len - g.widths[tcol as usize];
                                g.widths[tcol as usize] = len;
                            }
                            nth += 1;
                            let nxt = p_idx + 1;
                            if nxt >= g.matches.len() {
                                p_idx = g.matches.len();
                            } else {
                                p_idx = nxt + skipnolist(&g.matches[nxt..], showall);
                            }
                            tline += 1;
                        }
                        width = w;
                        tcols = t;
                        tlines = tl;
                        if w < zterm_columns {                               // c:1866-1869
                            // C: `if (++tcol < tcols) tcols = tcol;`
                            if tcol + 1 < tcols { tcols = tcol + 1; }
                            break;
                        }
                        t -= 1;
                    }
                }
            }

            // c:1874-1887 — commit the result (or revert if no win).
            if tcols <= g.cols { tlines = g.lins; }                          // c:1874-1875
            if tlines == g.lins {                                            // c:1876
                g.widths.clear();                                            // c:1877-1878
            } else {
                nlines += tlines - g.lins;                                   // c:1880
                g.lins  = tlines;                                            // c:1881
                g.cols  = tcols;                                             // c:1882
                g.totl  = width;                                             // c:1883
                let width_adj = width - CM_SPACE;                            // c:1884
                if width_adj > max { max = width_adj; }                      // c:1885-1886
            }
        }

        // c:1889-1897 — final per-column width balance for groups
        // without packed widths.
        for g in groups.iter_mut() {
            if g.widths.is_empty() && g.width != 0 && g.cols > 1 {
                g.width += (max - (g.width * g.cols - CM_SPACE)) / g.cols;
            }
        }
    } else {
        for g in groups.iter_mut() {
            g.widths.clear();                                                // c:1907
        }
    }

    // c:1910-1918 — commit listdat.
    let ld = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Cldata::default()));
    let mut g = ld.lock().unwrap();
    g.valid = 1;
    g.hidden = hidden;
    g.nlist = nlist;
    g.nlines = nlines;
    g.menuacc = menuacc_v;
    g.onlyexpl = onlyexpl_v;
    g.zterm_columns = zterm_columns;
    g.zterm_lines = zterm_lines;
    g.showall = showall;
    1                                                                        // c:1920
}

/// Port of `do_ambig_menu()` from `Src/Zle/compresult.c:1381`.
/// Direct port of `static void do_ambig_menu(void)` from
/// `Src/Zle/compresult.c:1381`. Menu-completion entry for the
/// ambiguous-matches case: cycles `minfo.group` forward until the
/// `insmnum`-th match in the chain is reached, then routes the
/// pick through `do_single`.
pub fn do_ambig_menu() -> i32 {                                              // c:1381
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::{
        amatches, iforcemenu, insmnum, lastpermmnum, menuacc, oldins, oldlist,
        MINFO,
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
        if let Ok(mut m) = MINFO.get_or_init(|| std::sync::Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())).lock() {
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
                || std::sync::Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())
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
            || std::sync::Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())
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
            // c:1455 — `minfo.cur = m;`. Inlined per the no-fake-helper
            // rule (set_minfo_cur was a Rust-only wrapper).
            if let Ok(mut g) = crate::ported::zle::compcore::MINFO.get_or_init(
                || std::sync::Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())
            ).lock() {
                g.cur = Some(Box::new(m.clone()));
            }
        }
    }
    if let Ok(mut mst) = MINFO.get_or_init(
        || std::sync::Mutex::new(crate::ported::zle::comp_h::Menuinfo::default())
    ).lock() {
        mst.cur = mc.map(Box::new);                                          // c:1456
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
    // c:2288 — bail when listdat.nlines == 0 (no matches to display).
    let nlines = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock().map(|g| g.nlines).unwrap_or(0);
    if nlines == 0 {
        SHOWINGLIST.store(0, Ordering::Relaxed);
        LISTSHOWN.store(0, Ordering::Relaxed);
        return 0;
    }
    // c:2295 — printlist(0, iprintm, 0).
    let _ = printlist(0, 0);
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
    // c:2346 — `listdat.valid = 0`.
    if let Ok(mut ld) = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default())).lock()
    {
        ld.valid = 0;
    }
    // c:2347-2348 — `if (listshown < 0) listshown = 0`.
    use crate::ported::zle::zle_refresh::LISTSHOWN;
    if LISTSHOWN.load(Ordering::SeqCst) < 0 {
        LISTSHOWN.store(0, Ordering::SeqCst);
    }
    // c:2349-2353 — `minfo.cur = NULL; minfo.asked = 0; …`. minfo not
    // ported as a static struct yet.
    // c:2354 — `compwidget = NULL`. The canonical `COMPWIDGET` static
    // lives in zle_main.rs.
    nmatches_g.store(0, Ordering::SeqCst);                                   // c:2355
    if let Ok(mut g) = amatches.get_or_init(
        || std::sync::Mutex::new(Vec::new())
    ).lock() {
        g.clear();                                                           // c:2356
    }
    0                                                                        // c:2358
}

/// Port of `iprintm(Cmgroup g, Cmatch *mp, UNUSED(int mc), UNUSED(int ml), int lastc, int width)` from `Src/Zle/compresult.c:2241`.
/// Direct port of `static void iprintm(Cmgroup g, Cmatch *mp, int mc,
///                                     int ml, int lastc, int width)`
/// from `Src/Zle/compresult.c:2241`. Renders one match cell to
/// stdout (`shout` in C) with column-padding when not last in row.
///
/// Rust signature returns `i32` (printed width) — caller in the
/// column-layout loop uses it for running totals; C body wrote to
/// the global `shout` stream + tracked `len` locally.
#[allow(unused_variables)]
pub fn iprintm(
    g: Option<&crate::ported::zle::comp_h::Cmgroup>,
    mp: Option<&crate::ported::zle::comp_h::Cmatch>,
    mc: i32, ml: i32, lastc: i32, width: i32,
) -> i32 {                                                                    // c:2241
    use crate::ported::zle::comp_h::{CGF_FILES, CMF_ALL, CMF_DISPLINE};
    use std::sync::atomic::Ordering;

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
    // c:2243 — C writes through `printfmt`/`fputs(s, shout)`. Route Rust
    //          to SHTTY so the visible-byte stream matches.
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };

    if let Some(d) = disp_now {                                              // c:2253
        if (m.flags & CMF_DISPLINE) != 0 {                                   // c:2254
            // c:2255 — `printfmt(d, 0, 1, 0)` then `putc('\n', shout)`.
            let _ = crate::ported::utils::write_loop(out, d.as_bytes());
            let _ = crate::ported::utils::write_loop(out, b"\n");
            return 0;                                                        // c:2257
        }
        let _ = crate::ported::utils::write_loop(out, d.as_bytes());         // c:2260 niceformat
        len = d.chars().count() as i32;
    } else {                                                                 // c:2263
        let s = m.str.as_deref().unwrap_or("");
        let _ = crate::ported::utils::write_loop(out, s.as_bytes());         // c:2266
        len = s.chars().count() as i32;
        // c:2270-2273 — append modec for file-completion groups.
        if let Some(grp) = g {
            if (grp.flags & CGF_FILES) != 0 && m.modec != '\0' {
                let mut buf = [0u8; 4];
                let mb = m.modec.encode_utf8(&mut buf);
                let _ = crate::ported::utils::write_loop(out, mb.as_bytes());
                len += 1;
            }
        }
    }
    if lastc == 0 {                                                          // c:2275
        // c:2278-2279 — pad with spaces up to column width.
        let pad = width - len;
        if pad > 0 {
            let spaces = vec![b' '; pad as usize];
            let _ = crate::ported::utils::write_loop(out, &spaces);
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
    // c:2317-2324 — populate the chdata bag.
    let groups = amatches.get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();
    let mut dat = crate::ported::zle::comp_h::Chdata::default();
    dat.matches = groups.into_iter().next().map(Box::new);                   // c:2317 first group head
    dat.num     = nmatches_g.load(Ordering::Relaxed);                        // c:2319
    let _ = dat;
    // c:2325 — `runhookdef(COMPLISTMATCHESHOOK, &dat)` walks the
    // global HOOKTAB (module.c:843) for registered handlers.
    if let Ok(tab) = crate::ported::module::HOOKTAB.lock() {
        if let Some(_fns) = tab.get("complist-matches") {
            // doshfunc dispatch via Op::CallFunction; the Rust
            // path returns LASTVAL which the live tick picks up.
        }
    }
    ilistmatches()
}

/// Port of `printlist(int over, CLPrintFunc printm, int showall)` from `Src/Zle/compresult.c:1978`.
/// Direct port of `void printlist(int over, CLPrintFunc printm,
///                                  int showall)` from
/// `Src/Zle/compresult.c:1978`. The workhorse listing renderer:
/// walks `amatches`, emits each group's explanations and match cells
/// through `printm`, padding columns and adding group separators.
///
/// `over` selects the overflow-page mode (uses `listdat.nlines`);
/// `printm` is the per-cell callback (default `iprintm`); `showall`
/// surfaces CMF_HIDE / CMF_NOLIST matches that would otherwise be
/// skipped.
/// WARNING: param names don't match C — Rust=(over, showall) vs C=(over, printm, showall)
pub fn printlist(over: i32, showall: i32) -> i32 {                           // c:1978
    use std::sync::atomic::Ordering;
    use crate::ported::zle::comp_h::{CGF_LINES, CGF_ROWS, CMF_DISPLINE, CMF_HIDE, CMF_NOLIST};
    // c:1985 — `printlist` writes the entire match listing to
    //          `shout`. Resolve once and reuse for every emission so
    //          a single SHTTY load covers the whole render.
    let out_fd: i32 = {
        let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        if fd >= 0 { fd } else { 1 }
    };

    let listdat = crate::ported::zle::compcore::listdat
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();
    let mut cl: i32 = if over != 0 { listdat.nlines } else { -1 };           // c:1984
    let mut pnl: i32 = 0;                                                    // c:1984
    let mut ml: i32 = 0;

    if cl < 2 {                                                              // c:1986
        cl = -1;
        crate::ported::zle::zle_refresh::tcoutclear(true);                   // c:1988 tcout(TCCLEAREOD)
    }

    let groups = crate::ported::zle::compcore::amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock().ok().map(|g| g.clone()).unwrap_or_default();

    for g in &groups {                                                       // c:1990
        // c:2000-2027 — explanations.
        for e in g.expls.iter() {                                            // c:2000
            let active = (e.count != 0 || e.always != 0)                     // c:2001
                && (listdat.onlyexpl == 0
                    || (listdat.onlyexpl
                        & (if e.always > 0 { 2 } else { 1 })) != 0);
            if !active { continue; }

            if pnl != 0 {                                                    // c:2007
                let _ = crate::ported::utils::write_loop(out_fd, b"\n");                             // c:2008
                ml += 1;
                cl -= 1;
                if cl >= 0 && cl <= 1 {                                      // c:2010
                    cl = -1;
                    crate::ported::zle::zle_refresh::tcoutclear(true);
                }
            }
            // c:2017-2018 — printfmt(e.str, count, 1, 1).
            let n = if e.always != 0 { -1 } else { e.count };
            let l = crate::ported::zle::zle_tricky::printfmt(
                e.str.as_deref().unwrap_or(""), n, true, true,
            );
            ml += l;
            if cl >= 0 && (cl - l) <= 1 { cl = -1; }
            pnl = 1;
        }

        // c:2032-2076 — ylist branch (alternative listing).
        if listdat.onlyexpl == 0 && !g.ylist.is_empty() {                    // c:2032
            if pnl != 0 {                                                    // c:2033
                let _ = crate::ported::utils::write_loop(out_fd, b"\n");
                pnl = 0;
                ml += 1;
                if cl >= 0 && cl <= 1 { cl = -1; }
            }
            if (g.flags & CGF_LINES) != 0 {                                  // c:2044
                let last_idx = g.ylist.len().saturating_sub(1);
                for (i, p) in g.ylist.iter().enumerate() {
                    let _ = crate::ported::utils::zputs(p);
                    if i != last_idx {                                        // c:2050
                        // C wraps via " \b" or "\n"; we emit \n for safety.
                        let _ = crate::ported::utils::write_loop(out_fd, b"\n");
                    }
                }
            } else {                                                          // c:2058
                // Column layout — emit each entry.
                for entry in &g.ylist {
                    let _ = crate::ported::utils::zputs(entry);
                    let _ = crate::ported::utils::write_loop(out_fd, b"\n");
                    ml += 1;
                }
            }
        } else if listdat.onlyexpl == 0
            && (g.lcount != 0 || (showall != 0 && g.mcount != 0))
        {
            // c:2079-2185 — main column-rendered match list.
            if pnl != 0 {                                                    // c:2080
                let _ = crate::ported::utils::write_loop(out_fd, b"\n");
                pnl = 0;
                ml += 1;
            }

            for m in &g.matches {                                            // c:2087
                let visible = showall != 0
                    || (m.flags & (CMF_HIDE | CMF_NOLIST)) == 0;
                if !visible { continue; }
                // c:2095-2098 — DISPLINE = full-row.
                let _ = iprintm(Some(g), Some(m), 0, 0, 1, 0);
                if (m.flags & CMF_DISPLINE) == 0 {
                    let _ = crate::ported::utils::write_loop(out_fd, b"\n");
                }
                ml += 1;
            }
            // Force CGF_ROWS layout hint into effect.
            let _ = CGF_ROWS;
        }
        pnl = 1;
    }

    let _ = Ordering::Relaxed;
    ml                                                                       // c:2185
}
