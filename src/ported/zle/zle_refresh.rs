//! ZLE refresh - screen redraw routines
//!
//! Direct port from zsh/Src/Zle/zle_refresh.c

use std::io::{self, Write};

use super::zle_main::Zle;

// Text attributes after displaying prompts                                 // c:149
/// Text attributes for display
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
    /// Render this attribute set as the corresponding ANSI SGR escape.
    /// Mirrors `tsetcap()` from Src/Zle/zle_refresh.c which the C
    /// source uses to emit termcap-derived attribute changes — we
    /// emit the literal CSI codes since modern terminals handle them
    /// uniformly.
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

/// A single display element (character + attributes)
#[derive(Debug, Clone, Default)]
pub struct RefreshElement {
    pub chr: char,
    pub atr: TextAttr,
    pub width: u8,
}

impl RefreshElement {
    /// Construct a refresh cell holding a single character with default
    /// attributes. Equivalent to a freshly-zeroed `REFRESH_ELEMENT`
    /// from Src/Zle/zle_refresh.h — the C source uses this struct to
    /// represent each on-screen cell during the diff/paint cycle in
    /// zrefresh.
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
    /// VideoBuffer — same shape as the per-cell attr write that
    /// `zrefresh()` performs at Src/Zle/zle_refresh.c when applying
    /// region_highlights[] to each cell.
    pub fn with_attr(chr: char, atr: TextAttr) -> Self {
        let width = unicode_width::UnicodeWidthChar::width(chr).unwrap_or(1) as u8;
        RefreshElement { chr, atr, width }
    }
}

/// Video buffer for screen state
#[derive(Debug, Clone)]
pub struct VideoBuffer {
    /// Buffer contents - 2D array of lines
    pub lines: Vec<Vec<RefreshElement>>,
    /// Number of columns
    pub cols: usize,
    /// Number of rows
    pub rows: usize,
}

impl VideoBuffer {
    /// Allocate a fresh video buffer of `cols × rows` filled with
    /// blank cells.
    /// Equivalent to `resetvideo()` at Src/Zle/zle_refresh.c:725 —
    /// the C source allocates `nlnct * winw` cells for the `nbuf`
    /// array each refresh.
    pub fn new(cols: usize, rows: usize) -> Self {
        let lines = vec![vec![RefreshElement::new(' '); cols]; rows];
        VideoBuffer { lines, cols, rows }
    }

    /// Reset every cell to a blank-attribute space.
    /// Used by `zrefresh()` between frames to wipe the working buffer
    /// before the new paint pass — see the loop at zle_refresh.c around
    /// `freevideo()` (zle_refresh.c:700) which serves the same role.
    pub fn clear(&mut self) {
        for line in &mut self.lines {
            for elem in line.iter_mut() {
                *elem = RefreshElement::new(' ');
            }
        }
    }

    /// Reshape the buffer for a new terminal size.
    /// Equivalent to the cols/lines update + `nbuf`/`obuf` reallocation
    /// chain in zle_refresh.c that fires on SIGWINCH (see the
    /// `winw`/`winh` re-read in `zrefresh()` at zle_refresh.c:975).
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
    /// the C source's index path is unchecked (uses winw/nlnct
    /// invariants), but our safe variant lets host code probe.
    pub fn get(&self, row: usize, col: usize) -> Option<&RefreshElement> {
        self.lines.get(row).and_then(|line| line.get(col))
    }
}

/// Refresh parameters
#[derive(Debug, Clone, Default)]
pub struct RefreshState {
    /// Number of columns
    pub columns: usize,  // winw, window width                              // c:682
    /// Number of lines  
    pub lines: usize,  // winh, window height                               // c:682
    /// Current line on screen (cursor row)
    pub vln: usize,  // video cursor position line                          // c:680
    /// Current column on screen (cursor col)
    pub vcs: usize,  // video cursor position column                        // c:680
    /// Prompt width (left)
    pub lpromptw: usize,  // prompt widths on screen                        // c:676
    /// Right prompt width
    pub rpromptw: usize,  // prompt widths on screen                        // c:676
    /// Scroll offset for horizontal scrolling
    pub scrolloff: usize,
    /// Region highlight start
    pub region_highlight_start: Option<usize>,
    /// Region highlight end
    pub region_highlight_end: Option<usize>,
    /// Old video buffer
    pub old_video: Option<VideoBuffer>,
    /// New video buffer
    pub new_video: Option<VideoBuffer>,
    /// Prompt string (left)
    pub lpromptbuf: String,
    /// Right prompt string
    pub rpromptbuf: String,
    /// Whether we need full redraw
    pub need_full_redraw: bool,
    /// Predisplay string (before main buffer)
    pub predisplay: String,
    /// Postdisplay string (after main buffer)
    pub postdisplay: String,
}

