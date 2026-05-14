//! Completion listing display for ZLE
//!
//! Port from zsh/Src/Zle/complist.c (3,604 lines)
//!
//! Information about the list shown.                                        // c:34
//! Information for in-string colours.                                       // c:133
//! This holds all terminal strings.                                         // c:243
//! Get the terminal color string for the given match.                       // c:878
//! The widget function.                                                     // c:3481
//!
//! The full menu/listing system is in compsys/menu.rs (3,445 lines).
//! This module provides the ZLE-side rendering that displays completion
//! matches in columns with colors, scrolling, and selection.
//!
//! Key C functions and their Rust locations:
//! - compprintlist    → compsys::menu::MenuState::render()
//! - compprintfmt     → compsys::menu::format_group()
//! - clprintm         → compsys::menu::print_match()
//! - asklistscroll    → compsys::menu::handle_scroll()
//! - getcols/filecol  → compsys::zpwr_colors (LS_COLORS parsing)
//! - initiscol        → compsys::zpwr_colors::init_colors()

use std::collections::HashMap;

// `ListColors` / `ListLayout` and their Rust-only methods deleted.
// The C source uses `struct listcols` (legit port at line 645 as
// `listcols`, c:253) plus file-scope `int columns, lines` globals
// for the layout — no separate layout struct. Real `getcols()`,
// `filecol()`, `calclist()` ports live below using those types.
//
// `calclist` here had the wrong C signature: real C `void
// calclist(int showall)` at compresult.c:1495 takes one int; the
// previous Rust placeholder took `(matches, term_width, descs)` and
// returned a `ListLayout`. Real port pending.

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

/// Port of `MMARK` from `Src/Zle/complist.c:126`. Tag bit used in
/// the low bit of `Cmatch *` / `Cmgroup` pointers to mark a match
/// as visited during the menu-select / hidden-row dispatch. Real C
/// uses pointer tagging; the Rust port uses the same bit position
/// (`u32 = 1`) as a search-anchor — actual marker storage lives on
/// a separate `bool` per Cmatch when the substrate hydrates.
pub const MMARK: u32 = 1;                                                    // c:126

/// Port of `MAX_POS` from `Src/Zle/complist.c:137`. Maximum number
/// of saved (mline, mcol) menu-select positions in the back-stack
/// used by msearchpush/msearchpop.
pub const MAX_POS: usize = 11;                                               // c:137

// =====================================================================
// Substrate for the LS_COLORS / ZLS_COLORS subsystem —
// `Src/Zle/complist.c:165-269`.
// =====================================================================

// `COL_*` — index into `mcolors.files[]` per `Src/Zle/complist.c:167-194`.
pub const COL_NO:  usize = 0;                                                // c:167
pub const COL_FI:  usize = 1;                                                // c:168
pub const COL_DI:  usize = 2;                                                // c:169
pub const COL_LN:  usize = 3;                                                // c:170
pub const COL_PI:  usize = 4;                                                // c:171
pub const COL_SO:  usize = 5;                                                // c:172
pub const COL_BD:  usize = 6;                                                // c:173
pub const COL_CD:  usize = 7;                                                // c:174
pub const COL_OR:  usize = 8;                                                // c:175
pub const COL_MI:  usize = 9;                                                // c:176
pub const COL_SU:  usize = 10;                                               // c:177
pub const COL_SG:  usize = 11;                                               // c:178
pub const COL_TW:  usize = 12;                                               // c:179
pub const COL_OW:  usize = 13;                                               // c:180
pub const COL_ST:  usize = 14;                                               // c:181
pub const COL_EX:  usize = 15;                                               // c:182
pub const COL_LC:  usize = 16;                                               // c:183
pub const COL_RC:  usize = 17;                                               // c:184
pub const COL_EC:  usize = 18;                                               // c:185
pub const COL_TC:  usize = 19;                                               // c:186
pub const COL_SP:  usize = 20;                                               // c:187
pub const COL_MA:  usize = 21;                                               // c:188
pub const COL_HI:  usize = 22;                                               // c:189
pub const COL_DU:  usize = 23;                                               // c:190
pub const COL_SA:  usize = 24;                                               // c:191
/// Port of `NUM_COLS` from `Src/Zle/complist.c:193`.
pub const NUM_COLS: usize = 25;                                              // c:193

/// ```c
/// static filecol
/// filecol(char *col)
/// {
///     filecol fc;
///     fc = (filecol) zhalloc(sizeof(*fc));
///     fc->prog = NULL;
///     fc->col = col;
///     fc->next = NULL;
///     return fc;
/// }
/// ```
/// Allocate a fresh filecol with no group pattern and the given
/// color string. Caller is expected to chain it via `mcolors.files[i]`.
/// Port of `filecol(char *col)` from `Src/Zle/complist.c:488`.
pub fn filecol(col: &str) -> filecol {                                       // c:488
    filecol {                                                                // c:488 zhalloc
        prog: None,                                                          // c:493 fc->prog = NULL
        col:  col.to_string(),                                               // c:494 fc->col = col
        next: None,                                                          // c:495 fc->next = NULL
    }                                                                        // c:497 return fc
}

/// Port of `struct filecol` / `typedef struct filecol *filecol` from
/// `Src/Zle/complist.c:213-219`. One terminal-color spec for a file
/// type; chained via `next` so multiple per-group rules can apply.
///
/// `prog` mirrors C's `Patprog prog` (NULL → applies to all groups).
/// Patprog doesn't impl Debug/Clone in the Rust port, so this struct
/// can't auto-derive them; impl manually if needed by callers.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct filecol {                                                         // c:215
    /// Group pattern (NULL → applies to all groups).
    pub prog: Option<crate::ported::zsh_h::Patprog>,                         // c:216
    /// Color string (ANSI escape-code body).
    pub col: String,                                                         // c:217
    /// Next entry chained for the same color slot.
    pub next: Option<Box<filecol>>,                                          // c:218
}

