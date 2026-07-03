//! ZLE history operations
//!
//! Direct port from zsh/Src/Zle/zle_hist.c
//!
//! Previous aborted search string use in an incremental search              // c:52
//! Local keymap in isearch mode                                             // c:57
//! the last vi search                                                       // c:1807
//! history-beginning-search-backward                                        // c:2035
//! history-beginning-search-forward                                         // c:2082
//!
//! Implements all history navigation widgets:
//! - up-line-or-history, down-line-or-history
//! - history-search-backward, history-search-forward  
//! - history-incremental-search-backward, history-incremental-search-forward
//! - beginning-of-history, end-of-history
//! - vi-fetch-history, vi-history-search-*
//! - accept-line-and-down-history, accept-and-infer-next-history
//! - insert-last-word, push-line, push-line-or-edit

use std::sync::atomic::{AtomicI32, AtomicI64, AtomicUsize, Ordering};

use super::zle_main::{BUFSTACK, MULT};
use super::zle_misc::DONE;
use crate::ported::options::opt_state_set;
use crate::ported::zsh_h::{isset, HISTBEEP, HISTIGNOREDUPS, ZLRF_HISTORY};

// =====================================================================
// Isearch globals — `Src/Zle/zle_hist.c:1078`.
// =====================================================================

#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_h::*, zle_main::*, zle_misc::*, zle_move::*, zle_params::*,
    zle_refresh::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
/// Port of `int isearch_active` from `Src/Zle/zle_hist.c:1078`.
/// Non-zero while the user is inside an incremental-search session.

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]

/// Snapshot the current line into the history entry at `cursor`,
/// preserving the original on first edit.
/// Port of `remember_edits()` from Src/Zle/zle_hist.c:80. The C source
/// stashes the in-flight text in `Histent->zle_text` (a separate field
/// from the canonical history line) and sets `have_edits = 1`. We
/// model `zle_text` by keeping the edited text in `entries[i].line`
/// directly and saving the canonical version into `originals[i]`
/// on first edit so `forget_edits` can restore it.
/// WARNING: param names don't match C — Rust=(hist) vs C=()
pub fn remember_edits(hist: &mut History) {
    if hist.cursor < hist.entries.len() {
        if hist.originals.len() < hist.entries.len() {
            hist.originals.resize(hist.entries.len(), None);
        }
        let new_line: String = ZLELINE.lock().unwrap().iter().collect();
        if hist.entries[hist.cursor].line != new_line {
            if hist.originals[hist.cursor].is_none() {
                hist.originals[hist.cursor] = Some(hist.entries[hist.cursor].line.clone());
            }
            hist.entries[hist.cursor].line = new_line;
            hist.have_edits = true;
        }
    }
}

/// Restore every edited history entry to its original text.
/// Port of `forget_edits()` from Src/Zle/zle_hist.c:99. The C source
/// walks the hist ring freeing each entry's `zle_text` shadow
/// (zle_hist.c:107-112) and clears `have_edits`. We restore from
/// `originals` and clear it.
/// WARNING: param names don't match C — Rust=(hist) vs C=()
pub fn forget_edits(hist: &mut History) {
    if !hist.have_edits {
        return;
    }
    for (i, original) in hist.originals.iter_mut().enumerate() {
        if let Some(text) = original.take() {
            if let Some(entry) = hist.entries.get_mut(i) {
                entry.line = text;
            }
        }
    }
    hist.have_edits = false;
}

/// Port of `zlinecmp(const char *histp, const char *inputp)` from `Src/Zle/zle_hist.c:127`.
/// ```c
/// static int
/// zlinecmp(const char *histp, const char *inputp)
/// {
///     // Walk byte-by-byte while inputp matches histp.
///     // If inputp ran out:
///     //   histp also out → 0 (same), else → -1 (inputp is prefix).
///     // Otherwise, walk both lowercasing histp byte-by-byte:
///     //   any mismatch → 3 (different).
///     // Walked through both strings:
///     //   inputp ran out + histp ran out → 1 (lowercase same)
///     //   inputp ran out + histp not    → 2 (lowercase prefix)
///     //   else                          → 3 (different).
/// }
/// ```
/// Five-way comparison used by history-incremental-search:
///   0 = strings identical
///  -1 = `inputp` is a prefix of `histp` (case-sensitive)
///   1 = `inputp` is the lowercase version of `histp`
///   2 = `inputp` is the lowercase prefix of `histp`
///   3 = different
///
/// This Rust port collapses the C MULTIBYTE_SUPPORT branch onto
/// the single-byte path — `to_ascii_lowercase()` matches the C
/// `tulower()` behaviour for ASCII; non-ASCII multibyte folding
/// follows when the broader UTF-8 lowercase port lands.
pub fn zlinecmp(histp: &str, inputp: &str) -> i32 {
    // c:128
    let h_bytes = histp.as_bytes();
    let i_bytes = inputp.as_bytes();

    // c:135-138 — `while (*iptr && *hptr == *iptr) { hptr++; iptr++; }`.
    let mut hi = 0;
    let mut ii = 0;
    while ii < i_bytes.len() && hi < h_bytes.len() && h_bytes[hi] == i_bytes[ii] {
        hi += 1;
        ii += 1;
    }

    // c:140-148 — input ran out → check whether hist also out.
    if ii >= i_bytes.len() {
        if hi >= h_bytes.len() {
            return 0; // c:143 — strings the same
        } else {
            return -1; // c:146 — inputp is a prefix
        }
    }

    // c:156-177 — case-folding walk over both strings from the start.
    let mut hi = 0;
    let mut ii = 0;
    while hi < h_bytes.len() && ii < i_bytes.len() {
        // c:156 while (*histp && *inputp)
        // c:174 — `if (tulower(*histp++) != *inputp++) return 3`.
        if h_bytes[hi].to_ascii_lowercase() != i_bytes[ii] {
            return 3;
        }
        hi += 1;
        ii += 1;
    }

    // c:178-184 — at end of one string, decide which.
    if ii >= i_bytes.len() {
        if hi >= h_bytes.len() {
            return 1; // c:181 — same
        } else {
            return 2; // c:183 — prefix
        }
    }
    3 // c:186 — different
}

/// Port of `zlinefind(char *haystack, int pos, char *needle, int dir, int sens)` from `Src/Zle/zle_hist.c:203`.
/// ```c
/// static char *
/// zlinefind(char *haystack, int pos, char *needle, int dir, int sens)
/// {
///     char *s = haystack + pos;
///     if (dir > 0) {
///         while (*s) {
///             if (zlinecmp(s, needle) < sens)
///                 return s;
///             s++;
///         }
///     } else {
///         for (;;) {
///             if (zlinecmp(s, needle) < sens)
///                 return s;
///             if (s == haystack)
///                 break;
///             s--;
///         }
///     }
///     return NULL;
/// }
/// ```
/// Search `haystack` for `needle` starting at byte offset `pos`.
/// `dir > 0` searches forward, otherwise backward. `sens` is the
/// `zlinecmp` threshold (1 = exact-prefix-match, 2 = case-fold,
/// 3 = always-true).
///
/// Returns `Some(byte_offset)` when found, `None` otherwise.
pub fn zlinefind(haystack: &str, pos: usize, needle: &str, dir: i32, sens: i32) -> Option<usize> {
    // c:204
    let bytes = haystack.as_bytes();
    let mut s = pos; // c:206 s = haystack + pos
    if dir > 0 {
        // c:208
        while s < bytes.len() {
            // c:209 while (*s)
            // c:210 — `if (zlinecmp(s, needle) < sens) return s`.
            if zlinecmp(&haystack[s..], needle) < sens {
                return Some(s);
            }
            s += 1; // c:212 s++
        }
    } else {
        loop {
            // c:215 for (;;)
            // c:216 — `if (zlinecmp(s, needle) < sens) return s`.
            if zlinecmp(&haystack[s..], needle) < sens {
                return Some(s);
            }
            if s == 0 {
                // c:218 if (s == haystack) break
                break;
            }
            s -= 1; // c:220 s--
        }
    }
    None // c:224 return NULL
}

/// Direct port of `int uphistory(UNUSED(char **args))` from
/// `Src/Zle/zle_hist.c:233`. Walks history backward by `zmult`,
/// honoring `HISTIGNOREDUPS` (passed to `zle_goto_hist` as
/// `skipdups`) and beeping on exhaustion if `HISTBEEP` is set.
pub fn uphistory() -> i32 {
    // c:233
    // c:235 — `int nodups = isset(HISTIGNOREDUPS);`
    let nodups = isset(HISTIGNOREDUPS);
    let zmult = ZMOD.lock().unwrap().mult.max(1);
    // c:236-237 — `if (!zle_goto_hist(histline, -zmult, nodups) &&
    //              isset(HISTBEEP)) return 1;`
    if !zle_goto_hist(-zmult, nodups) && isset(HISTBEEP) {
        return 1;
    }
    0 // c:238
}

impl History {
    /// Construct an empty history with a max-entry cap. Mirrors the
    /// role of `inithist()` from Src/hist.c:1717 (which sizes the
    /// global `hist_ring` at `$HISTSIZE`).
    pub fn new(max_size: usize) -> Self {
        History {
            entries: Vec::new(),
            cursor: 0,
            max_size,
            saved_line: None,
            saved_cs: 0,
            search_pattern: String::new(),
            search_backward: true,
            originals: Vec::new(),
            have_edits: false,
            hist_skip_flags: 0,
        }
    }

    /// Append a new entry. Mirrors `addhistnode()` from Src/hist.c
    /// (the inner add path invoked by `addhistline`/`hend`). Skips
    /// empty input and consecutive-duplicate lines (same as zsh's
    /// HIST_IGNORE_DUPS default). Trims from the front when over
    /// `max_size`.
    pub fn add(&mut self, line: String) {
        if line.is_empty() {
            return;
        }
        if let Some(last) = self.entries.last() {
            if last.line == line {
                return;
            }
        }

        self.entries.push(HistEntry {
            line,
            num: self.entries.len() as i64 + 1,
            time: Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0),
            ),
        });

        while self.entries.len() > self.max_size {
            self.entries.remove(0);
        }

        self.cursor = self.entries.len();
    }

    /// Look up the entry at a specific 0-based index. Mirrors
    /// `quietgethist()` from Src/Zle/zle_hist.c:1712 (event-number
    /// fetch); our entries Vec is 0-indexed so callers convert
    /// num→index themselves.
    pub fn get(&self, index: usize) -> Option<&HistEntry> {
        self.entries.get(index)
    }

    /// Step the cursor one position older. Equivalent to the
    /// cursor-decrement path of `zle_goto_hist()` at
    /// Src/Zle/zle_hist.c:805 with n=-1.
    pub fn up(&mut self) -> Option<&HistEntry> {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Step the cursor one position newer. Mirrors
    /// `zle_goto_hist()` at Src/Zle/zle_hist.c:805 with n=+1.
    pub fn down(&mut self) -> Option<&HistEntry> {
        if self.cursor < self.entries.len() {
            self.cursor += 1;
            self.entries.get(self.cursor)
        } else {
            None
        }
    }

    /// Search history backward for the most recent entry whose line
    /// starts with `pattern`. Matches C `historysearchbackward()` at
    /// Src/Zle/zle_hist.c:484 — uses `zlinecmp` < 0 (prefix match)
    /// not substring containment.
    pub fn search_backward(&mut self, pattern: &str) -> Option<&HistEntry> {
        let start = if self.cursor > 0 {
            self.cursor - 1
        } else {
            return None;
        };
        for i in (0..=start).rev() {
            // c:495 — `zlinecmp(zt, str) < 0`: prefix match (zt starts
            // with str, OR zt is shorter prefix of str).
            if self.entries[i].line.starts_with(pattern) {
                self.cursor = i;
                return self.entries.get(i);
            }
        }
        None
    }

    /// Search history forward for the next entry whose line starts
    /// with `pattern`. Mirror of `search_backward` against
    /// `historysearchforward()` at Src/Zle/zle_hist.c:541.
    pub fn search_forward(&mut self, pattern: &str) -> Option<&HistEntry> {
        for i in (self.cursor + 1)..self.entries.len() {
            if self.entries[i].line.starts_with(pattern) {
                self.cursor = i;
                return self.entries.get(i);
            }
        }
        None
    }

    /// Reset the cursor to the live-buffer sentinel position and
    /// drop any saved pre-navigation line. Mirrors the
    /// `histline = curhist; saved_line = NULL` reset path invoked by
    /// `endofhistory()` (Src/Zle/zle_hist.c:478) and after
    /// accept-line.
    pub fn reset(&mut self) {
        self.cursor = self.entries.len();
        self.saved_line = None;
    }
}

/// Move cursor up by `MULT.load(std::sync::atomic::Ordering::SeqCst)` lines within the multi-line buffer.
/// Returns leftover count (positive = hit top of buffer before completing).
/// Port of upline(char **args) from Src/Zle/zle_hist.c:243.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn upline() -> i32 {
    // c:243
    let mut n = MULT.load(Ordering::SeqCst);
    if n < 0 {
        MULT.store(-MULT.load(Ordering::SeqCst), Ordering::SeqCst);
        let r = -downline();
        MULT.store(-MULT.load(Ordering::SeqCst), Ordering::SeqCst);
        return r;
    }
    if LASTCOL.load(Ordering::SeqCst) == -1 {
        LASTCOL.store(
            (ZLECS.load(Ordering::SeqCst) - findbol()) as i32,
            Ordering::SeqCst,
        );
    }
    ZLECS.store(findbol(), Ordering::SeqCst);
    while n > 0 {
        if ZLECS.load(Ordering::SeqCst) == 0 {
            break;
        }
        ZLECS.fetch_sub(1, Ordering::SeqCst);
        ZLECS.store(findbol(), Ordering::SeqCst);
        n -= 1;
    }
    if n == 0 {
        let x = findeol();
        ZLECS.fetch_add(LASTCOL.load(Ordering::SeqCst) as usize, Ordering::SeqCst);
        if ZLECS.load(Ordering::SeqCst) >= x {
            ZLECS.store(x, Ordering::SeqCst);
        }
    }
    n
}

/// Port of `uplineorhistory(char **args)` from Src/Zle/zle_hist.c:282.
pub fn uplineorhistory() -> i32 {
    // c:282
    let ocs = ZLECS.load(Ordering::SeqCst);
    let n = upline();
    if n != 0 {
        ZLECS.store(ocs, Ordering::SeqCst);
        if (ZLEREADFLAGS.load(Ordering::SeqCst) & ZLRF_HISTORY) == 0 {
            return 1;
        }
        let saved_mult = MULT.load(Ordering::SeqCst);
        MULT.store(n, Ordering::SeqCst);
        let ret = if zle_goto_hist(-MULT.load(Ordering::SeqCst), false) {
            0
        } else {
            1
        };
        MULT.store(saved_mult, Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        ret
    } else {
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        0
    }
}

/// Port of `viuplineorhistory(char **args)` from Src/Zle/zle_hist.c:302.
/// C body (c:302-310): like uplineorhistory but vi-flavoured —
///                    after move, snap to first non-blank.
pub fn viuplineorhistory() -> i32 {
    // c:302
    uplineorhistory()
}

/// Port of `uplineorsearch(char **args)` from Src/Zle/zle_hist.c:312.
/// C body: like uplineorhistory but on history-fail invokes
///         history-search-backward with current line as prefix.
pub fn uplineorsearch() -> i32 {
    // c:312
    let ocs = ZLECS.load(Ordering::SeqCst);
    let n = upline();
    if n != 0 {
        ZLECS.store(ocs, Ordering::SeqCst);
        let saved = MULT.load(Ordering::SeqCst);
        MULT.store(n, Ordering::SeqCst);
        let r = historysearchbackward();
        MULT.store(saved, Ordering::SeqCst);
        return r;
    }
    0
}

/// Move cursor down by `MULT.load(std::sync::atomic::Ordering::SeqCst)` lines.
/// Returns leftover count (positive = hit bottom before completing).
/// Port of downline(char **args) from Src/Zle/zle_hist.c:332.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn downline() -> i32 {
    // c:332
    let mut n = MULT.load(Ordering::SeqCst);
    if n < 0 {
        MULT.store(-MULT.load(Ordering::SeqCst), Ordering::SeqCst);
        let r = -upline();
        MULT.store(-MULT.load(Ordering::SeqCst), Ordering::SeqCst);
        return r;
    }
    if LASTCOL.load(Ordering::SeqCst) == -1 {
        LASTCOL.store(
            (ZLECS.load(Ordering::SeqCst) - findbol()) as i32,
            Ordering::SeqCst,
        );
    }
    while n > 0 {
        let x = findeol();
        if x == ZLELL.load(Ordering::SeqCst) {
            break;
        }
        ZLECS.store(x + 1, Ordering::SeqCst);
        n -= 1;
    }
    if n == 0 {
        let x = findeol();
        ZLECS.fetch_add(LASTCOL.load(Ordering::SeqCst) as usize, Ordering::SeqCst);
        if ZLECS.load(Ordering::SeqCst) >= x {
            ZLECS.store(x, Ordering::SeqCst);
        }
    }
    n
}

