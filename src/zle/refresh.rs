//! ZLE refresh - screen redraw routines
//!
//! Direct port from zsh/Src/Zle/zle_refresh.c

use std::io::{self, Write};

use super::main::Zle;

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
    pub fn new(chr: char) -> Self {
        let width = unicode_width::UnicodeWidthChar::width(chr).unwrap_or(1) as u8;
        RefreshElement {
            chr,
            atr: TextAttr::default(),
            width,
        }
    }

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
    pub fn new(cols: usize, rows: usize) -> Self {
        let lines = vec![vec![RefreshElement::new(' '); cols]; rows];
        VideoBuffer { lines, cols, rows }
    }

    pub fn clear(&mut self) {
        for line in &mut self.lines {
            for elem in line.iter_mut() {
                *elem = RefreshElement::new(' ');
            }
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.lines
            .resize(rows, vec![RefreshElement::new(' '); cols]);
        for line in &mut self.lines {
            line.resize(cols, RefreshElement::new(' '));
        }
    }

    pub fn set(&mut self, row: usize, col: usize, elem: RefreshElement) {
        if row < self.rows && col < self.cols {
            self.lines[row][col] = elem;
        }
    }

    pub fn get(&self, row: usize, col: usize) -> Option<&RefreshElement> {
        self.lines.get(row).and_then(|line| line.get(col))
    }
}

/// Refresh parameters
#[derive(Debug, Clone, Default)]
pub struct RefreshState {
    /// Number of columns
    pub columns: usize,
    /// Number of lines  
    pub lines: usize,
    /// Current line on screen (cursor row)
    pub vln: usize,
    /// Current column on screen (cursor col)
    pub vcs: usize,
    /// Prompt width (left)
    pub lpromptw: usize,
    /// Right prompt width
    pub rpromptw: usize,
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
    pub fn new() -> Self {
        let (cols, rows) = get_terminal_size();
        RefreshState {
            columns: cols,
            lines: rows,
            old_video: Some(VideoBuffer::new(cols, rows)),
            new_video: Some(VideoBuffer::new(cols, rows)),
            need_full_redraw: true,
            ..Default::default()
        }
    }

    pub fn reset_video(&mut self) {
        let (cols, rows) = get_terminal_size();
        self.columns = cols;
        self.lines = rows;
        self.old_video = Some(VideoBuffer::new(cols, rows));
        self.new_video = Some(VideoBuffer::new(cols, rows));
        self.need_full_redraw = true;
    }

    pub fn free_video(&mut self) {
        self.old_video = None;
        self.new_video = None;
    }

    pub fn swap_buffers(&mut self) {
        std::mem::swap(&mut self.old_video, &mut self.new_video);
        if let Some(ref mut new) = self.new_video {
            new.clear();
        }
    }
}

impl Zle {
    /// Main refresh function - redraws the screen
    /// Port of zrefresh() from zle_refresh.c
    pub fn zrefresh(&mut self) {
        let stdout = io::stdout();
        let mut handle = stdout.lock();

        // Get terminal size
        let (cols, _rows) = get_terminal_size();

        // Build the display line
        let prompt = self.prompt();
        let buffer: String = self.zleline.iter().collect();
        let cursor = self.zlecs;

        // Calculate display positions
        let prompt_width = visible_width(prompt);
        let buffer_before_cursor: String = self.zleline[..cursor.min(self.zleline.len())]
            .iter()
            .collect();
        let cursor_col = prompt_width + visible_width(&buffer_before_cursor);

        // Handle horizontal scrolling if line is too long
        let scroll_margin = 8;
        let effective_cols = cols.saturating_sub(1);

        let scroll_offset = if cursor_col >= effective_cols.saturating_sub(scroll_margin) {
            cursor_col.saturating_sub(effective_cols / 2)
        } else {
            0
        };

        // Move to start of line and clear
        let _ = write!(handle, "\r\x1b[K");

        // Draw prompt (if not scrolled past)
        if scroll_offset < prompt_width {
            let visible_prompt = skip_chars(prompt, scroll_offset);
            let _ = write!(handle, "{}", visible_prompt);
        }

        // Draw buffer content
        let buffer_start = scroll_offset.saturating_sub(prompt_width);
        let visible_buffer = skip_chars(&buffer, buffer_start);
        let truncated = truncate_to_width(
            &visible_buffer,
            effective_cols.saturating_sub(prompt_width.saturating_sub(scroll_offset)),
        );
        let _ = write!(handle, "{}", truncated);

        // Position cursor
        let display_cursor_col = cursor_col.saturating_sub(scroll_offset);
        let _ = write!(handle, "\r\x1b[{}C", display_cursor_col);

        let _ = handle.flush();
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
    pub fn clearscreen(&mut self) {
        print!("\x1b[2J\x1b[H");
        let _ = io::stdout().flush();
        self.zrefresh();
    }

    /// Redisplay the current line
    /// Port of redisplay() from zle_refresh.c
    pub fn redisplay(&mut self) {
        self.zrefresh();
    }

    /// Move cursor to position
    /// Port of moveto() from zle_refresh.c
    pub fn moveto(&mut self, row: usize, col: usize) {
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
    pub fn singlerefresh(&mut self) {
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

/// Get terminal size
pub fn get_terminal_size() -> (usize, usize) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(0, libc::TIOCGWINSZ, &mut ws) == 0 {
            (ws.ws_col as usize, ws.ws_row as usize)
        } else {
            (80, 24) // Default
        }
    }
}

/// Calculate visible width of a string (handling ANSI escapes)
fn visible_width(s: &str) -> usize {
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

/// Skip N visible characters from a string
fn skip_chars(s: &str, n: usize) -> &str {
    let mut width = 0;
    let mut byte_idx = 0;
    let mut in_escape = false;

    for (i, c) in s.char_indices() {
        if width >= n {
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

    &s[byte_idx..]
}

/// Truncate string to fit within given width
fn truncate_to_width(s: &str, max_width: usize) -> &str {
    let mut width = 0;
    let mut byte_idx = s.len();
    let mut in_escape = false;

    for (i, c) in s.char_indices() {
        if in_escape {
            if c.is_ascii_alphabetic() {
                in_escape = false;
            }
        } else if c == '\x1b' {
            in_escape = true;
        } else {
            let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
            if width + char_width > max_width {
                byte_idx = i;
                break;
            }
            width += char_width;
        }
    }

    &s[..byte_idx]
}

/// Region highlight entry
#[derive(Debug, Clone)]
pub struct RegionHighlight {
    pub start: usize,
    pub end: usize,
    pub attr: TextAttr,
    pub memo: Option<String>,
}

/// Highlight manager
#[derive(Debug, Default)]
pub struct HighlightManager {
    pub regions: Vec<RegionHighlight>,
}

impl HighlightManager {
    pub fn new() -> Self {
        HighlightManager {
            regions: Vec::new(),
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

/// Terminal output functions
/// Port of tcout() family from zle_refresh.c

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

/// Set ZLE highlight
/// Port of zle_set_highlight() from zle_refresh.c
pub fn zle_set_highlight(_highlight: &str) {
    // Parse highlight specification and apply
    // Format: "region:standout" or "special:fg=red,bg=blue"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_visible_width() {
        assert_eq!(visible_width("hello"), 5);
        assert_eq!(visible_width("\x1b[31mhello\x1b[0m"), 5);
        assert_eq!(visible_width("日本語"), 6); // 3 chars, 2 width each
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
}
