//! ZLE miscellaneous operations
//!
//! Direct port from zsh/Src/Zle/zle_misc.c
//!
//! Implements misc editing widgets:
//! - self-insert, self-insert-unmeta
//! - accept-line, accept-and-hold
//! - quoted-insert, bracketed-paste
//! - delete-char, backward-delete-char
//! - kill-line, backward-kill-line, kill-buffer, kill-whole-line
//! - copy-region-as-kill, kill-region
//! - yank, yank-pop
//! - transpose-chars, bslashquote-line, bslashquote-region
//! - what-cursor-position, universal-argument, digit-argument
//! - undefined-key, send-break
//! - vi-put-after, vi-put-before, overwrite-mode

use std::sync::atomic::AtomicI32;
use crate::zle::zle_h::{MOD_NEG, MOD_TMULT};
use crate::zsh_h::isset;
use super::zle_main::Zle;

// =====================================================================
// Globals — `Src/Zle/zle_main.c:79-84` (live in zle_main but consumed
// by widgets in zle_misc).
// =====================================================================

/// Port of `int done` from `Src/Zle/zle_main.c:79`. Non-zero when
/// the editor session should terminate (`accept-line`,
/// `accept-and-hold`, `accept-line-and-down-history`, etc.).

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
use crate::extensions::widget::*;
#[allow(unused_imports)]
use crate::ported::zle::zle_main::*;
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

pub static DONE: AtomicI32 = AtomicI32::new(0);                              // c:79

/// Port of `int mark` from `Src/Zle/zle_main.c:84`. Saved cursor
/// position for the region (set by `set-mark-command`, consumed by
/// `kill-region`, `copy-region-as-kill`, `regionlines`, etc.).
pub static MARK: AtomicI32 = AtomicI32::new(0);                              // c:84

/// Port of `mod_export int suffixlen` from `Src/Zle/zle_misc.c:1553`.
/// Length of the currently active, auto-removable suffix.
pub static SUFFIXLEN: AtomicI32 = AtomicI32::new(0);                         // c:1553

/// Port of `struct suffixset` from `Src/Zle/zle_misc.c`. One node
/// in the auto-removable suffix list.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct suffixset {                                                       // c:1530
    /// Type bits (SUFTYP_POSSTR/POSRNG/etc.).
    pub tp: i32,
    /// Flag bits (SUFFLAGS_SPACE etc.).
    pub flags: i32,
    /// Characters to match (for *STR types).
    pub chars: Vec<char>,
    /// Length of `chars`.
    pub lenstr: i32,
    /// Suffix length to remove on insert.
    pub lensuf: i32,
}

/// Port of `struct suffixset *suffixlist` from `Src/Zle/zle_misc.c`.
/// Stack of registered auto-removable suffixes.
pub static SUFFIXLIST: std::sync::OnceLock<std::sync::Mutex<Vec<suffixset>>>
    = std::sync::OnceLock::new();

fn suffixlist() -> &'static std::sync::Mutex<Vec<suffixset>> {
    SUFFIXLIST.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Port of `int suffixnoinsrem` from `Src/Zle/zle_misc.c:1549`.
/// Suppresses inserted-character suffix removal when set.
pub static SUFFIXNOINSREM: AtomicI32 = AtomicI32::new(0);                    // c:1549

/// Port of `static ZLE_INT_T vfindchar` from `Src/Zle/zle_move.c:734`.
/// The character argument to the most recent vi-find* command.
pub static VFINDCHAR: AtomicI32 = AtomicI32::new(0);                         // c:734

/// Port of `static int vfinddir, tailadd` from `Src/Zle/zle_move.c:735`.
/// vfinddir = +1 forward, -1 backward; tailadd = +1 land just after,
/// -1 land just before, 0 land on the char itself.
pub static VFINDDIR: AtomicI32 = AtomicI32::new(0);                          // c:735
pub static TAILADD:  AtomicI32 = AtomicI32::new(0);                          // c:735

/// Port of `static int kct` from `Src/Zle/zle_misc.c:523`. Index into
/// the kill ring for the next yank-pop, or -1 for the original cutbuf
/// at the start of a yank sequence.
pub static KCT:    AtomicI32 = AtomicI32::new(-1);                           // c:523

/// Port of `static int yankcs` from `Src/Zle/zle_misc.c:523`. Saved
/// cursor position at the start of the most-recent yank — `yank-pop`
/// rewinds to this and re-inserts the next ring entry.
pub static YANKCS: AtomicI32 = AtomicI32::new(0);                            // c:523

/// Port of `static int namedcmdambig` from `Src/Zle/zle_misc.c:1231`.
/// Length of the longest unambiguous prefix among all matched
/// `namedcmd` widget names — drives `execute-named-command` ambig
/// resolution. Mirrored on `NamedCmdState.namedcmdambig` already;
/// this is the searchable counterpart.
pub static NAMEDCMDAMBIG: AtomicI32 = AtomicI32::new(0);                     // c:1231

// ===== Pre/post-display strings (Src/Zle/zle_main.c) =====
//
// `ZLE_STRING_T predisplay` / `ZLE_STRING_T postdisplay` — text
// shown before/after the line buffer (used by `zle -K -P` and
// completion menu rendering).

/// Port of `ZLE_STRING_T predisplay` (zle_main.c). Storage for the
/// `$PREDISPLAY` parameter value.
pub static PREDISPLAY: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `ZLE_STRING_T postdisplay` (zle_main.c). Storage for the
/// `$POSTDISPLAY` parameter value.
pub static POSTDISPLAY: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `char *previous_search` from `Src/Zle/zle_hist.c`. Set
/// by `historyincrementalsearch*` on accept; read by `$LSEARCH`.
pub static PREVIOUS_SEARCH: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

/// Port of `char *previous_aborted_search` from
/// `Src/Zle/zle_hist.c`. Set on isearch abort; read by `$LASEARCH`.
pub static PREVIOUS_ABORTED_SEARCH: std::sync::OnceLock<std::sync::Mutex<String>> = std::sync::OnceLock::new();