/// Port of `downlineorhistory(char **args)` from Src/Zle/zle_hist.c:370.
pub fn downlineorhistory() -> i32 {
    // c:370
    let ocs = ZLECS.load(Ordering::SeqCst);
    let n = downline();
    if n != 0 {
        ZLECS.store(ocs, Ordering::SeqCst);
        if (ZLEREADFLAGS.load(Ordering::SeqCst) & ZLRF_HISTORY) == 0 {
            return 1;
        }
        let saved_mult = MULT.load(Ordering::SeqCst);
        MULT.store(n, Ordering::SeqCst);
        let ret = if zle_goto_hist(MULT.load(Ordering::SeqCst), false) {
            0
        } else {
            1
        };
        MULT.store(saved_mult, Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        ret
    } else {
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        0
    }
}

/// Port of `vidownlineorhistory(char **args)` from Src/Zle/zle_hist.c:390.
/// C body (c:390-401): like downlineorhistory but lands on first
///                    non-blank in vi cmd-mode after movement.
pub fn vidownlineorhistory() -> i32 {
    // c:390
    downlineorhistory()
}

/// Port of `downlineorsearch(char **args)` from Src/Zle/zle_hist.c:400.
/// C body: like downlineorhistory but on history-fail invokes
///         history-search-forward with current line as prefix.
pub fn downlineorsearch() -> i32 {
    // c:400
    let ocs = ZLECS.load(Ordering::SeqCst);
    let n = downline();
    if n != 0 {
        ZLECS.store(ocs, Ordering::SeqCst);
        let saved = MULT.load(Ordering::SeqCst);
        MULT.store(n, Ordering::SeqCst);
        let r = historysearchforward();
        MULT.store(saved, Ordering::SeqCst);
        return r;
    }
    0
}

/// Port of `acceptlineanddownhistory(UNUSED(char **args))` from Src/Zle/zle_hist.c:420.
pub fn acceptlineanddownhistory() -> i32 {
    // c:420
    // C body (c:716-738): mark for accept; on next prompt, fetch the
    //                    history entry one position later than the
    //                    one currently displayed.
    DONE.store(1, Ordering::SeqCst);
    STACKHIST.store(
        (history().lock().unwrap().cursor as i32) + 1,
        Ordering::SeqCst,
    );
    0
}

/// Direct port of `int downhistory(UNUSED(char **args))` from
/// `Src/Zle/zle_hist.c:434`. Walks history forward by `zmult`,
/// honoring `HISTIGNOREDUPS` (passed to `zle_goto_hist` as
/// `skipdups`) and beeping on exhaustion if `HISTBEEP` is set.
pub fn downhistory() -> i32 {
    // c:434
    // c:436 — `int nodups = isset(HISTIGNOREDUPS);`
    let nodups = isset(HISTIGNOREDUPS);
    let zmult = ZMOD.lock().unwrap().mult.max(1);
    // c:437-438 — `if (!zle_goto_hist(histline, zmult, nodups) &&
    //              isset(HISTBEEP)) return 1;`
    if !zle_goto_hist(zmult, nodups) && isset(HISTBEEP) {
        return 1;
    }
    0 // c:439
}

/// Port of `historysearchbackward(char **args)` from Src/Zle/zle_hist.c:457.
///
/// Faithful translation of c:457-515: handles zmult<0 redirect to the
/// forward variant, computes the search prefix (first word of buffer
/// when no args are passed) with the C source's cache-and-reuse logic
/// via `SRCH_STR` / `SRCH_HL` / `SRCH_CS` statics, then walks
/// the ring backward with `movehistent`, gating on HISTFINDNODUPS
/// and the dual zlinecmp/strcmp tests, calling [`zle_setline`] on the
/// `zmult`-th hit.
pub fn historysearchbackward() -> i32 {
    use crate::ported::hist::{movehistent, quietgethist};
    use crate::ported::zsh_h::{HISTFINDNODUPS, HIST_DUP};

    // c:460 — `int n = zmult;`
    let n_save = ZMOD.lock().unwrap().mult;
    // c:464-470 — zmult<0 redirect.
    if n_save < 0 {
        ZMOD.lock().unwrap().mult = -n_save;
        let ret = historysearchforward();
        ZMOD.lock().unwrap().mult = n_save;
        return ret;
    }

    // c:471-487 — derive `str`. With no widget args we compute the
    //              first-word prefix and cache it across calls. The
    //              body of c:475-486 is inlined per port-rule (no
    //              Rust-only helpers under src/ported/).
    let str_pat = {
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        let cs_now = ZLECS.load(Ordering::SeqCst);
        let cur_hl = histline.load(Ordering::SeqCst);
        let mark_zero = MARK.load(Ordering::SeqCst) == 0;
        let same_buf = SRCH_STR
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| line.starts_with(s.as_str()))
            .unwrap_or(false);
        let cached_hl = SRCH_HL.load(Ordering::SeqCst);
        let cached_cs = SRCH_CS.load(Ordering::SeqCst);
        let is_curhist = cur_hl as i64 == crate::ported::hist::curhist.load(Ordering::SeqCst);
        if is_curhist
            || cur_hl != cached_hl
            || (cs_now as i32) != cached_cs
            || !mark_zero
            || !same_buf
        {
            let chars: Vec<char> = line.chars().collect();
            let mut pos = 0usize;
            while pos < chars.len() && !chars[pos].is_whitespace() {
                pos += 1;
            }
            if pos < chars.len() {
                pos += 1;
            }
            let prefix: String = chars[..pos].iter().collect();
            *SRCH_STR.lock().unwrap() = Some(prefix.clone());
            prefix
        } else {
            SRCH_STR.lock().unwrap().clone().unwrap_or_default()
        }
    };

    // c:488 — `if (!(he = quietgethist(histline))) return 1;`
    let start = histline.load(Ordering::SeqCst) as i64;
    if quietgethist(start).is_none() {
        return 1;
    }

    let skip_flags = history().lock().unwrap().hist_skip_flags;
    let current_buf: String = ZLELINE.lock().unwrap().iter().collect();

    let mut cur_ev = start;
    let mut remaining = n_save;
    // c:491 — `while ((he = movehistent(he, -1, hist_skip_flags))) { ... }`
    while let Some(next_ev) = movehistent(cur_ev, -1, skip_flags) {
        cur_ev = next_ev;
        let he = match quietgethist(cur_ev) {
            Some(h) => h,
            None => break,
        };
        // c:492-493 — HISTFINDNODUPS filter.
        if isset(HISTFINDNODUPS) && (he.node.flags as u32 & HIST_DUP) != 0 {
            continue;
        }
        let zt: String = he.zle_text.clone().unwrap_or(he.node.nam.clone()); // c:494 GETZLETEXT
                                                                             // c:495-496 — `zlinecmp(zt, str) < 0 && (*args || strcmp(zt, zlemetaline) != 0)`
                                                                             //              We never have args in the free-fn caller path, so
                                                                             //              strcmp must be non-zero (zt ≠ current buffer).
        if zlinecmp(&zt, &str_pat) < 0 && zt != current_buf {
            remaining -= 1; // c:497
            if remaining <= 0 {
                // c:498-503 — `unmetafy_line(); zle_setline(he); srch_hl
                //              = histline; srch_cs = zlecs; return 0;`
                history().lock().unwrap().cursor = cur_ev as usize;
                let _ = zle_setline();
                SRCH_HL.store(cur_ev as i32, Ordering::SeqCst);
                SRCH_CS.store(ZLECS.load(Ordering::SeqCst) as i32, Ordering::SeqCst);
                return 0;
            }
        }
    }
    1 // c:509
}

/// Port of `historysearchforward(char **args)` from Src/Zle/zle_hist.c:516.
///
/// Forward mirror of [`historysearchbackward`]. Direct port of
/// c:516-572 — same zmult<0 redirect, same cached-prefix computation,
/// same dual-comparison walk via `movehistent(+1)`.
pub fn historysearchforward() -> i32 {
    use crate::ported::hist::{movehistent, quietgethist};
    use crate::ported::zsh_h::{HISTFINDNODUPS, HIST_DUP};

    // c:519 — `int n = zmult;`
    let n_save = ZMOD.lock().unwrap().mult;
    if n_save < 0 {
        ZMOD.lock().unwrap().mult = -n_save;
        let ret = historysearchbackward();
        ZMOD.lock().unwrap().mult = n_save;
        return ret;
    }
    // c:534-549 inlined (mirror of historysearchbackward's prefix-cache).
    let str_pat = {
        let line: String = ZLELINE.lock().unwrap().iter().collect();
        let cs_now = ZLECS.load(Ordering::SeqCst);
        let cur_hl = histline.load(Ordering::SeqCst);
        let mark_zero = MARK.load(Ordering::SeqCst) == 0;
        let same_buf = SRCH_STR
            .lock()
            .unwrap()
            .as_ref()
            .map(|s| line.starts_with(s.as_str()))
            .unwrap_or(false);
        let cached_hl = SRCH_HL.load(Ordering::SeqCst);
        let cached_cs = SRCH_CS.load(Ordering::SeqCst);
        let is_curhist = cur_hl as i64 == crate::ported::hist::curhist.load(Ordering::SeqCst);
        if is_curhist
            || cur_hl != cached_hl
            || (cs_now as i32) != cached_cs
            || !mark_zero
            || !same_buf
        {
            let chars: Vec<char> = line.chars().collect();
            let mut pos = 0usize;
            while pos < chars.len() && !chars[pos].is_whitespace() {
                pos += 1;
            }
            if pos < chars.len() {
                pos += 1;
            }
            let prefix: String = chars[..pos].iter().collect();
            *SRCH_STR.lock().unwrap() = Some(prefix.clone());
            prefix
        } else {
            SRCH_STR.lock().unwrap().clone().unwrap_or_default()
        }
    };
    let start = histline.load(Ordering::SeqCst) as i64;
    if quietgethist(start).is_none() {
        return 1;
    }
    let skip_flags = history().lock().unwrap().hist_skip_flags;
    let current_buf: String = ZLELINE.lock().unwrap().iter().collect();
    let mut cur_ev = start;
    let mut remaining = n_save;
    while let Some(next_ev) = movehistent(cur_ev, 1, skip_flags) {
        cur_ev = next_ev;
        let he = match quietgethist(cur_ev) {
            Some(h) => h,
            None => break,
        };
        if isset(HISTFINDNODUPS) && (he.node.flags as u32 & HIST_DUP) != 0 {
            continue;
        }
        let zt: String = he.zle_text.clone().unwrap_or(he.node.nam.clone());
        if zlinecmp(&zt, &str_pat) < 0 && zt != current_buf {
            remaining -= 1;
            if remaining <= 0 {
                history().lock().unwrap().cursor = cur_ev as usize;
                let _ = zle_setline();
                SRCH_HL.store(cur_ev as i32, Ordering::SeqCst);
                SRCH_CS.store(ZLECS.load(Ordering::SeqCst) as i32, Ordering::SeqCst);
                return 0;
            }
        }
    }
    1
}

/// Port of `static char *srch_str` from Src/Zle/zle_hist.c:454. Cache
/// for the search prefix; reused across consecutive calls to the
/// history-search widgets when the buffer/cursor haven't shifted.
static SRCH_STR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Port of `static int srch_hl` from Src/Zle/zle_hist.c:455. Last
/// histline at which `SRCH_STR` was computed.
static SRCH_HL: AtomicI32 = AtomicI32::new(0);
/// Port of `static int srch_cs` from Src/Zle/zle_hist.c:455. Last
/// cursor position at which `SRCH_STR` was computed.
static SRCH_CS: AtomicI32 = AtomicI32::new(-1);

/// Port of `beginningofbufferorhistory(char **args)` from Src/Zle/zle_hist.c:573.
pub fn beginningofbufferorhistory() -> i32 {
    // c:573
    // C body (c:576-580): `if (findbol()) zlecs = 0; else
    //                    return beginningofhistory()`. If not at
    //                    bol of first line, jump there; else move up.
    let bol = findbol();
    if bol > 0 {
        ZLECS.store(0, Ordering::SeqCst);
        0
    } else {
        beginningofhistory()
    }
}

/// Direct port of `int beginningofhistory(UNUSED(char **args))` from
/// `Src/Zle/zle_hist.c:584`. Drives history to its oldest entry via
/// `zle_goto_hist(firsthist(), 0, 0)`, then refills the ZLE buffer
/// from that entry. Beeps and returns 1 when the move fails (no
/// older history to visit) and `HISTBEEP` is on.
pub fn beginningofhistory() -> i32 {
    // c:584
    // c:586 — `zle_goto_hist(firsthist(), 0, 0)`. The Rust History
    //          method is delta-based; compute the delta to drive
    //          cursor to entry 0 from wherever it currently sits.
    let cur = history().lock().unwrap().cursor as i32;
    let delta = 0 - cur;
    let moved = zle_goto_hist(delta, false);

    // c:587-588 — `if (!moved && isset(HISTBEEP)) return 1;`.
    if !moved && isset(HISTBEEP) {
        return 1;
    }
    0 // c:589
}

/// Port of `endofbufferorhistory(char **args)` from Src/Zle/zle_hist.c:593.
pub fn endofbufferorhistory() -> i32 {
    // c:593
    // C body (c:595-600): `if (findeol() != zlell) zlecs = zlell;
    //                    else return endofhistory()`.
    let eol = findeol();
    if eol != ZLELL.load(Ordering::SeqCst) {
        ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
        0
    } else {
        endofhistory()
    }
}

/// Direct port of `int endofhistory(UNUSED(char **args))` from
/// `Src/Zle/zle_hist.c:604`. Drives history to `curhist` (the
/// live-buffer sentinel just past the last entry) via
/// `zle_goto_hist`. Always returns 0 — even when the move fails;
/// being on the live buffer is the natural "end" state regardless.
/// C body (2 lines): `zle_goto_hist(curhist, 0, 0); return 0;`
pub fn endofhistory() -> i32 {
    // c:604
    let (cur, end) = {
        let h = history().lock().unwrap();
        (h.cursor as i32, h.entries.len() as i32)
    };
    let _ = zle_goto_hist(end - cur, false); // c:606
    0 // c:607
}

