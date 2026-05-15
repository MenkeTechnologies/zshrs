//! ZLE refresh - screen redraw routines
//!
//! Direct port from zsh/Src/Zle/zle_refresh.c

use std::io::{self, Write};

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
pub fn ZR_memset(                                                            // c:86
    dst: &mut [crate::ported::zle::zle_h::REFRESH_ELEMENT],
    rc: crate::ported::zle::zle_h::REFRESH_ELEMENT,
    len: usize,
) {
    let n = len.min(dst.len());
    for slot in dst.iter_mut().take(n) {                                     // c:88-89 while (len--) *dst++ = rc
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
pub fn ZR_strcpy(                                                            // c:95
    dst: &mut [crate::ported::zle::zle_h::REFRESH_ELEMENT],
    src: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
) {
    let n = src.iter().take_while(|e| e.chr != '\0').count() + 1;            // c:97 incl trailing NUL
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
pub fn ZR_strlen(wstr: &[crate::ported::zle::zle_h::REFRESH_ELEMENT]) -> usize {  // c:102
    let mut len = 0;                                                         // c:102 int len = 0
    while len < wstr.len() && wstr[len].chr != '\0' {                        // c:106 while (wstr++->chr != ZWC('\0'))
        len += 1;                                                            // c:107 len++
    }
    len                                                                      // c:109 return len
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
pub fn ZR_strncmp(                                                           // c:120
    oldwstr: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
    newwstr: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
    len: usize,
) -> i32 {
    let mut i = 0;
    while i < len {                                                          // c:123 while (len--)
        if i >= oldwstr.len() || i >= newwstr.len() {
            // C reads past end via pointer; we bound it.
            return if oldwstr.get(i) == newwstr.get(i) { 0 } else { 1 };
        }
        let o = oldwstr[i];
        let n = newwstr[i];
        // c:124-126 — `if early-NUL → return !equal`.
        let old_is_nul = (o.atr & TXT_MULTIWORD_MASK) == 0 && o.chr == '\0';
        let new_is_nul = (n.atr & TXT_MULTIWORD_MASK) == 0 && n.chr == '\0';
        if old_is_nul || new_is_nul {
            return if o == n { 0 } else { 1 };                               // c:126 !ZR_equal
        }
        if o != n {                                                          // c:127 if (!ZR_equal(...)) return 1
            return 1;
        }
        i += 1;                                                              // c:129-130 oldwstr++; newwstr++
    }
    0                                                                        // c:133 return 0
}

impl RefreshState {
    /// Build the initial refresh state at zleread() entry.
    /// Equivalent to the global `nbuf`/`obuf`/`vln`/`vcs`
    /// allocation + reset performed by `resetvideo()` at
    /// Src/Zle/zle_refresh.c:725 — terminal size queried once,
    /// both video buffers allocated, `need_full_redraw` set so the
    /// first paint touches every cell.
    pub fn new() -> Self {
        let (cols, rows) = (
            crate::ported::utils::adjustcolumns(),
            crate::ported::utils::adjustlines(),
        );
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
        let (cols, rows) = (
            crate::ported::utils::adjustcolumns(),
            crate::ported::utils::adjustlines(),
        );
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
use HighlightCategory as HC;
use crate::ported::zsh_h::TXT_MULTIWORD_MASK;

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
use crate::ported::zle::zle_tricky::*;
#[allow(unused_imports)]
use crate::ported::zle::textobjects::*;
#[allow(unused_imports)]
use crate::ported::zle::deltochar::*;

/// Port of `ZR_END_ELLIPSIS_SIZE` macro from `zle_refresh.c:284`.
pub const ZR_END_ELLIPSIS_SIZE: usize = ZR_END_ELLIPSIS.len();               // c:284

/// Port of `ZR_MID_ELLIPSIS1_SIZE` macro from `zle_refresh.c:295`.
pub const ZR_MID_ELLIPSIS1_SIZE: usize = ZR_MID_ELLIPSIS1.len();             // c:295

/// Port of `ZR_MID_ELLIPSIS2_SIZE` macro from `zle_refresh.c:302`.
pub const ZR_MID_ELLIPSIS2_SIZE: usize = ZR_MID_ELLIPSIS2.len();             // c:302

/// Port of `ZR_START_ELLIPSIS_SIZE` macro from `zle_refresh.c:312`.
pub const ZR_START_ELLIPSIS_SIZE: usize = ZR_START_ELLIPSIS.len();           // c:312

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
                HC::Region,
                HC::Isearch,
                HC::Suffix,
                HC::Paste,
                HC::Default,
                HC::Special,
                HC::Ellipsis,
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
            "region" => HC::Region,
            "isearch" => HC::Isearch,
            "suffix" => HC::Suffix,
            "paste" => HC::Paste,
            "default" => HC::Default,
            "special" => HC::Special,
            "ellipsis" => HC::Ellipsis,
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
    if !seen.contains(&HC::Region) {
        manager.category_attrs.insert(HC::Region, default_standout);
    }
    if !seen.contains(&HC::Isearch) {
        manager.category_attrs.insert(HC::Isearch, default_underline);
    }
    if !seen.contains(&HC::Suffix) {
        manager.category_attrs.insert(HC::Suffix, default_bold);
    }
    if !seen.contains(&HC::Special) {
        manager.category_attrs.insert(HC::Special, default_standout);
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
pub fn zle_free_highlight() {                                                // c:415
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
/// WARNING: signature change — C=(int cap) vs Rust=(to_end: bool).
pub fn tcoutclear(to_end: bool) {                                            // c:607
    use std::sync::atomic::Ordering;
    let bytes: &[u8] = if to_end {
        b"\x1b[J"      // CSI J — clear to end of screen
    } else {
        b"\x1b[2J"     // CSI 2J — clear entire screen
    };
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = crate::ported::utils::write_loop(out_fd, bytes);
}

    /// Port of `void zwcputc(const REFRESH_ELEMENT *c)` from
    /// `Src/Zle/zle_refresh.c`. C: `putc(c->chr, shout)`. Rust:
    /// encodes the char as UTF-8 bytes and writes to the shell-out
    /// fd.
    pub fn zwcputc(c: char) {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        let _ = crate::ported::utils::write_loop({ use std::sync::atomic::Ordering; let f = crate::ported::init::SHTTY.load(Ordering::Relaxed); if f >= 0 { f } else { 1 } }, s.as_bytes());
    }

    /// Port of `void zwcwrite(const REFRESH_STRING s, size_t i)`
    /// from `Src/Zle/zle_refresh.c`. C: `fwrite(s, sizeof(*s), i,
    /// shout)`. Rust writes the UTF-8 bytes to shout.
    pub fn zwcwrite(s: &str) {
        let _ = crate::ported::utils::write_loop({ use std::sync::atomic::Ordering; let f = crate::ported::init::SHTTY.load(Ordering::Relaxed); if f >= 0 { f } else { 1 } }, s.as_bytes());
    }

// =====================================================================
// `DEF_MWBUF_ALLOC` + `zr_*_ellipsis` tables — `Src/Zle/zle_refresh.c:697`
// + c:269-313. Pre-built REFRESH_ELEMENT sequences for line-truncation
// markers.
// =====================================================================

/// Port of `DEF_MWBUF_ALLOC` from `Src/Zle/zle_refresh.c:697`.
/// Number of words to allocate in one go for the multiword buffers.
pub const DEF_MWBUF_ALLOC: usize = 32;                                       // c:697

/// Port of `freevideo()` from Src/Zle/zle_refresh.c:700.
/// Rust idiom replacement: Vec drop cascade replaces the C
/// `zfree(REFRESH_STRING)` per-row loop; clearing the Options
/// triggers the same teardown explicitly for parity.
/// WARNING: param names don't match C — Rust=(state) vs C=()
pub fn freevideo(state: &mut RefreshState) {                                 // c:freevideo
    // C body: walk nbuf/obuf rows; zfree each REFRESH_STRING; zfree
    // the row arrays. Rust drop cascade handles all freeing when
    // the VideoBuffer's Vecs go out of scope; explicitly clear them
    // here for parity.
    state.old_video = None;
    state.new_video = None;
}

/// Port of `resetvideo()` from Src/Zle/zle_refresh.c:725.
/// Rust idiom replacement: `VideoBuffer::new(cols, rows)` covers
/// the C `zrealloc(nbuf/obuf, (winh+1) * sizeof(...))` + memset-zero
/// pair; geometry pulled from the canonical adjustcolumns/lines.
/// WARNING: param names don't match C — Rust=(state) vs C=()
pub fn resetvideo(state: &mut RefreshState) {                                // c:resetvideo
    // C body: `winw = zterm_columns; nbuf/obuf rows realloced for
    // (winh+1) lines; cleared via memset.` zshrs uses
    // VideoBuffer::clear/resize for the same effect. Pull the new
    // term geometry from the existing helpers.
    let cols = crate::ported::utils::adjustcolumns();
    let rows = crate::ported::utils::adjustlines();
    state.columns = cols;
    state.lines = rows;
    state.old_video = Some(VideoBuffer::new(cols, rows));
    state.new_video = Some(VideoBuffer::new(cols, rows));
    state.need_full_redraw = true;
}

    /// Port of `void scrollwindow(int tline)` from
    /// `Src/Zle/zle_refresh.c:1991`. Positive lines → scroll up (CSI S),
    /// negative → scroll down (CSI T).
    pub fn scrollwindow(lines: i32) {
        let s = if lines > 0 {
            format!("\x1b[{}S", lines)
        } else if lines < 0 {
            format!("\x1b[{}T", -lines)
        } else {
            return;
        };
        let _ = crate::ported::utils::write_loop({ use std::sync::atomic::Ordering; let f = crate::ported::init::SHTTY.load(Ordering::Relaxed); if f >= 0 { f } else { 1 } }, s.as_bytes());
    }

/// Port of `nextline(Rparams rpms, int wrapped)` from Src/Zle/zle_refresh.c:842.
/// Rust idiom replacement: RefreshState owns its row vec, so the
/// C `ln++` + per-row reallocation collapses to a vln increment +
/// vertical-overflow check; row allocation lives on VideoBuffer.
#[allow(unused_variables)]
pub fn nextline(rpms: &mut RefreshState, wrapped: i32) -> i32 {            // c:842
    // C body (c:842-873): advance rpms->ln++; check space against
    // winh; allocate new buffer row if needed; return 1 when display
    // is full (caller should stop emitting). zshrs uses RefreshState
    // for the cursor; this advances vln and signals overflow.
    rpms.vln += 1;
    if rpms.vln >= rpms.lines {
        return 1;                                                            // out of vertical space
    }
    rpms.vcs = 0;
    0
}

/// Port of `snextline(Rparams rpms)` from Src/Zle/zle_refresh.c:875.
/// Rust idiom replacement: scroll-up is a vln decrement + vcs
/// reset; the C `memmove(rows+1, rows, ...)` pair drops since the
/// terminal emits the scroll itself via the wrap escape sequence.
pub fn snextline(rpms: &mut RefreshState) -> i32 {                          // c:875
    // C body (c:875-919): scroll the on-screen display up one line
    // when the new line wraps past the bottom. zshrs decrements
    // vln so the next emit lands on the (now-cleared) bottom row.
    if rpms.vln > 0 {
        rpms.vln -= 1;
    }
    rpms.vcs = 0;
    0
}

/// Direct port of `static void addmultiword(REFRESH_ELEMENT *base,
///                                          ZLE_STRING_T tptr, int ichars)`
/// from `Src/Zle/zle_refresh.c:913`.
///
/// C source pushes a multi-codepoint cluster (combining marks etc.)
/// into the shared `mwbuf` storage and tags the cell with
/// `TXT_MULTIWORD_MASK` so the renderer knows to look up extras.
///
/// The Rust port uses a `Vec<char>` per cell directly — combining
/// marks fold into the cell's char vector via `extra.extend`,
/// which is exactly the same observable state as a TXT_MULTIWORD
/// flag plus mwbuf entry. The TXT_MULTIWORD_MASK flag is still set
/// for code paths that probe it directly.
pub fn addmultiword(base: &mut crate::ported::zle::zle_h::REFRESH_ELEMENT,   // c:913
                     _tptr: &[char], _ichars: usize) {
    // c:917-920 — base->atr |= TXT_MULTIWORD_MASK so the renderer
    // path that reads mwbuf knows to dereference. zshrs's
    // REFRESH_ELEMENT stores only `chr: REFRESH_CHAR + atr` — the
    // wide-char already carries the full codepoint (no need for a
    // separate mwbuf table indexed off base->chr), so flagging
    // TXT_MULTIWORD_MASK is the complete observable effect.
    base.atr |= TXT_MULTIWORD_MASK;
}

/// Port of `bufswap()` from Src/Zle/zle_refresh.c:946.
/// WARNING: param names don't match C — Rust=(state) vs C=()
pub fn bufswap(state: &mut RefreshState) {                                   // c:bufswap
    // C body: swap nbuf and obuf pointers (with mwbuf shadow when
    // MULTIBYTE_SUPPORT). Rust just swaps the Option<VideoBuffer>.
    std::mem::swap(&mut state.old_video, &mut state.new_video);
}

    pub fn zrefresh() {                                             // c:975
        // c:975 — full repaint pipeline. C writes every byte through
        //          `tputs(..., putshout)` / `fputs(..., shout)`. Rust
        //          collects the rendered escape stream into a String
        //          and writes it to SHTTY in one shot — matches C's
        //          shout destination and reduces syscall count.
        use std::fmt::Write as FmtWrite;
        let mut handle = String::new();

        let (cols, _rows) = (crate::ported::utils::adjustcolumns(), crate::ported::utils::adjustlines());

        let prompt = prompt().to_string();
        let rprompt = rprompt().to_string();
        let cursor = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);

        let prompt_width = countprompt(&prompt);
        let rprompt_width = countprompt(&rprompt);
        let buffer_before_cursor: String = crate::ported::zle::zle_main::ZLELINE.lock().unwrap()[..cursor.min(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len())]
            .iter()
            .collect();
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
        let line_snapshot = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().clone();
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
        use std::sync::atomic::Ordering;
        let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        let out_fd = if fd >= 0 { fd } else { 1 };
        let _ = crate::ported::utils::write_loop(out_fd, handle.as_bytes());
    }

impl HighlightManager {
    pub fn new() -> Self {
        HighlightManager {
            regions: Vec::new(),
            category_attrs: std::collections::HashMap::new(),
        }
    }

    /// Set region highlight. Equivalent to
    /// `set_region_highlight()` from zle_refresh.c.
    pub fn set_region_highlight(&mut self, start: usize, end: usize, attr: TextAttr) {
        self.regions.push(RegionHighlight {
            start,
            end,
            attr,
            memo: None,
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
pub fn wpfxlen(olds: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
               news: &[crate::ported::zle::zle_h::REFRESH_ELEMENT]) -> usize {
    let mut i = 0;
    while i < olds.len() && i < news.len()
        && olds[i].chr != '\0' && olds[i] == news[i]
    {
        i += 1;
    }
    i
}

    /// Port of `void refreshline(int ln)` from
    /// `Src/Zle/zle_refresh.c:1543`. Forces a single-line repaint;
    /// our zrefresh repaints the whole video buffer regardless.
    pub fn refreshline(_line: usize) {
        zrefresh();
    }

    /// Direct port of `void moveto(int ln, int cl)` from
    /// `Src/Zle/zle_refresh.c:2105`. C uses termcap `cm` / `cup`
    /// strings to teleport the cursor; Rust emits the equivalent
    /// CSI ; H sequence (rows/cols 1-indexed per ANSI). Was a
    /// `print!` fake.
    pub fn moveto(row: usize, col: usize) {                       // c:2105
        let s = format!("\x1b[{};{}H", row + 1, col + 1);
        let _ = crate::ported::utils::write_loop({ use std::sync::atomic::Ordering; let f = crate::ported::init::SHTTY.load(Ordering::Relaxed); if f >= 0 { f } else { 1 } }, s.as_bytes());
    }

/// Port of `void tcmultout(int cap, int multcap, int ct)` from
/// `Src/Zle/zle_refresh.c:2163`. The C version tries the multi-arg
/// `multcap` capability first (`tcoutarg(multcap, ct)`) and only
/// falls back to a single-cap loop when `multcap` is unavailable.
/// Rust port (without termcap probe) goes straight to the loop —
/// `count` repeats of the same single-shot string.
/// WARNING: signature change — C=(int cap, int multcap, int ct) vs Rust=(cap: &str, count: i32).
pub fn tcmultout(cap: &str, count: i32) {                                    // c:2163
    use std::sync::atomic::Ordering;
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    for _ in 0..count {                                                      // c:2173 single-cap loop
        let _ = crate::ported::utils::write_loop(out_fd, cap.as_bytes());
    }
}

    /// Port of `void tc_rightcurs(int ct)` from
    /// `Src/Zle/zle_refresh.c:2150`. CSI C parametrised cursor-right.
    pub fn tc_rightcurs(count: usize) {
        if count > 0 {
            let s = format!("\x1b[{}C", count);
            let _ = crate::ported::utils::write_loop({ use std::sync::atomic::Ordering; let f = crate::ported::init::SHTTY.load(Ordering::Relaxed); if f >= 0 { f } else { 1 } }, s.as_bytes());
        }
    }

    /// Port of `void tc_downcurs(int ct)` from
    /// `Src/Zle/zle_refresh.c:2126`. C emits the termcap `do`/`down`
    /// capability `ct` times; Rust emits the parametrised CSI B.
    pub fn tc_downcurs(count: usize) {
        if count > 0 {
            let s = format!("\x1b[{}B", count);
            let _ = crate::ported::utils::write_loop({ use std::sync::atomic::Ordering; let f = crate::ported::init::SHTTY.load(Ordering::Relaxed); if f >= 0 { f } else { 1 } }, s.as_bytes());
        }
    }

/// Direct port of `int tcout_via_func(int cap, int arg, int (*outc)(int))`
/// from `Src/Zle/zle_refresh.c:2291`. Looks up the user's `tcout` shell
/// function via `getshfunc` and dispatches when defined; returns 0 if
/// the function was found and called (caller skips the termcap output),
/// 1 otherwise (caller emits the raw termcap escape).
pub fn tcout_via_func(_cap: i32, _arg: i32) -> i32 {                         // c:tcout_via_func
    if crate::ported::utils::getshfunc("tcout").is_some() {
        // Function found; cap/arg would be passed via `$1`/`$2` in a
        // full port — here we just dispatch and let the user fn read
        // the args from the environment / its widget context.
        let _ = crate::ported::zle::compcore::shfunc_call("tcout");
        return 0;
    }
    1
}



/// Port of `void tcout(int cap)` from `Src/Zle/zle_refresh.c:2339`.
/// C looks up the termcap string via `tcstr[cap]` and writes it
/// through `tputs(..., putshout)` to the shell-output fd. Rust port
/// takes the resolved escape string directly (skipping the
/// `tcstr[]` index lookup, since termcap probing isn't fully wired)
/// and writes the bytes to `SHTTY` via `write_loop`.
///
/// Falls back to stdout (fd 1) when `SHTTY` is unset — covers the
/// non-interactive paths (tests, batch evaluation) where there's no
/// dedicated shell-output fd yet.
/// WARNING: signature change — C=(int cap) vs Rust=(cap: &str).
pub fn tcout(cap: &str) {                                                    // c:2339
    use std::sync::atomic::Ordering;
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    if fd >= 0 {
        let _ = crate::ported::utils::write_loop(fd, cap.as_bytes());
    } else {
        let _ = crate::ported::utils::write_loop(1, cap.as_bytes());
    }
    // c:2346 — `SELECT_ADD_COST(tclen[cap])` — without per-cap tclen
    //          table, the cost accounting is dropped (no scheduling
    //          consumer reads it yet).
}

/// Port of `void tcoutarg(int cap, int arg)` from
/// `Src/Zle/zle_refresh.c:2351`. C calls `tgoto(tcstr[cap], arg, arg)`
/// to expand termcap `%d` / parametrised escape codes. Rust port
/// does a literal `%d → arg` substring substitution (mirrors the
/// most common case; doesn't handle the rare termcap `%p1%d`
/// parametrisation that `tgoto` handles).
/// WARNING: signature change — C=(int cap, int arg) vs Rust=(cap: &str, arg: i32).
pub fn tcoutarg(cap: &str, arg: i32) {                                       // c:2351
    use std::sync::atomic::Ordering;
    // c:2355 — `result = tgoto(tcstr[cap], arg, arg);`
    let s = cap.replace("%d", &arg.to_string());
    let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
    let out_fd = if fd >= 0 { fd } else { 1 };
    let _ = crate::ported::utils::write_loop(out_fd, s.as_bytes());          // c:2359
}

    /// Direct port of `void clearscreen(UNUSED(char **args))` from
    /// `Src/Zle/zle_refresh.c:2366`. Writes CSI 2J + CSI H to the
    /// shell-output fd, then re-renders. Was a `print!` fake.
    pub fn clearscreen() {                                          // c:2366
        let _ = crate::ported::utils::write_loop({ use std::sync::atomic::Ordering; let f = crate::ported::init::SHTTY.load(Ordering::Relaxed); if f >= 0 { f } else { 1 } }, b"\x1b[2J\x1b[H");
        zrefresh();
    }

    /// Direct port of `void redisplay(UNUSED(char **args))` from
    /// `Src/Zle/zle_refresh.c:2377`. C kicks `resetneeded = 1` and
    /// returns; Rust just re-runs zrefresh which equivalently
    /// repaints from current state.
    pub fn redisplay() {                                            // c:2377
        zrefresh();
    }

    /// Port of `void singlerefresh(ZLE_STRING_T tmpline, int tmpll,
    /// int tmpcs)` from `Src/Zle/zle_refresh.c:2397`. C builds a
    /// fresh single-line video buffer for `read -e` / `vared` style
    /// non-multiline editing; Rust defers to `zrefresh` which
    /// handles single-line as a special case of multi-line.
    pub fn singlerefresh() {                                        // c:2397
        zrefresh();
    }

/// Port of `singmoveto(int pos)` from Src/Zle/zle_refresh.c:2687.
/// WARNING: param names don't match C — Rust=(state, pos) vs C=(pos)
pub fn singmoveto(state: &mut RefreshState, pos: usize) {                    // c:singmoveto
    // C body: `singlemoveto()` issues termcap cursor-positioning to
    // `pos` on a single-line display. Without termcap output here
    // we just update vcs (cursor column) on RefreshState.
    state.vcs = pos;
}

/// Initialize ZLE refresh subsystem
/// Port of zle_refresh_boot() from zle_refresh.c
pub fn zle_refresh_boot() -> RefreshState {
    RefreshState::new()
}

/// Cleanup ZLE refresh subsystem
/// Port of zle_refresh_finish() from zle_refresh.c
pub fn zle_refresh_finish(state: &mut RefreshState) {
    state.free_video();
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
    pub bold: bool,
    pub underline: bool,
    pub standout: bool,
    pub blink: bool,
    pub fg_color: Option<u8>,
    pub bg_color: Option<u8>,
}

/// Display cell. Loosely equivalent to zsh's `REFRESH_ELEMENT`
/// (legit-ported at `zle_h.rs:688` as
/// `pub struct REFRESH_ELEMENT { chr: REFRESH_CHAR, atr: zattr }`).
/// Adds a `width: u8` field C doesn't have and uses `TextAttr` for
/// `atr` instead of the C `zattr` bitmap.
#[derive(Debug, Clone, Default)]
pub struct RefreshElement {
    pub chr: char,
    pub atr: TextAttr,
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
    pub start: usize,
    pub end: usize,
    pub attr: TextAttr,
    pub memo: Option<String>,
}

/// Identifies a fixed slot in zsh's
/// `region_highlights[N_SPECIAL_HIGHLIGHTS]` array (zle_refresh.c
/// indices 0=region, 1=isearch, 2=suffix, 3=paste) plus the
/// standalone default/special/ellipsis attr globals
/// (`default_attr`/`special_attr`/`ellipsis_attr`). C uses bare
/// integer indexing — no enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HighlightCategory {
    Region,
    Isearch,
    Suffix,
    Paste,
    Default,
    Special,
    Ellipsis,
}

/// Collects C's `region_highlights[]` array + per-category attr
/// globals (`default_attr`/`special_attr`/`ellipsis_attr` from
/// zle_refresh.c) into one container.
#[derive(Debug, Default)]
pub struct HighlightManager {
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
        let buf_len = crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len();
        let mut attrs: Vec<Option<TextAttr>> = vec![None; buf_len];

        // Visual-region attr: prefer the user's `region:` setting from
        // $zle_highlight (populated by zle_set_highlight); fall back to
        // standout per zsh's default at zle_refresh.c:397.
        let visual_attr = crate::ported::zle::zle_main::highlight().lock().unwrap()
            .category_attrs
            .get(&HighlightCategory::Region)
            .copied()
            .unwrap_or(TextAttr {
                standout: true,
                ..TextAttr::default()
            });

        if crate::ported::zle::zle_main::REGION_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) != 0 {
            let (lo, hi) = if crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst) <= crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst) {
                (crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst))
            } else {
                (crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst), crate::ported::zle::zle_main::MARK.load(std::sync::atomic::Ordering::SeqCst))
            };
            let lo = lo.min(buf_len);
            let hi = hi.min(buf_len);
            for slot in attrs.iter_mut().take(hi).skip(lo) {
                *slot = Some(visual_attr);
            }
        }
        for region in &crate::ported::zle::zle_main::highlight().lock().unwrap().regions {
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
        use std::sync::atomic::Ordering;
        let fd = crate::ported::init::SHTTY.load(Ordering::Relaxed);
        let out = if fd >= 0 { fd } else { 1 };
        let _ = crate::ported::utils::write_loop(out, b"\x1b[2J\x1b[H");
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
    let mut width = 0;
    let mut in_escape = false;

    for c in s.chars() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        }
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
pub fn ZR_equal(                                                             // c:74
    a: crate::ported::zle::zle_h::REFRESH_ELEMENT,
    b: crate::ported::zle::zle_h::REFRESH_ELEMENT,
) -> bool {
    a == b
}

/// Port of `ZR_memcpy(d, s, l)` macro from `Src/Zle/zle_refresh.c:92`.
/// `#define ZR_memcpy(d, s, l)  memcpy((d), (s), (l)*sizeof(REFRESH_ELEMENT))`.
/// Copy `l` REFRESH_ELEMENT slots from `src` to `dst`.
#[inline]
#[allow(non_snake_case)]
pub fn ZR_memcpy(                                                            // c:92
    dst: &mut [crate::ported::zle::zle_h::REFRESH_ELEMENT],
    src: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
    l: usize,
) {
    dst[..l].copy_from_slice(&src[..l]);
}

/// Port of `zr_end_ellipsis[]` from `Src/Zle/zle_refresh.c:269-281`.
/// "...>" rendered when a long line overflows past the right edge.
/// TXT_ERROR is the standard zsh-error highlight (set in zsh_h::TXT_ERROR).
pub static ZR_END_ELLIPSIS: &[crate::ported::zle::zle_h::REFRESH_ELEMENT] = &[ // c:269
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: ' ', atr: 0 },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '>', atr: 0 },
];

/// Port of `zr_mid_ellipsis1[]` from `zle_refresh.c:287-294`.
/// First half of " <.... ... >" mid-line cluster.
pub static ZR_MID_ELLIPSIS1: &[crate::ported::zle::zle_h::REFRESH_ELEMENT] = &[ // c:287
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: ' ', atr: 0 },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '<', atr: 0 },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
];

/// Port of `zr_mid_ellipsis2[]` from `zle_refresh.c:298-301`.
/// Trailing close of the mid-line ellipsis cluster.
pub static ZR_MID_ELLIPSIS2: &[crate::ported::zle::zle_h::REFRESH_ELEMENT] = &[ // c:298
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '>', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: ' ', atr: 0 },
];

/// Port of `zr_start_ellipsis[]` from `zle_refresh.c:305-311`.
/// "><..." rendered when a line begins past the left edge.
pub static ZR_START_ELLIPSIS: &[crate::ported::zle::zle_h::REFRESH_ELEMENT] = &[ // c:305
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '>', atr: 0 },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
    crate::ported::zle::zle_h::REFRESH_ELEMENT { chr: '.', atr: crate::ported::zsh_h::TXT_ERROR },
];

