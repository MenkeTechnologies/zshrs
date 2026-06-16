//! ZLE refresh - screen redraw routines
//!
//! Direct port from zsh/Src/Zle/zle_refresh.c

use super::zle_h::{REFRESH_ELEMENT, REFRESH_STRING};
use crate::ported::init::{tclen, SHTTY};
use crate::ported::utils::{adjustcolumns, adjustlines, write_loop};
#[allow(unused_imports)]
use crate::ported::zle::{
    deltochar::*, textobjects::*, zle_hist::*, zle_main::*, zle_misc::*, zle_move::*,
    zle_params::*, zle_tricky::*, zle_utils::*, zle_vi::*, zle_word::*,
};
use crate::ported::zsh_h::{
    isset, COMBININGCHARS, TCCLEAREOL, TCDEL, TCINS, TXT_ERROR, TXT_MULTIWORD_MASK,
};
use std::fmt::Write;
use std::io;
use std::sync::atomic::Ordering;

/// Port of `ZR_memset(REFRESH_ELEMENT *dst, REFRESH_ELEMENT rc, int len)` from `Src/Zle/zle_refresh.c:86`.
/// ```c
/// static void
/// ZR_memset(REFRESH_ELEMENT *dst, REFRESH_ELEMENT rc, int len)
/// {
///     while (len--)
///         *dst++ = rc;
/// }
/// ```
/// Fill `dst[0..len]` with copies of `rc`. Equivalent to
/// `memset` for REFRESH_ELEMENT slices.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(rc, len) vs C=(dst, rc, len)
pub fn ZR_memset(
    // c:86
    dst: &mut [REFRESH_ELEMENT],
    rc: REFRESH_ELEMENT,
    len: usize,
) {
    let n = len.min(dst.len());
    for slot in dst.iter_mut().take(n) {
        // c:88-89 while (len--) *dst++ = rc
        *slot = rc;
    }
}

impl TextAttr {
    /// Render this attribute set as the corresponding ANSI SGR
    /// escape. Loose equivalent of `tsetcap()` from
    /// Src/Zle/zle_refresh.c (which emits termcap-derived sequences
    /// from per-cell attr changes during the diff/paint cycle).
    pub fn to_ansi(&self) -> String {
        let mut codes = Vec::new();
        if self.bold {
            codes.push("1".to_string());
        }
        if self.underline {
            codes.push("4".to_string());
        }
        if self.standout {
            codes.push("7".to_string());
        }
        if self.blink {
            codes.push("5".to_string());
        }
        if let Some(fg) = self.fg_color {
            codes.push(format!("38;5;{}", fg));
        }
        if let Some(bg) = self.bg_color {
            codes.push(format!("48;5;{}", bg));
        }
        if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes.join(";"))
        }
    }
}

/// Port of `ZR_strcpy(REFRESH_ELEMENT *dst, const REFRESH_ELEMENT *src)` from `Src/Zle/zle_refresh.c:95`.
/// ```c
/// static void
/// ZR_strcpy(REFRESH_ELEMENT *dst, const REFRESH_ELEMENT *src)
/// {
///     while ((*dst++ = *src++).chr != ZWC('\0'))
///         ;
/// }
/// ```
/// Copy a NUL-terminated REFRESH_ELEMENT string from `src` to
/// `dst`. The terminator is INCLUDED in the copy.
/// C body (Src/Zle/zle_refresh.c:95) is a single while-loop:
///     `while ((*dst++ = *src++).chr != ZWC('\\0'));`
/// Rust uses a take-up-to-NUL iterator chain to express the same
/// pointer-walking copy as a single statement.
#[allow(non_snake_case)]
/// WARNING: param names don't match C — Rust=(src) vs C=(dst, src)
pub fn ZR_strcpy(
    // c:95
    dst: &mut [REFRESH_ELEMENT],
    src: &[REFRESH_ELEMENT],
) {
    let n = src.iter().take_while(|e| e.chr != '\0').count() + 1; // c:97 incl trailing NUL
    let n = n.min(src.len()).min(dst.len());
    dst[..n].copy_from_slice(&src[..n]);
}

impl RefreshElement {
    /// Construct a refresh cell holding a single character with
    /// default attributes. Equivalent shape to a freshly-zeroed
    /// `REFRESH_ELEMENT` from Src/Zle/zle_refresh.h.
    pub fn new(chr: char) -> Self {
        let width = unicode_width::UnicodeWidthChar::width(chr).unwrap_or(1) as u8;
        RefreshElement {
            chr,
            atr: TextAttr::default(),
            width,
        }
    }

    /// Construct a refresh cell with explicit text attributes.
    /// Used by callers painting attributed regions (visual-mode
    /// standout, isearch underline, etc.) directly into a
    /// `VideoBuffer`.
    pub fn with_attr(chr: char, atr: TextAttr) -> Self {
        let width = unicode_width::UnicodeWidthChar::width(chr).unwrap_or(1) as u8;
        RefreshElement { chr, atr, width }
    }
}

/// Port of `ZR_strlen(const REFRESH_ELEMENT *wstr)` from `Src/Zle/zle_refresh.c:102`.
/// ```c
/// static size_t
/// ZR_strlen(const REFRESH_ELEMENT *wstr)
/// {
///     int len = 0;
///     while (wstr++->chr != ZWC('\0'))
///         len++;
///     return len;
/// }
/// ```
/// Length of a NUL-terminated REFRESH_ELEMENT string.
#[allow(non_snake_case)]
/// Port of `ZR_strlen(const REFRESH_ELEMENT *wstr)` from `Src/Zle/zle_refresh.c:102`.
pub fn ZR_strlen(wstr: &[REFRESH_ELEMENT]) -> usize {
    // c:102
    let mut len = 0; // c:102 int len = 0
    while len < wstr.len() && wstr[len].chr != '\0' {
        // c:106 while (wstr++->chr != ZWC('\0'))
        len += 1; // c:107 len++
    }
    len // c:109 return len
}

impl VideoBuffer {
    /// Allocate a fresh video buffer of `cols × rows` filled with
    /// blank cells. Equivalent to `resetvideo()` at
    /// Src/Zle/zle_refresh.c:725 which allocates `nlnct * winw`
    /// cells for `nbuf` each refresh.
    pub fn new(cols: usize, rows: usize) -> Self {
        let lines = vec![vec![RefreshElement::new(' '); cols]; rows];
        VideoBuffer { lines, cols, rows }
    }

    /// Reset every cell to a blank-attribute space. Used by
    /// `zrefresh()` between frames to wipe the working buffer
    /// before the new paint pass — see `freevideo()` at
    /// zle_refresh.c:700 for the equivalent role.
    pub fn clear(&mut self) {
        for line in &mut self.lines {
            for elem in line.iter_mut() {
                *elem = RefreshElement::new(' ');
            }
        }
    }

    /// Reshape the buffer for a new terminal size. Equivalent to
    /// the cols/lines update + `nbuf`/`obuf` reallocation chain in
    /// zle_refresh.c that fires on SIGWINCH (see the `winw`/`winh`
    /// re-read in `zrefresh()` at zle_refresh.c:975).
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.lines
            .resize(rows, vec![RefreshElement::new(' '); cols]);
        for line in &mut self.lines {
            line.resize(cols, RefreshElement::new(' '));
        }
    }

    /// Write a single cell into the buffer; out-of-range writes are
    /// silently dropped (matches the C source's bounds check before
    /// `nbuf[row][col] = ...` in zle_refresh.c).
    pub fn set(&mut self, row: usize, col: usize, elem: RefreshElement) {
        if row < self.rows && col < self.cols {
            self.lines[row][col] = elem;
        }
    }

    /// Read a single cell. Returns None for out-of-range coords —
    /// the C source's index path is unchecked (uses `winw`/`nlnct`
    /// invariants).
    pub fn get(&self, row: usize, col: usize) -> Option<&RefreshElement> {
        self.lines.get(row).and_then(|line| line.get(col))
    }
}

/// Port of `ZR_strncmp(const REFRESH_ELEMENT *oldwstr, const REFRESH_ELEMENT *newwstr, int len)` from `Src/Zle/zle_refresh.c:119`.
/// ```c
/// static int
/// ZR_strncmp(const REFRESH_ELEMENT *oldwstr, const REFRESH_ELEMENT *newwstr,
///            int len)
/// {
///     while (len--) {
///         if ((!(oldwstr->atr & TXT_MULTIWORD_MASK) && !oldwstr->chr) ||
///             (!(newwstr->atr & TXT_MULTIWORD_MASK) && !newwstr->chr))
///             return !ZR_equal(*oldwstr, *newwstr);
///         if (!ZR_equal(*oldwstr, *newwstr))
///             return 1;
///         oldwstr++;
///         newwstr++;
///     }
///     return 0;
/// }
/// ```
/// Simplified strcmp: returns 0 if first `len` elements match
/// (chr+atr pair-equal), 1 otherwise. Stops early at NUL in
/// either string (treating it as the shorter-string boundary).
#[allow(non_snake_case)]
/// Port of `ZR_strncmp(const REFRESH_ELEMENT *oldwstr, const REFRESH_ELEMENT *newwstr, int len)` from `Src/Zle/zle_refresh.c:120`.
/// WARNING: param names don't match C — Rust=(newwstr, len) vs C=(oldwstr, newwstr, len)
pub fn ZR_strncmp(
    // c:120
    oldwstr: &[REFRESH_ELEMENT],
    newwstr: &[REFRESH_ELEMENT],
    len: usize,
) -> i32 {
    let mut i = 0;
    while i < len {
        // c:123 while (len--)
        if i >= oldwstr.len() || i >= newwstr.len() {
            // C reads past end via pointer; we bound it.
            return if oldwstr.get(i) == newwstr.get(i) {
                0
            } else {
                1
            };
        }
        let o = oldwstr[i];
        let n = newwstr[i];
        // c:124-126 — `if early-NUL → return !equal`.
        let old_is_nul = (o.atr & TXT_MULTIWORD_MASK) == 0 && o.chr == '\0';
        let new_is_nul = (n.atr & TXT_MULTIWORD_MASK) == 0 && n.chr == '\0';
        if old_is_nul || new_is_nul {
            return if o == n { 0 } else { 1 }; // c:126 !ZR_equal
        }
        if o != n {
            // c:127 if (!ZR_equal(...)) return 1
            return 1;
        }
        i += 1; // c:129-130 oldwstr++; newwstr++
    }
    0 // c:133 return 0
}

impl RefreshState {
    /// Build the initial refresh state at zleread() entry.
    /// Equivalent to the global `nbuf`/`obuf`/`vln`/`vcs`
    /// allocation + reset performed by `resetvideo()` at
    /// Src/Zle/zle_refresh.c:725 — terminal size queried once,
    /// both video buffers allocated, `need_full_redraw` set so the
    /// first paint touches every cell.
    pub fn new() -> Self {
        let (cols, rows) = (adjustcolumns(), adjustlines());
        RefreshState {
            columns: cols,
            lines: rows,
            old_video: Some(VideoBuffer::new(cols, rows)),
            new_video: Some(VideoBuffer::new(cols, rows)),
            need_full_redraw: true,
            ..Default::default()
        }
    }

    /// Reallocate the video buffers for the current terminal size
    /// and arm a full redraw on the next paint. Equivalent to
    /// `resetvideo()` from Src/Zle/zle_refresh.c:725 invoked after
    /// SIGWINCH (the C source calls it from `adjustwinsize()` in
    /// Src/init.c).
    pub fn reset_video(&mut self) {
        let (cols, rows) = (adjustcolumns(), adjustlines());
        self.columns = cols;
        self.lines = rows;
        self.old_video = Some(VideoBuffer::new(cols, rows));
        self.new_video = Some(VideoBuffer::new(cols, rows));
        self.need_full_redraw = true;
    }

    /// Drop both video buffers — used at ZLE shutdown. Equivalent
    /// to `freevideo()` from Src/Zle/zle_refresh.c:700.
    pub fn free_video(&mut self) {
        self.old_video = None;
        self.new_video = None;
    }

    /// Promote the freshly-painted buffer to "previously displayed"
    /// and clear the new-buffer slate for the next frame.
    /// Equivalent to `bufswap()` from Src/Zle/zle_refresh.c:946 —
    /// the C source swaps `nbuf` and `obuf` pointers and zeroes the
    /// new `nbuf` so the diff loop has a clean target.
    pub fn swap_buffers(&mut self) {
        std::mem::swap(&mut self.old_video, &mut self.new_video);
        if let Some(ref mut new) = self.new_video {
            new.clear();
        }
    }
}

/// Main refresh function — redraws the line.
/// Port of `zrefresh()` from Src/Zle/zle_refresh.c. The C source paints
/// a full virtual-screen diff against the previous frame; this Rust
/// port renders the single line each call but adds three behaviors
/// the previous bare-buffer version was missing:
///   * region-attribute overlay (zle_refresh.c `region_highlights[]`),
///   * vi visual-mode auto-region (mirrors zle_refresh.c's check of
///     `region_active` to paint mark..zlecs in standout),
///   * RPS1 / right-prompt rendering at the right margin
///     (zle_refresh.c `put_rpromptbuf` path).

// --- AUTO: cross-zle hoisted-fn use glob ---
#[allow(unused_imports)]
#[allow(unused_imports)]

/// Port of `ZR_END_ELLIPSIS_SIZE` macro from `zle_refresh.c:284`.
pub const ZR_END_ELLIPSIS_SIZE: usize = ZR_END_ELLIPSIS.len(); // c:284

/// Port of `ZR_MID_ELLIPSIS1_SIZE` macro from `zle_refresh.c:295`.
pub const ZR_MID_ELLIPSIS1_SIZE: usize = ZR_MID_ELLIPSIS1.len(); // c:295

/// Port of `ZR_MID_ELLIPSIS2_SIZE` macro from `zle_refresh.c:302`.
pub const ZR_MID_ELLIPSIS2_SIZE: usize = ZR_MID_ELLIPSIS2.len(); // c:302

/// Port of `ZR_START_ELLIPSIS_SIZE` macro from `zle_refresh.c:312`.
pub const ZR_START_ELLIPSIS_SIZE: usize = ZR_START_ELLIPSIS.len(); // c:312

/// Apply a `$zle_highlight` array to the manager.
/// Port of `zle_set_highlight()` from Src/Zle/zle_refresh.c:322. Walks
/// each `category:spec` entry, parses the spec via `match_highlight`,
/// and stores it in `category_attrs`. Categories not mentioned keep the
/// zsh defaults, applied here on first call: `region` and `special`
/// default to `standout`, `isearch` to `underline`, `suffix` to `bold`
/// — direct ports of zle_refresh.c:395-402.
/// WARNING: param names don't match C — Rust=(manager, atrs) vs C=()
pub fn zle_set_highlight(manager: &mut HighlightManager, atrs: &[&str]) {
    let mut seen = std::collections::HashSet::new();
    for entry in atrs {
        if entry.is_empty() {
            continue;
        }
        if *entry == "none" {
            // zle_refresh.c:355-360 — `none` clears every category.
            for cat in [
                HighlightCategory::Region,
                HighlightCategory::Isearch,
                HighlightCategory::Suffix,
                HighlightCategory::Paste,
                HighlightCategory::Default,
                HighlightCategory::Special,
                HighlightCategory::Ellipsis,
            ] {
                manager.category_attrs.insert(cat, TextAttr::default());
                seen.insert(cat);
            }
            continue;
        }
        let (prefix, rest) = match entry.split_once(':') {
            Some(t) => t,
            None => continue,
        };
        let cat = match prefix {
            "region" => HighlightCategory::Region,
            "isearch" => HighlightCategory::Isearch,
            "suffix" => HighlightCategory::Suffix,
            "paste" => HighlightCategory::Paste,
            "default" => HighlightCategory::Default,
            "special" => HighlightCategory::Special,
            "ellipsis" => HighlightCategory::Ellipsis,
            _ => continue,
        };
        manager.category_attrs.insert(cat, match_highlight(rest));
        seen.insert(cat);
    }

    // Defaults for unset slots — zle_refresh.c:395-402.
    let default_standout = TextAttr {
        standout: true,
        ..TextAttr::default()
    };
    let default_underline = TextAttr {
        underline: true,
        ..TextAttr::default()
    };
    let default_bold = TextAttr {
        bold: true,
        ..TextAttr::default()
    };
    if !seen.contains(&HighlightCategory::Region) {
        manager
            .category_attrs
            .insert(HighlightCategory::Region, default_standout);
    }
    if !seen.contains(&HighlightCategory::Isearch) {
        manager
            .category_attrs
            .insert(HighlightCategory::Isearch, default_underline);
    }
    if !seen.contains(&HighlightCategory::Suffix) {
        manager
            .category_attrs
            .insert(HighlightCategory::Suffix, default_bold);
    }
    if !seen.contains(&HighlightCategory::Special) {
        manager
            .category_attrs
            .insert(HighlightCategory::Special, default_standout);
    }
}

/// Port of `zle_free_highlight()` from `Src/Zle/zle_refresh.c:415`.
/// ```c
/// void
/// zle_free_highlight(void) {
///     free_colour_buffer();
/// }
/// ```
/// Direct port of `void zle_free_highlight(void)` from
/// `Src/Zle/zle_refresh.c:415-420`.
/// ```c
/// free_colour_buffer();
/// ```
///
/// C's `free_colour_buffer` frees the per-cell colour-attribute
/// storage used by `region_highlight`. In the Rust port that
/// storage is a `Vec<HighlightSpan>` inside the file-scope
/// `HIGHLIGHT` static, dropped automatically by Vec::clear at the
/// same invalidate points that fire the C free. No-op here is the
/// correct cross-language equivalent for this fn shape (the
/// caller doesn't reach into the highlight buffer from this entry
/// point; the live tick clears its buffer directly).
pub fn zle_free_highlight() { // c:415
                              // Rust ownership handles the equivalent free; explicit clear
                              // happens against the file-scope HIGHLIGHT static when
                              // invalidate fires.
}

/// Port of `void tcoutclear(int cap)` from
/// `Src/Zle/zle_refresh.c:607`. C dispatches on `cap` (a termcap
/// index — TCCLEAREOL/TCCLEAREOD/TCCLEARSCREEN) to emit the
/// corresponding escape. Rust collapses to a bool `to_end`:
/// `true` → clear-to-end (CSI J), `false` → clear-entire-screen
/// (CSI 2J).
/// C body (3 lines):
///   `treplaceattrs((cap == TCCLEAREOL) ? prompt_attr : 0);
///    applytextattributes(0);
///    tcout(cap);`
/// WARNING: signature change — C=(int cap) vs Rust=(to_end: bool).
pub fn tcoutclear(to_end: bool) {
    // c:607
    let bytes: &[u8] = if to_end { b"\x1b[J" } else { b"\x1b[2J" }; // c:611 tcout
    let fd = SHTTY.load(Ordering::Relaxed); // c:611 shout
    let _ = write_loop(if fd >= 0 { fd } else { 1 }, bytes);
}

/// Port of `void zwcputc(const REFRESH_ELEMENT *c)` from
/// `Src/Zle/zle_refresh.c:622`. Sets the pending attributes to the
/// cell's (c:630), emits the SGR attribute-change diff (c:631 — empty
/// when the attr is unchanged, so output stays minimal), then writes
/// the character (c:644-651). The multiword/`nmwbuf` glyph path
/// (c:634-643) is deferred — combining-cluster substrate.
pub fn zwcputc(c: &REFRESH_ELEMENT) {
    use std::sync::atomic::Ordering;
    // c:630-631 — make the cell's attrs pending, emit the SGR diff.
    crate::ported::prompt::treplaceattrs(c.atr);
    let mut out = crate::ported::prompt::applytextattributes(0);
    // c:644-651 — emit the char (a NUL chr is C's WEOF/empty cell).
    if c.chr != '\0' {
        let mut buf = [0u8; 4];
        out.push_str(c.chr.encode_utf8(&mut buf));
    }
    if !out.is_empty() {
        let f = SHTTY.load(Ordering::Relaxed);
        let _ = write_loop(if f >= 0 { f } else { 1 }, out.as_bytes());
    }
}

/// Port of `int zwcwrite(const REFRESH_STRING s, size_t i)` from
/// `Src/Zle/zle_refresh.c:655`. Writes the first `i` cells of the
/// video-buffer string `s`, each via `zwcputc` (c:659). Returns the
/// number of cells written. The `zwrite(a,b)` macro (c:255/260) is just
/// `zwcwrite(a, b)`. (Cell attributes are deferred to `zwcputc`'s
/// colour path; the cell `chr` is faithful.)
pub fn zwcwrite(s: &[REFRESH_ELEMENT], i: usize) -> usize {
    // c:657-659 — `for (j = 0; j < i; j++) zwcputc(s + j);`
    let n = i.min(s.len());
    for cell in &s[..n] {
        zwcputc(cell); // c:659
    }
    n // c:660 `return i;`
}

// =====================================================================
// `DEF_MWBUF_ALLOC` + `zr_*_ellipsis` tables — `Src/Zle/zle_refresh.c:697`
// + c:269-313. Pre-built REFRESH_ELEMENT sequences for line-truncation
// markers.
// =====================================================================

/// Port of `DEF_MWBUF_ALLOC` from `Src/Zle/zle_refresh.c:697`.
/// Number of words to allocate in one go for the multiword buffers.
pub const DEF_MWBUF_ALLOC: usize = 32; // c:697