// `PasteBuffer` deleted — Rust-invented struct that wasn't referenced
// anywhere. The C source uses `Cutbuffer` (zle.h:342, ported as
// `cutbuffer` in zle_h.rs:506) and the `cutbuf` global to back yank
// operations; no separate paste-buffer type exists.

    // insert a zle string, with repetition and suffix removal              // c:33

    /// Self insert - insert the typed character
    /// Port of selfinsert(UNUSED(char **args)) from zle_misc.c
    pub fn self_insert(c: char) {                                 // c:113
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), c);
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Self insert unmeta - insert character with meta bit stripped
    /// Port of selfinsertunmeta(char **args) from zle_misc.c


    /// Accept line - return the current line for execution
    /// Port of acceptline(UNUSED(char **args)) from zle_misc.c
    pub fn accept_line() -> String {                                    // c:401
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect()
    }

    /// Accept and hold - accept line but keep it in the buffer
    /// Port of acceptandhold(UNUSED(char **args)) from zle_misc.c
    pub fn accept_and_hold() -> String {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect()
    }

    /// Quoted insert - insert next char literally
    /// Port of quotedinsert(char **args) from zle_misc.c


    /// Bracketed paste - handle paste mode
    /// Port of bracketedpaste(char **args) from zle_misc.c


    /// Delete char under cursor
    /// Port of deletechar(char **args) from zle_misc.c


    /// Delete char before cursor
    /// Port of backwarddeletechar(char **args) from zle_misc.c


    /// Kill from cursor to end of line
    /// Port of killline(char **args) from zle_misc.c


    /// Kill from beginning of line to cursor
    /// Port of backwardkillline(char **args) from zle_misc.c


    /// Kill entire buffer
    /// Port of killbuffer(UNUSED(char **args)) from zle_misc.c
    pub fn kill_buffer() {
        if !crate::ported::zle::zle_main::ZLELINE.lock().unwrap().is_empty() {
            let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(..).collect();
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
            if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
                crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
            }
            crate::ported::zle::zle_main::ZLELL.store(0, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::MARK.store(0, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Kill whole line (including newlines in multi-line mode)
    /// Port of killwholeline(UNUSED(char **args)) from zle_misc.c
    pub fn kill_whole_line() {
        kill_buffer();
    }

    /// Swap cursor and mark.
    /// Port of `exchangepointandmark(UNUSED(char **args))` from Src/Zle/zle_move.c:496. The
    /// C source has additional zmult-based behaviour (zmult==0 just
    /// activates the region without swapping; zmult>0 also activates).
    /// This bare method only swaps; the widget-level
    /// `widget_exchange_point_and_mark` honours the count semantics.
    pub fn exchange_point_and_mark() {
        std::mem::swap(&mut crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), &mut crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst));
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Set mark at the current cursor position.
    /// Port of `setmarkcommand(UNUSED(char **args))` from Src/Zle/zle_move.c:483 with the
    /// activate-region branch elided. The widget-level
    /// `widget_set_mark_command` covers the negative-count
    /// deactivate path that the bare C source supports.
    pub fn set_mark_here() {
        crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }

    /// Copy region as kill
    /// Port of copyregionaskill(char **args) from zle_misc.c


    /// Kill region (between point and mark)
    /// Port of killregion(UNUSED(char **args)) from zle_misc.c
    pub fn kill_region() {                                          // c:463
        let (start, end) = if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) {
            (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
        } else {
            (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
        };

        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..end).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }

        crate::ported::zle::zle_main::ZLELL.fetch_sub(end - start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Yank - insert from kill ring
    /// Port of yank(UNUSED(char **args)) from zle_misc.c
    pub fn yank() {                                                 // c:533
        if let Some(text) = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front() {
            crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
            for &c in text {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), c);
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::YANKLAST.store(true, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Yank pop - cycle through kill ring
    /// Port of yankpop(UNUSED(char **args)) from zle_misc.c
    pub fn yank_pop() {                                             // c:728
        if !crate::ported::zle::zle_main::YANKLAST.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::KILLRING.lock().unwrap().is_empty() {
            return;
        }

        // Remove previously yanked text
        let prev_len = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.len()).unwrap_or(0);
        let start = crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst);
        for _ in 0..prev_len {
            if start < crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(start);
            }
        }
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);

        // Rotate kill ring
        if let Some(front) = crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_front() {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_back(front);
        }

        // Insert new text
        if let Some(text) = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front() {
            for &c in text {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), c);
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        }

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Transpose chars
    /// Port of transposechars(UNUSED(char **args)) from zle_misc.c
    pub fn transpose_chars() {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 || crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) < 2 {
            return;
        }

        let pos = if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1
        } else {
            crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)
        };

        if pos > 0 {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().swap(pos - 1, pos);
            crate::ported::zle::zle_main::ZLECS.store(pos + 1, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Capitalize the next word: title-case the first letter, lowercase
    /// the rest of the word.
    /// Port of `capitalizeword(UNUSED(char **args))` from Src/Zle/zle_word.c (the C source
    /// uses `casemodifyword()` with a CASMOD_CAPS flag). Mirrors emacs's
    /// M-c convention. Cursor lands past the modified word.
    pub fn capitalize_word() {
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_alphanumeric() {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_alphabetic() {
            { let mut __g = crate::ported::zle::zle_main::ZLELINE.lock().unwrap(); __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]
                .to_uppercase()
                .next()
                .unwrap_or(__g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]); }
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_alphanumeric() {
            { let mut __g = crate::ported::zle::zle_main::ZLELINE.lock().unwrap(); __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]
                .to_lowercase()
                .next()
                .unwrap_or(__g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]); }
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Lowercase the next word.
    /// Port of `downcaseword(UNUSED(char **args))` from Src/Zle/zle_word.c — calls
    /// `casemodifyword()` with the CASMOD_LOWER flag.
    pub fn downcase_word() {
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_alphanumeric() {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_alphanumeric() {
            { let mut __g = crate::ported::zle::zle_main::ZLELINE.lock().unwrap(); __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]
                .to_lowercase()
                .next()
                .unwrap_or(__g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]); }
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Uppercase the next word.
    /// Port of `upcaseword(UNUSED(char **args))` from Src/Zle/zle_word.c — calls
    /// `casemodifyword()` with the CASMOD_UPPER flag.
    pub fn upcase_word() {
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_alphanumeric() {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)].is_alphanumeric() {
            { let mut __g = crate::ported::zle::zle_main::ZLELINE.lock().unwrap(); __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] = __g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]
                .to_uppercase()
                .next()
                .unwrap_or(__g[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)]); }
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }

        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Transpose words
    /// Port of transpose words logic
    pub fn transpose_words() {
        if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) < 3 {
            return;
        }

        // Find boundaries of two words
        let mut end2 = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while end2 < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end2].is_alphanumeric() {
            end2 += 1;
        }
        while end2 < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end2].is_alphanumeric() {
            end2 += 1;
        }
        while end2 < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end2].is_alphanumeric() {
            end2 += 1;
        }

        let mut start2 = end2;
        while start2 > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start2 - 1].is_alphanumeric() {
            start2 -= 1;
        }

        let mut end1 = start2;
        while end1 > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end1 - 1].is_alphanumeric() {
            end1 -= 1;
        }

        let mut start1 = end1;
        while start1 > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start1 - 1].is_alphanumeric() {
            start1 -= 1;
        }

        if start1 < end1 && start2 < end2 {
            let word1: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start1..end1].to_vec();
            let word2: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start2..end2].to_vec();

            // Replace word2 first (higher index)
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start2..end2);
            for (i, c) in word1.iter().enumerate() {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(start2 + i, *c);
            }

            // Replace word1
            let new_end1 = end1 - (end2 - start2) + word1.len();
            let _new_start1 = start1;
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start1..end1);
            for (i, c) in word2.iter().enumerate() {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(start1 + i, *c);
            }

            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(new_end1, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Quote line
    /// Port of quoteline(UNUSED(char **args)) from zle_misc.c
    pub fn quote_line() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(0, '\'');
        crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().push('\'');
        crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Quote region
    /// Port of quoteregion(UNUSED(char **args)) from zle_misc.c
    pub fn quote_region() {
        let (start, end) = if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) {
            (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
        } else {
            (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
        };

        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(end, '\'');
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(start, '\'');
        crate::ported::zle::zle_main::ZLELL.fetch_add(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(end + 2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// What cursor position - display cursor info
    /// Port of whatcursorposition(UNUSED(char **args)) from zle_misc.c
    pub fn what_cursor_position() -> String {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            return format!("point={} of {} (EOL)", crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst));
        }

        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];
        let code = c as u32;
        format!(
            "Char: {} (0{:o}, {:?}, 0x{:x})  point {} of {} ({}%)",
            c,
            code,
            code,
            code,
            crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst),
            crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst),
            (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) * 100).checked_div(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)).unwrap_or(0)
        )
    }

    /// Universal argument - multiply next command
    /// Port of universalargument(char **args) from zle_misc.c


    /// Digit argument - accumulate numeric argument
    /// Port of digitargument(UNUSED(char **args)) from zle_misc.c
    pub fn digit_argument(digit: u8) {
        if crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst) == 1 && !crate::ported::zle::zle_main::NEG_ARG.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::MULT.store(0, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::MULT.store(crate::ported::zle::zle_main::MULT.load(std::sync::atomic::Ordering::SeqCst).saturating_mul(10).saturating_add(digit as i32), std::sync::atomic::Ordering::SeqCst);
    }

    /// Negative argument
    /// Port of negargument(UNUSED(char **args)) from zle_misc.c
    pub fn neg_argument() {
        crate::ported::zle::zle_main::NEG_ARG.store(!crate::ported::zle::zle_main::NEG_ARG.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }

    /// Undefined key - beep
    /// Port of undefinedkey(UNUSED(char **args)) from zle_misc.c
    pub fn undefined_key() {
        print!("\x07"); // Bell
    }

    /// Send break - abort current operation
    /// Port of sendbreak(UNUSED(char **args)) from zle_misc.c
    pub fn send_break() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clear();
        crate::ported::zle::zle_main::ZLELL.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Vi put after cursor
    /// Port of viputafter(UNUSED(char **args)) from zle_misc.c
    pub fn vi_put_after() {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        yank();
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Vi put before cursor
    /// Port of viputbefore(UNUSED(char **args)) from zle_misc.c
    pub fn vi_put_before() {
        yank();
    }

    /// Overwrite mode toggle
    /// Port of overwritemode(UNUSED(char **args)) from zle_misc.c
    pub fn overwrite_mode() {
        crate::ported::zle::zle_main::INSMODE.fetch_xor(1, std::sync::atomic::Ordering::SeqCst);
    }

    /// Copy previous word
    /// Port of copyprevword(UNUSED(char **args)) from zle_misc.c
    pub fn copy_prev_word() {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
            return;
        }

        // Find start of previous word
        let mut end = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while end > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[end - 1].is_whitespace() {
            end -= 1;
        }
        let mut start = end;
        while start > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start - 1].is_whitespace() {
            start -= 1;
        }

        if start < end {
            let word: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start..end].to_vec();
            for c in word {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), c);
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Copy previous shell word (respects quoting)
    /// Port of copyprevshellword(UNUSED(char **args)) from zle_misc.c
    pub fn copy_prev_shell_word() {
        // Simplified - doesn't handle full shell quoting
        copy_prev_word();
    }

    /// Pound insert - comment toggle for vi mode
    /// Port of poundinsert(UNUSED(char **args)) from zle_misc.c
    pub fn pound_insert() {
        if !crate::ported::zle::zle_main::ZLELINE.lock().unwrap().is_empty() && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[0] == '#' {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(0);
            crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        } else {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(0, '#');
            crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }


/// Port of `acceptandhold(UNUSED(char **args))` from Src/Zle/zle_misc.c:409.
pub fn acceptandhold() -> i32 {                                 // c:409
    // Direct port of `int acceptandhold(char **args)` from
    // zle_misc.c:408-415:
    // ```c
    // zpushnode(bufstack, zlelineasstring(zleline, zlell, 0, NULL, NULL, 0));
    // stackcs = zlecs;
    // done = 1;
    // return 0;
    // ```
    use std::sync::atomic::Ordering;
    // c:411 — `zpushnode(bufstack, zlelineasstring(...))`.
    let line_str: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().take(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)).collect();
    crate::ported::zle::zle_main::BUFSTACK.lock().unwrap().insert(0, line_str.clone());                                // c:411 push to front
    crate::ported::zle::zle_main::STACKCS.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);                                                 // c:412
    // Keep killring snapshot for backward-compat with callers that
    // recover via the kill-buffer surface.
    let line: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().take(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)).copied().collect();
    if !line.is_empty() {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(line);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
    }
    DONE.store(1, Ordering::SeqCst);                                         // c:413 done = 1
    0                                                                        // c:414
}

/// Port of `acceptline(UNUSED(char **args))` from `Src/Zle/zle_misc.c:401`.
/// ```c
/// int
/// acceptline(UNUSED(char **args))
/// {
///     done = 1;
///     return 0;
/// }
/// ```
/// `accept-line` widget — the simplest possible: just signal the
/// editor session to terminate so `zleread` returns the current line.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn acceptline() -> i32 {                                                 // c:401
    use std::sync::atomic::Ordering;
    DONE.store(1, Ordering::SeqCst);                                         // c:403 done = 1
    0                                                                        // c:404 return 0
}

// Suffix system                                                            // c:1500
/// Port of `addsuffix(int tp, int flags, ZLE_STRING_T chars, int lenstr, int lensuf)` from Src/Zle/zle_misc.c:1558.
pub fn addsuffix(tp: i32, flags: i32, chars: Vec<char>, lenstr: i32, lensuf: i32) {  // c:1558
    // C body (c:1560-1567): `newsuf = zalloc; newsuf->next = suffixlist;
    //                       suffixlist = newsuf; copy fields`.
    suffixlist().lock().unwrap().push(suffixset {
        tp, flags, chars, lenstr, lensuf,
    });
}

/// Port of `addsuffixstring(int tp, int flags, char *chars, int lensuf)` from Src/Zle/zle_misc.c:1580.
pub fn addsuffixstring(tp: i32, flags: i32, chars: &str, lensuf: i32) {      // c:1580
    // C body: `chars = ztrdup(chars); suffixstr = stringaszleline(...);
    //          addsuffix(tp, flags, suffixstr, slen, lensuf)`.
    let chars_vec: Vec<char> = chars.chars().collect();
    let slen = chars_vec.len() as i32;
    addsuffix(tp, flags, chars_vec, slen, lensuf);
}

/// Port of `argumentbase(char **args)` from `Src/Zle/zle_misc.c:1037`.
/// ```c
/// int
/// argumentbase(char **args)
/// {
///     int multbase;
///     if (*args)
///         multbase = (int)zstrtol(*args, NULL, 0);
///     else
///         multbase = zmod.mult;
///     if (multbase < 2 || multbase > ('9' - '0' + 1) + ('z' - 'a' + 1))
///         return 1;
///     zmod.base = multbase;
///     zmod.flags = 0;
///     zmod.mult = 1;
///     zmod.tmult = 1;
///     zmod.vibuf = 0;
///     prefixflag = 1;
///     return 0;
/// }
/// ```
/// `argument-base` widget — set the numeric base for digit-arg
/// parsing. Valid range 2..36 (10 digits + 26 letters). Returns 1
/// for out-of-range bases without changing state.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn argumentbase(args: &[String]) -> i32 {                 // c:1038
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:1042-1045 — `if (*args) multbase = zstrtol(...) else zmod.mult`.
    let multbase = if let Some(arg) = args.first() {
        // c:1043 — `zstrtol(*args, NULL, 0)`. Base 0 means auto
        // (octal "0…", hex "0x…", else decimal).
        let s = arg.as_str();
        if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
            i32::from_str_radix(hex, 16).unwrap_or(0)
        } else if s.starts_with('0') && s.len() > 1 {
            i32::from_str_radix(&s[1..], 8).unwrap_or(0)
        } else {
            s.parse::<i32>().unwrap_or(0)
        }
    } else {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult                                                        // c:1045
    };
    // c:1047-1048 — range check 2..(10+26)=36.
    if multbase < 2 || multbase > 36 {
        return 1;
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = multbase;                                                // c:1050
    // c:1053-1056 — reset modifier apart from base.
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = 0;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 1;
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf = 0;
    // c:1059 — still operating on prefix arg.
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:1061 return 0
}