impl RefreshState {
    /// Build the initial refresh state at zleread() entry.
    /// Equivalent to the global `nbuf`/`obuf`/`vln`/`vcs` allocation +
    /// reset performed by `resetvideo()` at Src/Zle/zle_refresh.c:725
    /// — terminal size queried once, both video buffers allocated,
    /// `need_full_redraw` set so the first paint touches every cell.
    pub fn new() -> Self {
        let (cols, rows) = (crate::ported::utils::adjustcolumns(), crate::ported::utils::adjustlines());
        RefreshState {
            columns: cols,
            lines: rows,
            old_video: Some(VideoBuffer::new(cols, rows)),
            new_video: Some(VideoBuffer::new(cols, rows)),
            need_full_redraw: true,
            ..Default::default()
        }
    }

    /// Reallocate the video buffers for the current terminal size and
    /// arm a full redraw on the next paint.
    /// Port of `resetvideo()` from Src/Zle/zle_refresh.c:725 invoked
    /// after SIGWINCH (the C source calls it from
    /// `adjustwinsize()` in Src/init.c).
    pub fn reset_video(&mut self) {
        let (cols, rows) = (crate::ported::utils::adjustcolumns(), crate::ported::utils::adjustlines());
        self.columns = cols;
        self.lines = rows;
        self.old_video = Some(VideoBuffer::new(cols, rows));
        self.new_video = Some(VideoBuffer::new(cols, rows));
        self.need_full_redraw = true;
    }

    /// Drop both video buffers — used at ZLE shutdown.
    /// Port of `freevideo()` from Src/Zle/zle_refresh.c:700.
    pub fn free_video(&mut self) {                                           // c:700
        self.old_video = None;
        self.new_video = None;
    }

    /// Promote the freshly-painted buffer to "previously displayed" and
    /// clear the new-buffer slate for the next frame.
    /// Port of `bufswap()` from Src/Zle/zle_refresh.c:946 — the C
    /// source swaps `nbuf` and `obuf` pointers and zeroes the new
    /// nbuf so the diff loop has a clean target.
    pub fn swap_buffers(&mut self) {
        std::mem::swap(&mut self.old_video, &mut self.new_video);
        if let Some(ref mut new) = self.new_video {
            new.clear();
        }
    }
}

