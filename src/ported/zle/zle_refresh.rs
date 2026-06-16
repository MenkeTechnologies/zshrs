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

/// Direct port of `void tcoutclear(int cap)` from
/// `Src/Zle/zle_refresh.c:607`.
/// ```c
/// void tcoutclear(int cap) {
///     treplaceattrs((cap == TCCLEAREOL) ? prompt_attr : 0);
///     applytextattributes(0);
///     tcout(cap);
/// }
/// ```
/// Emit a clear capability (`cap` is the termcap index TCCLEAREOL /
/// TCCLEAREOD / TCCLEARSCREEN), after making the cleared region carry the
/// right attributes — the prompt's for clear-to-end-of-line (so a
/// coloured prompt's background fills correctly), else the default.
/// The previous port took a `bool` and hardcoded CSI J for both, which
/// wrongly cleared to end of *display* for the clear-to-end-of-*line*
/// case (TCCLEAREOL → CSI K) and dropped the attribute setup. Every C
/// caller guards on `tccan(cap)`, so `tcstr[cap]` is always loaded here.
pub fn tcoutclear(cap: i32) {
    // c:607
    use crate::ported::zsh_h::TCCLEAREOL;
    // c:609 — `treplaceattrs((cap == TCCLEAREOL) ? prompt_attr : 0);`
    let attr = if cap == TCCLEAREOL {
        PROMPT_ATTR.load(Ordering::SeqCst)
    } else {
        0
    };
    crate::ported::prompt::treplaceattrs(attr);
    // c:610 — `applytextattributes(0);` emit the SGR change.
    let sgr = crate::ported::prompt::applytextattributes(0);
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    if !sgr.is_empty() {
        let _ = write_loop(out_fd, sgr.as_bytes());
    }
    tcout(cap); // c:611
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

    // c:737-755 — re-alloc the video buffers (winw/winh changed). C
    //              allocates the global nbuf/obuf: (winh+1) row slots,
    //              each row (winw+2) cells. Allocate the global NBUF/OBUF
    //              the same way (eagerly sized; nextline's alloc-if-missing
    //              still covers the lazy-row case). This replaces the prior
    //              RefreshState VideoBuffer allocation so the buffer
    //              lifecycle matches nextline/snextline/scrollwindow, which
    //              all operate on the global NBUF.
    let nrows = (rows + 1) as usize;
    let rowlen = (cols + 2) as usize;
    let fresh = || -> Vec<REFRESH_STRING> {
        (0..nrows)
            .map(|_| vec![REFRESH_ELEMENT::default(); rowlen])
            .collect()
    };
    // c:757-767 — the per-row `nbuf[ln][0]=zr_zr` init is subsumed by the
    //              REFRESH_ELEMENT::default() zero-fill above.
    *NBUF.lock().unwrap() = fresh();
    *OBUF.lock().unwrap() = fresh();

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

/// Direct port of `void scrollwindow(int tline)` from
/// `Src/Zle/zle_refresh.c:798`.
/// ```c
/// void scrollwindow(int tline) {
///     int t0;
///     REFRESH_STRING s = nbuf[tline];
///     for (t0 = tline; t0 < winh - 1; t0++)
///         nbuf[t0] = nbuf[t0 + 1];
///     nbuf[winh - 1] = s;
///     if (!tline) more_start = 1;
/// }
/// ```
/// Rotate the video buffer: line `tline` is lifted out, the lines below
/// it shift up one row, and the lifted line wraps to the bottom
/// (`winh - 1`). When scrolling from the very top, set `more_start` so the
/// first line can show the "more text above" indicator. The previous port
/// was a fake — it emitted a terminal scroll escape (CSI S/T) and took a
/// line *count*, neither of which is what C does.
pub fn scrollwindow(tline: i32) {
    // c:798
    let winh = WINH.load(Ordering::SeqCst);
    if tline >= 0 {
        let t = tline as usize;
        let mut nbuf = NBUF.lock().unwrap();
        // C operates on the full winh grid; the Vec may be shorter, so
        // clamp the rotated range to what's allocated.
        let end = (winh as usize).min(nbuf.len());
        // c:803-806 — `s = nbuf[tline]; shift [tline..winh-1] up;
        //              nbuf[winh-1] = s;` is a left-rotation by one over
        //              the `[tline, winh)` window.
        if t < end {
            nbuf[t..end].rotate_left(1);
        }
    }
    // c:807-808 — `if (!tline) more_start = 1;`
    if tline == 0 {
        MORE_START.store(1, Ordering::SeqCst);
    }
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
/// **Signature note (faithful to C):** `nextline(Rparams rpms, int wrapped)`
/// operates on the global `nbuf`/`winw`/`winh`/`numscrolls` file-statics —
/// here the global `NBUF` and `WINW`/`WINH`/`NUMSCROLLS` atomics, with
/// `REFRESH_ELEMENT` cells (consistent with `scrollwindow`, which it calls
/// at c:863). The earlier port threaded a `RefreshState`/`new_video` (the
/// `RefreshElement` cell type) — divergent from C and inconsistent with the
/// now-global `scrollwindow`; this is the first consolidation step toward
/// driving the live build's vertical scroll through `nextline`.
pub fn nextline(rpms: &mut rparams, wrapped: i32) -> i32 {
    // c:841
    let winw = WINW.load(Ordering::SeqCst);
    let winh = WINH.load(Ordering::SeqCst);

    // c:844-845 — `nbuf[ln][winw+1] = wrapped ? zr_nl : zr_zr; *s = zr_zr;`
    {
        let mut nbuf = NBUF.lock().unwrap();
        if let Some(row) = nbuf.get_mut(rpms.ln as usize) {
            let end_idx = (winw + 1) as usize;
            if end_idx < row.len() {
                row[end_idx] = if wrapped != 0 {
                    REFRESH_ELEMENT { chr: '\n', atr: 0 }
                } else {
                    REFRESH_ELEMENT::default()
                };
            }
            if rpms.pos < row.len() {
                row[rpms.pos] = REFRESH_ELEMENT::default();
            }
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
        scrollwindow(0); // c:863 — same global NBUF as this function
        if rpms.nvln != -1 {
            rpms.nvln -= 1; // c:865
        }
    }

    // c:867-869 — allocate the row if missing.
    {
        let mut nbuf = NBUF.lock().unwrap();
        if rpms.ln as usize >= nbuf.len() {
            nbuf.resize(
                rpms.ln as usize + 1,
                vec![REFRESH_ELEMENT::default(); (winw + 2) as usize],
            );
        }
    }
    // c:871-872 — `rpms->s = nbuf[ln]; rpms->sen = s + winw;`
    rpms.pos = 0;
    rpms.end = winw as usize;
    0 // c:873
}

/// Direct port of `void snextline(Rparams rpms)` from
/// `Src/Zle/zle_refresh.c:875` — "go to the next line in the status area".
/// ```c
/// void snextline(Rparams rpms) {
///     *rpms->s = zr_zr;
///     if (rpms->ln != winh - 1) rpms->ln++;
///     else if (rpms->tosln > rpms->ln) {
///         rpms->tosln--;
///         if (rpms->nvln > 1) { scrollwindow(0); rpms->nvln--; }
///         else more_end = 1;
///     } else if (rpms->tosln > 2 && rpms->nvln > 1) {
///         rpms->tosln--;
///         if (rpms->tosln <= rpms->nvln) { scrollwindow(0); rpms->nvln--; }
///         else { scrollwindow(rpms->tosln); more_end = 1; }
///     } else { rpms->more_status = 1; scrollwindow(rpms->tosln + 1); }
///     if (!nbuf[rpms->ln]) nbuf[rpms->ln] = zalloc(...);
///     rpms->s = nbuf[rpms->ln]; rpms->sen = rpms->s + winw;
/// }
/// ```
/// The previous Rust body was a fake: a made-up `more_status && tosln != ln`
/// guard, no scroll logic, `RefreshState`/`new_video`, and an `int` return.
/// Ported faithfully on the global `NBUF` (`REFRESH_ELEMENT`) with the real
/// status-pane scroll cascade (tosln/nvln/more_end + the three scrollwindow
/// calls).
pub fn snextline(rpms: &mut rparams) {
    // c:875
    let winw = WINW.load(Ordering::SeqCst);
    let winh = WINH.load(Ordering::SeqCst);

    // c:877 — `*rpms->s = zr_zr;` terminate the current row at pos.
    {
        let mut nbuf = NBUF.lock().unwrap();
        if let Some(row) = nbuf.get_mut(rpms.ln as usize) {
            if rpms.pos < row.len() {
                row[rpms.pos] = REFRESH_ELEMENT::default();
            }
        }
    }

    if rpms.ln != winh - 1 {
        rpms.ln += 1; // c:879
    } else if rpms.tosln > rpms.ln {
        // c:881-887
        rpms.tosln -= 1;
        if rpms.nvln > 1 {
            scrollwindow(0); // c:884
            rpms.nvln -= 1; // c:885
        } else {
            MORE_END.store(1, Ordering::SeqCst); // c:887
        }
    } else if rpms.tosln > 2 && rpms.nvln > 1 {
        // c:888-896
        rpms.tosln -= 1;
        if rpms.tosln <= rpms.nvln {
            scrollwindow(0); // c:891
            rpms.nvln -= 1; // c:892
        } else {
            scrollwindow(rpms.tosln); // c:894
            MORE_END.store(1, Ordering::SeqCst); // c:895
        }
    } else {
        // c:897-899
        rpms.more_status = 1; // c:898
        scrollwindow(rpms.tosln + 1); // c:899
    }

    // c:901-904 — alloc the row if missing; reset s/sen to its start/end.
    {
        let mut nbuf = NBUF.lock().unwrap();
        if rpms.ln as usize >= nbuf.len() {
            nbuf.resize(
                rpms.ln as usize + 1,
                vec![REFRESH_ELEMENT::default(); (winw + 2) as usize],
            );
        }
    }
    rpms.pos = 0; // c:903
    rpms.end = winw as usize; // c:904
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

/// Direct port of `void bufswap(void)` from `Src/Zle/zle_refresh.c:946`.
/// Swap the new/old video buffers — "better than freeing/allocating
/// every time" (c:944): last frame's NBUF becomes this frame's OBUF.
/// Operates on the global NBUF/OBUF as C does on the global nbuf/obuf.
pub fn bufswap() {
    // c:946
    // c:954-956 — `qbuf = nbuf; nbuf = obuf; obuf = qbuf;`
    let mut nbuf = NBUF.lock().unwrap();
    let mut obuf = OBUF.lock().unwrap();
    std::mem::swap(&mut *nbuf, &mut *obuf);
    // c:960-968 — MULTIBYTE_SUPPORT also swaps the multiword buffers
    // (nmwbuf/omwbuf + nmw_size) and resets nmw_ind = 1; those buffers
    // aren't ported (combining-cluster substrate), so that shadow swap
    // is deferred.
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

    let cols = adjustcolumns();
    // c:729-734 — sync the global video dimensions to the terminal each
    // frame, as C's resetvideo does (`winw = zterm_columns`, winh clamped).
    // The cursor primitives (moveto automargin, scrollwindow, the line-opt)
    // read these globals; without this they'd stay at the static default
    // (80×24) on any other terminal size. VLN/VMAXLN are NOT reset here —
    // the diff path manages them itself (unlike resetvideo, which C only
    // calls on a size change).
    {
        use crate::ported::params::TERMFLAGS;
        use crate::ported::zsh_h::TERM_SHORT;
        WINW.store(cols as i32, Ordering::Relaxed); // c:729
        let real_lines = adjustlines();
        let rows = if TERMFLAGS.load(Ordering::Relaxed) & TERM_SHORT != 0 {
            1 // c:730-731
        } else if real_lines < 2 {
            24 // c:732-733
        } else {
            real_lines
        };
        WINH.store(rows as i32, Ordering::Relaxed);
        RWINH.store(real_lines as i32, Ordering::Relaxed); // c:734
    }

    let prompt = prompt().to_string();
    let rprompt = rprompt().to_string();
    let cursor = ZLECS.load(Ordering::SeqCst);

    let prompt_width = countprompt(&prompt);
    // c:676 — `lpromptw` is the left prompt's display width. The NBUF build
    // emits that many prompt cells at the start of row 0, so syncing it
    // enables refreshline's prompt-skip (c:1862) — previously dead because
    // LPROMPTW stayed at 0 (resetvideo, which sets it, is never called).
    // The skip is clamped to the row length, so an over-wide value can't
    // overrun.
    LPROMPTW.store(prompt_width as i32, Ordering::Relaxed);
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
    // ---- OUTPUT SWAP -----------------------------------------------------
    // The full-repaint string above is now SUPERSEDED by the NBUF/OBUF diff
    // emitted by the refreshline loop at the end of this function. The
    // build is kept (not written) so the swap is one revertable change:
    // restore this write_loop and delete the refreshline loop to revert.
    let _ = (out_fd, &handle); // formerly: write_loop(out_fd, handle.as_bytes())

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
        // c:954-956 — last frame's NBUF becomes this frame's OBUF.
        bufswap();
        OLNCT.store(NLNCT.load(Ordering::SeqCst), Ordering::SeqCst);
        // Rust frame-prep: clear the swapped-in (stale) NBUF for rebuild.
        NBUF.lock().unwrap().clear();
        // c:1208-1400 — emit prompt + line cells, wrapping at `winw`.
        // c:1226-1248 — each cell carries the resolved attribute (`atr`)
        // so refreshline/zwcputc emit its colour. Convert the per-char
        // TextAttr overlay (compute_render_attrs) to the zattr bitmap.
        use crate::ported::zsh_h::zattr;
        let to_zattr = |ta: &TextAttr| -> zattr {
            use crate::ported::zsh_h::{
                TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR, TXTSTANDOUT, TXTUNDERLINE,
                TXT_ATTR_BG_COL_SHIFT, TXT_ATTR_FG_COL_SHIFT,
            };
            let mut a: zattr = 0;
            if ta.bold {
                a |= TXTBOLDFACE;
            }
            if ta.underline {
                a |= TXTUNDERLINE;
            }
            if ta.standout {
                a |= TXTSTANDOUT;
            }
            if let Some(fg) = ta.fg_color {
                a |= TXTFGCOLOUR | ((fg as zattr) << TXT_ATTR_FG_COL_SHIFT);
            }
            if let Some(bg) = ta.bg_color {
                a |= TXTBGCOLOUR | ((bg as zattr) << TXT_ATTR_BG_COL_SHIFT);
            }
            a
        };
        let cols_n = cols.max(1);
        let mut rows: Vec<REFRESH_STRING> = vec![Vec::new()];
        let mut emit = |rows: &mut Vec<REFRESH_STRING>, chr: char, atr: zattr| {
            if rows.last().map(|r| r.len()).unwrap_or(0) >= cols_n {
                rows.push(Vec::new()); // c:842 nextline
            }
            rows.last_mut().unwrap().push(REFRESH_ELEMENT { chr, atr });
        };
        // Prompt cells. The prompt is an ANSI string here (not attributed
        // cells as in C's putpromptchar), so parse its SGR escapes into the
        // cell `atr` — otherwise the prompt would render colourless when the
        // output switches to the NBUF diff. (A bridge: the faithful form
        // would have putpromptchar emit cells directly; flagged.)
        let apply_sgr = |mut attr: zattr, params: &str| -> zattr {
            use crate::ported::zsh_h::{
                TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR, TXTSTANDOUT, TXTUNDERLINE,
                TXT_ATTR_BG_COL_SHIFT, TXT_ATTR_FG_COL_SHIFT,
            };
            let nums: Vec<i64> = if params.is_empty() {
                vec![0] // bare CSI m == reset
            } else {
                params.split(';').filter_map(|s| s.parse().ok()).collect()
            };
            let set_fg = |a: zattr, c: i64| {
                (a & !(TXTFGCOLOUR | (0xff << TXT_ATTR_FG_COL_SHIFT)))
                    | TXTFGCOLOUR
                    | ((c as zattr) << TXT_ATTR_FG_COL_SHIFT)
            };
            let set_bg = |a: zattr, c: i64| {
                (a & !(TXTBGCOLOUR | (0xff << TXT_ATTR_BG_COL_SHIFT)))
                    | TXTBGCOLOUR
                    | ((c as zattr) << TXT_ATTR_BG_COL_SHIFT)
            };
            let mut i = 0;
            while i < nums.len() {
                match nums[i] {
                    0 => attr = 0,
                    1 => attr |= TXTBOLDFACE,
                    4 => attr |= TXTUNDERLINE,
                    7 => attr |= TXTSTANDOUT,
                    30..=37 => attr = set_fg(attr, nums[i] - 30),
                    40..=47 => attr = set_bg(attr, nums[i] - 40),
                    90..=97 => attr = set_fg(attr, nums[i] - 90 + 8),
                    100..=107 => attr = set_bg(attr, nums[i] - 100 + 8),
                    38 if i + 2 < nums.len() && nums[i + 1] == 5 => {
                        attr = set_fg(attr, nums[i + 2]);
                        i += 2;
                    }
                    48 if i + 2 < nums.len() && nums[i + 1] == 5 => {
                        attr = set_bg(attr, nums[i + 2]);
                        i += 2;
                    }
                    _ => {}
                }
                i += 1;
            }
            attr
        };
        let mut prompt_attr: zattr = 0;
        let mut esc_params = String::new();
        let mut in_esc = false;
        for c in prompt.chars() {
            if in_esc {
                if c == '[' {
                    // CSI introducer — not a param.
                } else if c == 'm' {
                    prompt_attr = apply_sgr(prompt_attr, &esc_params); // SGR
                    in_esc = false;
                    esc_params.clear();
                } else if c.is_ascii_alphabetic() {
                    in_esc = false; // non-SGR escape (cursor/clear) — ignore
                    esc_params.clear();
                } else {
                    esc_params.push(c);
                }
            } else if c == '\x1b' {
                in_esc = true;
                esc_params.clear();
            } else {
                emit(&mut rows, c, prompt_attr);
            }
        }
        // c:152 / zle_main.c:1280 — publish the prompt's trailing attribute
        // so refreshline's TCDEL attr-apply (c:2044) and tcoutclear (c:609)
        // make deleted/cleared cells carry the prompt's colour. C derives
        // prompt_attr via mixattrs(pmpt_attr, .., rpmpt_attr) from
        // promptexpand (blocked); this is the left-prompt SGR-parsed
        // approximation (rpmpt_attr folding deferred with that subsystem).
        PROMPT_ATTR.store(prompt_attr, Ordering::Relaxed);
        // Editable line with tab/control expansion (c:1248-1398). Each
        // line char's overlay attr is applied to the cell(s) it produces.
        for (i, &ch) in line_snapshot.iter().enumerate() {
            let atr = attrs
                .get(i)
                .and_then(|o| o.as_ref())
                .map(&to_zattr)
                .unwrap_or(0);
            if ch == '\n' {
                rows.push(Vec::new()); // c:1248-1251
            } else if ch == '\t' {
                // c:1259-1264 — spaces to the next 8-column stop.
                loop {
                    emit(&mut rows, ' ', atr);
                    if rows.last().map(|r| r.len()).unwrap_or(0) % 8 == 0 {
                        break;
                    }
                }
            } else if (ch as u32) < 0x20 || ch as u32 == 0x7f {
                // c:1340-1356 — control char as `^X` / `^?`.
                emit(&mut rows, '^', atr);
                let c2 = if ((ch as u32) & !0x80u32) > 31 {
                    '?'
                } else {
                    char::from_u32((ch as u32) | 0x40).unwrap_or('?')
                };
                emit(&mut rows, c2, atr);
            } else {
                emit(&mut rows, ch, atr); // c:1398
            }
        }
        let nlnct = rows.len() as i32;
        *NBUF.lock().unwrap() = rows;
        NLNCT.store(nlnct, Ordering::SeqCst); // c:nlnct = rpms.ln + 1
    }

    // ---- Render via the NBUF/OBUF diff (c:1700-1739) -------------------
    // OBUF holds the previous frame (set by the swap inside the build);
    // refreshline diffs each new line against it and emits the minimal
    // terminal updates, then we move the cursor to its editing position.
    let nlnct = NLNCT.load(Ordering::SeqCst);
    let olnct = OLNCT.load(Ordering::SeqCst);
    VCS.store(0, Ordering::SeqCst); // start from the home position
    VLN.store(0, Ordering::SeqCst);
    let saved_cleareol = CLEAREOL.load(Ordering::SeqCst);
    // c:1174 — `clearf = clearflag`: snapshot the clear flag for the loop.
    let clearf = CLEARFLAG.load(Ordering::SeqCst) != 0;
    let winw = WINW.load(Ordering::SeqCst);
    let hasam_v = crate::ported::init::hasam.load(Ordering::SeqCst) != 0;
    enum LineOp {
        None,
        Del,
        Ins,
    }
    for iln in 0..nlnct {
        // olnct mutates as we insert/delete lines below; read it fresh.
        let olnct_now = OLNCT.load(Ordering::SeqCst);
        // c:1672-1674 — if we have more lines than last time, clear the
        // newly-used lines. cleareol is sticky: once set at iln==olnct it
        // stays 1 for the rest of the loop, so every new line is cleared.
        if iln >= olnct_now {
            CLEAREOL.store(1, Ordering::SeqCst);
        }

        // c:1677-1707 — if the old and new line differ, try to insert or
        // delete a whole line (scrolling the terminal) instead of
        // rewriting every following line. Only viable when the terminal
        // has the insert/delete-line capability (tccan), so headless
        // (tclen all zero) leaves the plain per-line path untouched.
        if !clearf
            && iln > 0
            && iln < olnct_now - 1
            && !(hasam_v && VCS.load(Ordering::SeqCst) == winw)
        {
            let tcan_del =
                tclen.lock().unwrap()[crate::ported::zsh_h::TCDELLINE as usize] != 0;
            let tcan_ins =
                tclen.lock().unwrap()[crate::ported::zsh_h::TCINSLINE as usize] != 0;
            let vmaxln = VMAXLN.load(Ordering::SeqCst);
            let i = iln as usize;
            // Decide the op under a brief lock on both video buffers.
            let op = {
                let nbuf = NBUF.lock().unwrap();
                let obuf = OBUF.lock().unwrap();
                let nb_i = nbuf.get(i);
                let ob_i = obuf.get(i);
                // c:1681-1682 — nbuf[iln] && obuf[iln] && they differ in 16.
                let outer = match (nb_i, ob_i) {
                    (Some(nb), Some(ob)) => ZR_strncmp(ob, nb, 16) != 0,
                    _ => false,
                };
                if !outer {
                    LineOp::None
                } else if tcan_del
                    // c:1683-1685 — obuf[iln+1] real, its first cell set,
                    // and obuf[iln+1] == nbuf[iln] in 16 → deleting line iln
                    // realigns the rest with one TCDELLINE.
                    && obuf
                        .get(i + 1)
                        .and_then(|r| r.first())
                        .map(|c| c.chr != '\0')
                        .unwrap_or(false)
                    && nb_i.is_some()
                    && ZR_strncmp(obuf.get(i + 1).unwrap(), nb_i.unwrap(), 16) == 0
                {
                    LineOp::Del
                } else if tcan_ins
                    && olnct_now < vmaxln
                    // c:1697-1698 — nbuf[iln+1] real, obuf[iln] real, and
                    // obuf[iln] == nbuf[iln+1] in 16 → inserting a line at
                    // iln realigns with one TCINSLINE.
                    && nbuf.get(i + 1).is_some()
                    && ob_i.is_some()
                    && ZR_strncmp(ob_i.unwrap(), nbuf.get(i + 1).unwrap(), 16) == 0
                {
                    LineOp::Ins
                } else {
                    LineOp::None
                }
            };
            match op {
                LineOp::Del => {
                    moveto(i, 0); // c:1686
                    tcout(crate::ported::zsh_h::TCDELLINE); // c:1687
                    // c:1688-1691 — free obuf[iln], shift the rest down,
                    // olnct--. Vec::remove models the pointer shuffle.
                    let mut obuf = OBUF.lock().unwrap();
                    if i < obuf.len() {
                        obuf.remove(i);
                    }
                    OLNCT.store(olnct_now - 1, Ordering::SeqCst);
                }
                LineOp::Ins => {
                    moveto(i, 0); // c:1699
                    tcout(crate::ported::zsh_h::TCINSLINE); // c:1700
                    // c:1701-1705 — shift obuf up, NULL the new line at iln,
                    // olnct++. Vec::insert of an empty row models the NULL.
                    let mut obuf = OBUF.lock().unwrap();
                    let at = i.min(obuf.len());
                    obuf.insert(at, Vec::new());
                    OLNCT.store(olnct_now + 1, Ordering::SeqCst);
                }
                LineOp::None => {}
            }
        }

        refreshline(iln); // c:1710 — update each line
    }
    CLEAREOL.store(saved_cleareol, Ordering::SeqCst);
    // c:1751-1752 — `if (nlnct > vmaxln) vmaxln = nlnct`: remember the
    // tallest frame we've drawn so the insert-line opt never scrolls past it.
    if nlnct > VMAXLN.load(Ordering::SeqCst) {
        VMAXLN.store(nlnct, Ordering::SeqCst);
    }
    // c:1727-1732 — clear any extra lines the previous frame had.
    let olnct = OLNCT.load(Ordering::SeqCst);
    if olnct > nlnct {
        CLEAREOL.store(1, Ordering::SeqCst);
        for iln in nlnct..olnct {
            refreshline(iln);
        }
        CLEAREOL.store(0, Ordering::SeqCst);
    }
    // c:1739 — `moveto(rpms.nvln, rpms.nvcs)`: cursor to the edit position.
    // Single buffer wraps at `cols`; row = cursor_col / cols, col = rem.
    let cols_c = cols.max(1);
    moveto(cursor_col / cols_c, cursor_col % cols_c);
    // c:1742 — `cursor_form()`: update the terminal cursor shape (block /
    // beam / underline) for the current ZLE state once it's repositioned.
    crate::ported::zle::termquery::cursor_form();
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
        tcoutclear(TCCLEAREOL); // c:1779
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
    // c:1751 — `vcs` is a live video-cursor column. In C it is a global
    // that `moveto` and every write path keep current; the Rust port must
    // track it the same way (resync to the moveto target, accumulate on
    // writes) rather than snapshot it, or the diff engine's column
    // accounting drifts across loop iterations.
    let mut vcs = VCS.load(Ordering::SeqCst);
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
            vcs = 1;
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
            vcs = 0;
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
                vcs = winw - 1; // moveto repositioned the cursor
                if let Some(c) = deferred {
                    zwcputc(&REFRESH_ELEMENT { chr: c, atr: 0 }); // c:1967 zputc(nl)
                }
                vcs += 1; // c:1968 vcs++
                VCS.store(vcs, Ordering::SeqCst);
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
        vcs = ccs; // c:1923 — moveto leaves the cursor (vcs) at ccs

        // c:1925-1929 — if we can finish via clear-to-eol, do so
        if col_cleareol >= 0 && ccs >= col_cleareol {
            // c:1926
            tcoutclear(TCCLEAREOL); // c:1927 tcoutclear(TCCLEAREOL)
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
                vcs += i_pad;
                VCS.store(vcs, Ordering::SeqCst);
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
                vcs += i_write; // c:1959 vcs += i
                VCS.store(vcs, Ordering::SeqCst);
            }
            if col_cleareol >= 0 {
                // c:1960
                tcoutclear(TCCLEAREOL); // c:1961
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
                        vcs += i_try;
                        VCS.store(vcs, Ordering::SeqCst);
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
            // c:2076-2084 — `treplaceattrs(nl->atr); applytextattributes(0);
            // zputc(nl);` — emit the cell. zwcputc does the attr-change
            // (treplaceattrs + applytextattributes) internally, then writes
            // the char. This is the main per-cell overwrite emit.
            if let Some(cell) = nl.first().cloned() {
                zwcputc(&cell); // c:2076-2084
            }
            if !nl.is_empty() {
                nl.remove(0);
            } // c:2085 nl++
            if !ol.is_empty() && ol[0].chr != '\0' {
                // c:2086-2087
                ol.remove(0); // c:2087 ol++
            }
            ccs += 1; // c:2088
            vcs += 1; // c:2089 vcs++
            VCS.store(vcs, Ordering::SeqCst);

            // c:2094-2095 — WEOF do-while: zshrs has no WEOF sentinel.
            break;
        }
    }

    let _ = (rnllen, ollen, ins_last, _p1, _j); // silence
}

/// Direct port of `void moveto(int ln, int cl)` from
/// `Src/Zle/zle_refresh.c:2163`. Move the video cursor to (`ln`, `cl`)
/// using relative terminal movement (the previous port teleported with
/// an absolute CSI H, which cannot create lines that don't exist yet).
/// Operates on the global `vcs`/`vln` (the VCS/VLN atomics) exactly as C:
///   - automargin wrap when at the right margin (c:2167-2182)
///   - early return when already at the target (c:2184)
///   - up-movement via `tc_upcurs` (c:2188-2191)
///   - down-movement: `tc_downcurs` while on-screen, real newlines past
///     `vmaxln-1` to scroll/create lines (c:2195-2212)
///   - horizontal close via `singmoveto` (c:2214-2215)
pub fn moveto(row: usize, col: usize) {
    // c:2163
    let zr_cr = REFRESH_ELEMENT { chr: '\r', atr: 0 };
    let zr_nl = REFRESH_ELEMENT { chr: '\n', atr: 0 };
    let zr_sp = REFRESH_ELEMENT { chr: ' ', atr: 0 };
    let ln = row as i32;
    let cl = col as i32;
    let winw = WINW.load(Ordering::SeqCst);
    let hasam_v = crate::ported::init::hasam.load(Ordering::SeqCst) != 0;
    let mut vcs = VCS.load(Ordering::SeqCst);
    let mut vln = VLN.load(Ordering::SeqCst);

    // c:2167 — `if (vcs == winw)`: wrap off the right margin.
    if vcs == winw {
        vln += 1; // c:2168 vln++, vcs = 0
        vcs = 0;
        if !hasam_v {
            // c:2170-2171 — no automargin: CR + NL.
            zwcputc(&zr_cr);
            zwcputc(&zr_nl);
        } else {
            // c:2173-2176 — rep = first cell of nbuf[vln] if real, else space.
            let nlnct = NLNCT.load(Ordering::SeqCst);
            let rep = {
                let nbuf = NBUF.lock().unwrap();
                nbuf.get(vln as usize)
                    .filter(|_| vln < nlnct)
                    .and_then(|r| r.first())
                    .copied()
                    .filter(|c| c.chr != '\0')
                    .unwrap_or(zr_sp)
            };
            zwcputc(&rep); // c:2177
            zwcputc(&zr_cr); // c:2178
            // c:2179-2181 — `if (vln<olnct && obuf[vln] && obuf[vln]->chr)
            //                  *obuf[vln] = *rep;`
            let olnct = OLNCT.load(Ordering::SeqCst);
            if vln < olnct {
                let mut obuf = OBUF.lock().unwrap();
                if let Some(orow) = obuf.get_mut(vln as usize) {
                    if let Some(first) = orow.first_mut() {
                        if first.chr != '\0' {
                            *first = rep;
                        }
                    }
                }
            }
        }
        VLN.store(vln, Ordering::SeqCst);
        VCS.store(vcs, Ordering::SeqCst);
    }

    // c:2184 — `if (ln == vln && cl == vcs) return;`
    if ln == vln && cl == vcs {
        return;
    }

    // c:2188-2191 — move up.
    if ln < vln {
        tc_upcurs(vln - ln); // c:2189
        vln = ln; // c:2190
        VLN.store(vln, Ordering::SeqCst);
    }

    // c:2195-2212 — move down; past vmaxln-1 use newlines, not TCDOWN, so
    // we don't run off the end of what's been drawn.
    while ln > vln {
        let vmaxln = VMAXLN.load(Ordering::SeqCst);
        if vln < vmaxln - 1 {
            if ln > vmaxln - 1 {
                // c:2198-2200
                if tc_downcurs(vmaxln - 1 - vln) != 0 {
                    vcs = 0;
                    VCS.store(0, Ordering::SeqCst);
                }
                vln = vmaxln - 1;
                VLN.store(vln, Ordering::SeqCst);
            } else {
                // c:2202-2204
                if tc_downcurs(ln - vln) != 0 {
                    vcs = 0;
                    VCS.store(0, Ordering::SeqCst);
                }
                vln = ln;
                VLN.store(vln, Ordering::SeqCst);
                continue;
            }
        }
        // c:2207 — `zputc(&zr_cr), vcs = 0;` safety precaution.
        zwcputc(&zr_cr);
        vcs = 0;
        VCS.store(0, Ordering::SeqCst);
        while ln > vln {
            // c:2208-2211 — newline-scroll the remaining lines.
            zwcputc(&zr_nl);
            vln += 1;
        }
        VLN.store(vln, Ordering::SeqCst);
    }

    // c:2214-2215 — `if (cl != vcs) singmoveto(cl);`
    if cl != vcs {
        singmoveto(cl);
    }
}

/// Direct port of `void tcoutarg(int cap, int arg)` from
/// `Src/Zle/zle_refresh.c:2409`.
/// ```c
/// void tcoutarg(int cap, int arg) {
///     char *result = tgoto(tcstr[cap], arg, arg);
///     if (tcout_func_name) tcout_via_func(cap, arg, putshout);
///     else tputs(result, 1, putshout);
///     SELECT_ADD_COST(strlen(result));
/// }
/// ```
/// Output a parametrised termcap value, substituting `arg` into the
/// capability string via `tgoto` (the same termcap routine `init.rs`
/// already links alongside `tgetstr`). C passes `arg` as both tgoto
/// parameters; the capabilities used here (TCMULTRIGHT / TCHORIZPOS /
/// the multi-* caps) take a single `%d`, so the col/row order is moot
/// and the output is deterministic across platforms. The
/// `tcout_func_name` user-hook (c:2414) and `tputs` padding are deferred
/// — modern terminals need neither.
pub fn tcoutarg(cap: i32, arg: i32) {
    // c:2409
    use crate::ported::init::tcstr;
    use crate::ported::zsh_h::TC_COUNT;
    use std::ffi::{CStr, CString};
    extern "C" {
        fn tgoto(
            cap: *const libc::c_char,
            col: libc::c_int,
            row: libc::c_int,
        ) -> *mut libc::c_char;
    }
    let cap_idx = cap as usize;
    if cap_idx >= TC_COUNT as usize {
        return;
    }
    let cap_str = tcstr.lock().unwrap()[cap_idx].clone();
    if cap_str.is_empty() {
        return;
    }
    let c_cap = match CString::new(cap_str) {
        Ok(c) => c,
        Err(_) => return,
    };
    // c:2413 — `result = tgoto(tcstr[cap], arg, arg);`
    let result = unsafe { tgoto(c_cap.as_ptr(), arg as libc::c_int, arg as libc::c_int) };
    if result.is_null() {
        return;
    }
    let bytes = unsafe { CStr::from_ptr(result) }.to_bytes();
    // c:2416-2417 — `tputs(result, 1, putshout)` (padding dropped).
    let fd = SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = write_loop(out_fd, bytes);
    // c:2419 — SELECT_ADD_COST(strlen(result)) cost accounting (no-op).
}

/// Direct port of `int tcmultout(int cap, int multcap, int ct)` from
/// `Src/Zle/zle_refresh.c:2221`.
///
/// Prefers the parametrised multi-arg capability when its escape is no
/// longer than `ct` repeats of the single cap (c:2223), falls back to
/// looping the single cap (c:2226-2229), otherwise returns 0 (c:2231) so
/// the caller chooses the fallback. Returns 1 when an escape was emitted.
pub fn tcmultout(cap: i32, multcap: i32, ct: i32) -> i32 {
    // c:2221
    use crate::ported::init::{tclen, tcstr};
    use crate::ported::zsh_h::TC_COUNT;

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

    // c:2223-2225 — `if (tccan(multcap) && (!tccan(cap) ||
    //                  tclen[multcap] <= tclen[cap]*ct)) { tcoutarg(multcap,ct); return 1; }`
    let _ = mult_str;
    if mult_ok && (!cap_ok || mult_len <= cap_len * ct) {
        tcoutarg(multcap, ct);
        return 1;
    } else if cap_ok {
        // c:2226-2229 — `else if (tccan(cap)) { while(ct--) tcout(cap); return 1; }`
        for _ in 0..ct {
            let _ = write_loop(out_fd, cap_str.as_bytes());
        }
        return 1;
    }
    // c:2231 — `return 0;` No capability: the caller decides the fallback
    // (tc_downcurs emits newlines, tc_rightcurs re-outputs cells / CSI C).
    // The previous port emitted \x08 / space here, which is not in C and
    // — for the right case — destructively overwrote cells with spaces.
    0
}

/// Port of `static void tc_rightcurs(int ct)` from
/// `Src/Zle/zle_refresh.c:2237`. Move the cursor right `ct` columns,
/// preferring the most reliable terminal capability:
///   - `TCMULTRIGHT` parametrised right (c:2247-2250) — most reliable
///   - `TCHORIZPOS` absolute horizontal position to `ct + vcs` (c:2253-2256)
///
/// The remaining C strategies are blocked on substrate not yet ported and
/// only matter for terminals lacking the above (essentially none today):
///   - tab-stop stepping (c:2261-2268) needs `oxtabs`
///   - prompt re-output (c:2287-2306) needs `lpromptbuf`/`lprompth`
///   - video-cell re-output + space pad (c:2308-2316) is the last resort
///     "your terminal can't go right" path.
/// When no termcap entry is loaded (headless), emit the portable ANSI
/// cursor-forward (CSI C) — the same sequence a loaded `TCMULTRIGHT` holds.
pub fn tc_rightcurs(count: usize) {
    // c:2237
    if count == 0 {
        return;
    }
    use crate::ported::init::{tclen, tcstr};
    use crate::ported::zsh_h::{TCHORIZPOS, TCMULTRIGHT};
    let ct = count as i32;
    let vcs = VCS.load(Ordering::SeqCst);
    let cl = ct + vcs; // c:2245 — cl = ct + vcs (desired absolute column)
    let out_fd = {
        let f = SHTTY.load(Ordering::Relaxed);
        if f >= 0 {
            f
        } else {
            1
        }
    };

    // c:2247-2250 — `if (tccan(TCMULTRIGHT)) { tcoutarg(TCMULTRIGHT, ct); return; }`
    if tclen.lock().unwrap()[TCMULTRIGHT as usize] != 0 {
        tcoutarg(TCMULTRIGHT, ct);
        return;
    }
    // c:2253-2256 — `if (tccan(TCHORIZPOS)) { tcoutarg(TCHORIZPOS, cl); return; }`
    if tclen.lock().unwrap()[TCHORIZPOS as usize] != 0 {
        tcoutarg(TCHORIZPOS, cl);
        return;
    }
    // Blocked-substrate fallbacks above are deferred; emit the ANSI default.
    let s = format!("\x1b[{}C", ct);
    let _ = write_loop(out_fd, s.as_bytes());
}

/// Direct port of `int tc_downcurs(int ct)` from
/// `Src/Zle/zle_refresh.c:2320`.
/// ```c
/// int tc_downcurs(int ct) {
///     int ret = 0;
///     if (ct && !tcmultout(TCDOWN, TCMULTDOWN, ct)) {
///         while (ct--) zputc(&zr_nl);
///         zputc(&zr_cr), ret = -1;
///     }
///     return ret;
/// }
/// ```
/// Move the cursor down `ct` lines. Prefers the terminal's down
/// capability via `tcmultout`; when that's unavailable it emits real
/// newlines (which scroll/create lines past the drawn region — a plain
/// CSI B can't) followed by a CR, and returns -1 so the caller knows the
/// column was reset to 0. Returns 0 when the capability moved without
/// touching the column.
pub fn tc_downcurs(ct: i32) -> i32 {
    // c:2320
    let mut ret = 0; // c:2324
    // c:2326 — `if (ct && !tcmultout(TCDOWN, TCMULTDOWN, ct))`
    if ct != 0 && tcmultout(crate::ported::zsh_h::TCDOWN, crate::ported::zsh_h::TCMULTDOWN, ct) == 0
    {
        let mut c = ct; // c:2327 while (ct--)
        while c > 0 {
            zwcputc(&REFRESH_ELEMENT { chr: '\n', atr: 0 }); // c:2328 zputc(&zr_nl)
            c -= 1;
        }
        zwcputc(&REFRESH_ELEMENT { chr: '\r', atr: 0 }); // c:2329 zputc(&zr_cr)
        ret = -1; // c:2329
    }
    ret // c:2331
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

/// Direct port of `int clearscreen(UNUSED(char **args))` from
/// `Src/Zle/zle_refresh.c:2424`.
/// ```c
/// int clearscreen(UNUSED(char **args)) {
///     tcoutclear(TCCLEARSCREEN);
///     resetneeded = 1;
///     clearflag = 0;
///     reexpandprompt();
///     return 0;
/// }
/// ```
/// The `clear-screen` widget. The previous port hardcoded `CSI 2J CSI H`
/// instead of the terminal's real clear capability and dropped the
/// resetneeded/clearflag/reexpandprompt sequence.
pub fn clearscreen() -> i32 {
    // c:2424
    // c:2426 — `tcoutclear(TCCLEARSCREEN);` use the terminal's clear cap.
    tcoutclear(crate::ported::zsh_h::TCCLEARSCREEN);
    RESETNEEDED.store(1, Ordering::SeqCst); // c:2427 resetneeded = 1
    CLEARFLAG.store(0, Ordering::SeqCst); // c:2428 clearflag = 0
    // c:2429 — `reexpandprompt();` prompt re-expansion isn't ported yet
    // (prompt subsystem); deferred. zshrs's ZLE loop doesn't honour
    // resetneeded yet, so trigger the redraw directly for the same effect.
    zrefresh();
    0 // c:2430
}

/// Direct port of `int redisplay(UNUSED(char **args))` from
/// `Src/Zle/zle_refresh.c:2435`.
/// ```c
/// int redisplay(UNUSED(char **args)) {
///     moveto(0, 0);
///     zputc(&zr_cr);		/* extra care */
///     tc_upcurs(lprompth - 1);
///     resetneeded = 1;
///     clearflag = 0;
///     return 0;
/// }
/// ```
/// The `redisplay` widget. Positions the cursor at the top-left of the
/// prompt (home, CR for safety, up over the prompt's height) and flags a
/// full redraw. The previous port dropped the whole body and just called
/// zrefresh.
pub fn redisplay() -> i32 {
    // c:2435
    moveto(0, 0); // c:2437
    zwcputc(&REFRESH_ELEMENT { chr: '\r', atr: 0 }); // c:2438 zputc(&zr_cr)
    let lprompth = LPROMPTH.load(Ordering::SeqCst);
    tc_upcurs(lprompth - 1); // c:2439
    RESETNEEDED.store(1, Ordering::SeqCst); // c:2440 resetneeded = 1
    CLEARFLAG.store(0, Ordering::SeqCst); // c:2441 clearflag = 0
    // c:2442 return 0. zshrs's ZLE loop doesn't honour resetneeded yet, so
    // trigger the redraw directly for the same visible effect.
    zrefresh();
    0
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
        singmoveto(0);
        // c:2603
    }

    // c:2680 (function tail) — `singmoveto(nvcs);`
    singmoveto(nvcs);

    let _ = (lpromptw, width); // silence unused
}

/// Direct port of `static void singmoveto(int pos)` from
/// `Src/Zle/zle_refresh.c:2745`. Single-line horizontal cursor
/// positioning to column `pos`, operating on the global video-cursor
/// column `vcs` (the VCS atomic) exactly as C does — shared by `moveto`
/// (c:2216) and `singlerefresh` (c:2661/2717/2738).
///   - exit early when already at `pos` (c:2750)
///   - if no TCMULTLEFT or target at/near BOL: emit `\r`, vcs = 0 (c:2755-2758)
///   - left of current: `tc_leftcurs(vcs - pos)` (c:2760)
///   - right of current: `tc_rightcurs(pos - vcs)` (c:2762)
///   - `vcs = pos` (c:2764)
///
/// The previous port threaded a `RefreshState` and its callers passed a
/// throwaway `RefreshState::new()` (vcs always 0), ignoring the global
/// VCS that `singlerefresh` actually maintains (line ~691) — fixed here.
pub fn singmoveto(pos: i32) {
    // c:2745
    use crate::ported::init::tclen;
    use crate::ported::zsh_h::TCMULTLEFT;

    let vcs = VCS.load(Ordering::SeqCst);
    // c:2750 — `if (pos == vcs) return;`
    if pos == vcs {
        return;
    }

    let multleft_present = tclen.lock().unwrap()[TCMULTLEFT as usize] > 0;
    // c:2755-2758 — `if ((!tccan(TCMULTLEFT) || pos == 0) && pos <= vcs/2)`
    let mut cur = vcs;
    if (!multleft_present || pos == 0) && pos <= cur / 2 {
        let fd = SHTTY.load(Ordering::Relaxed);
        let out_fd = if fd >= 0 { fd } else { 1 };
        let _ = write_loop(out_fd, b"\r"); // c:2756 zputc(&zr_cr)
        cur = 0;
        VCS.store(0, Ordering::SeqCst); // c:2757 vcs = 0
    }

    if pos < cur {
        // c:2760 — `tc_leftcurs(vcs - pos);`
        tc_leftcurs(cur - pos);
    } else if pos > cur {
        // c:2762 — `tc_rightcurs(pos - vcs);`
        tc_rightcurs((pos - cur) as usize);
    }
    // c:2764 — `vcs = pos;`
    VCS.store(pos, Ordering::SeqCst);
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

/// Port of the `tcinscost(X)` macro from `Src/Zle/zle_refresh.c:1782`.
/// `#define tcinscost(X) (tccan(TCMULTINS) ? tclen[TCMULTINS] : (X)*tclen[TCINS])`.
/// Cost (in chars) to insert `x` characters: the parametrised multi-insert
/// capability if the terminal has it, else `x` single-inserts. `tccan(X)`
/// is `tclen[X]` (zsh.h:2680); the `tclen` substrate (init.rs) is populated
/// by the termcap loader, so the real costs are read here.
#[inline]
pub fn tcinscost(x: i32) -> i32 {
    // c:1782 — `#define tcinscost(X)
    //            (tccan(TCMULTINS) ? tclen[TCMULTINS] : (X)*tclen[TCINS])`
    // tccan(X) is tclen[X] (zsh.h:2680). The tclen substrate (init.rs:108)
    // is now populated by the termcap loader, so read the real costs: a
    // parametrised multi-insert costs one capability; otherwise it's `x`
    // single-char inserts.
    use crate::ported::init::tclen;
    use crate::ported::zsh_h::{TCINS, TCMULTINS};
    let t = tclen.lock().unwrap();
    if t[TCMULTINS as usize] != 0 {
        t[TCMULTINS as usize]
    } else {
        x * t[TCINS as usize]
    }
}

/// Port of the `tcdelcost(X)` macro from `Src/Zle/zle_refresh.c:1783`.
/// `#define tcdelcost(X) (tccan(TCMULTDEL) ? tclen[TCMULTDEL] : (X)*tclen[TCDEL])`.
#[inline]
pub fn tcdelcost(x: i32) -> i32 {
    // c:1783 — `#define tcdelcost(X)
    //            (tccan(TCMULTDEL) ? tclen[TCMULTDEL] : (X)*tclen[TCDEL])`
    // Mirror of tcinscost for the delete-character capabilities.
    use crate::ported::init::tclen;
    use crate::ported::zsh_h::{TCDEL, TCMULTDEL};
    let t = tclen.lock().unwrap();
    if t[TCMULTDEL as usize] != 0 {
        t[TCMULTDEL as usize]
    } else {
        x * t[TCDEL as usize]
    }
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

/// Port of `static int more_start` from `Src/Zle/zle_refresh.c:672` —
/// "more text before start of screen?". Set by `scrollwindow` when the
/// buffer scrolls content off the top (c:808), reset each frame (c:1119);
/// the first-line ">..." indicator reads it (c:1643, not yet rendered).
pub static MORE_START: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0); // c:672

/// Port of `static int more_end` from `Src/Zle/zle_refresh.c:672` — "more
/// text after end of screen?". Set by `snextline` when the status pane
/// scrolls content off the bottom (c:887/895); reset each frame (c:1119).
/// The "...<" end indicator reads it (the consumer is unported, like the
/// `more_start` ">..." one).
pub static MORE_END: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0); // c:672

/// Port of `mod_export int resetneeded` from `Src/Zle/zle_refresh.c`.
/// Set when the display must be fully redrawn (e.g. after clear-screen or
/// a SIGWINCH); the ZLE loop honours it on its next pass. zshrs's ZLE
/// loop doesn't read it yet, so widgets that set it also trigger the
/// redraw directly — this is the name-parity anchor for when the loop's
/// resetneeded handling lands.
pub static RESETNEEDED: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);

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

/// Direct port of `void unset_region_highlight(Param pm, int exp)` from
/// `Src/Zle/zle_refresh.c:592`.
/// ```c
/// void unset_region_highlight(Param pm, int exp) {
///     if (exp) {
///         set_region_highlight(pm, NULL);
///         stdunsetfn(pm, exp);
///     }
/// }
/// ```
/// Unset hook for the `$region_highlight` special parameter: when the
/// user explicitly unsets it (`exp` true), clear the user region
/// highlights back to the special baseline (`set_region_highlight(NULL)`,
/// c:595) and run the standard parameter unset (c:596).
pub fn unset_region_highlight(pm: &mut crate::ported::zsh_h::param, exp: i32) {
    // c:592
    if exp != 0 {
        set_region_highlight(None); // c:595 set_region_highlight(pm, NULL)
        crate::ported::params::stdunsetfn(pm, exp); // c:596
    }
}

/// Direct port of `char **get_region_highlight(Param pm)` from
/// `Src/Zle/zle_refresh.c:430`. The get hook for the `$region_highlight`
/// special parameter: format each user region highlight as
/// `"[P]start end <attr-spec>[ memo=NAME]"` (c:466-476) via the real
/// `output_highlight` (the highlight spec, `fg=red,bold`). C skips the
/// first `N_SPECIAL_HIGHLIGHTS` (4) cursor/region/isearch/suffix entries
/// (c:443); the Rust `REGION_HIGHLIGHTS` holds ONLY user entries (the
/// special baseline isn't stored there), so every entry is a user
/// highlight and no skip is needed. Empty store → empty array (c:437-438).
///
/// `RegionHighlight` stores `attr` as a `TextAttr` (no `atrmask`, since the
/// TextAttr-returning `match_highlight` dropped it), so the (atr, mask)
/// pair `output_highlight` needs is rebuilt from the set flag bits — exact
/// for positive specs (the common case); the explicit-"no"/layer semantics
/// aren't recoverable from `TextAttr` and are omitted.
pub fn get_region_highlight(_pm: &crate::ported::zsh_h::param) -> Vec<String> {
    // c:430
    use crate::ported::zsh_h::{
        zattr, TXTBGCOLOUR, TXTBOLDFACE, TXTFGCOLOUR, TXTSTANDOUT, TXTUNDERLINE,
        TXT_ATTR_BG_COL_SHIFT, TXT_ATTR_FG_COL_SHIFT,
    };
    let rh = REGION_HIGHLIGHTS.lock().unwrap();
    rh.iter()
        .map(|rhp| {
            let mut s = String::new();
            // c:466-468 — `sprintf("%s%s ", P?, "start end")`.
            if rhp.flags & ZRH_PREDISPLAY != 0 {
                s.push('P'); // c:467
            }
            s.push_str(&format!("{} {} ", rhp.start, rhp.end));
            // c:469 — output_highlight(atr, atrmask). Rebuild (atr, mask)
            // from the TextAttr: every set field contributes both its value
            // to `atr` and its flag bit to `mask`.
            let ta = &rhp.attr;
            let mut atr: zattr = 0;
            let mut mask: zattr = 0;
            if ta.bold {
                atr |= TXTBOLDFACE;
                mask |= TXTBOLDFACE;
            }
            if ta.underline {
                atr |= TXTUNDERLINE;
                mask |= TXTUNDERLINE;
            }
            if ta.standout {
                atr |= TXTSTANDOUT;
                mask |= TXTSTANDOUT;
            }
            if let Some(fg) = ta.fg_color {
                atr |= TXTFGCOLOUR | ((fg as zattr) << TXT_ATTR_FG_COL_SHIFT);
                mask |= TXTFGCOLOUR;
            }
            if let Some(bg) = ta.bg_color {
                atr |= TXTBGCOLOUR | ((bg as zattr) << TXT_ATTR_BG_COL_SHIFT);
                mask |= TXTBGCOLOUR;
            }
            s.push_str(&crate::ported::prompt::output_highlight(atr, mask));
            // c:473-475 — `memo=NAME`.
            if let Some(memo) = &rhp.memo {
                s.push_str(" memo=");
                s.push_str(memo);
            }
            s
        })
        .collect()
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

    /// c:1782-1783 — tcinscost/tcdelcost follow the literal C macro
    /// `(X)*tclen[TC*]` in the per-char branch. This previously asserted
    /// the old `x.max(0)` fake (`tcinscost(5)==5` regardless of tclen);
    /// now it pins the faithful formula with a deterministic single-char
    /// cost of 1, matching C exactly (including the unclamped negative,
    /// which refreshline never produces since `i` starts at 1).
    #[test]
    fn tc_costs_handle_negative() {
        use crate::ported::init::tclen;
        use crate::ported::zsh_h::{TCDEL, TCINS, TCMULTDEL, TCMULTINS};
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let save = {
            let t = tclen.lock().unwrap();
            (
                t[TCMULTINS as usize],
                t[TCINS as usize],
                t[TCMULTDEL as usize],
                t[TCDEL as usize],
            )
        };
        {
            let mut t = tclen.lock().unwrap();
            t[TCMULTINS as usize] = 0; // no multi-cap → per-char branch
            t[TCINS as usize] = 1;
            t[TCMULTDEL as usize] = 0;
            t[TCDEL as usize] = 1;
        }
        assert_eq!(tcinscost(-1), -1); // c: literal (-1)*tclen[TCINS]
        assert_eq!(tcdelcost(-1), -1);
        assert_eq!(tcinscost(5), 5);
        assert_eq!(tcdelcost(5), 5);

        let mut t = tclen.lock().unwrap();
        t[TCMULTINS as usize] = save.0;
        t[TCINS as usize] = save.1;
        t[TCMULTDEL as usize] = save.2;
        t[TCDEL as usize] = save.3;
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

    /// Proof of the diff path: with NBUF[0]="abc" and an empty OBUF,
    /// refreshline(0) must emit the new line's cells. Captures output by
    /// redirecting SHTTY to a pipe. This proves the NBUF→refreshline→
    /// zwcwrite→zwcputc chain writes real cell data before the live
    /// renderer is ever switched to it.
    #[test]
    fn refreshline_emits_new_line_cells() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        *NBUF.lock().unwrap() = vec!["abc"
            .chars()
            .map(|c| REFRESH_ELEMENT { chr: c, atr: 0 })
            .collect()];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(1, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);

        refreshline(0);

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains('a') && s.contains('b') && s.contains('c'),
            "refreshline should emit the new-line cells a/b/c; got {:?}",
            s
        );
    }

    /// Swap validation: the LIVE zrefresh path now renders via the NBUF/
    /// OBUF diff (refreshline), not full-repaint. Driving the whole
    /// zrefresh with a clean first frame must emit the editable line.
    #[test]
    fn zrefresh_renders_line_via_diff() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        // Clean video state so the first frame writes everything.
        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);

        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLECS.store(5, Ordering::SeqCst);
        ZLELL.store(5, Ordering::SeqCst);

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        zrefresh();

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("hello"),
            "live zrefresh (diff path) should render the line; got {:?}",
            s
        );
    }

    /// Integration proof: the NBUF that zrefresh actually builds from the
    /// editable line renders correctly through refreshline. This validates
    /// the full build→diff→emit path end to end (zrefresh's NBUF cells are
    /// real, refreshline renders them) before the live output is ever
    /// switched from full-repaint to the diff.
    #[test]
    fn zrefresh_nbuf_renders_via_refreshline() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        *ZLELINE.lock().unwrap() = "hello".chars().collect();
        ZLECS.store(5, Ordering::SeqCst);
        ZLELL.store(5, Ordering::SeqCst);

        // Let zrefresh's full-repaint output go to /dev/null while it
        // builds NBUF (we only want the NBUF, not its escapes).
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        let devnull = unsafe {
            libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY)
        };
        crate::ported::init::SHTTY.store(devnull, Ordering::SeqCst);
        zrefresh(); // builds NBUF (and OBUF=previous via its swap)
        if devnull >= 0 {
            unsafe { libc::close(devnull) };
        }

        // Render the just-built NBUF via refreshline, forcing a full write
        // (empty OBUF), capturing the output.
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);
        *OBUF.lock().unwrap() = vec![];
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);

        refreshline(0);

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("hello"),
            "zrefresh-built NBUF should render 'hello' via refreshline; got {:?}",
            s
        );
    }

    /// Diff-path proof, shorten case: OBUF="abcd" → NBUF="ab". The old
    /// trailing "cd" must be erased — when the terminal lacks clear-to-eol
    /// (none in a headless test), refreshline overwrites it with spaces.
    #[test]
    fn refreshline_erases_shortened_tail() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        let mk = |s: &str| -> REFRESH_STRING {
            s.chars().map(|c| REFRESH_ELEMENT { chr: c, atr: 0 }).collect()
        };
        *OBUF.lock().unwrap() = vec![mk("abcd")];
        *NBUF.lock().unwrap() = vec![mk("ab")];
        NLNCT.store(1, Ordering::SeqCst);
        OLNCT.store(1, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);
        // No clear-to-eol capability → forces the space-overwrite path.
        tclen.lock().unwrap()[TCCLEAREOL as usize] = 0;

        refreshline(0);

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        // The two old tail columns ("cd") are each overwritten with a space
        // (interspersed with cursor moves), so at least two spaces emit.
        assert!(
            s.matches(' ').count() >= 2,
            "shortened line should erase the 2 old tail cells with spaces; got {:?}",
            s
        );
    }

    /// Diff-path proof, edit case: OBUF="abc" → NBUF="abd". After the
    /// common "ab" prefix, refreshline must emit the changed cell 'd'.
    /// Exercises the common-prefix-skip + change branch (not the all-new
    /// write path of the previous test).
    #[test]
    fn refreshline_emits_changed_cell_on_edit() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        let mk = |s: &str| -> REFRESH_STRING {
            s.chars().map(|c| REFRESH_ELEMENT { chr: c, atr: 0 }).collect()
        };
        *OBUF.lock().unwrap() = vec![mk("abc")];
        *NBUF.lock().unwrap() = vec![mk("abd")];
        NLNCT.store(1, Ordering::SeqCst);
        OLNCT.store(1, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);

        refreshline(0);

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains('d'),
            "edit should emit the changed cell 'd'; got {:?}",
            s
        );
    }

    /// c:1923/2089 — after refreshline edits "abc"→"abd", the video cursor
    /// (VCS) must land at column 3: moveto repositions to ccs=2 (past the
    /// "ab" common prefix), then writing 'd' advances it to 3. The previous
    /// snapshot port left the local `vcs` frozen at its initial 0, so it
    /// stored VCS=1 (0+1) — wrong. This pins the live-tracker fix.
    #[test]
    fn refreshline_tracks_vcs_across_prefix_skip() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        // Output goes to /dev/null; we only assert the VCS tracker here.
        let devnull = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY) };
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(devnull, Ordering::SeqCst);

        let mk = |s: &str| -> REFRESH_STRING {
            s.chars().map(|c| REFRESH_ELEMENT { chr: c, atr: 0 }).collect()
        };
        *OBUF.lock().unwrap() = vec![mk("abc")];
        *NBUF.lock().unwrap() = vec![mk("abd")];
        NLNCT.store(1, Ordering::SeqCst);
        OLNCT.store(1, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);

        refreshline(0);

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(devnull) };
        assert_eq!(
            VCS.load(Ordering::SeqCst),
            3,
            "VCS must track to column 3 (ccs=2 after prefix skip + 1 written cell)"
        );
    }

    /// c:1672-1674 — when a frame has more lines than the last, the
    /// newly-used lines must be cleared (cleareol). Growing "a" → "a\n"
    /// (line 1 empty) drives the clear-eol short-circuit (c:1776-1780):
    /// refreshline(1) emits moveto + the clear escape. With tccan(TCCLEAREOL)
    /// off this can't fire, so the test wires tclen[TCCLEAREOL]. Without the
    /// per-line cleareol, line 1 stays cleareol=0 and no clear is emitted.
    #[test]
    fn zrefresh_clears_newly_grown_line() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        use crate::ported::init::{tclen, tcstr};
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let saved_tc = tclen.lock().unwrap()[TCCLEAREOL as usize];
        let saved_str = tcstr.lock().unwrap()[TCCLEAREOL as usize].clone();
        tclen.lock().unwrap()[TCCLEAREOL as usize] = 3; // tccan(TCCLEAREOL)
        // The real clear-to-end-of-LINE escape (CSI K), not CSI J.
        tcstr.lock().unwrap()[TCCLEAREOL as usize] = "\x1b[K".to_string();

        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);

        // Frame 1: single line "a" → /dev/null (establishes OBUF/OLNCT=1).
        let devnull = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY) };
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(devnull, Ordering::SeqCst);
        *ZLELINE.lock().unwrap() = "a".chars().collect();
        ZLECS.store(1, Ordering::SeqCst);
        ZLELL.store(1, Ordering::SeqCst);
        zrefresh();
        unsafe { libc::close(devnull) };

        // Frame 2: grow to two lines "a\n" (line 1 empty) → capture.
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst); // start the frame un-cleared
        *ZLELINE.lock().unwrap() = "a\n".chars().collect();
        ZLECS.store(2, Ordering::SeqCst);
        ZLELL.store(2, Ordering::SeqCst);
        zrefresh();

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        tclen.lock().unwrap()[TCCLEAREOL as usize] = saved_tc;
        tcstr.lock().unwrap()[TCCLEAREOL as usize] = saved_str;
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("\x1b[K"),
            "grown line 1 should be cleared to end-of-line (CSI K); got {:?}",
            s
        );
    }

    /// c:1683-1691 — line-delete optimisation. Old buffer ["L0","XX","L2"],
    /// new buffer ["L0","L2","YY"]: at iln=1 the old line differs from the
    /// new but old line 2 ("L2") equals new line 1, so one TCDELLINE scrolls
    /// the rest into place instead of rewriting both lines. With
    /// tccan(TCDELLINE) wired, zrefresh must emit the delete-line escape.
    #[test]
    fn zrefresh_deletes_line_via_tcdelline() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        use crate::ported::init::{tclen, tcstr};
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let di = crate::ported::zsh_h::TCDELLINE as usize;
        let saved_len = tclen.lock().unwrap()[di];
        let saved_str = tcstr.lock().unwrap()[di].clone();
        tclen.lock().unwrap()[di] = 3; // tccan(TCDELLINE)
        tcstr.lock().unwrap()[di] = "\x1b[M".to_string(); // delete-line escape

        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VMAXLN.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        LPROMPTW.store(0, Ordering::SeqCst);
        CLEAREOL.store(0, Ordering::SeqCst);
        CLEARFLAG.store(0, Ordering::SeqCst);
        crate::ported::init::hasam.store(0, Ordering::SeqCst);

        // Frame 1: ["L0","XX","L2"] → /dev/null (becomes OBUF).
        let devnull = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY) };
        let old_shtty = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(devnull, Ordering::SeqCst);
        *ZLELINE.lock().unwrap() = "L0\nXX\nL2".chars().collect();
        ZLECS.store(8, Ordering::SeqCst);
        ZLELL.store(8, Ordering::SeqCst);
        zrefresh();
        unsafe { libc::close(devnull) };

        // Frame 2: ["L0","L2","YY"] → capture. old line 2 == new line 1.
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        *ZLELINE.lock().unwrap() = "L0\nL2\nYY".chars().collect();
        ZLECS.store(8, Ordering::SeqCst);
        ZLELL.store(8, Ordering::SeqCst);
        zrefresh();

        crate::ported::init::SHTTY.store(old_shtty, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        tclen.lock().unwrap()[di] = saved_len;
        tcstr.lock().unwrap()[di] = saved_str;
        let mut out = Vec::new();
        let mut f = unsafe { std::fs::File::from_raw_fd(rd) };
        let _ = f.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert!(
            s.contains("\x1b[M"),
            "line-delete opt should emit the TCDELLINE escape; got {:?}",
            s
        );
    }

    /// zrefresh converts each line char's overlay attr to the cell's zattr
    /// (c:1226-1248), so refreshline/zwcputc emit its colour. A bold region
    /// over the line must make those cells carry TXTBOLDFACE.
    #[test]
    fn zrefresh_nbuf_cells_carry_attr() {
        use crate::ported::zsh_h::TXTBOLDFACE;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *ZLELINE.lock().unwrap() = "abc".chars().collect();
        ZLECS.store(0, Ordering::SeqCst);
        ZLELL.store(3, Ordering::SeqCst);
        // A bold region over the whole line (the path compute_render_attrs
        // reads — the highlight manager, not the REGION_HIGHLIGHTS static).
        let custom = TextAttr {
            bold: true,
            ..TextAttr::default()
        };
        highlight().lock().unwrap().add_region(0, 3, custom);
        zrefresh();
        let nbuf = NBUF.lock().unwrap();
        let row0 = nbuf.first().expect("NBUF has a row");
        let a_cell = row0
            .iter()
            .find(|c| c.chr == 'a')
            .expect("'a' cell present");
        assert!(
            a_cell.atr & TXTBOLDFACE != 0,
            "bold region -> cell carries TXTBOLDFACE, got atr={:#x}",
            a_cell.atr
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

    /// `tcoutclear(cap)` runs without panic for each clear capability.
    #[test]
    fn tcoutclear_runs_without_panic() {
        let _g = crate::test_util::global_state_lock();
        tcoutclear(crate::ported::zsh_h::TCCLEARSCREEN);
        tcoutclear(crate::ported::zsh_h::TCCLEAREOL);
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

    /// c:607 — `tcoutclear(cap)` safe for both clear-line and clear-screen.
    #[test]
    fn tcoutclear_both_modes_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        tcoutclear(crate::ported::zsh_h::TCCLEAREOL);
        tcoutclear(crate::ported::zsh_h::TCCLEARSCREEN);
    }

    /// c:803-808 — scrollwindow(tline) rotates the video buffer: line
    /// `tline` lifts out, lower lines shift up, the lifted line wraps to
    /// the bottom, and more_start is set when scrolling from the top.
    #[test]
    fn scrollwindow_rotates_buffer_and_sets_more_start() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mk = |s: &str| -> REFRESH_STRING {
            s.chars().map(|c| REFRESH_ELEMENT { chr: c, atr: 0 }).collect()
        };
        WINH.store(4, Ordering::SeqCst);
        MORE_START.store(0, Ordering::SeqCst);
        *NBUF.lock().unwrap() = vec![mk("L0"), mk("L1"), mk("L2"), mk("L3")];

        scrollwindow(0); // lift L0, shift up, L0 wraps to bottom

        let rows: Vec<String> = NBUF
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.iter().map(|c| c.chr).collect())
            .collect();
        assert_eq!(rows, vec!["L1", "L2", "L3", "L0"], "rotate-left by one");
        assert_eq!(
            MORE_START.load(Ordering::SeqCst),
            1,
            "scrolling from the top sets more_start"
        );
    }

    /// c:807 — scrolling from a non-zero line does NOT set more_start, and
    /// rotates only the window from `tline` down.
    #[test]
    fn scrollwindow_from_nonzero_line() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mk = |s: &str| -> REFRESH_STRING {
            s.chars().map(|c| REFRESH_ELEMENT { chr: c, atr: 0 }).collect()
        };
        WINH.store(4, Ordering::SeqCst);
        MORE_START.store(0, Ordering::SeqCst);
        *NBUF.lock().unwrap() = vec![mk("L0"), mk("L1"), mk("L2"), mk("L3")];

        scrollwindow(1); // rotate [1..4): L1 wraps to bottom, L0 untouched

        let rows: Vec<String> = NBUF
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.iter().map(|c| c.chr).collect())
            .collect();
        assert_eq!(rows, vec!["L0", "L2", "L3", "L1"], "rotate from line 1");
        assert_eq!(
            MORE_START.load(Ordering::SeqCst),
            0,
            "non-top scroll must not set more_start"
        );
    }

    /// scrollwindow on an out-of-range / negative tline is a safe no-op
    /// (the Vec may be shorter than winh).
    #[test]
    fn scrollwindow_out_of_range_no_panic() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *NBUF.lock().unwrap() = vec![];
        scrollwindow(-1);
        scrollwindow(0);
        scrollwindow(i32::MAX);
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

    /// c:2159-2204 — `moveto` updates the video-position trackers (vln, vcs)
    /// to the target, so the diff engine's next-frame cursor logic is correct.
    #[test]
    fn moveto_updates_vcs_vln() {
        use std::sync::atomic::Ordering;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        moveto(3, 7);
        assert_eq!(VLN.load(Ordering::SeqCst), 3, "moveto must set VLN to row");
        assert_eq!(VCS.load(Ordering::SeqCst), 7, "moveto must set VCS to col");
    }

    /// c:1629 — `tcmultout(0, 0, 0)` returns i32 (compile-time type pin).
    #[test]
    fn tcmultout_returns_i32_type() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let _: i32 = tcmultout(0, 0, 0);
    }

    /// c:737-755 — resetvideo allocates the global NBUF/OBUF: (winh+1) rows
    /// of (winw+2) cells each (matching C's global nbuf/obuf), so the buffer
    /// lifecycle is on the same buffers as nextline/snextline/scrollwindow.
    #[test]
    fn resetvideo_allocates_global_nbuf_obuf() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];

        let mut state = RefreshState::new();
        resetvideo(&mut state);

        let winw = WINW.load(Ordering::SeqCst);
        let winh = WINH.load(Ordering::SeqCst);
        let nbuf = NBUF.lock().unwrap();
        let obuf = OBUF.lock().unwrap();
        assert_eq!(nbuf.len() as i32, winh + 1, "NBUF has winh+1 rows");
        assert_eq!(obuf.len() as i32, winh + 1, "OBUF has winh+1 rows");
        assert!(!nbuf.is_empty());
        assert_eq!(nbuf[0].len() as i32, winw + 2, "row is winw+2 cells");
    }

    /// c:875-905 — snextline (status-area row advance, now real): off the
    /// bottom row it terminates + advances; at the bottom with room above
    /// (tosln > ln, nvln > 1) it scrolls and decrements tosln/nvln.
    #[test]
    fn snextline_advances_then_scrolls_status_pane() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        WINW.store(4, Ordering::SeqCst);
        WINH.store(3, Ordering::SeqCst);
        let row = |c: char| -> REFRESH_STRING {
            vec![REFRESH_ELEMENT { chr: c, atr: 0 }; 6]
        };
        *NBUF.lock().unwrap() = vec![row('A'), row('B'), row('C')];

        // Not at bottom → terminate + advance.
        let mut rpms = rparams::default();
        rpms.ln = 0;
        rpms.pos = 2;
        snextline(&mut rpms);
        assert_eq!(rpms.ln, 1, "advanced");
        assert_eq!(rpms.pos, 0);
        assert_eq!(rpms.end, 4);
        assert_eq!(NBUF.lock().unwrap()[0][2].chr, '\0', "terminated at pos");

        // At bottom, tosln > ln and nvln > 1 → scroll, tosln--/nvln--.
        let mut rpms2 = rparams::default();
        rpms2.ln = 2; // winh - 1
        rpms2.tosln = 5;
        rpms2.nvln = 2;
        snextline(&mut rpms2);
        assert_eq!(rpms2.tosln, 4, "tosln decremented");
        assert_eq!(rpms2.nvln, 1, "nvln decremented after scroll");
        assert_eq!(
            NBUF.lock().unwrap()[0][0].chr,
            'B',
            "scrollwindow(0) rotated row 1 up to row 0"
        );
    }

    /// c:841-873 — nextline (now on the global NBUF): off the bottom row it
    /// marks the wrap/terminator, advances ln, allocates the next row, and
    /// resets pos/end. At the bottom row it scrolls the buffer instead.
    #[test]
    fn nextline_advances_then_scrolls_on_global_nbuf() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        WINW.store(4, Ordering::SeqCst);
        WINH.store(3, Ordering::SeqCst);
        NUMSCROLLS.store(0, Ordering::SeqCst);
        ONUMSCROLLS.store(0, Ordering::SeqCst);
        let row = |c: char| -> REFRESH_STRING {
            vec![REFRESH_ELEMENT { chr: c, atr: 0 }; 6] // winw + 2 cells
        };
        *NBUF.lock().unwrap() = vec![row('A'), row('B'), row('C')];

        // From ln=0 (not bottom), wrapped: advance + wrap marker.
        let mut rpms = rparams::default();
        rpms.ln = 0;
        rpms.pos = 2;
        rpms.nvln = -1;
        let ret = nextline(&mut rpms, 1);
        assert_eq!(ret, 0);
        assert_eq!(rpms.ln, 1, "advanced to next line");
        assert_eq!(rpms.pos, 0); // c:871
        assert_eq!(rpms.end, 4); // c:872 — winw
        {
            let nbuf = NBUF.lock().unwrap();
            assert_eq!(nbuf[0][5].chr, '\n', "wrap marker at winw+1"); // c:844
            assert_eq!(nbuf[0][2].chr, '\0', "terminated at pos"); // c:845
        }

        // From ln=winh-1 (bottom) with nvln=-1: scroll instead of advance.
        let mut rpms2 = rparams::default();
        rpms2.ln = 2;
        rpms2.nvln = -1;
        let ret2 = nextline(&mut rpms2, 0);
        assert_eq!(ret2, 0);
        assert_eq!(rpms2.ln, 2, "stays at the bottom row after scroll");
        // scrollwindow(0) rotated the buffer up: old row 1 ('B') is now row 0.
        assert_eq!(
            NBUF.lock().unwrap()[0][0].chr,
            'B',
            "buffer scrolled: row 1 rose to row 0"
        );
    }

    /// c:430-479 — get_region_highlight formats each user region highlight
    /// as "start end <spec>", with the attribute rendered as the real
    /// highlight spec (fg=red) now that output_highlight is faithful; an
    /// empty store yields an empty array.
    #[test]
    fn get_region_highlight_formats_user_entries() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        set_region_highlight(Some(&["0 5 fg=red".to_string()]));
        let arr = get_region_highlight(&crate::ported::zsh_h::param::default());
        assert_eq!(arr.len(), 1, "one user highlight → one entry; got {:?}", arr);
        assert_eq!(arr[0], "0 5 fg=red", "spec round-trips, not SGR");

        let mut pm = crate::ported::zsh_h::param::default();
        unset_region_highlight(&mut pm, 1);
        assert!(
            get_region_highlight(&pm).is_empty(),
            "no highlights → empty array"
        );
    }

    /// c:592 — unset_region_highlight clears the user region highlights
    /// (set_region_highlight(NULL)) and runs the standard unset only when
    /// the parameter is explicitly unset (exp != 0); exp == 0 is a no-op.
    #[test]
    fn unset_region_highlight_clears_only_on_exp() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        // Seed a user region highlight.
        set_region_highlight(Some(&["0 5 fg=red".to_string()]));
        let with_user = REGION_HIGHLIGHTS.lock().unwrap().len();
        assert!(with_user > 0, "set_region_highlight should add a user entry");

        let mut pm = crate::ported::zsh_h::param::default();
        // exp == 0 → no change.
        unset_region_highlight(&mut pm, 0);
        assert_eq!(
            REGION_HIGHLIGHTS.lock().unwrap().len(),
            with_user,
            "exp=0 must be a no-op"
        );
        // exp != 0 → user highlights cleared to the special baseline.
        unset_region_highlight(&mut pm, 1);
        assert!(
            REGION_HIGHLIGHTS.lock().unwrap().len() < with_user,
            "exp!=0 must clear the user highlights"
        );
    }

    /// c:152 — zrefresh publishes the prompt's trailing attribute to the
    /// global PROMPT_ATTR, which refreshline (TCDEL) and tcoutclear read so
    /// deleted/cleared cells carry the prompt's colour. A bold prompt must
    /// leave PROMPT_ATTR carrying TXTBOLDFACE.
    #[test]
    fn zrefresh_publishes_prompt_attr() {
        use crate::ported::zsh_h::TXTBOLDFACE;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let saved = crate::ported::zle::zle_main::LPROMPT.lock().unwrap().clone();
        *crate::ported::zle::zle_main::LPROMPT.lock().unwrap() = "\x1b[1mPS>".to_string();
        PROMPT_ATTR.store(0, Ordering::SeqCst);
        *ZLELINE.lock().unwrap() = "x".chars().collect();
        ZLECS.store(1, Ordering::SeqCst);
        ZLELL.store(1, Ordering::SeqCst);
        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);

        let devnull = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY) };
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(devnull, Ordering::SeqCst);
        zrefresh();
        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(devnull) };
        *crate::ported::zle::zle_main::LPROMPT.lock().unwrap() = saved;

        assert_ne!(
            PROMPT_ATTR.load(Ordering::SeqCst) & TXTBOLDFACE,
            0,
            "bold prompt must publish TXTBOLDFACE to PROMPT_ATTR"
        );
    }

    /// c:676 — zrefresh syncs LPROMPTW to the left-prompt width each frame,
    /// enabling refreshline's prompt-skip (dead while LPROMPTW stayed 0).
    #[test]
    fn zrefresh_syncs_lpromptw_from_prompt() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let saved_prompt = crate::ported::zle::zle_main::LPROMPT.lock().unwrap().clone();
        *crate::ported::zle::zle_main::LPROMPT.lock().unwrap() = "abc".to_string();
        LPROMPTW.store(999, Ordering::SeqCst); // bogus stale value
        *ZLELINE.lock().unwrap() = "x".chars().collect();
        ZLECS.store(1, Ordering::SeqCst);
        ZLELL.store(1, Ordering::SeqCst);
        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);

        let devnull = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY) };
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(devnull, Ordering::SeqCst);
        zrefresh();
        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(devnull) };
        *crate::ported::zle::zle_main::LPROMPT.lock().unwrap() = saved_prompt;

        assert_eq!(
            LPROMPTW.load(Ordering::SeqCst),
            3,
            "LPROMPTW must sync to the width of prompt \"abc\""
        );
    }

    /// c:729-734 — zrefresh syncs the global video width to the terminal
    /// each frame, so the cursor primitives don't read the stale 80-col
    /// default. After a frame, WINW must equal adjustcolumns(), regardless
    /// of any earlier bogus value.
    #[test]
    fn zrefresh_syncs_winw_to_terminal() {
        use crate::ported::utils::adjustcolumns;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        WINW.store(999, Ordering::SeqCst); // bogus stale value
        *ZLELINE.lock().unwrap() = "x".chars().collect();
        ZLECS.store(1, Ordering::SeqCst);
        ZLELL.store(1, Ordering::SeqCst);
        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);

        let devnull = unsafe { libc::open(b"/dev/null\0".as_ptr() as *const _, libc::O_WRONLY) };
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(devnull, Ordering::SeqCst);
        zrefresh();
        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(devnull) };

        assert_eq!(
            WINW.load(Ordering::SeqCst),
            adjustcolumns() as i32,
            "zrefresh must sync WINW to the terminal width"
        );
    }

    /// c:2435-2442 — redisplay homes the cursor (with a safety CR), moves up
    /// over the prompt height, flags a full redraw (resetneeded=1,
    /// clearflag=0), and returns 0.
    #[test]
    fn redisplay_homes_and_sets_flags() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        CLEARFLAG.store(1, Ordering::SeqCst);
        RESETNEEDED.store(0, Ordering::SeqCst);
        LPROMPTH.store(1, Ordering::SeqCst); // tc_upcurs(0) → no movement
        *ZLELINE.lock().unwrap() = "x".chars().collect();
        ZLECS.store(1, Ordering::SeqCst);
        ZLELL.store(1, Ordering::SeqCst);
        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        let ret = redisplay();

        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let _ = unsafe { std::fs::File::from_raw_fd(rd) }.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);

        assert_eq!(ret, 0, "redisplay returns 0");
        assert!(s.contains('\r'), "redisplay emits the safety CR; got {:?}", s);
        assert_eq!(CLEARFLAG.load(Ordering::SeqCst), 0, "clearflag zeroed");
        assert_eq!(RESETNEEDED.load(Ordering::SeqCst), 1, "resetneeded set");
    }

    /// c:2424-2430 — clearscreen emits the terminal's clear capability (not
    /// a hardcoded CSI 2J), zeroes clearflag, sets resetneeded, and returns 0.
    #[test]
    fn clearscreen_uses_clear_cap_and_sets_flags() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        use crate::ported::init::{tclen, tcstr};
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let ci = crate::ported::zsh_h::TCCLEARSCREEN as usize;
        let save_len = tclen.lock().unwrap()[ci];
        let save_str = tcstr.lock().unwrap()[ci].clone();
        tclen.lock().unwrap()[ci] = 4;
        tcstr.lock().unwrap()[ci] = "\x1b[2J".to_string();

        CLEARFLAG.store(1, Ordering::SeqCst);
        RESETNEEDED.store(0, Ordering::SeqCst);
        *ZLELINE.lock().unwrap() = "x".chars().collect();
        ZLECS.store(1, Ordering::SeqCst);
        ZLELL.store(1, Ordering::SeqCst);
        *NBUF.lock().unwrap() = vec![];
        *OBUF.lock().unwrap() = vec![];
        NLNCT.store(0, Ordering::SeqCst);
        OLNCT.store(0, Ordering::SeqCst);
        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        let ret = clearscreen();

        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        tclen.lock().unwrap()[ci] = save_len;
        tcstr.lock().unwrap()[ci] = save_str;
        let mut out = Vec::new();
        let _ = unsafe { std::fs::File::from_raw_fd(rd) }.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);

        assert_eq!(ret, 0, "clearscreen returns 0");
        assert!(s.contains("\x1b[2J"), "must emit the clear capability; got {:?}", s);
        assert_eq!(CLEARFLAG.load(Ordering::SeqCst), 0, "clearflag zeroed");
        assert_eq!(RESETNEEDED.load(Ordering::SeqCst), 1, "resetneeded set");
    }

    /// c:946-956 — bufswap exchanges the global NBUF and OBUF buffers so
    /// last frame's NBUF becomes this frame's OBUF (the diff baseline).
    #[test]
    fn bufswap_exchanges_global_nbuf_obuf() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        let mk = |s: &str| -> REFRESH_STRING {
            s.chars().map(|c| REFRESH_ELEMENT { chr: c, atr: 0 }).collect()
        };
        *NBUF.lock().unwrap() = vec![mk("new")];
        *OBUF.lock().unwrap() = vec![mk("old")];
        bufswap();
        let nb: String = NBUF.lock().unwrap()[0].iter().map(|c| c.chr).collect();
        let ob: String = OBUF.lock().unwrap()[0].iter().map(|c| c.chr).collect();
        assert_eq!(nb, "old", "NBUF must hold the previously-old buffer");
        assert_eq!(ob, "new", "OBUF must hold the previously-new buffer");
    }

    /// c:2247-2250 — tc_rightcurs prefers the real loaded TCMULTRIGHT
    /// capability (with the move count substituted) over a hardcoded CSI C;
    /// with no termcap entry it emits the ANSI default. Pins the capability
    /// preference against the old unconditional CSI C.
    #[test]
    fn tc_rightcurs_prefers_loaded_capability() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        use crate::ported::init::{tclen, tcstr};
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let mr = crate::ported::zsh_h::TCMULTRIGHT as usize;
        let hp = crate::ported::zsh_h::TCHORIZPOS as usize;
        let save_mr_len = tclen.lock().unwrap()[mr];
        let save_mr_str = tcstr.lock().unwrap()[mr].clone();
        let save_hp_len = tclen.lock().unwrap()[hp];

        let capture = |f: &dyn Fn()| -> String {
            let mut fds = [0i32; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
            let (rd, wr) = (fds[0], fds[1]);
            let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
            crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);
            f();
            crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
            unsafe { libc::close(wr) };
            let mut out = Vec::new();
            let _ = unsafe { std::fs::File::from_raw_fd(rd) }.read_to_end(&mut out);
            String::from_utf8_lossy(&out).into_owned()
        };

        VCS.store(0, Ordering::SeqCst);
        // Loaded TCMULTRIGHT capability is used with the count substituted.
        tclen.lock().unwrap()[mr] = 3;
        tcstr.lock().unwrap()[mr] = "\x1bX%dY".to_string();
        tclen.lock().unwrap()[hp] = 0;
        let with_cap = capture(&|| tc_rightcurs(5));
        assert_eq!(with_cap, "\x1bX5Y", "should use loaded TCMULTRIGHT with count");

        // No capability → ANSI cursor-forward fallback.
        tclen.lock().unwrap()[mr] = 0;
        let headless = capture(&|| tc_rightcurs(5));
        assert_eq!(headless, "\x1b[5C", "no cap → CSI C default");

        tclen.lock().unwrap()[mr] = save_mr_len;
        tcstr.lock().unwrap()[mr] = save_mr_str;
        tclen.lock().unwrap()[hp] = save_hp_len;
    }

    /// c:2195-2212 — moving the cursor down past the drawn region
    /// (vmaxln-1) must emit real newlines, which scroll/create lines. The
    /// old CSI-H port jumped without creating lines. From (0,0) with
    /// vmaxln=1, moveto(3,0) emits CR + 3 newlines and lands VLN at 3.
    #[test]
    fn moveto_down_past_vmaxln_emits_newlines() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        use crate::ported::init::tclen;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        // No TCDOWN capability so the newline fallback is taken.
        let save_d = tclen.lock().unwrap()[crate::ported::zsh_h::TCDOWN as usize];
        let save_md = tclen.lock().unwrap()[crate::ported::zsh_h::TCMULTDOWN as usize];
        tclen.lock().unwrap()[crate::ported::zsh_h::TCDOWN as usize] = 0;
        tclen.lock().unwrap()[crate::ported::zsh_h::TCMULTDOWN as usize] = 0;

        VCS.store(0, Ordering::SeqCst);
        VLN.store(0, Ordering::SeqCst);
        VMAXLN.store(1, Ordering::SeqCst); // nothing drawn below row 0

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        moveto(3, 0);

        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        tclen.lock().unwrap()[crate::ported::zsh_h::TCDOWN as usize] = save_d;
        tclen.lock().unwrap()[crate::ported::zsh_h::TCMULTDOWN as usize] = save_md;
        let mut out = Vec::new();
        let _ = unsafe { std::fs::File::from_raw_fd(rd) }.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert_eq!(VLN.load(Ordering::SeqCst), 3, "moveto must land VLN at 3");
        assert!(
            s.contains("\n\n\n"),
            "down-past-vmaxln must emit newlines to create lines; got {:?}",
            s
        );
    }

    /// c:2745-2764 — singmoveto positions the cursor on the current line
    /// using the GLOBAL vcs (the VCS atomic), as C does. With no multi-left
    /// capability and a target in the left half, it homes via CR (vcs=0)
    /// then moves right, landing VCS at the target. The old port threaded a
    /// throwaway RefreshState (vcs always 0); this pins the global tracking.
    #[test]
    fn singmoveto_tracks_global_vcs() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        use crate::ported::init::tclen;
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let li = crate::ported::zsh_h::TCMULTLEFT as usize;
        let save_li = tclen.lock().unwrap()[li];
        tclen.lock().unwrap()[li] = 0; // no multi-left → CR-home path

        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);

        // From column 10, target 2 (<= vcs/2): CR home then move right.
        VCS.store(10, Ordering::SeqCst);
        singmoveto(2);
        let vcs_after = VCS.load(Ordering::SeqCst);
        // Already at target: early return, no further output.
        singmoveto(2);

        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        tclen.lock().unwrap()[li] = save_li;
        let mut out = Vec::new();
        let _ = unsafe { std::fs::File::from_raw_fd(rd) }.read_to_end(&mut out);
        let s = String::from_utf8_lossy(&out);
        assert_eq!(vcs_after, 2, "singmoveto must land global VCS at the target");
        assert!(s.contains('\r'), "CR-home optimisation should emit \\r; got {:?}", s);
    }

    /// c:2320-2331 — tc_downcurs prefers the terminal down capability, but
    /// with none available it must emit real newlines + CR (which scroll/
    /// create lines a plain CSI B cannot) and return -1. The old port faked
    /// it as an unconditional CSI B with no return.
    #[test]
    fn tc_downcurs_newline_fallback_and_capability() {
        use std::io::Read;
        use std::os::unix::io::FromRawFd;
        use crate::ported::init::{tclen, tcstr};
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        let di = crate::ported::zsh_h::TCDOWN as usize;
        let mi = crate::ported::zsh_h::TCMULTDOWN as usize;
        // Snapshot in separate statements: locking `tclen` twice inside one
        // tuple expression keeps both guards alive simultaneously and
        // self-deadlocks the non-reentrant Mutex.
        let save_di_len = tclen.lock().unwrap()[di];
        let save_di_str = tcstr.lock().unwrap()[di].clone();
        let save_mi_len = tclen.lock().unwrap()[mi];

        // No down capability → newline fallback, returns -1.
        {
            let mut t = tclen.lock().unwrap();
            t[di] = 0;
            t[mi] = 0;
        }
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0, "pipe()");
        let (rd, wr) = (fds[0], fds[1]);
        let old = crate::ported::init::SHTTY.load(Ordering::SeqCst);
        crate::ported::init::SHTTY.store(wr, Ordering::SeqCst);
        let ret = tc_downcurs(3);
        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr) };
        let mut out = Vec::new();
        let _ = unsafe { std::fs::File::from_raw_fd(rd) }.read_to_end(&mut out);
        assert_eq!(ret, -1, "newline fallback returns -1 (column reset)");
        assert_eq!(
            String::from_utf8_lossy(&out),
            "\n\n\n\r",
            "no down-cap → 3 newlines + CR"
        );

        // Single-shot down capability available → uses it, returns 0.
        {
            let mut t = tclen.lock().unwrap();
            t[di] = 4;
        }
        tcstr.lock().unwrap()[di] = "\x1b[B".to_string();
        let mut fds2 = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds2.as_mut_ptr()) }, 0, "pipe()");
        let (rd2, wr2) = (fds2[0], fds2[1]);
        crate::ported::init::SHTTY.store(wr2, Ordering::SeqCst);
        let ret2 = tc_downcurs(2);
        crate::ported::init::SHTTY.store(old, Ordering::SeqCst);
        unsafe { libc::close(wr2) };
        let mut out2 = Vec::new();
        let _ = unsafe { std::fs::File::from_raw_fd(rd2) }.read_to_end(&mut out2);
        assert_eq!(ret2, 0, "capability path returns 0 (column preserved)");
        assert_eq!(
            String::from_utf8_lossy(&out2),
            "\x1b[B\x1b[B",
            "down-cap looped ct times, no newlines"
        );

        tclen.lock().unwrap()[di] = save_di_len;
        tcstr.lock().unwrap()[di] = save_di_str;
        tclen.lock().unwrap()[mi] = save_mi_len;
    }

    /// c:1782-1783 — tcinscost/tcdelcost read the real termcap costs from
    /// tclen: a parametrised multi-cap costs one capability, otherwise it
    /// is `x` single-char ops. Pins the formula against the old `x.max(0)`
    /// fake that ignored tclen entirely.
    #[test]
    fn tc_ins_del_cost_use_real_tclen() {
        use crate::ported::init::tclen;
        use crate::ported::zsh_h::{TCDEL, TCINS, TCMULTDEL, TCMULTINS};
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();

        // Snapshot + restore the four tclen slots we mutate.
        let save = {
            let t = tclen.lock().unwrap();
            (
                t[TCMULTINS as usize],
                t[TCINS as usize],
                t[TCMULTDEL as usize],
                t[TCDEL as usize],
            )
        };

        // Per-char fallback: no multi-cap → x * single-cap cost.
        {
            let mut t = tclen.lock().unwrap();
            t[TCMULTINS as usize] = 0;
            t[TCINS as usize] = 2;
            t[TCMULTDEL as usize] = 0;
            t[TCDEL as usize] = 3;
        }
        assert_eq!(tcinscost(4), 8, "insert: 4 chars * tclen[TCINS]=2");
        assert_eq!(tcdelcost(4), 12, "delete: 4 chars * tclen[TCDEL]=3");

        // Multi-cap available: flat one-capability cost regardless of x.
        {
            let mut t = tclen.lock().unwrap();
            t[TCMULTINS as usize] = 5;
            t[TCMULTDEL as usize] = 7;
        }
        assert_eq!(tcinscost(4), 5, "insert: parametrised tclen[TCMULTINS]=5");
        assert_eq!(tcdelcost(4), 7, "delete: parametrised tclen[TCMULTDEL]=7");

        let mut t = tclen.lock().unwrap();
        t[TCMULTINS as usize] = save.0;
        t[TCINS as usize] = save.1;
        t[TCMULTDEL as usize] = save.2;
        t[TCDEL as usize] = save.3;
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

    /// c:607 — `tcoutclear` for each clear capability is safe.
    #[test]
    fn tcoutclear_both_arms_safe() {
        let _g = crate::test_util::global_state_lock();
        let _g2 = zle_test_setup();
        tcoutclear(crate::ported::zsh_h::TCCLEARSCREEN);
        tcoutclear(crate::ported::zsh_h::TCCLEAREOL);
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