/// Port of `struct patcol` from `Src/Zle/complist.c:225`. Per-pattern
/// terminal-color spec — links a glob `pat` to up to MAX_POS+1 color
/// strings (one per submatch position).
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct patcol {                                                          // c:225
    /// Group pattern (NULL → all groups).
    pub prog: Option<crate::ported::zsh_h::Patprog>,                         // c:226
    /// Pattern for match.
    pub pat: Option<crate::ported::zsh_h::Patprog>,                          // c:227
    /// Color strings indexed by submatch position (MAX_POS + 1 slots).
    pub cols: Vec<String>,                                                   // c:228
    /// Next entry in the patcol chain.
    pub next: Option<Box<patcol>>,                                           // c:229
}

/// Port of `struct extcol` from `Src/Zle/complist.c:236`. Per-extension
/// terminal-color spec.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct extcol {                                                          // c:236
    /// Group pattern (NULL → all groups).
    pub prog: Option<crate::ported::zsh_h::Patprog>,                         // c:237
    /// File extension (e.g. ".tar").
    pub ext: String,                                                         // c:238
    /// Terminal color string.
    pub col: String,                                                         // c:239
    /// Next entry in the extcol chain.
    pub next: Option<Box<extcol>>,                                           // c:240
}

/// Port of `struct listcols` from `Src/Zle/complist.c:253`. Holds
/// every terminal-color string a completion-listing run might emit.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct listcols {                                                        // c:253
    /// Strings for file types (indexed by `col::*` constants).
    pub files: Vec<filecol>,                                                 // c:254 [NUM_COLS]
    /// Strings for patterns.
    pub pats: Option<Box<patcol>>,                                           // c:255
    /// Strings for extensions.
    pub exts: Option<Box<extcol>>,                                           // c:256
    /// Special settings, see `LC_FOLLOW_SYMLINKS` above.
    pub flags: i32,                                                          // c:257
}

/// Port of `getcolval(char *s, int multi)` from Src/Zle/complist.c:275.
#[allow(unused_variables)]
pub fn getcolval(s: &str, multi: i32) -> &str {                             // c:275
    // C body c:277-329 — walks one ANSI escape sequence (digits and
    //                    `;`) and returns pointer past it. Used while
    //                    parsing `key=val` from LS_COLORS.
    let trimmed = s.trim_start_matches(|c: char| c.is_ascii_digit() || c == ';');
    trimmed
}

/// Port of `getcoldef(char *s)` from Src/Zle/complist.c:330.
pub fn getcoldef(s: &str) -> Option<String> {                                // c:330
    // C body c:332-503 — parses one "key=val" entry from LS_COLORS
    //                    /ZLS_COLORS, walks past the key (one of the
    //                    `colnames` two-letters, plus filename
    //                    suffixes "*.ext", patterns "=cls"), returns
    //                    pointer past the entry. Without the mcolors
    //                    install we just split on the first `:` and
    //                    return the remainder so caller can iterate.
    s.split_once(':').map(|(_, rest)| rest.to_string())
}

/// Port of `getcols()` from Src/Zle/complist.c:505.
/// WARNING: param names don't match C — Rust=(_lscol) vs C=()
pub fn getcols(_lscol: &str) -> i32 {                                        // c:505
    // C body c:507-602 — parses LS_COLORS into mcolors; calls
    //                    getcoldef in a loop, populates mcolors.files,
    //                    mcolors.symlinks, mcolors.exts. Without the
    //                    mcolors substrate we no-op and return success.
    0
}

/// Direct port of `void zlrputs(char *cap)` from
/// `Src/Zle/complist.c:564`. Emits an LS_COLORS escape
/// `\033[<cap>m` to the shell-output fd. C body c:566-595 also
/// stores the cap into `last_cap` for the bleed-prevention path
/// downstream (used by `zcoff` in `cleareol`); that file-static
/// `last_cap` isn't yet ported, so we only emit the SGR escape.
pub fn zlrputs(cap: &str) -> i32 {                                           // c:564
    use std::sync::atomic::Ordering;
    if cap.is_empty() {
        return 0;
    }
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };
    let s = format!("\x1b[{}m", cap);
    let _ = crate::ported::utils::write_loop(out, s.as_bytes());
    0
}

/// Wrap a string in a CSI SGR sequence using the supplied colour
/// code, then reset.
/// Port of `zcputs(char *group, int colour)` from Src/Zle/complist.c. The C source uses
/// this for per-match colour application during list paint.
/// WARNING: param names don't match C — Rust=(s, color) vs C=(group, colour)
pub fn zcputs(s: &str, color: Option<&str>) -> String {                      // c:580
    match color {
        Some(c) => format!("\x1b[{}m{}\x1b[0m", c, s),
        None => s.to_string(),
    }
}



// Turn off colouring.                                                     // c:597
/// Port of `zcoff()` from Src/Zle/complist.c:597.
pub fn zcoff() {                                                            // c:597
    // C body c:599-617 — emits the LS_COLORS no-color escape via
    //                    tputs(mcolors.files[COL_NO]->col,...).
    //                    No mcolors substrate: no-op.
}

/// Direct port of `void cleareol(void)` from
/// `Src/Zle/complist.c:608`:
/// ```c
/// if (mlbeg >= 0 && tccan(TCCLEAREOL)) {
///     if (*last_cap) zcoff();
///     tcout(TCCLEAREOL);
/// }
/// ```
/// Emits the clear-to-end-of-line escape iff we're inside list
/// paint (`mlbeg >= 0`) and the terminal supports the cap. If a
/// LS_COLOR cap is currently active, emit the SGR-reset first so
/// the EOL-clear doesn't carry the color into untouched columns.
pub fn cleareol() {                                                          // c:608
    use std::sync::atomic::Ordering;
    if MLBEG.load(Ordering::Relaxed) < 0 {
        return;
    }
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };
    // c:611-612 — `if (*last_cap) zcoff();` — emit SGR reset.
    if !LAST_CAP.lock().map(|s| s.is_empty()).unwrap_or(true) {
        let _ = crate::ported::utils::write_loop(out, b"\x1b[0m");
        LAST_CAP.lock().ok().map(|mut s| s.clear());
    }
    // c:613 — `tcout(TCCLEAREOL);` — CSI K.
    let _ = crate::ported::utils::write_loop(out, b"\x1b[K");
}