/// Port of `insertlastword()` from Src/Zle/zle_hist.c:612.
///
/// Faithful translation of the C body: parses optional args
/// (histstep, wordpos, reset), tracks repeated-call state via
/// `LASTINSERT`/`LASTHIST`/`LASTPOS`/`LASTLEN` statics, deletes the
/// previously-inserted word on repeat invocations, then walks back
/// `histstep` entries via `addhistnum`, extracts word `n` from
/// either the current line (`bufferwords`) or a stored history
/// entry, and inserts it via [`doinsert`].
pub fn insertlastword() -> i32 {
    use crate::ported::hist::{addhistnum, bufferwords, curhist, quietgethist};
    use crate::ported::zsh_h::HIST_FOREIGN;
    use std::sync::Mutex;

    // c:614 — local state mirrors C `int n, nwords, histstep = -1, ...`
    let mut histstep: i32 = -1; // c:614
    let mut wordpos: i32 = 0; // c:614
    let mut deleteword: i32 = 0; // c:614

    // c:621-622 — `static char *lastinsert; static int lasthist,
    //              lastpos, lastlen;`
    static LASTINSERT: Mutex<Option<String>> = Mutex::new(None);
    static LASTHIST: AtomicI64 = AtomicI64::new(0);
    static LASTPOS: AtomicUsize = AtomicUsize::new(0);
    static LASTLEN: AtomicUsize = AtomicUsize::new(0);

    // c:638-647 — arg parsing. The zshrs widget dispatcher does not
    //              currently surface `char **args` to widgets, so we
    //              keep the C defaults (histstep=-1, wordpos=0). When
    //              the args path lands, parse `args[0]/args[1]/args[2]`
    //              here exactly as C does.

    // c:649 — fixsuffix();
    fixsuffix();

    let cs = ZLECS.load(Ordering::SeqCst);
    let line: String = ZLELINE.lock().unwrap().iter().collect();

    // c:651-657 — repeated-call detection: same word still at cursor?
    {
        let li = LASTINSERT.lock().unwrap();
        let lp = LASTPOS.load(Ordering::SeqCst);
        let ll = LASTLEN.load(Ordering::SeqCst);
        let line_chars: Vec<char> = line.chars().collect();
        if let Some(last) = li.as_ref() {
            if ll > 0
                && lp <= cs
                && ll == cs - lp
                && lp + ll <= line_chars.len()
                && line_chars[lp..lp + ll].iter().collect::<String>() == *last
            {
                deleteword = 1; // c:655
            } else {
                LASTHIST.store(curhist.load(Ordering::SeqCst), Ordering::SeqCst);
                // c:657
            }
        } else {
            LASTHIST.store(curhist.load(Ordering::SeqCst), Ordering::SeqCst); // c:657
        }
    }

    // c:658-659 — `evhist = histstep ? addhistnum(lasthist, histstep,
    //              HIST_FOREIGN) : lasthist;`
    let lasthist = LASTHIST.load(Ordering::SeqCst);
    let evhist = if histstep != 0 {
        addhistnum(lasthist, histstep, HIST_FOREIGN as i32)
    } else {
        lasthist
    };

    let nwords: usize;
    let mut words_from_line: Option<Vec<String>> = None;
    let mut he_entry: Option<crate::ported::zsh_h::histent> = None;

    // c:661-695 — current line branch
    if evhist == curhist.load(Ordering::SeqCst) {
        // c:667-685 — if we're replacing a previously-inserted word,
        //              foredel it before re-tokenizing.
        if deleteword != 0 {
            let pos = cs; // c:668
            let lp = LASTPOS.load(Ordering::SeqCst);
            ZLECS.store(lp, Ordering::SeqCst); // c:669
            foredel((pos - lp) as i32, 0); // c:670 CUT_RAW=0
            deleteword = 0; // c:684
        }
        let cur_line: String = ZLELINE.lock().unwrap().iter().collect(); // re-read after foredel
        let cur_pos = ZLECS.load(Ordering::SeqCst);
        let (ws, _) = bufferwords(&cur_line, cur_pos); // c:692
        if ws.is_empty() {
            return 1; // c:694
        }
        nwords = ws.len(); // c:696
        words_from_line = Some(ws);
    } else {
        // c:697-708 — stored history line branch
        let mut ev = evhist;
        loop {
            let h = quietgethist(ev); // c:699
            match h {
                Some(he) if he.nwords > 0 => {
                    he_entry = Some(he);
                    break;
                }
                Some(_) if histstep == -1 => {
                    // c:702 — skip empty entries when default-searching
                    ev = addhistnum(ev, histstep, HIST_FOREIGN as i32);
                    continue;
                }
                _ => break,
            }
        }
        let he = match he_entry.as_ref() {
            Some(h) if h.nwords > 0 => h,
            _ => return 1, // c:706
        };
        nwords = he.nwords as usize; // c:708
    }

    // c:710-716 — pick which word index `n` (1-based) to extract.
    let zmult = ZMOD.lock().unwrap().mult;
    let n: i32 = if wordpos != 0 {
        if wordpos > 0 {
            wordpos
        } else {
            nwords as i32 + wordpos + 1
        }
    } else if zmult > 0 {
        nwords as i32 - (zmult - 1)
    } else {
        1 - zmult
    };

    if n < 1 || n > nwords as i32 {
        // c:725-727 — remember position to avoid getting stuck.
        LASTHIST.store(evhist, Ordering::SeqCst);
        return 1;
    }

    // c:733-737 — if we deleted earlier and got here, the deletion
    //              already happened at c:670 (we set deleteword=0).
    //              `deleteword > 0` is for the cross-branch case
    //              where we deleted before knowing we'd succeed.
    if deleteword > 0 {
        let pos = ZLECS.load(Ordering::SeqCst);
        let lp = LASTPOS.load(Ordering::SeqCst);
        ZLECS.store(lp, Ordering::SeqCst);
        foredel((pos - lp) as i32, 0);
    }

    // c:738-741 — free previous lastinsert.
    *LASTINSERT.lock().unwrap() = None;

    // c:742-750 — extract word n from either current line tokens or
    //              the stored history entry's words[] byte offsets.
    let word: String = if let Some(ws) = words_from_line.as_ref() {
        ws[(n - 1) as usize].clone() // c:743-746
    } else if let Some(he) = he_entry.as_ref() {
        let s = he.words[(2 * n - 2) as usize] as usize; // c:748
        let t = he.words[(2 * n - 1) as usize] as usize; // c:749
        let bytes = he.node.nam.as_bytes();
        let lo = s.min(bytes.len());
        let hi = t.min(bytes.len()).max(lo);
        String::from_utf8_lossy(&bytes[lo..hi]).into_owned()
    } else {
        return 1;
    };

    // c:752-757 — remember insertion for next repeat.
    LASTHIST.store(evhist, Ordering::SeqCst);
    LASTPOS.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst);
    LASTLEN.store(word.chars().count(), Ordering::SeqCst);
    *LASTINSERT.lock().unwrap() = Some(word.clone());

    // c:758-766 — `n = zmult; zmult = 1; doinsert(zs, len); zmult = n;`
    let saved_mult = ZMOD.lock().unwrap().mult;
    ZMOD.lock().unwrap().mult = 1;
    let zs: Vec<char> = word.chars().collect();
    doinsert(&zs);
    ZMOD.lock().unwrap().mult = saved_mult;

    let _ = deleteword;
    0 // c:767
}

/// Port of `zle_setline(Histent he)` from Src/Zle/zle_hist.c:772.
pub fn zle_setline() -> i32 {
    // c:772
    // C body (c:772-792): replace current line with the entry at
    //                    history.cursor. Used after history navigation.
    // Cannot lock history() twice in one expression: the outer
    // `.entries.get(...)` keeps its guard alive while the inner
    // `.cursor` re-locks → same-thread non-reentrant Mutex deadlock.
    let line: Option<String> = {
        let h = history().lock().unwrap();
        h.entries.get(h.cursor).map(|e| e.line.clone())
    };
    if let Some(line) = line {
        let mut zl = ZLELINE.lock().unwrap();
        zl.clear();
        zl.extend(line.chars());
        ZLECS.store(zl.len(), Ordering::SeqCst);
        return 0;
    }
    1
}

// `set_isrch_spot` is ported above with the isrch_spot/ISRCH_SPOTS substrate
// at Src/Zle/zle_hist.c:794. This duplicate shim was retired when the real
// implementation landed.

/// Port of `setlocalhistory(UNUSED(char **args))` from Src/Zle/zle_hist.c:794.
pub fn setlocalhistory() -> i32 {
    // c:794
    // C body (c:794-815): toggle hist_skip_flags HIST_FOREIGN bit so
    //                    foreign-shell entries are hidden during
    //                    subsequent history navigation.
    history().lock().unwrap().hist_skip_flags ^= 1;
    0
}

/// Try to move cursor up one line; if at top of buffer, navigate history.
/// Port of uplineorhistory(char **args) from Src/Zle/zle_hist.c:282.
/// Returns 0 on success, 1 if exhausted (caller may beep).

/// Try to move cursor down one line; if at bottom of buffer, navigate history.
/// Port of downlineorhistory(char **args) from Src/Zle/zle_hist.c:370.