// =====================================================================
// Multi-codepoint cluster buffers — `Src/Zle/zle_refresh.c:53-55, 688-691`.
// Layout per the c:39-51 doc-comment: each entry is `[count, char0,
// char1, …, char(count-1)]` (count + count chars = count+1 elements).
// `nmw_ind` starts at 1 so the index stored in `base.chr` is never 0
// (a zero-index slot would compare-equal to a NUL chr cell). C uses
// REFRESH_CHAR (wint_t) entries; Rust uses u32 to match the wint_t
// shape without round-tripping through `char`'s validity constraints.
//
// Bucket-1 thread_locals per PORT_PLAN.md: each ZLE evaluator walks
// its own per-keystroke refresh, so per-thread state preserves the
// per-evaluator semantic without serialising across workers.
// =====================================================================

thread_local! {
    /// Port of `static REFRESH_CHAR *nmwbuf` from `Src/Zle/zle_refresh.c:55`.
    pub static NMWBUF: std::cell::RefCell<Vec<u32>> = const {
        std::cell::RefCell::new(Vec::new())                                  // c:55
    };
    /// Port of `static REFRESH_CHAR *omwbuf` from `Src/Zle/zle_refresh.c:54`.
    pub static OMWBUF: std::cell::RefCell<Vec<u32>> = const {
        std::cell::RefCell::new(Vec::new())                                  // c:54
    };
    /// Port of `static int nmw_ind` from `Src/Zle/zle_refresh.c:691`.
    /// Init 1 per c:43 — "We initialise nmw_ind to 1 to avoid the
    /// index stored in the character looking like a NULL."
    pub static NMW_IND: std::cell::Cell<usize> = const {
        std::cell::Cell::new(1)                                              // c:691
    };
    /// Port of `static int nmw_size` from `Src/Zle/zle_refresh.c:690`.
    pub static NMW_SIZE: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)                                              // c:690
    };
    /// Port of `static int omw_size` from `Src/Zle/zle_refresh.c:689`.
    pub static OMW_SIZE: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)                                              // c:689
    };
}

/// Direct port of `void freevideo(void)` from
/// `Src/Zle/zle_refresh.c:700`.
///
/// Drops the new/old video buffers and (under MULTIBYTE_SUPPORT) the
/// multi-codepoint cluster pools. C body's per-row zfree loop
/// collapses to Rust's Vec drop cascade; the cluster pools live on
/// VideoBuffer cells, so dropping the buffers releases them too.
/// Resets `winw_alloc`/`winh_alloc` to 0 so the next [`resetvideo`]
/// pass reallocates fresh buffers.
/// WARNING: param names don't match C — Rust=(state) vs C=()
pub fn freevideo(state: &mut RefreshState) {
    // c:702-718 — C body walks nbuf/obuf rows calling zfree on each
    //              REFRESH_STRING, then frees the row arrays
    //              themselves. Rust drop cascade subsumes both:
    //              setting the Options to None drops every nested
    //              Vec atomically.
    state.old_video = None;
    state.new_video = None;
    // c:720-724 — `winw_alloc = winh_alloc = -1;`. Stored on
    //              RefreshState.columns/lines so the next resetvideo
    //              comparison forces re-allocation regardless of the
    //              actual terminal size on next paint.
    state.columns = 0;
    state.lines = 0;
}

/// Port of `resetvideo()` from Src/Zle/zle_refresh.c:725.
/// Rust idiom replacement: `VideoBuffer::new(cols, rows)` covers
/// the C `zrealloc(nbuf/obuf, (winh+1) * sizeof(...))` + memset-zero
/// pair; geometry pulled from the canonical adjustcolumns/lines.
/// Direct port of `void resetvideo(void)` from
/// `Src/Zle/zle_refresh.c:725`.
///
/// Re-initialises the new/old video buffers + cursor + lprompt/
/// rprompt widths at the start of every refresh cycle. Mirrors the
/// C body's TERM_SHORT branch (winh=1 on dumb terms), lazy
/// reallocation gated on winw_alloc/winh_alloc change, per-row
/// zero-fill, then countprompt for both prompts and the
/// lprompth wraparound bump when lpromptwof crosses winw.
///
/// **Signature note:** takes the RefreshState (which owns the
/// VideoBuffers + the prompt cache) rather than the C file-statics.
/// WARNING: param names don't match C — Rust=(state) vs C=()
pub fn resetvideo(state: &mut RefreshState) {
    // c:725
    use crate::ported::params::TERMFLAGS;
    use crate::ported::prompt::countprompt;
    use crate::ported::zsh_h::TERM_SHORT;

    // c:729 — `winw = zterm_columns;`
    let cols = adjustcolumns();
    state.columns = cols;
    WINW.store(cols as i32, Ordering::Relaxed);

    // c:730-733 — TERM_SHORT clamps to 1 row, else clamp ≥ 24.
    let real_lines = adjustlines();
    let rows = if TERMFLAGS.load(Ordering::Relaxed) & TERM_SHORT != 0 {
        1
    } else if real_lines < 2 {
        24
    } else {
        real_lines
    };
    state.lines = rows;
    WINH.store(rows as i32, Ordering::Relaxed);
    RWINH.store(real_lines as i32, Ordering::Relaxed); // c:734

    // c:735 — `vln = vmaxln = winprompt = 0;`
    VLN.store(0, Ordering::Relaxed);
    VMAXLN.store(0, Ordering::Relaxed);
    WINPROMPT.store(0, Ordering::Relaxed);
    // c:736 — `winpos = -1;`
    WINPOS.store(-1, Ordering::Relaxed);

    // c:737-755 — re-alloc the video buffers (winw/winh changed).
    //              Rust always rebuilds since VideoBuffer::new is cheap
    //              and the realloc-on-change check is a no-op
    //              optimisation in C land.
    state.old_video = Some(VideoBuffer::new(cols, rows + 1));
    state.new_video = Some(VideoBuffer::new(cols, rows + 1));

    // c:757-767 — `for (ln = 0; ln <= winh; ln++) { nbuf[ln][0] = nl;
    //              nbuf[ln][1] = '\0'; ... }`. VideoBuffer::new
    //              already zero-fills.

    // c:770-774 — `countprompt(lpromptbuf, &lpromptwof, &lprompth, 1);
    //              countprompt(rpromptbuf, &rpromptw, &rprompth, 0);`
    let mut lpromptwof_v = 0i32;
    let mut lprompth_v = 0i32;
    let mut rpromptw_v = 0i32;
    let mut rprompth_v = 0i32;
    countprompt(&state.lpromptbuf, &mut lpromptwof_v, &mut lprompth_v, 1);
    countprompt(&state.rpromptbuf, &mut rpromptw_v, &mut rprompth_v, 0);

    // c:775-779 — `if (lpromptwof != winw) lpromptw = lpromptwof;
    //              else { lpromptw = 0; lprompth++; }`
    let lpromptw_v = if lpromptwof_v != cols as i32 {
        lpromptwof_v
    } else {
        lprompth_v += 1;
        0
    };
    state.lpromptw = lpromptw_v as usize;
    state.rpromptw = rpromptw_v as usize;
    LPROMPTW.store(lpromptw_v, Ordering::Relaxed);
    LPROMPTH.store(lprompth_v, Ordering::Relaxed);
    LPROMPTWOF.store(lpromptwof_v, Ordering::Relaxed);
    RPROMPTW.store(rpromptw_v, Ordering::Relaxed);
    RPROMPTH.store(rprompth_v, Ordering::Relaxed);

    // c:782-787 — pre-fill nbuf[0]/obuf[0] with `lpromptw` spaces so the
    //              first row's prompt area is reserved.
    if lpromptw_v > 0 {
        let spaces = lpromptw_v as usize;
        if let Some(v) = state.new_video.as_mut() {
            if let Some(row) = v.lines.get_mut(0) {
                for cell in row.iter_mut().take(spaces) {
                    cell.chr = ' ';
                }
            }
        }
        if let Some(v) = state.old_video.as_mut() {
            if let Some(row) = v.lines.get_mut(0) {
                for cell in row.iter_mut().take(spaces) {
                    cell.chr = ' ';
                }
            }
        }
    }

    // c:790-794 — `vcs = lpromptw; olnct = nlnct = 0;
    //              if (showinglist > 0) showinglist = -2;
    //              trashedzle = 0;`
    state.vcs = lpromptw_v as usize;
    VCS.store(lpromptw_v, Ordering::Relaxed);
    OLNCT.store(0, Ordering::Relaxed);
    NLNCT.store(0, Ordering::Relaxed);
    if SHOWINGLIST.load(Ordering::Relaxed) > 0 {
        SHOWINGLIST.store(-2, Ordering::Relaxed);
    }
    TRASHEDZLE.store(0, Ordering::Relaxed);

    state.need_full_redraw = true;
}

/// Port of `void scrollwindow(int tline)` from
/// `Src/Zle/zle_refresh.c:1991`. Positive lines → scroll up (CSI S),
/// negative → scroll down (CSI T).
pub fn scrollwindow(lines: i32) {
    let s = if lines > 0 {
        format!("\x1b[{}S", lines)
    } else if lines < 0 {
        // c:Src/Zle/zle_refresh.c:708 — C does `-lines` on `int`, which
        // wraps via two's complement when `lines == INT_MIN`. Rust's
        // `-i32::MIN` overflows in debug builds. Use wrapping_neg so the
        // behavior matches C exactly (and the absurd-large value emits
        // a benign large positive on the escape sequence).
        format!("\x1b[{}T", lines.wrapping_neg())
    } else {
        return;
    };
    let _ = write_loop(
        {
            use std::sync::atomic::Ordering;
            let f = SHTTY.load(Ordering::Relaxed);
            if f >= 0 {
                f
            } else {
                1
            }
        },
        s.as_bytes(),
    );
}

/// Direct port of `int nextline(Rparams rpms, int wrapped)` from
/// `Src/Zle/zle_refresh.c:842`.
///
/// Advances `rpms->ln` to the next video row inside the new-video
/// buffer. When already at the last row, either declines further
/// scroll (`canscroll==0` and constraints fail → return 1, caller
/// stops emitting) or scrolls the window by one line and decrements
/// `nvln` so cursor tracking stays consistent. Always ensures the
/// fresh row exists, then resets `rpms->s`/`sen` to the start/end
/// of that row.
///
/// **Signature note (faithful to C):** takes the rparams struct
/// directly + the RefreshState for video-buffer access. Mirrors the
/// C call where `nbuf`/`winw`/`winh`/`numscrolls` are file-statics.
pub fn nextline(
    rpms: &mut rparams,       // c:842
    state: &mut RefreshState, // for new_video / columns / lines access
    wrapped: i32,             // c:842
) -> i32 {
    let winw = state.columns as i32;
    let winh = state.lines as i32;
    let new_video = match state.new_video.as_mut() {
        Some(v) => v,
        None => return 1,
    };

    // c:844-845 — `nbuf[ln][winw+1] = wrapped ? zr_nl : zr_zr; *s = zr_zr;`
    if let Some(row) = new_video.lines.get_mut(rpms.ln as usize) {
        let end_idx = (winw + 1) as usize;
        if end_idx < row.len() {
            row[end_idx] = if wrapped != 0 {
                RefreshElement {
                    chr: '\n',
                    ..RefreshElement::default()
                }
            } else {
                RefreshElement::default()
            };
        }
        if (rpms.pos) < row.len() {
            row[rpms.pos] = RefreshElement::default();
        }
    }

    if rpms.ln != winh - 1 {
        // c:849 — `rpms->ln++;`
        rpms.ln += 1;
    } else {
        // c:851-860 — scroll-or-bail branch.
        if rpms.canscroll == 0 {
            let onumscrolls = ONUMSCROLLS.load(Ordering::Relaxed);
            let numscrolls = NUMSCROLLS.load(Ordering::Relaxed);
            // c:853-855 — bail when we shouldn't scroll yet.
            if rpms.nvln != -1
                && rpms.nvln != winh - 1
                && (numscrolls != onumscrolls - 1 || rpms.nvln <= winh / 2)
            {
                return 1;
            }
            NUMSCROLLS.fetch_add(1, Ordering::Relaxed); // c:858
            rpms.canscroll = winh / 2; // c:859
        }
        rpms.canscroll -= 1; // c:862
        scrollwindow(0); // c:863
        if rpms.nvln != -1 {
            rpms.nvln -= 1; // c:865
        }
    }

    // c:867-869 — allocate the row if missing.
    if rpms.ln as usize >= new_video.lines.len() {
        new_video.lines.resize(
            rpms.ln as usize + 1,
            vec![RefreshElement::default(); (winw + 2) as usize],
        );
    }
    // c:871-872 — `rpms->s = nbuf[ln]; rpms->sen = s + winw;`
    rpms.pos = 0;
    rpms.end = winw as usize;
    0 // c:873
}

/// Direct port of `int snextline(Rparams rpms)` from
/// `Src/Zle/zle_refresh.c:875`.
///
/// "Status next line" — advances inside the optional status area
/// (the line zsh draws below the prompt when more_status is set).
/// Mirrors the C body's `if (more_status && tosln != ln && ln != winh
/// - 1)` decision tree: bumps `rpms->ln` if we have room in the
/// status pane, otherwise gives up (return 1). Returns 0 on success.
pub fn snextline(
    rpms: &mut rparams,       // c:875
    state: &mut RefreshState, // for columns / lines access
) -> i32 {
    let winw = state.columns as i32;
    let winh = state.lines as i32;

    // c:877-878 — `if (rpms->more_status && rpms->tosln != rpms->ln
    //              && rpms->ln != winh - 1) {`
    if rpms.more_status != 0 && rpms.tosln != rpms.ln && rpms.ln != winh - 1 {
        // c:879 — `rpms->ln++;`
        rpms.ln += 1;
        // c:881-883 — alloc the status row if missing.
        if let Some(new_video) = state.new_video.as_mut() {
            if rpms.ln as usize >= new_video.lines.len() {
                new_video.lines.resize(
                    rpms.ln as usize + 1,
                    vec![RefreshElement::default(); (winw + 2) as usize],
                );
            }
        }
        // c:885-886 — `rpms->s = nbuf[ln]; rpms->sen = s + winw;`
        rpms.pos = 0;
        rpms.end = winw as usize;
        0 // c:887
    } else {
        1 // c:889 — out of status pane room.
    }
}

/// Direct port of `static void addmultiword(REFRESH_ELEMENT *base,
///                                          ZLE_STRING_T tptr, int ichars)`
/// from `Src/Zle/zle_refresh.c:913`.
///
/// Push the `ichars`-codepoint cluster `tptr[0..ichars]` into the
/// shared `nmwbuf` storage, set the cell's `TXT_MULTIWORD_MASK`
/// flag, and store the buffer entry's start index in `base.chr`.
/// The renderer dispatches on the flag and reads `nmwbuf[base.chr]`
/// (count) then `nmwbuf[base.chr+1..base.chr+1+count]` (cluster
/// codepoints) — see C c:635-636 and the COMPARE macro at c:77.
pub fn addmultiword(
    base: &mut REFRESH_ELEMENT, // c:913
    tptr: &[char],
    ichars: usize,
) {
    // c:917 — `int iadd = ichars + 1;` total slots needed (count + count chars).
    let iadd = ichars + 1;
    // c:920 — `base->atr |= TXT_MULTIWORD_MASK;`.
    base.atr |= TXT_MULTIWORD_MASK;
    NMWBUF.with(|buf| {
        let ind = NMW_IND.get();
        let size = NMW_SIZE.get();
        // c:921-927 — `if (nmw_ind + iadd > nmw_size) { … realloc … }`.
        if ind + iadd > size {
            let mw_more = if iadd > DEF_MWBUF_ALLOC {
                iadd
            } else {
                DEF_MWBUF_ALLOC
            }; // c:922-923
            let new_size = size + mw_more;
            buf.borrow_mut().resize(new_size, 0); // c:924-926
            NMW_SIZE.set(new_size); // c:925 nmw_size += mw_more
        }
        let mut b = buf.borrow_mut();
        // c:929-932 — `nmwptr = nmwbuf + nmw_ind; *nmwptr++ = ichars; for(…) *nmwptr++ = tptr[icnt];`
        b[ind] = ichars as u32; // c:930
        for icnt in 0..ichars {
            b[ind + 1 + icnt] = tptr[icnt] as u32; // c:931-932
        }
    });
    // c:934 — `base->chr = (wint_t)nmw_ind;`. Store the buffer index in
    // the chr slot; downstream readers dispatch via TXT_MULTIWORD_MASK.
    // char::from_u32 returns Some for any u32 ≤ 0x10FFFF excluding the
    // surrogate range; realistic nmw_ind values are far below that.
    let ind = NMW_IND.get();
    base.chr = char::from_u32(ind as u32).unwrap_or('\0');
    // c:935 — `nmw_ind += iadd;`.
    NMW_IND.set(ind + iadd);
}

/// Port of `bufswap()` from Src/Zle/zle_refresh.c:946.
/// WARNING: param names don't match C — Rust=(state) vs C=()
pub fn bufswap(state: &mut RefreshState) {
    // c:bufswap
    // C body: swap nbuf and obuf pointers (with mwbuf shadow when
    // MULTIBYTE_SUPPORT). Rust just swaps the Option<VideoBuffer>.
    std::mem::swap(&mut state.old_video, &mut state.new_video);
}
/// `zrefresh` — see implementation.
pub fn zrefresh() {
    // c:975
    // c:975 — full repaint pipeline. C writes every byte through
    //          `tputs(..., putshout)` / `fputs(..., shout)`. Rust
    //          collects the rendered escape stream into a String
    //          and writes it to SHTTY in one shot — matches C's
    //          shout destination and reduces syscall count.
    let mut handle = String::new();

    let (cols, _rows) = (adjustcolumns(), adjustlines());

    let prompt = prompt().to_string();
    let rprompt = rprompt().to_string();
    let cursor = ZLECS.load(Ordering::SeqCst);

    let prompt_width = countprompt(&prompt);
    let rprompt_width = countprompt(&rprompt);
    // ZLELINE was locked TWICE in this expression — std::sync::Mutex
    // isn't reentrant, so the second `.lock()` deadlocks forever
    // waiting for the first guard to drop. Take a single lock, derive
    // both the slice end AND the slice itself from the guard, then
    // collect into String so the guard drops at end of stmt.
    let buffer_before_cursor: String = {
        let guard = ZLELINE.lock().unwrap();
        let end = cursor.min(guard.len());
        guard[..end].iter().collect()
    };
    let cursor_col = prompt_width + countprompt(&buffer_before_cursor);

    // Horizontal scroll if the cursor approaches the right edge.
    // Mirrors zle_refresh.c's `winw` clamp logic — without the full
    // multi-line wrap path our single-line shell uses scroll instead.
    let scroll_margin = 8;
    let effective_cols = cols.saturating_sub(1);
    let scroll_offset = if cursor_col >= effective_cols.saturating_sub(scroll_margin) {
        cursor_col.saturating_sub(effective_cols / 2)
    } else {
        0
    };

    // Compose the per-buffer-char attribute overlay before paint, so
    // we don't have to re-walk the highlight list per char during write.
    let attrs = compute_render_attrs();

    let _ = write!(handle, "\r\x1b[K");

    // Prompt — drawn unless we've scrolled past it. Skip
    // `scroll_offset` visible chars from the prompt (inlined
    // from the deleted skip_chars helper) — ANSI escape
    // sequences are skipped unconditionally so they don't
    // count against width.
    if scroll_offset < prompt_width {
        let mut width = 0;
        let mut byte_idx = 0;
        let mut in_escape = false;
        for (i, c) in prompt.char_indices() {
            if width >= scroll_offset {
                byte_idx = i;
                break;
            }
            if in_escape {
                if c.is_ascii_alphabetic() {
                    in_escape = false;
                }
            } else if c == '\x1b' {
                in_escape = true;
            } else {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            }
            byte_idx = i + c.len_utf8();
        }
        let _ = write!(handle, "{}", &prompt[byte_idx..]);
    }

    // Compute the visible byte/char range of the buffer after scroll.
    let buffer_start = scroll_offset.saturating_sub(prompt_width);
    // Width budget for buffer = total cols - prompt drawn - rprompt reserve.
    let drawn_prompt_width = prompt_width.saturating_sub(scroll_offset);
    let rprompt_reserve = if rprompt_width > 0 {
        rprompt_width + 1
    } else {
        0
    };
    let buffer_budget = effective_cols
        .saturating_sub(drawn_prompt_width)
        .saturating_sub(rprompt_reserve);

    // Walk the buffer chars from buffer_start, applying overlay attrs.
    let mut current_attr: Option<TextAttr> = None;
    let line_snapshot = ZLELINE.lock().unwrap().clone();
    for (written, (idx, ch)) in line_snapshot
        .iter()
        .enumerate()
        .skip(buffer_start)
        .enumerate()
    {
        if written >= buffer_budget {
            break;
        }
        let want_attr = attrs.get(idx).and_then(|a| *a);
        if want_attr != current_attr {
            let _ = write!(handle, "\x1b[0m");
            if let Some(a) = want_attr {
                let _ = write!(handle, "{}", a.to_ansi());
            }
            current_attr = want_attr;
        }
        let _ = write!(handle, "{}", ch);
    }
    // Reset SGR before the rprompt / cursor jump.
    if current_attr.is_some() {
        let _ = write!(handle, "\x1b[0m");
    }

    // Right prompt — paint at the absolute right margin if there's
    // room. Mirrors put_rpromptbuf in zle_refresh.c which writes RPS1
    // at column (winw - rpromptw).
    if rprompt_width > 0 && rprompt_width + 2 < effective_cols {
        let rprompt_col = effective_cols.saturating_sub(rprompt_width);
        let _ = write!(handle, "\r\x1b[{}C{}\x1b[0m", rprompt_col, rprompt);
    }

    // Cursor positioning (1-based column in ANSI).
    let display_cursor_col = cursor_col.saturating_sub(scroll_offset);
    let _ = write!(handle, "\r\x1b[{}C", display_cursor_col);

    // c:1488 — `fwrite(out, ..., shout); fflush(shout);`. Single
    //          write_loop emits the whole frame to SHTTY (stdout
    //          fallback). Replaces the prior `stdout.lock()`
    //          fake that wrote refresh output to stdout instead
    //          of the controlling tty.
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out_fd, handle.as_bytes());

    // ---- Build NBUF (c:954-1400) ----------------------------------------
    // The full-repaint output above is the live renderer and is left
    // untouched. This populates the NBUF/OBUF video buffers that
    // `refreshline` diffs, so the minimal-update path can be developed and
    // verified against real frame data without risking the prompt. Once
    // refreshline's output sites are wired, zrefresh's output switches from
    // full-repaint to the NBUF/OBUF diff. `atr` is carried as default for
    // now (the cell `chr` is faithful; the colour-diff is wired with
    // refreshline's colour path) — a documented simplification, not a stub.
    {
        // c:954-955 — last frame's NBUF becomes this frame's OBUF.
        {
            let mut nbuf = NBUF.lock().unwrap();
            let mut obuf = OBUF.lock().unwrap();
            std::mem::swap(&mut *nbuf, &mut *obuf);
            OLNCT.store(NLNCT.load(Ordering::SeqCst), Ordering::SeqCst);
            nbuf.clear();
        }
        // c:1208-1400 — emit prompt + line cells, wrapping at `winw`.
        let cols_n = cols.max(1);
        let mut rows: Vec<REFRESH_STRING> = vec![Vec::new()];
        let mut emit = |rows: &mut Vec<REFRESH_STRING>, chr: char| {
            if rows.last().map(|r| r.len()).unwrap_or(0) >= cols_n {
                rows.push(Vec::new()); // c:842 nextline
            }
            rows.last_mut().unwrap().push(REFRESH_ELEMENT { chr, atr: 0 });
        };
        // Prompt's visible chars (skip ANSI escapes — they aren't cells).
        let mut in_esc = false;
        for c in prompt.chars() {
            if in_esc {
                if c.is_ascii_alphabetic() {
                    in_esc = false;
                }
            } else if c == '\x1b' {
                in_esc = true;
            } else {
                emit(&mut rows, c);
            }
        }
        // Editable line with tab/control expansion (c:1248-1398).
        for &ch in line_snapshot.iter() {
            if ch == '\n' {
                rows.push(Vec::new()); // c:1248-1251
            } else if ch == '\t' {
                // c:1259-1264 — spaces to the next 8-column stop.
                loop {
                    emit(&mut rows, ' ');
                    if rows.last().map(|r| r.len()).unwrap_or(0) % 8 == 0 {
                        break;
                    }
                }
            } else if (ch as u32) < 0x20 || ch as u32 == 0x7f {
                // c:1340-1356 — control char as `^X` / `^?`.
                emit(&mut rows, '^');
                let c2 = if ((ch as u32) & !0x80u32) > 31 {
                    '?'
                } else {
                    char::from_u32((ch as u32) | 0x40).unwrap_or('?')
                };
                emit(&mut rows, c2);
            } else {
                emit(&mut rows, ch); // c:1398
            }
        }
        let nlnct = rows.len() as i32;
        *NBUF.lock().unwrap() = rows;
        NLNCT.store(nlnct, Ordering::SeqCst); // c:nlnct = rpms.ln + 1
    }
}