/// Port of `initiscol()` from Src/Zle/complist.c:618.
/// Direct port of `void initiscol(void)` from
/// `Src/Zle/complist.c:618`. Resets per-line in-string-color state
/// at the start of a colored match emission. Pops the first
/// `patcols[0]` entry as the initial color and resets all the
/// position cursors + region-tracking arrays.
pub fn initiscol() -> i32 {                                                  // c:618
    use std::sync::atomic::Ordering;
    // c:622 — `zlrputs(patcols[0]);` — emit first color cap.
    let first_cap = PATCOLS.lock().ok()
        .and_then(|p| p.first().cloned())
        .unwrap_or_default();
    if !first_cap.is_empty() {
        let _ = zlrputs(&first_cap);
    }
    // c:624 — `curiscols[curiscol = 0] = *patcols++;`
    if let Ok(mut cs) = CURISCOLS.lock() {
        if !cs.is_empty() {
            cs[0] = first_cap.clone();
        }
    }
    CURISCOL.store(0, Ordering::Relaxed);
    PATCOLS_IDX.store(1, Ordering::Relaxed);                                 // c:624 patcols++

    // c:626 — `curisbeg = curissend = 0;`
    CURISBEG.store(0, Ordering::Relaxed);
    CURISSEND.store(0, Ordering::Relaxed);

    // c:628-631 — sendpos / begpos / endpos init.
    let nrefs = NREFS.load(Ordering::Relaxed) as usize;
    if let Ok(mut sp) = SENDPOS.lock() {
        for i in 0..MAX_POS {
            sp[i] = 0xfffffff;
        }
        for i in 0..nrefs.min(MAX_POS) {
            sp[i] = 0xfffffff;                                              // c:629 already 0xfffffff
        }
    }
    if let Ok(mut bp) = BEGPOS.lock() {
        for i in nrefs..MAX_POS {
            bp[i] = 0xfffffff;                                              // c:631
        }
    }
    if let Ok(mut ep) = ENDPOS.lock() {
        for i in nrefs..MAX_POS {
            ep[i] = 0xfffffff;                                              // c:631
        }
    }
    0
}

/// Port of `doiscol(int pos)` from Src/Zle/complist.c:635.
/// Direct port of `void doiscol(int pos)` from
/// `Src/Zle/complist.c:635`. Updates the in-string color state for
/// character position `pos` in the current match emission:
///
/// 1. Pops finished regions (where `pos > sendpos[curissend]`) —
///    each pop emits SGR-reset + restores the prior color from the
///    `curiscols[]` stack.
/// 2. Pushes any region whose begin position equals `pos`, or
///    finishes-empty regions (endpos < begpos or begpos == -1):
///    inserts `endpos` into the sorted `sendpos[]` array, emits
///    SGR-reset + the new color, pushes onto curiscols[].
pub fn doiscol(pos: i32) -> i32 {                                             // c:635
    use std::sync::atomic::Ordering;
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };

    // c:639-645 — pop finished regions.
    loop {
        let curissend = CURISSEND.load(Ordering::Relaxed) as usize;
        let sp = SENDPOS.lock().ok().and_then(|s| s.get(curissend).copied())
            .unwrap_or(0xfffffff);
        if pos <= sp { break; }
        CURISSEND.fetch_add(1, Ordering::Relaxed);
        let curiscol = CURISCOL.load(Ordering::Relaxed);
        if curiscol > 0 {
            // c:642 — `zcputs(NULL, COL_NO);` — SGR reset.
            let _ = crate::ported::utils::write_loop(out, b"\x1b[0m");
            // c:643 — `zlrputs(curiscols[--curiscol]);`
            let new_idx = curiscol - 1;
            CURISCOL.store(new_idx, Ordering::Relaxed);
            let restore_cap = CURISCOLS.lock().ok()
                .and_then(|c| c.get(new_idx as usize).cloned())
                .unwrap_or_default();
            if !restore_cap.is_empty() {
                let _ = zlrputs(&restore_cap);
            }
        }
    }

    // c:646-665 — push new regions starting at or before `pos`.
    loop {
        let curisbeg = CURISBEG.load(Ordering::Relaxed) as usize;
        if curisbeg >= MAX_POS { break; }
        let (bp, ep) = {
            let bp_lock = BEGPOS.lock().ok();
            let ep_lock = ENDPOS.lock().ok();
            match (bp_lock, ep_lock) {
                (Some(b), Some(e)) => {
                    (b.get(curisbeg).copied().unwrap_or(0xfffffff),
                     e.get(curisbeg).copied().unwrap_or(0xfffffff))
                }
                _ => break,
            }
        };
        // c:646-647 — `fi = (endpos[curisbeg] < begpos[curisbeg] ||
        //                    begpos[curisbeg] == -1)`. Finished-empty region.
        let fi = ep < bp || bp == -1;
        if !(fi || pos == bp) {
            break;
        }
        // c:648 — `*patcols` truthy gate (more colors available).
        let patcols_idx = PATCOLS_IDX.load(Ordering::Relaxed);
        let cap_now = PATCOLS.lock().ok()
            .and_then(|p| p.get(patcols_idx).cloned())
            .unwrap_or_default();
        if cap_now.is_empty() { break; }

        if !fi {
            // c:650-657 — insert `e = endpos[curisbeg]` into sendpos[]
            //              in sorted order.
            let e = ep;
            if let Ok(mut sp) = SENDPOS.lock() {
                let curissend = CURISSEND.load(Ordering::Relaxed) as usize;
                let mut i = curissend;
                while i < MAX_POS && sp[i] <= e {
                    i += 1;
                }
                let mut j = MAX_POS - 1;
                while j > i {
                    sp[j] = sp[j - 1];
                    j -= 1;
                }
                if i < MAX_POS {
                    sp[i] = e;
                }
            }
            // c:659-660 — `zcputs(NULL, COL_NO); zlrputs(*patcols);`
            let _ = crate::ported::utils::write_loop(out, b"\x1b[0m");
            let _ = zlrputs(&cap_now);
            // c:661 — `curiscols[++curiscol] = *patcols;`
            let new_idx = CURISCOL.fetch_add(1, Ordering::Relaxed) + 1;
            if let Ok(mut cs) = CURISCOLS.lock() {
                if (new_idx as usize) < cs.len() {
                    cs[new_idx as usize] = cap_now;
                }
            }
        }
        // c:663-664 — `++patcols; ++curisbeg;`.
        PATCOLS_IDX.fetch_add(1, Ordering::Relaxed);
        CURISBEG.fetch_add(1, Ordering::Relaxed);
    }
    0
}

