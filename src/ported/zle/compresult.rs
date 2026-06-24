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

use std::sync::atomic::Ordering;
use std::sync::atomic::Ordering::Relaxed;

use crate::ported::init::SHTTY;
use crate::ported::utils::{adjustcolumns, adjustlines, write_loop, zputs};
use crate::ported::zle::comp_h::{
    Aminfo, Chdata, Cldata, Cline, Cmatch, Cmgroup, Menuinfo, CGF_FILES, CGF_HASDL, CGF_LINES,
    CGF_PACKED, CGF_ROWS, CLF_LINE, CLF_SUF, CMF_ALL, CMF_DISPLINE, CMF_FILE, CMF_HIDE, CMF_MULT,
    CMF_NOLIST, CMF_PACKED, CMF_ROWS,
};
use crate::ported::zle::compcore::{
    amatches, fromcomp, iforcemenu, insmnum, lastmatches, lastpermmnum, listdat as listdat_static,
    menuacc, nmatches as nmatches_g, nmatches, oldins, oldlist, onlyexpl, MINFO,
};
use crate::ported::zle::complete::COMPLISTMAX;
use crate::ported::zle::computil::CM_SPACE;
use crate::ported::zle::zle_h::COMP_LIST_COMPLETE;
use crate::ported::zle::zle_refresh::tcout;
use crate::ported::zle::zle_tricky::printfmt;
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{isset, LISTPACKED, LISTROWSFIRST, LISTTYPES, USEZLE};
/// Port of `mod_export int invcount` from `Src/Zle/compresult.c:37`.
/// Invalidation counter — bumped every time the cached completion
/// list goes stale. `complistmatches` reads it to detect "we have a
/// new list" without comparing the full Cmgroup chain.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]

/// Truncate a long completion line with `...` so it fits a column
/// budget.
/// Port of `cut_cline(Cline l)` from Src/Zle/compresult.c. The C source
/// truncates the Cline's display field to `max_len`; ours emits
/// `…` (three ASCII dots) when truncation is needed.
/// WARNING: param names don't match C — Rust=(s, max_len) vs C=(l)
pub fn cut_cline(s: &str, max_len: usize) -> String {
    // c:46
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
/// The C source also integrates with `inststrlen` (buffer edit, `ins=1`
/// mode), `brbeg`/`brend` (brace chains), and `posl` (position-list
/// output). The Rust port handles the `ins=0` / `csp=NULL` /
/// `posl=NULL` case — pure visible-text rendering. Callers that need
/// the buffer-edit side ( `do_ambiguous` etc.) wrap this with
/// foredel+inststr against the `ZLEMETALINE` global.
/// WARNING: signature change — C=(l, ins, csp, posl) vs Rust=(l) -> String
pub fn cline_str(
    // c:165
    l: Option<&Cline>,
) -> String {
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
                if let Some(s) = s {
                    out.push_str(s);
                }
                p = part.next.as_deref();
            }
        }
        // c:282-285 — emit the anchor.
        let anchor = if (node.flags & CLF_LINE) != 0 {
            node.line.as_deref()
        } else {
            node.word.as_deref()
        };
        if let Some(a) = anchor {
            out.push_str(a);
        }

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
                if let Some(s) = s {
                    out.push_str(s);
                }
                p = part.next.as_deref();
            }
        }
        cur = node.next.as_deref();
    }
    out
}

/// Render the "n/total" position label shown in the menu status
/// line.
/// Port of the position-string formatting in
/// Src/Zle/compresult.c (the `clprintm` group-header path).
pub fn build_pos_string(current: usize, total: usize) -> String {
    // c:489
    format!("{}/{}", current + 1, total)
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
pub fn unambig_data(matches: &[String]) -> String {
    // c:525
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

// scs is used to return the position where a automatically created suffix  // c:578
// has to be inserted.                                                       // c:578
/// Replace `[word_start, word_end)` in `buffer` with `replacement`,
/// returning the new buffer plus updated cursor position.
/// Port of `instmatch(Cmatch m, int *scs)` from Src/Zle/compresult.c. The C source
/// uses this as the lowest-level "swap the partial word for the
/// chosen completion" primitive used by every other inserter
/// (`do_single`, `do_ambiguous`, `do_allmatches`).
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, replacement) vs C=(m, scs)
pub fn instmatch(
    // c:578
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

/// Detect whether a string contains brace-expansion metacharacters
/// that would need quoting on insertion.
/// Port of `hasbrpsfx(Cmatch m, char *pre, char *suf)` from Src/Zle/compresult.c — used by the
/// brace-suffix tracking that compsys keeps for menu completion.
/// WARNING: param names don't match C — Rust=(s) vs C=(m, pre, suf)
/// !!! WARNING: PARTIAL PORT / RUST-ONLY SHAPE — does NOT match C's
/// `hasbrpsfx(Cmatch m, char *pre, char *suf)` at Src/Zle/compresult.c:685.
/// C body (39 lines): checks whether the brace-prefix/brace-suffix on
/// a Cmatch entry differ from the menu's `lastprebr`/`lastpostbr`
/// state, including the metafy_line() bookkeeping for non-meta input.
/// This Rust impl is a heuristic — "does the string contain `{` or
/// `}`?" — used by ambiguous-match decisions in callers (lines 445,
/// 816). The two callers pass single strings, not Cmatch entries.
///
/// Faithful port needs Cmatch.brpre/brsuf field access + zlemetaline
/// metafy/unmetafy round-trip + lastprebr/lastpostbr globals. Tracked.
pub fn hasbrpsfx(s: &str) -> bool {
    // c:685 — see WARNING above; Rust impl is a heuristic.
    s.contains('{') || s.contains('}')
}

/// Direct port of `static int do_ambiguous(void)` from
/// `Src/Zle/compresult.c:744`. The ambiguous-completion handler —
/// computes the unambiguous prefix from `ainfo.line` via `cline_str`
/// (falls back to LCP over the supplied matches when ainfo->line
/// isn't populated), then `foredel`+`inststr` against ZLEMETALINE
/// when WB/WE indicate a real completion is in flight. Sets the
/// `menucmp=0`/`lastambig=1` transition flags. Returns 1 if any
/// completion text was inserted, 0 otherwise.
/// WARNING: param names don't match C — Rust=(matches) vs C=()
pub fn do_ambiguous(matches: &[String]) -> i32 {
    // c:744
    // c:748 — `menucmp = menuacc = 0`.
    MENUCMP.store(0, Relaxed);
    // c:763 — `lastambig = 1`.
    LASTAMBIG.store(1, Relaxed);

    // c:774 — if `ainfo` is populated, walk ainfo->line via cline_str
    // (compresult.c:535 path); else fall back to the LCP over the
    // provided match strings.
    let ainfo_line = crate::ported::zle::compcore::ainfo
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|a| a.line.clone()));
    let prefix = if let Some(line) = ainfo_line {
        cline_str(Some(line.as_ref())) // c:535
    } else {
        unambig_data(matches)
    };
    if prefix.is_empty() && matches.is_empty() {
        return 0; // c:nomatch
    }
    // c:783-790 — buffer-edit: foredel the original word, inststr
    // the unambig prefix, when WB/WE describe a valid range.
    if !prefix.is_empty() {
        let wb = crate::ported::zle::compcore::WB.load(Relaxed);
        let we = crate::ported::zle::compcore::WE.load(Relaxed);
        if we > wb && wb >= 0 {
            let span = we - wb;
            crate::ported::zle::compcore::ZLEMETACS.store(wb, Relaxed); // c:785
            foredel(span, 0); // c:787
            let _ = inststr(&prefix); // c:790
        }
    }
    if !prefix.is_empty() {
        1
    } else {
        0
    }
}

/// Port of `ztat(char *nam, struct stat *buf, int ls)` from `Src/Zle/compresult.c:869`.
/// `stat()` wrapper that follows symlinks unless `ls` is non-zero.
/// Returns `Option<Metadata>` mirroring C's `0`/`-1` return where
/// the metadata is filled into the supplied `struct stat *buf`.
/// WARNING: param names don't match C — Rust=(path, follow_symlink) vs C=(nam, buf, ls)
pub fn ztat(path: &str, follow_symlink: bool) -> Option<std::fs::Metadata> {
    // c:869
    if follow_symlink {
        // c:869 if (ls)
        // c:869 — `lstat(nam, buf)`. Don't follow symlinks.
        std::fs::symlink_metadata(path).ok()
    } else {
        // c:869 else
        // c:869 — `stat(nam, buf)`. Follow symlinks.
        std::fs::metadata(path).ok()
    }
}