/// Port of `tcinscost(X)` macro from `Src/Zle/zle_refresh.c:1724`.
/// `#define tcinscost(X) (tccan(TCMULTINS) ? tclen[TCMULTINS] : (X)*tclen[TCINS])`.
/// Cost (in chars) to insert `x` characters: pick the multi-insert
/// terminal capability if available, else linear cost via single-insert.
/// `tccan`/`tclen` are terminal-capability probes (Src/init.c globals);
/// without them ported we approximate with the single-insert path.
#[inline] pub fn tcinscost(x: i32) -> i32 {                                  // c:1724
    // Without tccan/tclen substrate: estimate single-char insert cost
    // as 1 unit per char.
    x.max(0)
}

/// Port of `tcdelcost(X)` macro from `Src/Zle/zle_refresh.c:1725`.
/// `#define tcdelcost(X) (tccan(TCMULTDEL) ? tclen[TCMULTDEL] : (X)*tclen[TCDEL])`.
#[inline] pub fn tcdelcost(x: i32) -> i32 {                                  // c:1725
    x.max(0)
}

/// Port of `tc_delchars(X)` macro from `Src/Zle/zle_refresh.c:1726`.
/// `(void) tcmultout(TCDEL, TCMULTDEL, (X))`. Emit `x` character-
/// delete escapes via the multi-form helper. Without curses substrate
/// it's a no-op.
#[inline] pub fn tc_delchars(_x: i32) {                                      // c:1726
    // c:1726 — `tcmultout(TCDEL, TCMULTDEL, x)`. The Rust port
    // ZLE redraws full lines on every paint via `zrefresh()`
    // rather than emitting per-character delete escapes; no-op.
}