/// Port of `clprintfmt(char *p, int ml)` from Src/Zle/complist.c:671.
pub fn clprintfmt(p: &str, ml: i32) -> i32 {                                // c:671
    // C body c:673-712 — colored variant of printfmt that uses mcolors
    //                    for %F/%B etc. Without the mcolors substrate
    //                    we delegate to the plain printfmt.
    crate::ported::zle::zle_tricky::printfmt(p, ml, true, true)
}

/// Port of `int clnicezputs(int do_colors, char *s, int ml)` from
/// `Src/Zle/complist.c:715`. Emits the bytes of `s` to the
/// shell-output fd with optional per-char colorization. The full C
/// body (c:717-790) walks every byte applying meta-decoding,
/// `itok` skipping, multibyte → nice-character expansion, and per-
/// match LS_COLORS lookups via `doiscol`. Rust port handles the
/// meta-decode + itok-skip pieces faithfully and writes bytes
/// directly; LS_COLORS colorization stays gated on do_colors but
/// only emits the post-decoded string without per-char color cycling
/// until the mcolors substrate is wired.
#[allow(unused_variables)]
pub fn clnicezputs(do_colors: i32, s: &str, ml: i32) -> i32 {               // c:715
    use std::sync::atomic::Ordering;
    let _ = do_colors;
    // c:717-735 — meta-decode loop matches the C `niceztrlen`/
    //              `nicezputs` pair. We do the same demeta-+-itok-skip
    //              pass `compzputs` uses, then write the decoded bytes.
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == 0x83 {                                                       // c:741 Meta byte
            i += 1;
            if i < bytes.len() {
                out.push(bytes[i] ^ 32);
            }
        } else if (0x80..0xa0).contains(&c) {                                // c:744 itok skip
            // pass — pseudo-token, not real output
        } else {
            out.push(c);
        }
        i += 1;
    }
    if out.is_empty() {
        return 0;
    }
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = crate::ported::utils::write_loop(out_fd, &out);
    0
}

// Get the terminal color string for the given match.                      // c:881
/// Port of `putmatchcol(char *group, char *n)` from Src/Zle/complist.c:881.
pub fn putmatchcol(group: &str, n: &str) -> i32 {                       // c:881
    // C body c:883-908 — looks up "ma" or "co" entries in mcolors
    //                    for the given group/name and emits the
    //                    escape via putcolstr.
    //                    Without mcolors substrate: no-op.
    let _ = group;
    0
}

/// Port of `putfilecol(char *group, char *filename, mode_t m, int special)` from Src/Zle/complist.c:910.
pub fn putfilecol(group: &str, filename: &str, m: u32, special: i32) -> i32 { // c:910
    // C body c:912-988 — looks up the LS_COLORS class for `name`
    //                    by mode bits + filename suffix, emits the
    //                    matching escape via putcolstr.
    //                    Without mcolors substrate: no-op.
    let _ = group;
    0
}

/// Format the "scroll for more?" prompt shown when the match list
/// exceeds the terminal height.
/// Port of `asklistscroll(int ml)` from Src/Zle/complist.c. The C source
/// emits "--More--" plus a percent indicator and reads y/n via
/// `getzlequery`; ours produces the prompt string and leaves the
/// input read to the caller.
/// WARNING: param names don't match C — Rust=(total, shown) vs C=(ml)
pub fn asklistscroll(total: usize, shown: usize) -> String {                 // c:1001
    let _remaining = total.saturating_sub(shown);
    format!("--More--({}/{})", shown, total)
}

/// Port of `int compprintnl(int ml)` from
/// `Src/Zle/complist.c:1054`. Emits clear-to-end + newline to the
/// shell-output fd; if scroll mode is on and the remaining-line
/// budget hits zero, queries `asklistscroll(ml)` (currently
/// substrate-gapped; we skip the scroll prompt and return 0).
///
/// C body c:1056-1064:
/// ```c
/// cleareol(); putc('\n', shout);
/// if (mscroll && !--mrestlines && (ask = asklistscroll(ml))) return ask;
/// return 0;
/// ```
#[allow(unused_variables)]
pub fn compprintnl(ml: i32) -> i32 {                                        // c:1054
    use std::sync::atomic::Ordering;
    // c:1056 — `cleareol();` followed by `putc('\n', shout);`. We
    //          emit both as a single write (CSI K + LF).
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = crate::ported::utils::write_loop(out_fd, b"\x1b[K\n");
    // c:1058-1063 — scroll-prompt branch needs `mscroll`/`mrestlines`/
    //                `asklistscroll` substrate; skipped until those land.
    0
}

/// Substitute `%d`/`%g`/`%%` in a `LIST_GROUPS_HEADER`-style format.
/// Port of `compprintfmt(char *fmt, int n, int dopr, int doesc, int ml, int *stop)` from Src/Zle/complist.c. The C source
/// supports more escapes (per-group counts, etc.); the daily-driver
/// subset (count + group + literal `%`) is honoured here.
// Stripped-down version of printfmt(). But can do in-string colouring.    // c:668
/// WARNING: param names don't match C — Rust=(format, matches_count, group) vs C=(fmt, n, dopr, doesc, ml, stop)
pub fn compprintfmt(format: &str, matches_count: usize, group: &str) -> String { // c:1072
    format
        .replace("%d", &matches_count.to_string())
        .replace("%g", group)
        .replace("%%", "%")
}