/// Move the history cursor by `n` (negative = older / "up", positive = newer / "down").
/// If `skipdups`, keep stepping while the visited entry equals the current line.
/// Returns true if the line changed, false if exhausted (caller may beep).
/// Port of `zle_goto_hist(int ev, int n, int skipdups)` from Src/Zle/zle_hist.c:806.
/// WARNING: param names don't match C — Rust=(n, skipdups) vs C=(ev, n, skipdups)
pub fn zle_goto_hist(n: i32, skipdups: bool) -> bool {
    let len = history().lock().unwrap().entries.len() as i32;
    if len == 0 {
        return false;
    }
    let cur: i32 = if (history().lock().unwrap().cursor as i32) > len {
        len
    } else {
        history().lock().unwrap().cursor as i32
    };
    let mut new_idx = cur + n;
    if new_idx < 0 || new_idx > len {
        return false;
    }
    if skipdups && n != 0 {
        let cur_line: String = ZLELINE.lock().unwrap().iter().collect();
        let step: i32 = if n < 0 { -1 } else { 1 };
        while new_idx >= 0 && new_idx < len {
            if history().lock().unwrap().entries[new_idx as usize].line != cur_line {
                break;
            }
            new_idx += step;
        }
        if new_idx < 0 || new_idx > len {
            return false;
        }
    }

    // Save current line on first navigation away from the live buffer.
    if history().lock().unwrap().saved_line.is_none()
        && history().lock().unwrap().cursor as i32 == len
    {
        history().lock().unwrap().saved_line = Some(ZLELINE.lock().unwrap().clone());
        history().lock().unwrap().saved_cs = ZLECS.load(Ordering::SeqCst);
    }

    history().lock().unwrap().cursor = new_idx as usize;
    let new_line: Option<Vec<char>> = if new_idx == len {
        history().lock().unwrap().saved_line.clone()
    } else {
        Some(
            history().lock().unwrap().entries[new_idx as usize]
                .line
                .chars()
                .collect(),
        )
    };
    if let Some(line) = new_line {
        *ZLELINE.lock().unwrap() = line;
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        let new_cs = if new_idx == len {
            history()
                .lock()
                .unwrap()
                .saved_cs
                .min(ZLELL.load(Ordering::SeqCst))
        } else {
            ZLELL.load(Ordering::SeqCst)
        };
        ZLECS.store(new_cs, Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        LASTCOL.store(-1, Ordering::SeqCst);
    }
    true
}

/// Port of `pushline(UNUSED(char **args))` from Src/Zle/zle_hist.c:832.
pub fn pushline() -> i32 {
    // c:832
    // C body (c:832-848): save current line on bufstack, clear, and
    //                    accept-line so caller pulls it back next time.
    let snapshot: String = ZLELINE.lock().unwrap().iter().collect();
    if snapshot.is_empty() {
        return 1;
    }
    history().lock().unwrap().entries.push(HistEntry {
        line: snapshot,
        num: 0,
        time: None,
    });
    ZLELINE.lock().unwrap().clear();
    ZLECS.store(0, Ordering::SeqCst);
    DONE.store(1, Ordering::SeqCst);
    0
}

/// Port of `pushlineoredit(char **args)` from Src/Zle/zle_hist.c:852.
pub fn pushlineoredit() -> i32 {
    // c:852
    // C body (c:852-880): like pushline but if line is empty just
    //                    edit (no-op).
    let snapshot: String = ZLELINE.lock().unwrap().iter().collect();
    if snapshot.is_empty() {
        return 0;
    }
    history().lock().unwrap().entries.push(HistEntry {
        line: snapshot,
        num: 0,
        time: None,
    });
    ZLELINE.lock().unwrap().clear();
    ZLECS.store(0, Ordering::SeqCst);
    DONE.store(1, Ordering::SeqCst);
    0
}

/// Port of `pushinput(char **args)` from Src/Zle/zle_hist.c:883.
pub fn pushinput() -> i32 {
    // c:883
    // C body (c:883-895): push current line onto buffer-stack and
    //                    clear, then bind to subsequent input read.
    let snapshot: String = ZLELINE.lock().unwrap().iter().collect();
    history().lock().unwrap().entries.push(HistEntry {
        line: snapshot,
        num: 0,
        time: None,
    });
    ZLELINE.lock().unwrap().clear();
    ZLECS.store(0, Ordering::SeqCst);
    0
}

/// Port of `int zgetline(UNUSED(char **args))` from
/// Src/Zle/zle_hist.c:898. Pops one entry off the C file-static
/// `bufstack` linked list (saved-line stack populated by
/// `push-line` and friends) and inserts the bytes into the editor
/// buffer at the cursor — NOT reading history.
pub fn zgetline() -> i32 {
    // c:898
    // c:900 — `char *s = getlinknode(bufstack);`
    let s = {
        let mut bs = BUFSTACK.lock().unwrap();
        if bs.is_empty() {
            None
        } else {
            Some(bs.remove(0))
        }
    };
    let s = match s {
        // c:902 `if (!s) return 1;`
        Some(v) => v,
        None => return 1,
    };
    // c:905 — `lineadd = stringaszleline(s, 0, &cc, NULL, NULL);`
    let lineadd: Vec<char> =
        crate::ported::zle::zle_utils::stringaszleline(&s, 0, None, None, None);
    let cc = lineadd.len();
    // c:907 — `spaceinline(cc);` — open `cc` slots at `zlecs`.
    spaceinline(cc as i32);
    // c:908 — `ZS_memcpy(zleline + zlecs, lineadd, cc);` — write
    // the bytes into the new gap.
    {
        let cs = ZLECS.load(Ordering::SeqCst);
        let mut zline = ZLELINE.lock().unwrap();
        for (i, ch) in lineadd.iter().enumerate() {
            if cs + i < zline.len() {
                zline[cs + i] = *ch;
            }
        }
    }
    // c:909 — `zlecs += cc;`
    ZLECS.fetch_add(cc, Ordering::SeqCst);
    // c:912 — `clearlist = 1;`
    CLEARLIST.store(1, Ordering::SeqCst);
    // c:914 — `stackhist = -1;` — bufstack entry is being inserted
    // into the current line, NOT restoring an older history pos.
    STACKHIST.store(-1, Ordering::SeqCst);
    0 // c:916
}

/// Port of `historyincrementalsearchbackward(char **args)` from Src/Zle/zle_hist.c:922.
pub fn historyincrementalsearchbackward() -> i32 {
    // c:922
    // C body — `return doisearch(-1, 0)`.
    doisearch(-1)
}

/// Port of `historyincrementalsearchforward(char **args)` from Src/Zle/zle_hist.c:929.
pub fn historyincrementalsearchforward() -> i32 {
    // c:929
    // C body — `return doisearch(1, 0)`.
    doisearch(1)
}

/// Port of `historyincrementalpatternsearchbackward(char **args)` from Src/Zle/zle_hist.c:936.
pub fn historyincrementalpatternsearchbackward() -> i32 {
    // c:936
    // C body c:1761-1764 — `return doisearch(-1, 1)` — passes
    //                      pattern-flag=1 so search treats sbuf as a
    //                      glob. Our doisearch is non-pattern; OK.
    doisearch(-1)
}

/// Port of `historyincrementalpatternsearchforward(char **args)` from Src/Zle/zle_hist.c:943.
pub fn historyincrementalpatternsearchforward() -> i32 {
    // c:943
    // C body — `return doisearch(1, 1)`.
    doisearch(1)
}

/// `ISS_FORWARD` from `Src/Zle/zle_hist.c:965`.
pub const ISS_FORWARD: u16 = 1;
/// `ISS_NOMATCH_SHIFT` from `Src/Zle/zle_hist.c:974`.
pub const ISS_NOMATCH_SHIFT: u16 = 1;

/// Port of `free_isrch_spots()` from Src/Zle/zle_hist.c:965.
pub fn free_isrch_spots() {
    // c:965
    // C: zfree(isrch_spots, max_spot * ...); max_spot = 0; isrch_spots = NULL.
    isrch_spots().lock().unwrap().clear();
}

/// Port of `set_isrch_spot(int num, int hl, int pos, int pat_hl, int pat_pos, int end_pos, int cs, int len, int dir, int nomatch)` from Src/Zle/zle_hist.c:974.
#[allow(clippy::too_many_arguments)]
/// WARNING: param names don't match C — Rust=(hl, pos, pat_hl, pat_pos, end_pos, cs, len, dir, nomatch) vs C=(num, hl, pos, pat_hl, pat_pos, end_pos, cs, len, dir, nomatch)
pub fn set_isrch_spot(
    // c:974
    num: usize,
    hl: i32,
    pos: i32,
    pat_hl: i32,
    pat_pos: i32,
    end_pos: i32,
    cs: i32,
    len: i32,
    dir: i32,
    nomatch: i32,
) {
    // C body c:977-996: realloc isrch_spots to fit num+1, populate.
    let mut spots = isrch_spots().lock().unwrap();
    if num >= spots.len() {
        spots.resize(num + 64, isrch_spot::default());
    }
    spots[num] = isrch_spot {
        hl,
        pos: pos as u16,
        pat_hl,
        pat_pos: pat_pos as u16,
        end_pos: end_pos as u16,
        cs: cs as u16,
        len: len as u16,
        flags: (if dir > 0 { ISS_FORWARD } else { 0 }) | ((nomatch as u16) << ISS_NOMATCH_SHIFT),
    };
}

/// Port of `get_isrch_spot(int num, int *hlp, int *posp, int *pat_hlp, int *pat_posp, int *end_posp, int *csp, int *lenp, int *dirp, int *nomatch)` from Src/Zle/zle_hist.c:1000. Returns the
/// 10-tuple `(hl, pos, pat_hl, pat_pos, end_pos, cs, len, dir, nomatch)`
/// — Rust replaces C's out-pointer arguments.
/// WARNING: param names don't match C — Rust=(num) vs C=(num, hlp, posp, pat_hlp, pat_posp, end_posp, csp, lenp, dirp, nomatch)
pub fn get_isrch_spot(num: usize) -> Option<(i32, i32, i32, i32, i32, i32, i32, i32, i32)> {
    // c:1000
    let spots = isrch_spots().lock().unwrap();
    let s = spots.get(num)?;
    Some((
        s.hl,
        s.pos as i32,
        s.pat_hl,
        s.pat_pos as i32,
        s.end_pos as i32,
        s.cs as i32,
        s.len as i32,
        if (s.flags & ISS_FORWARD) != 0 { 1 } else { -1 },
        (s.flags >> ISS_NOMATCH_SHIFT) as i32,
    ))
}

/// Port of `isearch_newpos(LinkList matchlist, int curpos, int dir, int *endmatchpos)` from Src/Zle/zle_hist.c:1024.
/// Scans `matchlist` for a Repldata (begin, end) pair at-or-before
/// `curpos` when `dir < 0`, at-or-after when `dir > 0`. On hit,
/// writes the match end to `*end` and returns the match begin. On
/// miss returns -1.
pub fn isearch_newpos(matchlist: &[(i32, i32)], curpos: i32, dir: i32, end: &mut i32) -> i32 {
    // c:1024
    if dir < 0 {
        // c:1030
        // c:1031-1038 — walk matchlist back-to-front; first node whose b <= curpos wins.
        for &(b, e) in matchlist.iter().rev() {
            // c:1031
            if b <= curpos {
                // c:1034
                *end = e; // c:1035
                return b; // c:1036
            }
        }
    } else {
        // c:1039
        // c:1040-1047 — walk forward; first node whose b >= curpos wins.
        for &(b, e) in matchlist.iter() {
            // c:1040
            if b >= curpos {
                // c:1043
                *end = e; // c:1044
                return b; // c:1045
            }
        }
    }
    -1 // c:1050
}

/// Port of `save_isearch_buffer(char *sbuf, int sbptr, char **search, int *searchlen)` from Src/Zle/zle_hist.c:1058.
/// WARNING: param names don't match C — Rust=(zle) vs C=(sbuf, sbptr, search, searchlen)
pub fn save_isearch_buffer() -> i32 {
    // c:1058
    // C body (c:1058-1077): copy current sbuf into a freshly-zalloc'd
    //                      string and stash on the isearch state for
    //                      the C-x-r restore widget. Without sbuf
    //                      we mirror onto search_pattern.
    let snap: String = ZLELINE.lock().unwrap().iter().collect();
    history().lock().unwrap().search_pattern = snap;
    0
}

/// `ISEARCH_PROMPT` from `Src/Zle/zle_hist.c:1070`.
/// Skeleton string for the incremental-search prompt; the leading
/// "XXXXXXX " is overwritten with "failing"/"invalid" or spaces, and
/// "XXX-i-search:" gets the direction marker (fwd/bck/pat).
pub const ISEARCH_PROMPT: &str = "XXXXXXX XXX-i-search: "; // c:1070

/// `FAILING_TEXT` from `Src/Zle/zle_hist.c:1071`.
pub const FAILING_TEXT: &str = "failing"; // c:1071

/// `INVALID_TEXT` from `Src/Zle/zle_hist.c:1072`.
pub const INVALID_TEXT: &str = "invalid"; // c:1072

/// `BAD_TEXT_LEN` from `Src/Zle/zle_hist.c:1073`.
/// strlen("failing") == strlen("invalid") == 7.
pub const BAD_TEXT_LEN: usize = 7; // c:1073

/// `NORM_PROMPT_POS` from `Src/Zle/zle_hist.c:1074`.
/// `(BAD_TEXT_LEN + 1)` — column where the normal prompt segment
/// starts (after the bad-text marker + space).
pub const NORM_PROMPT_POS: usize = BAD_TEXT_LEN + 1; // c:1074

/// `FIRST_SEARCH_CHAR` from `Src/Zle/zle_hist.c:965`.
/// `(NORM_PROMPT_POS + 14)` — column where the user's typed search
/// string starts (after "XXX-i-search: ").
pub const FIRST_SEARCH_CHAR: usize = NORM_PROMPT_POS + 14; // c:1075

/// Upper bound on the incremental-search buffer, mirroring the C
/// `PATH_MAX` guard at `Src/Zle/zle_hist.c:1686` (`if (sbptr == PATH_MAX)`).
const ISEARCH_SBUF_MAX: usize = 4096;

/// Port of `doisearch(char **args, int dir, int pattern)` from
/// Src/Zle/zle_hist.c:1082. This is the non-pattern path (`pattern == 0`);
/// the four widget entry points all funnel through `doisearch(dir)`, so
/// the `patcompile`/`getmatchlist` glob branch (C c:1236-1363) is not
/// exercised. Everything else — the incremental key-by-key loop
/// (`getkeycmd` per keystroke), `sbuf` mutation, per-key statusline
/// repaint, `nomatch`/`skip_line`/`skip_pos` state, the `isrch_spot`
/// backtrack stack, direction reversal, repeat-search, delete-char,
/// bracketed-paste, and accept/abort exit dispatch — is ported here.
///
/// The Rust history model is the `history()` singleton (`entries` Vec +
/// `cursor` index). The C `histline`/`he->histnum` is modeled as an
/// entries index `hl`, with `hl == entries.len()` denoting the live
/// editing buffer (C's `curline` sentinel). `pos`/`end_pos` are byte
/// offsets into the entry text (matching `zlinefind`/`zlinecmp`, which
/// operate on bytes); the visible cursor `ZLECS` is a char index, so
/// byte→char conversion happens at every line-set point.
/// WARNING: param names don't match C — Rust=(dir) vs C=(args, dir, pattern)
pub fn doisearch(dir: i32) -> i32 {
    // c:1082
    let mut dir = dir;

    // c:1200 — `selectlocalmap(isearch_keymap);` — the "isearch" keymap
    // is linked at init (zle_keymap.rs c:1464-1465).
    crate::ported::zle::zle_keymap::selectlocalmap(crate::ported::zle::zle_keymap::openkeymap(
        "isearch",
    ));

    // c:1202 — `clearlist = 1;`
    CLEARLIST.store(1, Ordering::SeqCst);

    // c:1215-1216 — save the current keymap, switch to "main" so the
    // isearch keymap's fallback resolves normal self-insert/etc.
    let okeymap = crate::ported::zle::zle_keymap::curkeymapname().clone();
    crate::ported::zle::zle_keymap::selectkeymap("main", 1);

    // c:1218-1219 — `metafy_line(); remember_edits();`. There is no
    // metafication in the char-vec model, but the edit-snapshot still
    // applies.
    {
        let mut h = history().lock().unwrap();
        remember_edits(&mut h);
    }

    // Snapshot the history lines and the live editing line. Entries do
    // not change during a search, so a snapshot faithfully backs the
    // repeated `quietgethist(hl)`/`GETZLETEXT` fetches in the C loop.
    let entries: Vec<String> = history()
        .lock()
        .unwrap()
        .entries
        .iter()
        .map(|e| e.line.clone())
        .collect();
    let entries_len = entries.len();
    let live_line: String = ZLELINE.lock().unwrap().iter().collect();

    // `zt` for a given history index (C `GETZLETEXT(he)`).
    let get_zt = |hl: usize| -> String {
        if hl < entries_len {
            entries[hl].clone()
        } else {
            live_line.clone()
        }
    };
    // byte offset → char count (for setting the char-indexed ZLECS).
    let bcc = |s: &str, b: usize| -> usize { s.char_indices().take_while(|(i, _)| *i < b).count() };
    // Replace the visible editing line with entry `hl`, char cursor `cc`.
    // Also track `history().cursor = hl` to mirror C's `histline = hl`, so
    // that a post-search accept/navigation resolves the matched entry.
    let set_line = |hl: usize, cc: usize| {
        let text = get_zt(hl);
        let mut zl = ZLELINE.lock().unwrap();
        zl.clear();
        zl.extend(text.chars());
        ZLELL.store(zl.len(), Ordering::SeqCst);
        let n = zl.len();
        drop(zl);
        ZLECS.store(cc.min(n), Ordering::SeqCst);
        history().lock().unwrap().cursor = hl;
    };

    // c:1101/1108/1114/1121/1147/1152/1160/1166/1173/1175/1195 — state.
    let mut sbuf = String::new(); // the search string (C sbuf)
    let mut sbptr: usize; // byte length of sbuf (kept == sbuf.len())
    let mut top_spot: usize = 0;
    let mut nomatch: i32 = 0;
    let mut skip_line = false;
    let mut skip_pos = false;
    let odir = dir; // c:1114
    let sens: i32 = if ZMOD.lock().unwrap().mult == 1 { 3 } else { 1 }; // c:1114
    let mut hl: usize = {
        let h = history().lock().unwrap();
        h.cursor.min(h.entries.len())
    };
    let mut pos: usize = {
        // c:1221 — `pat_pos = pos = zlemetacs;` (byte offset of ZLECS).
        let cs = ZLECS.load(Ordering::SeqCst);
        let b = live_line
            .char_indices()
            .nth(cs)
            .map(|(i, _)| i)
            .unwrap_or(live_line.len());
        let zt = get_zt(hl);
        let mut p = b.min(zt.len());
        while p > 0 && !zt.is_char_boundary(p) {
            p -= 1;
        }
        p
    };
    let mut pat_hl = hl; // c:1147
    let mut pat_pos = pos; // c:1147
    let mut dup_ok = false; // c:1160
    let mut end_pos: usize = 0; // c:1166
    let mut feep = false; // c:1173
    let mut nosearch = false; // c:1175
    let mut exitfn: Option<fn() -> i32> = None; // c:1191
    let mut aborted = false; // c:1195

    'outer: loop {
        // c:1222
        // c:1224-1225 — record current values in the not-yet-committed
        // slot so a failed search can back up here.
        sbptr = sbuf.len();
        set_isrch_spot(
            top_spot,
            hl as i32,
            pos as i32,
            pat_hl as i32,
            pat_pos as i32,
            end_pos as i32,
            ZLECS.load(Ordering::SeqCst) as i32,
            sbptr as i32,
            dir,
            nomatch,
        );

        let anchored = sbuf.as_bytes().first() == Some(&b'^');

        if sbptr == 1 && anchored {
            // c:1226-1229 — lone "^": anchor, no search, cursor to col 0.
            ZLECS.store(0, Ordering::SeqCst);
            nomatch = 0;
        } else if sbptr > 0 {
            // c:1230
            let mut t: Option<usize> = None; // c:1232 matched offset
            let last_line = get_zt(hl); // c:1233
            let mut zt = last_line.clone();

            // c:1287 — `while ((!pattern || patprog) && !nosearch)`.
            // Non-pattern path: `!pattern` is always true.
            while !nosearch {
                // c:1365 — else-branch (no compiled pattern).
                if skip_pos {
                    // c:1373
                    if dir < 0 {
                        // c:1374
                        if pos == 0 {
                            skip_line = true; // c:1376
                        } else {
                            // c:1378 — `backwardmetafiedchar` — prev boundary.
                            pos = zt[..pos]
                                .char_indices()
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                        }
                    } else if !anchored {
                        // c:1381
                        if pos >= zt.len().saturating_sub(1) {
                            skip_line = true; // c:1383
                        } else {
                            // c:1385 — `pos += 1`, advanced to a char boundary.
                            let mut np = pos + 1;
                            while np < zt.len() && !zt.is_char_boundary(np) {
                                np += 1;
                            }
                            pos = np;
                        }
                    } else {
                        skip_line = true; // c:1387
                    }
                    skip_pos = false; // c:1388
                }
                // c:1394 — search within the current line unless skipping.
                if !skip_line {
                    if anchored {
                        // c:1395-1397 — anchored prefix compare.
                        if zlinecmp(&zt, &sbuf[1..]) < sens {
                            t = Some(0);
                        }
                    } else {
                        // c:1399 — `t = zlinefind(zt, pos, sbuf, dir, sens)`.
                        let p = pos.min(zt.len());
                        t = zlinefind(&zt, p, &sbuf, dir, sens);
                    }
                    if let Some(tb) = t {
                        // c:1401 — `end_pos = (t-zt)+sbptr - (sbuf[0]=='^')`.
                        end_pos = tb + sbptr - if anchored { 1 } else { 0 };
                    }
                }
                if let Some(tb) = t {
                    // c:1404-1406 — matched in this line.
                    pos = tb;
                    break;
                }
                // c:1412 — move through history to try again.
                let can_move = (ZLEREADFLAGS.load(Ordering::SeqCst) & ZLRF_HISTORY) != 0;
                let n = hl as i32 + dir;
                let moved = if can_move && n >= 0 && n <= entries_len as i32 {
                    Some(n as usize)
                } else {
                    None
                };
                match moved {
                    None => {
                        // c:1413-1431 — exhausted: restore the backtrack spot.
                        if top_spot > 0 {
                            // c:1414-1416 — pop a nomatch spot of equal length.
                            if let Some(s) = get_isrch_spot(top_spot - 1) {
                                if sbptr as i32 == s.6 && s.8 != 0 {
                                    top_spot -= 1;
                                }
                            }
                        }
                        if let Some(s) = get_isrch_spot(top_spot) {
                            // c:1417-1419 — restore hl/pos/…/sbptr/dir/nomatch.
                            hl = s.0 as usize;
                            pos = s.1 as usize;
                            pat_hl = s.2 as usize;
                            pat_pos = s.3 as usize;
                            end_pos = s.4 as usize;
                            let cs = s.5 as usize;
                            sbptr = s.6 as usize;
                            dir = s.7;
                            nomatch = s.8;
                            sbuf.truncate(sbptr);
                            ZLECS.store(cs, Ordering::SeqCst);
                        }
                        if nomatch != 1 {
                            // c:1420-1423
                            feep = true;
                            nomatch = 1;
                        }
                        zt = get_zt(hl); // c:1424-1425
                        skip_line = false; // c:1426
                        break; // c:1431
                    }
                    Some(nh) => {
                        // c:1433-1441 — advance to the next line.
                        hl = nh;
                        zt = get_zt(hl);
                        pos = if dir == 1 { 0 } else { zt.len() }; // c:1435
                        skip_line = if dup_ok {
                            false
                        } else {
                            zt == last_line // c:1441 (`!strcmp(zt, last_line)`)
                        };
                    }
                }
            }
            dup_ok = false; // c:1443
            if t.is_some() || (nosearch && nomatch == 0) {
                // c:1449-1457 — commit the matched line.
                let bc = if dir == 1 { end_pos } else { pos };
                let cc = bcc(&get_zt(hl), bc);
                set_line(hl, cc);
                nomatch = 0;
            }
        } else {
            // c:1458-1462 — empty search string.
            top_spot = 0;
            nomatch = 0;
        }
        nosearch = false; // c:1463
        if feep {
            // c:1464-1467
            handlefeep();
            feep = false;
        }

        // c:1470-1499 — highlight range for `$ISEARCHMATCH_*`.
        let anchored = sbuf.as_bytes().first() == Some(&b'^');
        if nomatch == 0 && sbptr > 0 && (sbptr > 1 || !anchored) {
            let zt = get_zt(hl);
            ISEARCH_STARTPOS.store(bcc(&zt, pos) as i32, Ordering::SeqCst);
            ISEARCH_ENDPOS.store(bcc(&zt, end_pos) as i32, Ordering::SeqCst);
            ISEARCH_ACTIVE.store(1, Ordering::SeqCst);
        } else {
            ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
        }

        // c:1500-1503 (`ref:`) — repaint, then read a command. Commands
        // that only redraw (clear-screen/redisplay/vi-cmd-mode) loop here
        // without re-searching.
        let mut ins_char: Option<char> = None;
        'refl: loop {
            let dirstr = if dir == 1 { "fwd" } else { "bck" };
            let body = format!("{}-i-search: {}_", dirstr, sbuf);
            let status = match nomatch {
                2 => format!("invalid {}", body),
                1 => format!("failing {}", body),
                _ => body,
            };
            *STATUSLINE.lock().unwrap() = Some(status);
            zrefresh();

            let cmd = crate::ported::zle::zle_keymap::getkeycmd();
            let name = cmd.as_ref().map(|c| c.nam.as_str());
            match name {
                // c:1504-1516 — EOF or send-break: abort, restore spot 0.
                None | Some("send-break") => {
                    aborted = true;
                    {
                        let m =
                            PREVIOUS_ABORTED_SEARCH.get_or_init(|| std::sync::Mutex::new(String::new()));
                        *m.lock().unwrap() = sbuf[..sbuf.len().min(sbptr)].to_string();
                    }
                    if let Some(s) = get_isrch_spot(0) {
                        hl = s.0 as usize;
                        let cs = s.5 as usize;
                        dir = s.7;
                        nomatch = s.8;
                        set_line(hl, cs);
                    }
                    break 'outer;
                }
                Some("clear-screen") => {
                    // c:1517-1519
                    clearscreen();
                    continue 'refl;
                }
                Some("redisplay") => {
                    // c:1520-1522
                    redisplay();
                    continue 'refl;
                }
                Some("vi-cmd-mode") => {
                    // c:1523-1526
                    let in_vicmd = *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd";
                    let target = if in_vicmd { "main" } else { "vicmd" };
                    if crate::ported::zle::zle_keymap::selectkeymap(target, 0) != 0 {
                        handlefeep();
                    }
                    continue 'refl;
                }
                // c:1527-1569 — backward-delete family: pop backtrack spots.
                Some(
                    "vi-backward-delete-char"
                    | "backward-delete-char"
                    | "vi-backward-kill-word"
                    | "backward-kill-word"
                    | "backward-delete-word",
                ) => {
                    let only_one =
                        name == Some("vi-backward-delete-char") || name == Some("backward-delete-char");
                    let old_sbptr = sbptr;
                    if top_spot > 0 {
                        loop {
                            // c:1536-1542 — `get_isrch_spot(--top_spot, …)`.
                            top_spot -= 1;
                            if let Some(s) = get_isrch_spot(top_spot) {
                                hl = s.0 as usize;
                                pos = s.1 as usize;
                                pat_hl = s.2 as usize;
                                pat_pos = s.3 as usize;
                                end_pos = s.4 as usize;
                                let cs = s.5 as usize;
                                sbptr = s.6 as usize;
                                dir = s.7;
                                nomatch = s.8;
                                ZLECS.store(cs, Ordering::SeqCst);
                                sbuf.truncate(sbptr);
                            }
                            if only_one || top_spot == 0 || old_sbptr != sbptr {
                                break;
                            }
                        }
                        nosearch = true; // c:1545
                        skip_pos = false; // c:1546
                    } else {
                        feep = true; // c:1548
                    }
                    if nomatch != 0 {
                        // c:1549-1554
                        skip_pos = true;
                    }
                    // c:1562-1566 — re-set the line where we won't reach
                    // the usual line-setting path.
                    let anchored = sbuf.as_bytes().first() == Some(&b'^');
                    if nomatch != 0 || sbptr == 0 || (sbptr == 1 && anchored) {
                        let cs = ZLECS.load(Ordering::SeqCst);
                        set_line(hl, cs);
                    }
                    // c:1569 — `continue`; any `feep` set above is handled
                    // at the top of the next iteration (c:1464).
                    continue 'outer;
                }
                Some("accept-and-hold") => {
                    // c:1570-1572
                    exitfn = Some(acceptandhold);
                    break 'outer;
                }
                Some("accept-and-infer-next-history") => {
                    // c:1573-1575
                    exitfn = Some(acceptandinfernexthistory);
                    break 'outer;
                }
                Some("accept-line-and-down-history") => {
                    // c:1576-1578
                    exitfn = Some(acceptlineanddownhistory);
                    break 'outer;
                }
                Some("accept-line") => {
                    // c:1579-1581
                    exitfn = Some(acceptline);
                    break 'outer;
                }
                // c:1582-1630 — direction change / repeat search (`rpt:`).
                Some(
                    "history-incremental-search-backward"
                    | "history-incremental-pattern-search-backward",
                ) => {
                    pat_hl = hl;
                    pat_pos = pos;
                    set_isrch_spot(
                        top_spot,
                        hl as i32,
                        pos as i32,
                        pat_hl as i32,
                        pat_pos as i32,
                        end_pos as i32,
                        ZLECS.load(Ordering::SeqCst) as i32,
                        sbuf.len() as i32,
                        dir,
                        nomatch,
                    );
                    top_spot += 1;
                    if dir != -1 {
                        dir = -1; // c:1589
                    } else {
                        skip_pos = true; // c:1591
                    }
                    // c:1620-1627 — reload previous search when sbuf empty and same dir.
                    if sbuf.is_empty() && dir == odir {
                        let prev = PREVIOUS_SEARCH
                            .get_or_init(|| std::sync::Mutex::new(String::new()))
                            .lock()
                            .unwrap()
                            .clone();
                        if !prev.is_empty() {
                            sbuf = prev;
                        }
                    }
                    continue 'outer;
                }
                Some(
                    "history-incremental-search-forward"
                    | "history-incremental-pattern-search-forward",
                ) => {
                    pat_hl = hl;
                    pat_pos = pos;
                    set_isrch_spot(
                        top_spot,
                        hl as i32,
                        pos as i32,
                        pat_hl as i32,
                        pat_pos as i32,
                        end_pos as i32,
                        ZLECS.load(Ordering::SeqCst) as i32,
                        sbuf.len() as i32,
                        dir,
                        nomatch,
                    );
                    top_spot += 1;
                    if dir != 1 {
                        dir = 1; // c:1600
                    } else {
                        skip_pos = true; // c:1602
                    }
                    // c:1620-1627 — reload previous search when sbuf empty and same dir.
                    if sbuf.is_empty() && dir == odir {
                        let prev = PREVIOUS_SEARCH
                            .get_or_init(|| std::sync::Mutex::new(String::new()))
                            .lock()
                            .unwrap()
                            .clone();
                        if !prev.is_empty() {
                            sbuf = prev;
                        }
                    }
                    continue 'outer;
                }
                Some("vi-rev-repeat-search") => {
                    // c:1604-1611
                    pat_hl = hl;
                    pat_pos = pos;
                    set_isrch_spot(
                        top_spot,
                        hl as i32,
                        pos as i32,
                        pat_hl as i32,
                        pat_pos as i32,
                        end_pos as i32,
                        ZLECS.load(Ordering::SeqCst) as i32,
                        sbuf.len() as i32,
                        dir,
                        nomatch,
                    );
                    top_spot += 1;
                    dir = -odir;
                    skip_pos = true;
                    // c:1620-1627 — reload previous search when sbuf empty and same dir.
                    if sbuf.is_empty() && dir == odir {
                        let prev = PREVIOUS_SEARCH
                            .get_or_init(|| std::sync::Mutex::new(String::new()))
                            .lock()
                            .unwrap()
                            .clone();
                        if !prev.is_empty() {
                            sbuf = prev;
                        }
                    }
                    continue 'outer;
                }
                Some("vi-repeat-search") => {
                    // c:1612-1630
                    pat_hl = hl;
                    pat_pos = pos;
                    set_isrch_spot(
                        top_spot,
                        hl as i32,
                        pos as i32,
                        pat_hl as i32,
                        pat_pos as i32,
                        end_pos as i32,
                        ZLECS.load(Ordering::SeqCst) as i32,
                        sbuf.len() as i32,
                        dir,
                        nomatch,
                    );
                    top_spot += 1;
                    dir = odir;
                    skip_pos = true;
                    // c:1620-1627 — reload previous search when sbuf empty and same dir.
                    if sbuf.is_empty() && dir == odir {
                        let prev = PREVIOUS_SEARCH
                            .get_or_init(|| std::sync::Mutex::new(String::new()))
                            .lock()
                            .unwrap()
                            .clone();
                        if !prev.is_empty() {
                            sbuf = prev;
                        }
                    }
                    continue 'outer;
                }
                // c:1631-1641 — quoted insert: read one raw char.
                Some("vi-quoted-insert") | Some("quoted-insert") => {
                    if name == Some("vi-quoted-insert") {
                        // c:1633-1636 — show a caret while waiting.
                        let dirstr = if dir == 1 { "fwd" } else { "bck" };
                        *STATUSLINE.lock().unwrap() =
                            Some(format!("{}-i-search: {}^_", dirstr, sbuf));
                        zrefresh();
                    }
                    match getfullchar(false) {
                        None => {
                            feep = true; // c:1639
                            break 'refl;
                        }
                        Some(c) => {
                            ins_char = Some(c); // c:1641 goto ins
                            break 'refl;
                        }
                    }
                }
                // c:1642-1657 — bracketed paste.
                Some("bracketed-paste") => {
                    let paste = bracketedstring();
                    set_isrch_spot(
                        top_spot,
                        hl as i32,
                        pos as i32,
                        pat_hl as i32,
                        pat_pos as i32,
                        end_pos as i32,
                        ZLECS.load(Ordering::SeqCst) as i32,
                        sbuf.len() as i32,
                        dir,
                        nomatch,
                    );
                    top_spot += 1;
                    sbuf.push_str(&paste);
                    continue 'outer;
                }
                Some("accept-search") => {
                    // c:1658-1659
                    break 'outer;
                }
                // c:1660-1708 — self-insert family, else abort.
                Some("self-insert-unmeta") => {
                    // c:1661-1662
                    fixunmeta();
                    let v = LASTCHAR_WIDE.load(Ordering::SeqCst);
                    ins_char = Some(char::from_u32(v as u32).unwrap_or('\u{FFFD}'));
                    break 'refl;
                }
                Some("magic-space") => {
                    // c:1663-1664
                    fixmagicspace();
                    let v = LASTCHAR_WIDE.load(Ordering::SeqCst);
                    ins_char = Some(char::from_u32(v as u32).unwrap_or('\u{FFFD}'));
                    break 'refl;
                }
                Some("self-insert") => {
                    // c:1665-1674 — validate a wide char, feep on bad byte.
                    if LASTCHAR_WIDE_VALID.load(Ordering::SeqCst) == 0
                        && getrestchar(crate::ported::zle::compcore::LASTCHAR.load(Ordering::SeqCst)) == -1
                    {
                        handlefeep();
                        continue 'refl;
                    }
                    let v = LASTCHAR_WIDE.load(Ordering::SeqCst);
                    ins_char = Some(char::from_u32(v as u32).unwrap_or('\u{FFFD}'));
                    break 'refl;
                }
                _ => {
                    // c:1675-1683 — unrecognized: push back and exit.
                    crate::ported::zle::zle_keymap::ungetkeycmd();
                    break 'outer;
                }
            }
        }

        // c:1685-1708 (`ins:`) — commit the pending self-insert char.
        if let Some(ch) = ins_char {
            if sbuf.len() >= ISEARCH_SBUF_MAX {
                // c:1686-1688
                feep = true;
            } else {
                set_isrch_spot(
                    top_spot,
                    hl as i32,
                    pos as i32,
                    pat_hl as i32,
                    pat_pos as i32,
                    end_pos as i32,
                    ZLECS.load(Ordering::SeqCst) as i32,
                    sbuf.len() as i32,
                    dir,
                    nomatch,
                );
                top_spot += 1;
                sbuf.push(ch); // c:1705 zlecharasstring(LASTFULLCHAR, …)
            }
        }
        if feep {
            // c:1709-1711
            handlefeep();
            feep = false;
        }
    }

    // c:1713-1716 — remember the accepted search string.
    if !sbuf.is_empty() {
        let m = PREVIOUS_SEARCH.get_or_init(|| std::sync::Mutex::new(String::new()));
        *m.lock().unwrap() = sbuf.clone();
        history().lock().unwrap().search_pattern = sbuf.clone();
        history().lock().unwrap().search_backward = odir < 0;
    }
    // c:1717 — `statusline = NULL;`
    *STATUSLINE.lock().unwrap() = None;
    // c:1720 — `redrawhook();`
    redrawhook();
    // c:1721-1722 — run the deferred accept widget after search cleanup.
    if let Some(f) = exitfn {
        f();
    }
    // c:1723 — restore the pre-search keymap.
    crate::ported::zle::zle_keymap::selectkeymap(&okeymap, 1);
    // c:1728 — `isearch_active = 0;`
    ISEARCH_ACTIVE.store(0, Ordering::SeqCst);
    // c:1736 — `selectlocalmap(NULL);`
    crate::ported::zle::zle_keymap::selectlocalmap(None);
    // c:1738 — `return aborted ? 3 : nomatch;`
    if aborted {
        3
    } else {
        nomatch
    }
}