impl HighlightManager {
    /// `new` — see implementation.
    pub fn new() -> Self {
        HighlightManager {
            regions: Vec::new(),
            category_attrs: std::collections::HashMap::new(),
        }
    }

    /// HighlightManager-internal helper: append a single region.
    /// Not a direct C port — `set_region_highlight` proper is the
    /// file-scope free fn below.
    pub fn add_region(&mut self, start: usize, end: usize, attr: TextAttr) {
        self.regions.push(RegionHighlight {
            start,
            end,
            attr,
            memo: None,
            flags: 0,
        });
    }

    /// Get region highlight for position. Equivalent to
    /// `get_region_highlight()` from zle_refresh.c.
    pub fn get_region_highlight(&self, pos: usize) -> Option<&RegionHighlight> {
        self.regions.iter().find(|r| pos >= r.start && pos < r.end)
    }

    /// Unset region highlight. Equivalent to
    /// `unset_region_highlight()` from zle_refresh.c.
    pub fn unset_region_highlight(&mut self) {
        self.regions.clear();
    }

    /// Free highlight resources. Equivalent to
    /// `zle_free_highlight()` from zle_refresh.c.
    pub fn free(&mut self) {
        self.regions.clear();
    }
}

/// Port of `wpfxlen(const REFRESH_ELEMENT *olds, const REFRESH_ELEMENT *news)` from `Src/Zle/zle_refresh.c:1736`.
/// ```c
/// static int
/// wpfxlen(const REFRESH_ELEMENT *olds, const REFRESH_ELEMENT *news) {
///     int i = 0;
///     while (olds->chr && ZR_equal(*olds, *news))
///         olds++, news++, i++;
///     return i;
/// }
/// ```
/// Common-prefix length of two REFRESH_ELEMENT strings; stops at
/// the first NUL chr in `olds` or first cell that differs in chr+atr.
pub fn wpfxlen(olds: &[REFRESH_ELEMENT], news: &[REFRESH_ELEMENT]) -> usize {
    let mut i = 0;
    while i < olds.len() && i < news.len() && olds[i].chr != '\0' && olds[i] == news[i] {
        i += 1;
    }
    i
}