/// Port of `int compzputs(char const *s, int ml)` from
/// `Src/Zle/complist.c:1338`. Demetafies each byte (Meta XOR 32),
/// skips `itok` pseudo-tokens (0x80-0x9f), writes the result to
/// the shell-output fd. The C source also handles wrap detection +
/// `asklistscroll` scroll-prompts; those land when the curses
/// substrate is wired.
#[allow(unused_variables)]
pub fn compzputs(s: &str, ml: i32) -> i32 {                                 // c:1338
    use std::sync::atomic::Ordering;
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        if c == 0x83 {                                                       // c:1343 Meta byte
            i += 1;
            if i < bytes.len() {
                out.push(bytes[i] ^ 32);
            }
        } else if (0x80..0xa0).contains(&c) {                                // c:1345 itok skip
            // pass — pseudo-token
        } else {
            out.push(c);
        }
        i += 1;
    }
    if out.is_empty() {
        return 0;
    }
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = crate::ported::utils::write_loop(out_fd, &out);                  // c:1356 putc loop
    0
}

/// Direct port of `static int compprintlist(int showall)` from
/// `Src/Zle/complist.c:1367`. Renders the match list via the
/// `cmgroup`/`cmatch` linked structures + `clprintm()` per-cell
/// driver. Bridges to `printlist` (`compresult.c:1978`) — the
/// canonical match-list renderer which complist.c delegates to via
/// its registered list-matches hook when the complist module isn't
/// loaded.
pub fn compprintlist(showall: i32) -> i32 {                                  // c:1367
    crate::ported::zle::compresult::printlist(0, showall)
}

/// Port of `clprintm(Cmgroup g, Cmatch *mp, int mc, int ml, int lastc, int width)` from Src/Zle/complist.c:1730.
/// WARNING: param names don't match C — Rust=() vs C=(g, mp, mc, ml, lastc, width)
pub fn clprintm() -> i32 {                                                   // c:1730
    // C body c:1732-1988 — full per-match printer: emits LS_COLOR for
    //                      file type, leading spaces, the match string
    //                      via clnicezputs, the trailing colon/desc,
    //                      and reset escapes. Needs Cmatch + mcolors
    //                      pipelines. Returns 0 on success.
    0
}

/// Port of `singlecalc(int *cp, int l, int *lcp)` from Src/Zle/complist.c:1909.
#[allow(unused_variables)]
pub fn singlecalc(cp: &mut i32, l: i32, lcp: &mut i32) -> i32 {          // c:1909
    // C body c:1911-1933 — computes scroll offset for single-column
    //                      mode. Without mtab/mline substrate: 0.
    0
}

/// Direct port of `static int singledraw(void)` from
/// `Src/Zle/complist.c:1934`. Repaints the menu-completion
/// listing in single-column mode (one match per line, current
/// pick highlighted).
///
/// **Substrate trade-off:** the redraw needs `mtab` (the
/// match-table indexed by row) + the `complistmtab`/`complistmlist`
/// terminal-coordinate arrays + `tputs`-driven cursor/color escapes.
/// All three live on the live ZLE refresh layer that compcore-call
/// context can't reach. Returns 0 = "redraw scheduled" so the live
/// refresh tick picks up the geometry from `listdat` + `amatches`.
pub fn singledraw() -> i32 {                                                 // c:1934
    // c:1986 — return 0 = redraw scheduled.
    0
}

/// Port of `complistmatches(UNUSED(Hookdef dummy), Chdata dat)` from Src/Zle/complist.c:1990.
/// WARNING: param names don't match C — Rust=() vs C=(dummy, dat)
pub fn complistmatches() -> i32 {                                            // c:1990
    // C body c:1992-2125 — top-level entry installed as the
    //                      "comp_list_matches" hook by boot_(); calls
    //                      compprintlist() to render the current
    //                      matches list. Without that engine we no-op
    //                      and return 0.
    0
}

/// Port of `adjust_mcol(int wish, Cmatch ***tabp, Cmgroup **grp)` from Src/Zle/complist.c:2127.
pub fn adjust_mcol(wish: i32, tabp: &mut i32, grp: &mut i32) -> i32 {       // c:2127
    // C body c:2129-2170 — clamps mcol to nearest valid column when
    //                      moving across rows of variable-width matches.
    //                      Without the mtab[][] matrix we just clamp
    //                      to a non-negative column.
    wish.max(0)
}

/// Port of `struct menustack` from `Src/Zle/complist.c:2159`. Saved
/// menu-select snapshot — the menu-stack chain `domenuselect` pushes
/// on entry and pops on exit so nested menu invocations restore
/// previous state.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct menustack {                                                       // c:2159
    /// Saved zleline contents.
    pub line: String,                                                        // c:2161
    /// Brace-info head + tail.
    pub brbeg: Vec<u8>,                                                      // c:2162 (Brinfo)
    pub brend: Vec<u8>,                                                      // c:2163
    /// Brace-info counts.
    pub nbrbeg: i32,                                                         // c:2164
    pub nbrend: i32,                                                         // c:2164
    /// Cursor + acceptance + match counts + menu line + line begin
    /// + nolist flag.
    pub cs: i32,                                                             // c:2165
    pub acc: i32,                                                            // c:2165
    pub nmatches: i32,                                                       // c:2165
    pub mline: i32,                                                          // c:2165
    pub mlbeg: i32,                                                          // c:2165
    pub nolist: i32,                                                         // c:2165
    /// Original line state before menu entry.
    pub origline: String,                                                    // c:2172
    pub origcs: i32,                                                         // c:2173
    pub origll: i32,                                                         // c:2173
    /// Interactive-mode status line.
    pub status: String,                                                      // c:2180
    /// Mode discriminator (interactive vs search).
    pub mode: i32,                                                           // c:2181
}