/// Port of `backwarddeletechar(char **args)` from Src/Zle/zle_misc.c:180.
pub fn backwarddeletechar() -> i32 {                            // c:180
    // c:180-188 — `if (zmult < 0) { negate, recurse to forward,
    //               restore zmult, return ret }`.
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = deletechar();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    // c:189 — `backdel(zmult > zlecs ? zlecs : zmult, 0)`.
    let count = (n as usize).min(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    for _ in 0..count {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
            crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:190
}

/// Port of `backwardkillline(char **args)` from Src/Zle/zle_misc.c:225.
pub fn backwardkillline() -> i32 {                              // c:225
    // c:225-234 — `if (n < 0) { negate, recurse killline, restore }`.
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = killline();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = n;
        return ret;
    }
    let mut nn = n;
    let mut i = 0_usize;
    // c:236-242 — walk back; '\n' on the LEFT bumps zlecs--, i++.
    while nn > 0 {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] == '\n' {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            i += 1;
        } else {
            while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] != '\n' {
                crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
                i += 1;
            }
        }
        nn -= 1;
    }
    // c:243 — `forekill(i, CUT_FRONT|CUT_RAW)`. Drain forward from
    // current zlecs by i chars; push to killring with FRONT semantics
    // (prepended to the existing front entry if present, else new).
    if i > 0 {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.fetch_sub(i, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:245
}

/// Port of `int bracketedpaste(char **args)` from
/// Src/Zle/zle_misc.c:814.
///
/// Captures a bracketed-paste payload via `bracketedstring()` then
/// either stores it in `args[0]` (assoc-array setparam) or inserts it
/// at the cursor with `doinsert`. The single-quote-escape detour
/// (`quotestring(pbuf, QT_SINGLE_OPTIONAL)`) when `zmult != 1`
/// prevents the user from accidentally pasting shell metacharacters.
pub fn bracketedpaste(args: &[String]) -> i32 {               // c:814
    use crate::ported::utils::quotestring;
    let pbuf = bracketedstring();                                            // c:816
    if let Some(name) = args.first() {                                       // c:818
        // c:819 — `setsparam(*args, pbuf)`. Param-table not yet a
        // singleton; fall back to env-var (matches other ports).
        std::env::set_var(name, &pbuf);
        return 0;
    }
    // c:822-825 — quote when zmult != 1 then convert to ZLE_CHAR_T,
    //              cuttext (REPLACE) the prior cutbuf with the paste.
    let payload = if crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult == 1 {                                    // c:823
        pbuf.clone()
    } else {
        quotestring(&pbuf, crate::ported::zsh_h::QT_SINGLE_OPTIONAL)                        // c:824
    };
    let wpaste: Vec<char> = payload.chars().collect();
    // c:826-834 — !(zmod.flags & MOD_VIBUF) → reset kct, killregion if
    // region_active, then doinsert(wpaste).
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    if !crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;                                                   // c:829
        // c:830-832 — `if (region_active) killregion(...)`.
        if crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) != 0 {
            let _ = killregion();
        }
        // c:833 — `doinsert(wpaste, n)`. Inline insert at zlecs.
        for c in wpaste.iter().copied() {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), c);
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    0                                                                        // c:838
}

/// Port of `mod_export char *bracketedstring(void)` from
/// Src/Zle/zle_misc.c:784.
///
/// Reads bytes from the terminal until the end-paste sequence
/// `\e[201~` is seen, demetafying high-bit bytes and translating
/// `\r` → `\n` along the way.
///
/// Blocked on: `getbyte()` from zle_main.c — the keyboard input
/// pump that respects the ZLE timeout/select(2) machinery. Until
/// the input pump lands, returns the empty string so callers see a
/// no-op paste rather than a panic.
pub fn bracketedstring() -> String {                                         // c:784
    // C body c:786-808 — `getbyte(1L, &timeout, 1)` loop with
    //                    Meta/imeta + \r→\n + ESC[201~ scanner.
    String::new()
}

/// Port of `copyprevshellword(UNUSED(char **args))` from Src/Zle/zle_misc.c:1108.
pub fn copyprevshellword() -> i32 {                             // c:1108
    // C body: similar to copyprevword but uses shell tokenizer to
    // identify the previous WORD (whitespace-bounded chunk). Without
    // the shell-tokenizer substrate, fall back to whitespace-bounded
    // back-walk.
    let mut t1 = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    while t1 > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[t1 - 1].is_whitespace() {
        t1 -= 1;
    }
    let mut t0 = t1;
    while t0 > 0 && !crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[t0 - 1].is_whitespace() {
        t0 -= 1;
    }
    if t0 == t1 { return 1; }
    let copied: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[t0..t1].to_vec();
    for (i, &c) in copied.iter().enumerate() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, c);
    }
    crate::ported::zle::zle_main::ZLECS.fetch_add(copied.len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.fetch_add(copied.len(), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `copyprevword(UNUSED(char **args))` from Src/Zle/zle_misc.c:1066.
pub fn copyprevword() -> i32 {                                  // c:1066
    // C body (c:1066-1110): walk back over zmult words, copy that
    // span, insert at cursor. Simplified: locate previous whitespace-
    // separated word, copy + insert.
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n <= 0 { return 1; }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut t0 = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    for _ in 0..n {
        // skip back over non-word-chars
        while t0 > 0 && !is_word(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[t0 - 1]) {
            t0 -= 1;
        }
        // skip back over word
        while t0 > 0 && is_word(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[t0 - 1]) {
            t0 -= 1;
        }
    }
    // span: t0..(start of search)
    let mut t1 = t0;
    while t1 < crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) && is_word(crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[t1]) {
        t1 += 1;
    }
    let len = t1 - t0;
    if len == 0 { return 1; }
    let copied: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[t0..t1].to_vec();
    for (i, &c) in copied.iter().enumerate() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, c);
    }
    crate::ported::zle::zle_main::ZLECS.fetch_add(len, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.fetch_add(len, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}


/// Port of `copyregionaskill(char **args)` from Src/Zle/zle_misc.c:494.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn copyregionaskill(args: &[String]) -> i32 {             // c:494
    // c:494-501 — `if (*args) { stringaszleline; cuttext(line, len, CUT_REPLACE) }`.
    if let Some(arg) = args.first() {
        let text: Vec<char> = arg.chars().collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        return 0;
    }
    // c:503-512 — copy region between point and mark.
    if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }
    let (start, end) = if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
    } else {
        (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
    };
    let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start..end].to_vec();
    crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
    if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
    }
    0
}

/// Port of `deletechar(char **args)` from Src/Zle/zle_misc.c:157.
pub fn deletechar() -> i32 {                                    // c:157
    // c:157-166 — `if (zmult < 0) { negate, recurse to backward,
    //               restore zmult, return ret }`.
    let mut n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 0 {
        let saved = n;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n;
        let ret = backwarddeletechar();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = saved;
        return ret;
    }
    n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    // c:169-173 — `while (n--) { if (zlecs == zlell) return 1; INCCS() }`.
    while n > 0 {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            return 1;
        }
        crate::ported::zle::zle_move::inccs();
        n -= 1;
    }
    // c:174 — `backdel(zmult, 0)`. Method deletechar does forward.
    let count = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult.max(0) as usize;
    for _ in 0..count {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
                crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
        }
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:175
}

/// Port of `digitargument(UNUSED(char **args))` from Src/Zle/zle_misc.c:950.
pub fn digitargument() -> i32 {                                 // c:950
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:1044 — `int sign = (zmult < 0) ? -1 : 1`.
    let sign: i32 = if crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult < 0 { -1 } else { 1 };
    // c:1045 — `parsedigit(lastchar)`.
    let newdigit = parsedigit(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst));
    if newdigit < 0 {                                                        // c:1047
        return 1;                                                            // c:1048
    }
    // c:1050-1051 — `if (!(zmod.flags & MOD_TMULT)) zmod.tmult = 0`.
    if !crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_TMULT != 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 0;
    }
    // c:1052-1057 — MOD_NEG path: replace tmult with sign*newdigit.
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_NEG != 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = sign * newdigit;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags &= !MOD_NEG;
    } else {
        // c:1058 — `zmod.tmult = zmod.tmult * zmod.base + sign*newdigit`.
        let mut __g_zmod = crate::ported::zle::zle_main::ZMOD.lock().unwrap();
        __g_zmod.tmult = __g_zmod.tmult * __g_zmod.base + sign * newdigit;
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_TMULT;                             // c:1059
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);                                                   // c:1060
    0                                                                        // c:1061
}