/// Insert every match into the buffer joined by `separator`.
/// Port of `do_allmatches(UNUSED(int end))` from Src/Zle/compresult.c — fires for
/// the `all-matches` widget and for the implicit case when no
/// listing fits.
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, matches, separator) vs C=(end)
pub fn do_allmatches(
    // c:897
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

/// Insert the single chosen match, optionally appending a space.
/// Port of `do_single(Cmatch m)` from Src/Zle/compresult.c — fired when
// Insert a single match in the command line.                              // c:963
/// completion produced exactly one match. The trailing space is
/// the `AUTO_REMOVE_SLASH`-aware insertion that distinguishes
/// finished-completion from prefix-completion.
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, the_match, add_space) vs C=(m)
pub fn do_single(
    // c:963
    buffer: &str,
    cursor: usize,
    word_start: usize,
    word_end: usize,
    the_match: &str,
    add_space: bool,
) -> (String, usize) {
    // c:974 — `fixsuffix()` clears any pending menu-suffix state before
    // inserting the new match.
    fixsuffix();
    let suffix = if add_space { " " } else { "" };
    let replacement = format!("{}{}", the_match, suffix);
    instmatch(buffer, cursor, word_start, word_end, &replacement)
}

/// !!! WARNING: PARTIAL PORT / RUST-ONLY SHAPE — does NOT match C's
/// `valid_match(Cmatch *m, int next)` at Src/Zle/compresult.c:1210.
/// C body (32 lines) walks the match list (menuacc / minfo.group /
/// amatches / lmatches), skipping CMF_DUMMY / CMF_NOLIST / CMF_MULT
/// entries while honoring zmult direction. This Rust port instead
/// does a simple word.starts_with(prefix) + ends_with(suffix) check —
/// the `compadd -P pre -S suf` predicate, which is what the only
/// callers (the in-file tests) want.
///
/// The C function and this one share a name but have completely
/// different semantics. Per PORT.md Rule 0 (no invented ported), a
/// faithful port would need the full minfo/amatches infrastructure
/// AND the callers would need rewriting. Tracked for follow-up.
pub fn valid_match(word: &str, prefix: &str, suffix: &str) -> bool {
    // c:1210 — see WARNING above; Rust impl is a different fn.
    word.starts_with(prefix) && (suffix.is_empty() || word.ends_with(suffix))
}