impl Zle {
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
    pub fn zrefresh(&mut self) {                                             // c:975
        let stdout = io::stdout();
        let mut handle = stdout.lock();

        let (cols, _rows) = (crate::ported::utils::adjustcolumns(), crate::ported::utils::adjustlines());

        let prompt = self.prompt().to_string();
        let rprompt = self.rprompt().to_string();
        let cursor = self.zlecs;

        let prompt_width = countprompt(&prompt);
        let rprompt_width = countprompt(&rprompt);
        let buffer_before_cursor: String = self.zleline[..cursor.min(self.zleline.len())]
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
        let attrs = self.compute_render_attrs();

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
        for (written, (idx, ch)) in self
            .zleline
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

        let _ = handle.flush();
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
    pub fn compute_render_attrs(&self) -> Vec<Option<TextAttr>> {
        let buf_len = self.zleline.len();
        let mut attrs: Vec<Option<TextAttr>> = vec![None; buf_len];

        // Visual-region attr: prefer the user's `region:` setting from
        // $zle_highlight (populated by zle_set_highlight); fall back to
        // standout per zsh's default at zle_refresh.c:397.
        let visual_attr = self
            .highlight
            .category_attrs
            .get(&HighlightCategory::Region)
            .copied()
            .unwrap_or(TextAttr {
                standout: true,
                ..TextAttr::default()
            });

        if self.region_active != 0 {
            let (lo, hi) = if self.mark <= self.zlecs {
                (self.mark, self.zlecs)
            } else {
                (self.zlecs, self.mark)
            };
            let lo = lo.min(buf_len);
            let hi = hi.min(buf_len);
            for slot in attrs.iter_mut().take(hi).skip(lo) {
                *slot = Some(visual_attr);
            }
        }
        for region in &self.highlight.regions {
            let start = region.start.min(buf_len);
            let end = region.end.min(buf_len);
            for slot in attrs.iter_mut().take(end).skip(start) {
                *slot = Some(region.attr);
            }
        }
        attrs
    }

    /// Full screen refresh - clears and redraws everything
    pub fn full_refresh(&mut self) -> io::Result<()> {
        print!("\x1b[2J\x1b[H");
        self.zrefresh();
        io::stdout().flush()
    }

    /// Partial refresh (optimize for minimal updates)
    pub fn partial_refresh(&mut self) -> io::Result<()> {
        self.zrefresh();
        io::stdout().flush()
    }

    /// Clear the screen
    /// Port of clearscreen() from zle_refresh.c
    pub fn clearscreen(&mut self) {                                          // c:2366
        print!("\x1b[2J\x1b[H");
        let _ = io::stdout().flush();
        self.zrefresh();
    }

    /// Redisplay the current line
    /// Port of redisplay() from zle_refresh.c
    pub fn redisplay(&mut self) {                                            // c:2377
        self.zrefresh();
    }

    // move the cursor to line ln (relative to the prompt line),            // c:2100
    // absolute column cl; update vln, vcs - video line and column          // c:2101
    /// Move cursor to position
    /// Port of moveto() from zle_refresh.c
    pub fn moveto(&mut self, row: usize, col: usize) {                       // c:2105
        // ANSI escape: ESC [ row ; col H (1-indexed)
        print!("\x1b[{};{}H", row + 1, col + 1);
        let _ = io::stdout().flush();
    }

    /// Move cursor down
    /// Port of tc_downcurs() from zle_refresh.c  
    pub fn tc_downcurs(&mut self, count: usize) {
        if count > 0 {
            print!("\x1b[{}B", count);
            let _ = io::stdout().flush();
        }
    }

    /// Move cursor right
    /// Port of tc_rightcurs() from zle_refresh.c
    pub fn tc_rightcurs(&mut self, count: usize) {
        if count > 0 {
            print!("\x1b[{}C", count);
            let _ = io::stdout().flush();
        }
    }

    /// Scroll window up
    /// Port of scrollwindow() from zle_refresh.c
    pub fn scrollwindow(&mut self, lines: i32) {
        if lines > 0 {
            // Scroll up
            print!("\x1b[{}S", lines);
        } else if lines < 0 {
            // Scroll down
            print!("\x1b[{}T", -lines);
        }
        let _ = io::stdout().flush();
    }

    /// Single line refresh
    /// Port of singlerefresh() from zle_refresh.c
    pub fn singlerefresh(&mut self) {                                        // c:2397
        self.zrefresh();
    }

    /// Refresh a single line
    /// Port of refreshline() from zle_refresh.c
    pub fn refreshline(&mut self, _line: usize) {
        self.zrefresh();
    }

    /// Write a wide character
    /// Port of zwcputc() from zle_refresh.c
    pub fn zwcputc(&self, c: char) {
        print!("{}", c);
    }

    /// Write a string of wide characters
    /// Port of zwcwrite() from zle_refresh.c
    pub fn zwcwrite(&self, s: &str) {
        print!("{}", s);
    }
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

/// Region highlight entry
#[derive(Debug, Clone)]
pub struct RegionHighlight {
    pub start: usize,
    pub end: usize,
    pub attr: TextAttr,
    pub memo: Option<String>,
}

/// Highlight category — fixed slots that mirror zsh's
/// `region_highlights[N_SPECIAL_HIGHLIGHTS]` indices in
/// Src/Zle/zle_refresh.c (0=region, 1=isearch, 2=suffix, 3=paste) plus
/// the standalone `default` / `special` / `ellipsis` attrs that the C
/// source tracks as separate globals (`default_attr`, `special_attr`,
/// `ellipsis_attr`).
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

/// Highlight manager
#[derive(Debug, Default)]
pub struct HighlightManager {
    pub regions: Vec<RegionHighlight>,
    /// Per-category attrs from `$zle_highlight`. Index by `HighlightCategory`.
    /// Port of the per-slot atr storage in `region_highlights[]` and the
    /// `default_attr`/`special_attr`/`ellipsis_attr` globals in
    /// Src/Zle/zle_refresh.c — populated by `zle_set_highlight()` (the
    /// freestanding port in this file).
    pub category_attrs: std::collections::HashMap<HighlightCategory, TextAttr>,
}

impl HighlightManager {
    pub fn new() -> Self {
        HighlightManager {
            regions: Vec::new(),
            category_attrs: std::collections::HashMap::new(),
        }
    }

    /// Set region highlight
    /// Port of set_region_highlight() from zle_refresh.c
    pub fn set_region_highlight(&mut self, start: usize, end: usize, attr: TextAttr) {
        self.regions.push(RegionHighlight {
            start,
            end,
            attr,
            memo: None,
        });
    }

    /// Get region highlight for position
    /// Port of get_region_highlight() from zle_refresh.c  
    pub fn get_region_highlight(&self, pos: usize) -> Option<&RegionHighlight> {
        self.regions.iter().find(|r| pos >= r.start && pos < r.end)
    }

    /// Unset region highlight
    /// Port of unset_region_highlight() from zle_refresh.c
    pub fn unset_region_highlight(&mut self) {
        self.regions.clear();
    }

    /// Free highlight resources
    /// Port of zle_free_highlight() from zle_refresh.c
    pub fn free(&mut self) {
        self.regions.clear();
    }
}

/// Terminal output functions. Port of tcout() family from zle_refresh.c.
pub fn tcout(cap: &str) {
    print!("{}", cap);
}

pub fn tcoutarg(cap: &str, arg: i32) {
    // Simple substitution for %d in capability string
    let s = cap.replace("%d", &arg.to_string());
    print!("{}", s);
}

pub fn tcmultout(cap: &str, count: i32) {
    for _ in 0..count {
        print!("{}", cap);
    }
}

pub fn tcoutclear(to_end: bool) {
    if to_end {
        print!("\x1b[J"); // Clear to end of screen
    } else {
        print!("\x1b[2J"); // Clear entire screen
    }
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
                if let Some(rest) = other.strip_prefix("fg=") {
                    attr.fg_color = match_colour(rest);
                } else if let Some(rest) = other.strip_prefix("bg=") {
                    attr.bg_color = match_colour(rest);
                }
                // Anything else (hl=, layer=, opacity=, unknown name) is
                // silently dropped — same as the C source's "found = 0"
                // exit path at prompt.c:2122 when no clause matched.
            }
        }
    }
    attr
}