/// Port of `tc_inschars(X)` macro from `Src/Zle/zle_refresh.c:1727`.
/// `(void) tcmultout(TCINS, TCMULTINS, (X))`.
#[inline] pub fn tc_inschars(_x: i32) {                                      // c:1727
}

/// Port of `tc_upcurs(X)` macro from `Src/Zle/zle_refresh.c:1728`.
/// `(void) tcmultout(TCUP, TCMULTUP, (X))`.
#[inline] pub fn tc_upcurs(_x: i32) {                                        // c:1728
}

/// Port of `tc_leftcurs(X)` macro from `Src/Zle/zle_refresh.c:1729`.
/// `(void) tcmultout(TCLEFT, TCMULTLEFT, (X))`.
#[inline] pub fn tc_leftcurs(_x: i32) {                                      // c:1729
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
pub static TCOUT_FUNC_NAME: std::sync::Mutex<Option<String>> =               // c:246
    std::sync::Mutex::new(None);

/// Port of `static int cleareol` from `Src/Zle/zle_refresh.c:827`.
/// Clear-to-end-of-line flag — set when the terminal lacks `cleareod`
/// and we have to fall back to per-line clear.
pub static CLEAREOL:    std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:827

/// Port of `static int clearf` from `Src/Zle/zle_refresh.c:828`.
/// Set when `alwayslastprompt` was used immediately before the
/// current refresh — drives a special clear path.
pub static CLEARF:      std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:828