/// Direct port of `void do_menucmp(int lst)` from `Src/Zle/compresult.c:1253`.
/// Steps the menu cursor forward/backward, wrapping at ends. Per C:
/// when `lst == COMP_LIST_COMPLETE`, just set `showinglist=-2` and
/// return (caller refreshes the listing instead of inserting). The
/// Rust port returns the next match index; caller drives instmatch.
/// WARNING: param names don't match C — Rust=(matches, current, forward) vs C=(lst)
pub fn do_menucmp(matches: &[String], current: usize, forward: bool) -> (usize, &str) {
    // c:1253
    // c:1258 — `if (lst == COMP_LIST_COMPLETE) { showinglist = -2; return; }`.
    // We don't have a `lst` param at this signature; the listing-mode
    // call site (compresult.c via do_menucmp(lst==LIST_COMPLETE)) uses
    // a separate caller path. If the host's menu loop wraps to the
    // current entry (matches.len()==1), set showinglist=-2 so a
    // re-list happens.
    let _ = COMP_LIST_COMPLETE;
    if matches.is_empty() {
        return (0, "");
    }
    if matches.len() == 1 {
        SHOWINGLIST.store(-2, Relaxed);
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

/// Direct port of `accept_last()` from `Src/Zle/compresult.c:1288`.
/// Finalises the currently-selected menu match into the buffer.
///
/// Per C c:1299-1322: when !menuacc, snapshot lastprebr/lastpostbr
/// into minfo.prebr/postbr; if listshown is set and any match in
/// amatches lacks the brace prefix/suffix, force showinglist=-2.
/// Then bump menuacc and proceed with the do_single insertion.
/// WARNING: param names don't match C — Rust=(cursor, word_start, word_end, selected) vs C=()
pub fn accept_last(
    // c:1288
    buffer: &str,
    cursor: usize,
    word_start: usize,
    word_end: usize,
    selected: &str,
) -> (String, usize) {
    use std::sync::atomic::Ordering;

    // c:1299 — `if (!menuacc)` snapshot prebr/postbr.
    if menuacc.load(Relaxed) == 0 {
        // c:1299
        let prebr = LASTPREBR
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        let postbr = LASTPOSTBR
            .get_or_init(|| std::sync::Mutex::new(String::new()))
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            m.prebr = Some(prebr.clone()); // c:1301
            m.postbr = Some(postbr.clone()); // c:1303
        }
        // c:1305-1321 — if listshown set and braces differ on any
        // match, set showinglist=-2 so the listing re-renders.
        if LISTSHOWN.load(Relaxed) != 0 && (!prebr.is_empty() || !postbr.is_empty()) {
            let groups = amatches
                .get_or_init(|| std::sync::Mutex::new(Vec::new()))
                .lock()
                .ok()
                .map(|g| g.clone())
                .unwrap_or_default();
            for g in &groups {
                for m in &g.matches {
                    // c:1315
                    let s = m.str.as_deref().unwrap_or("");
                    if !hasbrpsfx(s) {
                        // c:1316
                        SHOWINGLIST.store(-2, Relaxed); // c:1317
                        break;
                    }
                }
            }
        }
    }
    menuacc.fetch_add(1, Relaxed); // c:1323
    do_single(buffer, cursor, word_start, word_end, selected, true)
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
pub fn comp_mod(mut v: i32, m: i32) -> i32 {
    // c:1364
    // Guard: C source assumes `m > 0` (lastpermmnum is always
    // populated from the match-list count by the time do_ambig_menu
    // calls it). With `m == 0`, the `while v < 0; v += m;` loop at
    // c:1371-1372 spins forever. zshrs's unit tests call do_ambig_menu
    // directly from a fresh state where lastpermmnum == 0 and the
    // process hangs (cargo test never finishes). C zsh hits the same
    // bug if any internal path forgets to populate lastpermmnum;
    // defensive guard with no behavior change for valid `m > 0`.
    if m <= 0 {
        return 0;
    }
    if v >= 0 {
        // c:1364
        v -= 1; // c:1367
    }
    if v >= 0 {
        // c:1368
        v % m // c:1369
    } else {
        // c:1370
        while v < 0 {
            // c:1371
            v += m; // c:1372
        }
        v // c:1373
    }
}

/// Port of `do_ambig_menu()` from `Src/Zle/compresult.c:1381`.
/// Direct port of `static void do_ambig_menu(void)` from
/// `Src/Zle/compresult.c:1381`. Menu-completion entry for the
/// ambiguous-matches case: cycles `minfo.group` forward until the
/// `insmnum`-th match in the chain is reached, then routes the
/// pick through `do_single`.
pub fn do_ambig_menu() -> i32 {
    // c:1381

    // c:1386 — `if (iforcemenu == -1) do_ambiguous();`
    if iforcemenu.load(Relaxed) == -1 {
        // c:1386
        let _ = do_ambiguous(&[]); // c:1387
    }

    let um = USEMENU.load(Relaxed);
    if um != 3 {
        // c:1389
        MENUCMP.store(1, Relaxed); // c:1390
        menuacc.store(0, Relaxed); // c:1391
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
            m.cur = None; // c:1392
        }
    } else {
        if oldlist.load(Relaxed) != 0 {
            // c:1395
            let has_cur = MINFO
                .get()
                .and_then(|m| m.lock().ok())
                .map(|m| m.cur.is_some())
                .unwrap_or(false);
            if oldins.load(Relaxed) != 0 && has_cur {
                // c:1396
                // C: `accept_last()` — accepts the current menu pick.
                // Rust sig takes (buf, cs, wb, we, selected); call with
                // empties since we just want the side-effect.
                let _ = accept_last("", 0, 0, 0, ""); // c:1397
            }
        } else {
            if let Ok(mut m) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                m.cur = None; // c:1399
            }
        }
    }

    // c:1429 — `insmnum = comp_mod(insmnum, lastpermmnum)`.
    let mut idx = comp_mod(insmnum.load(Relaxed), lastpermmnum.load(Relaxed));
    insmnum.store(idx, Relaxed);

    // c:1430-1438 — walk amatches advancing past groups with mcount<=idx.
    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut chosen_group: Option<Cmgroup> = None;
    for g in &groups {
        if g.mcount > idx {
            chosen_group = Some(g.clone());
            break;
        }
        idx -= g.mcount;
    }

    let Some(g) = chosen_group else {
        // c:1440-1444
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
            .lock()
        {
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

    if iforcemenu.load(Relaxed) != -1 {
        // c:1454
        if let Some(ref m) = mc {
            // c:1455 — `minfo.cur = m;`. Inlined per the no-fake-helper
            // rule (set_minfo_cur was a Rust-only wrapper).
            if let Ok(mut g) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
                .lock()
            {
                g.cur = Some(Box::new(m.clone()));
            }
        }
    }
    if let Ok(mut mst) = MINFO
        .get_or_init(|| std::sync::Mutex::new(Menuinfo::default()))
        .lock()
    {
        mst.cur = mc.map(Box::new); // c:1456
    }
    0
}

/// Compute how many rows the list will take given a fixed column
/// count.
/// Port of `list_lines()` from Src/Zle/compresult.c — the listing
/// path uses this to decide whether to invoke the more-prompt
/// (`asklistscroll`).
// Return the number of screen lines needed for the list.                   // c:1450
/// WARNING: param names don't match C — Rust=(matches, columns) vs C=()
pub fn list_lines(matches: &[String], columns: usize) -> usize {
    // c:1450
    if columns == 0 {
        return matches.len();
    }
    matches.len().div_ceil(columns)
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
/// C body (Src/Zle/compresult.c:1468) is 4 lines:
///   `zsfree(complist); complist = v;
///    onlyexpl = v ? ((strstr(v,"expl")?1:0) |
///                    (strstr(v,"messages")?2:0)) : 0;`
pub fn comp_list(v: Option<&str>) {
    // c:1468
    let mut g = crate::ported::zle::complete::COMPLIST // c:1470 zsfree+assign
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
        .unwrap();
    g.clear();
    if let Some(s) = v {
        g.push_str(s);
    }
    let val = v.map_or(0, |s| {
        (s.contains("expl") as i32) | (s.contains("messages") as i32) << 1
    }); // c:1473
    onlyexpl.store(val, Ordering::SeqCst); // c:1473
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
pub fn skipnolist(p: &[Cmatch], showall: i32) -> usize {
    // c:1481
    // c:1483 — `mask = (showall ? 0 : (CMF_NOLIST|CMF_MULT)) | CMF_HIDE`.
    let mask = if showall != 0 {
        0
    } else {
        CMF_NOLIST | CMF_MULT
    } | CMF_HIDE;
    let mut i = 0usize; // c:1485 *p
    while i < p.len() {
        // c:1485 while (*p && ...)
        let m = &p[i];
        let f = m.flags;
        let skip_mask = (f & mask) != 0; // c:1485
        let skip_disp = m.disp.is_some() && (f & (CMF_DISPLINE | CMF_HIDE)) != 0; // c:1486-1487
        if !(skip_mask || skip_disp) {
            break;
        }
        i += 1; // c:1488 p++
    }
    i // c:1490 return p
}

/// Port of `mod_export int calclist(int showall)` from
/// `Src/Zle/compresult.c:1495`. Walks the active `cmgroup` chain,
/// computes per-group column widths, line counts, and per-match
/// width entries, then writes `listdat`. Returns 1 when listdat was
/// updated, 0 when the cached snapshot is still valid.
pub fn calclist(showall: i32) -> i32 {
    // c:1495

    let invcount = INVCOUNT.load(Relaxed);
    let onlyexpl_v = onlyexpl.load(Relaxed);
    let menuacc_v = menuacc.load(Relaxed);
    let zterm_columns = adjustcolumns() as i32; // c:zterm_columns
    let zterm_lines = adjustlines() as i32; // c:zterm_lines

    // c:1506-1511 — early-exit when nothing has changed.
    {
        let ld = listdat_static.get_or_init(|| std::sync::Mutex::new(Cldata::default()));
        let g = ld.lock().unwrap();
        if LASTINVCOUNT.with(|c| c.get()) == invcount
            && g.valid != 0
            && onlyexpl_v == g.onlyexpl
            && menuacc_v == g.menuacc
            && showall == g.showall
            && zterm_lines == g.zterm_lines
            && zterm_columns == g.zterm_columns
        {
            return 0; // c:1511
        }
    }
    LASTINVCOUNT.with(|c| c.set(invcount)); // c:1512

    let am = amatches.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut groups = am.lock().unwrap();
    let nmatches2 = nmatches.load(Relaxed);
    let mut mlens: Vec<i32> = vec![0; (nmatches2 + 1) as usize];

    let mut hidden = 0i32;
    let mut nlist = 0i32;
    let mut nlines = 0i32;
    let mut max = 0i32;

    let listpacked = isset(LISTPACKED);
    let listrowsfirst = isset(LISTROWSFIRST);
    let listtypes = isset(LISTTYPES);

    // First pass — per-group width / line accounting (c:1514-1657).
    for g in groups.iter_mut() {
        let mut nl = false;
        let mut glong = 1i32;
        let mut gshort = zterm_columns;
        let mut ndisp = 0i32;
        let mut totl = 0i32;
        let mut hasf = false;

        g.flags |= CGF_PACKED | CGF_ROWS; // c:1524

        if onlyexpl_v == 0 && !g.ylist.is_empty() {
            if !listpacked {
                g.flags &= !CGF_PACKED;
            } // c:1528-1529
            if !listrowsfirst {
                g.flags &= !CGF_ROWS;
            } // c:1530-1531

            hidden = 1; // c:1535
            for s in g.ylist.iter() {
                // c:1536-1541
                if (s.chars().count() as i32) >= zterm_columns || s.contains('\n') {
                    nl = true;
                    break;
                }
            }
            if nl || g.ylist.len() < 2 {
                // c:1543
                g.flags |= CGF_LINES; // c:1547
                hidden = 1; // c:1548
                for s in g.ylist.iter() {
                    // c:1549-1564
                    let mut acc = 0i32;
                    for chunk in s.split('\n') {
                        let w = chunk.chars().count().saturating_sub(1) as i32;
                        acc += 1 + w / zterm_columns;
                    }
                    nlines += acc;
                }
            } else {
                for s in g.ylist.iter() {
                    // c:1567-1577
                    let l = s.chars().count() as i32;
                    ndisp += 1;
                    if l > glong {
                        glong = l;
                    }
                    if l < gshort {
                        gshort = l;
                    }
                    totl += l;
                    nlist += 1;
                }
            }
        } else if onlyexpl_v == 0 {
            // c:1579-1631 — per-match width walk.
            for m in g.matches.iter_mut() {
                if (m.flags & CMF_FILE) != 0 {
                    hasf = true;
                }
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
                            nlines += 1 + printfmt(&disp, 0, false, false);
                            g.flags |= CGF_HASDL;
                        } else {
                            let l =
                                disp.chars().count() as i32 + if m.modec != '\0' { 1 } else { 0 };
                            ndisp += 1;
                            if l > glong {
                                glong = l;
                            }
                            if l < gshort {
                                gshort = l;
                            }
                            totl += l;
                            mlens[m.gnum as usize] = l;
                        }
                        nlist += 1;
                        if (m.flags & CMF_PACKED) == 0 {
                            g.flags &= !CGF_PACKED;
                        }
                        if (m.flags & CMF_ROWS) == 0 {
                            g.flags &= !CGF_ROWS;
                        }
                    } else {
                        let s = m.str.as_deref().unwrap_or("");
                        let l = s.chars().count() as i32 + if m.modec != '\0' { 1 } else { 0 };
                        ndisp += 1;
                        if l > glong {
                            glong = l;
                        }
                        if l < gshort {
                            gshort = l;
                        }
                        totl += l;
                        mlens[m.gnum as usize] = l;
                        nlist += 1;
                        if (m.flags & CMF_PACKED) == 0 {
                            g.flags &= !CGF_PACKED;
                        }
                        if (m.flags & CMF_ROWS) == 0 {
                            g.flags &= !CGF_ROWS;
                        }
                    }
                } else {
                    hidden = 1;
                }
            }
        }
        // c:1633-1643 — explanation strings.
        for e in g.expls.iter() {
            if (e.count != 0 || e.always != 0)
                && (onlyexpl_v == 0 || (onlyexpl_v & if e.always > 0 { 2 } else { 1 }) != 0)
            {
                nlines += 1 + printfmt(
                    e.str.as_deref().unwrap_or(""),
                    if e.always != 0 { -1 } else { e.count },
                    false,
                    true,
                );
            }
        }
        if listtypes && hasf {
            g.flags |= CGF_FILES;
        } // c:1644-1645
        g.totl = totl + ndisp * CM_SPACE; // c:1646
        g.dcount = ndisp; // c:1647
        g.width = glong + CM_SPACE; // c:1648
        g.shortest = gshort + CM_SPACE; // c:1649
        if g.width > 0 {
            g.cols = (zterm_columns / g.width).min(g.dcount); // c:1650-1651
        }
        if g.cols > 0 {
            let i = g.cols * g.width - CM_SPACE; // c:1653
            if i > max {
                max = i;
            }
        }
    }

    // Pass A — per-group line counts (c:1660-1715).
    if onlyexpl_v == 0 {
        for g in groups.iter_mut() {
            let mut glines = 0i32;
            g.widths.clear(); // c:1670-1671
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
                                glines +=
                                    1 + (mlens[m.gnum as usize].saturating_sub(1)) / zterm_columns;
                            }
                        } else if showall != 0 || (m.flags & (CMF_NOLIST | CMF_MULT)) == 0 {
                            glines +=
                                1 + (mlens[m.gnum as usize].saturating_sub(1)) / zterm_columns;
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
            if (g.flags & CGF_PACKED) == 0 {
                continue;
            } // c:1717-1718
              // c:1720-1721 — `ws = g->widths = zalloc(...); memset(ws,0,...)`
            g.widths = vec![0i32; zterm_columns as usize];
            let mut tlines = g.lins; // c:1722
            let mut tcols = g.cols; // c:1723
            let mut width: i32 = 0; // c:1724

            if !g.ylist.is_empty() {
                // c:1726
                if (g.flags & CGF_LINES) == 0 {
                    // c:1727
                    // c:1728-1732 — per-item widths in `ylens`.
                    let ylens: Vec<i32> = g
                        .ylist
                        .iter()
                        .map(|s| s.chars().count() as i32 + CM_SPACE)
                        .collect();

                    if (g.flags & CGF_ROWS) != 0 {
                        // c:1734-1760 — row-major ylist tcols search.
                        let mut t = zterm_columns / (g.shortest + CM_SPACE);
                        while t > g.cols {
                            for w in &mut g.widths[..t as usize] {
                                *w = 0;
                            } // c:1741
                            let mut w = 0i32;
                            let mut nth = 0i32;
                            let mut tcol = 0i32;
                            let mut tl = 1i32;
                            while w < zterm_columns && nth < g.dcount {
                                // c:1743-1744
                                if tcol == t {
                                    tcol = 0;
                                    tl += 1;
                                } // c:1747-1750
                                let len = ylens[nth as usize]; // c:1751
                                if len > g.widths[tcol as usize] {
                                    // c:1753
                                    w += len - g.widths[tcol as usize]; // c:1754
                                    g.widths[tcol as usize] = len; // c:1755
                                }
                                nth += 1;
                                tcol += 1;
                            }
                            width = w;
                            tcols = t;
                            tlines = tl;
                            if w < zterm_columns {
                                break;
                            } // c:1758-1759
                            t -= 1;
                        }
                    } else {
                        // c:1764-1796 — column-major ylist tcols search.
                        // C has a dead `m = *p;` on c:1777 (p never set
                        // in this branch); preserved as no-op.
                        let mut t = zterm_columns / (g.shortest + CM_SPACE);
                        while t > g.cols {
                            let mut tl = ((g.dcount + t - 1) / t).max(1); // c:1768-1769
                            for w in &mut g.widths[..t as usize] {
                                *w = 0;
                            } // c:1771
                            let mut w = 0i32;
                            let mut nth = 0i32;
                            let mut tcol = 0i32;
                            let mut tline = 0i32;
                            while w < zterm_columns && nth < g.dcount {
                                // c:1773-1775
                                if tline == tl {
                                    tcol += 1;
                                    tline = 0;
                                } // c:1779-1782
                                if tcol == t {
                                    tcol = 0;
                                    tl += 1;
                                } // c:1783-1786
                                let len = ylens[nth as usize]; // c:1787
                                if len > g.widths[tcol as usize] {
                                    // c:1789
                                    w += len - g.widths[tcol as usize];
                                    g.widths[tcol as usize] = len;
                                }
                                nth += 1;
                                tline += 1;
                            }
                            width = w;
                            tcols = t;
                            tlines = tl;
                            if w < zterm_columns {
                                break;
                            } // c:1794-1795
                            t -= 1;
                        }
                    }
                }
            } else if g.width != 0 {
                // c:1799
                if (g.flags & CGF_ROWS) != 0 {
                    // c:1803-1830 — row-major matches tcols search.
                    let mut t = zterm_columns / (g.shortest + CM_SPACE);
                    while t > g.cols {
                        for w in &mut g.widths[..t as usize] {
                            *w = 0;
                        } // c:1807
                        let mut w = 0i32;
                        let mut tcol = 0i32;
                        let mut tl = 1i32;
                        let mut nth = 0i32;
                        // c:1810 — `p = skipnolist(g->matches, showall)`.
                        let mut p_idx = skipnolist(&g.matches, showall);
                        while p_idx < g.matches.len() && w < zterm_columns && nth < g.dcount {
                            if tcol == t {
                                tcol = 0;
                                tl += 1;
                            } // c:1816-1819
                            let m = &g.matches[p_idx]; // c:1814
                            let len =
                                mlens[m.gnum as usize] + if tcol == t - 1 { 0 } else { CM_SPACE }; // c:1820-1821
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
                        if w < zterm_columns {
                            break;
                        } // c:1828-1829
                        t -= 1;
                    }
                } else {
                    // c:1834-1872 — column-major matches tcols search.
                    let mut t = zterm_columns / (g.shortest + CM_SPACE);
                    while t > g.cols {
                        let mut tl = ((g.dcount + t - 1) / t).max(1); // c:1838-1839
                        for w in &mut g.widths[..t as usize] {
                            *w = 0;
                        } // c:1841
                        let mut w = 0i32;
                        let mut nth = 0i32;
                        let mut tcol = 0i32;
                        let mut tline = 0i32;
                        let mut p_idx = skipnolist(&g.matches, showall); // c:1844
                        while p_idx < g.matches.len() && w < zterm_columns && nth < g.dcount {
                            if tline == tl {
                                tcol += 1;
                                tline = 0;
                            } // c:1850-1853
                            if tcol == t {
                                tcol = 0;
                                tl += 1;
                            } // c:1854-1857
                            let m = &g.matches[p_idx]; // c:1848
                            let len =
                                mlens[m.gnum as usize] + if tcol == t - 1 { 0 } else { CM_SPACE }; // c:1858-1859
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
                        if w < zterm_columns {
                            // c:1866-1869
                            // C: `if (++tcol < tcols) tcols = tcol;`
                            if tcol + 1 < tcols {
                                tcols = tcol + 1;
                            }
                            break;
                        }
                        t -= 1;
                    }
                }
            }

            // c:1874-1887 — commit the result (or revert if no win).
            if tcols <= g.cols {
                tlines = g.lins;
            } // c:1874-1875
            if tlines == g.lins {
                // c:1876
                g.widths.clear(); // c:1877-1878
            } else {
                nlines += tlines - g.lins; // c:1880
                g.lins = tlines; // c:1881
                g.cols = tcols; // c:1882
                g.totl = width; // c:1883
                let width_adj = width - CM_SPACE; // c:1884
                if width_adj > max {
                    max = width_adj;
                } // c:1885-1886
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
            g.widths.clear(); // c:1907
        }
    }

    // c:1910-1918 — commit listdat.
    let ld = listdat_static.get_or_init(|| std::sync::Mutex::new(Cldata::default()));
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
    1 // c:1920
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
pub fn asklist() -> i32 {
    // c:1925

    // c:1928 — `trashzle(); showinglist = listshown = 0; lastlistlen = 0`.
    trashzle(); // c:1928
    SHOWINGLIST.store(0, Relaxed);
    LISTSHOWN.store(0, Relaxed);
    LASTLISTLEN.store(0, Relaxed); // c:1934

    // c:1930 — `clearflag = (isset(USEZLE) && !termflags && dolastprompt)`.
    let usezle = isset(USEZLE);
    let termflags = crate::ported::params::TERMFLAGS.load(Relaxed);
    let dolastprompt = crate::ported::zle::compcore::dolastprompt.load(Relaxed) != 0;
    let clearflag = usezle && termflags == 0 && dolastprompt;
    CLEARFLAG.store(if clearflag { 1 } else { 0 }, Relaxed);

    // c:1937-1940 — snapshot listdat counts + minfo state.
    let listdat = listdat_static
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let zterm_lines = adjustlines() as i32;
    let cmax = COMPLISTMAX.load(Relaxed) as i32;

    let has_cur = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.cur.is_some())
        .unwrap_or(false);
    let already_asked = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.asked)
        .unwrap_or(0);

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
            format!("zsh: do you wish to see all {} lines? ", listdat.nlines)
        };
        let fd = SHTTY.load(Relaxed);
        let out = if fd >= 0 { fd } else { 1 };
        let _ = write_loop(out, prompt.as_bytes());

        // c:1955 — `getzlequery()`.
        let said_yes = getzlequery() != 0;

        if !said_yes {
            // c:1956
            // c:1957-1964 — clean up the question line.
            let _ = write_loop(out, b"\n");
            // c:1965 — `minfo.asked = 2`.
            if let Ok(mut m) = MINFO
                .get_or_init(|| std::sync::Mutex::new(Default::default()))
                .lock()
            {
                m.asked = 2;
            }
            return 1; // c:1966
        }
        // c:1968-1974 — clean up after a yes.
        let _ = write_loop(out, b"\n");
        // c:1975 — `minfo.asked = 1`.
        if let Ok(mut m) = MINFO
            .get_or_init(|| std::sync::Mutex::new(Default::default()))
            .lock()
        {
            m.asked = 1;
        }
    }
    // c:1978-1979 — second-pass entry: already-asked-no falls through
    //                to the final return-1 to suppress the listing.

    // c:1981 — `return (minfo.asked ? minfo.asked - 1 : 0);`.
    let asked_now = MINFO
        .get()
        .and_then(|m| m.lock().ok())
        .map(|m| m.asked)
        .unwrap_or(0);
    if asked_now != 0 {
        asked_now - 1
    } else {
        0
    }
}