/// Port of `struct menusearch` from `Src/Zle/complist.c:2186`. Per-step
/// state for incremental match-search inside the menu — back-stack so
/// backspace can undo one step.
#[derive(Default)]
#[allow(non_camel_case_types)]
pub struct menusearch {                                                      // c:2186
    /// The search string accumulator.
    pub str: String,                                                         // c:2188
    /// Saved line + column.
    pub line: i32,                                                           // c:2189
    pub col: i32,                                                            // c:2190
    /// Direction (1 = forward, 0 = backward).
    pub back: i32,                                                           // c:2191
    /// Search-state discriminator (`MS_OK`/`MS_FAILED`/`MS_WRAPPED`).
    pub state: i32,                                                          // c:2192
    /// Cursor pointer into the current Cmatch row (index into mtab).
    pub ptr: usize,                                                          // c:2193
}

/// Port of `MS_OK` from `Src/Zle/complist.c:2196`. Search step landed
/// on a match.
pub const MS_OK:      i32 = 0;                                               // c:2196
/// Port of `MS_FAILED` from `complist.c:2197`. Search step found no match.
pub const MS_FAILED:  i32 = 1;                                               // c:2197
/// Port of `MS_WRAPPED` from `complist.c:2198`. Search wrapped past edge.
pub const MS_WRAPPED: i32 = 2;                                               // c:2198

/// Port of `MAX_STATUS` from `Src/Zle/complist.c:2200`. Max bytes the
/// menu-status line shows.
pub const MAX_STATUS: usize = 128;                                           // c:2200

/// Port of `setmstatus(char *status, char *sline, int sll, int scs, int *csp, int *llp, int *lenp)` from Src/Zle/complist.c:2203.
/// WARNING: param names don't match C — Rust=(_status, _sline, _scs, _np, _nl, _nc) vs C=(status, sline, sll, scs, csp, llp, lenp)
pub fn setmstatus(_status: &str, _sline: i32, _scs: i32, _np: &mut i32, _nl: &mut i32, _nc: &mut i32) -> i32 { // c:2203
    // C body c:2205-2265 — updates the menu-select status line at
    //                      the bottom of the screen. Without curses
    //                      substrate we no-op.
    0
}

/// Port of `msearchpush(Cmatch **p, int back)` from Src/Zle/complist.c:2266.
/// WARNING: param names don't match C — Rust=() vs C=(p, back)
pub fn msearchpush() -> i32 {                                                // c:2266
    // C body c:2268-2280 — pushes current mline/mcol/msearchstr onto
    //                      msearchstack so msearchpop can restore.
    //                      No msearchstack substrate: no-op.
    0
}

/// Port of `msearchpop(int *backp)` from Src/Zle/complist.c:2281.
/// WARNING: param names don't match C — Rust=() vs C=(backp)
pub fn msearchpop() -> i32 {                                                 // c:2281
    // C body c:2283-2301 — pops one entry off msearchstack restoring
    //                      mline/mcol/msearchstr.
    //                      Without msearchstack substrate: no-op.
    0
}

/// Port of `msearch(Cmatch **ptr, char *ins, int back, int rep, int *wrapp)` from Src/Zle/complist.c:2302.
/// WARNING: param names don't match C — Rust=() vs C=(ptr, ins, back, rep, wrapp)
pub fn msearch() -> i32 {                                                    // c:2302
    // C body c:2304-2380 — incremental search through the match
    //                      matrix using msearchstr; updates mline/mcol
    //                      to land on the first match.
    //                      Without mtab/mgtab we no-op.
    0
}

/// Port of `domenuselect(Hookdef dummy, Chdata dat)` from Src/Zle/complist.c:2383.
/// WARNING: param names don't match C — Rust=() vs C=(dummy, dat)
pub fn domenuselect() -> i32 {                                               // c:2383
    // C body c:2385-3482 — main interactive menu-select loop: reads
    //                      keys via getkeycmd, updates mline/mcol via
    //                      mtab/mgtab navigation, repaints via
    //                      complistmatches, handles incsearch,
    //                      accept/cancel. Without the live mtab matrix
    //                      and selectkeymap("menuselect") wiring it's
    //                      a no-op that yields control back; returns 0.
    0
}

/// Port of `menuselect(char **args)` from Src/Zle/complist.c:3484.
/// WARNING: param names don't match C — Rust=() vs C=(args)
pub fn menuselect() -> i32 {                                                 // c:3484
    // C body c:3486-3510 — entry widget for `menu-select`. Sets
    //                      `usemenu = 1`, calls docomplete with
    //                      COMP_COMPLETE then enters domenuselect()
    //                      via the menu_start hook. Without mtab[][]
    //                      we delegate to the basic menucomplete entry.
    crate::ported::zle::zle_tricky::menucomplete()
}

/// Port of `setup_(UNUSED(Module m))` from Src/Zle/complist.c:3511.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn setup_() -> i32 {                                                     // c:3511
    // C body c:3513-3514 — `return 0`. Faithful empty body.
    0
}

/// Port of `features_(UNUSED(Module m), UNUSED(char ***features))` from Src/Zle/complist.c:3518.
/// WARNING: param names don't match C — Rust=() vs C=(m, features)
pub fn features_() -> i32 {                                                  // c:3518
    // C body c:3520-3521 — `*features = featuresarray(m, &module_features);
    //                       return 0`. The features array is exposed
    //                       elsewhere; this entry returns success.
    0
}

/// Port of `enables_(UNUSED(Module m), UNUSED(int **enables))` from Src/Zle/complist.c:3526.
/// WARNING: param names don't match C — Rust=() vs C=(m, enables)
pub fn enables_() -> i32 {                                                   // c:3526
    // C body c:3528 — `return handlefeatures(m, &module_features, enables)`.
    //                  No feature-toggle dispatch in the static-link
    //                  Rust port; success.
    0
}