/// Parse a colour token (named or numeric) into a 256-colour palette index.
/// Mirrors the eight ANSI base names + 256-colour numeric form supported
/// by `match_colour()` (Src/prompt.c, called from `match_highlight`). The
/// 24-bit `#rrggbb` form and `bright-foo` aliases are not surfaced.
fn match_colour(name: &str) -> Option<u8> {
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
}

/// Apply a `$zle_highlight` array to the manager.
/// Port of `zle_set_highlight()` from Src/Zle/zle_refresh.c:322. Walks
/// each `category:spec` entry, parses the spec via `match_highlight`,
/// and stores it in `category_attrs`. Categories not mentioned keep the
/// zsh defaults, applied here on first call: `region` and `special`
/// default to `standout`, `isearch` to `underline`, `suffix` to `bold`
/// — direct ports of zle_refresh.c:395-402.
pub fn zle_set_highlight(manager: &mut HighlightManager, atrs: &[&str]) {
    use HighlightCategory as HC;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_countprompt() {
        assert_eq!(countprompt("hello"), 5);
        assert_eq!(countprompt("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(countprompt("日本語"), 6); // 3 chars, 2 width each
    }

    #[test]
    fn test_video_buffer() {
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
        let mut state = RefreshState::new();
        assert!(state.old_video.is_some());
        assert!(state.new_video.is_some());

        state.swap_buffers();
        state.free_video();
        assert!(state.old_video.is_none());
    }

    #[test]
    fn compute_render_attrs_empty_buffer_yields_empty_overlay() {
        let zle = Zle::new();
        assert!(zle.compute_render_attrs().is_empty());
    }

    #[test]
    fn compute_render_attrs_visual_mode_paints_mark_to_cursor_in_standout() {
        let mut zle = Zle::new();
        zle.zleline = "hello world".chars().collect();
        zle.zlell = zle.zleline.len();
        zle.mark = 2;
        zle.zlecs = 7;
        zle.region_active = 1; // charwise visual
        let attrs = zle.compute_render_attrs();
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
        let mut zle = Zle::new();
        zle.zleline = "abcdef".chars().collect();
        zle.zlell = 6;
        zle.mark = 5;
        zle.zlecs = 1;
        zle.region_active = 2; // linewise — same swap behavior
        let attrs = zle.compute_render_attrs();
        // Range collapses to (1..5).
        assert!(attrs[0].is_none());
        for slot in attrs.iter().take(5).skip(1) {
            assert!(slot.unwrap().standout);
        }
        assert!(attrs[5].is_none());
    }

    #[test]
    fn match_highlight_handles_combined_attrs() {
        let attr = match_highlight("bold,fg=red,underline");
        assert!(attr.bold);
        assert!(attr.underline);
        assert_eq!(attr.fg_color, Some(1));
    }

    #[test]
    fn match_highlight_named_and_numeric_colors() {
        assert_eq!(match_highlight("fg=cyan").fg_color, Some(6));
        assert_eq!(match_highlight("bg=42").bg_color, Some(42));
        // Out-of-range numeric → ignored (parse fails for u8).
        assert_eq!(match_highlight("fg=999").fg_color, None);
    }

    #[test]
    fn match_highlight_negation_clears_attr() {
        let attr = match_highlight("bold,nobold,underline");
        assert!(!attr.bold);
        assert!(attr.underline);
    }

    #[test]
    fn match_highlight_none_resets_everything() {
        let attr = match_highlight("bold,fg=red,none,underline");
        // After `none` the only thing surviving is the trailing `underline`.
        assert!(!attr.bold);
        assert!(attr.underline);
        assert_eq!(attr.fg_color, None);
    }

    #[test]
    fn zle_set_highlight_populates_categories_and_defaults() {
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
        // When the user sets `zle_highlight=(region:fg=red,bold)` via
        // zle_set_highlight, vi visual-mode should paint the region
        // with that attr instead of the default standout.
        let mut zle = Zle::new();
        zle.zleline = "abcde".chars().collect();
        zle.zlell = 5;
        zle.mark = 1;
        zle.zlecs = 4;
        zle.region_active = 1;
        zle_set_highlight(&mut zle.highlight, &["region:fg=red,bold"]);
        let attrs = zle.compute_render_attrs();
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
        let mut zle = Zle::new();
        zle.zleline = "abcde".chars().collect();
        zle.zlell = 5;
        let custom = TextAttr {
            bold: true,
            fg_color: Some(1),
            ..TextAttr::default()
        };
        zle.highlight
            .set_region_highlight(1, 4, custom);
        let attrs = zle.compute_render_attrs();
        assert!(attrs[0].is_none());
        for slot in attrs.iter().take(4).skip(1) {
            let a = slot.expect("custom");
            assert!(a.bold);
            assert_eq!(a.fg_color, Some(1));
        }
        assert!(attrs[4].is_none());
    }
}

/// Port of `addmultiword()` from Src/Zle/zle_refresh.c:913. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn addmultiword(_base: &mut crate::ported::zle::zle_h::REFRESH_ELEMENT,  // c:addmultiword
                     _tptr: &[char], _ichars: usize) {
    // C body: adds a multi-codepoint cluster to the multiword
    // buffer (mwbuf), setting `base->atr |= TXT_MULTIWORD_MASK`.
    // Substrate (mwbuf grow + mwsize tracking) deferred. zshrs's
    // VideoBuffer holds Vec<char> per cell; multi-word path collapses
    // to single-char storage.
}

/// Port of `bufswap()` from Src/Zle/zle_refresh.c:946. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bufswap(state: &mut RefreshState) {                                   // c:bufswap
    // C body: swap nbuf and obuf pointers (with mwbuf shadow when
    // MULTIBYTE_SUPPORT). Rust just swaps the Option<VideoBuffer>.
    std::mem::swap(&mut state.old_video, &mut state.new_video);
}