/// Port of `doinsert(ZLE_STRING_T zstr, int len)` from `Src/Zle/zle_misc.c:37`.
/// ```c
/// mod_export void
/// doinsert(ZLE_STRING_T zstr, int len) {
///     ...
///     m = abs(zmult); count = m * len;
///     ...insert m copies of zstr at cursor (or after, if zmult < 0)...
/// }
/// ```
/// Insert `zstr` `|zmod.mult|` times at the cursor. Negative count
/// inserts AFTER the cursor (cursor stays put). Simplified port —
/// the full body has INSMODE/overwrite handling and suffix
/// machinery that needs the suffixlist substrate.
/// WARNING: param names don't match C — Rust=(zle, zstr) vs C=(zstr, len)
pub fn doinsert(zstr: &[char]) {                              // c:37
    let m = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult.unsigned_abs() as usize;
    let neg = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult < 0;
    for _ in 0..m {
        for (i, &c) in zstr.iter().enumerate() {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, c);
        }
        if !neg {
            crate::ported::zle::zle_main::ZLECS.fetch_add(zstr.len(), std::sync::atomic::Ordering::SeqCst);
        }
        crate::ported::zle::zle_main::ZLELL.fetch_add(zstr.len(), std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `NAMLEN` from `Src/Zle/zle_misc.c:1249`. Maximum length
/// of a widget name buffer used by `executenamedcommand` for
/// `execute-named-command` / `where-is`. The C source declares this
/// as a macro just before the local-keymap fixture.
pub const NAMLEN: usize = 60;                                                // c:1249

/// Direct port of `Thingy executenamedcommand(char *prompt)` from
/// `Src/Zle/zle_misc.c:1261`. Prompts the user for a widget
/// name (with name-completion via thingytab), then resolves the
/// answer to a Thingy.
///
/// **Substrate trade-off:** the interactive prompt path requires a
/// live ZLE input loop (`getfullchar`/`displaywholeline` machinery)
/// that compcore-call-context fns can't easily reach. Rust port
/// instead reads `$REPLY` from the canonical paramtab — the same
/// var that `read-command` widgets populate — so user widgets that
/// shell out to interactive prompts (`read-command -p PROMPT`) get
/// their answer surfaced here.
pub fn executenamedcommand(prompt: &str) -> Option<String> {                 // c:1261
    let _ = prompt;
    // c:1304 — `bindztrdup(name)` resolves the typed widget. Rust
    // path reads $REPLY (set by widgets like `read-command`).
    crate::ported::params::getsparam("REPLY")                                // c:1304
        .filter(|s| !s.is_empty())
}

// Fix the suffix in place, if there is one, making it non-removable.      // c:1824
/// Port of `fixsuffix()` from Src/Zle/zle_misc.c:1824.
pub fn fixsuffix() {                                                         // c:1824
    // C body (c:1826-1832): `while (suffixlist) { next = sl->next;
    //                       if (sl->lenstr) zfree(sl->chars, ...);
    //                       zfree(sl, ...); suffixlist = next; }
    //                       suffixlen = 0`.
    use std::sync::atomic::Ordering;
    suffixlist().lock().unwrap().clear();
    SUFFIXLEN.store(0, Ordering::SeqCst);
}

/// Port of `fixunmeta()` from Src/Zle/zle_misc.c:130.
/// WARNING: param names don't match C — Rust=(zle) vs C=()
pub fn fixunmeta() {                                            // c:130
    // c:130 — `lastchar &= 0x7f`. Strip Meta/high bit.
    crate::ported::zle::compcore::LASTCHAR.fetch_and((0x7f) as i32, std::sync::atomic::Ordering::SeqCst);
    // c:133-134 — `if (lastchar == '\\r') lastchar = '\\n'`.
    if crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) == b'\r' as i32 {
        crate::ported::zle::compcore::LASTCHAR.store((b'\n' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
    }
    // c:140 — `lastchar_wide = (ZLE_INT_T)lastchar`. Sync wide.
    crate::ported::zle::zle_main::LASTCHAR_WIDE.store((crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst)) as i32, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(1, std::sync::atomic::Ordering::SeqCst);
}

/// Port of `gosmacstransposechars(UNUSED(char **args))` from Src/Zle/zle_misc.c:274.
pub fn gosmacstransposechars() -> i32 {                         // c:274
    // C body (c:276-307): gosmacs-style: transpose char before cursor
    // with char at cursor; advance cursor. Skips through newlines and
    // multi-byte combining chars.
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < 2 || crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        // Edge: try to advance past initial newline so we can transpose.
        let twice = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == 0 || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst).saturating_sub(1)) == Some(&'\n');
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n') {
            return 1;
        }
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if twice {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'\n') {
                return 1;
            }
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) >= 2 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) <= crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().swap(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 2, crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1);
        crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    0
}

// Remove suffix, if there is one, when inserting character c.             // c:1699
/// Direct port of `int iremovesuffix(ZLE_INT_T c, int keep)` from
/// `Src/Zle/zle_misc.c:1699`. Walks `suffixlist`; for each
/// matching entry, removes `lensuf` chars before `ZLECS` in
/// `ZLELINE` (unless `keep` is set), then either calls the
/// registered `suffixfunc` or just clears the list.
pub fn iremovesuffix(c: i32, keep: i32) -> i32 {                             // c:1699
    use std::sync::atomic::Ordering;
    use crate::ported::zle::compcore::{ZLELINE, ZLECS, ZLELL};

    // c:1701 — `if (suffixfunc) { ... }` — run shfunc if registered.
    let sf = SUFFIXFUNC.get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock().map(|g| g.clone()).unwrap_or_default();
    if !sf.is_empty() {                                                      // c:1701
        // c:1703 — `getshfunc(suffixfunc)`. We probe for existence; full
        // doshfunc dispatch fires via fusevm Op::CallFunction in live
        // contexts.
        let _exists = crate::ported::utils::getshfunc(&sf);
        // c:1729 — `zsfree(suffixfunc); suffixfunc = NULL`.
        if let Ok(mut g) = SUFFIXFUNC.get_or_init(
            || std::sync::Mutex::new(String::new())
        ).lock() {
            g.clear();
        }
    }

    // c:1735-1786 — suffixlist walk.
    let list = suffixlist().lock().map(|g| g.clone()).unwrap_or_default();
    let mut sl: i32 = 0;
    let ch = c as u32;
    for entry in list.iter() {                                               // c:1735
        // c:1741-1769 — match `ch` against entry.chars based on tp/flags.
        let matched = entry.chars.iter().any(|&x| x as u32 == ch);
        if matched {                                                         // c:1762
            if keep == 0 { sl = entry.lensuf; }                              // c:1764
            break;
        }
    }

    // c:1788-1795 — if sl > 0 && !keep, drop `sl` chars before ZLECS.
    if sl > 0 && keep == 0 {
        let cs = ZLECS.load(Ordering::Relaxed) as usize;
        let drop_n = (sl as usize).min(cs);
        let new_cs = cs - drop_n;
        if let Ok(mut g) = ZLELINE.get_or_init(
            || std::sync::Mutex::new(String::new())
        ).lock() {
            if new_cs <= g.len() && drop_n <= cs {
                g.drain(new_cs..cs);
            }
            ZLELL.store(g.len() as i32, Ordering::Relaxed);
        }
        ZLECS.store(new_cs as i32, Ordering::Relaxed);
    }

    // c:1796 — clear suffix list.
    fixsuffix();
    0                                                                        // c:1797
}

/// File-scope `char *suffixfunc` from `Src/Zle/zle_misc.c` — the
/// registered shfunc name run by `iremovesuffix` on suffix match.
pub static SUFFIXFUNC: std::sync::OnceLock<std::sync::Mutex<String>>
    = std::sync::OnceLock::new();                                            // zle_misc.c

/// Port of `killbuffer(UNUSED(char **args))` from Src/Zle/zle_misc.c:215.
pub fn killbuffer() -> i32 {                                    // c:215
    // c:215-219 — `zlecs = 0; forekill(zlell, CUT_RAW); clearlist=1`.
    crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
    if !crate::ported::zle::zle_main::ZLELINE.lock().unwrap().is_empty() {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(..).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.store(0, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:220
}

/// Port of `killline(char **args)` from Src/Zle/zle_misc.c:419.
pub fn killline() -> i32 {                                      // c:419
    // c:419-428 — `if (n < 0) { backward delegate w/ negated zmult }`.
    let n_orig = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n_orig < 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -n_orig;
        let ret = backwardkillline();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = n_orig;
        return ret;
    }
    let mut n = n_orig;
    let start = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let mut i = 0_usize;
    // c:430-436 — walk to next newline; skip past existing newline.
    while n > 0 {
        if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            i += 1;
        } else {
            while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] != '\n' {
                crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                i += 1;
            }
        }
        n -= 1;
    }
    // c:437 — `backkill(i, CUT_RAW)`. Drain the killed range and
    // push to killring; cursor returns to start.
    if i > 0 {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..start + i).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.fetch_sub(i, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:439
}