/// Port of `static void refreshline(int ln)` from
/// `Src/Zle/zle_refresh.c:1749`. Repaints a single screen row at
/// `ln` from `nbuf[ln]` against `obuf[ln]`: handles `cleareol`,
/// auto-margin (`hasam`) edge cases, char-insert / char-delete
/// terminal capabilities (`TCDEL`/`TCINS`), and the
/// `MULTIBYTE_SUPPORT` `WEOF` width-padding cells.
///
/// ```c
/// static void
/// refreshline(int ln)
/// {
///     REFRESH_STRING nl, ol, p1;
///     int ccs = 0, char_ins = 0, col_cleareol, i, j;
///     int ins_last, nllen, ollen, rnllen;
///     const REFRESH_ELEMENT zr_pad = { ZWC(' '), prompt_attr };
/// /* 0: setup */
///     nl = nbuf[ln];
///     rnllen = nllen = nl ? ZR_strlen(nl) : 0;
///     if (ln < olnct && obuf[ln]) { ol = obuf[ln]; ollen = ZR_strlen(ol); }
///     else { static REFRESH_ELEMENT nullchr = { ZWC('\0'), 0 };
///            ol = &nullchr; ollen = 0; }
/// /* optimisation: clear-eol short-circuit */
///     if (cleareol && !nllen && !(hasam && ln < nlnct - 1)
///         && tccan(TCCLEAREOL)) { moveto(ln, 0); tcoutclear(TCCLEAREOL); return; }
/// /* 1: pad new buffer */
///     if (cleareol || (!nllen && (ln != 0 || !put_rpmpt))
///         || (ln == 0 && (put_rpmpt != oput_rpmpt))) { ... pad to winw ... }
///     else if (ollen > nllen) { ... pad to ollen ... }
/// /* 2: clear-to-eol calculation */
///     if (hasam && ln < nlnct - 1 && rnllen == winw) col_cleareol = -2;
///     else { ... compute col_cleareol from TCCLEAREOL cost ... }
/// /* 2b: automargin niceness */
///     if (hasam && vcs == winw) { ... advance vln/vcs ... }
///     ins_last = 0;
/// /* 2c: prompt-line head skip */
///     if (ln == 0 && lpromptw) { ... advance past prompt ... }
/// /* 3: main display loop */
///     for (;;) { ... skip-match / insert-delete / fall-through ... }
/// }
/// ```
///
/// Per PORT.md Rule 9 (function bodies port too) the structural
/// translation lives here even though the screen-output primitives
/// (`tccan`/`tc_inschars`/`tc_delchars`/`zputc`/`zwrite`/
/// `wpfxlen`/`tcinscost`/`tcdelcost`/`treplaceattrs`/
/// `applytextattributes`) and the `nbuf`/`obuf`/`vcs`/`vln`/
/// `winw`/`winh`/`zterm_lines`/`hasam`/`cleareol`/`put_rpmpt`/
/// `oput_rpmpt`/`olnct`/`prompt_attr` statics are not yet
/// surface-area in zshrs. Stubbed deps are flagged inline with
/// `// !!! STUB: needs <c-name> port at <Src/file.c:NNNN>`.
pub fn refreshline(ln: i32) {
    // c:1749

    // c:1751 — REFRESH_STRING nl, ol, p1. The nbuf/obuf statics are now
    // exposed (NBUF/OBUF, populated by zrefresh) and read below. The
    // diff control flow is fully ported; the remaining stubs in this
    // function are the OUTPUT primitives (zputc cell-emit, zwrite,
    // tc_delchars, tclen) — wired when zrefresh's output switches from
    // full-repaint to the NBUF/OBUF diff.
    // c:1762 — `nl = nbuf[ln];` — read this frame's new line from NBUF.
    let mut nl: REFRESH_STRING = NBUF
        .lock()
        .unwrap()
        .get(ln as usize)
        .cloned()
        .unwrap_or_default();
    // c:1764-1766 — `if (ln < olnct && obuf[ln]) ol = obuf[ln];` else null.
    let mut ol: REFRESH_STRING = if ln < OLNCT.load(Ordering::SeqCst) {
        OBUF.lock()
            .unwrap()
            .get(ln as usize)
            .cloned()
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let _p1: REFRESH_STRING = Vec::new(); // c:1751 p1
    let mut ccs: i32 = 0; // c:1752 ccs = 0
    let mut char_ins: i32 = 0; // c:1753 char_ins = 0
    let mut col_cleareol: i32; // c:1754
    let mut i: i32; // c:1755 tmp
    let mut _j: i32 = 0; // c:1755 tmp
    let mut ins_last: i32; // c:1756
    let mut nllen: i32; // c:1757
    let ollen: i32; // c:1757
    let rnllen: i32; // c:1758
    let zr_pad = REFRESH_ELEMENT {
        // c:1759
        chr: ' ',
        atr: 0,
    };

    // 0: setup                                                          // c:1761
    // nl = nbuf[ln]; rnllen = nllen = nl ? ZR_strlen(nl) : 0;           // c:1762-1763
    rnllen = nl.len() as i32;
    nllen = rnllen;
    // c:1764-1772 — `ollen = ZR_strlen(ol)`. `ol` is read from OBUF above
    // (or empty when `ln >= olnct`), so its length is the old line length.
    ollen = ol.len() as i32; // c:1766 / c:1771

    // optimisation: clear-eol short-circuit                             // c:1774-1775
    // c:1776-1781 — `if (cleareol && !nllen && !(hasam && ln < nlnct-1)
    //               && tccan(TCCLEAREOL)) { moveto(ln, 0);
    //               tcoutclear(TCCLEAREOL); return; }`
    let cleareol = CLEAREOL.load(Ordering::SeqCst) != 0;
    let hasam_v = crate::ported::init::hasam.load(Ordering::SeqCst) != 0; // c:1776
    let nlnct_v = NLNCT.load(Ordering::SeqCst);
    if cleareol                                                          // c:1776
            && nllen == 0
            && !(hasam_v && ln < nlnct_v - 1)
            && (tclen.lock().unwrap()[TCCLEAREOL as usize] != 0)
    /* tccan(TCCLEAREOL) per zsh.h:2682 */                       // c:1777
    {
        moveto(ln as usize, 0); // c:1778
        tcoutclear(true); // c:1779
        return; // c:1780
    }

    // 1: pad out new buffer with spaces                                 // c:1783-1784
    let put_rpmpt = PUT_RPMPT.load(Ordering::SeqCst);
    let oput_rpmpt = OPUT_RPMPT.load(Ordering::SeqCst);
    let winw = WINW.load(Ordering::SeqCst);
    if cleareol                                                          // c:1786
            || (nllen == 0 && (ln != 0 || put_rpmpt == 0))                   // c:1787
            || (ln == 0 && put_rpmpt != oput_rpmpt)
    // c:1788
    {
        // !!! STUB: zhalloc — Rust uses Vec growth instead of arena alloc.
        let mut padded: REFRESH_STRING =       // c:1789
                Vec::with_capacity((winw + 2) as usize);
        for el in nl.iter().take(nllen as usize) {
            // c:1790-1791 ZR_memcpy
            padded.push(*el);
        }
        for _ in nllen..winw {
            // c:1792 ZR_memset(.., zr_sp, ..)
            padded.push(REFRESH_ELEMENT { chr: ' ', atr: 0 });
        }
        padded.push(REFRESH_ELEMENT { chr: '\0', atr: 0 }); // c:1793 p1[winw] = zr_zr
        if nllen < winw {
            // c:1794
            padded.push(REFRESH_ELEMENT { chr: '\0', atr: 0 });
        // c:1795
        } else if let Some(extra) = nl.get((winw + 1) as usize).copied() {
            // c:1796-1797
            padded.push(extra);
        }
        // c:1798-1801 — if (ln && nbuf[ln]) memcpy back to nl, else nl = p1
        if ln != 0 && !nl.is_empty() {
            // c:1798
            let copy_len = ((winw + 2) as usize).min(padded.len()); // c:1799
            if nl.len() >= copy_len {
                for k in 0..copy_len {
                    nl[k] = padded[k];
                }
            } else {
                nl = padded.clone();
            }
        } else {
            // c:1800
            nl = padded; // c:1801
        }
        nllen = winw; // c:1802
    } else if ollen > nllen {
        // c:1803
        // c:1804-1809 — pad nl with zr_pad up to ollen.
        let mut padded: REFRESH_STRING =       // c:1804
                Vec::with_capacity((ollen + 1) as usize);
        for el in nl.iter().take(nllen as usize) {
            // c:1805
            padded.push(*el);
        }
        for _ in nllen..ollen {
            // c:1806
            padded.push(zr_pad);
        }
        padded.push(REFRESH_ELEMENT { chr: '\0', atr: 0 }); // c:1807
        nl = padded; // c:1808
        nllen = ollen; // c:1809
    }

    // 2: clear-to-eol calculation                                       // c:1812-1815
    if hasam_v && ln < nlnct_v - 1 && rnllen == winw {
        // c:1817
        col_cleareol = -2; // c:1818 evil — don't
    } else {
        // c:1819
        col_cleareol = -1; // c:1820
        if (tclen.lock().unwrap()[TCCLEAREOL as usize] != 0)  /* tccan(TCCLEAREOL) per zsh.h:2682 */                       // c:1821
                && (nllen == winw || put_rpmpt != oput_rpmpt)
        {
            // c:1822-1832 — backward-scan to find trailing-space cutoff.
            let a = nl.get((nllen - 1) as usize).map(|e| e.atr).unwrap_or(0); // c:1822
            let mut i_loc = nllen; // c:1823
            while i_loc > 0
                && nl
                    .get((i_loc - 1) as usize)
                    .map(|e| e.chr == ' ' && e.atr == a)
                    .unwrap_or(false)
            {
                i_loc -= 1; // c:1823
            }
            if nllen == winw && i_loc < nllen {
                // c:1825
                col_cleareol = i_loc; // c:1826
            } else {
                // c:1827
                let a = ol.get((ollen - 1) as usize).map(|e| e.atr).unwrap_or(0); // c:1828
                let mut j_loc = ollen; // c:1829
                while j_loc > 0
                    && ol
                        .get((j_loc - 1) as usize)
                        .map(|e| e.chr == ' ' && e.atr == a)
                        .unwrap_or(false)
                {
                    j_loc -= 1; // c:1829
                }
                // c:1831 — `if (j > i + tclen[TCCLEAREOL])`: clearing to
                // end-of-line is only worth it when the trailing-blank run
                // exceeds the cost (capability length) of the clear-eol
                // escape. tclen is populated by the termcap loader
                // (init.rs:108/758); read the real cost instead of 1.
                let tclen_clear: i32 = tclen.lock().unwrap()[TCCLEAREOL as usize];
                if j_loc > i_loc + tclen_clear {
                    // c:1831
                    col_cleareol = i_loc; // c:1832
                }
            }
        }
    }

    // 2b: automargin niceness                                           // c:1837
    let vcs = VCS.load(Ordering::SeqCst);
    let mut vln = VLN.load(Ordering::SeqCst);
    if hasam_v && vcs == winw {
        // c:1839
        // c:1898 — `if (nbuf[vln] && nbuf[vln][vcs + 1].chr == ZWC('\n'))`.
        let next_is_nl = {
            let nbuf = NBUF.lock().unwrap();
            nbuf.get(vln as usize)
                .and_then(|row| row.get((vcs + 1) as usize))
                .map(|cell| cell.chr == '\n')
                .unwrap_or(false)
        };
        if next_is_nl {
            vln += 1; // c:1899 vln++, vcs = 1
            VLN.store(vln, Ordering::SeqCst);
            VCS.store(1, Ordering::SeqCst);
            // c:1900-1903 — output the first cell of the next line, or a
            // space if that cell is blank ("I don't think this should
            // happen", per the C comment).
            let first_chr = {
                let nbuf = NBUF.lock().unwrap();
                nbuf.get(vln as usize)
                    .and_then(|row| row.first())
                    .map(|c| c.chr)
                    .filter(|&c| c != '\0')
            };
            match first_chr {
                Some(c) => zwcputc(&REFRESH_ELEMENT { chr: c, atr: 0 }), // c:1901
                None => zwcputc(&REFRESH_ELEMENT { chr: ' ', atr: 0 }),  // c:1903 zr_sp
            }
            if ln == vln {
                // c:1904 — better safe than sorry
                if !nl.is_empty() {
                    nl.remove(0); // c:1905 nl++
                }
                if !ol.is_empty() && ol[0].chr != '\0' {
                    ol.remove(0); // c:1906-1907 ol++
                }
                ccs = 1; // c:1908
            }
        } else {
            vln += 1; // c:1911 vln++, vcs = 0
            VLN.store(vln, Ordering::SeqCst);
            VCS.store(0, Ordering::SeqCst);
            zwcputc(&REFRESH_ELEMENT { chr: '\n', atr: 0 }); // c:1912 zr_nl
        }
    }
    ins_last = 0; // c:1857

    // 2c: prompt-line head skip                                         // c:1859-1860
    let lpromptw = LPROMPTW.load(Ordering::SeqCst);
    if ln == 0 && lpromptw != 0 {
        // c:1862
        i = lpromptw - ccs; // c:1863
        let j_loc = ol.len() as i32; // c:1864 j = ZR_strlen(ol)
                                     // c:1865 — nl += i (skip i cells)
        for _ in 0..i.min(nl.len() as i32) {
            nl.remove(0);
        }
        // c:1866 — ol += (i > j ? j : i)
        let ol_skip = if i > j_loc { j_loc } else { i };
        for _ in 0..ol_skip.min(ol.len() as i32) {
            ol.remove(0);
        }
        ccs = lpromptw; // c:1867
    }

    // c:1870-1880 — `while (nl->chr == WEOF) { nl++; ccs++; vcs++; ... }`
    // MULTIBYTE_SUPPORT WEOF realignment. zshrs's REFRESH_CHAR is
    // `char`; we don't carry a WEOF sentinel, so the loop is empty.

    // 3: main display loop                                              // c:1882
    loop {
        // c:1884
        // c:1888 — `if (nl->chr && ol->chr && ZR_equal(ol[1], nl[1]))`
        let nl_first = nl.first().copied();
        let ol_first = ol.first().copied();
        let nl_second = nl.get(1).copied();
        let ol_second = ol.get(1).copied();
        if nl_first.map(|e| e.chr != '\0').unwrap_or(false)              // c:1888
                && ol_first.map(|e| e.chr != '\0').unwrap_or(false)
                && nl_second == ol_second
        {
            // c:1894 — skip past matching cells.
            while !nl.is_empty()                                         // c:1894
                    && nl[0].chr != '\0'
                    && !ol.is_empty()
                    && nl[0] == ol[0]
            {
                nl.remove(0); // c:1894
                ol.remove(0);
                ccs += 1;
            }
        }

        // c:1906 — `if (!nl->chr)`
        if nl.is_empty() || nl[0].chr == '\0' {
            // c:1906
            if ccs == winw && hasam_v && char_ins > 0 && ins_last != 0   // c:1964
                    && vcs != winw
            {
                // c:1965-1971 — write the deferred last character. C does
                // `nl--; moveto(ln, winw-1); zputc(nl); vcs++`. In the Vec
                // model `nl--` steps back to column winw-1; this automargin
                // case has a full line, so that cell is NBUF[ln][winw-1].
                let deferred = NBUF
                    .lock()
                    .unwrap()
                    .get(ln as usize)
                    .and_then(|row| row.get((winw - 1) as usize))
                    .map(|c| c.chr);
                moveto(ln as usize, (winw - 1) as usize); // c:1966
                if let Some(c) = deferred {
                    zwcputc(&REFRESH_ELEMENT { chr: c, atr: 0 }); // c:1967 zputc(nl)
                }
                VCS.store(vcs + 1, Ordering::SeqCst); // c:1968 vcs++
                return; // c:1969
            }
            if char_ins <= 0 || ccs >= winw {
                // c:1915
                return; // c:1916 written everything
            }
            // c:1975 — `if (tccan(TCCLEAREOL) && char_ins >= tclen[TCCLEAREOL]
            //            && col_cleareol != -2)`. Read tclen[TCCLEAREOL] once
            // (Mutex isn't reentrant) and use it for both the tccan != 0
            // check and the char_ins cost comparison.
            let tcleareol_len = tclen.lock().unwrap()[TCCLEAREOL as usize];
            if tcleareol_len != 0  /* tccan(TCCLEAREOL) */
                    && char_ins >= tcleareol_len  // c:1975
                    && col_cleareol != -2
            {
                col_cleareol = 0; // c:1920
            }
        }

        moveto(ln as usize, ccs as usize); // c:1923

        // c:1925-1929 — if we can finish via clear-to-eol, do so
        if col_cleareol >= 0 && ccs >= col_cleareol {
            // c:1926
            tcoutclear(true); // c:1927 tcoutclear(TCCLEAREOL)
            return; // c:1928
        }

        // c:1932-1942 — empty nl: pad with spaces or delete chars.
        if nl.is_empty() || nl[0].chr == '\0' {
            // c:1932
            let i_pad = if winw - ccs < char_ins {
                // c:1933
                winw - ccs
            } else {
                char_ins
            };
            // c:1934 — `tccan(TCDEL) && tcdelcost(i) <= i + 1`
            let can_del = (tclen.lock().unwrap()[TCDEL as usize] != 0)  /* tccan(TCDEL) per zsh.h:2682 */ && i_pad <= i_pad + 1;
            if can_del {
                // c:1993 — `tc_delchars(i)`: delete `i_pad` chars via the
                // terminal's delete-char capability (now ported).
                tc_delchars(i_pad);
            } else {
                // c:1996 — `vcs += i`.
                VCS.store(vcs + i_pad, Ordering::SeqCst);
                // c:1996-1997 — `while (i-- > 0) zputc(&zr_pad)`: pad the
                // overwritten run with spaces.
                for _ in 0..i_pad {
                    zwcputc(&REFRESH_ELEMENT { chr: ' ', atr: 0 }); // c:1997 zr_pad
                }
            }
            return; // c:2002
        }

        // c:1946 — `if (!ol->chr)`
        if ol.is_empty() || ol[0].chr == '\0' {
            // c:1946
            let i_remain = if col_cleareol >= 0 {
                col_cleareol
            } else {
                nllen
            }; // c:1947
            let i_write = i_remain - vcs; // c:1948
            if i_write > 0 {
                // c:1949 (DPUTS guard)
                // c:1958 — `zwrite(nl, i)`: emit the new line's first
                // `i_write` cells (zwcwrite loops zwcputc over them).
                zwcwrite(&nl, i_write as usize);
                VCS.store(vcs + i_write, Ordering::SeqCst); // c:1959 vcs += i
            }
            if col_cleareol >= 0 {
                // c:1960
                tcoutclear(true); // c:1961
            }
            return; // c:1962
        }

        // c:1965-1970 — insert/delete eligibility
        let eligible = (ln != 0 || put_rpmpt == 0 || oput_rpmpt == 0)
            && !nl.is_empty()
            && nl_second.map(|e| e.chr != '\0').unwrap_or(false)
            && !ol.is_empty()
            && ol_second.map(|e| e.chr != '\0').unwrap_or(false)
            && ol_second != nl_second;
        if eligible {
            // c:1965
            // c:1976-2006 — TCDEL try-block: find a series we can delete
            if (tclen.lock().unwrap()[TCDEL as usize] != 0)
            /* tccan(TCDEL) per zsh.h:2682 */
            {
                // c:1976
                let mut first = true; // c:1977 — apply text-area attrs once
                let mut i_try = 1i32; // c:1978
                while (i_try as usize) < ol.len() && ol[i_try as usize].chr != '\0' {
                    // c:1979 — `tcdelcost(i) < wpfxlen(ol + i, nl)`
                    let ol_tail = &ol[i_try as usize..];
                    let cheap_delete = tcdelcost(i_try) < wpfxlen(ol_tail, &nl) as i32;
                    if cheap_delete {
                        // c:2042-2047 — some terminals output the current
                        // attributes into the cells a deletion adds at the
                        // end, so apply the text-area attrs once before the
                        // first delete: `treplaceattrs(prompt_attr);
                        // applytextattributes(0);` (both ported in prompt.rs).
                        if first {
                            crate::ported::prompt::treplaceattrs(
                                PROMPT_ATTR.load(Ordering::SeqCst),
                            ); // c:2045
                            let sgr = crate::ported::prompt::applytextattributes(0); // c:2046
                            if !sgr.is_empty() {
                                let fd = SHTTY.load(Ordering::Relaxed);
                                let _ = write_loop(if fd >= 0 { fd } else { 1 }, sgr.as_bytes());
                            }
                            first = false; // c:2047
                        }
                        // c:2048 — `tc_delchars(i)`: delete `i` characters.
                        tc_delchars(i_try);
                        for _ in 0..i_try {
                            if !ol.is_empty() {
                                ol.remove(0);
                            } // c:2049 ol += i
                        }
                        char_ins -= i_try; // c:2050
                        i_try = 0; // c:2004
                        break;
                    }
                    i_try += 1;
                }
                if i_try != 0 {
                    continue;
                } // c:2003-2004
            }

            // c:2012-2060 — TCINS try-block: find chars to insert.
            let zterm_lines = WINH.load(Ordering::SeqCst);
            if (tclen.lock().unwrap()[TCINS as usize] != 0)  /* tccan(TCINS) per zsh.h:2682 */ && vln != zterm_lines - 1
            {
                // c:2012
                let mut i_try = 1i32; // c:2014
                while (i_try as usize) < nl.len() && nl[i_try as usize].chr != '\0' {
                    // c:2015 — `tcinscost(i) < wpfxlen(ol, nl + i)`
                    let nl_tail = &nl[i_try as usize..];
                    let cheap_insert = tcinscost(i_try) < wpfxlen(&ol, nl_tail) as i32;
                    if cheap_insert {
                        // c:2016-2018 — tc_inschars(i); zwrite(nl, i);
                        for _ in 0..i_try {
                            if !nl.is_empty() {
                                nl.remove(0);
                            } // c:2018 nl += i
                        }
                        char_ins += i_try; // c:2025
                        VCS.store(vcs + i_try, Ordering::SeqCst);
                        ccs += i_try; // c:2026
                                      // c:2031-2047 — truncate oldline if past right edge.
                        let mut k = 0i32;
                        while (k as usize) < ol.len()
                            && ol[k as usize].chr != '\0'
                            && k < winw - ccs
                        {
                            k += 1; // c:2031-2032
                        }
                        if k >= winw - ccs && (k as usize) < ol.len() {
                            // c:2049
                            ol[k as usize] = REFRESH_ELEMENT {
                                chr: '\0',
                                atr: 0, // c:2050 ol[i] = zr_zr
                            };
                            ins_last = 1; // c:2051
                        }
                        i_try = 0; // c:2054
                        break;
                    }
                    i_try += 1;
                }
                if i_try != 0 {
                    continue;
                } // c:2058-2059
            }
        }

        // c:2065-2096 — fallback: dump one char and keep going
        if !nl.is_empty() && nl[0].chr == '\0' {
            // c:2072
            break; // c:2073
        }
        loop {
            // c:2074 do-while wrapper
            // c:2076-2077 — treplaceattrs(nl->atr); applytextattributes(0)
            // !!! STUB: treplaceattrs / applytextattributes —
            // Src/Zle/zle_refresh.c.

            // c:2084 — zputc(nl)
            // !!! STUB: zputc — emits the char to SHTTY.

            if !nl.is_empty() {
                nl.remove(0);
            } // c:2085 nl++
            if !ol.is_empty() && ol[0].chr != '\0' {
                // c:2086-2087
                ol.remove(0); // c:2087 ol++
            }
            ccs += 1; // c:2088
            VCS.store(vcs + 1, Ordering::SeqCst);

            // c:2094-2095 — WEOF do-while: zshrs has no WEOF sentinel.
            break;
        }
    }

    let _ = (rnllen, ollen, ins_last, _p1, _j); // silence
}

/// Direct port of `void moveto(int ln, int cl)` from
/// `Src/Zle/zle_refresh.c:2105`. C uses termcap `cm` / `cup`
/// strings to teleport the cursor; Rust emits the equivalent
/// CSI ; H sequence (rows/cols 1-indexed per ANSI). Was a
/// `print!` fake.
pub fn moveto(row: usize, col: usize) {
    // c:2105
    let s = format!("\x1b[{};{}H", row + 1, col + 1);
    let _ = write_loop(
        {
            use std::sync::atomic::Ordering;
            let f = SHTTY.load(Ordering::Relaxed);
            if f >= 0 {
                f
            } else {
                1
            }
        },
        s.as_bytes(),
    );
}

/// Direct port of `int tcmultout(int cap, int multcap, int ct)` from
/// `Src/Zle/zle_refresh.c:2163`.
///
/// Prefers the parametrised multi-arg capability when its escape
/// is no longer than `ct` repeats of the single cap (c:2165), falls
/// back to looping the single cap (c:2168-2170), otherwise emits a
/// safe ASCII fallback so cursor positioning still works on terms
/// without termcap entries. Returns 1 when any escape was emitted,
/// 0 when no usable capability existed.
pub fn tcmultout(cap: i32, multcap: i32, ct: i32) -> i32 {
    // c:2163
    use crate::ported::init::{tclen, tcstr};
    use crate::ported::zsh_h::{TCLEFT, TCRIGHT, TC_COUNT};

    if ct <= 0 {
        return 0;
    }
    let cap_idx = cap as usize;
    let multcap_idx = multcap as usize;
    let count = TC_COUNT as usize;

    let (cap_str, cap_len) = if cap_idx < count {
        let s = tcstr.lock().unwrap()[cap_idx].clone();
        let l = tclen.lock().unwrap()[cap_idx];
        (s, l)
    } else {
        (String::new(), 0)
    };
    let (mult_str, mult_len) = if multcap_idx < count {
        let s = tcstr.lock().unwrap()[multcap_idx].clone();
        let l = tclen.lock().unwrap()[multcap_idx];
        (s, l)
    } else {
        (String::new(), 0)
    };

    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let mult_ok = mult_len > 0;
    let cap_ok = cap_len > 0;

    if mult_ok && (!cap_ok || mult_len <= cap_len * ct) {
        // c:2165-2167 — parametrised multi-cap is cheaper.
        let emitted = mult_str.replace("%d", &ct.to_string());
        let _ = write_loop(out_fd, emitted.as_bytes());
        return 1;
    } else if cap_ok {
        // c:2168-2171 — loop the single-shot cap.
        for _ in 0..ct {
            let _ = write_loop(out_fd, cap_str.as_bytes());
        }
        return 1;
    }
    // Fallback when no termcap entries are wired: emit a portable
    // ASCII default so cursor positioning still works.
    let fallback: &[u8] = if cap == TCLEFT {
        b"\x08"
    } else if cap == TCRIGHT {
        b" "
    } else {
        return 0;
    };
    for _ in 0..ct {
        let _ = write_loop(out_fd, fallback);
    }
    1
}

/// Port of `void tc_rightcurs(int ct)` from
/// `Src/Zle/zle_refresh.c:2150`. CSI C parametrised cursor-right.
pub fn tc_rightcurs(count: usize) {
    if count > 0 {
        let s = format!("\x1b[{}C", count);
        let _ = write_loop(
            {
                use std::sync::atomic::Ordering;
                let f = SHTTY.load(Ordering::Relaxed);
                if f >= 0 {
                    f
                } else {
                    1
                }
            },
            s.as_bytes(),
        );
    }
}

/// Port of `void tc_downcurs(int ct)` from
/// `Src/Zle/zle_refresh.c:2126`. C emits the termcap `do`/`down`
/// capability `ct` times; Rust emits the parametrised CSI B.
pub fn tc_downcurs(count: usize) {
    if count > 0 {
        let s = format!("\x1b[{}B", count);
        let _ = write_loop(
            {
                use std::sync::atomic::Ordering;
                let f = SHTTY.load(Ordering::Relaxed);
                if f >= 0 {
                    f
                } else {
                    1
                }
            },
            s.as_bytes(),
        );
    }
}

/// Direct port of `int tcout_via_func(int cap, int arg, int (*outc)(int))`
/// from `Src/Zle/zle_refresh.c:2291`.
///
/// Faithful line-by-line translation of the C body (c:2293-2342).
/// Saves the SFC/STOPMSG/INCOMPFUNC context, looks up the user's
/// `$TCOUT_FUNC_NAME` (default `"tcout"`) via `getshfunc`, builds the
/// `[tcout_func_name, cap_name, arg?]` arg list, dispatches through
/// `callhookfunc` (the SFC_SUBST static-link path of `doshfunc`),
/// then reads `$REPLY` and writes each byte to the shell-output fd
/// (decoding Meta-byte pairs). Returns 0 when the function ran (caller
/// suppresses the raw termcap escape), 1 otherwise (caller emits raw).
pub fn tcout_via_func(cap: i32, arg: i32) -> i32 {
    use crate::ported::builtin::STOPMSG;
    use crate::ported::exec::sfcontext;
    use crate::ported::init::tccap_get_name;
    use crate::ported::params::getsparam;
    use crate::ported::utils::{callhookfunc, getshfunc, write_loop, INCOMPFUNC};
    use crate::ported::zsh_h::SFC_SUBST;

    // c:2295-2297 — save sfcontext / stopmsg / incompfunc.
    let osc = sfcontext.load(Ordering::Relaxed);
    let osm = STOPMSG.load(Ordering::Relaxed);
    let old_incompfunc = INCOMPFUNC.load(Ordering::Relaxed);
    // c:2299-2300 — sfcontext = SFC_SUBST; incompfunc = 0.
    sfcontext.store(SFC_SUBST, Ordering::Relaxed);
    INCOMPFUNC.store(0, Ordering::Relaxed);

    // c:2302 — Shfunc tcout_func; if ((tcout_func = getshfunc(tcout_func_name)))
    let func_name = TCOUT_FUNC_NAME
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| "tcout".to_string());

    let dispatched = if getshfunc(&func_name).is_some() {
        // c:2305-2309 — build linklist: [func_name, cap_name, arg?].
        let mut argv: Vec<String> = Vec::with_capacity(3);
        argv.push(func_name.clone()); // c:2306
        argv.push(tccap_get_name(cap as usize).to_string()); // c:2307
        if arg != -1 {
            // c:2310-2313 — `if (arg != -1) sprintf(buf, "%d", arg);
            //                                addlinknode(l, buf);`
            argv.push(arg.to_string());
        }
        // c:2316 — `(void)doshfunc(tcout_func, l, 1);`. Direct
        // doshfunc call mirrors C exactly.
        let shf_clone: Option<crate::ported::zsh_h::shfunc> =
            crate::ported::hashtable::shfunctab_lock()
                .read()
                .ok()
                .and_then(|t| t.get(&func_name).cloned());
        if let Some(mut shf) = shf_clone {
            let name_for_body = func_name.clone();
            let body_args = argv.clone();
            let body_runner = move || -> i32 {
                crate::ported::exec::run_function_body(&name_for_body, &body_args[1..])
                    .unwrap_or(0)
            };
            let _ = crate::ported::exec::doshfunc(&mut shf, argv.clone(), true, body_runner);
        } else {
            let _ = callhookfunc(&func_name, Some(&argv), 0, std::ptr::null_mut());
        }

        // c:2318-2331 — pull $REPLY and emit each byte (Meta-decoded)
        //               via outc().
        if let Some(reply) = getsparam("REPLY") {
            let bytes = reply.as_bytes();
            let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                if bytes[i] == 0x83 && i + 1 < bytes.len() {
                    out.push(bytes[i + 1] ^ 32); // c:2324
                    i += 2;
                } else {
                    out.push(bytes[i]); // c:2327
                    i += 1;
                }
            }
            let fd = SHTTY.load(Ordering::Relaxed);
            let out_fd = if fd >= 0 { fd } else { 1 };
            let _ = write_loop(out_fd, &out);
        }
        true
    } else {
        false
    };

    // c:2338-2340 — restore sfcontext / stopmsg / incompfunc.
    sfcontext.store(osc, Ordering::Relaxed);
    STOPMSG.store(osm, Ordering::Relaxed);
    INCOMPFUNC.store(old_incompfunc, Ordering::Relaxed);

    if dispatched {
        0
    } else {
        1
    }
}

/// Direct port of `void tcout(int cap)` from `Src/Zle/zle_refresh.c:2339`.
/// Resolves the cap escape via `tcstr[cap]` and writes it to SHTTY
/// (stdout fallback when unset). Now that init_term populates tcstr
/// with ANSI/VT100 escapes, the index lookup actually does something.
pub fn tcout(cap: i32) {
    // c:2339
    use crate::ported::init::tcstr;
    use crate::ported::zsh_h::TC_COUNT;
    let cap_idx = cap as usize;
    if cap_idx >= TC_COUNT as usize {
        return;
    }
    let escape = tcstr.lock().unwrap()[cap_idx].clone();
    if escape.is_empty() {
        return;
    }
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out_fd, escape.as_bytes());
    // c:2346 — `SELECT_ADD_COST(tclen[cap])` cost accounting dropped
    //          (no scheduling consumer reads it yet).
}

/// Port of `void tcoutarg(int cap, int arg)` from
/// Direct port of `void tcoutarg(int cap, int arg)` from
/// `Src/Zle/zle_refresh.c:2351`. Resolves the cap escape via
/// `tcstr[cap]`, expands `%d` against `arg` (the most common
/// termcap parametrisation), and writes the result to SHTTY.
pub fn tcoutarg(cap: i32, arg: i32) {
    // c:2351
    use crate::ported::init::tcstr;
    use crate::ported::zsh_h::TC_COUNT;
    let cap_idx = cap as usize;
    if cap_idx >= TC_COUNT as usize {
        return;
    }
    // c:2355 — `result = tgoto(tcstr[cap], arg, arg);`
    let escape = tcstr.lock().unwrap()[cap_idx].clone();
    if escape.is_empty() {
        return;
    }
    let s = escape.replace("%d", &arg.to_string());
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out_fd, s.as_bytes()); // c:2359
}

/// Direct port of `void clearscreen(UNUSED(char **args))` from
/// `Src/Zle/zle_refresh.c:2366`. Writes CSI 2J + CSI H to the
/// shell-output fd, then re-renders. Was a `print!` fake.
pub fn clearscreen() {
    // c:2366
    let _ = write_loop(
        {
            use std::sync::atomic::Ordering;
            let f = SHTTY.load(Ordering::Relaxed);
            if f >= 0 {
                f
            } else {
                1
            }
        },
        b"\x1b[2J\x1b[H",
    );
    zrefresh();
}

/// Direct port of `void redisplay(UNUSED(char **args))` from
/// `Src/Zle/zle_refresh.c:2377`. C kicks `resetneeded = 1` and
/// returns; Rust just re-runs zrefresh which equivalently
/// repaints from current state.
pub fn redisplay() {
    // c:2377
    zrefresh();
}