/// Port of `freevideo()` from Src/Zle/zle_refresh.c:700. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freevideo(state: &mut RefreshState) {                                 // c:freevideo
    // C body: walk nbuf/obuf rows; zfree each REFRESH_STRING; zfree
    // the row arrays. Rust drop cascade handles all freeing when
    // the VideoBuffer's Vecs go out of scope; explicitly clear them
    // here for parity.
    state.old_video = None;
    state.new_video = None;
}

/// Port of `nextline()` from Src/Zle/zle_refresh.c:842. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn nextline(state: &mut RefreshState, _wrapped: i32) -> i32 {            // c:842
    // C body (c:842-873): advance rpms->ln++; check space against
    // winh; allocate new buffer row if needed; return 1 when display
    // is full (caller should stop emitting). zshrs uses RefreshState
    // for the cursor; this advances vln and signals overflow.
    state.vln += 1;
    if state.vln >= state.lines {
        return 1;                                                            // out of vertical space
    }
    state.vcs = 0;
    0
}

/// Port of `resetvideo()` from Src/Zle/zle_refresh.c:725. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
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

/// Port of `singmoveto()` from Src/Zle/zle_refresh.c:2687. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn singmoveto(state: &mut RefreshState, pos: usize) {                    // c:singmoveto
    // C body: `singlemoveto()` issues termcap cursor-positioning to
    // `pos` on a single-line display. Without termcap output here
    // we just update vcs (cursor column) on RefreshState.
    state.vcs = pos;
}