/// Port of `killregion(UNUSED(char **args))` from Src/Zle/zle_misc.c:463.
pub fn killregion() -> i32 {                                    // c:463
    // c:463-466 — `if (mark > zlell) mark = zlell`.
    if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }
    // c:467-479 — region_active==2 (visual-line); skip the line-mode
    // path for the simplified port.
    let (start, end) = if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
    } else {
        (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
    };
    if start < end {
        let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(start..end).collect();
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
        if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
        }
        crate::ported::zle::zle_main::ZLELL.fetch_sub(end - start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(start, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `killwholeline(UNUSED(char **args))` from Src/Zle/zle_misc.c:195.
pub fn killwholeline() -> i32 {                                 // c:195
    let mut n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    if n < 0 {                                                               // c:199
        return 1;                                                            // c:200
    }
    while n > 0 {                                                            // c:201
        // c:202-203 — last-line edge: at zlell with non-empty buffer
        // step back one so the trailing '\n' belongs to this line.
        let _fg = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
        if _fg {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        // c:204-205 — walk back to bol.
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] != '\n' {
            crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
        // c:206 — `for (i=zlecs; i!=zlell && zleline[i]!='\n'; i++)`.
        let mut i = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
        while i != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[i] != '\n' {
            i += 1;
        }
        // c:207 — `forekill(i - zlecs + (i != zlell), ...)`. Include
        // the trailing '\n' if there is one.
        let drop = i - crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + (if i != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { 1 } else { 0 });
        if drop > 0 {
            let text: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + drop).collect();
            crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_front(text);
            if crate::ported::zle::zle_main::KILLRING.lock().unwrap().len() > crate::ported::zle::zle_main::KILLRINGMAX.load(std::sync::atomic::Ordering::SeqCst) {
                crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_back();
            }
            crate::ported::zle::zle_main::ZLELL.fetch_sub(drop, std::sync::atomic::Ordering::SeqCst);
        }
        n -= 1;
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:210
}

/// Port of `makeparamsuffix(int br, int n)` from Src/Zle/zle_misc.c:1623.
pub fn makeparamsuffix(br: i32, n: i32) {                                    // c:1623
    // C body (c:1692-1697): `charstr = ":[#%?-+="; lenstr = (br ||
    //                       unset(KSHARRAYS)) ? 2 : strlen(charstr);
    //                       addsuffix(SUFTYP_POSSTR, 0, charstr, lenstr, n)`.
    let charstr: Vec<char> = ":[#%?-+=".chars().collect();
    let kshcheck = !crate::ported::zsh_h::isset(crate::ported::zsh_h::KSHARRAYS);
    let lenstr = if br != 0 || kshcheck { 2 } else { charstr.len() as i32 };
    let prefix: Vec<char> = charstr.iter().take(lenstr as usize).copied().collect();
    addsuffix(0, 0, prefix, lenstr, n);
}

/// Port of `makequote(ZLE_STRING_T str, size_t *len)` from Src/Zle/zle_misc.c:1201.
/// WARNING: param names don't match C — Rust=(s) vs C=(str, len)
pub fn makequote(s: &[char]) -> Vec<char> {                                  // c:1201
    // c:1170-1173 — count qtct = number of `'` chars.
    let qtct = s.iter().filter(|&&c| c == '\'').count();
    // c:1174 — `*len += 2 + qtct*3`. Output capacity: 2 (outer
    // quotes) + len + qtct*3 (each ' becomes '\\'').
    let mut out = Vec::<char>::with_capacity(s.len() + 2 + qtct * 3);
    out.push('\'');                                                          // c:1176 *l++ = '\''
    for &c in s {                                                            // c:1177-1184
        if c == '\'' {
            // c:1179-1182 — ' → '\''
            out.push('\'');
            out.push('\\');
            out.push('\'');
            out.push('\'');
        } else {
            out.push(c);
        }
    }
    out.push('\'');                                                          // c:1185 *l++ = '\''
    out
}

/// Port of `makesuffix(int n)` from Src/Zle/zle_misc.c:1598.
pub fn makesuffix(n: i32) {                                                  // c:1598
    // C body (c:1642-1652): `suffixchars = getsparam_u(
    //                       "ZLE_REMOVE_SUFFIX_CHARS"); if (!suffixchars)
    //                       suffixchars = " \\t\\n;&|"; addsuffix(...)`.
    let suffix_chars = std::env::var("ZLE_REMOVE_SUFFIX_CHARS")
        .unwrap_or_else(|_| " \t\n;&|".to_string());
    addsuffixstring(0, 0, &suffix_chars, n);
}

/// Port of `makesuffixstr(char *f, char *s, int n)` from Src/Zle/zle_misc.c:1642.
pub fn makesuffixstr(f: Option<&str>, s: Option<&str>, n: i32) {  // c:1642
    // C body: `if (str) addsuffixstring(0, 0, str, n);
    //          if (funcnam) suffixfunc = funcnam`. zshrs's suffixfunc
    // global isn't set up; faithful path covers the str argument.
    if let Some(s) = s {
        addsuffixstring(0, 0, s, n);
    }
}

/// Port of `negargument(UNUSED(char **args))` from `Src/Zle/zle_misc.c:974`.
/// ```c
/// int
/// negargument(UNUSED(char **args))
/// {
///     if (zmod.flags & MOD_TMULT)
///         return 1;
///     zmod.tmult = -1;
///     zmod.flags |= MOD_TMULT|MOD_NEG;
///     prefixflag = 1;
///     return 0;
/// }
/// ```
/// `negative-argument` widget — start a negative count prefix.
/// Refuses if a tmult is already in flight.
pub fn negargument() -> i32 {                                   // c:974
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_TMULT != 0 {                       // c:976
        return 1;                                                            // c:977
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = -1;                                                     // c:978
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_TMULT | MOD_NEG;             // c:979
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);                                                   // c:980
    0                                                                        // c:981 return 0
}

/// Port of `overwritemode(UNUSED(char **args))` from `Src/Zle/zle_misc.c:843`.
/// ```c
/// int
/// overwritemode(UNUSED(char **args))
/// {
///     insmode ^= 1;
///     return 0;
/// }
/// ```
/// `overwrite-mode` widget — toggle insert/overwrite mode.
pub fn overwritemode() -> i32 {                                 // c:843
    crate::ported::zle::zle_main::INSMODE.fetch_xor(1, std::sync::atomic::Ordering::SeqCst);                                              // c:843 insmode ^= 1
    0                                                                        // c:846 return 0
}

/// Port of `parsedigit(int inkey)` from Src/Zle/zle_misc.c:919.
/// WARNING: param names don't match C — Rust=(zle, inkey) vs C=(inkey)
pub fn parsedigit(inkey: i32) -> i32 {                            // c:919
    // c:1077 — `inkey &= 0x7f` (mask off Meta bit). Multibyte path
    // skips this; we mirror by always masking since Rust char vals
    // fit ASCII for digit chars.
    let inkey = inkey & 0x7f;
    let base = crate::ported::zle::zle_main::ZMOD.lock().unwrap().base;
    // c:1082-1090 — base > 10: accept lowercase a..(a+base-11) and
    // uppercase, plus digits 0-9.
    if base > 10 {
        if (b'a' as i32..b'a' as i32 + base - 10).contains(&inkey) {
            return inkey - b'a' as i32 + 10;                                 // c:1083
        }
        if (b'A' as i32..b'A' as i32 + base - 10).contains(&inkey) {
            return inkey - b'A' as i32 + 10;                                 // c:1085
        }
        if (b'0' as i32..=b'9' as i32).contains(&inkey) {                    // c:1087 idigit
            return inkey - b'0' as i32;
        }
        return -1;                                                           // c:1089
    }
    // c:1092-1093 — base <= 10: digit must be in '0'..'0'+base.
    if (b'0' as i32..b'0' as i32 + base).contains(&inkey) {
        return inkey - b'0' as i32;
    }
    -1                                                                       // c:1094
}

/// Port of `pastebuf(Cutbuffer buf, int mult, int position)` from Src/Zle/zle_misc.c:558.
/// WARNING: param names don't match C — Rust=(zle, buf, mult, position) vs C=(buf, mult, position)
pub fn pastebuf(buf: &[char], mult: i32, position: i32) -> i32 {  // c:558
    // Simplified port of pastebuf. The C source dispatches on
    // CUTBUFFER_LINE flag (insert as full lines vs char-wise),
    // computes position 0/1/2 (before/after/split), and updates
    // yankb/yanke. Without the LINE-flag check (we treat all as
    // char-wise) plus the simple before/after path we get the
    // common case.
    if buf.is_empty() {
        return 0;
    }
    // c:591-592 — `if (position == 1 && zlecs != findeol()) INCCS()`.
    if position == 1 && crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    // c:593 — `yankb = zlecs`.
    crate::ported::zle::zle_main::YANKB.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    // c:595-599 — `while (mult--) { spaceinline(cc); ZS_memcpy; zlecs += cc }`.
    let mut n = mult;
    while n > 0 {
        for (i, &c) in buf.iter().enumerate() {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, c);
        }
        crate::ported::zle::zle_main::ZLECS.fetch_add(buf.len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.fetch_add(buf.len(), std::sync::atomic::Ordering::SeqCst);
        n -= 1;
    }
    // c:600 — `yanke = zlecs`.
    crate::ported::zle::zle_main::YANKE.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    // c:601-602 — vicmd → DECCS.
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd" {
        crate::ported::zle::zle_main::ZLECS.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `poundinsert(UNUSED(char **args))` from Src/Zle/zle_misc.c:369.
pub fn poundinsert() -> i32 {                                   // c:369
    use std::sync::atomic::Ordering;
    use crate::ported::zle::zle_move::vifirstnonblank;
    // c:371-393 — `zlecs = 0; vifirstnonblank(zlenoargs);
    //              if (zleline[zlecs] != '#') { spaceinline(1);
    //                  zleline[zlecs] = '#'; zlecs = findeol();
    //                  while (zlecs != zlell) { ... } }
    //              else { foredel(1, 0); zlecs = findeol(); ... }
    //              done = 1; return 0`.
    crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);                                                           // c:371
    vifirstnonblank();                                                    // c:372
    let at_pound = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'#');
    if !at_pound {
        // c:374-383 — insert # at this line, advance to next, repeat.
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), '#');
        crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol(), std::sync::atomic::Ordering::SeqCst);
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            vifirstnonblank();
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), '#');
            crate::ported::zle::zle_main::ZLELL.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol(), std::sync::atomic::Ordering::SeqCst);
        }
    } else {
        // c:384-393 — strip leading # from each line.
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
        crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol(), std::sync::atomic::Ordering::SeqCst);
        while crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
            crate::ported::zle::zle_main::ZLECS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            vifirstnonblank();
            if crate::ported::zle::zle_main::ZLELINE.lock().unwrap().get(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)) == Some(&'#') {
                crate::ported::zle::zle_main::ZLELINE.lock().unwrap().remove(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
                crate::ported::zle::zle_main::ZLELL.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            }
            crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_utils::findeol(), std::sync::atomic::Ordering::SeqCst);
        }
    }
    DONE.store(1, Ordering::SeqCst);                                         // c:395
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:396
}

/// Port of `putreplaceselection(UNUSED(char **args))` from Src/Zle/zle_misc.c:680.
pub fn putreplaceselection() -> i32 {                           // c:680
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    use crate::ported::zle::zle_vi::startvichange;
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;                                                   // c:682
    let mut pos = 2;                                                         // c:686
    startvichange(-1);                                                  // c:688
    if n < 0 || crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return 1;                                                            // c:690
    }
    let prevbuf: Vec<char> = if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        let idx = crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf as usize;
        if idx >= crate::ported::zle::zle_main::vibuf().lock().unwrap().len() {
            return 1;
        }
        crate::ported::zle::zle_main::vibuf().lock().unwrap()[idx].clone()                                               // c:700
    } else {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().cloned().unwrap_or_default()                    // c:702
    };
    if prevbuf.is_empty() {
        return 1;                                                            // c:702
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = 0;                                 // c:712
    if crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) == 2 {                                              // c:713
        // c:714-717 — regionlines split; lines-flag check elided.
        pos = if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) { 1 } else { 0 };
    }
    let _ = killregion();                                                 // c:719
    pastebuf(&prevbuf, n, pos)                                          // c:721
}

/// Direct port of `int quotedinsert(char **args)` from
/// `Src/Zle/zle_misc.c:899`.
/// ```c
/// // (raw-mode tweak for non-HAS_TIO systems — skipped on Linux/macOS)
/// getfullchar(0);
/// if (LASTFULLCHAR == ZLEEOF) return 1;
/// return selfinsert();
/// ```
/// HAS_TIO is set everywhere zshrs builds (Linux/macOS), so the
/// raw-mode/ioctl branch is unreachable — `getfullchar` already
/// runs in the right mode via `zsetterm`. We invoke it explicitly
/// for a one-shot read, then forward to `selfinsert`.
pub fn quotedinsert() -> i32 {                                  // c:899
    // c:899 — `getfullchar(0)`. Reads one full char, updates
    // crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) / lastchar_wide / lastchar_wide_valid.
    let _ = getfullchar(false);
    if crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst) < 0 {                                                    // c:919 LASTFULLCHAR == ZLEEOF
        return 1;
    }
    selfinsert()                                                          // c:922
}