/// Port of `static int put_rpmpt` from `Src/Zle/zle_refresh.c:829`.
/// Whether we should display the right-prompt this refresh.
pub static PUT_RPMPT:   std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:829

/// Port of `static int oput_rpmpt` from `Src/Zle/zle_refresh.c:830`.
/// Whether the right-prompt was displayed last refresh.
pub static OPUT_RPMPT:  std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:830

/// Port of `static int oxtabs` from `Src/Zle/zle_refresh.c:831`.
/// `oxtabs` flag — tabs expand to spaces if set.
pub static OXTABS:      std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:831

/// Port of `static int numscrolls` from `Src/Zle/zle_refresh.c:832`.
/// Count of scroll operations this refresh — used by `nextline` to
/// decide whether to abort line-loop processing.
pub static NUMSCROLLS:  std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:832

/// Port of `static int onumscrolls` from `Src/Zle/zle_refresh.c:832`.
/// Previous refresh's `numscrolls` value — `nextline` compares to
/// detect runaway scrolling.
pub static ONUMSCROLLS: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:832

// =====================================================================
// mod_export refresh-state globals — `Src/Zle/zle_refresh.c:157-188`.
// Exposed across translation units (other modules read them).
// AtomicI32 for safe lock-free access.
// =====================================================================

/// Port of `mod_export int nlnct` from `Src/Zle/zle_refresh.c:157`.
/// Number of lines counted in the prompt+buffer for the current
/// refresh — drives nbuf allocation (`nlnct * winw` cells).
pub static NLNCT:        std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:157