/// Port of `snextline()` from Src/Zle/zle_refresh.c:875. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn snextline(state: &mut RefreshState) -> i32 {                          // c:875
    // C body (c:875-919): scroll the on-screen display up one line
    // when the new line wraps past the bottom. zshrs decrements
    // vln so the next emit lands on the (now-cleared) bottom row.
    if state.vln > 0 {
        state.vln -= 1;
    }
    state.vcs = 0;
    0
}

/// Port of `tcout_via_func()` from Src/Zle/zle_refresh.c:2291. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn tcout_via_func(_cap: i32, _arg: i32) -> i32 {                         // c:tcout_via_func
    // C body: looks up `tcout` shell function; if defined, calls it
    // with cap+arg; else falls back to direct termcap output. Without
    // shfunc-call substrate, defer to normal termcap path (no-op
    // here — caller chooses fallback).
    1
}

/// Port of `wpfxlen()` from `Src/Zle/zle_refresh.c:1736`.
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

/// Port of `zle_free_highlight()` from `Src/Zle/zle_refresh.c:415`.
/// ```c
/// void
/// zle_free_highlight(void) {
///     free_colour_buffer();
/// }
/// ```
/// Drop the colour buffer used by region highlighting. C calls
/// `free_colour_buffer()`; without that substrate this is a no-op
/// (Rust drop cascade handles the buffer when it goes out of scope).
pub fn zle_free_highlight() {                                                // c:415
    // free_colour_buffer() — substrate not yet ported. The colour-
    // buffer storage is local to zrefresh path and Rust will drop
    // it via Vec::clear when the surrounding state is rebuilt.
}

/// Port of `ZR_memset()` from `Src/Zle/zle_refresh.c:85`.
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
pub fn ZR_memset(                                                            // c:85
    dst: &mut [crate::ported::zle::zle_h::REFRESH_ELEMENT],
    rc: crate::ported::zle::zle_h::REFRESH_ELEMENT,
    len: usize,
) {
    let n = len.min(dst.len());
    for slot in dst.iter_mut().take(n) {                                     // c:88-89 while (len--) *dst++ = rc
        *slot = rc;
    }
}