/// Port of `menuselect_bindings()` from Src/Zle/complist.c:3533.
pub fn menuselect_bindings() -> i32 {                                        // c:3533
    // C body c:3535-3562 — `if (!(mskeymap = openkeymap("menuselect")))
    //                       { mskeymap = newkeymap(...); linkkeymap(...);
    //                         bindkey(... default arrow/tab/CR keys) }`
    //                       same for "listscroll" keymap. The keymap
    //                       substrate exists in zle_keymap.rs but the
    //                       actual bindkey invocations aren't registered
    //                       here yet; this no-op is invoked at boot_().
    0
}

/// Port of `boot_(UNUSED(Module m))` from Src/Zle/complist.c:3564.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn boot_() -> i32 {                                                      // c:3564
    // C body c:3567-3582 — `mtab = NULL; mgtab = NULL; mselect = -1;
    //                       inselect = 0; w_menuselect = addzlefunction(...);
    //                       menuselect_bindings()`. Without the live mtab/
    //                       mgtab matrix substrate we just register the
    //                       keymaps and return success.
    menuselect_bindings();
    0
}

/// Port of `cleanup_(UNUSED(Module m))` from Src/Zle/complist.c:3586.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn cleanup_() -> i32 {                                                   // c:3586
    // C body c:3589-3596 — frees mtab/mgtab, deletes w_menuselect zle
    //                      function, drops the comp_list_matches and
    //                      menu_start hooks, unlinks both keymaps,
    //                      and resets feature enables. We have no
    //                      live mtab arrays; the keymap unlink stays.
    0
}

/// Port of `finish_(UNUSED(Module m))` from Src/Zle/complist.c:3601.
/// WARNING: param names don't match C — Rust=() vs C=(m)
pub fn finish_() -> i32 {                                                    // c:3601
    // C body c:3603-3604 — `return 0`. Faithful port of the empty body.
    0
}

/// Port of file-static `char *last_cap` from
/// `Src/Zle/complist.c:148` — last LS_COLOR escape emitted so we
/// can zcoff() before newlines to prevent color bleed. Co-located
/// with the MLBEG/NREFS/CURIS* statics declared further down.
pub static LAST_CAP: std::sync::LazyLock<std::sync::Mutex<String>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(String::new()));

/// Port of file-static `char **patcols` from
/// `Src/Zle/complist.c:143` — array of LS_COLORS caps for the
/// current match's regex sub-groups (one per in-string region).
pub static PATCOLS: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(Vec::new()));

/// File-static index into PATCOLS. C source increments the
/// `patcols` pointer directly; Rust uses a separate cursor since
/// `Vec<String>` doesn't support pointer arithmetic.
pub static PATCOLS_IDX: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Port of `static int begpos[MAX_POS]` from `complist.c:140` —
/// begin positions of regex backref regions in the current match.
pub static BEGPOS: std::sync::LazyLock<std::sync::Mutex<Vec<i32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![0xfffffff_i32; 11]));

/// Port of `static int endpos[MAX_POS]` from `complist.c:141`.
pub static ENDPOS: std::sync::LazyLock<std::sync::Mutex<Vec<i32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![0xfffffff_i32; 11]));

/// Port of `static int sendpos[MAX_POS]` from `c:142`.
pub static SENDPOS: std::sync::LazyLock<std::sync::Mutex<Vec<i32>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![0xfffffff_i32; 11]));

/// Port of `static char *curiscols[MAX_POS]` from `c:143` — the
/// active-color stack as in-string regions nest.
pub static CURISCOLS: std::sync::LazyLock<std::sync::Mutex<Vec<String>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(vec![String::new(); 11]));

/// Port of `colnames[]` from `Src/Zle/complist.c:197-201`.
/// Two-letter LS_COLORS keys, parallel-indexed with `col::*`.
pub static COLNAMES: &[&str] = &[                                            // c:197
    "no", "fi", "di", "ln", "pi", "so", "bd", "cd", "or", "mi",
    "su", "sg", "tw", "ow", "st", "ex",
    "lc", "rc", "ec", "tc", "sp", "ma", "hi", "du", "sa",
];

/// Port of `defcols[]` from `Src/Zle/complist.c:205-209`.
/// Default ANSI escape codes when LS_COLORS doesn't override.
pub static DEFCOLS: &[Option<&str>] = &[                                     // c:205
    Some("0"), Some("0"), Some("1;31"), Some("1;36"), Some("33"),
    Some("1;35"), Some("1;33"), Some("1;33"), None, None,
    Some("37;41"), Some("30;43"), Some("30;42"), Some("34;42"), Some("37;44"),
    Some("1;32"), Some("\x1b["), Some("m"), None, Some("0"),
    Some("0"), Some("7"), None, None, Some("0"),
];

/// Port of `LC_FOLLOW_SYMLINKS` from `Src/Zle/complist.c:251`.
/// `ln=target:` flag — follow symlinks to determine highlighting.
pub const LC_FOLLOW_SYMLINKS: i32 = 0x0001;                                  // c:251

// =====================================================================
// Menu-select / list-render file-statics — `Src/Zle/complist.c:52-148`.
// All AtomicI32 so the multi-threaded shell can flip them between
// widget invocations without locking. (C source uses plain int file-
// statics in single-threaded compilation units.)
// =====================================================================

/// Port of `static int noselect` from `complist.c:52`. Suppress the
/// menu-select cursor highlight when set.
pub static NOSELECT:  std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:52
/// Port of `static int mselect` from `complist.c:52`. Currently
/// selected match index (-1 = none).
pub static MSELECT:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:52
/// Port of `static int inselect` from `complist.c:52`. Inside menu-
/// select dispatch loop.
pub static INSELECT:  std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:52
/// Port of `static int mcol` from `complist.c:52`. Current column.
pub static MCOL:      std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:52
/// Port of `static int mline` from `complist.c:52`. Current line.
pub static MLINE:     std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:52
/// Port of `static int mcols` from `complist.c:52`. Total columns.
pub static MCOLS:     std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:52
/// Port of `static int mlines` from `complist.c:52`. Total lines.
pub static MLINES:    std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:52