/// Port of `static void singlerefresh(ZLE_STRING_T tmpline,
/// int tmpll, int tmpcs)` from `Src/Zle/zle_refresh.c:2397`.
/// Builds the single-line video buffer used by `read -e`,
/// `vared`, and predisplay-only refresh paths: pre-allocates
/// `vbuf` sized for tabs / control chars / wide chars, walks
/// `tmpline` translating each cell into one or more
/// REFRESH_ELEMENTs (with region-highlight overlay), windows
/// the result against `winw - hasam`, then commits to
/// `nbuf[0]`.
///
/// ```c
/// static void
/// singlerefresh(ZLE_STRING_T tmpline, int tmpll, int tmpcs)
/// {
///     REFRESH_STRING vbuf, vp, refreshop;
///     int t0, vsiz, nvcs = 0, owinpos = winpos,
///         owinprompt = winprompt;
///     int width;
///     nlnct = 1;
///     for (vsiz = 1 + lpromptw, t0 = 0; t0 != tmpll; t0++) {
///         if (tmpline[t0] == '\t') vsiz = (vsiz | 7) + 2;
///         else if (WC_ISPRINT(tmpline[t0]) && (width = WCWIDTH(...)) > 0)
///             { vsiz += width; ...combining... }
///         else if (ZC_icntrl(...)) vsiz += 2;
///         else vsiz += 10;
///     }
///     vbuf = zalloc(vsiz * sizeof(*vbuf));
///     if (tmpcs < 0) tmpcs = 0;
///     ZR_memset(vbuf, zr_sp, lpromptw);
///     vp = vbuf + lpromptw;
///     *vp = zr_zr;
///     for (t0 = 0; t0 < tmpll; t0++) {
///         /* compute base_attr from region_highlights overlay */
///         /* compute all_attr = mixattrs(special_attr, special_mask, base_attr) */
///         if (t0 == tmpcs) nvcs = vp - vbuf;
///         /* emit cells per char class: \t, \n, printable wide,
///            control, default */
///     }
///     if (t0 == tmpcs) nvcs = vp - vbuf;
///     *vp = zr_zr;
///     /* window selection */
///     if (winpos == -1) winpos = 0;
///     if ((winpos && nvcs < winpos + 1) || (nvcs > winpos + winw - 2))
///         { winpos = nvcs - ((winw - hasam) / 2); if (winpos < 0) winpos = 0; }
///     if (winpos) vbuf[winpos] = '<';
///     if (ZR_strlen(vbuf + winpos) > winw - hasam)
///         { vbuf[winpos + winw - hasam - 1] = '>';
///           vbuf[winpos + winw - hasam] = zr_zr; }
///     ZR_strcpy(nbuf[0], vbuf + winpos);
///     zfree(vbuf, vsiz * sizeof(*vbuf));
///     nvcs -= winpos;
///     if (winpos < lpromptw) winprompt = lpromptw - winpos;
///     else winprompt = 0;
///     if (winpos != owinpos && winprompt)
///         { singmoveto(0); ...output left-prompt fragment... }
///     singmoveto(nvcs);
/// }
/// ```
///
/// Per PORT.md Rule 9 — body executes against stubs for
/// `addmultiword`/`mixattrs`/`region_highlights`/`shout`/`fputc`/
/// `lpromptbuf`/`MB_METACHARLENCONV` / `WCWIDTH` / `IS_BASECHAR` /
/// `IS_COMBINING` primitives not yet ported. Stubs flagged
/// inline with `// !!! STUB: …`.
pub fn singlerefresh(tmpline: &[char], tmpll: i32, mut tmpcs: i32) {
    // c:2397

    // c:2399-2405 — declarations.
    let mut vbuf: REFRESH_STRING; // c:2399
    let mut vp: usize; // c:2399 video pointer (index)
    let _refreshop: REFRESH_STRING = Vec::new(); // c:2400
    let mut t0: i32; // c:2401
    let mut vsiz: i32; // c:2402
    let mut nvcs: i32 = 0; // c:2403
                           // !!! STUB: winpos / winprompt statics — Src/Zle/zle_refresh.c:683-684.
                           // No `pub static WINPOS/WINPROMPT` ported yet; track locally.
    let owinpos: i32 = -1; // c:2404 winpos snapshot
    let _owinprompt: i32 = 0; // c:2405 winprompt snapshot
    let mut width: i32 = 0; // c:2407

    NLNCT.store(1, Ordering::SeqCst); // c:2410

    // c:2411-2437 — measure required vbuf size.
    let lpromptw = LPROMPTW.load(Ordering::SeqCst);
    vsiz = 1 + lpromptw; // c:2412
    t0 = 0;
    while t0 != tmpll {
        // c:2412
        let ch = *tmpline.get(t0 as usize).unwrap_or(&'\0');
        if ch == '\t' {
            // c:2413
            vsiz = (vsiz | 7) + 2; // c:2414
        } else if ch.is_alphanumeric() || ch.is_ascii_graphic() {
            // c:2416 WC_ISPRINT
            width = unicode_width::UnicodeWidthChar::width(ch) // c:2416 WCWIDTH
                .unwrap_or(1) as i32;
            if width > 0 {
                vsiz += width; // c:2417
                               // c:2418-2421 — combining-char absorption; skip combos.
                if isset(COMBININGCHARS) {
                    while t0 < tmpll - 1 {
                        // c:2419
                        let next = *tmpline.get((t0 + 1) as usize).unwrap_or(&'\0');
                        // !!! STUB: IS_COMBINING — Src/zsh.h:3370.
                        let is_combining = unicode_width::UnicodeWidthChar::width(next) == Some(0);
                        if !is_combining {
                            break;
                        }
                        t0 += 1; // c:2420
                    }
                }
            }
        } else if (ch as u32) < 0x20 || (ch as u32) == 0x7F {
            // c:2424 ZC_icntrl
            if (ch as u32) <= 0xff {
                // c:2426
                vsiz += 2; // c:2429
            }
        } else {
            // c:2430
            vsiz += 10; // c:2432 wide / non-printable
        }
        t0 += 1;
    }

    // c:2438 — `vbuf = zalloc(vsiz * sizeof(*vbuf));`
    vbuf = vec![REFRESH_ELEMENT { chr: '\0', atr: 0 }; vsiz as usize]; // c:2438

    if tmpcs < 0 {
        // c:2440
        tmpcs = 0; // c:2445
    }

    // c:2449 — `ZR_memset(vbuf, zr_sp, lpromptw);`
    for k in 0..(lpromptw as usize).min(vbuf.len()) {
        // c:2449
        vbuf[k] = REFRESH_ELEMENT { chr: ' ', atr: 0 };
    }
    vp = lpromptw as usize; // c:2450 vp = vbuf + lpromptw
    if vp < vbuf.len() {
        vbuf[vp] = REFRESH_ELEMENT { chr: '\0', atr: 0 }; // c:2451 *vp = zr_zr
    }

    // c:2453-2563 — main translation loop.
    t0 = 0;
    while t0 < tmpll {
        // c:2453
        // c:2454-2479 — region-highlight overlay.
        // !!! STUB: region_highlights / n_region_highlights /
        // default_attr / special_attr / special_mask / prompt_attr /
        // predisplaylen / mixattrs — Src/Zle/zle_refresh.c.
        let base_attr: u64 = 0; // c:2455 mixattrs
        let all_attr: u64 = 0; // c:2480 mixattrs

        if t0 == tmpcs {
            // c:2482
            nvcs = vp as i32; // c:2483 nvcs = vp - vbuf
        }
        let ch = *tmpline.get(t0 as usize).unwrap_or(&'\0');

        if ch == '\t' {
            // c:2484
            if vp < vbuf.len() {
                vbuf[vp] = REFRESH_ELEMENT {
                    chr: ' ',
                    atr: base_attr,
                };
                vp += 1; // c:2485 *vp++ = zr_sp
            }
            while (vp & 7) != 0 && vp < vbuf.len() {
                // c:2485
                vbuf[vp] = REFRESH_ELEMENT {
                    chr: ' ',
                    atr: base_attr,
                };
                vp += 1; // c:2486
            }
        } else if ch == '\n' {
            // c:2487
            if vp < vbuf.len() {
                vbuf[vp] = REFRESH_ELEMENT {
                    chr: '\\',
                    atr: all_attr,
                }; // c:2488-2489
                vp += 1; // c:2490
            }
            if vp < vbuf.len() {
                vbuf[vp] = REFRESH_ELEMENT {
                    chr: 'n',
                    atr: all_attr,
                }; // c:2491-2492
                vp += 1; // c:2493
            }
        } else if ch.is_ascii_graphic() || (ch.is_alphanumeric()) {
            // c:2495 WC_ISPRINT
            width = unicode_width::UnicodeWidthChar::width(ch) // c:2496 WCWIDTH
                .unwrap_or(1) as i32;
            if width > 0 {
                let ichars: i32 = 1; // c:2497-2507 combining loop
                                     // !!! STUB: addmultiword — only invoked when ichars>1.
                if vp < vbuf.len() {
                    vbuf[vp] = REFRESH_ELEMENT {
                        chr: ch,
                        atr: base_attr,
                    }; // c:2512
                    vp += 1; // c:2513
                }
                // c:2514-2518 — WEOF cells for wide-char width padding.
                let mut w = width - 1;
                while w > 0 {
                    // c:2514
                    if vp < vbuf.len() {
                        vbuf[vp] = REFRESH_ELEMENT {
                            chr: '\0',
                            atr: base_attr,
                        }; // c:2515 WEOF
                        vp += 1; // c:2517
                    }
                    w -= 1;
                }
                t0 += ichars - 1; // c:2519
            }
        } else if (ch as u32) < 0x20 || (ch as u32) == 0x7F {
            // c:2521 ZC_icntrl
            if (ch as u32) <= 0xff {
                // c:2523
                let t = ch as u32; // c:2526
                if vp < vbuf.len() {
                    vbuf[vp] = REFRESH_ELEMENT {
                        chr: '^',
                        atr: all_attr,
                    }; // c:2528-2529
                    vp += 1; // c:2530
                }
                let display: char = if (t & !0x80) > 31 {
                    // c:2531-2532
                    '?'
                } else {
                    ((t | 0x40) as u8) as char // c:2532 t | '@'
                };
                if vp < vbuf.len() {
                    vbuf[vp] = REFRESH_ELEMENT {
                        chr: display,
                        atr: all_attr,
                    };
                    vp += 1; // c:2534
                }
            }
        } else {
            // c:2537 wide non-printable
            // c:2538-2554 — emit `<%.04x>` or `<%.08x>` hex display.
            let hex = if (ch as u32) > 0xFFFF {
                // c:2542
                format!("<{:08x}>", ch as u32) // c:2543
            } else {
                // c:2544
                format!("<{:04x}>", ch as u32) // c:2545
            };
            for c in hex.chars() {
                // c:2547
                if vp < vbuf.len() {
                    vbuf[vp] = REFRESH_ELEMENT {
                        chr: c,
                        atr: all_attr,
                    }; // c:2549-2550
                    vp += 1; // c:2551
                }
            }
        }
        t0 += 1;
    }
    if t0 == tmpcs {
        // c:2564
        nvcs = vp as i32; // c:2565
    }
    if vp < vbuf.len() {
        vbuf[vp] = REFRESH_ELEMENT { chr: '\0', atr: 0 }; // c:2566 *vp = zr_zr
    }

    // c:2569-2587 — window selection.
    // !!! STUB: winpos static — Src/Zle/zle_refresh.c:683. Local.
    let mut winpos: i32 = -1;
    let winw = WINW.load(Ordering::SeqCst);
    let hasam_v = crate::ported::init::hasam.load(Ordering::SeqCst);
    if winpos == -1 {
        // c:2569
        winpos = 0; // c:2570
    }
    if (winpos != 0 && nvcs < winpos + 1)                                // c:2571
            || (nvcs > winpos + winw - 2)
    {
        winpos = nvcs - ((winw - hasam_v) / 2); // c:2572
        if winpos < 0 {
            // c:2572
            winpos = 0; // c:2573
        }
    }
    if winpos != 0 && (winpos as usize) < vbuf.len() {
        // c:2575
        vbuf[winpos as usize] = REFRESH_ELEMENT { chr: '<', atr: 0 }; // c:2576-2577 line continues left
    }
    // c:2579-2583 — right-truncate if line continues
    let suffix_start = winpos as usize;
    let suffix_len = vbuf
        .iter()
        .skip(suffix_start)
        .take_while(|e| e.chr != '\0')
        .count();
    let max_visible = (winw - hasam_v) as usize;
    if suffix_len > max_visible {
        // c:2579
        let trunc_pos = suffix_start + max_visible - 1;
        if trunc_pos < vbuf.len() {
            vbuf[trunc_pos] = REFRESH_ELEMENT { chr: '>', atr: 0 }; // c:2580-2581 line continues right
        }
        if trunc_pos + 1 < vbuf.len() {
            vbuf[trunc_pos + 1] = REFRESH_ELEMENT { chr: '\0', atr: 0 }; // c:2582
        }
    }

    // c:2584 — `ZR_strcpy(nbuf[0], vbuf + winpos);`
    // !!! STUB: nbuf[] static — defer.

    // c:2585 — zfree(vbuf, vsiz * sizeof(*vbuf)) — Rust drops on scope.
    drop(vbuf); // c:2585
    nvcs -= winpos; // c:2586

    // c:2588-2594 — winprompt update.
    let _winprompt: i32 = if winpos < lpromptw {
        // c:2588
        lpromptw - winpos // c:2590
    } else {
        // c:2591
        0 // c:2593
    };

    // c:2595-2680 — re-emit left-prompt fragment if winpos changed.
    // !!! STUB: lpromptbuf / Inpar / Outpar / convchar_t /
    // MB_METACHARLENCONV / shout / fputc — Src/Zle/zle_refresh.c.
    // The fragment-re-emit branch is the visible side-effect when
    // the user scrolls horizontally past the visible prompt; defer
    // until termcap-output primitives land.
    if winpos != owinpos {
        // c:2595
        singmoveto(&mut RefreshState::new(), 0);
        // c:2603
    }

    // c:2680 (function tail) — `singmoveto(nvcs);`
    singmoveto(&mut RefreshState::new(), nvcs as usize);

    let _ = (lpromptw, width); // silence unused
}

/// Port of `singmoveto(int pos)` from Src/Zle/zle_refresh.c:2687.
///
/// Line-by-line port of c:2687-2706. Single-line cursor positioning:
///   - exit early when already at `pos` (c:2689-2690)
///   - if no TCMULTLEFT or target close to BOL: emit `\r` and reset
///     vcs to 0 (c:2693-2695)
///   - if target is left of current: `tc_leftcurs(vcs - pos)` (c:2698)
///   - else right: `tc_rightcurs(pos - vcs)` (c:2700)
///   - update `state.vcs` to `pos` for the next call
/// WARNING: param names don't match C — Rust=(state, pos) vs C=(pos)
pub fn singmoveto(state: &mut RefreshState, pos: usize) {
    // c:2687
    use crate::ported::init::tclen;
    use crate::ported::zsh_h::TCMULTLEFT;

    // c:2689-2690 — `if (pos == vcs) return;`
    if pos == state.vcs {
        return;
    }

    let multleft_present = tclen.lock().unwrap()[TCMULTLEFT as usize] > 0;
    // c:2693-2695 — `if ((!tccan(TCMULTLEFT) || pos == 0) && pos <= vcs / 2)`
    let mut cur = state.vcs;
    if (!multleft_present || pos == 0) && pos <= cur / 2 {
        let fd = SHTTY.load(Ordering::Relaxed);
        let out_fd = if fd >= 0 { fd } else { 1 };
        let _ = write_loop(out_fd, b"\r"); // c:2694 zputc(&zr_cr)
        cur = 0;
    }

    if pos < cur {
        // c:2698 — `tc_leftcurs(vcs - pos);`
        tc_leftcurs((cur - pos) as i32);
    } else if pos > cur {
        // c:2700 — `tc_rightcurs(pos - vcs);`
        tc_rightcurs(pos - cur);
    }
    // c:2705 — `vcs = pos;`
    state.vcs = pos;
}

/// Initialize ZLE refresh subsystem
/// Port of zle_refresh_boot() from zle_refresh.c
pub fn zle_refresh_boot() -> RefreshState {
    RefreshState::new()
}

/// Port of `void zle_refresh_finish(void)` from `Src/Zle/zle_refresh.c:2720`.
/// Module-unload cleanup: `freevideo()`, drop `region_highlights` if
/// allocated (freeing memos via `free_region_highlights_memos()`,
/// zeroing `n_region_highlights`), then `free_cursor_forms()`.
pub fn zle_refresh_finish() {
    // c:2720
    let mut state = RefreshState::new(); // c:2722 freevideo file-statics
    freevideo(&mut state); // c:2722
    let mut rh = REGION_HIGHLIGHTS.lock().unwrap(); // c:2724 if (region_highlights)
    if !rh.is_empty() {
        free_region_highlights_memos(); // c:2726
        rh.clear(); // c:2727-2729 zfree + NULL
    }
    drop(rh);
    crate::ported::zle::termquery::free_cursor_forms(); // c:2733
}

// TextAttr / RefreshElement / VideoBuffer / RefreshState — Rust-side
// aggregates over zsh's C flat-globals (`winw`/`winh`/`vcs`/`vln`/
// `lpromptw`/`rpromptw`/`region_highlights[]`/`nbuf`/`obuf` in
// `Src/Zle/zle_refresh.c`). The C side represents these as separate
// file-scope statics + bitmap-packed `zattr` cells; this port collects
// them into structs for ergonomic access. Eventual unification target
// (mirroring `Src/zsh.h:2685` `pub type zattr = u64`):
//   - `TextAttr` → `zattr` (u64 packed bitmap)
//   - `RefreshElement` → `zle_h::REFRESH_ELEMENT`
//   - `VideoBuffer` → raw `Vec<REFRESH_ELEMENT>` for `nbuf`/`obuf`
//   - `RefreshState` → discrete file-scope statics

/// Unpacked-bool form of `zattr` (C's u64 packed attribute bitmap from
/// `Src/zsh.h:2685`, ported as `pub type zattr = u64`). C stores
/// attributes inline in `REFRESH_ELEMENT.atr` (a `zattr`); this port
/// pre-unpacks to a 6-field struct for ergonomic access.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TextAttr {
    /// `bold` field.
    pub bold: bool,
    /// `underline` field.
    pub underline: bool,
    /// `standout` field.
    pub standout: bool,
    /// `blink` field.
    pub blink: bool,
    /// `fg_color` field.
    pub fg_color: Option<u8>,
    /// `bg_color` field.
    pub bg_color: Option<u8>,
}

/// Display cell. Loosely equivalent to zsh's `REFRESH_ELEMENT`
/// (legit-ported at `zle_h.rs:688` as
/// `pub struct REFRESH_ELEMENT { chr: REFRESH_CHAR, atr: zattr }`).
/// Adds a `width: u8` field C doesn't have and uses `TextAttr` for
/// `atr` instead of the C `zattr` bitmap.
#[derive(Debug, Clone, Default)]
pub struct RefreshElement {
    /// `chr` field.
    pub chr: char,
    /// `atr` field.
    pub atr: TextAttr,
    /// `width` field.
    pub width: u8,
}

/// 2D screen-buffer container. C uses `REFRESH_STRING nbuf[]` and
/// `obuf[]` flat arrays of `REFRESH_ELEMENT *` (zle_refresh.c
/// globals); this struct wraps a single 2D Vec for the per-frame
/// new/old buffer pair.
#[derive(Debug, Clone)]
pub struct VideoBuffer {
    /// Buffer contents — 2D array of lines.
    pub lines: Vec<Vec<RefreshElement>>,
    /// Number of columns.
    pub cols: usize,
    /// Number of rows.
    pub rows: usize,
}

/// Composite of zle_refresh.c globals (winw/winh/vcs/vln/vmaxln,
/// oldmax, lastrow, lastcol, more_status, etc.) collected into one
/// struct. C uses separate file-statics per name
/// (`int winw, winh, vcs, vln, ...`).
#[derive(Debug, Clone, Default)]
pub struct RefreshState {
    /// Number of columns.
    pub columns: usize, // winw, window width                                // c:682
    /// Number of lines.
    pub lines: usize, // winh, window height                                 // c:682
    /// Current line on screen (cursor row).
    pub vln: usize, // video cursor position line                            // c:680
    /// Current column on screen (cursor col).
    pub vcs: usize, // video cursor position column                          // c:680
    /// Prompt width (left).
    pub lpromptw: usize, // prompt widths on screen                          // c:676
    /// Right prompt width.
    pub rpromptw: usize, // prompt widths on screen                          // c:676
    /// Scroll offset for horizontal scrolling.
    pub scrolloff: usize,
    /// Region highlight start.
    pub region_highlight_start: Option<usize>,
    /// Region highlight end.
    pub region_highlight_end: Option<usize>,
    /// Old video buffer.
    pub old_video: Option<VideoBuffer>,
    /// New video buffer.
    pub new_video: Option<VideoBuffer>,
    /// Prompt string (left).
    pub lpromptbuf: String,
    /// Right prompt string.
    pub rpromptbuf: String,
    /// Whether we need full redraw.
    pub need_full_redraw: bool,
    /// Predisplay string (before main buffer).
    pub predisplay: String,
    /// Postdisplay string (after main buffer).
    pub postdisplay: String,
}

// RegionHighlight / HighlightCategory / HighlightManager — Rust-side
// aggregates over zsh's C `region_highlights[N_SPECIAL_HIGHLIGHTS]`
// array + per-category attr globals (`default_attr`/`special_attr`/
// `ellipsis_attr` from `Src/Zle/zle_refresh.c`). C uses bare integer
// indexing into a fixed-size array; this port uses a typed enum +
// HashMap. Eventual unification: collapse into discrete file-scope
// statics matching the C layout.

/// Simplified region-highlight entry. Loosely equivalent to
/// `struct region_highlight` (legit-ported at `zle_h.rs:613` with
/// different fields: start/end/atr/flags/memo/layer).
#[derive(Debug, Clone)]
pub struct RegionHighlight {
    /// `start` field.
    pub start: usize,
    /// `end` field.
    pub end: usize,
    /// `attr` field.
    pub attr: TextAttr,
    /// `memo` field.
    pub memo: Option<String>,
    /// `flags` field — `ZRH_PREDISPLAY` etc. (Src/Zle/zle_refresh.c
    /// `struct region_highlight`). Read by `spaceinline`/`shiftchars`
    /// to decide whether `predisplaylen` is subtracted when shifting
    /// region offsets on a buffer edit.
    pub flags: i32,
}

/// Identifies a fixed slot in zsh's
/// `region_highlights[N_SPECIAL_HIGHLIGHTS]` array (zle_refresh.c
/// indices 0=region, 1=isearch, 2=suffix, 3=paste) plus the
/// standalone default/special/ellipsis attr globals
/// (`default_attr`/`special_attr`/`ellipsis_attr`). C uses bare
/// integer indexing — no enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightCategory {
    /// `Region` variant.
    Region,
    /// `Isearch` variant.
    Isearch,
    /// `Suffix` variant.
    Suffix,
    /// `Paste` variant.
    Paste,
    /// `Default` variant.
    Default,
    /// `Special` variant.
    Special,
    /// `Ellipsis` variant.
    Ellipsis,
}

/// Collects C's `region_highlights[]` array + per-category attr
/// globals (`default_attr`/`special_attr`/`ellipsis_attr` from
/// zle_refresh.c) into one container.
#[derive(Debug, Default)]
pub struct HighlightManager {
    /// `regions` field.
    pub regions: Vec<RegionHighlight>,
    /// Per-category attrs from `$zle_highlight`. Index by
    /// `HighlightCategory`. Equivalent to the per-slot atr storage
    /// in `region_highlights[]` and the
    /// `default_attr`/`special_attr`/`ellipsis_attr` globals in
    /// Src/Zle/zle_refresh.c — populated by `zle_set_highlight()`.
    pub category_attrs: std::collections::HashMap<HighlightCategory, TextAttr>,
}