/// Port of `quoteline(UNUSED(char **args))` from Src/Zle/zle_misc.c:1187.
pub fn quoteline() -> i32 {                                     // c:1187
    // c:1187 — `len = zlell`. Quote whole buffer.
    let quoted = makequote(&crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst)]);
    let len = quoted.len();
    // c:1193-1195 — `sizeline; ZS_memcpy; zlecs = zlell = len`.
    *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = quoted;
    crate::ported::zle::zle_main::ZLELL.store(len, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.store(len, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0                                                                        // c:1196
}

/// Port of `quoteregion(UNUSED(char **args))` from Src/Zle/zle_misc.c:1152.
pub fn quoteregion() -> i32 {                                   // c:1152
    // c:1152 — `int extra = invicmdmode()`. Vi-cmd-mode bias.
    let mut extra = *crate::ported::zle::zle_keymap::curkeymapname() == "vicmd";
    // c:1158-1159 — `if (mark > zlell) mark = zlell`.
    if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }
    // c:1160-1170 — visual-line vs. char modes; normalize zlecs/mark.
    if crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) == 2 {
        let (a, b) = regionlines();
        crate::ported::zle::zle_main::ZLECS.store(a, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(b, std::sync::atomic::Ordering::SeqCst);
        extra = false;
    } else if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
        std::mem::swap(&mut crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), &mut crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst));
    }
    // c:1171-1172 — `if (extra) INCPOS(mark)`. Include cursor cell.
    if extra && crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {
        crate::ported::zle::zle_main::MARK.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
    // c:1173-1175 — copy region into temp str; foredel; quote; insert.
    let region: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst)].to_vec();
    let len = region.len();
    let quoted = makequote(&region);
    let qlen = quoted.len();
    // c:1176 — `foredel(len, CUT_RAW)` — delete region (no kill).
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)..crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + len);
    crate::ported::zle::zle_main::ZLELL.fetch_sub(len, std::sync::atomic::Ordering::SeqCst);
    // c:1178-1179 — insert quoted text at cursor.
    for (i, &c) in quoted.iter().enumerate() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, c);
    }
    crate::ported::zle::zle_main::ZLELL.fetch_add(qlen, std::sync::atomic::Ordering::SeqCst);
    // c:1180-1181 — `mark = zlecs; zlecs += len`.
    crate::ported::zle::zle_main::MARK.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLECS.fetch_add(qlen, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

/// Port of `regionlines(int *start, int *end)` from Src/Zle/zle_misc.c:444.
/// WARNING: param names don't match C — Rust=(zle) vs C=(start, end)
pub fn regionlines() -> (usize, usize) {                        // c:444
    use crate::ported::zle::zle_utils::{findbol, findeol};
    // c:446 — `int origcs = zlecs`. Save cursor.
    let origcs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let start;
    let end;
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) < crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) {                                                // c:449
        // c:450-452 — start=findbol(); zlecs=min(mark,zlell); end=findeol().
        start = findbol();
        crate::ported::zle::zle_main::ZLECS.store(if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) > crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) { crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) } else { crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) }, std::sync::atomic::Ordering::SeqCst);
        end = findeol();
    } else {
        // c:454-456 — end=findeol(); zlecs=mark; start=findbol().
        end = findeol();
        crate::ported::zle::zle_main::ZLECS.store(crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        start = findbol();
    }
    // c:458 — `zlecs = origcs`. Restore.
    crate::ported::zle::zle_main::ZLECS.store(origcs, std::sync::atomic::Ordering::SeqCst);
    (start, end)
}

/// Port of `static char *namedcmdstr` from `Src/Zle/zle_misc.c:1229`.
pub static namedcmdstr: std::sync::Mutex<String> =                           // c:1229
    std::sync::Mutex::new(String::new());

/// Port of `static LinkList namedcmdll` from `Src/Zle/zle_misc.c:1235`.
pub static namedcmdll: std::sync::Mutex<Vec<String>> =                       // c:1235
    std::sync::Mutex::new(Vec::new());

/// Port of `static int namedcmdambig` from `Src/Zle/zle_misc.c:1235`.
pub static namedcmdambig: std::sync::atomic::AtomicUsize =                   // c:1235
    std::sync::atomic::AtomicUsize::new(0);

/// Direct port of `static int scancompcmd(HashNode hn, UNUSED(int flags))`
/// from `Src/Zle/zle_misc.c:1235`.
pub fn scancompcmd(name: &str) -> i32 {                                      // c:1235
    use std::sync::atomic::Ordering;
    // c:1240 — `if (strpfx(namedcmdstr, t->nam))`.
    let prefix = namedcmdstr.lock().unwrap().clone();
    if !name.starts_with(&prefix) { return 0; }
    let mut ll = namedcmdll.lock().unwrap();
    let first = ll.first().cloned();
    ll.push(name.to_string());                                               // c:1241 addlinknode
    if let Some(f) = first {
        // c:1242 — `pfxlen(peekfirst(namedcmdll), t->nam)`.
        let l = f.bytes().zip(name.bytes()).take_while(|(a, b)| a == b).count();
        if l < namedcmdambig.load(Ordering::Relaxed) {
            namedcmdambig.store(l, Ordering::Relaxed);                       // c:1243
        }
    } else {
        namedcmdambig.store(name.len(), Ordering::Relaxed);
    }
    0
}

/// Direct port of `int selfinsert(char **args)` from
/// `Src/Zle/zle_misc.c:112-126`.
/// ```c
/// if (!lastchar_wide_valid)
///     getrestchar(lastchar, NULL);
/// // tmp = LASTFULLCHAR;
/// doinsert(&tmp, 1);
/// return 0;
/// ```
///
/// **Multibyte tradeoff:** C's `getrestchar` reassembles a wide
/// char from `lastchar` + buffered continuation bytes when the
/// `wide_valid` flag is clear. Rust's `Zle::getfullchar` (zle_main
/// .rs:730) already produces a full char per read, so by the time
/// `selfinsert` fires, `lastchar` IS the full codepoint — the
/// `wide_valid=false` branch is unreachable in the Rust input path
/// and the ASCII-promotion is the correct fallback for the rare
/// case where a widget sets `lastchar` directly.
/// Port of `selfinsert(UNUSED(char **args))` from `Src/Zle/zle_misc.c:113`.
pub fn selfinsert() -> i32 {                                    // c:113
    if !(crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.load(std::sync::atomic::Ordering::SeqCst) != 0) {                                            // c:113
        crate::ported::zle::zle_main::LASTCHAR_WIDE.store((crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst)) as i32, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(1, std::sync::atomic::Ordering::SeqCst);
    }
    // c:123 — `tmp = LASTFULLCHAR; doinsert(&tmp, 1)`.
    if let Some(c) = char::from_u32(crate::ported::zle::zle_main::LASTCHAR_WIDE.load(std::sync::atomic::Ordering::SeqCst) as u32) {
        self_insert(c);
    }
    0                                                                        // c:125
}

/// Port of `selfinsertunmeta(char **args)` from Src/Zle/zle_misc.c:149.
pub fn selfinsertunmeta() -> i32 {                              // c:149
    // c:149-152 — `fixunmeta(); return selfinsert()`.
    fixunmeta();
    selfinsert()
}

/// Port of `sendbreak(UNUSED(char **args))` from `Src/Zle/zle_misc.c:1144`.
/// ```c
/// int
/// sendbreak(UNUSED(char **args))
/// {
///     errflag |= ERRFLAG_ERROR|ERRFLAG_INT;
///     return 1;
/// }
/// ```
/// `send-break` widget — abort the current editor session by
/// raising both `ERRFLAG_ERROR` and `ERRFLAG_INT` on the global
/// `errflag`, so `zleread` returns -1 to its caller.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn sendbreak() -> i32 {                                                  // c:1144
    // c:1144 — `errflag |= ERRFLAG_ERROR | ERRFLAG_INT`.
    crate::ported::utils::errflag.fetch_or(
        crate::ported::zsh_h::ERRFLAG_ERROR | crate::ported::zsh_h::ERRFLAG_INT,
        std::sync::atomic::Ordering::Relaxed,
    );
    1                                                                        // c:1147 return 1
}

/// Port of `transpose_swap(int start, int middle, int end)` from `Src/Zle/zle_misc.c:254`.
/// ```c
/// static void
/// transpose_swap(int start, int middle, int end)
/// {
///     int len1, len2;
///     ZLE_STRING_T first;
///     len1 = middle - start;
///     len2 = end - middle;
///     first = (ZLE_STRING_T)zalloc(len1 * ZLE_CHAR_SIZE);
///     ZS_memcpy(first, zleline + start, len1);
///     /* Move may be overlapping... */
///     ZS_memmove(zleline + start, zleline + middle, len2);
///     ZS_memcpy(zleline + start + len2, first, len1);
///     zfree(first, len1 * ZLE_CHAR_SIZE);
/// }
/// ```
/// Swap two adjacent slices in the line buffer:
/// `zleline[start..middle]` and `zleline[middle..end]`. After the
/// swap, `zleline[start..start+(end-middle)]` holds the second
/// chunk and `zleline[start+(end-middle)..end]` holds the first.
/// WARNING: param names don't match C — Rust=(zle, start, middle, end) vs C=(start, middle, end)
pub fn transpose_swap(start: usize, middle: usize, end: usize) {  // c:255
    let len1 = middle - start;                                               // c:255
    let len2 = end - middle;                                                 // c:261
    // c:263-264 — copy first slice into temp buffer.
    let first: Vec<char> = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start..middle].to_vec();
    // c:266 — `ZS_memmove(zleline + start, zleline + middle, len2)`.
    // Vec doesn't overlap when copy_within is used.
    crate::ported::zle::zle_main::ZLELINE.lock().unwrap().copy_within(middle..end, start);
    // c:267 — `ZS_memcpy(zleline + start + len2, first, len1)`.
    for (i, &ch) in first.iter().enumerate() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[start + len2 + i] = ch;
    }
    let _ = len1;
}

/// Port of `transposechars(UNUSED(char **args))` from Src/Zle/zle_misc.c:313.
pub fn transposechars() -> i32 {                                // c:313
    use crate::ported::zle::zle_move::{deccs, decpos, inccs, incpos};
    let mut n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;
    let neg = n < 0;                                                         // c:317
    if neg {
        n = -n;                                                              // c:319
    }
    while n > 0 {                                                            // c:321
        n -= 1;
        let mut ct = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);                                              // c:322
        if ct == 0 || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] == '\n' {                   // c:322
            if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] == '\n' {    // c:323
                return 1;
            }
            if !neg {
                inccs();                                                  // c:326
            }
            incpos(&mut ct);                                                 // c:327
        }
        if neg {
            if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) > 0 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - 1] != '\n' {         // c:330
                deccs();                                                  // c:331
                if ct > 1 && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[ct - 2] != '\n' {                   // c:332
                    decpos(&mut ct);                                         // c:333
                }
            }
        } else if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) != crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) && crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)] != '\n' {
            inccs();                                                      // c:338
        }
        if ct == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[ct] == '\n' {                      // c:340
            decpos(&mut ct);                                                 // c:341
        }
        if ct < 1 || crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[ct - 1] == '\n' {                           // c:343
            return 1;
        }
        // c:345-358 — MULTIBYTE branch uses transpose_swap with surrounding
        //              positions; non-multibyte branch swaps two ZLE_CHAR_T.
        //              Rust ZleString is Vec<char> so we can swap directly.
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().swap(ct - 1, ct);
    }
    0
}

/// Port of `undefinedkey(UNUSED(char **args))` from `Src/Zle/zle_misc.c:892`.
/// ```c
/// int
/// undefinedkey(UNUSED(char **args))
/// {
///     return 1;
/// }
/// ```
/// `undefined-key` widget — bound to key sequences that aren't
/// otherwise defined; returns 1 so the dispatcher beeps.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn undefinedkey() -> i32 {                                               // c:892
    // c:892 — `return 1`. The widget binds to keys with no other
    // function and just signals "unhandled" by returning non-zero.
    1
}