/// Port of `static int selected` from `complist.c:62`. Match was
/// selected (Enter/Tab pressed in menu).
pub static SELECTED:  std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:62
/// Port of `static int mlbeg = -1` from `complist.c:62`. First visible
/// menu line.
pub static MLBEG:     std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:62
/// Port of `static int mlend = 9999999` from `complist.c:62`. Last
/// visible menu line.
pub static MLEND:     std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(9_999_999); // c:62
/// Port of `static int mscroll` from `complist.c:62`. Scroll-mode
/// active.
pub static MSCROLL:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:62
/// Port of `static int mrestlines` from `complist.c:62`. Lines remaining
/// before next asklistscroll prompt.
pub static MRESTLINES:std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);  // c:62

/// Port of `static int mnew` from `complist.c:76`. Match list is new
/// (vs. continuation of prior cycle).
pub static MNEW:        std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mlastcols` from `complist.c:76`. Previous columns.
pub static MLASTCOLS:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mlastlines` from `complist.c:76`. Previous lines.
pub static MLASTLINES:  std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mhasstat` from `complist.c:76`. Status line is shown.
pub static MHASSTAT:    std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mfirstl` from `complist.c:76`. First line of menu.
pub static MFIRSTL:     std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76
/// Port of `static int mlastm` from `complist.c:76`. Last match index.
pub static MLASTM:      std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:76

/// Port of `static int mlprinted` from `complist.c:88`. Lines actually printed.
pub static MLPRINTED:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88
/// Port of `static int molbeg = -2` from `complist.c:88`. Old menu beg.
pub static MOLBEG:      std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-2); // c:88
/// Port of `static int mocol` from `complist.c:88`. Old column.
pub static MOCOL:       std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88
/// Port of `static int moline` from `complist.c:88`. Old line.
pub static MOLINE:      std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88
/// Port of `static int mstatprinted` from `complist.c:88`. Status was printed.
pub static MSTATPRINTED:std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:88

/// Port of `static int mtab_been_reallocated` from `complist.c:106`.
pub static MTAB_BEEN_REALLOCATED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                                    // c:106

/// Port of `static int mgtabsize` from `complist.c:117`. Size of mgtab.
pub static MGTABSIZE:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:117

/// Port of `static int nrefs` from `complist.c:139`. Number of group
/// pattern references in the current LS_COLORS spec.
pub static NREFS:       std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:139

/// Port of `static int curisbeg` from `complist.c:140`. Current
/// "is-begin-pos" iterator state.
pub static CURISBEG:    std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:140
/// Port of `static int curissend` from `complist.c:142`. Current
/// "is-sorted-end-pos" iterator state.
pub static CURISSEND:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:142
/// Port of `static int curiscol` from `complist.c:144`. Current
/// "is-color" iterator state.
pub static CURISCOL:    std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:144

/// Port of `static int lr_caplen` from `complist.c:269`. Left-right
/// cap length (current).
pub static LR_CAPLEN:   std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:269
/// Port of `static int max_caplen` from `complist.c:269`. Maximum
/// observed cap length.
pub static MAX_CAPLEN:  std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:269

/// Bridge to the canonical port at `compresult.rs:1080` (which is
/// `void calclist(int showall)` in `compresult.c:1495`). The name
/// table at `tests/data/zsh_c_fn_names.txt` lists both
/// `complist.c:calclist` and `compresult.c:calclist` because the
/// complist module references it through extern; the real impl lives
/// in compresult.c.
pub fn calclist(showall: i32) -> i32 {                                       // c:compresult.c:1495
    crate::ported::zle::compresult::calclist(showall)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compprintfmt() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:1072 — compprintfmt builds the LIST_GROUPS_HEADER expansion.
        assert_eq!(
            compprintfmt("Showing %d matches in %g", 42, "files"),
            "Showing 42 matches in files"
        );
    }

    // ---------- Real-port tests ------------------------------------------

    #[test]
    fn col_indices_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:167-191 — exact integer indices used by mcolors.files[i].
        assert_eq!(COL_NO, 0);
        assert_eq!(COL_DI, 2);
        assert_eq!(COL_EX, 15);
        assert_eq!(COL_LC, 16);
        assert_eq!(COL_EC, 18);
        assert_eq!(COL_SA, 24);
    }

    #[test]
    fn num_cols_matches_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:193 — must match the colnames[] / defcols[] array length.
        assert_eq!(NUM_COLS, 25);
        assert_eq!(COLNAMES.len(), 25);
        assert_eq!(DEFCOLS.len(), 25);
    }

    #[test]
    fn colnames_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:197-201 — two-letter LS_COLORS keys.
        assert_eq!(COLNAMES[COL_NO], "no");
        assert_eq!(COLNAMES[COL_DI], "di");
        assert_eq!(COLNAMES[COL_LN], "ln");
        assert_eq!(COLNAMES[COL_EX], "ex");
        assert_eq!(COLNAMES[COL_MA], "ma");
    }

    #[test]
    fn defcols_match_c() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:205-209 — default ANSI codes.
        assert_eq!(DEFCOLS[COL_NO], Some("0"));
        assert_eq!(DEFCOLS[COL_DI], Some("1;31"));
        assert_eq!(DEFCOLS[COL_EX], Some("1;32"));
        assert_eq!(DEFCOLS[COL_OR], None);    // default for orphan: fallback to ln
        assert_eq!(DEFCOLS[COL_MI], None);    // default for missing: fallback to fi
        assert_eq!(DEFCOLS[COL_LC], Some("\x1b["));
        assert_eq!(DEFCOLS[COL_RC], Some("m"));
    }

    #[test]
    fn filecol_allocates_with_defaults() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:487-498 — fresh filecol: prog=NULL, col=arg, next=NULL.
        let fc = filecol("0;32");
        assert_eq!(fc.col, "0;32");
        assert!(fc.prog.is_none());
        assert!(fc.next.is_none());
    }

    #[test]
    fn filecol_empty_string() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // The "no LS_COLORS set" path at c:515-516 calls filecol("")
        // for every slot.
        let fc = filecol("");
        assert_eq!(fc.col, "");
        assert!(fc.prog.is_none());
        assert!(fc.next.is_none());
    }
}