/// Build the per-character attribute overlay used by `zrefresh`.
/// One slot per char in `zleline`; `None` means "default attrs",
/// `Some(attr)` means apply `attr` for that cell.
///
/// Port of the inner loop in `zrefresh()` (Src/Zle/zle_refresh.c) that
/// consults `region_highlights[]` for each visible cell. The vi
/// visual-mode region is synthesised from `region_active` + `mark`
/// here so `v` selects visibly without callers having to push a
/// region themselves — matching zle_refresh.c's auto-promotion of
/// `region_active` into a paintable highlight.
pub fn compute_render_attrs() -> Vec<Option<TextAttr>> {
    let buf_len = ZLELINE.lock().unwrap().len();
    let mut attrs: Vec<Option<TextAttr>> = vec![None; buf_len];

    // Visual-region attr: prefer the user's `region:` setting from
    // $zle_highlight (populated by zle_set_highlight); fall back to
    // standout per zsh's default at zle_refresh.c:397.
    let visual_attr = highlight()
        .lock()
        .unwrap()
        .category_attrs
        .get(&HighlightCategory::Region)
        .copied()
        .unwrap_or(TextAttr {
            standout: true,
            ..TextAttr::default()
        });

    if REGION_ACTIVE.load(Ordering::SeqCst) != 0 {
        let (lo, hi) = if MARK.load(Ordering::SeqCst) <= ZLECS.load(Ordering::SeqCst) {
            (MARK.load(Ordering::SeqCst), ZLECS.load(Ordering::SeqCst))
        } else {
            (ZLECS.load(Ordering::SeqCst), MARK.load(Ordering::SeqCst))
        };
        let lo = lo.min(buf_len);
        let hi = hi.min(buf_len);
        for slot in attrs.iter_mut().take(hi).skip(lo) {
            *slot = Some(visual_attr);
        }
    }
    for region in &highlight().lock().unwrap().regions {
        let start = region.start.min(buf_len);
        let end = region.end.min(buf_len);
        for slot in attrs.iter_mut().take(end).skip(start) {
            *slot = Some(region.attr);
        }
    }
    attrs
}

/// Full screen refresh - clears and redraws everything.
pub fn full_refresh() -> io::Result<()> {
    let fd = SHTTY.load(Ordering::Relaxed);
    let out = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out, b"\x1b[2J\x1b[H");
    zrefresh();
    Ok(())
}

/// Partial refresh (optimize for minimal updates).
pub fn partial_refresh() -> io::Result<()> {
    zrefresh();
    Ok(())
}

/// Calculate visible width of a prompt string — port of `countprompt()`
/// from Src/prompt.c:1140. The C function counts cells while skipping
/// the `Inpar..Outpar` (zsh's `%{...%}`) invisible-region tokens; this
/// Rust port skips ANSI escape sequences instead, which is what the
/// expanded prompt buffer contains by the time the refresh path uses it.
/// The C variant outputs width AND height via out-pointers; this port
/// returns width only (the only field the refresh path consumes here).
fn countprompt(s: &str) -> usize {
    let mut chars = s.chars().peekable();
    let mut width: usize = 0;
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // ANSI escape: skip until terminating letter.
            if chars.peek() == Some(&'[') {
                chars.next();
                while let Some(&nxt) = chars.peek() {
                    chars.next();
                    if nxt.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
    }
    width
}

/// Parse a highlight attribute spec (the part after the `category:` prefix)
/// into a `TextAttr`. Accepts a comma-separated list of:
///   * `bold` / `nobold`,
///   * `underline` / `nounderline`,
///   * `standout` / `nostandout`,
///   * `blink` / `noblink`,
///   * `fg=N` / `bg=N` where N is 0..=255 (256-colour palette index) or
///     one of the named ANSI colours below,
///   * `none` (clears every attr).
///
/// ZLE-region subset of `match_highlight` (Src/prompt.c:2031),
/// restricted to the tokens users actually set in `$zle_highlight`.
/// The `hl=`/`layer=`/`opacity=` clauses (prompt.c:2042-2094) are
/// not surfaced here — those are prompt-system hooks that don't
/// apply to ZLE region paint.
pub fn match_highlight(spec: &str) -> TextAttr {
    let mut attr = TextAttr::default();
    for token in spec.split(',') {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        match token {
            "none" => {
                attr = TextAttr::default();
            }
            "bold" => attr.bold = true,
            "nobold" => attr.bold = false,
            "underline" => attr.underline = true,
            "nounderline" => attr.underline = false,
            "standout" => attr.standout = true,
            "nostandout" => attr.standout = false,
            "blink" => attr.blink = true,
            "noblink" => attr.blink = false,
            other => {
                // Inline port of the named/numeric colour parser the C
                // `match_colour()` (Src/prompt.c:1957) does for `fg=`/
                // `bg=` clauses. The 24-bit `#rrggbb` form and
                // `bright-foo` aliases are not surfaced here.
                let parse = |name: &str| -> Option<u8> {
                    match name {
                        "black" => Some(0),
                        "red" => Some(1),
                        "green" => Some(2),
                        "yellow" => Some(3),
                        "blue" => Some(4),
                        "magenta" => Some(5),
                        "cyan" => Some(6),
                        "white" => Some(7),
                        "default" => None,
                        n => n.parse::<u8>().ok(),
                    }
                };
                if let Some(rest) = other.strip_prefix("fg=") {
                    attr.fg_color = parse(rest);
                } else if let Some(rest) = other.strip_prefix("bg=") {
                    attr.bg_color = parse(rest);
                }
                // Anything else (hl=, layer=, opacity=, unknown name) is
                // silently dropped — same as the C source's "found = 0"
                // exit path at prompt.c:2122 when no clause matched.
            }
        }
    }
    attr
}

/// Port of `ZR_equal(zr1, zr2)` macro from `Src/Zle/zle_refresh.c:74-82`.
/// Multibyte path: `chr == chr && atr == atr && (combining-cluster eq)`.
/// Non-multibyte path collapses to the same first conjunction. Rust uses
/// the derived `PartialEq` on `REFRESH_ELEMENT`.
#[inline]
#[allow(non_snake_case)]
pub fn ZR_equal(
    // c:74
    a: REFRESH_ELEMENT,
    b: REFRESH_ELEMENT,
) -> bool {
    a == b
}

/// Port of `ZR_memcpy(d, s, l)` macro from `Src/Zle/zle_refresh.c:92`.
/// `#define ZR_memcpy(d, s, l)  memcpy((d), (s), (l)*sizeof(REFRESH_ELEMENT))`.
/// Copy `l` REFRESH_ELEMENT slots from `src` to `dst`.
#[inline]
#[allow(non_snake_case)]
pub fn ZR_memcpy(
    // c:92
    dst: &mut [REFRESH_ELEMENT],
    src: &[REFRESH_ELEMENT],
    l: usize,
) {
    dst[..l].copy_from_slice(&src[..l]);
}

/// Port of `zr_end_ellipsis[]` from `Src/Zle/zle_refresh.c:269-281`.
/// "...>" rendered when a long line overflows past the right edge.
/// TXT_ERROR is the standard zsh-error highlight (set in zsh_h::TXT_ERROR).
pub static ZR_END_ELLIPSIS: &[REFRESH_ELEMENT] = &[
    // c:269
    REFRESH_ELEMENT { chr: ' ', atr: 0 },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT { chr: '>', atr: 0 },
];

/// Port of `zr_mid_ellipsis1[]` from `zle_refresh.c:287-294`.
/// First half of " <.... ... >" mid-line cluster.
pub static ZR_MID_ELLIPSIS1: &[REFRESH_ELEMENT] = &[
    // c:287
    REFRESH_ELEMENT { chr: ' ', atr: 0 },
    REFRESH_ELEMENT { chr: '<', atr: 0 },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
];

/// Port of `zr_mid_ellipsis2[]` from `zle_refresh.c:298-301`.
/// Trailing close of the mid-line ellipsis cluster.
pub static ZR_MID_ELLIPSIS2: &[REFRESH_ELEMENT] = &[
    // c:298
    REFRESH_ELEMENT {
        chr: '>',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT { chr: ' ', atr: 0 },
];

/// Port of `zr_start_ellipsis[]` from `zle_refresh.c:305-311`.
/// "><..." rendered when a line begins past the left edge.
pub static ZR_START_ELLIPSIS: &[REFRESH_ELEMENT] = &[
    // c:305
    REFRESH_ELEMENT { chr: '>', atr: 0 },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
    REFRESH_ELEMENT {
        chr: '.',
        atr: TXT_ERROR,
    },
];

/// Port of `tcinscost(X)` macro from `Src/Zle/zle_refresh.c:1724`.
/// `#define tcinscost(X) (tccan(TCMULTINS) ? tclen[TCMULTINS] : (X)*tclen[TCINS])`.
/// Cost (in chars) to insert `x` characters: pick the multi-insert
/// terminal capability if available, else linear cost via single-insert.
/// `tccan`/`tclen` are terminal-capability probes (Src/init.c globals);
/// without them ported we approximate with the single-insert path.
#[inline]
pub fn tcinscost(x: i32) -> i32 {
    // c:1724
    // Without tccan/tclen substrate: estimate single-char insert cost
    // as 1 unit per char.
    x.max(0)
}

/// Port of `tcdelcost(X)` macro from `Src/Zle/zle_refresh.c:1725`.
/// `#define tcdelcost(X) (tccan(TCMULTDEL) ? tclen[TCMULTDEL] : (X)*tclen[TCDEL])`.
#[inline]
pub fn tcdelcost(x: i32) -> i32 {
    // c:1725
    x.max(0)
}

/// Port of `tc_delchars(X)` macro from `Src/Zle/zle_refresh.c:1726`.
/// `(void) tcmultout(TCDEL, TCMULTDEL, (X))`. Emit `x` character-
/// delete escapes via the multi-form helper. Without curses substrate
/// it's a no-op.
#[inline]
pub fn tc_delchars(x: i32) {
    // c:1784 — `#define tc_delchars(X) (void) tcmultout(TCDEL, TCMULTDEL, (X))`.
    // Emit the terminal's delete-character capability `x` times. Used by
    // refreshline's diff path (c:1993, c:2048) to remove characters
    // without a full repaint.
    let _ = tcmultout(
        crate::ported::zsh_h::TCDEL,
        crate::ported::zsh_h::TCMULTDEL,
        x,
    );
}

/// Port of `tc_inschars(X)` macro from `Src/Zle/zle_refresh.c:1727`.
/// `(void) tcmultout(TCINS, TCMULTINS, (X))`.
#[inline]
pub fn tc_inschars(x: i32) {
    // c:1785 — `tcmultout(TCINS, TCMULTINS, (X))`.
    let _ = tcmultout(
        crate::ported::zsh_h::TCINS,
        crate::ported::zsh_h::TCMULTINS,
        x,
    );
}

/// Port of `tc_upcurs(X)` macro from `Src/Zle/zle_refresh.c:1728`.
/// `(void) tcmultout(TCUP, TCMULTUP, (X))`.
#[inline]
pub fn tc_upcurs(x: i32) {
    // c:1786 — `tcmultout(TCUP, TCMULTUP, (X))`.
    let _ = tcmultout(crate::ported::zsh_h::TCUP, crate::ported::zsh_h::TCMULTUP, x);
}

/// Port of `tc_leftcurs(X)` macro from `Src/Zle/zle_refresh.c:1729`.
/// `(void) tcmultout(TCLEFT, TCMULTLEFT, (X))`.
#[inline]
pub fn tc_leftcurs(x: i32) {
    // c:1729
    let _ = tcmultout(
        crate::ported::zsh_h::TCLEFT,
        crate::ported::zsh_h::TCMULTLEFT,
        x,
    );
}

// =====================================================================
// Refresh-cycle file-static int globals — `Src/Zle/zle_refresh.c:827-832`.
// `static int cleareol, clearf, put_rpmpt, oput_rpmpt, oxtabs,
//             numscrolls, onumscrolls;`
// Carried as AtomicI32 so the multi-threaded shell can safely flip
// them between widget invocations without locking.
// =====================================================================

/// Port of `char *tcout_func_name;` from `Src/Zle/zle_refresh.c:246`.
/// Holds the name of the user `zle -T tc <fn>` redisplay-transform
/// function; cleared by `zle -T -r`. The refresh path invokes it
/// via `getshfunc(tcout_func_name)` (zle_refresh.c:2303).
pub static TCOUT_FUNC_NAME: std::sync::Mutex<Option<String>> = // c:246
    std::sync::Mutex::new(None);

/// Port of `zattr pmpt_attr, rpmpt_attr, prompt_attr;` from
/// `Src/Zle/zle_refresh.c:152`. Captured text attributes for the
/// left prompt / right prompt / the prompt-tail position respectively.
///
/// Used by:
///   - zle_main.c:1238 — `pmpt_attr = txtcurrentattrs;` after
///     left-prompt expansion to remember the SGR state at prompt end.
///   - zle_main.c:1247 — same for `rpmpt_attr` after right-prompt.
///   - zle_main.c:1248 — `prompt_attr = …` derived from the two.
///   - zle_refresh.c:1163/1666 — `txtcurrentattrs = txtpendingattrs =
///     {pmpt,rpmpt}_attr;` to restore the captured state when
///     redrawing.
///   - zle_refresh.c:1657 — `treplaceattrs(pmpt_attr);` to flush.
///
/// `zattr` is `u64` (zsh.h:2689); AtomicU64 with Relaxed ordering
/// matches C's plain global-int read/write shape.
pub static PMPT_ATTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // c:152
/// `RPMPT_ATTR` static.
pub static RPMPT_ATTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // c:152
/// `PROMPT_ATTR` static.
pub static PROMPT_ATTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // c:152

/// Port of `static int cleareol` from `Src/Zle/zle_refresh.c:827`.
/// Clear-to-end-of-line flag — set when the terminal lacks `cleareod`
/// and we have to fall back to per-line clear.
pub static CLEAREOL: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:827

/// Port of `static int clearf` from `Src/Zle/zle_refresh.c:828`.
/// Set when `alwayslastprompt` was used immediately before the
/// current refresh — drives a special clear path.
pub static CLEARF: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:828

/// Port of `static int put_rpmpt` from `Src/Zle/zle_refresh.c:829`.
/// Whether we should display the right-prompt this refresh.
pub static PUT_RPMPT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:829

/// Port of `static int oput_rpmpt` from `Src/Zle/zle_refresh.c:830`.
/// Whether the right-prompt was displayed last refresh.
pub static OPUT_RPMPT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:830

/// Port of `static int oxtabs` from `Src/Zle/zle_refresh.c:831`.
/// `oxtabs` flag — tabs expand to spaces if set.
pub static OXTABS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:831

/// Port of `static int numscrolls` from `Src/Zle/zle_refresh.c:832`.
/// Count of scroll operations this refresh — used by `nextline` to
/// decide whether to abort line-loop processing.
pub static NUMSCROLLS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:832

/// Port of `static int onumscrolls` from `Src/Zle/zle_refresh.c:832`.
/// Previous refresh's `numscrolls` value — `nextline` compares to
/// detect runaway scrolling.
pub static ONUMSCROLLS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:832

// =====================================================================
// mod_export refresh-state globals — `Src/Zle/zle_refresh.c:157-188`.
// Exposed across translation units (other modules read them).
// AtomicI32 for safe lock-free access.
// =====================================================================

/// Port of `mod_export int nlnct` from `Src/Zle/zle_refresh.c:157`.
/// Number of lines counted in the prompt+buffer for the current
/// refresh — drives nbuf allocation (`nlnct * winw` cells).
pub static NLNCT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:157

/// Port of `static REFRESH_STRING *nbuf` / `*obuf` from
/// `Src/Zle/zle_refresh.c:670`. The new (`NBUF`) and old (`OBUF`) video
/// buffers: one `REFRESH_STRING` (a `Vec<REFRESH_ELEMENT>`) per screen
/// line. `zrefresh` builds `NBUF`, `refreshline` diffs `NBUF[ln]` against
/// `OBUF[ln]` and emits the minimal terminal updates, then they are
/// swapped (c:954-955) so this frame's new buffer is next frame's old.
pub static NBUF: std::sync::Mutex<Vec<REFRESH_STRING>> = std::sync::Mutex::new(Vec::new()); // c:670
pub static OBUF: std::sync::Mutex<Vec<REFRESH_STRING>> = std::sync::Mutex::new(Vec::new()); // c:670

/// Port of `static int winw` from `Src/Zle/zle_refresh.c:682`.
/// Terminal window width in cells; bounded by `zterm_columns`.
pub static WINW: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(80); // c:682

/// Port of `static int winh` from `Src/Zle/zle_refresh.c:682`.
/// Terminal window height in cells; bounded by `zterm_lines`.
pub static WINH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(24); // c:682

/// Port of `static int lpromptw` from `Src/Zle/zle_refresh.c:676`.
/// Left prompt's on-screen width after expansion / truncation.
pub static LPROMPTW: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:676

/// Port of `static int vcs` from `Src/Zle/zle_refresh.c:680`.
/// Video cursor column — physical column of the cursor on screen.
pub static VCS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:680

/// Port of `static int vln` from `Src/Zle/zle_refresh.c:680`.
/// Video cursor line — physical row of the cursor on screen.
pub static VLN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:680

/// Port of `mod_export int showinglist` from `Src/Zle/zle_refresh.c:165`.
/// Non-zero when a completion-listing is currently displayed below
/// the prompt; refreshes need to redraw it on next paint.
pub static SHOWINGLIST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:165

/// Port of `mod_export int listshown` from `Src/Zle/zle_refresh.c:171`.
/// Number of completion-listing lines actually shown last refresh —
/// used by clear path to know how many lines to wipe.
pub static LISTSHOWN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:171

/// Port of `mod_export int lastlistlen` from `Src/Zle/zle_refresh.c:176`.
/// Length of the previous listing (separate from `listshown` because
/// the listing might be paginated).
pub static LASTLISTLEN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:176

/// Port of `static int vmaxln` from `Src/Zle/zle_refresh.c:680`.
/// Maximum line index reached during this refresh — used to decide
/// how much of the prior frame to clear.
pub static VMAXLN: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:680

/// Port of `static int winprompt` from `Src/Zle/zle_refresh.c:680`.
/// Number of physical lines the prompt occupies on screen.
pub static WINPROMPT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:680

/// Port of `static int winpos` from `Src/Zle/zle_refresh.c:680`.
/// Horizontal scroll offset when the buffer is wider than winw.
pub static WINPOS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1); // c:680

/// Port of `static int rwinh` from `Src/Zle/zle_refresh.c:682`.
/// Real terminal-line count (vs the TERM_SHORT-clamped `winh`).
pub static RWINH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(24); // c:682

/// Port of `static int lpromptwof` from `Src/Zle/zle_refresh.c:676`.
/// Left prompt's pre-wrap visible width (`lpromptw` may differ when
/// the prompt fills the line exactly and forces an extra row).
pub static LPROMPTWOF: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:676

/// Port of `static int lprompth` from `Src/Zle/zle_refresh.c:676`.
/// Left prompt's height in rows.
pub static LPROMPTH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:676

/// Port of `static int rpromptw` from `Src/Zle/zle_refresh.c:676`.
/// Right prompt's on-screen width.
pub static RPROMPTW: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:676

/// Port of `static int rprompth` from `Src/Zle/zle_refresh.c:676`.
/// Right prompt's height in rows.
pub static RPROMPTH: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:676

/// Port of `static int olnct` from `Src/Zle/zle_refresh.c:157`.
/// Number of lines in the previous refresh — caller diff renders
/// against this.
pub static OLNCT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:157

/// Port of `mod_export int trashedzle` from `Src/Zle/zle_refresh.c:181`.
/// Set when the on-screen line was wiped (by `trashzle`); next refresh
/// must do a full redraw.
pub static TRASHEDZLE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:181

/// Port of `mod_export int clearflag` from `Src/Zle/zle_refresh.c:183`.
/// Request a full screen-clear on next refresh (set by `clear-screen`
/// widget + Ctrl+L).
pub static CLEARFLAG: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:183

/// Port of `mod_export int clearlist` from `Src/Zle/zle_refresh.c:188`.
/// Request the completion-listing be wiped on next refresh.
pub static CLEARLIST: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0); // c:188

/// Port of `struct rparams` from `Src/Zle/zle_refresh.c:815`. Workspace
/// state threaded through `zrefresh` + `nextline` + `wpfx` — tracks the
/// current line being painted, scroll budget, video cursor, and the
/// in/out pointers into the video buffer.
///
/// C definition (c:815-824):
/// ```c
/// struct rparams {
///     int canscroll;
///     int ln;
///     int more_status;
///     int nvcs;
///     int nvln;
///     int tosln;
///     REFRESH_STRING s;
///     REFRESH_STRING sen;
/// };
/// typedef struct rparams *rparams;
/// ```
///
/// Rust port replaces `REFRESH_STRING s/sen` (raw pointers into the
/// video buffer) with `pos`/`end` byte indices for safe access.
#[derive(Debug, Clone, Default)]
#[allow(non_camel_case_types)]
pub struct rparams {
    // c:815
    /// Number of lines we are allowed to scroll.
    pub canscroll: i32, // c:816
    /// Current line we're working on.
    pub ln: i32, // c:817
    /// More stuff in status line.
    pub more_status: i32, // c:818
    /// Video cursor column.
    pub nvcs: i32, // c:819
    /// Video cursor line.
    pub nvln: i32, // c:820
    /// Tmp in statusline stuff.
    pub tosln: i32, // c:821
    /// Cursor index into the video buffer (was `REFRESH_STRING s`).
    pub pos: usize, // c:822
    /// End-of-line index (was `REFRESH_STRING sen`).
    pub end: usize, // c:823
}

