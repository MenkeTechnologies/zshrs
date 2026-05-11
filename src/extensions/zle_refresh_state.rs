//! Rust-only refresh-engine helpers used by zle_refresh.rs paint
//! routines.
//!
//! **None of these types are ports — zsh's C source represents the
//! same state differently and the pending unification is a separate
//! work item.** See per-type docs for the C-side mapping.
//!
//! Why these live here instead of in `src/ported/zle/zle_refresh.rs`:
//! the drift gate enforces that types under `src/ported/` mirror
//! upstream C structs. Each of these is a higher-level Rust
//! abstraction over C's flat-array + bitmap-attr model — adequate
//! for the current paint path, but blocking a faithful port.
//!
//! Full migration target:
//!   - `TextAttr` → `zattr` (u64 packed flag bitmap from
//!     Src/zsh.h:2685 — `pub type zattr = u64` already exists)
//!   - `RefreshElement` → `crate::ported::zle::zle_h::REFRESH_ELEMENT`
//!   - `VideoBuffer` → raw `Vec<REFRESH_ELEMENT>` for `nbuf`/`obuf`
//!     globals matching Src/Zle/zle_refresh.c:114-115
//!   - `RefreshState` → discrete file-scope statics (`winw`, `winh`,
//!     `vcs`, `vln`, `lpromptw`, `rpromptw`, etc. — already partially
//!     ported elsewhere in zle_refresh.rs)
//!   - `RegionHighlight` → `crate::ported::zle::zle_h::region_highlight`
//!     (already legit-ported at zle_h.rs:613 with the C-shape fields)
//!   - `HighlightCategory` → bare i32 + `N_SPECIAL_HIGHLIGHTS` index
//!     constants
//!   - `HighlightManager` → discrete file-scope statics matching
//!     `region_highlights[]` + `default_attr`/`special_attr`/
//!     `ellipsis_attr` globals

// Text attributes after displaying prompts                                 // c:149
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