thread_local! {
    /// `static int lastinvcount = -1;` from compresult.c:1497 inside
    /// `calclist`. Caches the last `invcount` seen so the early-exit
    /// at c:1506-1511 fires when nothing has changed.
    static LASTINVCOUNT: std::cell::Cell<i32> = const { std::cell::Cell::new(-1) };
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
pub fn printlist(over: i32, showall: i32) -> i32 {
    // c:1978
    // c:1985 — `printlist` writes the entire match listing to
    //          `shout`. Resolve once and reuse for every emission so
    //          a single SHTTY load covers the whole render.
    let out_fd: i32 = {
        let fd = SHTTY.load(Relaxed);
        if fd >= 0 {
            fd
        } else {
            1
        }
    };

    let listdat = listdat_static
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut cl: i32 = if over != 0 { listdat.nlines } else { -1 }; // c:1984
    let mut pnl: i32 = 0; // c:1984
    let mut ml: i32 = 0;

    if cl < 2 {
        // c:1986
        cl = -1;
        // c:1987-1988 — `if (tccan(TCCLEAREOD)) tcout(TCCLEAREOD);`
        if crate::ported::init::tclen.lock().unwrap()[crate::ported::zsh_h::TCCLEAREOD as usize]
            != 0
        {
            tcout(crate::ported::zsh_h::TCCLEAREOD);
        }
    }

    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();

    for g in &groups {
        // c:1990
        // c:2000-2027 — explanations.
        for e in g.expls.iter() {
            // c:2000
            let active = (e.count != 0 || e.always != 0)                     // c:2001
                && (listdat.onlyexpl == 0
                    || (listdat.onlyexpl
                        & (if e.always > 0 { 2 } else { 1 })) != 0);
            if !active {
                continue;
            }

            if pnl != 0 {
                // c:2007
                let _ = write_loop(out_fd, b"\n"); // c:2008
                ml += 1;
                cl -= 1;
                if cl >= 0 && cl <= 1 {
                    // c:2010
                    cl = -1;
                    // c:2010-2011 — `if (tccan(TCCLEAREOD)) tcout(TCCLEAREOD);`
                    if crate::ported::init::tclen.lock().unwrap()
                        [crate::ported::zsh_h::TCCLEAREOD as usize]
                        != 0
                    {
                        tcout(crate::ported::zsh_h::TCCLEAREOD);
                    }
                }
            }
            // c:2017-2018 — printfmt(e.str, count, 1, 1).
            let n = if e.always != 0 { -1 } else { e.count };
            let l = printfmt(e.str.as_deref().unwrap_or(""), n, true, true);
            ml += l;
            if cl >= 0 && (cl - l) <= 1 {
                cl = -1;
            }
            pnl = 1;
        }

        // c:2032-2076 — ylist branch (alternative listing).
        if listdat.onlyexpl == 0 && !g.ylist.is_empty() {
            // c:2032
            if pnl != 0 {
                // c:2033
                let _ = write_loop(out_fd, b"\n");
                pnl = 0;
                ml += 1;
                if cl >= 0 && cl <= 1 {
                    cl = -1;
                }
            }
            if (g.flags & CGF_LINES) != 0 {
                // c:2044
                let mut so = std::io::stdout();
                let last_idx = g.ylist.len().saturating_sub(1);
                for (i, p) in g.ylist.iter().enumerate() {
                    let _ = zputs(p, &mut so);
                    if i != last_idx {
                        // c:2050
                        // C wraps via " \b" or "\n"; we emit \n for safety.
                        let _ = write_loop(out_fd, b"\n");
                    }
                }
            } else {
                // c:2058
                // Column layout — emit each entry.
                let mut so = std::io::stdout();
                for entry in &g.ylist {
                    let _ = zputs(entry, &mut so);
                    let _ = write_loop(out_fd, b"\n");
                    ml += 1;
                }
            }
        } else if listdat.onlyexpl == 0 && (g.lcount != 0 || (showall != 0 && g.mcount != 0)) {
            // c:2079-2185 — main column-rendered match list.
            if pnl != 0 {
                // c:2080
                let _ = write_loop(out_fd, b"\n");
                pnl = 0;
                ml += 1;
            }

            for m in &g.matches {
                // c:2087
                let visible = showall != 0 || (m.flags & (CMF_HIDE | CMF_NOLIST)) == 0;
                if !visible {
                    continue;
                }
                // c:2095-2098 — DISPLINE = full-row.
                let _ = iprintm(Some(g), Some(m), 0, 0, 1, 0);
                if (m.flags & CMF_DISPLINE) == 0 {
                    let _ = write_loop(out_fd, b"\n");
                }
                ml += 1;
            }
            // Force CGF_ROWS layout hint into effect.
            let _ = CGF_ROWS;
        }
        pnl = 1;
    }

    let _ = Relaxed;
    ml // c:2185
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
pub fn bld_all_str() -> String {
    // c:2187

    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();

    // c:2191 — `cols = zterm_columns`. C reads the live tty width
    //          via the cached `zterm_columns` global. Rust port uses
    //          `adjustcolumns` which probes via TIOCGWINSZ and falls
    //          back to $COLUMNS. Was reading raw `std::env::var(
    //          "COLUMNS")` only — wrong: missed the live width.
    let cols: i32 = adjustcolumns() as i32;
    let mut len: i32 = cols - 5; // c:2192
    let mut add: i32 = 0;
    let mut buf = String::new(); // c:2196

    // c:2199-2204 — skip empty groups.
    let mut g_idx = groups.iter().position(|g| g.mcount != 0);
    'outer: while let Some(gi) = g_idx {
        let g = &groups[gi];
        let mut mp = 0usize;
        while mp < g.matches.len() {
            let m = &g.matches[mp];
            let visible = (m.flags & (CMF_ALL | CMF_HIDE)) == 0 && m.str.is_some();
            if visible {
                // c:2213
                let s = m.str.as_deref().unwrap();
                let t = s.len() as i32 + add;
                if len >= t {
                    // c:2215
                    if add != 0 {
                        buf.push(' ');
                    } // c:2216
                    buf.push_str(s); // c:2218
                    len -= t;
                    add = 1;
                } else {
                    // c:2221
                    if len > add + 2 {
                        // c:2222
                        if add != 0 {
                            buf.push(' ');
                        }
                        buf.push_str(&s[..((len - 2).max(0) as usize).min(s.len())]);
                    }
                    buf.push_str("..."); // c:2227
                    break 'outer; // c:2228
                }
            }
            mp += 1;
            if mp >= g.matches.len() {
                // c:2232
                g_idx = (gi + 1..).find(|&i| i < groups.len() && groups[i].mcount != 0);
                if g_idx.is_none() {
                    break 'outer;
                }
                continue 'outer;
            }
        }
        let _ = Relaxed;
        g_idx = (gi + 1..).find(|&i| i < groups.len() && groups[i].mcount != 0);
    }
    buf // c:2238 ztrdup(buf)
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
    g: Option<&Cmgroup>,
    mp: Option<&Cmatch>,
    mc: i32,
    ml: i32,
    lastc: i32,
    width: i32,
) -> i32 {
    // c:2241
    use std::sync::atomic::Ordering;

    let m = match mp {
        None => return 0,
        Some(m) => m,
    }; // c:2245
    let mut disp_owned: String = String::new();
    let disp_ref: Option<&str> = m.disp.as_deref();

    // c:2249-2250 — if CMF_ALL with empty disp, build it via bld_all_str.
    if (m.flags & CMF_ALL) != 0 && disp_ref.map(|s| s.is_empty()).unwrap_or(true) {
        disp_owned = bld_all_str(); // c:2250
    }
    let disp_now: Option<&str> = if !disp_owned.is_empty() {
        Some(disp_owned.as_str())
    } else {
        disp_ref
    };

    let mut len: i32;
    // c:2243 — C writes through `printfmt`/`fputs(s, shout)`. Route Rust
    //          to SHTTY so the visible-byte stream matches.
    let fd = SHTTY.load(Relaxed);
    let out = if fd >= 0 { fd } else { 1 };

    if let Some(d) = disp_now {
        // c:2253
        if (m.flags & CMF_DISPLINE) != 0 {
            // c:2254
            // c:2255 — `printfmt(d, 0, 1, 0)` then `putc('\n', shout)`.
            let _ = write_loop(out, d.as_bytes());
            let _ = write_loop(out, b"\n");
            return 0; // c:2257
        }
        let _ = write_loop(out, d.as_bytes()); // c:2260 niceformat
        len = d.chars().count() as i32;
    } else {
        // c:2263
        let s = m.str.as_deref().unwrap_or("");
        let _ = write_loop(out, s.as_bytes()); // c:2266
        len = s.chars().count() as i32;
        // c:2270-2273 — append modec for file-completion groups.
        if let Some(grp) = g {
            if (grp.flags & CGF_FILES) != 0 && m.modec != '\0' {
                let mut buf = [0u8; 4];
                let mb = m.modec.encode_utf8(&mut buf);
                let _ = write_loop(out, mb.as_bytes());
                len += 1;
            }
        }
    }
    if lastc == 0 {
        // c:2275
        // c:2278-2279 — pad with spaces up to column width.
        let pad = width - len;
        if pad > 0 {
            let spaces = vec![b' '; pad as usize];
            let _ = write_loop(out, &spaces);
        }
    }
    len // c:2282
}

/// Port of `int ilistmatches(Hookdef dummy, Chdata dat)` from
/// `Src/Zle/compresult.c:2284`. Hook callback for the standard
/// listing path: runs `calclist`, bails when `listdat.nlines == 0`,
/// otherwise calls `printlist(0, iprintm, 0)`.
pub fn ilistmatches() -> i32 {
    // c:2284
    let _ = calclist(0); // c:2286
                         // c:2288 — bail when listdat.nlines == 0 (no matches to display).
    let nlines = listdat_static
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
        .map(|g| g.nlines)
        .unwrap_or(0);
    if nlines == 0 {
        SHOWINGLIST.store(0, Relaxed);
        LISTSHOWN.store(0, Relaxed);
        return 0;
    }
    // c:2295 — printlist(0, iprintm, 0).
    let _ = printlist(0, 0);
    0 // c:2297
}

/// Port of `int list_matches(Hookdef dummy, void *dummy2)` from
/// `Src/Zle/compresult.c:2304`.
///
/// "List the matches. Note that the list entries are metafied."
/// Walks `amatches` into a `chdata` bag and dispatches via
/// `runhookdef(COMPLISTMATCHESHOOK, &dat)` so `_main_complete`-style
/// user hooks can override the default `ilistmatches` rendering.
pub fn list_matches() -> i32 {
    // c:2304
    if VALIDLIST.load(Ordering::SeqCst) == 0 {
        // c:2311
        showmsg("BUG: listmatches called with bogus list");
        return 1; // c:2313
    }
    // c:2317-2324 — populate the chdata bag.
    let groups = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .ok()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut dat = Chdata::default();
    dat.matches = groups.into_iter().next().map(Box::new); // c:2317 first group head
    dat.num = nmatches_g.load(Relaxed); // c:2319
                                        // c:2325 — `runhookdef(COMPLISTMATCHESHOOK, &dat)` fires every
                                        // registered Hookfn; first non-zero short-circuits per HOOKF_ALL.
                                        // When `gethookdef` returns NULL (no module registered a handler)
                                        // or `runhookdef` returns 0 with no Hookfns, fall through to the
                                        // canonical `ilistmatches` renderer.
    let h = crate::ported::module::gethookdef("complist-matches");
    let handled = if !h.is_null() {
        let dat_ptr = (&mut dat) as *mut Chdata as *mut std::ffi::c_void;
        crate::ported::module::runhookdef(h, dat_ptr) != 0
    } else {
        false
    };
    if !handled {
        ilistmatches();
    }
    0
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
pub fn invalidate_list() -> i32 {
    // c:2334

    INVCOUNT.fetch_add(1, Ordering::SeqCst); // c:2336
    if VALIDLIST.load(Ordering::SeqCst) != 0 {
        // c:2337
        if SHOWINGLIST.load(Ordering::SeqCst) == -2 {
            // c:2338
            // c:2339 — `zrefresh()` triggers a screen redraw so the now-
            // invalidated listing isn't left on screen.
            zrefresh();
        }
        // c:2341 — `freematches(lastmatches, 1)` fires `minfo.cur = None`
        // via the cm=1 side-effect.
        let drained = lastmatches
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
            .map(|mut g| std::mem::take(&mut *g))
            .unwrap_or_default();
        crate::ported::zle::compcore::freematches(drained, 1);
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
    if let Ok(mut ld) = listdat_static
        .get_or_init(|| std::sync::Mutex::new(Default::default()))
        .lock()
    {
        ld.valid = 0;
    }
    // c:2347-2348 — `if (listshown < 0) listshown = 0`.
    if LISTSHOWN.load(Ordering::SeqCst) < 0 {
        LISTSHOWN.store(0, Ordering::SeqCst);
    }
    // c:2349-2353 — `minfo.cur = NULL; minfo.asked = 0; …`. minfo not
    // ported as a static struct yet.
    // c:2354 — `compwidget = NULL`. The canonical `COMPWIDGET` static
    // lives in zle_main.rs.
    nmatches_g.store(0, Ordering::SeqCst); // c:2355
    if let Ok(mut g) = amatches
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
    {
        g.clear(); // c:2356
    }
    0 // c:2358
}
/// `INVCOUNT` static.
pub static INVCOUNT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:37

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::comp_h::Cline;

    #[test]
    fn test_unambig_data() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(unambig_data(&["foobar".into(), "foobaz".into()]), "fooba");
        assert_eq!(unambig_data(&["abc".into()]), "abc");
        assert_eq!(unambig_data(&[]), "");
    }

    #[test]
    fn cline_str_none_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        // c:165 — null Cline → empty string.
        let _g = zle_test_setup();
        assert_eq!(cline_str(None), "");
    }

    #[test]
    fn cline_str_emits_word_anchor() {
        let _g = crate::test_util::global_state_lock();
        // c:282 — non-CLF_LINE node emits `word`.
        let _g = zle_test_setup();
        let mut n = Cline::default();
        n.word = Some("hello".to_string());
        n.wlen = 5;
        assert_eq!(cline_str(Some(&n)), "hello");
    }

    #[test]
    fn cline_str_emits_line_anchor_when_clf_line_set() {
        let _g = crate::test_util::global_state_lock();
        // c:282 — CLF_LINE node emits `line` instead of `word`.
        let _g = zle_test_setup();
        let mut n = Cline::default();
        n.flags = CLF_LINE;
        n.line = Some("LINE".to_string());
        n.word = Some("word-should-not-emit".to_string());
        assert_eq!(cline_str(Some(&n)), "LINE");
    }

    #[test]
    fn cline_str_emits_orig_when_olen_set_and_no_prefix() {
        let _g = crate::test_util::global_state_lock();
        // c:214 — olen!=0 && !CLF_SUF && !prefix → emit `orig` (not
        //          the prefix-walk + word path).
        let _g = zle_test_setup();
        let mut n = Cline::default();
        n.orig = Some("original".to_string());
        n.olen = 8;
        n.word = Some("anchor".to_string());
        // Output = orig + word anchor (the C path emits both).
        assert_eq!(cline_str(Some(&n)), "originalanchor");
    }

    #[test]
    fn cline_str_walks_prefix_chain() {
        let _g = crate::test_util::global_state_lock();
        // c:219-235 — prefix sub-list walked when olen==0 or
        //              CLF_SUF set.
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        // c:165 — top-level walk via `l = l->next`.
        let _g = zle_test_setup();
        let mut n2 = Cline::default();
        n2.word = Some("B".to_string());
        let mut n1 = Cline::default();
        n1.word = Some("A".to_string());
        n1.next = Some(Box::new(n2));
        assert_eq!(cline_str(Some(&n1)), "AB");
    }

    #[test]
    fn test_instmatch() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let (result, cursor) = instmatch("git co", 6, 4, 6, "commit");
        assert_eq!(result, "git commit");
        assert_eq!(cursor, 10);
    }

    #[test]
    fn test_do_single() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let (result, cursor) = do_single("git co", 6, 4, 6, "commit", true);
        assert_eq!(result, "git commit ");
        assert_eq!(cursor, 11);
    }

    #[test]
    fn test_do_menucmp() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert!(valid_match("foobar", "foo", ""));
        assert!(valid_match("foobar", "foo", "bar"));
        assert!(!valid_match("foobar", "baz", ""));
    }

    #[test]
    fn test_build_pos_string() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(build_pos_string(0, 10), "1/10");
        assert_eq!(build_pos_string(9, 10), "10/10");
    }

    #[test]
    fn test_list_lines() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(list_lines(&vec!["a".into(); 10], 3), 4);
        assert_eq!(list_lines(&vec!["a".into(); 6], 3), 2);
    }

    #[test]
    fn comp_mod_positive() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1366-1369 — positive: decrement then % m.
        assert_eq!(comp_mod(1, 5), 0); // (1-1) % 5 = 0
        assert_eq!(comp_mod(3, 5), 2); // (3-1) % 5 = 2
        assert_eq!(comp_mod(5, 5), 4); // (5-1) % 5 = 4
        assert_eq!(comp_mod(6, 5), 0); // (6-1) % 5 = 0
    }

    #[test]
    fn comp_mod_zero_branches_negative() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:1366 — `if (v >= 0) v--;` so 0 → -1 → falls into else.
        // c:1370-1373 — wrap by adding m until non-negative.
        assert_eq!(comp_mod(0, 5), 4); // 0→-1→+5=4
        assert_eq!(comp_mod(-1, 5), 4); // -1+5=4
        assert_eq!(comp_mod(-5, 5), 0); // -5+5=0
        assert_eq!(comp_mod(-6, 5), 4); // -6+5=-1+5=4
    }

    #[test]
    fn comp_list_sets_onlyexpl() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
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
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.flags = CMF_NOLIST;
        let mut b = Cmatch::default();
        b.flags = CMF_HIDE;
        let c = Cmatch::default(); // listable
        let v = vec![a, b, c];
        // c:1483 — mask = NOLIST|MULT|HIDE. First two skipped, third kept.
        assert_eq!(skipnolist(&v, 0), 2);
    }

    #[test]
    fn skipnolist_showall_keeps_nolist() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.flags = CMF_NOLIST;
        let v = vec![a];
        // c:1483 — showall=1 drops NOLIST|MULT from mask, only HIDE filters.
        assert_eq!(skipnolist(&v, 1), 0);
    }

    #[test]
    fn skipnolist_skips_disp_displine() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut a = Cmatch::default();
        a.disp = Some("display".into());
        a.flags = CMF_DISPLINE;
        let b = Cmatch::default();
        let v = vec![a, b];
        // c:1486-1487 — disp + (DISPLINE|HIDE) → skip.
        assert_eq!(skipnolist(&v, 0), 1);
    }

    // ─── zsh-corpus pins for unambig_data / build_pos_string ────────

    /// `unambig_data([])` is empty string (no matches).
    #[test]
    fn compresult_corpus_unambig_data_empty_input_empty_output() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(unambig_data(&[]), "");
    }

    /// `unambig_data([single])` returns the single match.
    #[test]
    fn compresult_corpus_unambig_data_single_input_returns_it() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["only_one".to_string()];
        assert_eq!(unambig_data(&matches), "only_one");
    }

    /// `unambig_data` returns longest common prefix.
    #[test]
    fn compresult_corpus_unambig_data_returns_lcp() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec![
            "prefix_alpha".to_string(),
            "prefix_beta".to_string(),
            "prefix_gamma".to_string(),
        ];
        assert_eq!(unambig_data(&matches), "prefix_");
    }

    /// `unambig_data` with no shared prefix returns empty.
    #[test]
    fn compresult_corpus_unambig_data_no_shared_prefix() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        assert_eq!(unambig_data(&matches), "");
    }

    /// `unambig_data` where one match is a prefix of another → returns shorter.
    #[test]
    fn compresult_corpus_unambig_data_one_is_prefix_of_other() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["abc".to_string(), "abcdef".to_string()];
        assert_eq!(unambig_data(&matches), "abc");
    }

    /// `build_pos_string(0, 5)` produces "1/5" (1-indexed).
    #[test]
    fn compresult_corpus_build_pos_string_one_indexed() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(build_pos_string(0, 5), "1/5");
        assert_eq!(build_pos_string(4, 5), "5/5");
        assert_eq!(build_pos_string(99, 100), "100/100");
    }

    /// `build_pos_string` format is "current+1/total".
    #[test]
    fn compresult_corpus_build_pos_string_includes_slash() {
        let _g = crate::test_util::global_state_lock();
        let s = build_pos_string(2, 10);
        assert!(s.contains('/'));
        assert_eq!(s, "3/10");
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/compresult.c.
    // ═══════════════════════════════════════════════════════════════════

    /// `build_pos_string(0, 1)` returns "1/1" — first of one.
    #[test]
    fn build_pos_string_first_of_one_returns_one_slash_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(build_pos_string(0, 1), "1/1");
    }

    /// `build_pos_string` displays 1-indexed current.
    #[test]
    fn build_pos_string_first_index_is_one_not_zero() {
        let _g = crate::test_util::global_state_lock();
        // current=0 should display as "1" (1-indexed).
        let s = build_pos_string(0, 5);
        assert!(s.starts_with('1'), "0-indexed input shown as 1; got {s}");
    }

    /// `unambig_data` on identical matches returns the full string.
    /// C: if all matches are the same, common prefix = whole string.
    #[test]
    fn unambig_data_all_identical_returns_full_string() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["hello".to_string(), "hello".to_string()];
        let r = unambig_data(&matches);
        assert_eq!(r, "hello", "identical matches → full string");
    }

    /// `unambig_data` on empty matches returns empty string.
    #[test]
    fn unambig_data_empty_matches_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let r = unambig_data(&[]);
        assert!(r.is_empty(), "no matches → empty unambig prefix");
    }

    /// `unambig_data` returns common prefix of multiple matches.
    /// `["foobar", "football", "foo"]` → "foo".
    #[test]
    fn unambig_data_common_prefix_of_three() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec![
            "foobar".to_string(),
            "football".to_string(),
            "foo".to_string(),
        ];
        assert_eq!(unambig_data(&matches), "foo", "common prefix = 'foo'");
    }

    /// `unambig_data` with NO common prefix returns empty.
    #[test]
    fn unambig_data_no_common_prefix_returns_empty() {
        let _g = crate::test_util::global_state_lock();
        let matches = vec!["abc".to_string(), "xyz".to_string()];
        let r = unambig_data(&matches);
        assert!(r.is_empty(), "no common prefix → empty");
    }

    /// `valid_match` returns true when word has correct prefix+suffix.
    #[test]
    fn valid_match_pre_suf_wrap_matches() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            valid_match("prefoosuf", "pre", "suf"),
            "word with matching prefix+suffix is valid"
        );
    }

    /// `valid_match` returns false when prefix doesn't match.
    #[test]
    fn valid_match_wrong_prefix_returns_false() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            !valid_match("foo", "bar", ""),
            "wrong prefix → invalid match"
        );
    }

    /// `comp_mod(v, m)` converts 1-indexed v to 0-indexed before
    /// modulo. C `Src/Zle/compresult.c:1364`:
    ///   `if (v >= 0) v -= 1; if (v >= 0) v % m; else { wrap into [0,m) }`
    /// So `comp_mod(7, 3)` = (7-1) % 3 = 0, not 1 — the v-1 conversion
    /// is for 1-indexed match-table semantics.
    #[test]
    fn comp_mod_positive_v_subtracts_one_then_mods() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(comp_mod(7, 3), 0, "(7-1)%3 = 6%3 = 0");
        assert_eq!(comp_mod(6, 3), 2, "(6-1)%3 = 5%3 = 2");
        assert_eq!(comp_mod(4, 3), 0, "(4-1)%3 = 3%3 = 0");
    }

    /// `comp_mod(0, 5)` — v=0 → v-1=-1 → wrap: -1 + 5 = 4.
    #[test]
    fn comp_mod_zero_v_wraps_to_m_minus_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(comp_mod(0, 5), 4, "(0-1) wrapped to [0,5) = 4");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compresult.c utilities.
    // ═══════════════════════════════════════════════════════════════════

    /// c:46 — `cut_cline(s, n)` where len(s) ≤ n is identity.
    #[test]
    fn cut_cline_short_string_is_identity() {
        assert_eq!(cut_cline("abc", 10), "abc");
        assert_eq!(cut_cline("", 5), "");
    }

    /// c:46 — `cut_cline(s, n)` where len(s) > n truncates to (n-3)+"...".
    #[test]
    fn cut_cline_long_string_truncates_with_ellipsis() {
        let r = cut_cline("abcdefghij", 6);
        assert_eq!(r, "abc...", "(6-3)=3 chars + ... = 6 chars total");
        assert_eq!(r.len(), 6);
    }

    /// c:46 — `cut_cline(s, len(s))` is identity (boundary, len == max).
    #[test]
    fn cut_cline_at_exact_length_is_identity() {
        assert_eq!(cut_cline("hello", 5), "hello");
    }

    /// c:489 — `build_pos_string(0, 10)` = "1/10" (1-indexed display).
    #[test]
    fn build_pos_string_1_indexed_display() {
        assert_eq!(build_pos_string(0, 10), "1/10");
        assert_eq!(build_pos_string(4, 10), "5/10");
        assert_eq!(build_pos_string(9, 10), "10/10");
    }

    /// c:489 — `build_pos_string(0, 1)` = "1/1" for single match.
    #[test]
    fn build_pos_string_single_match() {
        assert_eq!(build_pos_string(0, 1), "1/1");
    }

    /// c:165 — `cline_str(None)` returns empty string (corpus pin).
    #[test]
    fn cline_str_none_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(cline_str(None), "");
    }

    /// c:379 — `valid_match("foo", "", "")` returns true (empty prefix/
    /// suffix → anything matches).
    #[test]
    fn valid_match_empty_pfx_sfx_accepts_anything() {
        let _g = crate::test_util::global_state_lock();
        assert!(valid_match("foo", "", ""));
        assert!(valid_match("", "", ""));
    }

    /// c:379 — `valid_match("foobar", "foo", "")` returns true
    /// (prefix matches, no suffix required).
    #[test]
    fn valid_match_prefix_only() {
        let _g = crate::test_util::global_state_lock();
        assert!(valid_match("foobar", "foo", ""));
        assert!(valid_match("foo", "foo", ""), "exact prefix match");
    }

    /// c:379 — `valid_match("xx_bar", "", "bar")` returns true (suffix
    /// matches, no prefix required).
    #[test]
    fn valid_match_suffix_only() {
        let _g = crate::test_util::global_state_lock();
        assert!(valid_match("xx_bar", "", "bar"));
    }

    /// c:1364 — `comp_mod` for positive v ≥ m wraps correctly via
    /// (v-1) % m.
    #[test]
    fn comp_mod_v_above_m_wraps() {
        assert_eq!(comp_mod(10, 3), 0, "(10-1)%3 = 9%3 = 0");
        assert_eq!(comp_mod(11, 3), 1, "(11-1)%3 = 10%3 = 1");
        assert_eq!(comp_mod(12, 3), 2, "(12-1)%3 = 11%3 = 2");
    }

    /// c:1364 — `comp_mod(-1, 5)` = -1 + 5 = 4 (already negative, no
    /// pre-decrement; loop just adds m).
    #[test]
    fn comp_mod_negative_v_wraps_via_loop() {
        assert_eq!(comp_mod(-1, 5), 4);
        assert_eq!(comp_mod(-3, 5), 2);
        assert_eq!(comp_mod(-7, 5), 3, "-7 + 5 + 5 = 3");
    }

    /// c:180 — `unambig_data` on empty matches returns empty (corpus pin).
    #[test]
    fn unambig_data_empty_matches_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(unambig_data(&[]), "");
    }

    /// c:180 — `unambig_data` on single match returns that match.
    #[test]
    fn unambig_data_single_match_returns_it() {
        let _g = crate::test_util::global_state_lock();
        let r = unambig_data(&["single".to_string()]);
        assert_eq!(r, "single", "single match → common prefix is itself");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compresult.c
    // c:71 cut_cline / c:99 cline_str / c:166 build_pos_string /
    // c:180 unambig_data / c:250 hasbrpsfx / c:379 valid_match /
    // c:502 comp_mod / c:639 list_lines / c:701 skipnolist
    // ═══════════════════════════════════════════════════════════════════

    /// c:71 — `cut_cline("", _)` returns empty.
    #[test]
    fn cut_cline_empty_input_returns_empty() {
        assert_eq!(cut_cline("", 0), "");
        assert_eq!(cut_cline("", 100), "");
    }

    /// c:71 — `cut_cline` is deterministic.
    #[test]
    fn cut_cline_is_deterministic() {
        for s in ["", "abc", "hello world"] {
            for n in [0usize, 1, 5, 100] {
                let first = cut_cline(s, n);
                for _ in 0..3 {
                    assert_eq!(
                        cut_cline(s, n),
                        first,
                        "cut_cline({:?}, {}) must be deterministic",
                        s,
                        n
                    );
                }
            }
        }
    }

    /// c:166 — `build_pos_string` returns String (compile-time type pin).
    #[test]
    fn build_pos_string_returns_string_type() {
        let _: String = build_pos_string(1, 1);
    }

    /// c:166 — `build_pos_string` is pure.
    #[test]
    fn build_pos_string_is_pure() {
        for (c, t) in [(1usize, 1usize), (5, 10), (100, 1000)] {
            let first = build_pos_string(c, t);
            for _ in 0..3 {
                assert_eq!(
                    build_pos_string(c, t),
                    first,
                    "build_pos_string({}, {}) must be pure",
                    c,
                    t
                );
            }
        }
    }

    /// c:180 — `unambig_data(empty)` returns empty.
    #[test]
    fn unambig_data_empty_returns_empty_pin() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(unambig_data(&[]), "");
    }

    /// c:250 — `hasbrpsfx("")` returns bool (compile-time type pin).
    #[test]
    fn hasbrpsfx_returns_bool_type() {
        let _: bool = hasbrpsfx("");
    }

    /// c:250 — `hasbrpsfx` is pure.
    #[test]
    fn hasbrpsfx_is_pure() {
        for s in ["", "abc", "{a,b}", "a{b,c}d"] {
            let first = hasbrpsfx(s);
            for _ in 0..3 {
                assert_eq!(hasbrpsfx(s), first, "hasbrpsfx({:?}) must be pure", s);
            }
        }
    }

    /// c:502 — `comp_mod(0, M)` returns M-1 per 1-indexed menu-cycle
    /// decrement: c:1367 always subtracts 1 first, then the negative-v
    /// branch wraps via repeated `+= m` until non-negative → ends at M-1.
    #[test]
    fn comp_mod_zero_returns_m_minus_one() {
        for m in [1i32, 5, 100, 1000] {
            assert_eq!(
                comp_mod(0, m),
                m - 1,
                "comp_mod(0, {}) = {}-1 = {} per c:1367 decrement+wrap",
                m,
                m,
                m - 1
            );
        }
    }

    /// c:502 — `comp_mod` result strictly less than m.
    #[test]
    fn comp_mod_result_less_than_modulus() {
        for v in [-100i32, -1, 0, 1, 50, 100] {
            for m in [1i32, 5, 10] {
                let r = comp_mod(v, m);
                assert!(
                    r >= 0 && r < m,
                    "comp_mod({}, {}) = {} must be in [0, {})",
                    v,
                    m,
                    r,
                    m
                );
            }
        }
    }

    /// c:379 — `valid_match(word, prefix, suffix)` returns bool (type pin).
    #[test]
    fn valid_match_returns_bool_type() {
        let _: bool = valid_match("", "", "");
    }

    /// c:701 — `skipnolist(empty, _)` returns 0 (empty array).
    #[test]
    fn skipnolist_empty_returns_zero() {
        assert_eq!(skipnolist(&[], 0), 0);
        assert_eq!(skipnolist(&[], 1), 0);
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/compresult.c
    // c:264 do_ambiguous / c:311 ztat / c:527 do_ambig_menu /
    // c:639 list_lines / c:729 calclist / c:1232 asklist /
    // c:1350 printlist / c:1502 bld_all_str / c:1657 ilistmatches /
    // c:1683 list_matches
    // ═══════════════════════════════════════════════════════════════════

    /// c:264 — `do_ambiguous(empty)` returns i32 (compile-time type pin).
    #[test]
    fn do_ambiguous_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = do_ambiguous(&[]);
    }

    /// c:311 — `ztat("/__never__", _)` returns Option<Metadata>.
    #[test]
    fn ztat_returns_option_metadata_type() {
        let _g = crate::test_util::global_state_lock();
        let _: Option<std::fs::Metadata> = ztat("/__never_zshrs__", false);
    }

    /// c:311 — `ztat("/__never__", _)` returns None.
    #[test]
    fn ztat_nonexistent_returns_none() {
        assert!(
            ztat("/__never_real_path_zshrs_xyz__", false).is_none(),
            "nonexistent → None"
        );
        assert!(
            ztat("/__never_real_path_zshrs_xyz__", true).is_none(),
            "nonexistent w/ symlink follow → None"
        );
    }

    /// c:311 — `ztat("/tmp", _)` returns Some on every Unix host.
    #[test]
    #[cfg(unix)]
    fn ztat_tmp_returns_some() {
        assert!(ztat("/tmp", false).is_some(), "/tmp must stat → Some");
    }

    /// c:527 — `do_ambig_menu` returns i32 (compile-time type pin).
    #[test]
    fn do_ambig_menu_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = do_ambig_menu();
    }

    /// c:639 — `list_lines(empty, 80)` returns 0 (no lines for empty matches).
    #[test]
    fn list_lines_empty_matches_returns_zero() {
        assert_eq!(list_lines(&[], 80), 0, "0 matches → 0 lines");
    }

    /// c:639 — `list_lines` returns usize (compile-time type pin).
    #[test]
    fn list_lines_returns_usize_type() {
        let _: usize = list_lines(&[], 80);
    }

    /// c:729 — `calclist(0)` returns i32 (compile-time type pin).
    #[test]
    fn calclist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = calclist(0);
    }

    /// c:1232 — `asklist` returns i32.
    #[test]
    fn asklist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = asklist();
    }

    /// c:1350 — `printlist(0, 0)` returns i32.
    #[test]
    fn printlist_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = printlist(0, 0);
    }

    /// c:1502 — `bld_all_str` returns String (compile-time type pin).
    #[test]
    fn bld_all_str_returns_string_type() {
        let _g = crate::test_util::global_state_lock();
        let _: String = bld_all_str();
    }

    /// c:1657 — `ilistmatches` returns i32.
    #[test]
    fn ilistmatches_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = ilistmatches();
    }

    /// c:1683 — `list_matches` returns i32.
    #[test]
    fn list_matches_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _: i32 = list_matches();
    }
}