/// Port of `ZR_strcpy()` from `Src/Zle/zle_refresh.c:94`.
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
#[allow(non_snake_case)]
pub fn ZR_strcpy(                                                            // c:94
    dst: &mut [crate::ported::zle::zle_h::REFRESH_ELEMENT],
    src: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
) {
    let mut i = 0;
    loop {                                                                   // c:97 while ((*dst++ = *src++).chr != ZWC('\0'))
        if i >= dst.len() || i >= src.len() {
            break;
        }
        dst[i] = src[i];
        if src[i].chr == '\0' {
            break;
        }
        i += 1;
    }
}

/// Port of `ZR_strlen()` from `Src/Zle/zle_refresh.c:101`.
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
pub fn ZR_strlen(wstr: &[crate::ported::zle::zle_h::REFRESH_ELEMENT]) -> usize {  // c:101
    let mut len = 0;                                                         // c:104 int len = 0
    while len < wstr.len() && wstr[len].chr != '\0' {                        // c:106 while (wstr++->chr != ZWC('\0'))
        len += 1;                                                            // c:107 len++
    }
    len                                                                      // c:109 return len
}

/// Port of `ZR_strncmp()` from `Src/Zle/zle_refresh.c:119`.
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
pub fn ZR_strncmp(                                                           // c:119
    oldwstr: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
    newwstr: &[crate::ported::zle::zle_h::REFRESH_ELEMENT],
    len: usize,
) -> i32 {
    use crate::ported::zsh_h::TXT_MULTIWORD_MASK;
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
        let mut buf = [REFRESH_ELEMENT::default(); 2];
        let fill = re('y', 0);
        ZR_memset(&mut buf, fill, 99);  // len > dst.len()
        assert_eq!(buf[0], fill);
        assert_eq!(buf[1], fill);
    }

    #[test]
    fn zr_strlen_counts_to_nul() {
        // c:106 — `while (wstr++->chr != ZWC('\0')) len++`.
        let s = [re('h', 0), re('i', 0), re('\0', 0)];
        assert_eq!(ZR_strlen(&s), 2);
    }

    #[test]
    fn zr_strlen_empty_starts_with_nul() {
        let s = [re('\0', 0)];
        assert_eq!(ZR_strlen(&s), 0);
    }

    #[test]
    fn zr_strcpy_copies_through_nul() {
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
        // c:127 — pair-equal in chr+atr: returns 0.
        let a = [re('h', 0), re('i', 0)];
        let b = [re('h', 0), re('i', 0)];
        assert_eq!(ZR_strncmp(&a, &b, 2), 0);
    }

    #[test]
    fn zr_strncmp_diff_chr_returns_1() {
        let a = [re('h', 0), re('i', 0)];
        let b = [re('h', 0), re('o', 0)];
        // c:127 — `if (!ZR_equal(...)) return 1`.
        assert_eq!(ZR_strncmp(&a, &b, 2), 1);
    }

    #[test]
    fn zr_strncmp_diff_atr_returns_1() {
        // c:127 — atr is part of equality.
        let a = [re('h', 0)];
        let b = [re('h', TXTBOLDFACE)];
        assert_eq!(ZR_strncmp(&a, &b, 1), 1);
    }

    #[test]
    fn zr_strncmp_early_nul_old() {
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
        // c:124 — `(!(oldwstr->atr & TXT_MULTIWORD_MASK) && !oldwstr->chr)`.
        // If atr has MULTIWORD set, chr=='\0' is NOT a NUL terminator.
        let a = [re('\0', TXT_MULTIWORD_MASK)];
        let b = [re('\0', TXT_MULTIWORD_MASK)];
        // Both elements equal (same chr+atr) → returns 0; the
        // multiword mask path skips the early-NUL exit so we fall
        // through to the regular ZR_equal check.
        assert_eq!(ZR_strncmp(&a, &b, 1), 0);
    }
}