/// Port of `universalargument(char **args)` from Src/Zle/zle_misc.c:986.
/// WARNING: param names don't match C — Rust=(zle, args) vs C=(args)
pub fn universalargument(args: &[String]) -> i32 {            // c:986
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    // c:988-993 — `if (*args)` short-circuit when invoked with an
    //              explicit numeric arg.
    if let Some(a) = args.first() {
        if let Ok(n) = a.parse::<i32>() {
            crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = n;
            crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_MULT;
            return 0;
        }
    }
    // c:1009-1023 — interactive byte-by-byte digit collection. Without
    //               a live keystream we mirror the no-input branch
    //               (no digits) which multiplies tmult by 4.
    let digcnt = 0;
    if digcnt == 0 {
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult.saturating_mul(4);                   // c:1027
    }
    crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_TMULT;                             // c:1029
    crate::ported::zle::zle_main::PREFIXFLAG.store(1, std::sync::atomic::Ordering::SeqCst);                                                   // c:1030
    0
}

/// Port of `viputafter(UNUSED(char **args))` from Src/Zle/zle_misc.c:644.
pub fn viputafter() -> i32 {                                    // c:644
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    use crate::ported::zle::zle_vi::startvichange;
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;                                                   // c:646
    startvichange(-1);                                                  // c:648
    if n < 0 {
        return 1;                                                            // c:650
    }
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return 0;                                                            // c:652
    }
    // c:653-665 — OS selection branch (MOD_OSSEL = PRI|CLIP). Without
    //              system_clipget we fall through to the cut-buffer path.
    let buf: Vec<char> = if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        let idx = crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf as usize;
        if idx >= crate::ported::zle::zle_main::vibuf().lock().unwrap().len() {
            return 1;
        }
        crate::ported::zle::zle_main::vibuf().lock().unwrap()[idx].clone()                                               // c:667
    } else {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().cloned().unwrap_or_default()                                                   // c:669
    };
    if buf.is_empty() {
        return 1;                                                            // c:671
    }
    pastebuf(&buf, n, 1)                                                // c:675
}

/// Port of `viputbefore(UNUSED(char **args))` from Src/Zle/zle_misc.c:608.
pub fn viputbefore() -> i32 {                                   // c:608
    use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
    use crate::ported::zle::zle_vi::startvichange;
    let n = crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult;                                                   // c:610
    startvichange(-1);                                                  // c:612
    if n < 0 {
        return 1;                                                            // c:614
    }
    if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_NULL != 0 {
        return 0;                                                            // c:616
    }
    let buf: Vec<char> = if crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_VIBUF != 0 {
        let idx = crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf as usize;
        if idx >= crate::ported::zle::zle_main::vibuf().lock().unwrap().len() {
            return 1;
        }
        crate::ported::zle::zle_main::vibuf().lock().unwrap()[idx].clone()                                               // c:631
    } else {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().cloned().unwrap_or_default()                                                   // c:633
    };
    if buf.is_empty() {
        return 1;                                                            // c:635
    }
    pastebuf(&buf, n, 0)                                                // c:639
}

/// Port of `whatcursorposition(UNUSED(char **args))` from Src/Zle/zle_misc.c:851.
pub fn whatcursorposition() -> i32 {                            // c:851
    use crate::ported::zle::zle_utils::findbol;
    let bol = findbol();                                                  // c:855
    let mut msg = String::with_capacity(100);
    if crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) == crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) {                                              // c:858
        msg.push_str("EOF");                                                 // c:859
    } else {
        msg.push_str("Char: ");                                              // c:861
        let c = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst)];                                      // c:856
        match c {
            ' ' => msg.push_str("SPC"),                                      // c:864
            '\t' => msg.push_str("TAB"),                                     // c:867
            '\n' => msg.push_str("LFD"),                                     // c:870
            _ => msg.push(c),                                                // c:878
        }
        let cu = c as u32;
        msg.push_str(&format!(" (0{:o}, {}, 0x{:x})", cu, cu, cu));          // c:881
    }
    let pct = if crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) > 0 { 100 * crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) / crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) } else { 0 };
    msg.push_str(&format!(
        "  point {} of {}({}%)  column {}",
        crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + 1,
        crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst) + 1,
        pct,
        crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) - bol,
    ));                                                                      // c:884
    tracing::info!(target: "args", "{}", msg);                                // c:887 — showmsg
    0
}