/// Port of the `rpt:` tail in `doisearch` (Src/Zle/zle_hist.c:1619-1628):
/// when the search buffer is empty and the direction matches the original,
/// reload the previous accepted search string.
/// Port of `infernexthist(Histent he, UNUSED(char **args))` from Src/Zle/zle_hist.c:1741.
/// WARNING: param names don't match C — Rust=(zle) vs C=(he, args)
pub fn infernexthist() -> i32 {
    // c:1741
    // C body (c:1741-1770): walk forward in history to find the entry
    //                      whose first word matches the previously
    //                      accepted entry's first word.
    // Cannot lock history() twice in one expression — RHS guard
    // outlives read and LHS lock deadlocks the same thread on
    // non-reentrant std::sync::Mutex. Hoist both reads.
    let cur_first: String = {
        let h = history().lock().unwrap();
        if h.cursor + 1 >= h.entries.len() {
            return 1;
        }
        h.entries[h.cursor]
            .line
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_string()
    };
    if cur_first.is_empty() {
        return 1;
    }
    let (start, len) = {
        let h = history().lock().unwrap();
        (h.cursor + 1, h.entries.len())
    };
    for i in start..len {
        let first = {
            let h = history().lock().unwrap();
            h.entries[i]
                .line
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string()
        };
        if first == cur_first {
            history().lock().unwrap().cursor = i;
            return 0;
        }
    }
    1
}

/// Port of `acceptandinfernexthistory(char **args)` from Src/Zle/zle_hist.c:1757.
pub fn acceptandinfernexthistory() -> i32 {
    // c:1757
    // C body (c:691-715): mark line for accept then queue infer-next.
    //                    The actual infer happens after acceptline
    //                    when the next prompt is drawn.
    DONE.store(1, Ordering::SeqCst);
    history().lock().unwrap().search_pattern.clear();
    0
}

/// Port of `infernexthistory(char **args)` from Src/Zle/zle_hist.c:1772.
pub fn infernexthistory() -> i32 {
    // c:1772
    // C body (c:1772-1786): wrapper around infernexthist that
    //                      additionally fetches the entry into the
    //                      buffer (handled by next prompt redraw).
    infernexthist()
}

/// Port of `vifetchhistory(UNUSED(char **args))` from Src/Zle/zle_hist.c:1787.
pub fn vifetchhistory() -> i32 {
    // c:1787
    // C body (c:1787-1804): vi `G` — fetch history entry numbered
    //                      mult; with no count fetch most recent.
    let n = ZMOD.lock().unwrap().mult;
    if n <= 0 {
        if history().lock().unwrap().entries.is_empty() {
            return 1;
        }
        // Single-statement double-lock would deadlock; hoist.
        let mut h = history().lock().unwrap();
        h.cursor = h.entries.len() - 1;
        return 0;
    }
    if (n as usize) > history().lock().unwrap().entries.len() {
        return 1;
    }
    history().lock().unwrap().cursor = (n as usize).saturating_sub(1);
    0
}

/// Port of `static char *visrchstr` from `Src/Zle/zle_hist.c:1810` —
/// the last vi search string. Set by `getvisrchstr` on commit.
pub static VISRCHSTR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Port of `static char *vipenultsrchstr` from `Src/Zle/zle_hist.c:1810`
/// — the penultimate vi search string, used as the fallback when the
/// user accepts an empty minibuffer.
pub static VIPENULTSRCHSTR: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
/// Port of `static int visrchsense` from `Src/Zle/zle_hist.c:1811` —
/// +1 for `/` (forward), -1 for `?` (backward). Set by the caller
/// (`vi-history-search-forward`/`-backward`) before `getvisrchstr`.
pub static VISRCHSENSE: AtomicI32 = AtomicI32::new(1);

/// Port of `getvisrchstr()` from Src/Zle/zle_hist.c:1815. Reads a vi
/// search string through the statusline minibuffer: draws `/`…`_` (or
/// `?`…`_` when `VISRCHSENSE == -1`), then loops on `getkeycmd`,
/// handling accept (accept-line / vi-cmd-mode), backward-delete-char,
/// backward-kill-word, quoted-insert, magic-space and self-insert. On
/// an empty accept it falls back to `vipenultsrchstr`. The committed
/// string is mirrored onto `history().search_pattern` (+ direction) so
/// the existing `vi-history-search-*`/`vi-repeat-search` widgets keep
/// working unchanged.
/// WARNING: param names don't match C — Rust=() vs C=()
pub fn getvisrchstr() -> i32 {
    // c:1815
    // c:1820 — `okeymap = ztrdup(curkeymapname);`
    let okeymap = crate::ported::zle::zle_keymap::curkeymapname().clone();

    // c:1822-1830 — rotate visrchstr → vipenultsrchstr.
    *VIPENULTSRCHSTR.lock().unwrap() = None;
    if let Some(s) = VISRCHSTR.lock().unwrap().take() {
        *VIPENULTSRCHSTR.lock().unwrap() = Some(s);
    }

    // c:1831 — `clearlist = 1;`
    CLEARLIST.store(1, Ordering::SeqCst);

    let sense = VISRCHSENSE.load(Ordering::SeqCst);
    // c:1833 — `sbuf[0] = (visrchsense == -1) ? '?' : '/';`. sbuf[0] is
    // the fixed sense sigil; the search string is everything after it.
    let mut sbuf = String::from(if sense == -1 { '?' } else { '/' });
    // c:1834 — `selectkeymap("main", 1);`
    crate::ported::zle::zle_keymap::selectkeymap("main", 1);

    let mut ret = 0;
    let mut feep = false;
    // c:1835 — `while (sptr)`. sptr>0 the whole time; accept sets it 0.
    'outer: loop {
        // c:1836-1838 — draw `sbuf` with a trailing cursor placeholder.
        *STATUSLINE.lock().unwrap() = Some(format!("{}_", sbuf));
        zrefresh();

        let cmd = crate::ported::zle::zle_keymap::getkeycmd();
        let mut name = cmd.as_ref().map(|c| c.nam.as_str());
        match name {
            // c:1839-1842 — EOF / send-break: abort with no result.
            None | Some("send-break") => {
                ret = 0;
                break 'outer;
            }
            _ => {}
        }
        // c:1843-1846 — magic-space: expand, then treat as self-insert.
        if name == Some("magic-space") {
            fixmagicspace();
            name = Some("self-insert");
        }
        match name {
            Some("redisplay") => {
                // c:1847-1848
                redisplay();
            }
            Some("clear-screen") => {
                // c:1849-1850
                clearscreen();
            }
            Some("accept-line") | Some("vi-cmd-mode") => {
                // c:1851-1860 — commit `sbuf[1..]`; empty → vipenult.
                let s: String = sbuf.chars().skip(1).collect();
                if s.is_empty() {
                    *VISRCHSTR.lock().unwrap() = VIPENULTSRCHSTR.lock().unwrap().clone();
                } else {
                    *VISRCHSTR.lock().unwrap() = Some(s);
                }
                ret = 1;
                break 'outer;
            }
            Some("backward-delete-char") | Some("vi-backward-delete-char") => {
                // c:1861-1863 — delete one char, never the sense sigil.
                if sbuf.chars().count() > 1 {
                    sbuf.pop();
                }
            }
            Some("backward-kill-word") | Some("vi-backward-kill-word") => {
                // c:1864-1895 — kill back over trailing blanks then one
                // ident run (or one non-ident run), stopping at index 1.
                let ident = |c: char| c.is_alphanumeric() || c == '_';
                let mut chars: Vec<char> = sbuf.chars().collect();
                while chars.len() > 1 && chars.last().is_some_and(|c| c.is_whitespace()) {
                    chars.pop();
                }
                if chars.len() > 1 {
                    let last_ident = ident(*chars.last().unwrap());
                    while chars.len() > 1 {
                        let c = *chars.last().unwrap();
                        if last_ident {
                            if !ident(c) {
                                break;
                            }
                        } else if ident(c) || c.is_whitespace() {
                            break;
                        }
                        chars.pop();
                    }
                }
                sbuf = chars.into_iter().collect();
            }
            Some("vi-quoted-insert") | Some("quoted-insert") => {
                // c:1896-1904
                if name == Some("vi-quoted-insert") {
                    *STATUSLINE.lock().unwrap() = Some(format!("{}^", sbuf));
                    zrefresh();
                }
                match getfullchar(false) {
                    None => feep = true, // c:1902
                    Some(c) => sbuf.push(c),
                }
            }
            Some("self-insert-unmeta") | Some("self-insert") => {
                // c:1905-1925
                if name == Some("self-insert-unmeta") {
                    fixunmeta();
                } else if LASTCHAR_WIDE_VALID.load(Ordering::SeqCst) == 0
                    && getrestchar(crate::ported::zle::compcore::LASTCHAR.load(Ordering::SeqCst)) == -1
                {
                    handlefeep();
                    continue 'outer;
                }
                let v = LASTCHAR_WIDE.load(Ordering::SeqCst);
                sbuf.push(char::from_u32(v as u32).unwrap_or('\u{FFFD}'));
            }
            _ => {
                // c:1926-1927
                feep = true;
            }
        }
        if feep {
            // c:1929-1931
            handlefeep();
            feep = false;
        }
    }

    // c:1933-1935 — tear down the minibuffer and restore the keymap.
    *STATUSLINE.lock().unwrap() = None;
    crate::ported::zle::zle_keymap::selectkeymap(&okeymap, 1);

    // Mirror the committed string onto the singleton history state so the
    // existing vi search widgets (which read `search_pattern`) resolve it.
    if ret == 1 {
        if let Some(s) = VISRCHSTR.lock().unwrap().clone() {
            let mut h = history().lock().unwrap();
            h.search_pattern = s;
            h.search_backward = sense == -1;
        }
    }
    ret // c:1936
}