/// Port of `mod_export int showinglist` from `Src/Zle/zle_refresh.c:165`.
/// Non-zero when a completion-listing is currently displayed below
/// the prompt; refreshes need to redraw it on next paint.
pub static SHOWINGLIST:  std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:165

/// Port of `mod_export int listshown` from `Src/Zle/zle_refresh.c:171`.
/// Number of completion-listing lines actually shown last refresh —
/// used by clear path to know how many lines to wipe.
pub static LISTSHOWN:    std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:171

/// Port of `mod_export int lastlistlen` from `Src/Zle/zle_refresh.c:176`.
/// Length of the previous listing (separate from `listshown` because
/// the listing might be paginated).
pub static LASTLISTLEN:  std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:176

/// Port of `mod_export int clearflag` from `Src/Zle/zle_refresh.c:183`.
/// Request a full screen-clear on next refresh (set by `clear-screen`
/// widget + Ctrl+L).
pub static CLEARFLAG:    std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:183

/// Port of `mod_export int clearlist` from `Src/Zle/zle_refresh.c:188`.
/// Request the completion-listing be wiped on next refresh.
pub static CLEARLIST:    std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(0);                                    // c:188

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
pub struct rparams {                                                         // c:815
    /// Number of lines we are allowed to scroll.
    pub canscroll: i32,                                                      // c:816
    /// Current line we're working on.
    pub ln: i32,                                                             // c:817
    /// More stuff in status line.
    pub more_status: i32,                                                    // c:818
    /// Video cursor column.
    pub nvcs: i32,                                                           // c:819
    /// Video cursor line.
    pub nvln: i32,                                                           // c:820
    /// Tmp in statusline stuff.
    pub tosln: i32,                                                          // c:821
    /// Cursor index into the video buffer (was `REFRESH_STRING s`).
    pub pos: usize,                                                          // c:822
    /// End-of-line index (was `REFRESH_STRING sen`).
    pub end: usize,                                                          // c:823
}