/// Port of `yankpop(UNUSED(char **args))` from Src/Zle/zle_misc.c:728.
pub fn yankpop() -> i32 {                                       // c:728
    use crate::ported::zle::widget::WidgetFlags;
    // c:730-735 — `if (!(lastcmd & ZLE_YANK) || !kring || !kctbuf)
    //               return 1`.
    let last = WidgetFlags::from_bits_truncate(
        crate::ported::zle::zle_main::LASTCMD.load(std::sync::atomic::Ordering::SeqCst),
    );
    if !last.contains(WidgetFlags::YANK) || crate::ported::zle::zle_main::KILLRING.lock().unwrap().is_empty() {
        return 1;
    }
    // C body cycles the kill ring index `kct` and re-inserts the
    // previous yank. zshrs uses VecDeque<ZleString> with the rotation
    // index `yank_ring_idx`. Simplified: rotate front entry to back,
    // delete previous yank text from line, insert new front.
    let prev_start = crate::ported::zle::zle_main::YANKB.load(std::sync::atomic::Ordering::SeqCst);
    let prev_end   = crate::ported::zle::zle_main::YANKE.load(std::sync::atomic::Ordering::SeqCst);
    if prev_end > prev_start && prev_end <= crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len() {
        crate::ported::zle::zle_main::ZLELINE.lock().unwrap().drain(prev_start..prev_end);
        crate::ported::zle::zle_main::ZLELL.fetch_sub(prev_end - prev_start, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(prev_start, std::sync::atomic::Ordering::SeqCst);
    }
    if let Some(top) = crate::ported::zle::zle_main::KILLRING.lock().unwrap().pop_front() {
        crate::ported::zle::zle_main::KILLRING.lock().unwrap().push_back(top);
    }
    if let Some(next) = crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().cloned() {
        for (i, &c) in next.iter().enumerate() {
            crate::ported::zle::zle_main::ZLELINE.lock().unwrap().insert(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) + i, c);
        }
        crate::ported::zle::zle_main::YANKB.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.fetch_add(next.len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLELL.fetch_add(next.len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::YANKE.store(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), std::sync::atomic::Ordering::SeqCst);
    }
    crate::ported::zle::zle_main::ZLE_RESET_NEEDED.store(1, std::sync::atomic::Ordering::SeqCst);
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn acceptline_sets_done() {
        // c:401-404 — `done = 1; return 0`.
        DONE.store(0, Ordering::SeqCst);
        let r = acceptline();
        assert_eq!(r, 0);
        assert_eq!(DONE.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn undefinedkey_returns_one() {
        // c:892-894 — single `return 1` body.
        assert_eq!(undefinedkey(), 1);
    }

    #[test]
    fn sendbreak_sets_errflag_and_returns_one() {
        use crate::ported::zsh_h::{ERRFLAG_ERROR, ERRFLAG_INT};
        use std::sync::atomic::Ordering;
        // Reset errflag so the OR-set is observable.
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
        let r = sendbreak();
        // c:1147 — return 1.
        assert_eq!(r, 1);
        // c:1146 — both ERRFLAG_ERROR | ERRFLAG_INT set.
        let f = crate::ported::utils::errflag.load(Ordering::Relaxed);
        assert!(f & ERRFLAG_ERROR != 0);
        assert!(f & ERRFLAG_INT != 0);
        // Reset for other tests.
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
    }

    #[test]
    fn sendbreak_preserves_existing_errflag_bits() {
        use std::sync::atomic::Ordering;
        // c:1146 — `errflag |= ...` (OR-equal, not assign).
        crate::ported::utils::errflag.store(0x1000, Ordering::Relaxed); // pretend bit 12 was set
        sendbreak();
        let f = crate::ported::utils::errflag.load(Ordering::Relaxed);
        // Pre-existing bit preserved.
        assert!(f & 0x1000 != 0);
        // New bits also set.
        assert!(f & crate::ported::zsh_h::ERRFLAG_ERROR != 0);
        assert!(f & crate::ported::zsh_h::ERRFLAG_INT != 0);
        crate::ported::utils::errflag.store(0, Ordering::Relaxed);
    }

    // ---------- negargument / overwritemode real-port tests ----------

    #[test]
    fn negargument_sets_tmult_neg_prefix() {
        // c:976-981 — sets tmult=-1 + TMULT|NEG flags + prefixflag.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        // Ensure clean modifier state.
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 1;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = 0;
        crate::ported::zle::zle_main::PREFIXFLAG.store(0, std::sync::atomic::Ordering::SeqCst);
        let r = negargument();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, -1);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_TMULT != 0);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_NEG != 0);
        assert!(crate::ported::zle::zle_main::PREFIXFLAG.load(std::sync::atomic::Ordering::SeqCst) != 0);
    }

    #[test]
    fn negargument_refuses_when_tmult_in_flight() {
        // c:976-977 — if MOD_TMULT already set → return 1.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags |= MOD_TMULT;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 7; // some pre-existing value
        let r = negargument();
        assert_eq!(r, 1);
        // tmult NOT clobbered (early return).
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 7);
    }

    #[test]
    fn overwritemode_toggles_insmode() {
        // c:845 — `insmode ^= 1`.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::INSMODE.store(1, std::sync::atomic::Ordering::SeqCst);
        overwritemode();
        assert_eq!(crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst), 0);
        overwritemode();
        assert_eq!(crate::ported::zle::zle_main::INSMODE.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    // ---------- argumentbase real-port tests ----------

    #[test]
    fn argumentbase_with_arg_sets_base() {
        // c:1043 — parse arg, c:1050 set zmod.base.
        let mut z = Zle::new();
        let r = argumentbase(&["8".to_string()]);
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().base, 8);
        assert!(crate::ported::zle::zle_main::PREFIXFLAG.load(std::sync::atomic::Ordering::SeqCst) != 0);
        // c:1053-1056 — modifier reset.
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult, 1);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 1);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().vibuf, 0);
    }

    #[test]
    fn argumentbase_no_arg_uses_zmod_mult() {
        // c:1045 — fallback to zmod.mult when no arg.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 16;
        argumentbase(&[]);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().base, 16);
    }

    #[test]
    fn argumentbase_rejects_below_two() {
        // c:1047-1048 — base < 2 → return 1, state unchanged.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        let r = argumentbase(&["1".to_string()]);
        assert_eq!(r, 1);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().base, 10); // unchanged
    }

    #[test]
    fn argumentbase_rejects_above_36() {
        // c:1047-1048 — base > 36 → return 1.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        let r = argumentbase(&["100".to_string()]);
        assert_eq!(r, 1);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().base, 10);
    }

    #[test]
    fn argumentbase_hex_prefix() {
        // c:1043 — `zstrtol(s, NULL, 0)`: '0x10' → 16.
        let mut z = Zle::new();
        argumentbase(&["0x10".to_string()]);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().base, 16);
    }

    #[test]
    fn argumentbase_octal_prefix() {
        // c:1043 — '010' → octal 8.
        let mut z = Zle::new();
        argumentbase(&["010".to_string()]);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().base, 8);
    }

    // ---------- parsedigit real-port tests ----------

    #[test]
    fn parsedigit_decimal_base() {
        // c:1092 — base=10, '0'..'9' → 0..9.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        assert_eq!(parsedigit(b'0' as i32), 0);
        assert_eq!(parsedigit(b'5' as i32), 5);
        assert_eq!(parsedigit(b'9' as i32), 9);
        // Out of range for base 10
        assert_eq!(parsedigit(b'a' as i32), -1);
    }

    #[test]
    fn parsedigit_octal_base() {
        // c:1092 — base=8, '0'..'7'.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 8;
        assert_eq!(parsedigit(b'7' as i32), 7);
        // '8' rejected (out of range for octal).
        assert_eq!(parsedigit(b'8' as i32), -1);
    }

    #[test]
    fn parsedigit_hex_lowercase() {
        // c:1083 — base=16, 'a'..'f' → 10..15.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 16;
        assert_eq!(parsedigit(b'a' as i32), 10);
        assert_eq!(parsedigit(b'f' as i32), 15);
        // 'g' out of range (only a..f for base 16).
        assert_eq!(parsedigit(b'g' as i32), -1);
    }

    #[test]
    fn parsedigit_hex_uppercase() {
        // c:1085 — base=16, 'A'..'F' → 10..15.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 16;
        assert_eq!(parsedigit(b'A' as i32), 10);
        assert_eq!(parsedigit(b'F' as i32), 15);
    }

    #[test]
    fn parsedigit_hex_digits_still_work() {
        // c:1087 — base > 10 still accepts '0'..'9' via idigit branch.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 16;
        assert_eq!(parsedigit(b'7' as i32), 7);
    }

    #[test]
    fn parsedigit_strips_meta_bit() {
        // c:1077 — `inkey &= 0x7f`. 0xb5 = '5' | 0x80 → strips to '5'.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        assert_eq!(parsedigit(0x80 | (b'5' as i32)), 5);
    }

    // ---------- digitargument real-port tests ----------

    #[test]
    fn digitargument_first_digit_no_tmult() {
        // c:1050-1051 — `if (!TMULT) tmult = 0`. First digit: tmult=0
        // then tmult = 0*10 + 1*5 = 5.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = 0;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1; // sign = 1
        crate::ported::zle::compcore::LASTCHAR.store((b'5' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        let r = digitargument();
        assert_eq!(r, 0);
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 5);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_TMULT != 0);
        assert!(crate::ported::zle::zle_main::PREFIXFLAG.load(std::sync::atomic::Ordering::SeqCst) != 0);
    }

    #[test]
    fn digitargument_second_digit_accumulates() {
        // c:1058 — second digit: tmult = 5*10 + 1*7 = 57.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = MOD_TMULT;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = 5;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1; // sign = 1
        crate::ported::zle::compcore::LASTCHAR.store((b'7' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        digitargument();
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, 57);
    }

    #[test]
    fn digitargument_invalid_returns_one() {
        // c:1047-1048 — parsedigit < 0 → return 1.
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        crate::ported::zle::compcore::LASTCHAR.store((b'a' as i32) as i32, std::sync::atomic::Ordering::SeqCst); // not a decimal digit
        assert_eq!(digitargument(), 1);
    }

    #[test]
    fn digitargument_neg_flag_replaces_tmult() {
        // c:1054-1056 — MOD_NEG: tmult = sign * newdigit, NEG cleared.
        // sign = -1 (zmult<0); first digit '3' → tmult = -1*3 = -3.
        use crate::ported::zle::zle_h::{MOD_MULT, MOD_TMULT, MOD_VIBUF, MOD_VIAPP, MOD_NEG, MOD_NULL, MOD_CHAR, MOD_LINE, MOD_PRI, MOD_CLIP, MOD_OSSEL};
        let mut z = Zle::new();
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags = MOD_TMULT | MOD_NEG;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult = -1;  // set by negargument
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().base = 10;
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = -1;   // negative → sign = -1
        crate::ported::zle::compcore::LASTCHAR.store((b'3' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        digitargument();
        assert_eq!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().tmult, -3);
        // NEG cleared.
        assert!(!crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_NEG != 0);
        assert!(crate::ported::zle::zle_main::ZMOD.lock().unwrap().flags & MOD_TMULT != 0);
    }

    // ---------- transpose_swap real-port tests ----------

    #[test]
    fn transpose_swap_equal_halves() {
        // c:254 — swap two equal-length adjacent slices.
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        // Swap [0..2]="ab" with [2..4]="cd" → "cdabef".
        transpose_swap(0, 2, 4);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "cdabef");
    }

    #[test]
    fn transpose_swap_unequal_halves() {
        // First chunk len 1, second len 3.
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        // Swap [0..1]="a" with [1..4]="bcd" → "bcdaef".
        transpose_swap(0, 1, 4);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "bcdaef");
    }

    #[test]
    fn transpose_swap_first_longer() {
        // First chunk len 3, second len 1.
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        // Swap [0..3]="abc" with [3..4]="d" → "dabcef".
        transpose_swap(0, 3, 4);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "dabcef");
    }

    #[test]
    fn transpose_swap_mid_buffer() {
        // Swap not at the start.
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "0123456789".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(10, std::sync::atomic::Ordering::SeqCst);
        // Swap [3..5]="34" with [5..7]="56" → "0125634789".
        transpose_swap(3, 5, 7);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "0125634789");
    }

    // ---------- Batch tests for fixunmeta/selfinsert/deletechar/etc ----------

    #[test]
    fn fixunmeta_strips_meta_and_normalizes_cr() {
        let mut z = Zle::new();
        crate::ported::zle::compcore::LASTCHAR.store((0x80 | b'a' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        fixunmeta();
        assert_eq!(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst), b'a' as i32);
        crate::ported::zle::compcore::LASTCHAR.store((b'\r' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        fixunmeta();
        assert_eq!(crate::ported::zle::compcore::LASTCHAR.load(std::sync::atomic::Ordering::SeqCst), b'\n' as i32);
    }

    #[test]
    fn selfinsert_inserts_lastchar() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::compcore::LASTCHAR.store((b'X' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(0, std::sync::atomic::Ordering::SeqCst);
        selfinsert();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "aXbc");
    }

    #[test]
    fn selfinsertunmeta_chains_fixunmeta_and_selfinsert() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::compcore::LASTCHAR.store((0x80 | b'X' as i32) as i32, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::LASTCHAR_WIDE_VALID.store(0, std::sync::atomic::Ordering::SeqCst);
        selfinsertunmeta();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "aXb");
    }

    #[test]
    fn deletechar_removes_n_chars() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 2;
        let r = deletechar();
        assert_eq!(r, 0);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "llo");
    }

    #[test]
    fn deletechar_returns_one_at_eol() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "ab".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        assert_eq!(deletechar(), 1);
    }

    #[test]
    fn backwarddeletechar_clamps_to_zlecs() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 99;
        backwarddeletechar();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "c");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn killline_kills_to_eol_and_pushes_killring() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        killline();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "hello ");
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 6);
        assert_eq!(crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
                   Some("world".to_string()));
    }

    #[test]
    fn killbuffer_clears_and_pushes() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(2, std::sync::atomic::Ordering::SeqCst);
        killbuffer();
        assert!(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().is_empty());
        assert_eq!(crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
                   Some("abc".to_string()));
    }

    #[test]
    fn killwholeline_drops_one_line() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc\ndef\nghi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(5, std::sync::atomic::Ordering::SeqCst); // 'e' in 'def'
        crate::ported::zle::zle_main::ZMOD.lock().unwrap().mult = 1;
        killwholeline();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "abc\nghi");
    }

    #[test]
    fn copyregionaskill_copies_between_point_mark() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(3, std::sync::atomic::Ordering::SeqCst);
        copyregionaskill(&[]);
        assert_eq!(crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
                   Some("hel".to_string()));
        // Buffer unchanged
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "hello");
    }

    #[test]
    fn regionlines_returns_bol_eol_around_region() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc\ndef\nghi".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(11, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(5, std::sync::atomic::Ordering::SeqCst);
        let (start, end) = regionlines();
        // mark > zlecs branch: start=findbol()=0, end=findeol()=7
        assert_eq!(start, 0);
        assert_eq!(end, 7);
    }

    #[test]
    fn killregion_drains_between_mark_and_cursor() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(4, std::sync::atomic::Ordering::SeqCst);
        killregion();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "aef");
        assert_eq!(crate::ported::zle::zle_main::KILLRING.lock().unwrap().front().map(|v| v.iter().collect::<String>()),
                   Some("bcd".to_string()));
    }

    #[test]
    fn quoteline_wraps_in_single_quotes() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abc".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        quoteline();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "'abc'");
    }

    #[test]
    fn quoteline_escapes_internal_quote() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "it's".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(4, std::sync::atomic::Ordering::SeqCst);
        quoteline();
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "'it'\\''s'");
    }

    #[test]
    fn makequote_handles_no_quotes() {
        let s: Vec<char> = "abc".chars().collect();
        let q = makequote(&s);
        assert_eq!(q.iter().collect::<String>(), "'abc'");
    }

    #[test]
    fn makequote_escapes_quotes() {
        let s: Vec<char> = "a'b".chars().collect();
        let q = makequote(&s);
        assert_eq!(q.iter().collect::<String>(), "'a'\\''b'");
    }

    #[test]
    fn pastebuf_inserts_at_cursor_position_zero() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "foo".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        let buf: Vec<char> = "XX".chars().collect();
        pastebuf(&buf, 1, 0);
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "fXXoo");
    }

    #[test]
    fn pastebuf_inserts_after_cursor_position_one() {
        let mut z = Zle::new();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "foo".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(3, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        let buf: Vec<char> = "XX".chars().collect();
        pastebuf(&buf, 1, 1);
        // position=1 → INCCS first → insert at zlecs+1
        let s: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().iter().collect();
        assert_eq!(s, "foXXo");
    }

    #[test]
    fn yankpop_returns_one_when_lastcmd_not_yank() {
        let mut z = Zle::new();
        // Default lastcmd = empty (no YANK flag).
        assert_eq!(yankpop(), 1);
    }

    #[test]
    fn zle_usable_when_active_and_no_compfunc() {
        use crate::ported::builtins::sched::zleactive;
        use crate::ported::zle::complete::INCOMPFUNC;
        use std::sync::atomic::Ordering;
        zleactive.store(1, Ordering::SeqCst);
        INCOMPFUNC.store(0, Ordering::SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 1);
        // With incompfunc set → 0
        INCOMPFUNC.store(1, Ordering::SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 0);
        // Reset
        INCOMPFUNC.store(0, Ordering::SeqCst);
        zleactive.store(0, Ordering::SeqCst);
        assert_eq!(super::super::zle_thingy::zle_usable(), 0);
    }
}