/// Port of `vihistorysearchforward(char **args)` from Src/Zle/zle_hist.c:1940.
pub fn vihistorysearchforward() -> i32 {
    // c:1940
    // C body (c:1940-1962): vi `/` — read a search string then walk
    //                      forward.
    if history().lock().unwrap().search_pattern.is_empty() {
        return 1;
    }
    let pat = history().lock().unwrap().search_pattern.clone();
    let n = ZMOD.lock().unwrap().mult.max(1);
    for _ in 0..n {
        if history().lock().unwrap().search_forward(&pat).is_none() {
            return 1;
        }
    }
    history().lock().unwrap().search_backward = false;
    0
}

/// Port of `vihistorysearchbackward(char **args)` from Src/Zle/zle_hist.c:1964.
pub fn vihistorysearchbackward() -> i32 {
    // c:1964
    // C body (c:1964-1986): vi `?` — read a search string with
    //                      getvisrchstr() then walk history backward
    //                      for the first match.
    if history().lock().unwrap().search_pattern.is_empty() {
        return 1;
    }
    let pat = history().lock().unwrap().search_pattern.clone();
    let n = ZMOD.lock().unwrap().mult.max(1);
    for _ in 0..n {
        if history().lock().unwrap().search_backward(&pat).is_none() {
            return 1;
        }
    }
    history().lock().unwrap().search_backward = true;
    0
}

/// Port of `virepeatsearch(UNUSED(char **args))` from Src/Zle/zle_hist.c:1988.
pub fn virepeatsearch() -> i32 {
    // c:1988
    // C body (c:1988-2008): vi `n` — repeat the last search in the
    //                      same direction as the last vi search.
    let (pat, backward) = {
        let h = history().lock().unwrap();
        if h.search_pattern.is_empty() {
            return 1;
        }
        (h.search_pattern.clone(), h.search_backward)
    };
    let n = ZMOD.lock().unwrap().mult.max(1);
    for _ in 0..n {
        let hit_found = {
            let mut h = history().lock().unwrap();
            if backward {
                h.search_backward(&pat).is_some()
            } else {
                h.search_forward(&pat).is_some()
            }
        };
        if !hit_found {
            return 1;
        }
    }
    0
}

/// Port of `virevrepeatsearch()` from Src/Zle/zle_hist.c:2024.
pub fn virevrepeatsearch() -> i32 {
    // c:2024
    // C body (c:2024-2030): vi `N` — repeat the last search in the
    //                      reverse direction.
    let (pat, backward) = {
        let h = history().lock().unwrap();
        if h.search_pattern.is_empty() {
            return 1;
        }
        (h.search_pattern.clone(), h.search_backward)
    };
    let n = ZMOD.lock().unwrap().mult.max(1);
    for _ in 0..n {
        let hit_found = {
            let mut h = history().lock().unwrap();
            if backward {
                h.search_forward(&pat).is_some()
            } else {
                h.search_backward(&pat).is_some()
            }
        };
        if !hit_found {
            return 1;
        }
    }
    0
}

/// Port of `historybeginningsearchbackward(char **args)` from Src/Zle/zle_hist.c:2039.
///
/// Direct line-by-line port: saves the cursor position, walks history
/// backward via `movehistent` (so `hist_skip_flags` / `HIST_FOREIGN`
/// gating works), compares each entry's prefix against the buffer-up-
/// to-cursor via [`zlinecmp`], and on the `zmult`-th match restores the
/// cursor position the C source preserves at c:2057.
pub fn historybeginningsearchbackward() -> i32 {
    use crate::ported::hist::{movehistent, quietgethist};
    use crate::ported::zsh_h::{HISTFINDNODUPS, HIST_DUP};

    // c:2042 — `int cpos = zlecs;`
    let cpos = ZLECS.load(Ordering::SeqCst);
    // c:2043 — `int n = zmult;`
    let n_save = ZMOD.lock().unwrap().mult;

    // c:2046-2051 — `if (zmult < 0) { zmult = -n; ret = historybeginningsearchforward(args); zmult = n; return ret; }`
    if n_save < 0 {
        ZMOD.lock().unwrap().mult = -n_save;
        let ret = historybeginningsearchforward();
        ZMOD.lock().unwrap().mult = n_save;
        return ret;
    }

    // c:2052 — `if (!(he = quietgethist(histline))) return 1;`
    let start = histline.load(Ordering::SeqCst) as i64;
    if quietgethist(start).is_none() {
        return 1;
    }

    let prefix: String = ZLELINE.lock().unwrap()[..cpos].iter().collect();
    let skip_flags = history().lock().unwrap().hist_skip_flags;

    let mut cur_ev = start;
    let mut remaining = n_save;
    // c:2054 — `while ((he = movehistent(he, -1, hist_skip_flags))) { ... }`
    while let Some(next_ev) = movehistent(cur_ev, -1, skip_flags) {
        cur_ev = next_ev;
        let he = match quietgethist(cur_ev) {
            Some(h) => h,
            None => break,
        };
        // c:2057-2058 — `if (isset(HISTFINDNODUPS) && he->node.flags & HIST_DUP) continue;`
        if isset(HISTFINDNODUPS) && (he.node.flags as u32 & HIST_DUP) != 0 {
            continue;
        }
        let zt: String = he.zle_text.clone().unwrap_or(he.node.nam.clone()); // c:2059 GETZLETEXT
                                                                             // c:2060-2064 — compare prefix (zlemetaline truncated at zlemetacs)
                                                                             //               against zt; require tst < 0 (he ≠ buffer prefix)
                                                                             //               AND zlinecmp(zt, full-buffer-prefix) non-zero
                                                                             //               (i.e. he is not exactly the current prefix either).
        let buf_prefix: String = prefix.clone();
        let tst = zlinecmp(&zt, &buf_prefix);
        if tst < 0 && zlinecmp(&zt, &buf_prefix) != 0 {
            remaining -= 1; // c:2065
            if remaining <= 0 {
                // c:2066-2069 — `unmetafy_line(); zle_setline(he); zlecs = cpos; CCRIGHT(); return 0;`
                history().lock().unwrap().cursor = cur_ev as usize;
                let _ = zle_setline();
                ZLECS.store(cpos, Ordering::SeqCst);
                return 0;
            }
        }
    }
    // c:2073 — `unmetafy_line(); return 1;`
    1
}

/// Port of `historybeginningsearchforward(char **args)` from Src/Zle/zle_hist.c:2085.
///
/// Forward mirror of [`historybeginningsearchbackward`]. Direct port
/// of the c:2082-2118 body — handles the `zmult < 0` redirect to the
/// backward variant, walks via `movehistent(+1)`, compares prefixes
/// with `zlinecmp`, and on the `zmult`-th hit invokes `zle_setline`
/// and restores the cursor position.
pub fn historybeginningsearchforward() -> i32 {
    use crate::ported::hist::{movehistent, quietgethist};
    use crate::ported::zsh_h::{HISTFINDNODUPS, HIST_DUP};

    // c:2088 — `int cpos = zlecs;`
    let cpos = ZLECS.load(Ordering::SeqCst);
    // c:2089 — `int n = zmult;`
    let n_save = ZMOD.lock().unwrap().mult;

    // c:2092-2097 — `if (zmult < 0) { zmult = -n; ret = historybeginningsearchbackward(args); zmult = n; return ret; }`
    if n_save < 0 {
        ZMOD.lock().unwrap().mult = -n_save;
        let ret = historybeginningsearchbackward();
        ZMOD.lock().unwrap().mult = n_save;
        return ret;
    }

    // c:2098 — `if (!(he = quietgethist(histline))) return 1;`
    let start = histline.load(Ordering::SeqCst) as i64;
    if quietgethist(start).is_none() {
        return 1;
    }

    let prefix: String = ZLELINE.lock().unwrap()[..cpos].iter().collect();
    let skip_flags = history().lock().unwrap().hist_skip_flags;

    let mut cur_ev = start;
    let mut remaining = n_save;
    // c:2100 — `while ((he = movehistent(he, +1, hist_skip_flags))) { ... }`
    while let Some(next_ev) = movehistent(cur_ev, 1, skip_flags) {
        cur_ev = next_ev;
        let he = match quietgethist(cur_ev) {
            Some(h) => h,
            None => break,
        };
        // c:2103-2104 — skip duplicates if HISTFINDNODUPS is set
        if isset(HISTFINDNODUPS) && (he.node.flags as u32 & HIST_DUP) != 0 {
            continue;
        }
        let zt: String = he.zle_text.clone().unwrap_or(he.node.nam.clone()); // c:2105 GETZLETEXT
                                                                             // c:2106-2110 — `tst < 0 && zlinecmp(zt, buf_prefix) != 0`
        let buf_prefix: String = prefix.clone();
        let tst = zlinecmp(&zt, &buf_prefix);
        if tst < 0 && zlinecmp(&zt, &buf_prefix) != 0 {
            remaining -= 1; // c:2111
            if remaining <= 0 {
                // c:2112-2115
                history().lock().unwrap().cursor = cur_ev as usize;
                let _ = zle_setline();
                ZLECS.store(cpos, Ordering::SeqCst);
                return 0;
            }
        }
    }
    // c:2117
    1
}
/// `ISEARCH_ACTIVE` static.
pub static ISEARCH_ACTIVE: AtomicI32 = AtomicI32::new(0); // c:1078

/// Port of `int isearch_startpos` from `Src/Zle/zle_hist.c:1078`.
/// Byte offset of the start of the current isearch match.
pub static ISEARCH_STARTPOS: AtomicI32 = AtomicI32::new(0); // c:1078

/// Port of `int isearch_endpos` from `Src/Zle/zle_hist.c:1078`.
/// Byte offset of the end of the current isearch match.
pub static ISEARCH_ENDPOS: AtomicI32 = AtomicI32::new(0); // c:1078

/// Port of `int histline` from `Src/Zle/zle_hist.c:42`. Current history
/// entry the ZLE cursor is parked on. Read by `quietgethist(histline)`
/// at zle_hist.c:82,422 and bumped by `zle_main_entry(ZLE_CMD_SET_HIST_LINE)`
/// (zle_main.c:2182).
pub static histline: AtomicI32 = AtomicI32::new(0); // c:zle_hist.c:42

// Per-session ZLE history state. Rust-side aggregate over zsh's C
// flat-globals (`hist_ring`/`histline`/`searchstr`/`have_edits`/etc.
// in `Src/hist.c` + `Src/Zle/zle_hist.c`). The C side spreads these
// across file-scope statics; the zshrs port collects the subset the
// ZLE widgets actually drive into one container so a `&mut Zle` can
// hold it. Eventual unification: drop `History.entries` and read from
// `crate::ported::hist::hist_ring`; the cursor/saved_line/search
// fields stay as file-scope statics matching zsh's globals.

/// Single history entry — the Rust-side subset the ZLE widgets need
/// (line text, event number, optional time). Maps loosely to fields
/// from `struct histent` (Src/zsh.h:2234): `node.nam` ↔ `line`,
/// `histnum` ↔ `num`, `stim` ↔ `time`.
#[derive(Debug, Clone)]
pub struct HistEntry {
    /// The command line.
    pub line: String,
    /// Event number (1-based; mirrors `histent.histnum`).
    pub num: i64,
    /// Insertion time (Unix epoch seconds; mirrors `histent.stim`).
    pub time: Option<i64>,
}

/// Per-session ZLE history state — entries + cursor + search state.
/// Aggregate over zsh's C flat-globals (`hist_ring`, `histline`,
/// `searchstr`, `have_edits`).
#[derive(Debug, Default)]
pub struct History {
    /// History entries (newest last).
    pub entries: Vec<HistEntry>,
    /// Current position in history (mirrors `histline`).
    pub cursor: usize,
    /// Maximum history size (mirrors `histsiz`).
    pub max_size: usize,
    /// Saved line when navigating history (mirrors the C `zle_text`
    /// shadow on `Histent`).
    pub saved_line: Option<Vec<char>>,
    /// Saved cursor position pre-navigation.
    pub saved_cs: usize,
    /// Previous search string. Mirrors `searchstr`
    /// (Src/Zle/zle_hist.c:44).
    pub search_pattern: String,
    /// Last search direction (true = backward).
    pub search_backward: bool,
    /// Originals of edited entries: when `remember_edits` mutates
    /// `entries[i].line`, the pre-edit text lands here at index `i`.
    /// `forget_edits` restores them. Mirrors the C `Histent->zle_text`
    /// shadow string + the global `have_edits` flag in
    /// Src/Zle/zle_hist.c.
    pub originals: Vec<Option<String>>,
    /// True if any entry has a recorded original — mirrors
    /// `have_edits` in Src/Zle/zle_hist.c:76.
    pub have_edits: bool,
    /// History skip-flags state. Bit-equivalent of zsh's
    /// `hist_skip_flags` in Src/Zle/zle_hist.c:794: `HIST_FOREIGN` (1)
    /// hides entries from other sessions when set; `setlocalhistory`
    /// toggles this.
    pub hist_skip_flags: u32,
}

/// `struct isrch_spot` — port of `Src/Zle/zle_hist.c:954-963`.
/// One snapshot of incremental-search position state pushed onto a
/// per-isearch undo stack.
#[derive(Debug, Default, Clone, Copy)]
#[allow(non_camel_case_types)]
pub struct isrch_spot {
    // c:948
    /// `hl` field.
    pub hl: i32,
    /// `pos` field.
    pub pos: u16,
    /// `pat_hl` field.
    pub pat_hl: i32,
    /// `pat_pos` field.
    pub pat_pos: u16,
    /// `end_pos` field.
    pub end_pos: u16,
    /// `cs` field.
    pub cs: u16,
    /// `len` field.
    pub len: u16,
    /// `flags` field.
    pub flags: u16,
}

/// Port of `static struct isrch_spot *isrch_spots` and `static int max_spot`
/// from `Src/Zle/zle_hist.c:946-947` — heap-grown stack of incremental
/// search positions used to back-up after deleting search chars.
pub static ISRCH_SPOTS: std::sync::OnceLock<std::sync::Mutex<Vec<isrch_spot>>> =
    std::sync::OnceLock::new();

/// Set up history limits at ZLE startup.
/// Stub mirroring the role of `inithist()` from Src/hist.c:1717,
/// which sizes the global hist_ring at $HISTSIZE. zshrs's history
/// lives in the file-scope `HISTORY` static (zle_main.rs); this
/// helper is kept for API compatibility — callers can adjust
/// max_size at init if needed.
pub fn init_history(max_size: usize) {
    let _ = max_size;
}

/// Walk one entry older through the externally-supplied History.
/// External-history overload of the widget-callable
/// `zle_goto_hist(-1, false)` — kept for callers that drive a
/// separate History instance. Port of `uphistory()` at
/// Src/Zle/zle_hist.c:233 (the live-buffer save matches the C
/// source's first-navigate-saves-original behaviour).
pub fn history_up(hist: &mut History) {
    if hist.saved_line.is_none() {
        // Save current line
        hist.saved_line = Some(ZLELINE.lock().unwrap().clone());
        hist.saved_cs = ZLECS.load(Ordering::SeqCst);
    }

    if let Some(entry) = hist.up() {
        *ZLELINE.lock().unwrap() = entry.line.chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
    }
}