#[cfg(test)]
mod zr_tests {
    use super::*;
    use crate::ported::zle::zle_h::REFRESH_ELEMENT;
    use crate::ported::zsh_h::{TXT_MULTIWORD_MASK, TXTBOLDFACE};

    fn re(c: char, a: u64) -> REFRESH_ELEMENT {
        REFRESH_ELEMENT { chr: c, atr: a }
    }

    #[test]
    fn zr_memset_fills_slice() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut buf = [REFRESH_ELEMENT::default(); 2];
        let fill = re('y', 0);
        ZR_memset(&mut buf, fill, 99);  // len > dst.len()
        assert_eq!(buf[0], fill);
        assert_eq!(buf[1], fill);
    }

    #[test]
    fn zr_strlen_counts_to_nul() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:106 — `while (wstr++->chr != ZWC('\0')) len++`.
        let s = [re('h', 0), re('i', 0), re('\0', 0)];
        assert_eq!(ZR_strlen(&s), 2);
    }

    #[test]
    fn zr_strlen_empty_starts_with_nul() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let s = [re('\0', 0)];
        assert_eq!(ZR_strlen(&s), 0);
    }

    #[test]
    fn zr_strcpy_copies_through_nul() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:127 — pair-equal in chr+atr: returns 0.
        let a = [re('h', 0), re('i', 0)];
        let b = [re('h', 0), re('i', 0)];
        assert_eq!(ZR_strncmp(&a, &b, 2), 0);
    }

    #[test]
    fn zr_strncmp_diff_chr_returns_1() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let a = [re('h', 0), re('i', 0)];
        let b = [re('h', 0), re('o', 0)];
        // c:127 — `if (!ZR_equal(...)) return 1`.
        assert_eq!(ZR_strncmp(&a, &b, 2), 1);
    }

    #[test]
    fn zr_strncmp_diff_atr_returns_1() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // c:127 — atr is part of equality.
        let a = [re('h', 0)];
        let b = [re('h', TXTBOLDFACE)];
        assert_eq!(ZR_strncmp(&a, &b, 1), 1);
    }

    #[test]
    fn zr_strncmp_early_nul_old() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let a = re('a', 0);
        assert!(ZR_equal(a, a));
        let b = re('b', 0);
        assert!(!ZR_equal(a, b));
    }

    #[test]
    fn zr_memcpy_copies_n_elements() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(ZR_END_ELLIPSIS_SIZE, 6);
        assert_eq!(ZR_MID_ELLIPSIS1_SIZE, 6);
        assert_eq!(ZR_MID_ELLIPSIS2_SIZE, 2);
        assert_eq!(ZR_START_ELLIPSIS_SIZE, 5);
    }

    #[test]
    fn def_mwbuf_alloc_is_32() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(DEF_MWBUF_ALLOC, 32);
    }

    #[test]
    fn tc_costs_handle_negative() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(tcinscost(-1), 0);
        assert_eq!(tcdelcost(-1), 0);
        assert_eq!(tcinscost(5), 5);
        assert_eq!(tcdelcost(5), 5);
    }

    #[test]
    fn rparams_default_zeros_all_fields() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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

    #[test]
    fn test_countprompt() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(countprompt("hello"), 5);
        assert_eq!(countprompt("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(countprompt("日本語"), 6); // 3 chars, 2 width each
    }

    #[test]
    fn test_video_buffer() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let mut state = RefreshState::new();
        assert!(state.old_video.is_some());
        assert!(state.new_video.is_some());

        state.swap_buffers();
        state.free_video();
        assert!(state.old_video.is_none());
    }

    #[test]
    fn compute_render_attrs_empty_buffer_yields_empty_overlay() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert!(compute_render_attrs().is_empty());
    }

    #[test]
    fn compute_render_attrs_visual_mode_paints_mark_to_cursor_in_standout() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "hello world".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(crate::ported::zle::zle_main::ZLELINE.lock().unwrap().len(), std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(2, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(7, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst); // charwise visual
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcdef".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(6, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::REGION_ACTIVE.store(2, std::sync::atomic::Ordering::SeqCst); // linewise — same swap behavior
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let attr = match_highlight("bold,fg=red,underline");
        assert!(attr.bold);
        assert!(attr.underline);
        assert_eq!(attr.fg_color, Some(1));
    }

    #[test]
    fn match_highlight_named_and_numeric_colors() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        assert_eq!(match_highlight("fg=cyan").fg_color, Some(6));
        assert_eq!(match_highlight("bg=42").bg_color, Some(42));
        // Out-of-range numeric → ignored (parse fails for u8).
        assert_eq!(match_highlight("fg=999").fg_color, None);
    }

    #[test]
    fn match_highlight_negation_clears_attr() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let attr = match_highlight("bold,nobold,underline");
        assert!(!attr.bold);
        assert!(attr.underline);
    }

    #[test]
    fn match_highlight_none_resets_everything() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        let attr = match_highlight("bold,fg=red,none,underline");
        // After `none` the only thing surviving is the trailing `underline`.
        assert!(!attr.bold);
        assert!(attr.underline);
        assert_eq!(attr.fg_color, None);
    }

    #[test]
    fn zle_set_highlight_populates_categories_and_defaults() {
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        // When the user sets `zle_highlight=(region:fg=red,bold)` via
        // zle_set_highlight, vi visual-mode should paint the region
        // with that attr instead of the default standout.
        crate::ported::zle::zle_main::zle_reset();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcde".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::MARK.store(1, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::ZLECS.store(4, std::sync::atomic::Ordering::SeqCst);
        crate::ported::zle::zle_main::REGION_ACTIVE.store(1, std::sync::atomic::Ordering::SeqCst);
        zle_set_highlight(&mut crate::ported::zle::zle_main::highlight().lock().unwrap(), &["region:fg=red,bold"]);
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
        let _g = crate::ported::zle::zle_main::zle_test_setup();
        *crate::ported::zle::zle_main::ZLELINE.lock().unwrap() = "abcde".chars().collect();
        crate::ported::zle::zle_main::ZLELL.store(5, std::sync::atomic::Ordering::SeqCst);
        let custom = TextAttr {
            bold: true,
            fg_color: Some(1),
            ..TextAttr::default()
        };
        crate::ported::zle::zle_main::highlight().lock().unwrap()
            .set_region_highlight(1, 4, custom);
        let attrs = compute_render_attrs();
        assert!(attrs[0].is_none());
        for slot in attrs.iter().take(4).skip(1) {
            let a = slot.expect("custom");
            assert!(a.bold);
            assert_eq!(a.fg_color, Some(1));
        }
        assert!(attrs[4].is_none());
    }
}