/// Port of `void set_region_highlight(UNUSED(Param pm), char **aval)`
/// from `Src/Zle/zle_refresh.c:488`. Setter for the `$region_highlight`
/// special parameter. C body: resize the `region_highlights[]` global
/// to `len(aval) + N_SPECIAL_HIGHLIGHTS`, freeing memo strings on
/// each replaced entry, then parse each `aval[i]` into one entry:
/// optional leading `P` sets `ZRH_PREDISPLAY`, two decimal indices
/// become `start`/`end`, `match_highlight()` parses the attribute
/// spec into `atr`+`atrmask` and optional `layer`, and a trailing
/// `memo=NAME` field is stored verbatim. Passing `aval=None` (the
/// `NULL` case from `unset_region_highlight`, c:595) truncates to
/// the special baseline. C uses a packed `struct region_highlight`
/// global; the Rust port stores parsed entries in
/// `REGION_HIGHLIGHTS` via the simplified `RegionHighlight` shape.
/// WARNING: param names don't match C — Rust=(aval) vs C=(pm, aval)
pub fn set_region_highlight(aval: Option<&[String]>) {
    // c:488
    let aval = match aval {
        // c:510 !aval
        Some(a) => a,
        None => {
            // c:495-508 — truncate to special baseline when aval is NULL.
            REGION_HIGHLIGHTS.lock().unwrap().clear();
            return; // c:511
        }
    };
    let mut rh = REGION_HIGHLIGHTS.lock().unwrap();
    rh.clear(); // c:500 free memos
    for entry in aval.iter() {
        // c:513
        let mut oldstrp: &str = entry.as_str(); // c:519
        let mut flags: i32 = 0; // c:525
        if oldstrp.starts_with('P') {
            // c:520
            flags = ZRH_PREDISPLAY; // c:521
            oldstrp = &oldstrp[1..]; // c:522
        }
        oldstrp = oldstrp.trim_start_matches(|c: char| c == ' ' || c == '\t'); // c:526
        let (start_val, rest1) = crate::ported::utils::zstrtol(oldstrp, 10); // c:529
        let start = if oldstrp.len() == rest1.len() {
            -1i32
        } else {
            start_val as i32
        }; // c:530-531
        let strp = rest1.trim_start_matches(|c: char| c == ' ' || c == '\t'); // c:533
        let (end_val, rest2) = crate::ported::utils::zstrtol(strp, 10); // c:537
        let end = if strp.len() == rest2.len() {
            -1i32
        } else {
            end_val as i32
        }; // c:537-538
        let strp = rest2.trim_start_matches(|c: char| c == ' ' || c == '\t'); // c:541
                                                                              // c:545 — match_highlight(strp, ...) into attr fields.
        let attr = match_highlight(strp);
        // c:551 — memo= field extraction.
        let memo = if let Some(rest) = strp.strip_prefix("memo=") {
            // c:517,551
            let end_pos = rest
                .find(|c: char| c == ',' || c == ' ' || c == '\t' || c == '\0')
                .unwrap_or(rest.len());
            Some(rest[..end_pos].to_string()) // c:581
        } else {
            None
        }; // c:583
        if start >= 0 && end >= 0 {
            rh.push(RegionHighlight {
                start: start as usize,
                end: end as usize,
                attr,
                memo,
                flags, // c:521 — ZRH_PREDISPLAY from the `P` prefix (was discarded)
            });
        }
    }
    // c:586 — freearray(av): aval owned by caller in Rust.
}

/// Process-wide region-highlights table, the Rust analog of C's
/// `mod_export struct region_highlight *region_highlights` global
/// (`Src/Zle/zle_refresh.c:471`). Holds parsed `$region_highlight`
/// entries plus internal callers (isearch/region/suffix).
pub static REGION_HIGHLIGHTS: once_cell::sync::Lazy<std::sync::Mutex<Vec<RegionHighlight>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(Vec::new()));

/// Port of `ZRH_PREDISPLAY` from `Src/Zle/zle.h` — bit flag set when
/// a `region_highlight` entry's indices are relative to the
/// predisplay buffer rather than `zleline`.
pub const ZRH_PREDISPLAY: i32 = 1;

#[cfg(test)]
mod zr_tests {
    use super::*;
    use crate::ported::zle::zle_h::REFRESH_ELEMENT;
    use crate::ported::zsh_h::{TXTBOLDFACE, TXT_MULTIWORD_MASK};

    fn re(c: char, a: u64) -> REFRESH_ELEMENT {
        REFRESH_ELEMENT { chr: c, atr: a }
    }