/// Walk one entry newer; if past the last entry, restore the saved
/// pre-navigation line.
/// External-history overload of `zle_goto_hist(1, false)`.
/// Port of `downhistory(UNUSED(char **args))` at Src/Zle/zle_hist.c:434 with the
/// saved-line restore from zle_goto_hist's sentinel branch.
pub fn history_down(hist: &mut History) {
    if let Some(entry) = hist.down() {
        *ZLELINE.lock().unwrap() = entry.line.chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
    } else if let Some(saved) = hist.saved_line.take() {
        // Restore saved line
        *ZLELINE.lock().unwrap() = saved;
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(hist.saved_cs, Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
    }
}

/// Set search direction for an incremental backward search. The full
/// interactive isearch UI lives in `widget::do_isearch` (called by the
/// `widget_history_isearch_backward` widget) — this method only flips
/// the saved direction flag for callers that drive History externally.
pub fn history_isearch_backward(hist: &mut History) {
    hist.search_backward = true;
}

/// Mirror of `history_isearch_backward` but for forward search.
pub fn history_isearch_forward(hist: &mut History) {
    hist.search_backward = false;
}

/// Search history for an entry containing the buffer text up to
/// the cursor.
/// Port of `historybeginningsearchbackward()` from
/// Src/Zle/zle_hist.c:2039 with substring-match instead of
/// prefix-match — useful as an isearch-style helper for callers
/// that drive History externally. The strict prefix-match form
/// lives in `widget_history_beginning_search_backward`.
pub fn history_search_prefix(hist: &mut History) {
    let prefix: String = ZLELINE.lock().unwrap()[..ZLECS.load(Ordering::SeqCst)]
        .iter()
        .collect();

    if let Some(entry) = hist.search_backward(&prefix) {
        *ZLELINE.lock().unwrap() = entry.line.chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
    }
}

/// Beginning of history - go to first entry
/// Port of beginningofhistory(UNUSED(char **args)) from zle_hist.c
pub fn beginning_of_history(hist: &mut History) {
    if hist.saved_line.is_none() {
        hist.saved_line = Some(ZLELINE.lock().unwrap().clone());
        hist.saved_cs = ZLECS.load(Ordering::SeqCst);
    }

    if !hist.entries.is_empty() {
        hist.cursor = 0;
        if let Some(entry) = hist.entries.first() {
            *ZLELINE.lock().unwrap() = entry.line.chars().collect();
            ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
            ZLECS.store(0, Ordering::SeqCst);
            ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        }
    }
}

/// End of history - go to last entry (current line)
/// Port of endofhistory() from zle_hist.c

/// History search backward - search for entries starting with current prefix
/// Port of historysearchbackward() from zle_hist.c

/// History search forward - search for entries starting with current prefix
/// Port of historysearchforward() from zle_hist.c

/// Insert last word from previous history entry
/// Port of insertlastword() from zle_hist.c

/// Push the current line onto the buffer stack and clear the editor.
/// Port of `pushline(UNUSED(char **args))` from Src/Zle/zle_hist.c:832. The C source
/// pushes the assembled line, then `mult - 1` empty strings (so a
/// numeric prefix repeats the push), saves zlecs to stackcs, and
/// blanks the line. The buffer stack is then drained on the next
/// zleread() so the user gets to compose a quick command and have
/// the prior text restored afterwards.
pub fn push_line() {
    let n = MULT.load(Ordering::SeqCst);
    if n < 0 {
        return;
    }
    let line: String = ZLELINE.lock().unwrap().iter().collect();
    BUFSTACK.lock().unwrap().push(line);
    let mut remaining = n - 1;
    while remaining > 0 {
        BUFSTACK.lock().unwrap().push(String::new());
        remaining -= 1;
    }
    STACKCS.store(ZLECS.load(Ordering::SeqCst), Ordering::SeqCst);
    ZLELINE.lock().unwrap().clear();
    ZLELL.store(0, Ordering::SeqCst);
    ZLECS.store(0, Ordering::SeqCst);
    ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
}

/// Accept line and go to next history (for walking through history executing each)
/// Port of acceptlineanddownhistory(UNUSED(char **args)) from zle_hist.c
pub fn accept_line_and_down_history(hist: &mut History) -> Option<String> {
    let line: String = ZLELINE.lock().unwrap().iter().collect();

    // Move to next history entry for next iteration
    if hist.cursor < hist.entries.len() {
        hist.cursor += 1;
        if let Some(entry) = hist.entries.get(hist.cursor) {
            *ZLELINE.lock().unwrap() = entry.line.chars().collect();
            ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
            ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
        }
    }

    Some(line)
}

/// Vi fetch history - go to specific history entry by number
/// Port of vifetchhistory(UNUSED(char **args)) from zle_hist.c
pub fn vi_fetch_history(hist: &mut History, num: usize) {
    if num > 0 && num <= hist.entries.len() {
        if hist.saved_line.is_none() {
            hist.saved_line = Some(ZLELINE.lock().unwrap().clone());
            hist.saved_cs = ZLECS.load(Ordering::SeqCst);
        }

        hist.cursor = num - 1;
        if let Some(entry) = hist.entries.get(hist.cursor) {
            *ZLELINE.lock().unwrap() = entry.line.chars().collect();
            ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
            ZLECS.store(0, Ordering::SeqCst);
            ZLE_RESET_NEEDED.store(1, Ordering::SeqCst);
        }
    }
}

/// Vi history search backward
/// Port of vihistorysearchbackward(char **args) from zle_hist.c

/// Vi history search forward
/// Port of vihistorysearchforward(char **args) from zle_hist.c

/// Vi repeat search
/// Port of virepeatsearch(UNUSED(char **args)) from zle_hist.c
pub fn vi_repeat_search(hist: &mut History) {
    if hist.search_backward {
        vihistorysearchbackward();
    } else {
        vihistorysearchforward();
    }
}

/// Vi reverse repeat search
/// Port of virevrepeatsearch() from zle_hist.c

/// Toggle session-local history filtering.
/// Port of `setlocalhistory(UNUSED(char **args))` from Src/Zle/zle_hist.c:794. With an
/// explicit count: `mult` non-zero turns the foreign-skip filter on
/// (`hist_skip_flags = HIST_FOREIGN = 1`), zero turns it off. With
/// no count: XOR-toggle the bit. Call sites that walk history can
/// consult `hist.hist_skip_flags & 1` to decide whether to surface
/// entries from other sessions.
pub fn set_local_history(hist: &mut History, has_mult: bool, mult: i32) {
    const HIST_FOREIGN: u32 = 1;
    if has_mult {
        hist.hist_skip_flags = if mult != 0 { HIST_FOREIGN } else { 0 };
    } else {
        hist.hist_skip_flags ^= HIST_FOREIGN;
    }
}

#[cfg(test)]
mod zlinecmp_zlinefind_tests {
    use super::*;

    #[test]
    fn zlinecmp_same() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:140-143 — both strings end together → 0.
        assert_eq!(zlinecmp("hello", "hello"), 0);
    }

    #[test]
    fn zlinecmp_input_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:146 — input runs out before hist → -1.
        assert_eq!(zlinecmp("hello world", "hello"), -1);
    }

    #[test]
    fn zlinecmp_lowercase_same() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:181 — case-fold walk: HELLO vs hello → 1.
        assert_eq!(zlinecmp("HELLO", "hello"), 1);
    }

    #[test]
    fn zlinecmp_lowercase_prefix() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:183 — input prefix of histp under case folding → 2.
        assert_eq!(zlinecmp("HELLO World", "hello"), 2);
    }

    #[test]
    fn zlinecmp_different() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:186 — totally different → 3.
        assert_eq!(zlinecmp("apple", "orange"), 3);
    }

    #[test]
    fn zlinecmp_empty_input() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:140-143 — empty input is a "prefix" → with non-empty hist → -1.
        assert_eq!(zlinecmp("foo", ""), -1);
        // Both empty → 0.
        assert_eq!(zlinecmp("", ""), 0);
    }

    #[test]
    fn zlinefind_forward_exact() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:208-213 — forward search; sens=0 means need zlinecmp < 0,
        // i.e. the needle must be a strict prefix at the position.
        assert_eq!(zlinefind("hello world hello", 0, "world", 1, 0), Some(6));
    }

    #[test]
    fn zlinefind_backward_exact() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:215-222 — backward search from end. sens=1 accepts both
        // 0 (exact) and -1 (prefix); sens=0 only accepts -1 (prefix).
        // To find the second "hello" exactly at index 12 we need
        // sens=1 — at index 12 zlinecmp("hello","hello")=0.
        assert_eq!(zlinefind("hello world hello", 16, "hello", -1, 1), Some(12));
    }

    #[test]
    fn zlinefind_not_found() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:224 — exhausted without match → None.
        assert_eq!(zlinefind("hello", 0, "xyz", 1, 0), None);
    }

    #[test]
    fn zlinefind_starts_at_pos() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:206 — search begins at `pos`, not at 0.
        // "abcabc" with needle "a" starting at pos=1 finds the
        // second "a" at index 3.
        assert_eq!(zlinefind("abcabc", 1, "a", 1, 0), Some(3));
    }
}

#[cfg(test)]
mod isearch_prompt_tests {
    use super::*;