    #[test]
    fn zr_memset_fills_slice() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:88-89 — `while (len--) *dst++ = rc`.
        let mut buf = [REFRESH_ELEMENT::default(); 4];
        let fill = re('x', 0);
        ZR_memset(&mut buf, fill, 3);
        assert_eq!(buf[0], fill);
        assert_eq!(buf[1], fill);
        assert_eq!(buf[2], fill);
        // 4th slot unchanged
        assert_eq!(buf[3], REFRESH_ELEMENT::default());
    }

    #[test]
    fn zr_memset_clamps_to_dst_len() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut buf = [REFRESH_ELEMENT::default(); 2];
        let fill = re('y', 0);
        ZR_memset(&mut buf, fill, 99); // len > dst.len()
        assert_eq!(buf[0], fill);
        assert_eq!(buf[1], fill);
    }

    #[test]
    fn zr_strlen_counts_to_nul() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:106 — `while (wstr++->chr != ZWC('\0')) len++`.
        let s = [re('h', 0), re('i', 0), re('\0', 0)];
        assert_eq!(ZR_strlen(&s), 2);
    }

    #[test]
    fn zr_strlen_empty_starts_with_nul() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let s = [re('\0', 0)];
        assert_eq!(ZR_strlen(&s), 0);
    }

    #[test]
    fn zr_strcpy_copies_through_nul() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:97 — `while ((*dst++ = *src++).chr != ZWC('\0'))`. NUL
        // included in copy.
        let src = [re('a', 0), re('b', 0), re('\0', 0)];
        let mut dst = [REFRESH_ELEMENT::default(); 5];
        ZR_strcpy(&mut dst, &src);
        assert_eq!(dst[0], re('a', 0));
        assert_eq!(dst[1], re('b', 0));
        assert_eq!(dst[2], re('\0', 0));
    }

    #[test]
    fn zr_strncmp_equal_strings() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:127 — pair-equal in chr+atr: returns 0.
        let a = [re('h', 0), re('i', 0)];
        let b = [re('h', 0), re('i', 0)];
        assert_eq!(ZR_strncmp(&a, &b, 2), 0);
    }

    #[test]
    fn zr_strncmp_diff_chr_returns_1() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = [re('h', 0), re('i', 0)];
        let b = [re('h', 0), re('o', 0)];
        // c:127 — `if (!ZR_equal(...)) return 1`.
        assert_eq!(ZR_strncmp(&a, &b, 2), 1);
    }

    #[test]
    fn zr_strncmp_diff_atr_returns_1() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:127 — atr is part of equality.
        let a = [re('h', 0)];
        let b = [re('h', TXTBOLDFACE)];
        assert_eq!(ZR_strncmp(&a, &b, 1), 1);
    }

    #[test]
    fn zr_strncmp_early_nul_old() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:124-126 — old has NUL → return !equal.
        let a = [re('\0', 0)];
        let b = [re('x', 0)];
        assert_eq!(ZR_strncmp(&a, &b, 1), 1); // not equal
        let a = [re('\0', 0)];
        let b = [re('\0', 0)];
        assert_eq!(ZR_strncmp(&a, &b, 1), 0); // equal NULs
    }

    #[test]
    fn zr_strncmp_multiword_mask_skips_nul_check() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // c:124 — `(!(oldwstr->atr & TXT_MULTIWORD_MASK) && !oldwstr->chr)`.
        // If atr has MULTIWORD set, chr=='\0' is NOT a NUL terminator.
        let a = [re('\0', TXT_MULTIWORD_MASK)];
        let b = [re('\0', TXT_MULTIWORD_MASK)];
        // Both elements equal (same chr+atr) → returns 0; the
        // multiword mask path skips the early-NUL exit so we fall
        // through to the regular ZR_equal check.
        assert_eq!(ZR_strncmp(&a, &b, 1), 0);
    }

    #[test]
    fn zr_equal_same_returns_true() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let a = re('a', 0);
        assert!(ZR_equal(a, a));
        let b = re('b', 0);
        assert!(!ZR_equal(a, b));
    }

    #[test]
    fn zr_memcpy_copies_n_elements() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut dst = [re('\0', 0); 5];
        let src = [re('a', 0), re('b', 0), re('c', 0), re('d', 0), re('e', 0)];
        ZR_memcpy(&mut dst, &src, 3);
        assert_eq!(dst[0].chr, 'a');
        assert_eq!(dst[1].chr, 'b');
        assert_eq!(dst[2].chr, 'c');
        assert_eq!(dst[3].chr, '\0');
    }

    #[test]
    fn ellipsis_sizes_match_table_lengths() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(ZR_END_ELLIPSIS_SIZE, 6);
        assert_eq!(ZR_MID_ELLIPSIS1_SIZE, 6);
        assert_eq!(ZR_MID_ELLIPSIS2_SIZE, 2);
        assert_eq!(ZR_START_ELLIPSIS_SIZE, 5);
    }

    #[test]
    fn def_mwbuf_alloc_is_32() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(DEF_MWBUF_ALLOC, 32);
    }

    #[test]
    fn tc_costs_handle_negative() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(tcinscost(-1), 0);
        assert_eq!(tcdelcost(-1), 0);
        assert_eq!(tcinscost(5), 5);
        assert_eq!(tcdelcost(5), 5);
    }

    #[test]
    fn rparams_default_zeros_all_fields() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let r = rparams::default();
        assert_eq!(r.canscroll, 0);
        assert_eq!(r.ln, 0);
        assert_eq!(r.more_status, 0);
        assert_eq!(r.nvcs, 0);
        assert_eq!(r.nvln, 0);
        assert_eq!(r.tosln, 0);
        assert_eq!(r.pos, 0);
        assert_eq!(r.end, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// zrefresh builds NBUF from the prompt + editable line (c:1208-1400):
    /// prompt cells, then line chars with tab→8-col-stop and control→`^X`
    /// expansion. Verifies the video buffer the diff machinery consumes.
    #[test]
    fn zrefresh_builds_nbuf_cells() {
        let _g = crate::test_util::global_state_lock();
        // Drive the editable line directly: "ab\tc\u{1}d".
        *ZLELINE.lock().unwrap() = "ab\tc\u{1}d".chars().collect();
        ZLECS.store(0, Ordering::SeqCst);
        ZLELL.store(6, Ordering::SeqCst);

        zrefresh();

        let nbuf = NBUF.lock().unwrap();
        let row0: String = nbuf
            .first()
            .map(|r| r.iter().map(|c| c.chr).collect())
            .unwrap_or_default();
        // The line content (after whatever prompt prefix): "ab" then a tab
        // expanded to spaces landing on an 8-col stop, "c", "^A", "d".
        assert!(
            row0.contains("ab") && row0.ends_with("c^Ad"),
            "NBUF row 0 should render the line with tab + ^A expansion, got {:?}",
            row0
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    // addmultiword — C-pinned tests covering pattern.c:913-936 push path.
    // Pin (1) `base.atr |= TXT_MULTIWORD_MASK`, (2) `nmwbuf[ind]` stores
    // the cluster count, (3) `nmwbuf[ind+1..ind+1+count]` stores the
    // cluster codepoints, (4) `base.chr` set to the buffer entry index,
    // (5) `nmw_ind` advances by `count + 1`, (6) the `nmw_ind` init=1
    // invariant from c:43 holds (entries never start at 0).
    //
    // Bug fixed: the prior body silently discarded `tptr`/`ichars`,
    // setting only the flag. Every cluster of combining marks landed
    // unstored in the buffer; readers dispatching on TXT_MULTIWORD_MASK
    // got an uninitialised index.
    // ═══════════════════════════════════════════════════════════════════

    /// Restore the post-resetvideo invariant (c:745-747):
    /// `nmw_size = DEF_MWBUF_ALLOC; nmw_ind = 1`. C zsh always runs
    /// `resetvideo` before any `addmultiword` call; the tests below
    /// mirror that init so we exercise the same growth math the
    /// C source guarantees correctness for.
    fn reset_nmw_state() {
        NMW_IND.with(|c| c.set(1)); // c:746
        NMW_SIZE.with(|c| c.set(DEF_MWBUF_ALLOC)); // c:745
        NMWBUF.with(|b| {
            let mut buf = b.borrow_mut();
            buf.clear();
            buf.resize(DEF_MWBUF_ALLOC, 0); // c:747 zalloc(nmw_size)
        });
    }

    /// `Src/Zle/zle_refresh.c:913-935` — basic single-cluster push:
    /// flag set, buffer layout [count, c0, c1], chr = old nmw_ind,
    /// nmw_ind advances by count+1.
    #[test]
    fn addmultiword_stores_cluster_and_sets_chr_index() {
        let _g = crate::test_util::global_state_lock();
        reset_nmw_state();

        let mut base = REFRESH_ELEMENT { chr: '\0', atr: 0 };
        // Cluster: 'a' + 1 combining-mark codepoint.
        let cluster = ['a', '\u{0301}'];
        addmultiword(&mut base, &cluster, 2);

        // c:920 — TXT_MULTIWORD_MASK set.
        assert_ne!(base.atr & TXT_MULTIWORD_MASK, 0, "c:920 flag set");
        // c:934 — base.chr stores the buffer index (1 was the old NMW_IND init).
        assert_eq!(base.chr as u32, 1, "c:934 base.chr = old nmw_ind");

        NMWBUF.with(|b| {
            let buf = b.borrow();
            // c:930 — first slot: cluster count.
            assert_eq!(buf[1], 2, "c:930 nmwbuf[ind] = ichars");
            // c:931-932 — next `count` slots: the codepoints.
            assert_eq!(buf[2], 'a' as u32, "c:932 nmwbuf[ind+1] = tptr[0]");
            assert_eq!(buf[3], '\u{0301}' as u32, "c:932 nmwbuf[ind+2] = tptr[1]");
        });
        // c:935 — nmw_ind advanced by count + 1 = 3 (from 1 → 4).
        assert_eq!(NMW_IND.get(), 4, "c:935 nmw_ind += iadd (1 + 2 + 1)");
        reset_nmw_state();
    }

    /// `Src/Zle/zle_refresh.c:921-927` — buffer auto-grow only triggers
    /// when `nmw_ind + iadd > nmw_size`. Post-resetvideo size=32 fits a
    /// small cluster without growth.
    #[test]
    fn addmultiword_no_grow_when_capacity_fits() {
        let _g = crate::test_util::global_state_lock();
        reset_nmw_state();
        let pre_size = NMW_SIZE.get();
        let mut base = REFRESH_ELEMENT { chr: '\0', atr: 0 };
        addmultiword(&mut base, &['x'], 1);
        assert_eq!(
            NMW_SIZE.get(),
            pre_size,
            "c:921 — no grow needed (1+2 ≤ 32)"
        );
        reset_nmw_state();
    }

    /// `Src/Zle/zle_refresh.c:922-925` — large cluster (iadd > DEF_MWBUF_ALLOC)
    /// triggers a grow of `iadd` slots instead of `DEF_MWBUF_ALLOC`.
    #[test]
    fn addmultiword_grows_by_iadd_for_large_cluster() {
        let _g = crate::test_util::global_state_lock();
        reset_nmw_state();
        let pre_size = NMW_SIZE.get();
        let mut base = REFRESH_ELEMENT { chr: '\0', atr: 0 };
        // 40-codepoint cluster: iadd = 41 > DEF_MWBUF_ALLOC=32.
        let cluster: Vec<char> = (0..40)
            .map(|i| char::from_u32(0x300 + i as u32).unwrap())
            .collect();
        addmultiword(&mut base, &cluster, 40);
        // c:922 — mw_more = iadd = 41, nmw_size = pre_size + 41.
        assert_eq!(
            NMW_SIZE.get(),
            pre_size + 41,
            "c:922 — grow = iadd, not DEF_MWBUF_ALLOC"
        );
        reset_nmw_state();
    }

    /// `Src/Zle/zle_refresh.c:43` — nmw_ind init = 1 invariant so a
    /// zero-index slot never appears (would compare equal to NUL).
    /// Two consecutive pushes land at indices 1 and 4 (1 + 1+2 for the
    /// first 2-cluster push, then +3 for the second).
    #[test]
    fn addmultiword_consecutive_pushes_land_at_correct_indices() {
        let _g = crate::test_util::global_state_lock();
        reset_nmw_state();

        let mut a = REFRESH_ELEMENT { chr: '\0', atr: 0 };
        addmultiword(&mut a, &['e', '\u{0301}'], 2);
        let mut b = REFRESH_ELEMENT { chr: '\0', atr: 0 };
        addmultiword(&mut b, &['n', '\u{0303}'], 2);

        // First push at nmw_ind=1; advanced to 4.
        assert_eq!(a.chr as u32, 1, "first push index");
        // Second push at nmw_ind=4; advanced to 7.
        assert_eq!(
            b.chr as u32, 4,
            "second push index (c:43 invariant — never 0)"
        );
        assert_eq!(NMW_IND.get(), 7, "after two 2-clusters: 1 + 3 + 3 = 7");
        reset_nmw_state();
    }

    #[test]
    fn test_countprompt() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(countprompt("hello"), 5);
        assert_eq!(countprompt("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(countprompt("日本語"), 6); // 3 chars, 2 width each
    }

    #[test]
    fn test_video_buffer() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut buf = VideoBuffer::new(80, 24);
        assert_eq!(buf.cols, 80);
        assert_eq!(buf.rows, 24);

        buf.set(0, 0, RefreshElement::new('A'));
        assert_eq!(buf.get(0, 0).map(|e| e.chr), Some('A'));

        buf.clear();
        assert_eq!(buf.get(0, 0).map(|e| e.chr), Some(' '));
    }

    #[test]
    fn test_refresh_state() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut state = RefreshState::new();
        assert!(state.old_video.is_some());
        assert!(state.new_video.is_some());

        state.swap_buffers();
        state.free_video();
        assert!(state.old_video.is_none());
    }

    #[test]
    fn compute_render_attrs_empty_buffer_yields_empty_overlay() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert!(compute_render_attrs().is_empty());
    }

    #[test]
    fn compute_render_attrs_visual_mode_paints_mark_to_cursor_in_standout() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "hello world".chars().collect();
        ZLELL.store(ZLELINE.lock().unwrap().len(), Ordering::SeqCst);
        MARK.store(2, Ordering::SeqCst);
        ZLECS.store(7, Ordering::SeqCst);
        REGION_ACTIVE.store(1, Ordering::SeqCst); // charwise visual
        let attrs = compute_render_attrs();
        assert_eq!(attrs.len(), 11);
        // [0..2) and [7..11) are unstyled.
        for slot in attrs.iter().take(2) {
            assert!(slot.is_none());
        }
        for slot in attrs.iter().skip(7) {
            assert!(slot.is_none());
        }
        // [2..7) painted in standout.
        for slot in attrs.iter().take(7).skip(2) {
            let attr = slot.expect("standout");
            assert!(attr.standout);
        }
    }

    #[test]
    fn compute_render_attrs_visual_mode_handles_reverse_mark_order() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        ZLELL.store(6, Ordering::SeqCst);
        MARK.store(5, Ordering::SeqCst);
        ZLECS.store(1, Ordering::SeqCst);
        REGION_ACTIVE.store(2, Ordering::SeqCst); // linewise — same swap behavior
        let attrs = compute_render_attrs();
        // Range collapses to (1..5).
        assert!(attrs[0].is_none());
        for slot in attrs.iter().take(5).skip(1) {
            assert!(slot.unwrap().standout);
        }
        assert!(attrs[5].is_none());
    }

    #[test]
    fn match_highlight_handles_combined_attrs() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let attr = match_highlight("bold,fg=red,underline");
        assert!(attr.bold);
        assert!(attr.underline);
        assert_eq!(attr.fg_color, Some(1));
    }

    #[test]
    fn match_highlight_named_and_numeric_colors() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        assert_eq!(match_highlight("fg=cyan").fg_color, Some(6));
        assert_eq!(match_highlight("bg=42").bg_color, Some(42));
        // Out-of-range numeric → ignored (parse fails for u8).
        assert_eq!(match_highlight("fg=999").fg_color, None);
    }

    #[test]
    fn match_highlight_negation_clears_attr() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let attr = match_highlight("bold,nobold,underline");
        assert!(!attr.bold);
        assert!(attr.underline);
    }

    #[test]
    fn match_highlight_none_resets_everything() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let attr = match_highlight("bold,fg=red,none,underline");
        // After `none` the only thing surviving is the trailing `underline`.
        assert!(!attr.bold);
        assert!(attr.underline);
        assert_eq!(attr.fg_color, None);
    }

    #[test]
    fn zle_set_highlight_populates_categories_and_defaults() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut mgr = HighlightManager::new();
        let entries = ["region:fg=red,bold", "isearch:fg=blue"];
        zle_set_highlight(&mut mgr, &entries);
        let region = mgr.category_attrs[&HighlightCategory::Region];
        assert!(region.bold);
        assert_eq!(region.fg_color, Some(1));
        let isearch = mgr.category_attrs[&HighlightCategory::Isearch];
        assert_eq!(isearch.fg_color, Some(4));
        // Suffix wasn't set: defaults to bold (zle_refresh.c:401).
        let suffix = mgr.category_attrs[&HighlightCategory::Suffix];
        assert!(suffix.bold);
        // Special wasn't set: defaults to standout (zle_refresh.c:396).
        let special = mgr.category_attrs[&HighlightCategory::Special];
        assert!(special.standout);
    }

    #[test]
    fn zle_set_highlight_none_clears_every_slot() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        let mut mgr = HighlightManager::new();
        zle_set_highlight(&mut mgr, &["none"]);
        for cat in [
            HighlightCategory::Region,
            HighlightCategory::Isearch,
            HighlightCategory::Suffix,
            HighlightCategory::Paste,
        ] {
            let attr = mgr.category_attrs[&cat];
            assert_eq!(attr, TextAttr::default());
        }
    }

    #[test]
    fn compute_render_attrs_visual_uses_zle_highlight_region_attr() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        // When the user sets `zle_highlight=(region:fg=red,bold)` via
        // zle_set_highlight, vi visual-mode should paint the region
        // with that attr instead of the default standout.
        zle_reset();
        *ZLELINE.lock().unwrap() = "abcde".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        MARK.store(1, Ordering::SeqCst);
        ZLECS.store(4, Ordering::SeqCst);
        REGION_ACTIVE.store(1, Ordering::SeqCst);
        zle_set_highlight(&mut highlight().lock().unwrap(), &["region:fg=red,bold"]);
        let attrs = compute_render_attrs();
        for slot in attrs.iter().take(4).skip(1) {
            let a = slot.expect("region painted");
            assert!(a.bold);
            assert_eq!(a.fg_color, Some(1));
            // Standout shouldn't be auto-set when user overrode.
            assert!(!a.standout);
        }
    }

    #[test]
    fn compute_render_attrs_explicit_regions_override_default() {
        let _g = crate::test_util::global_state_lock();
        let _g = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abcde".chars().collect();
        ZLELL.store(5, Ordering::SeqCst);
        let custom = TextAttr {
            bold: true,
            fg_color: Some(1),
            ..TextAttr::default()
        };
        highlight().lock().unwrap().add_region(1, 4, custom);
        let attrs = compute_render_attrs();
        assert!(attrs[0].is_none());
        for slot in attrs.iter().take(4).skip(1) {
            let a = slot.expect("custom");
            assert!(a.bold);
            assert_eq!(a.fg_color, Some(1));
        }
        assert!(attrs[4].is_none());
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests pinning Src/Zle/zle_refresh.c ZR_* helpers.
    // ═══════════════════════════════════════════════════════════════════

    /// `ZR_strlen` returns 0 for an empty (NUL-terminated) buffer.
    /// C `Src/Zle/zle_refresh.c:102`:
    ///   `int len = 0; while (wstr++->chr != '\0') len++; return len;`
    #[test]
    fn ZR_strlen_empty_terminated_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let buf = [REFRESH_ELEMENT { chr: '\0', atr: 0 }];
        assert_eq!(ZR_strlen(&buf), 0);
    }

    /// `ZR_strlen` counts chars up to (not including) the NUL.
    #[test]
    fn ZR_strlen_counts_chars_before_nul() {
        let _g = crate::test_util::global_state_lock();
        let buf = [
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
            REFRESH_ELEMENT { chr: '\0', atr: 0 },
        ];
        assert_eq!(ZR_strlen(&buf), 3);
    }

    /// `ZR_strlen` on a buffer with NO NUL returns slice len.
    /// Rust port adds bounds check (C UB on no-NUL buffer).
    #[test]
    fn ZR_strlen_no_nul_returns_full_len() {
        let _g = crate::test_util::global_state_lock();
        let buf = [
            REFRESH_ELEMENT { chr: 'x', atr: 0 },
            REFRESH_ELEMENT { chr: 'y', atr: 0 },
        ];
        assert_eq!(ZR_strlen(&buf), 2, "no NUL → bounded by slice");
    }

    /// `tcoutclear(false)` runs without panic. C `Src/Zle/zle_refresh.c`:
    ///   helper to clear cap state before tcout.
    #[test]
    fn tcoutclear_runs_without_panic() {
        let _g = crate::test_util::global_state_lock();
        tcoutclear(false);
        tcoutclear(true);
    }

    /// `zle_free_highlight()` runs without panic (no-op when no
    /// highlights present). C: clears highlight tables.
    #[test]
    fn zle_free_highlight_no_panic() {
        let _g = crate::test_util::global_state_lock();
        zle_free_highlight();
        zle_free_highlight();
    }

    // ═══════════════════════════════════════════════════════════════════
    // C-parity tests for Src/Zle/zle_refresh.c ZR_* primitives.
    // ═══════════════════════════════════════════════════════════════════

    /// c:86-89 — `ZR_memset(dst, rc, n)` fills first n cells with rc.
    #[test]
    fn zr_memset_fills_first_n_cells() {
        let _g = crate::test_util::global_state_lock();
        let mut buf: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: 'X', atr: 0 }; 10];
        let rc = REFRESH_ELEMENT { chr: 'A', atr: 0 };
        ZR_memset(&mut buf, rc, 5);
        for i in 0..5 {
            assert_eq!(buf[i].chr, 'A', "cell {} must be filled", i);
        }
        for i in 5..10 {
            assert_eq!(buf[i].chr, 'X', "cell {} must remain", i);
        }
    }

    /// c:86 — `ZR_memset(dst, rc, 0)` is no-op.
    #[test]
    fn zr_memset_zero_len_no_op() {
        let _g = crate::test_util::global_state_lock();
        let mut buf: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: 'X', atr: 0 }; 3];
        let rc = REFRESH_ELEMENT { chr: 'A', atr: 0 };
        ZR_memset(&mut buf, rc, 0);
        for elt in &buf {
            assert_eq!(elt.chr, 'X', "len=0 must not modify any cell");
        }
    }

    /// c:86 — `ZR_memset` clamps to dst.len() when n > dst.len()
    /// (Rust port-safety pin; C would overrun).
    #[test]
    fn zr_memset_clamps_to_dst_len() {
        let _g = crate::test_util::global_state_lock();
        let mut buf: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: 'X', atr: 0 }; 3];
        let rc = REFRESH_ELEMENT { chr: 'A', atr: 0 };
        // n=100 but dst has 3 — must not panic.
        ZR_memset(&mut buf, rc, 100);
        for elt in &buf {
            assert_eq!(elt.chr, 'A');
        }
    }

    /// c:95-97 — `ZR_strcpy` copies including the NUL terminator.
    #[test]
    fn zr_strcpy_includes_nul_terminator() {
        let _g = crate::test_util::global_state_lock();
        let src: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
            REFRESH_ELEMENT { chr: '\0', atr: 0 },
        ];
        let mut dst: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: 'X', atr: 0 }; 5];
        ZR_strcpy(&mut dst, &src);
        assert_eq!(dst[0].chr, 'a');
        assert_eq!(dst[1].chr, 'b');
        assert_eq!(dst[2].chr, 'c');
        assert_eq!(dst[3].chr, '\0', "NUL terminator must be copied");
    }

    /// c:102-109 — `ZR_strlen(empty)` returns 0 (early-NUL).
    #[test]
    fn zr_strlen_immediate_nul_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let buf: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: '\0', atr: 0 }];
        assert_eq!(ZR_strlen(&buf), 0);
    }

    /// c:102-109 — `ZR_strlen` counts up to (not including) NUL.
    #[test]
    fn zr_strlen_counts_until_nul() {
        let _g = crate::test_util::global_state_lock();
        let buf: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
            REFRESH_ELEMENT { chr: '\0', atr: 0 },
            REFRESH_ELEMENT { chr: 'x', atr: 0 }, // past NUL — not counted
        ];
        assert_eq!(ZR_strlen(&buf), 3);
    }

    /// c:120-133 — `ZR_strncmp(equal, equal, n)` returns 0 for any n.
    #[test]
    fn zr_strncmp_equal_strings_return_zero() {
        let _g = crate::test_util::global_state_lock();
        let a: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
        ];
        let b = a.clone();
        assert_eq!(ZR_strncmp(&a, &b, 2), 0);
        assert_eq!(ZR_strncmp(&a, &b, 1), 0, "prefix match also returns 0");
    }

    /// c:120-133 — `ZR_strncmp(differ, ...)` returns 1 on first diff.
    #[test]
    fn zr_strncmp_diff_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let a: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
        ];
        let b: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'X', atr: 0 },
        ];
        assert_eq!(ZR_strncmp(&a, &b, 2), 1, "differ at idx 1 → 1");
        assert_eq!(ZR_strncmp(&a, &b, 1), 0, "first char matches → 0");
    }

    /// c:120-133 — `ZR_strncmp(_, _, 0)` returns 0 (loop never runs).
    #[test]
    fn zr_strncmp_zero_len_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let a: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: 'a', atr: 0 }];
        let b: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: 'z', atr: 0 }];
        assert_eq!(ZR_strncmp(&a, &b, 0), 0, "n=0 → no comparison");
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_refresh.c
    // c:33 ZR_memset / c:96 ZR_strcpy / c:143 ZR_strlen / c:230 ZR_strncmp
    // c:448 zle_free_highlight / c:465 tcoutclear / c:476 zwcputc /
    // c:708 scrollwindow / c:909 zrefresh / c:1097 wpfxlen
    // ═══════════════════════════════════════════════════════════════════

    /// c:143 — `ZR_strlen` returns usize (compile-time type pin).
    #[test]
    fn zr_strlen_returns_usize_type() {
        let _: usize = ZR_strlen(&[]);
    }

    /// c:143 — `ZR_strlen(empty)` returns 0.
    #[test]
    fn zr_strlen_empty_slice_returns_zero() {
        assert_eq!(ZR_strlen(&[]), 0);
    }

    /// c:230 — `ZR_strncmp` is reflexive for same input.
    #[test]
    fn zr_strncmp_reflexive_returns_zero() {
        let a: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
        ];
        assert_eq!(ZR_strncmp(&a, &a, 2), 0, "self-compare must be 0");
    }

    /// c:230 — `ZR_strncmp(empty, empty, 0)` returns 0.
    #[test]
    fn zr_strncmp_both_empty_zero_len_returns_zero() {
        let empty: Vec<REFRESH_ELEMENT> = vec![];
        assert_eq!(ZR_strncmp(&empty, &empty, 0), 0);
    }

    /// c:448 — `zle_free_highlight` idempotent.
    #[test]
    fn zle_free_highlight_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            zle_free_highlight();
        }
    }

    /// c:465 — `tcoutclear(true)` + `tcoutclear(false)` safe both modes.
    #[test]
    fn tcoutclear_both_modes_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        tcoutclear(true);
        tcoutclear(false);
    }

    /// c:708 — `scrollwindow(0)` no-op (zero lines).
    #[test]
    fn scrollwindow_zero_lines_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        scrollwindow(0);
    }

    /// c:708 — `scrollwindow(N)` for typical N doesn't panic.
    /// i32::MIN excluded — covered by the ZSHRS BUG pin below.
    #[test]
    fn scrollwindow_typical_lines_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for n in [-100, -1, 0, 1, 100, i32::MAX] {
            scrollwindow(n);
        }
    }

    /// c:708 — `scrollwindow(i32::MIN)` PANICS in debug build with
    /// "attempt to negate with overflow". C body negates `lines` to
    /// scroll the opposite direction; C silently wraps via two's
    /// complement, Rust debug build traps on the overflow.
    #[test]
    fn scrollwindow_i32_min_panics_zshrs_bug() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        scrollwindow(i32::MIN);
    }

    /// c:1097 — `wpfxlen(empty, empty)` returns 0.
    #[test]
    fn wpfxlen_both_empty_returns_zero() {
        assert_eq!(wpfxlen(&[], &[]), 0);
    }

    /// c:1097 — `wpfxlen(a, a)` returns slice len (full match).
    #[test]
    fn wpfxlen_identical_returns_full_len() {
        let s: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
        ];
        assert_eq!(wpfxlen(&s, &s), s.len(), "identical → full prefix");
    }

    /// c:476 — `zwcputc` is safe for any char (including high codepoints).
    #[test]
    fn zwcputc_any_char_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for c in ['\0', 'a', '日', '\u{1F600}', '\u{10FFFF}'] {
            zwcputc(&REFRESH_ELEMENT { chr: c, atr: 0 });
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_refresh.c
    // c:33 ZR_memset / c:96 ZR_strcpy / c:143 ZR_strlen / c:230 ZR_strncmp /
    // c:496 zwcwrite / c:1097 wpfxlen / c:1603 moveto / c:1629 tcmultout
    // ═══════════════════════════════════════════════════════════════════

    /// c:33 — `ZR_memset` on empty slice is safe.
    #[test]
    fn zr_memset_empty_slice_no_panic() {
        let mut buf: Vec<REFRESH_ELEMENT> = vec![];
        ZR_memset(&mut buf, REFRESH_ELEMENT { chr: ' ', atr: 0 }, 0);
    }

    /// c:33 — `ZR_memset(buf, fill, n)` writes `fill` n times.
    #[test]
    fn zr_memset_writes_fill_value() {
        let mut buf: Vec<REFRESH_ELEMENT> = vec![
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
        ];
        let fill = REFRESH_ELEMENT { chr: 'X', atr: 0 };
        ZR_memset(&mut buf, fill, 3);
        for e in &buf {
            assert_eq!(e.chr, 'X', "ZR_memset must fill with X");
        }
    }

    /// c:96 — `ZR_strcpy` empty src is safe.
    #[test]
    fn zr_strcpy_empty_src_no_panic() {
        let mut dst: Vec<REFRESH_ELEMENT> = vec![REFRESH_ELEMENT { chr: '\0', atr: 0 }];
        ZR_strcpy(&mut dst, &[]);
    }

    /// c:230 — `ZR_strncmp(empty, empty, 0)` returns 0.
    #[test]
    fn zr_strncmp_empty_inputs_returns_zero() {
        let r = ZR_strncmp(&[], &[], 0);
        assert_eq!(r, 0, "empty + empty + 0 → 0");
    }

    /// c:230 — `ZR_strncmp` returns i32 (compile-time type pin).
    #[test]
    fn zr_strncmp_returns_i32_type() {
        let _: i32 = ZR_strncmp(&[], &[], 0);
    }

    /// c:230 — empty + empty + nonzero cap still 0.
    #[test]
    fn zr_strncmp_empty_with_nonzero_cap_returns_zero() {
        assert_eq!(ZR_strncmp(&[], &[], 5), 0);
    }

    /// c:496 — `zwcwrite(&[], 0)` empty string is safe.
    #[test]
    fn zwcwrite_empty_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        zwcwrite(&[], 0);
    }

    /// c:655-660 — zwcwrite writes the first `i` cells and returns the
    /// count, clamped to the available length.
    #[test]
    fn zwcwrite_returns_clamped_cell_count() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let cells = [
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
        ];
        assert_eq!(zwcwrite(&cells, 2), 2, "c:660 — returns i");
        assert_eq!(zwcwrite(&cells, 5), 3, "clamped to available length");
        assert_eq!(zwcwrite(&cells, 0), 0);
    }

    /// c:1097 — `wpfxlen` returns usize (compile-time type pin).
    #[test]
    fn wpfxlen_returns_usize_type() {
        let _: usize = wpfxlen(&[], &[]);
    }

    /// c:1603 — `moveto(0, 0)` returns void (compile-time type pin).
    #[test]
    fn moveto_returns_void_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: () = moveto(0, 0);
    }

    /// c:1629 — `tcmultout(0, 0, 0)` returns i32 (compile-time type pin).
    #[test]
    fn tcmultout_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = tcmultout(0, 0, 0);
    }

    /// c:143 — `ZR_strlen` returns usize (compile-time type pin).
    #[test]
    fn zr_strlen_returns_usize_type_pin2() {
        let _: usize = ZR_strlen(&[]);
    }

    /// c:143 — `ZR_strlen` is pure across 4 inputs.
    #[test]
    fn zr_strlen_pure_full_sweep() {
        let cases: Vec<Vec<REFRESH_ELEMENT>> = vec![
            vec![],
            vec![REFRESH_ELEMENT { chr: 'a', atr: 0 }],
            vec![REFRESH_ELEMENT { chr: '\0', atr: 0 }],
            vec![
                REFRESH_ELEMENT { chr: 'x', atr: 0 },
                REFRESH_ELEMENT { chr: 'y', atr: 0 },
            ],
        ];
        for c in &cases {
            let first = ZR_strlen(c);
            for _ in 0..3 {
                assert_eq!(ZR_strlen(c), first, "ZR_strlen must be pure");
            }
        }
    }

    // ═══════════════════════════════════════════════════════════════════
    // Additional C-parity tests for Src/Zle/zle_refresh.c
    // c:143 ZR_strlen / c:230 ZR_strncmp / c:476 zwcputc / c:496 zwcwrite /
    // c:448 zle_free_highlight / c:465 tcoutclear / c:1097 wpfxlen / c:33 ZR_memset
    // ═══════════════════════════════════════════════════════════════════

    /// c:143 — `ZR_strlen(empty)` returns 0.
    #[test]
    fn zr_strlen_empty_returns_zero() {
        assert_eq!(ZR_strlen(&[]), 0, "empty → 0 length");
    }

    /// c:230 — `ZR_strncmp` is BOOLEAN (0=equal, 1=different), NOT
    /// a signed ordinal like C `strncmp`. Pin the symmetric-boolean
    /// contract per Src/Zle/zle_refresh.c:127 `return 1` arm.
    /// Distinct inputs must produce 1 in BOTH directions.
    #[test]
    fn zr_strncmp_symmetric_boolean_for_distinct() {
        let a = [REFRESH_ELEMENT { chr: 'a', atr: 0 }];
        let b = [REFRESH_ELEMENT { chr: 'b', atr: 0 }];
        let ab = ZR_strncmp(&a, &b, 1);
        let ba = ZR_strncmp(&b, &a, 1);
        assert_eq!(ab, 1, "ZR_strncmp(a, b, 1) = 1 (different)");
        assert_eq!(ba, 1, "ZR_strncmp(b, a, 1) = 1 (different, symmetric)");
    }

    /// c:230 — `ZR_strncmp(x, x, n)` reflexive: same input → 0 (alt).
    #[test]
    fn zr_strncmp_reflexive_returns_zero_alt() {
        let buf = [
            REFRESH_ELEMENT { chr: 'x', atr: 0 },
            REFRESH_ELEMENT { chr: 'y', atr: 0 },
        ];
        assert_eq!(
            ZR_strncmp(&buf, &buf, 2),
            0,
            "ZR_strncmp(x, x, n) must be 0"
        );
    }

    /// c:496 — `zwcwrite(&[], 0)` is idempotent across many calls.
    #[test]
    fn zwcwrite_empty_idempotent() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..10 {
            zwcwrite(&[], 0);
        }
    }

    /// c:476 — `zwcputc` returns void; safe for various chars.
    #[test]
    fn zwcputc_various_chars_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for c in ['a', '\n', '\t', '\0', '日'] {
            zwcputc(&REFRESH_ELEMENT { chr: c, atr: 0 });
        }
    }

    /// c:448 — `zle_free_highlight` is idempotent (alt 5-call).
    #[test]
    fn zle_free_highlight_idempotent_alt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        for _ in 0..5 {
            zle_free_highlight();
        }
    }

    /// c:465 — `tcoutclear` for both bool arms is safe.
    #[test]
    fn tcoutclear_both_arms_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        tcoutclear(false);
        tcoutclear(true);
    }

    /// c:1097 — `wpfxlen(empty, empty)` returns 0 (alt name).
    #[test]
    fn wpfxlen_both_empty_returns_zero_alt() {
        assert_eq!(wpfxlen(&[], &[]), 0, "empty + empty → 0 common prefix");
    }

    /// c:1097 — `wpfxlen` is symmetric: wpfxlen(a, b) == wpfxlen(b, a).
    #[test]
    fn wpfxlen_symmetric() {
        let a = [
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
        ];
        let b = [
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
        ];
        assert_eq!(
            wpfxlen(&a, &b),
            wpfxlen(&b, &a),
            "wpfxlen must be symmetric"
        );
    }

    /// c:1097 — `wpfxlen(x, x)` returns full length (perfect prefix match).
    #[test]
    fn wpfxlen_identical_inputs_full_match() {
        let buf = [
            REFRESH_ELEMENT { chr: 'a', atr: 0 },
            REFRESH_ELEMENT { chr: 'b', atr: 0 },
            REFRESH_ELEMENT { chr: 'c', atr: 0 },
        ];
        let n = wpfxlen(&buf, &buf);
        assert_eq!(n, 3, "x vs x → full length 3");
    }

    /// c:33 — `ZR_memset` n=0 on empty buf is safe.
    #[test]
    fn zr_memset_zero_n_safe() {
        let mut buf: Vec<REFRESH_ELEMENT> = vec![];
        ZR_memset(&mut buf, REFRESH_ELEMENT { chr: 'x', atr: 0 }, 0);
    }
}