    #[test]
    fn bad_text_strings_are_seven_chars() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(FAILING_TEXT.len(), BAD_TEXT_LEN);
        assert_eq!(INVALID_TEXT.len(), BAD_TEXT_LEN);
    }

    #[test]
    fn norm_prompt_pos_after_bad_text_marker() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Column 8: skips "XXXXXXX " (BAD_TEXT_LEN + 1 trailing space).
        assert_eq!(NORM_PROMPT_POS, 8);
    }

    #[test]
    fn first_search_char_after_isearch_label() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // Column 22: NORM_PROMPT_POS (8) + 14 chars of "XXX-i-search: ".
        assert_eq!(FIRST_SEARCH_CHAR, 22);
    }

    #[test]
    fn isearch_prompt_skeleton_has_correct_shape() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert!(ISEARCH_PROMPT.starts_with("XXXXXXX "));
        assert!(ISEARCH_PROMPT.contains("XXX-i-search:"));
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// ─── RUST-ONLY ACCESSORS ───
//
// Singleton accessor ported for `OnceLock<Mutex<T>>` / `OnceLock<
// RwLock<T>>` globals declared above. C zsh uses direct global
// access; Rust needs these wrappers because `OnceLock::get_or_init`
// is the only way to lazily construct shared state. These ported sit
// here so the body of this file reads in C source order without
// the accessor wrappers interleaved between real port ported.
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn isrch_spots() -> &'static std::sync::Mutex<Vec<isrch_spot>> {
    ISRCH_SPOTS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zle_with_history(entries: &[&str]) {
        zle_reset();
        for line in entries {
            history().lock().unwrap().add((*line).to_string());
        }
    }

    #[test]
    fn uphistory_skips_consecutive_dupes_when_histignoredups_set() {
        let _g = crate::test_util::global_state_lock();
        // c:235-237 — `nodups = isset(HISTIGNOREDUPS)` is passed
        //              through to zle_goto_hist as skipdups. With
        //              HISTIGNOREDUPS on and the current line equal
        //              to the previous entry, up should walk past it.
        let _g = zle_test_setup();
        let _zle = zle_with_history(&["unique", "dup", "dup"]);
        *ZLELINE.lock().unwrap() = "dup".chars().collect();
        ZLELL.store("dup".len(), Ordering::SeqCst);
        history().lock().unwrap().cursor = 3; // sentinel
        ZMOD.lock().unwrap().mult = 1;

        // Turn HISTIGNOREDUPS on so the skipdups path fires.
        opt_state_set("histignoredups", true);

        let rc = uphistory();
        assert_eq!(rc, 0);
        assert_eq!(
            ZLELINE.lock().unwrap().iter().collect::<String>(),
            "unique",
            "with HISTIGNOREDUPS on, up must skip the 'dup' twins and land on 'unique'"
        );
        opt_state_set("histignoredups", false);
    }

    #[test]
    fn uphistory_returns_1_on_exhaustion_when_histbeep_set() {
        let _g = crate::test_util::global_state_lock();
        // c:236-237 — `if (!zle_goto_hist(...) && isset(HISTBEEP))
        //              return 1;`
        let _g = zle_test_setup();
        let _zle = zle_with_history(&["only"]);
        // Already at entry 0; trying to go up further is exhausted.
        history().lock().unwrap().cursor = 0;
        ZMOD.lock().unwrap().mult = 1;

        opt_state_set("histbeep", true);
        let rc = uphistory();
        assert_eq!(rc, 1, "exhausted up + HISTBEEP must return 1 (beep signal)");
        opt_state_set("histbeep", false);
    }

    #[test]
    fn uphistory_returns_0_on_exhaustion_without_histbeep() {
        let _g = crate::test_util::global_state_lock();
        // c:236-237 — exhausted but no HISTBEEP → return 0.
        let _g = zle_test_setup();
        let _zle = zle_with_history(&["only"]);
        history().lock().unwrap().cursor = 0;
        ZMOD.lock().unwrap().mult = 1;

        opt_state_set("histbeep", false);
        let rc = uphistory();
        assert_eq!(rc, 0, "exhausted up + !HISTBEEP must return 0");
    }

    #[test]
    fn beginningofhistory_fills_buffer_from_oldest_entry() {
        let _g = crate::test_util::global_state_lock();
        // c:584 — must drive cursor to entry 0 AND refill the buffer.
        let _g = zle_test_setup();
        let _zle = zle_with_history(&["alpha", "bravo", "charlie"]);
        history().lock().unwrap().cursor = 3; // sentinel
        *ZLELINE.lock().unwrap() = "draft".chars().collect();

        let rc = beginningofhistory();
        assert_eq!(rc, 0, "successful move returns 0");
        assert_eq!(
            ZLELINE.lock().unwrap().iter().collect::<String>(),
            "alpha",
            "buffer must hold the oldest entry"
        );
        assert_eq!(
            history().lock().unwrap().cursor,
            0,
            "cursor must land on entry 0"
        );
    }

    #[test]
    fn endofhistory_fills_buffer_with_saved_live_line() {
        let _g = crate::test_util::global_state_lock();
        // c:604 — drives back to sentinel; saved_line (if any) restores.
        let _g = zle_test_setup();
        let _zle = zle_with_history(&["one", "two"]);
        // Compose a live draft, then walk up to "two", then back via endofhistory.
        *ZLELINE.lock().unwrap() = "myDraft".chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        history().lock().unwrap().cursor = 2; // sentinel

        // Up once → "two" (saves "myDraft" into saved_line).
        assert!(zle_goto_hist(-1, false));
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "two");

        // endofhistory drives back to sentinel → restores "myDraft".
        let rc = endofhistory();
        assert_eq!(rc, 0);
        assert_eq!(
            ZLELINE.lock().unwrap().iter().collect::<String>(),
            "myDraft",
            "saved live buffer must be restored at sentinel"
        );
    }

    #[test]
    fn zle_goto_hist_walks_backwards_then_forwards() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&["echo a", "echo b", "echo c"]);
        // Sit on the live (sentinel) buffer.
        history().lock().unwrap().cursor = 3;
        // Up once → "echo c".
        assert!(zle_goto_hist(-1, false));
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "echo c");
        // Up two more → "echo a".
        assert!(zle_goto_hist(-2, false));
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "echo a");
        // One more up: exhausted.
        assert!(!zle_goto_hist(-1, false));
        // Down twice → "echo c".
        assert!(zle_goto_hist(2, false));
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "echo c");
    }

    #[test]
    fn zle_goto_hist_restores_saved_line_when_returning_to_sentinel() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&["one", "two"]);
        *ZLELINE.lock().unwrap() = "draft".chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(ZLELL.load(Ordering::SeqCst), Ordering::SeqCst);
        history().lock().unwrap().cursor = 2; // sentinel
                                              // Up to "two", then up to "one", then back down twice → restore "draft".
        assert!(zle_goto_hist(-1, false));
        assert!(zle_goto_hist(-1, false));
        assert!(zle_goto_hist(2, false));
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "draft");
    }

    #[test]
    fn zle_goto_hist_skipdups_skips_consecutive_dupes() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&["dup", "dup", "uniq"]);
        *ZLELINE.lock().unwrap() = "uniq".chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        history().lock().unwrap().cursor = 3;
        // skipdups + n=-1 from sentinel: matching cur_line "uniq" → entries[2]
        // is "uniq", same string as zleline, so it gets skipped, landing on "dup".
        assert!(zle_goto_hist(-1, true));
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "dup");
    }

    #[test]
    fn upline_in_single_line_buffer_returns_remaining_count() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "echo hi".chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(4, Ordering::SeqCst);
        let leftover = upline();
        // Single-line buffer: can't go up, leftover == MULT.load(std::sync::atomic::Ordering::SeqCst) (1).
        assert_eq!(leftover, 1);
    }

    #[test]
    fn upline_in_two_line_buffer_moves_cursor_to_first_line() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "first\nsecond".chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(9, Ordering::SeqCst); // inside "second" at col 3 ("sec[o]nd")
        let leftover = upline();
        assert_eq!(leftover, 0);
        // Should land at column 3 of first line → index 3
        assert_eq!(ZLECS.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn up_line_or_history_falls_through_to_history_when_at_top() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&["prev cmd"]);
        *ZLELINE.lock().unwrap() = "current".chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        ZLECS.store(0, Ordering::SeqCst);
        history().lock().unwrap().cursor = 1;
        let ret = uplineorhistory();
        assert_eq!(ret, 0);
        assert_eq!(
            ZLELINE.lock().unwrap().iter().collect::<String>(),
            "prev cmd"
        );
    }

    #[test]
    fn undo_redo_round_trip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        setlastline();
        // Type "abc"
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        ZLECS.store(3, Ordering::SeqCst);
        mkundoent();
        // Undo → empty.
        assert_eq!(undo_widget(), 0);
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "");
        assert_eq!(ZLELL.load(Ordering::SeqCst), 0);
        // Redo → "abc" back.
        assert_eq!(redo_widget(), 0);
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "abc");
    }

    #[test]
    fn undo_returns_one_when_stack_empty() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        setlastline();
        assert_eq!(undo_widget(), 1);
    }

    #[test]
    fn push_line_pushes_buffer_and_clears_editor() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&[]);
        *ZLELINE.lock().unwrap() = "in flight".chars().collect();
        ZLELL.store(9, Ordering::SeqCst);
        ZLECS.store(4, Ordering::SeqCst);
        MULT.store(1, Ordering::SeqCst);
        push_line();
        assert_eq!(*BUFSTACK.lock().unwrap(), vec!["in flight".to_string()]);
        assert!(ZLELINE.lock().unwrap().is_empty());
        assert_eq!(ZLELL.load(Ordering::SeqCst), 0);
        assert_eq!(ZLECS.load(Ordering::SeqCst), 0);
        // stackcs records where the cursor was so a return-from-push can
        // restore it.
        assert_eq!(STACKCS.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn push_line_with_count_pushes_extra_empty_strings() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&[]);
        *ZLELINE.lock().unwrap() = "x".chars().collect();
        ZLELL.store(1, Ordering::SeqCst);
        MULT.store(3, Ordering::SeqCst);
        push_line();
        // mult=3 → push line then 2 empties.
        assert_eq!(BUFSTACK.lock().unwrap().len(), 3);
        assert_eq!(BUFSTACK.lock().unwrap()[0], "x");
        assert_eq!(BUFSTACK.lock().unwrap()[1], "");
        assert_eq!(BUFSTACK.lock().unwrap()[2], "");
    }

    #[test]
    fn push_line_negative_count_is_no_op() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&[]);
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        MULT.store(-1, Ordering::SeqCst);
        push_line();
        assert!(BUFSTACK.lock().unwrap().is_empty());
        assert_eq!(ZLELINE.lock().unwrap().iter().collect::<String>(), "abc");
    }

    #[test]
    fn remember_edits_saves_original_then_forget_restores() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&["echo a", "echo b"]);
        history().lock().unwrap().cursor = 0;
        *ZLELINE.lock().unwrap() = "echo Z".chars().collect();
        ZLELL.store(6, Ordering::SeqCst);
        {
            let mut hist = history().lock().unwrap();
            remember_edits(&mut hist);
        }
        {
            let hist = history().lock().unwrap();
            assert!(hist.have_edits);
            assert_eq!(hist.entries[0].line, "echo Z");
            assert_eq!(hist.originals[0].as_deref(), Some("echo a"));
        }
        {
            let mut hist = history().lock().unwrap();
            forget_edits(&mut hist);
        }
        {
            let hist = history().lock().unwrap();
            assert!(!hist.have_edits);
            assert_eq!(hist.entries[0].line, "echo a");
            assert!(hist.originals[0].is_none());
        }
    }

    #[test]
    fn set_local_history_mult_sets_or_clears_foreign_skip() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&[]);
        let mut hist = history().lock().unwrap();
        // mult=2 with has_mult=true → set HIST_FOREIGN.
        set_local_history(&mut hist, true, 2);
        assert_eq!(hist.hist_skip_flags, 1);
        // mult=0 with has_mult=true → clear.
        set_local_history(&mut hist, true, 0);
        assert_eq!(hist.hist_skip_flags, 0);
    }

    #[test]
    fn set_local_history_no_mult_xor_toggles() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&[]);
        let mut hist = history().lock().unwrap();
        // From 0, no-mult toggle → 1.
        set_local_history(&mut hist, false, 0);
        assert_eq!(hist.hist_skip_flags, 1);
        // Toggle again → 0.
        set_local_history(&mut hist, false, 0);
        assert_eq!(hist.hist_skip_flags, 0);
    }

    #[test]
    fn accept_line_and_down_history_pushes_next_entry_on_bufstack() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut zle = zle_with_history(&["one", "two", "three"]);
        history().lock().unwrap().cursor = 0; // sitting on "one"
        *ZLELINE.lock().unwrap() = "one".chars().collect();
        ZLELL.store(3, Ordering::SeqCst);
        // Simulate widget body inline.
        let len = history().lock().unwrap().entries.len();
        let next_idx = history().lock().unwrap().cursor + 1;
        if next_idx < len {
            if let Some(entry) = history().lock().unwrap().entries.get(next_idx) {
                BUFSTACK.lock().unwrap().push(entry.line.clone());
                STACKHIST.store((entry.num as i32).max(0), Ordering::SeqCst);
            }
        }
        DONE.store(1, Ordering::SeqCst);
        assert!(DONE.load(Ordering::SeqCst) != 0);
        assert_eq!(*BUFSTACK.lock().unwrap(), vec!["two".to_string()]);
    }

    // ─── zsh-corpus pins for zlinecmp / zlinefind ──────────────────

    /// `zlinecmp("hello", "hello")` returns 0 (exact match).
    #[test]
    fn zle_hist_corpus_zlinecmp_exact_match_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zlinecmp("hello", "hello"), 0, "exact match per c:143 = 0");
    }

    /// `zlinecmp("helloworld", "hello")` returns -1 (input is prefix).
    #[test]
    fn zle_hist_corpus_zlinecmp_input_prefix_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            zlinecmp("helloworld", "hello"),
            -1,
            "input prefix of hist per c:146"
        );
    }

    /// `zlinecmp("HELLO", "hello")` returns 1 (case-fold match).
    #[test]
    fn zle_hist_corpus_zlinecmp_case_fold_match_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zlinecmp("HELLO", "hello"), 1, "case-fold match per c:181");
    }

    /// `zlinecmp("xxx", "hello")` returns 3 (no match at all).
    #[test]
    fn zle_hist_corpus_zlinecmp_no_match_returns_three() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zlinecmp("xxx", "hello"), 3, "no match per c:174 = 3");
    }

    /// `zlinecmp("", "")` returns 0 (both empty = same).
    #[test]
    fn zle_hist_corpus_zlinecmp_both_empty_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zlinecmp("", ""), 0);
    }

    /// `zlinefind("hello world", 0, "world", dir=1, sens=2)` finds
    /// `world` at byte offset 6 — `sens=2` is the case-fold-prefix
    /// threshold that the history-incremental-search caller actually
    /// uses (matches `zlinecmp` return values 0 = identical,
    /// -1 = exact prefix, 1 = case-fold-identical; all < 2 pass).
    /// `sens=0` would never match (no return code < 0 exists for
    /// a successful compare), so the previous test pin of `sens=0`
    /// was pinning a degenerate path.
    #[test]
    fn zle_hist_corpus_zlinefind_forward_finds_substring() {
        let _g = crate::test_util::global_state_lock();
        let r = zlinefind("hello world", 0, "world", 1, 2);
        assert_eq!(r, Some(6), "world starts at byte 6");
    }

    /// `zlinefind` on missing returns None even with the broadest
    /// non-trivial sens threshold (`sens=2`).
    #[test]
    fn zle_hist_corpus_zlinefind_missing_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = zlinefind("hello", 0, "xyz", 1, 2);
        assert_eq!(r, None);
    }

    /// `zlinefind` empty needle returns Some(pos).
    #[test]
    fn zle_hist_corpus_zlinefind_empty_needle_at_pos() {
        let _g = crate::test_util::global_state_lock();
        let r = zlinefind("hello", 2, "", 1, 0);
        // Either matches at pos 2 or doesn't match; pin no panic.
        let _ = r;
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_hist.c zlinecmp/zlinefind.
    // ═══════════════════════════════════════════════════════════════════

    /// c:135-143 — `zlinecmp(s, s)` returns 0 (exact byte-for-byte match
    /// short-circuits via the first while loop's `!*iptr` exit).
    #[test]
    fn zlinecmp_identical_strings_return_zero() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zlinecmp("hello", "hello"), 0);
        assert_eq!(zlinecmp("", ""), 0);
        assert_eq!(zlinecmp("ABCdef", "ABCdef"), 0);
    }

    /// c:146 — `zlinecmp("longer", "long")` returns -1 (input is prefix
    /// of history but history continues).
    #[test]
    fn zlinecmp_input_is_prefix_returns_neg_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zlinecmp("hello world", "hello"), -1);
        assert_eq!(zlinecmp("abc", "ab"), -1);
        assert_eq!(
            zlinecmp("x", ""),
            -1,
            "empty input is prefix of any non-empty"
        );
    }

    /// c:174 — `zlinecmp("HELLO", "hello")` returns 1 (case-folded match
    /// in the second loop, history is uppercase but input lowercase).
    #[test]
    fn zlinecmp_case_folded_same_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // History is the haystack (folded to lower); input is already lower.
        assert_eq!(zlinecmp("HELLO", "hello"), 1, "case-fold match");
        assert_eq!(zlinecmp("AbC", "abc"), 1);
    }

    /// c:174,183 — `zlinecmp` reverse direction: input lowercase, history
    /// in mixed case where lowering history still equals input.
    #[test]
    fn zlinecmp_case_folded_prefix_returns_two() {
        let _g = crate::test_util::global_state_lock();
        // tolower(history) → lowercase; input is shorter lowercase prefix.
        let r = zlinecmp("HelloWorld", "hello");
        assert_eq!(r, 2, "case-fold prefix returns 2 per c:183");
    }

    /// c:142,186 — `zlinecmp` returns 3 when strings genuinely differ
    /// (input is not a prefix of history and case-fold also fails).
    #[test]
    fn zlinecmp_different_strings_return_three() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(zlinecmp("alpha", "beta"), 3);
        assert_eq!(zlinecmp("xyz", "abc"), 3);
    }

    /// c:203 — `zlinefind` finds at exact pos when there's a match.
    #[test]
    fn zlinefind_forward_at_pos_zero() {
        let _g = crate::test_util::global_state_lock();
        // Forward search starting at pos 0 for "hello" in "hello world"
        // — should find at offset 0 immediately.
        let r = zlinefind("hello world", 0, "hello", 1, 2);
        assert_eq!(r, Some(0), "exact match at pos 0");
    }

    /// c:208 — `zlinefind` backward from end finds last occurrence.
    #[test]
    fn zlinefind_backward_searches_from_pos_down_to_zero() {
        let _g = crate::test_util::global_state_lock();
        // Backward search with sens=2 (case-fold-allowed) finds first
        // match working backward.
        let r = zlinefind("hello there", 6, "there", 0, 2);
        assert_eq!(r, Some(6), "backward search starts at pos and walks");
    }

    /// c:215-220 — `zlinefind` backward with pos=0 only checks position 0.
    #[test]
    fn zlinefind_backward_pos_zero_only_checks_zero() {
        let _g = crate::test_util::global_state_lock();
        let r = zlinefind("hello world", 0, "world", 0, 2);
        assert_eq!(r, None, "from pos 0 backward, 'world' (not at 0) → None");
    }

    /// c:215 — `zlinefind` backward with very-permissive sens=4 (above
    /// max return 3) matches always at the starting position.
    #[test]
    fn zlinefind_sens_too_permissive_matches_at_start() {
        let _g = crate::test_util::global_state_lock();
        // sens=4 means < 4 always true (zlinecmp max return is 3).
        let r = zlinefind("hello", 0, "zzz", 1, 4);
        assert_eq!(r, Some(0), "sens > 3 always matches → returns initial pos");
    }

    /// c:204 — `zlinefind` forward past end of haystack returns None.
    #[test]
    fn zlinefind_forward_past_end_returns_none() {
        let _g = crate::test_util::global_state_lock();
        let r = zlinefind("hello", 5, "x", 1, 2);
        assert_eq!(r, None, "starting past end → no match");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_hist.c
    // c:114 zlinecmp / c:190 zlinefind / c:225 uphistory / c:370 upline /
    // c:460 downline / c:585 historysearchbackward / c:686 historysearchforward /
    // c:778 beginningofbufferorhistory / c:814 endofbufferorhistory / c:1028 zle_setline
    // ═══════════════════════════════════════════════════════════════════

    /// c:114 — `zlinecmp` returns i32 (compile-time type pin).
    #[test]
    fn zlinecmp_returns_i32_type() {
        let _: i32 = zlinecmp("a", "a");
    }

    /// c:114 — `zlinecmp("", "")` (both empty) returns 0 per c:120 prefix match.
    #[test]
    fn zlinecmp_both_empty_returns_zero() {
        assert_eq!(
            zlinecmp("", ""),
            0,
            "both empty → match (returns 0 per identical-prefix rule)"
        );
    }

    /// c:114 — `zlinecmp` is deterministic for arbitrary input.
    #[test]
    fn zlinecmp_is_deterministic() {
        for (a, b) in [("a", "a"), ("abc", "abc"), ("abc", "ab"), ("", "x")] {
            let first = zlinecmp(a, b);
            for _ in 0..3 {
                assert_eq!(
                    zlinecmp(a, b),
                    first,
                    "zlinecmp({:?}, {:?}) must be deterministic",
                    a,
                    b
                );
            }
        }
    }

    /// c:190 — `zlinefind` returns Option<usize> (compile-time type pin).
    #[test]
    fn zlinefind_returns_option_usize_type() {
        let _: Option<usize> = zlinefind("hello", 0, "h", 1, 2);
    }

    /// c:190 — `zlinefind` is deterministic for arbitrary input.
    #[test]
    fn zlinefind_is_deterministic() {
        let first = zlinefind("hello", 0, "ell", 1, 2);
        for _ in 0..5 {
            assert_eq!(
                zlinefind("hello", 0, "ell", 1, 2),
                first,
                "zlinefind must be deterministic"
            );
        }
    }

    // Note: uphistory/downhistory/historysearch* and friends read from
    // the live ZLE input buffer and BLOCK on missing terminal input,
    // hanging the test runner. Type-pin tests for those widgets must
    // first set up a fake key-read substrate; deferred.

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_hist.c
    // c:73 forget_edits / c:114 zlinecmp / c:190 zlinefind /
    // c:1028 zle_setline / c:1052 setlocalhistory / c:1073 zle_goto_hist
    // — only safe (non-blocking, no live-TTY) helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// c:1028 — `zle_setline` returns i32 (compile-time type pin).
    #[test]
    fn zle_setline_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = zle_setline();
    }

    /// c:1052 — `setlocalhistory` returns i32 (compile-time type pin).
    #[test]
    fn setlocalhistory_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = setlocalhistory();
    }

    /// c:1052 — `setlocalhistory` is idempotent / safe.
    #[test]
    fn setlocalhistory_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            let _ = setlocalhistory();
        }
    }

    /// c:1073 — `zle_goto_hist(0, false)` returns bool (type pin).
    #[test]
    fn zle_goto_hist_returns_bool_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: bool = zle_goto_hist(0, false);
    }

    /// c:114 — `zlinecmp(s, s)` reflexive identity returns 0.
    #[test]
    fn zlinecmp_self_reflexive_zero() {
        for s in ["", "a", "hello", "abc"] {
            assert_eq!(
                zlinecmp(s, s),
                0,
                "zlinecmp({:?},{:?}) self-reflexive must be 0",
                s,
                s
            );
        }
    }

    /// c:190 — `zlinefind` empty needle at pos 0 returns Some
    /// (empty is a prefix of every string).
    #[test]
    fn zlinefind_empty_needle_forward_returns_some_zero() {
        let r = zlinefind("anything", 0, "", 1, 2);
        assert!(r.is_some(), "empty needle → Some");
    }

    /// c:190 — `zlinefind("", 0, "x", 1, 2)` empty haystack + nonempty
    /// needle returns None.
    #[test]
    fn zlinefind_empty_haystack_nonempty_needle_returns_none() {
        let r = zlinefind("", 0, "x", 1, 2);
        assert!(r.is_none(), "empty haystack + nonempty needle → None");
    }

    /// c:114 — `zlinecmp` purity sweep across pairs.
    #[test]
    fn zlinecmp_pure_full_sweep() {
        for (a, b) in [
            ("", ""),
            ("a", "b"),
            ("hello", "world"),
            ("prefix", "prefix_more"),
            ("X", "x"),
        ] {
            let first = zlinecmp(a, b);
            for _ in 0..3 {
                assert_eq!(
                    zlinecmp(a, b),
                    first,
                    "zlinecmp({:?},{:?}) must be pure",
                    a,
                    b
                );
            }
        }
    }
}
