//! Menu completion and interactive selection
//!
//! Implements zsh-style menu completion with:
//! - Auto-scrolling viewport (no pagination prompts)
//! - Live command line updates as you navigate
//! - Column memory for vertical navigation
//! - Group support with headers
//! - Incremental search filtering
//! - Multi-select with accept-and-continue
//!
//! Based on zsh's Src/Zle/compresult.c and complist.c

use crate::completion::{Completion, CompletionGroup};
use crate::zpwr_colors::HeaderColors;
use crate::zstyle::ZStyleStore;
use unicode_width::UnicodeWidthStr;

/// ANSI color codes
pub mod ansi {
    pub const RESET: &str = "\x1b[0m";
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const UNDERLINE: &str = "\x1b[4m";
    pub const REVERSE: &str = "\x1b[7m";

    pub const BLACK: &str = "\x1b[30m";
    pub const RED: &str = "\x1b[31m";
    pub const GREEN: &str = "\x1b[32m";
    pub const YELLOW: &str = "\x1b[33m";
    pub const BLUE: &str = "\x1b[34m";
    pub const MAGENTA: &str = "\x1b[35m";
    pub const CYAN: &str = "\x1b[36m";
    pub const WHITE: &str = "\x1b[37m";

    pub const BG_BLACK: &str = "\x1b[40m";
    pub const BG_RED: &str = "\x1b[41m";
    pub const BG_GREEN: &str = "\x1b[42m";
    pub const BG_YELLOW: &str = "\x1b[43m";
    pub const BG_BLUE: &str = "\x1b[44m";
    pub const BG_MAGENTA: &str = "\x1b[45m";
    pub const BG_CYAN: &str = "\x1b[46m";
    pub const BG_WHITE: &str = "\x1b[47m";

    /// Build ANSI escape from semicolon-separated codes like "38;5;82"
    pub fn from_codes(codes: &str) -> String {
        if codes.is_empty() {
            String::new()
        } else {
            format!("\x1b[{}m", codes)
        }
    }
}

/// Terminal colors for completion display
#[derive(Clone, Debug)]
pub struct MenuColors {
    /// Normal text (no)
    pub normal: String,
    /// Selected item (ma)
    pub selected: String,
    /// Secondary row background (sp)
    pub secondary: String,
    /// Completion text (tc)
    pub completion: String,
    /// Description text (dc)  
    pub description: String,
    /// Prefix text
    pub prefix: String,
    /// Group header (so)
    pub header: String,
    /// Scroll indicator
    pub scroll: String,
    /// File (fi)
    pub file: String,
    /// Directory (di)
    pub directory: String,
    /// Executable (ex)
    pub executable: String,
    /// Symbolic link (ln)
    pub symlink: String,
}

impl Default for MenuColors {
    fn default() -> Self {
        Self {
            normal: String::new(),
            selected: "7".to_string(),  // reverse video
            secondary: "2".to_string(), // dim
            completion: String::new(),
            description: "36".to_string(), // cyan
            prefix: "32".to_string(),      // green
            header: "1;33".to_string(),    // bold yellow
            scroll: "2".to_string(),       // dim
            file: String::new(),
            directory: "1;34".to_string(),  // bold blue
            executable: "1;32".to_string(), // bold green
            symlink: "1;36".to_string(),    // bold cyan
        }
    }
}

/// Patterns for ZLS_COLORS (pattern=color mappings)
#[derive(Clone, Debug, Default)]
pub struct ZlsColorPatterns {
    /// Extension patterns: "*.ext" -> color
    pub extensions: Vec<(String, String)>,
    /// Glob patterns: "pattern" -> color (for completions)
    pub patterns: Vec<(String, String)>,
    /// File type patterns: "(#s)type(#e)" -> color
    pub file_types: Vec<(String, String)>,
}

impl MenuColors {
    /// Parse from ZLS_COLORS/ZLS_COLOURS environment or zstyle
    /// Format: key=color:key2=color2:*.ext=color3:=(#i)pat*=color4
    pub fn from_zstyle(styles: &ZStyleStore, context: &str) -> Self {
        let mut colors = Self::default();

        if let Some(vals) = styles.lookup_values(context, "list-colors") {
            for val in vals {
                if let Some((key, color)) = val.split_once('=') {
                    match key {
                        "no" => colors.normal = color.to_string(),
                        "ma" => colors.selected = color.to_string(),
                        "sp" => colors.secondary = color.to_string(),
                        "tc" => colors.completion = color.to_string(),
                        "dc" => colors.description = color.to_string(),
                        "so" => colors.header = color.to_string(),
                        "fi" => colors.file = color.to_string(),
                        "di" => colors.directory = color.to_string(),
                        "ex" => colors.executable = color.to_string(),
                        "ln" => colors.symlink = color.to_string(),
                        _ => {}
                    }
                }
            }
        }
        colors
    }

    /// Parse full ZLS_COLORS string (colon-separated)
    /// Supports: basic keys, extension patterns (*.ext), glob patterns, file type codes
    pub fn parse_zls_colors(zls_string: &str) -> (Self, ZlsColorPatterns) {
        let mut colors = Self::default();
        let mut patterns = ZlsColorPatterns::default();

        for spec in zls_string.split(':') {
            if spec.is_empty() {
                continue;
            }

            if let Some((key, color)) = spec.split_once('=') {
                // Check if it's an extension pattern
                if key.starts_with('*') {
                    patterns
                        .extensions
                        .push((key[1..].to_string(), color.to_string()));
                    continue;
                }

                // Check if it's a glob pattern (starts with = for completion match)
                if key.starts_with('=') {
                    patterns
                        .patterns
                        .push((key[1..].to_string(), color.to_string()));
                    continue;
                }

                // Check for special file type codes (2-char LS_COLORS codes)
                match key {
                    // Basic colors
                    "no" => colors.normal = color.to_string(),
                    "ma" => colors.selected = color.to_string(),
                    "sp" => colors.secondary = color.to_string(),
                    "tc" => colors.completion = color.to_string(),
                    "dc" => colors.description = color.to_string(),
                    "so" => colors.header = color.to_string(),

                    // File types (LS_COLORS compatible)
                    "fi" => colors.file = color.to_string(),
                    "di" => colors.directory = color.to_string(),
                    "ex" => colors.executable = color.to_string(),
                    "ln" => colors.symlink = color.to_string(),
                    "bd" => patterns
                        .file_types
                        .push(("block".to_string(), color.to_string())),
                    "cd" => patterns
                        .file_types
                        .push(("char".to_string(), color.to_string())),
                    "pi" | "p" => patterns
                        .file_types
                        .push(("pipe".to_string(), color.to_string())),
                    "su" => patterns
                        .file_types
                        .push(("setuid".to_string(), color.to_string())),
                    "sg" => patterns
                        .file_types
                        .push(("setgid".to_string(), color.to_string())),
                    "tw" => patterns
                        .file_types
                        .push(("sticky_world".to_string(), color.to_string())),
                    "ow" => patterns
                        .file_types
                        .push(("world_write".to_string(), color.to_string())),
                    "st" => patterns
                        .file_types
                        .push(("sticky".to_string(), color.to_string())),
                    "or" => patterns
                        .file_types
                        .push(("orphan".to_string(), color.to_string())),
                    "mi" => patterns
                        .file_types
                        .push(("missing".to_string(), color.to_string())),

                    _ => {
                        // Unknown key - treat as pattern if it contains glob chars
                        if key.contains('*') || key.contains('?') || key.contains('[') {
                            patterns.patterns.push((key.to_string(), color.to_string()));
                        }
                    }
                }
            }
        }

        (colors, patterns)
    }

    /// Match filename against patterns and return color if found
    pub fn match_filename(filename: &str, patterns: &ZlsColorPatterns) -> Option<String> {
        // Check extension patterns first (most common)
        for (ext, color) in &patterns.extensions {
            if filename.ends_with(ext) {
                return Some(color.clone());
            }
        }

        // Check glob patterns
        for (pattern, color) in &patterns.patterns {
            if Self::glob_match(pattern, filename) {
                return Some(color.clone());
            }
        }

        None
    }

    /// Simple glob matching (*, ?, [])
    fn glob_match(pattern: &str, text: &str) -> bool {
        // Handle case-insensitive flag (#i) at start
        let (pattern, case_insensitive) = if pattern.starts_with("(#i)") {
            (&pattern[4..], true)
        } else {
            (pattern, false)
        };

        let pattern: Vec<char> = if case_insensitive {
            pattern.to_lowercase().chars().collect()
        } else {
            pattern.chars().collect()
        };
        let text: Vec<char> = if case_insensitive {
            text.to_lowercase().chars().collect()
        } else {
            text.chars().collect()
        };

        Self::glob_match_impl(&pattern, &text)
    }

    fn glob_match_impl(pattern: &[char], text: &[char]) -> bool {
        let mut p = 0;
        let mut t = 0;
        let mut star_p = None;
        let mut star_t = 0;

        while t < text.len() {
            if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
                p += 1;
                t += 1;
            } else if p < pattern.len() && pattern[p] == '*' {
                star_p = Some(p);
                star_t = t;
                p += 1;
            } else if let Some(sp) = star_p {
                p = sp + 1;
                star_t += 1;
                t = star_t;
            } else {
                return false;
            }
        }

        while p < pattern.len() && pattern[p] == '*' {
            p += 1;
        }

        p == pattern.len()
    }

    /// Get ANSI escape for a color code string
    pub fn escape(&self, color: &str) -> String {
        ansi::from_codes(color)
    }
}

/// Direction for menu navigation (matches zsh complist.c keybindings)
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuMotion {
    // Basic movement
    Up,
    Down,
    Left,
    Right,

    // Page movement
    PageUp,
    PageDown,

    // Sequential navigation
    Next,
    Prev,

    // Absolute positions
    First,           // beginning-of-history
    Last,            // end-of-history
    BeginningOfLine, // vi-beginning-of-line
    EndOfLine,       // vi-end-of-line

    // Word/group movement (zsh viforwardblankword etc.)
    ForwardWord,       // forward-word
    BackwardWord,      // backward-word
    ForwardBlankWord,  // vi-forward-blank-word (next group)
    BackwardBlankWord, // vi-backward-blank-word (prev group)

    // Selection control
    Deselect,
    Accept,                // accept-line
    AcceptAndHold,         // accept-and-hold
    AcceptAndMenuComplete, // accept-and-menu-complete

    // Undo
    Undo,
}

/// Layout information for a completion group
#[derive(Clone, Debug, Default)]
pub struct GroupLayout {
    /// Group name/tag
    pub name: String,
    /// Explanation/header text
    pub explanation: Option<String>,
    /// Number of matches in this group
    pub count: usize,
    /// Starting index in the flat match list
    pub start_idx: usize,
    /// Starting row in the layout
    pub start_row: usize,
    /// Number of rows for this group (including header)
    pub row_count: usize,
    /// Column widths for this group
    pub col_widths: Vec<usize>,
    /// Number of columns
    pub cols: usize,
    /// Pack columns tightly (LIST_PACKED)
    pub packed: bool,
    /// Fill rows first (LIST_ROWS_FIRST)
    pub rows_first: bool,
    /// ANSI color code for this group (cyberpunk!)
    pub color: String,
    /// Whether items in this group have descriptions
    pub has_descriptions: bool,
    /// Width of completion column (for aligning descriptions)
    pub comp_col_width: usize,
}

/// ZPWR-style color palette for completion groups (from ~/.zpwr zstyle configs)
/// Format: "FG;BG" where BG uses 4x codes (41=red, 42=green, 43=yellow, 44=blue, 45=magenta, 46=cyan)
pub const GROUP_COLORS: &[&str] = &[
    "34;42;4",   // aliases: blue on green, underline
    "1;37;41",   // functions: bold white on red
    "1;37;4;43", // builtins: bold white on yellow, underline
    "1;37;44",   // executables: bold white on blue
    "1;32;45",   // parameters: bold green on magenta
    "1;34;41;4", // files: bold blue on red, underline
    "1;37;42",   // users: bold white on green
    "1;37;43",   // hosts: bold white on yellow
    "1;34;43;4", // global-aliases: bold blue on yellow, underline
    "1;4;37;45", // reserved-words: bold underline white on magenta
    "1;37;46",   // heads-remote: bold white on cyan
    "1;37;44",   // recent-branches: bold white on blue
];

/// A single item in the menu display
#[derive(Clone, Debug)]
pub struct MenuItem {
    /// The completion
    pub completion: Completion,
    /// Group index
    pub group_idx: usize,
    /// Index within group
    pub idx_in_group: usize,
    /// Display string (with any prefix/suffix)
    pub display: String,
    /// Description
    pub description: String,
    /// Display width of completion
    pub comp_width: usize,
    /// Display width of description
    pub desc_width: usize,
}

/// Rendered line for display
#[derive(Clone, Debug, Default)]
pub struct MenuLine {
    /// Characters with ANSI escapes
    pub content: String,
    /// Logical width (excluding escapes)
    pub width: usize,
    /// Is this a header line?
    pub is_header: bool,
}

impl MenuLine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn header(text: &str, _color: &str) -> Self {
        // ZPWR-style header: RED -<< BLUE text RED >>-
        let hc = HeaderColors::from_env();
        let formatted = format!("{}{}{}", hc.pre, text, hc.post);
        let width = UnicodeWidthStr::width(formatted.as_str());
        let content = hc.format(text);

        Self {
            content,
            width,
            is_header: true,
        }
    }

    pub fn append(&mut self, s: &str) {
        let w = UnicodeWidthStr::width(s);
        self.content.push_str(s);
        self.width += w;
    }

    pub fn append_with_width(&mut self, s: &str, width: usize) {
        self.content.push_str(s);
        self.width += width;
    }

    pub fn append_colored(&mut self, s: &str, color: &str) {
        let w = UnicodeWidthStr::width(s);
        if !color.is_empty() {
            self.content.push_str(&ansi::from_codes(color));
        }
        self.content.push_str(s);
        if !color.is_empty() {
            self.content.push_str(ansi::RESET);
        }
        self.width += w;
    }

    /// Pad to specified width with spaces
    pub fn pad_to(&mut self, target_width: usize) {
        if self.width < target_width {
            let pad = target_width - self.width;
            self.content.push_str(&" ".repeat(pad));
            self.width = target_width;
        }
    }

    /// Pad to width with colored background
    pub fn pad_to_colored(&mut self, target_width: usize, bg_color: &str) {
        if self.width < target_width {
            let pad = target_width - self.width;
            if !bg_color.is_empty() {
                self.content.push_str(&ansi::from_codes(bg_color));
            }
            self.content.push_str(&" ".repeat(pad));
            if !bg_color.is_empty() {
                self.content.push_str(ansi::RESET);
            }
            self.width = target_width;
        }
    }
}

/// The rendered menu display
#[derive(Clone, Debug, Default)]
pub struct MenuRendering {
    /// Terminal width used for this rendering
    pub term_width: usize,
    /// Terminal height used for this rendering  
    pub term_height: usize,
    /// Total number of matches
    pub total_matches: usize,
    /// Number of columns in layout
    pub cols: usize,
    /// Number of rows in layout (may exceed viewport)
    pub total_rows: usize,
    /// First visible row
    pub row_start: usize,
    /// Last visible row (exclusive)
    pub row_end: usize,
    /// Selected match index (None = no selection)
    pub selected_idx: Option<usize>,
    /// Selected row in the full layout
    pub selected_row: usize,
    /// Selected column
    pub selected_col: usize,
    /// Lines to display
    pub lines: Vec<MenuLine>,
    /// Status line text (e.g., "rows 5-12 of 47")
    pub status: Option<String>,
    /// Whether list is scrollable
    pub scrollable: bool,
}

/// Result of singledraw optimization (zsh singledraw())
/// Contains only the cells that need redrawing
#[derive(Clone, Debug)]
pub struct SingleDrawResult {
    /// Old selection cell: (screen_row, col, rendered_content)
    pub old_cell: Option<(usize, usize, MenuLine)>,
    /// New selection cell: (screen_row, col, rendered_content)
    pub new_cell: Option<(usize, usize, MenuLine)>,
}

/// Menu mode constants (from zsh complist.c)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuMode {
    /// Normal menu selection
    Normal,
    /// Interactive completion (MM_INTER) - typing filters live
    Interactive,
    /// Forward incremental search (MM_FSEARCH)
    ForwardSearch,
    /// Backward incremental search (MM_BSEARCH)  
    BackwardSearch,
}

/// Group flags (from zsh CGF_* constants)
#[derive(Clone, Copy, Debug, Default)]
pub struct GroupFlags {
    /// CGF_ROWS: Fill rows first (row-major), else column-major
    pub rows_first: bool,
    /// CGF_PACKED: Pack columns tightly with variable widths
    pub packed: bool,
    /// CGF_HASDL: Group has display lines (items with CMF_DISPLINE)
    pub has_displines: bool,
    /// CGF_FILES: This is a file completion group
    pub files: bool,
}

/// Menu stack entry for undo (from zsh struct menustack)
#[derive(Clone, Debug)]
pub struct MenuStackEntry {
    /// Saved command line
    pub line: String,
    /// Cursor position
    pub cs: usize,
    /// Menu line position
    pub mline: usize,
    /// Viewport start
    pub mlbeg: usize,
    /// Selected match index
    pub mselect: Option<usize>,
    /// Column position
    pub mcol: usize,
    /// Search string
    pub search: String,
    /// Menu mode
    pub mode: MenuMode,
}

/// Search stack entry for incremental search (from zsh struct menusearch)
#[derive(Clone, Debug)]
pub struct SearchStackEntry {
    /// Search string at this point
    pub str_: String,
    /// Line position
    pub line: usize,
    /// Column position  
    pub col: usize,
    /// Search direction (true = backward)
    pub back: bool,
    /// Search state (ok, failed, wrapped)
    pub state: SearchState,
}

/// Search state flags
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct SearchState {
    pub failed: bool,
    pub wrapped: bool,
}

/// Menu completion state machine
///
/// Implements zsh menuselect behavior from Src/Zle/complist.c
#[derive(Clone, Debug)]
pub struct MenuState {
    // === Core match data ===
    /// All completion items, flattened
    items: Vec<MenuItem>,
    /// Group layouts
    groups: Vec<GroupLayout>,
    /// Unfiltered items (for search restore)
    unfiltered_items: Vec<MenuItem>,

    // === Position grid (zsh mtab/mgtab) ===
    /// Match table: maps screen position to item index (zsh: mtab)
    /// Size is mcols * mlines, indexed as [line * mcols + col]
    mtab: Vec<Option<usize>>,
    /// Group table: maps screen position to group index (zsh: mgtab)
    mgtab: Vec<usize>,

    // === Selection state (zsh mselect, mcol, mline) ===
    /// Current selection index (None = not in menu mode)
    selected_idx: Option<usize>,
    /// Multi-select: set of marked/selected indices (zsh CMF_MULT)
    marked_indices: std::collections::HashSet<usize>,
    /// Current column in the grid (zsh: mcol)
    mcol: usize,
    /// Current line in the grid (zsh: mline)  
    mline: usize,
    /// Column memory for vertical navigation (zsh: wishcol)
    wish_col: usize,

    // === Viewport (zsh mlbeg, mlend) ===
    /// First visible line (zsh: mlbeg, -1 forces redraw)
    viewport_start: usize,
    /// Last visible line (zsh: mlend)
    viewport_end: usize,
    /// Previous viewport start for dirty detection (zsh: molbeg)
    prev_viewport_start: Option<usize>,
    /// Previous mcol (zsh: mocol)
    prev_mcol: usize,
    /// Previous mline (zsh: moline)
    prev_mline: usize,

    // === Layout dimensions ===
    /// Terminal width (zsh: zterm_columns)
    term_width: usize,
    /// Terminal height (zsh: zterm_lines)
    term_height: usize,
    /// Space available for completions (term_height - prompt lines - status)
    available_rows: usize,
    /// Number of columns in layout (zsh: mcols)
    mcols: usize,
    /// Number of lines in layout (zsh: mlines)
    mlines: usize,

    // === Menu mode ===
    /// Current menu mode
    mode: MenuMode,
    /// Menu stack for undo (zsh: struct menustack)
    undo_stack: Vec<MenuStackEntry>,
    /// Search stack for incremental search
    search_stack: Vec<SearchStackEntry>,

    // === Search/filter state ===
    /// Prefix being completed
    prefix: String,
    /// Search/filter string (used in search and interactive modes)
    search: String,
    /// Search state (ok/failed/wrapped)
    search_state: SearchState,
    /// Last successful search string (zsh: lastsearch)
    last_search: String,

    // === Display options ===
    /// Show group headers
    show_headers: bool,
    /// Has status line (zsh: mhasstat)
    has_status: bool,
    /// Status line printed (zsh: mstatprinted)
    status_printed: bool,
    /// Scroll step size (zsh: step from MENUSCROLL)
    scroll_step: usize,

    // === Colors ===
    colors: MenuColors,
    /// Custom colors for groups by name (from zstyle list-colors)
    group_colors: std::collections::HashMap<String, String>,
    /// Menu selection color (ma= from zstyle)
    selection_color: String,
    /// Prefix match color (from zstyle list-colors pattern)
    #[allow(dead_code)]
    prefix_color: String,
    /// List separator (ZPWR_CHAR_LOGO)
    list_separator: String,
    /// Follow symlinks for coloring (LC_FOLLOW_SYMLINKS behavior)
    follow_symlinks: bool,

    // === Legacy fields for compatibility ===
    #[allow(dead_code)]
    search_active: bool,
    #[allow(dead_code)]
    search_direction: SearchDirection,
    #[allow(dead_code)]
    interactive_mode: bool,
    layout_valid: bool,
    cached_total_rows: usize,
    cached_cols: usize,
    cached_col_width: usize,
}

impl Default for MenuState {
    fn default() -> Self {
        Self::new()
    }
}

impl MenuState {
    pub fn new() -> Self {
        // Load config from zpwr zstyle files
        let config = crate::zpwr_colors::load_zpwr_config();

        Self {
            // Core match data
            items: Vec::new(),
            groups: Vec::new(),
            unfiltered_items: Vec::new(),

            // Position grid (zsh mtab/mgtab)
            mtab: Vec::new(),
            mgtab: Vec::new(),

            // Selection state
            selected_idx: None,
            marked_indices: std::collections::HashSet::new(),
            mcol: 0,
            mline: 0,
            wish_col: 0,

            // Viewport
            viewport_start: 0,
            viewport_end: 9999999,
            prev_viewport_start: None,
            prev_mcol: 0,
            prev_mline: 0,

            // Layout dimensions
            term_width: 80,
            term_height: 24,
            available_rows: 20,
            mcols: 1,
            mlines: 0,

            // Menu mode
            mode: MenuMode::Normal,
            undo_stack: Vec::new(),
            search_stack: Vec::new(),

            // Search/filter
            prefix: String::new(),
            search: String::new(),
            search_state: SearchState::default(),
            last_search: String::new(),

            // Display options
            show_headers: true,
            has_status: true,
            status_printed: false,
            scroll_step: 0, // 0 = use default (half screen)

            // Colors
            colors: MenuColors::default(),
            group_colors: config.tag_colors,
            selection_color: if config.menu_selection.is_empty() {
                "37;1;4;44".to_string() // fallback
            } else {
                config.menu_selection
            },
            prefix_color: if config.prefix_color.is_empty() {
                "1;30".to_string() // fallback
            } else {
                config.prefix_color
            },
            list_separator: if config.list_separator.is_empty() {
                " -- ".to_string() // fallback: simple separator
            } else {
                config.list_separator
            },
            follow_symlinks: config.follow_symlinks,

            // Legacy compatibility
            search_active: false,
            search_direction: SearchDirection::Forward,
            interactive_mode: false,
            layout_valid: false,
            cached_total_rows: 0,
            cached_cols: 0,
            cached_col_width: 0,
        }
    }

    /// Set terminal size
    pub fn set_term_size(&mut self, width: usize, height: usize) {
        if self.term_width != width || self.term_height != height {
            self.term_width = width;
            self.term_height = height;
            self.layout_valid = false;
        }
    }

    /// Set available rows for completion display
    pub fn set_available_rows(&mut self, rows: usize) {
        if self.available_rows != rows {
            self.available_rows = rows;
            self.layout_valid = false;
        }
    }

    /// Set colors from zstyle
    pub fn set_colors(&mut self, colors: MenuColors) {
        self.colors = colors;
    }

    /// Set custom colors for groups by name (from zstyle list-colors)
    pub fn set_group_colors(&mut self, colors: std::collections::HashMap<String, String>) {
        self.group_colors = colors;
    }

    /// Set list separator (from zstyle list-separator)
    pub fn set_list_separator(&mut self, sep: &str) {
        self.list_separator = sep.to_string();
    }

    /// Set the prefix being completed
    pub fn set_prefix(&mut self, prefix: &str) {
        self.prefix = prefix.to_string();
    }

    /// Enable/disable group headers
    pub fn set_show_headers(&mut self, show: bool) {
        if self.show_headers != show {
            self.show_headers = show;
            self.layout_valid = false;
        }
    }

    /// Load completions from groups
    pub fn set_completions(&mut self, groups: &[CompletionGroup]) {
        self.items.clear();
        self.groups.clear();
        self.unfiltered_items.clear();
        self.layout_valid = false;

        let mut idx = 0;
        for (group_idx, group) in groups.iter().enumerate() {
            let start_idx = idx;

            for (i, comp) in group.matches.iter().enumerate() {
                let display = comp.disp.as_ref().unwrap_or(&comp.str_).clone();
                let description = comp
                    .desc
                    .clone()
                    .or_else(|| comp.exp.clone())
                    .unwrap_or_default();

                let comp_width = UnicodeWidthStr::width(display.as_str());
                let desc_width = UnicodeWidthStr::width(description.as_str());

                self.items.push(MenuItem {
                    completion: comp.clone(),
                    group_idx,
                    idx_in_group: i,
                    display,
                    description,
                    comp_width,
                    desc_width,
                });
                idx += 1;
            }

            // Look up color by group name, falling back to default palette
            let color = self
                .group_colors
                .get(&group.name)
                .or_else(|| self.group_colors.get(&group.name.to_lowercase()))
                .cloned()
                .unwrap_or_else(|| GROUP_COLORS[group_idx % GROUP_COLORS.len()].to_string());

            // Check if any items in this group have descriptions
            let has_descs = group
                .matches
                .iter()
                .any(|m| m.desc.is_some() || m.exp.is_some());

            self.groups.push(GroupLayout {
                name: group.name.clone(),
                explanation: group
                    .explanation
                    .clone()
                    .or_else(|| group.explanations.first().cloned()),
                count: group.matches.len(),
                start_idx,
                start_row: 0,
                row_count: 0,
                col_widths: Vec::new(),
                cols: 0,
                packed: false,
                rows_first: false,
                color,
                has_descriptions: has_descs,
                comp_col_width: 0,
            });
        }

        self.unfiltered_items = self.items.clone();
    }

    /// Check if menu is active
    pub fn is_active(&self) -> bool {
        self.selected_idx.is_some()
    }

    /// Get the currently selected completion
    pub fn selected(&self) -> Option<&Completion> {
        self.selected_idx
            .and_then(|idx| self.items.get(idx).map(|m| &m.completion))
    }

    /// Get selected index
    pub fn selected_index(&self) -> Option<usize> {
        self.selected_idx
    }

    /// Get the insert string for the currently selected completion
    pub fn selected_insert_string(&self) -> Option<String> {
        self.selected_idx
            .and_then(|idx| self.items.get(idx))
            .map(|m| m.completion.insert_str())
    }

    /// Total number of matches
    pub fn count(&self) -> usize {
        self.items.len()
    }

    /// Number of columns in layout (zsh: mcols)
    pub fn cols(&self) -> usize {
        self.mcols
    }

    /// Start menu completion (select first item)
    pub fn start(&mut self) {
        if !self.items.is_empty() {
            self.selected_idx = Some(0);
            self.wish_col = 0;
            self.viewport_start = 0;
            self.ensure_layout();
        }
    }

    /// Exit menu completion
    pub fn stop(&mut self) {
        self.selected_idx = None;
        self.mode = MenuMode::Normal;
        self.undo_stack.clear();
        self.search_stack.clear();
    }

    // === Mode switching (zsh MM_INTER, MM_FSEARCH, MM_BSEARCH) ===

    /// Get current menu mode
    pub fn get_mode(&self) -> MenuMode {
        self.mode
    }

    /// Toggle interactive mode (zsh vi-insert in menu)
    pub fn toggle_interactive(&mut self) {
        if self.mode == MenuMode::Interactive {
            self.mode = MenuMode::Normal;
            // Restore unfiltered items
            if !self.unfiltered_items.is_empty() {
                self.items = self.unfiltered_items.clone();
                self.layout_valid = false;
            }
        } else {
            self.mode = MenuMode::Interactive;
            self.search.clear();
            // Save current items for restore
            if self.unfiltered_items.is_empty() {
                self.unfiltered_items = self.items.clone();
            }
        }
    }

    /// Start forward incremental search (Ctrl+S in zsh)
    pub fn start_forward_search(&mut self) {
        self.mode = MenuMode::ForwardSearch;
        self.search.clear();
        self.search_state = SearchState::default();
        self.search_stack.clear();
    }

    /// Start backward incremental search (Ctrl+R in zsh)  
    pub fn start_backward_search(&mut self) {
        self.mode = MenuMode::BackwardSearch;
        self.search.clear();
        self.search_state = SearchState::default();
        self.search_stack.clear();
    }

    /// Cancel search mode
    pub fn cancel_search(&mut self) {
        self.mode = MenuMode::Normal;
        self.search.clear();
        self.search_state = SearchState::default();
    }

    /// Add character to search string (for interactive/search modes)
    pub fn search_input(&mut self, c: char) {
        // Save current state for backspace
        self.search_stack.push(SearchStackEntry {
            str_: self.search.clone(),
            line: self.mline,
            col: self.mcol,
            back: self.mode == MenuMode::BackwardSearch,
            state: self.search_state,
        });

        self.search.push(c);

        match self.mode {
            MenuMode::Interactive => {
                self.filter_by_search();
            }
            MenuMode::ForwardSearch | MenuMode::BackwardSearch => {
                self.do_incremental_search();
            }
            _ => {}
        }
    }

    /// Backspace in search mode
    pub fn search_backspace(&mut self) {
        if let Some(prev) = self.search_stack.pop() {
            self.search = prev.str_;
            self.mline = prev.line;
            self.mcol = prev.col;
            self.search_state = prev.state;

            if self.mode == MenuMode::Interactive {
                if self.search.is_empty() {
                    self.items = self.unfiltered_items.clone();
                } else {
                    self.filter_by_search();
                }
                self.layout_valid = false;
            }
        }
    }

    /// Get search string for display
    pub fn get_search_string(&self) -> &str {
        &self.search
    }

    /// Get search state for display
    pub fn get_search_state(&self) -> SearchState {
        self.search_state
    }

    /// Do incremental search (zsh msearch)
    fn do_incremental_search(&mut self) {
        if self.search.is_empty() {
            return;
        }

        let back = self.mode == MenuMode::BackwardSearch;
        let search_lower = self.search.to_lowercase();

        // Start from current position
        let start_idx = self.selected_idx.unwrap_or(0);
        let n = self.items.len();

        if n == 0 {
            self.search_state.failed = true;
            return;
        }

        // Search in specified direction
        let mut checked = 0;
        let mut idx = start_idx;
        let mut wrapped = false;

        loop {
            // Move to next position
            if back {
                if idx == 0 {
                    idx = n - 1;
                    wrapped = true;
                } else {
                    idx -= 1;
                }
            } else {
                idx = (idx + 1) % n;
                if idx == 0 {
                    wrapped = true;
                }
            }

            checked += 1;
            if checked > n {
                // Searched everything, no match
                self.search_state.failed = true;
                break;
            }

            // Check if this item matches
            if let Some(item) = self.items.get(idx) {
                let display_lower = item.display.to_lowercase();
                if display_lower.contains(&search_lower) {
                    // Found match
                    self.selected_idx = Some(idx);
                    self.search_state.failed = false;
                    self.search_state.wrapped = wrapped;

                    // Update mline/mcol from the new selection
                    let (row, col) = self.idx_to_visual_row_col(idx);
                    self.mline = row;
                    self.mcol = col;

                    self.ensure_selection_visible();

                    // Save for next search
                    self.last_search = self.search.clone();
                    return;
                }
            }

            // Back at start?
            if idx == start_idx {
                self.search_state.failed = true;
                break;
            }
        }
    }

    /// Continue search with last pattern
    pub fn search_again(&mut self, reverse: bool) {
        if self.last_search.is_empty() {
            return;
        }

        // Use last search string
        self.search = self.last_search.clone();

        if reverse {
            self.mode = MenuMode::BackwardSearch;
        } else {
            self.mode = MenuMode::ForwardSearch;
        }

        self.do_incremental_search();
    }

    // === Undo stack (zsh Menustack) ===

    /// Push current state to undo stack
    pub fn push_undo(&mut self, line: &str, cursor: usize) {
        self.undo_stack.push(MenuStackEntry {
            line: line.to_string(),
            cs: cursor,
            mline: self.mline,
            mlbeg: self.viewport_start,
            mselect: self.selected_idx,
            mcol: self.mcol,
            search: self.search.clone(),
            mode: self.mode,
        });
    }

    /// Pop from undo stack
    pub fn pop_undo(&mut self) -> Option<MenuStackEntry> {
        if let Some(entry) = self.undo_stack.pop() {
            self.mline = entry.mline;
            self.viewport_start = entry.mlbeg;
            self.selected_idx = entry.mselect;
            self.mcol = entry.mcol;
            self.search = entry.search.clone();
            self.mode = entry.mode;
            Some(entry)
        } else {
            None
        }
    }

    /// Check if undo is available
    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    // === Multi-select (zsh CMF_MULT, accept-and-hold) ===

    /// Mark current selection (like zsh accept-and-hold / Ctrl+Space)
    pub fn mark_current(&mut self) {
        if let Some(idx) = self.selected_idx {
            if self.marked_indices.contains(&idx) {
                self.marked_indices.remove(&idx);
            } else {
                self.marked_indices.insert(idx);
            }
        }
    }

    /// Mark current and move to next (accept-and-menu-complete)
    pub fn mark_and_next(&mut self) {
        self.mark_current();
        self.navigate(MenuMotion::Next);
    }

    /// Check if an index is marked
    pub fn is_marked(&self, idx: usize) -> bool {
        self.marked_indices.contains(&idx)
    }

    /// Get all marked completions
    pub fn marked_completions(&self) -> Vec<&Completion> {
        self.marked_indices
            .iter()
            .filter_map(|&idx| self.items.get(idx).map(|m| &m.completion))
            .collect()
    }

    /// Get all marked insert strings
    pub fn marked_insert_strings(&self) -> Vec<String> {
        self.marked_indices
            .iter()
            .filter_map(|&idx| self.items.get(idx).map(|m| m.completion.insert_str()))
            .collect()
    }

    /// Clear all marks
    pub fn clear_marks(&mut self) {
        self.marked_indices.clear();
    }

    /// Get number of marked items
    pub fn mark_count(&self) -> usize {
        self.marked_indices.len()
    }

    /// Check if multi-select is active (any marks)
    pub fn has_marks(&self) -> bool {
        !self.marked_indices.is_empty()
    }

    // === mtab-based navigation (zsh complist.c style) ===

    /// Get match at screen position (line, col) using mtab
    /// Returns (item_index, group_index) or None if no match at that position
    fn mtab_get(&self, line: usize, col: usize) -> Option<(usize, usize)> {
        if line >= self.mlines || col >= self.mcols {
            return None;
        }
        let pos = line * self.mcols + col;
        if pos >= self.mtab.len() {
            return None;
        }
        self.mtab[pos].map(|idx| (idx, self.mgtab[pos]))
    }

    /// Find the leftmost column of a match at the given position
    /// (matches can span multiple columns due to width)
    fn mtab_find_match_start(&self, line: usize, col: usize) -> usize {
        let pos = line * self.mcols + col;
        if pos >= self.mtab.len() {
            return col;
        }
        let target = self.mtab[pos];
        let row_start = line * self.mcols;

        let mut c = col;
        while c > 0 {
            let prev_pos = row_start + c - 1;
            if prev_pos >= self.mtab.len() || self.mtab[prev_pos] != target {
                break;
            }
            c -= 1;
        }
        c
    }

    /// Adjust column to find nearest valid match (zsh: adjust_mcol)
    /// Returns the adjusted column, or None if no valid match on this line
    fn adjust_mcol(&self, line: usize, wish_col: usize) -> Option<usize> {
        if line >= self.mlines {
            return None;
        }

        let row_start = line * self.mcols;

        // Check if wish_col has a valid match
        if wish_col < self.mcols {
            let pos = row_start + wish_col;
            if pos < self.mtab.len() && self.mtab[pos].is_some() {
                return Some(self.mtab_find_match_start(line, wish_col));
            }
        }

        // Search left and right for nearest match
        let mut left = wish_col.saturating_sub(1);
        let mut right = (wish_col + 1).min(self.mcols.saturating_sub(1));

        loop {
            // Check left
            if left < self.mcols {
                let pos = row_start + left;
                if pos < self.mtab.len() && self.mtab[pos].is_some() {
                    return Some(self.mtab_find_match_start(line, left));
                }
            }

            // Check right
            if right < self.mcols {
                let pos = row_start + right;
                if pos < self.mtab.len() && self.mtab[pos].is_some() {
                    return Some(self.mtab_find_match_start(line, right));
                }
            }

            // Move further out
            if left == 0 && right >= self.mcols.saturating_sub(1) {
                break;
            }
            left = left.saturating_sub(1);
            right = (right + 1).min(self.mcols.saturating_sub(1));
        }

        None
    }

    /// Navigate in the given direction (zsh domenuselect style)
    pub fn navigate(&mut self, motion: MenuMotion) -> bool {
        if self.items.is_empty() {
            return false;
        }

        self.ensure_layout();

        let old_idx = self.selected_idx;
        let rows = self.rows_for_items();
        let cols = self.cached_cols;

        match motion {
            MenuMotion::Deselect => {
                self.selected_idx = None;
            }

            // Absolute positions
            MenuMotion::First => {
                self.selected_idx = Some(0);
                self.wish_col = 0;
            }
            MenuMotion::Last => {
                self.selected_idx = Some(self.items.len() - 1);
                self.wish_col = cols.saturating_sub(1);
            }
            MenuMotion::BeginningOfLine => {
                if let Some(idx) = self.selected_idx {
                    let (row, _) = self.idx_to_visual_row_col(idx);
                    if let Some(new_idx) = self.visual_row_col_to_idx(row, 0) {
                        self.selected_idx = Some(new_idx);
                        self.wish_col = 0;
                    }
                }
            }
            MenuMotion::EndOfLine => {
                if let Some(idx) = self.selected_idx {
                    let (row, _) = self.idx_to_visual_row_col(idx);
                    let max_col = if let Some((_, group, _)) = self.find_group_for_idx(idx) {
                        group.cols.saturating_sub(1)
                    } else {
                        cols.saturating_sub(1)
                    };
                    // Find last valid item in row
                    for c in (0..=max_col).rev() {
                        if let Some(new_idx) = self.visual_row_col_to_idx(row, c) {
                            self.selected_idx = Some(new_idx);
                            self.wish_col = c;
                            break;
                        }
                    }
                }
            }

            // Sequential navigation
            MenuMotion::Next => match self.selected_idx {
                None => self.selected_idx = Some(0),
                Some(idx) => {
                    self.selected_idx = Some((idx + 1) % self.items.len());
                }
            },
            MenuMotion::Prev => match self.selected_idx {
                None => self.selected_idx = Some(self.items.len() - 1),
                Some(0) => self.selected_idx = Some(self.items.len() - 1),
                Some(idx) => self.selected_idx = Some(idx - 1),
            },

            // Group navigation (zsh viforwardblankword/vibackwardblankword)
            MenuMotion::ForwardBlankWord => {
                if let Some(idx) = self.selected_idx {
                    if let Some((group_idx, _, _)) = self.find_group_for_idx(idx) {
                        // Find first item in next group
                        let mut offset = 0;
                        for (i, g) in self.groups.iter().enumerate() {
                            if i == group_idx + 1 && g.count > 0 {
                                self.selected_idx = Some(offset);
                                self.wish_col = 0;
                                break;
                            }
                            offset += g.count;
                        }
                    }
                }
            }
            MenuMotion::BackwardBlankWord => {
                if let Some(idx) = self.selected_idx {
                    if let Some((group_idx, _, _)) = self.find_group_for_idx(idx) {
                        if group_idx > 0 {
                            // Find first item in previous group
                            let mut offset = 0;
                            for (i, g) in self.groups.iter().enumerate() {
                                if i == group_idx - 1 {
                                    self.selected_idx = Some(offset);
                                    self.wish_col = 0;
                                    break;
                                }
                                offset += g.count;
                            }
                        }
                    }
                }
            }

            // Word movement (page-like within visible area)
            MenuMotion::ForwardWord => {
                let page = self.available_rows.saturating_sub(2);
                if let Some(idx) = self.selected_idx {
                    let (row, _) = self.idx_to_visual_row_col(idx);
                    let new_row = (row + page).min(rows.saturating_sub(1));
                    self.try_select_row(new_row);
                }
            }
            MenuMotion::BackwardWord => {
                let page = self.available_rows.saturating_sub(2);
                if let Some(idx) = self.selected_idx {
                    let (row, _) = self.idx_to_visual_row_col(idx);
                    let new_row = row.saturating_sub(page);
                    self.try_select_row(new_row);
                }
            }

            // Accept actions (handled by caller)
            MenuMotion::Accept | MenuMotion::AcceptAndHold | MenuMotion::AcceptAndMenuComplete => {
                // These are handled by the caller
            }

            // Undo
            MenuMotion::Undo => {
                // Handled by pop_undo
            }

            // Directional movement
            MenuMotion::Up
            | MenuMotion::Down
            | MenuMotion::Left
            | MenuMotion::Right
            | MenuMotion::PageUp
            | MenuMotion::PageDown => {
                if let Some(idx) = self.selected_idx {
                    // Use group-aware row/col calculation
                    let (row, col) = self.idx_to_visual_row_col(idx);
                    let total_rows = rows;

                    let (new_row, new_col) = match motion {
                        MenuMotion::Up => {
                            if row > 0 {
                                (row - 1, self.wish_col)
                            } else {
                                // Wrap to bottom
                                (total_rows.saturating_sub(1), self.wish_col)
                            }
                        }
                        MenuMotion::Down => {
                            if row + 1 < total_rows {
                                (row + 1, self.wish_col)
                            } else {
                                // Wrap to top
                                (0, self.wish_col)
                            }
                        }
                        MenuMotion::Left => {
                            if col > 0 {
                                self.wish_col = col - 1;
                                (row, col - 1)
                            } else {
                                // Wrap to previous row, last column
                                if row > 0 {
                                    // Get column count for previous row's group
                                    if let Some(prev_idx) = self.visual_row_col_to_idx(row - 1, 0) {
                                        if let Some((_, group, _)) =
                                            self.find_group_for_idx(prev_idx)
                                        {
                                            let last_col = group.cols.saturating_sub(1);
                                            self.wish_col = last_col;
                                            (row - 1, last_col)
                                        } else {
                                            (row, col)
                                        }
                                    } else {
                                        (row, col)
                                    }
                                } else {
                                    (row, col)
                                }
                            }
                        }
                        MenuMotion::Right => {
                            // Get current group's column count
                            let max_col = if let Some((_, group, _)) = self.find_group_for_idx(idx)
                            {
                                group.cols.saturating_sub(1)
                            } else {
                                cols.saturating_sub(1)
                            };

                            if col < max_col {
                                self.wish_col = col + 1;
                                (row, col + 1)
                            } else {
                                // Wrap to next row, first column
                                if row + 1 < total_rows {
                                    self.wish_col = 0;
                                    (row + 1, 0)
                                } else {
                                    (row, col)
                                }
                            }
                        }
                        MenuMotion::PageUp => {
                            let page = self.scroll_step();
                            (row.saturating_sub(page), self.wish_col)
                        }
                        MenuMotion::PageDown => {
                            let page = self.scroll_step();
                            (
                                (row + page).min(total_rows.saturating_sub(1)),
                                self.wish_col,
                            )
                        }
                        _ => (row, col),
                    };

                    // Convert back to index using group-aware function
                    if let Some(new_idx) = self.visual_row_col_to_idx(new_row, new_col) {
                        self.selected_idx = Some(new_idx);
                    } else {
                        // Try to find a valid item in that row
                        self.try_select_row(new_row);
                    }
                }
            }
        }

        self.ensure_selection_visible();
        self.selected_idx != old_idx
    }

    /// Try to select any valid item in the given row
    fn try_select_row(&mut self, row: usize) {
        // Try wish_col first
        if let Some(idx) = self.visual_row_col_to_idx(row, self.wish_col) {
            self.selected_idx = Some(idx);
            return;
        }
        // Try from wish_col backwards
        for c in (0..self.wish_col).rev() {
            if let Some(idx) = self.visual_row_col_to_idx(row, c) {
                self.selected_idx = Some(idx);
                return;
            }
        }
        // Try from wish_col forwards
        for c in (self.wish_col + 1)..self.cached_cols {
            if let Some(idx) = self.visual_row_col_to_idx(row, c) {
                self.selected_idx = Some(idx);
                return;
            }
        }
    }

    /// Get scroll step size (zsh: MENUSCROLL)
    fn scroll_step(&self) -> usize {
        if self.scroll_step > 0 {
            self.scroll_step
        } else {
            // Default: half screen
            self.available_rows.saturating_sub(1) / 2
        }
    }

    /// Set scroll step size (from MENUSCROLL parameter)
    pub fn set_scroll_step(&mut self, step: usize) {
        self.scroll_step = step;
    }

    /// Number of rows needed for items (excluding headers)
    fn rows_for_items(&self) -> usize {
        // Sum up rows from all groups
        self.groups.iter().map(|g| g.row_count).sum()
    }

    /// Find which group an item index belongs to, and its local index within that group
    fn find_group_for_idx(&self, idx: usize) -> Option<(usize, &GroupLayout, usize)> {
        let mut offset = 0;
        for (gi, group) in self.groups.iter().enumerate() {
            if idx < offset + group.count {
                return Some((gi, group, idx - offset));
            }
            offset += group.count;
        }
        None
    }

    /// Convert global item index to (visual_row, col) accounting for per-group column counts
    fn idx_to_visual_row_col(&self, idx: usize) -> (usize, usize) {
        let mut row_offset = 0;
        let mut item_offset = 0;

        for group in &self.groups {
            if idx < item_offset + group.count {
                let local_idx = idx - item_offset;
                let cols = group.cols.max(1);
                let local_row = local_idx / cols;
                let col = local_idx % cols;
                return (row_offset + local_row, col);
            }
            row_offset += group.row_count;
            item_offset += group.count;
        }
        (0, 0)
    }

    /// Convert (visual_row, col) back to global item index
    fn visual_row_col_to_idx(&self, target_row: usize, target_col: usize) -> Option<usize> {
        let mut row_offset = 0;
        let mut item_offset = 0;

        for group in &self.groups {
            let group_end_row = row_offset + group.row_count;
            if target_row < group_end_row {
                let local_row = target_row - row_offset;
                let cols = group.cols.max(1);
                let local_idx = local_row * cols + target_col.min(cols - 1);
                if local_idx < group.count {
                    return Some(item_offset + local_idx);
                }
                return None;
            }
            row_offset += group.row_count;
            item_offset += group.count;
        }
        None
    }

    /// Ensure the selected item is visible in the viewport
    fn ensure_selection_visible(&mut self) {
        if let Some(idx) = self.selected_idx {
            // Use group-aware row calculation
            let (item_row, _) = self.idx_to_visual_row_col(idx);

            // Account for headers
            let display_row = self.item_row_to_display_row(item_row);

            if display_row < self.viewport_start {
                self.viewport_start = display_row;
            } else if display_row >= self.viewport_start + self.available_rows {
                self.viewport_start = display_row.saturating_sub(self.available_rows - 1);
            }
        }
    }

    /// Navigate to first item of next group
    fn navigate_to_next_group(&mut self) {
        if self.groups.is_empty() || self.items.is_empty() {
            return;
        }

        let current_idx = self.selected_idx.unwrap_or(0);

        // Find which group current selection is in
        let mut current_group = 0;
        let mut offset = 0;
        for (i, g) in self.groups.iter().enumerate() {
            if current_idx < offset + g.count {
                current_group = i;
                break;
            }
            offset += g.count;
        }

        // Move to next group (wrap around)
        let next_group = (current_group + 1) % self.groups.len();
        let mut new_idx = 0;
        for (i, g) in self.groups.iter().enumerate() {
            if i == next_group {
                break;
            }
            new_idx += g.count;
        }

        self.selected_idx = Some(new_idx);
        self.ensure_selection_visible();
    }

    /// Navigate to first item of previous group
    fn navigate_to_prev_group(&mut self) {
        if self.groups.is_empty() || self.items.is_empty() {
            return;
        }

        let current_idx = self.selected_idx.unwrap_or(0);

        // Find which group current selection is in
        let mut current_group = 0;
        let mut offset = 0;
        for (i, g) in self.groups.iter().enumerate() {
            if current_idx < offset + g.count {
                current_group = i;
                break;
            }
            offset += g.count;
        }

        // Move to previous group (wrap around)
        let prev_group = if current_group == 0 {
            self.groups.len() - 1
        } else {
            current_group - 1
        };

        let mut new_idx = 0;
        for (i, g) in self.groups.iter().enumerate() {
            if i == prev_group {
                break;
            }
            new_idx += g.count;
        }

        self.selected_idx = Some(new_idx);
        self.ensure_selection_visible();
    }

    /// Navigate to start of current row (vi-beginning-of-line)
    fn navigate_to_row_start(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.ensure_layout();

        if let Some(idx) = self.selected_idx {
            let (row, _col) = self.idx_to_visual_row_col(idx);

            // Find first valid item in this row (column 0)
            if let Some(new_idx) = self.visual_row_col_to_idx(row, 0) {
                self.selected_idx = Some(new_idx);
                self.wish_col = 0;
            }
        }
    }

    /// Navigate to end of current row (vi-end-of-line)
    fn navigate_to_row_end(&mut self) {
        if self.items.is_empty() {
            return;
        }
        self.ensure_layout();

        if let Some(idx) = self.selected_idx {
            let (row, _col) = self.idx_to_visual_row_col(idx);

            // Get the group's column count for this row
            let max_col = if let Some((_, group, _)) = self.find_group_for_idx(idx) {
                group.cols.saturating_sub(1)
            } else {
                0
            };

            // Find last valid item in this row
            for try_col in (0..=max_col).rev() {
                if let Some(new_idx) = self.visual_row_col_to_idx(row, try_col) {
                    self.selected_idx = Some(new_idx);
                    self.wish_col = try_col;
                    return;
                }
            }
        }
    }

    #[allow(dead_code)]
    /// Old function kept for compatibility - use idx_to_visual_row_col instead
    fn idx_to_row_col(&self, idx: usize, _rows: usize, cols: usize) -> (usize, usize) {
        if cols == 0 {
            return (0, 0);
        }
        let row = idx / cols;
        let col = idx % cols;
        (row, col)
    }

    #[allow(dead_code)]
    /// Old function kept for compatibility - use visual_row_col_to_idx instead
    fn row_col_to_idx(&self, row: usize, col: usize, _rows: usize, cols: usize) -> usize {
        row * cols + col
    }

    /// Convert item row to display row (accounting for headers)
    fn item_row_to_display_row(&self, item_row: usize) -> usize {
        if !self.show_headers || self.groups.len() <= 1 {
            return item_row;
        }

        let cols = self.cached_cols.max(1);
        let mut display_row = 0;
        let mut items_so_far = 0;

        for group in &self.groups {
            if group.count == 0 {
                continue;
            }

            // Add header row
            if group.explanation.is_some() {
                display_row += 1;
            }

            let group_rows = (group.count + cols - 1) / cols;
            let group_start_item_row = items_so_far / cols;
            let group_end_item_row = group_start_item_row + group_rows;

            if item_row < group_end_item_row {
                return display_row + (item_row - group_start_item_row);
            }

            display_row += group_rows;
            items_so_far += group.count;
        }

        display_row
    }

    /// Calculate layout if needed
    fn ensure_layout(&mut self) {
        if self.layout_valid {
            return;
        }

        if self.items.is_empty() {
            self.cached_cols = 0;
            self.cached_total_rows = 0;
            self.cached_col_width = 0;
            self.layout_valid = true;
            return;
        }

        let tw = self.term_width;
        let mut total_rows = 0;
        let mut item_offset = 0;

        // Calculate layout PER GROUP - each group gets its own column count
        for group in &mut self.groups {
            if group.count == 0 {
                group.cols = 0;
                group.col_widths.clear();
                group.row_count = 0;
                continue;
            }

            // Header
            if self.show_headers && group.explanation.is_some() {
                total_rows += 1;
            }

            let items = &self.items[item_offset..item_offset + group.count];
            let n = group.count;

            // Check if any items have descriptions
            let has_descriptions = items.iter().any(|item| !item.description.is_empty());
            group.has_descriptions = has_descriptions;

            if has_descriptions {
                // Multi-column layout WITH descriptions (like zsh - pack tightly!)
                // Each "column" = completion + separator + description
                let max_comp_width = items.iter().map(|i| i.comp_width).max().unwrap_or(10);
                let max_desc_width = items.iter().map(|i| i.desc_width).max().unwrap_or(10);
                let separator_width = 11; // " ///////// "

                // Width of one complete entry (comp + sep + desc + small padding)
                let entry_width = max_comp_width + separator_width + max_desc_width + 2;

                // How many columns fit? Pack tightly!
                let cols = (tw / entry_width).max(1).min(n);

                // Use actual entry width, not distributed width - no dead space!
                group.comp_col_width = max_comp_width + 2;
                group.cols = cols;
                group.col_widths = vec![entry_width; cols];
                group.row_count = (n + cols - 1) / cols;
            } else {
                // Multi-column layout for items without descriptions
                let max_cols = n.min(tw / 2); // At least 2 chars per item
                let mut best_cols = 1;
                let mut best_widths = vec![tw];

                for try_cols in (1..=max_cols).rev() {
                    // Calculate per-column max width (row-major layout)
                    let mut col_widths = vec![0usize; try_cols];
                    for (i, item) in items.iter().enumerate() {
                        let col = i % try_cols;
                        let item_width = item.comp_width + 2;
                        col_widths[col] = col_widths[col].max(item_width);
                    }

                    let total: usize = col_widths.iter().sum();
                    if total <= tw {
                        best_cols = try_cols;
                        // Distribute extra space
                        let extra = (tw - total) / try_cols;
                        for w in &mut col_widths {
                            *w += extra;
                        }
                        best_widths = col_widths;
                        break;
                    }
                }

                group.cols = best_cols;
                group.col_widths = best_widths;
                group.row_count = (n + best_cols - 1) / best_cols;
                group.comp_col_width = 0;
            }

            group.start_row = total_rows;
            total_rows += group.row_count;
            item_offset += group.count;
        }

        self.cached_cols = self.groups.first().map(|g| g.cols).unwrap_or(1);
        self.cached_col_width = tw / self.cached_cols.max(1);
        self.cached_total_rows = total_rows;

        // Build mtab/mgtab grid (like zsh complist.c lines 2084-2098)
        // This maps each screen position to a match/group
        self.mcols = tw; // zsh uses full terminal width
        self.mlines = total_rows;

        let grid_size = self.mcols * self.mlines;
        self.mtab = vec![None; grid_size];
        self.mgtab = vec![0; grid_size];

        // Fill the grid by iterating through groups and their items
        let mut display_row = 0;
        let mut global_idx = 0;

        for (group_idx, group) in self.groups.iter().enumerate() {
            if group.count == 0 {
                continue;
            }

            // Header row - mark as explanation (no match)
            if self.show_headers && group.explanation.is_some() {
                let row_start = display_row * self.mcols;
                for x in 0..self.mcols {
                    if row_start + x < grid_size {
                        self.mtab[row_start + x] = None; // Header, no match
                        self.mgtab[row_start + x] = group_idx;
                    }
                }
                display_row += 1;
            }

            // Item rows
            let cols = group.cols.max(1);
            for row in 0..group.row_count {
                let row_start = display_row * self.mcols;
                let mut x = 0usize;

                for col in 0..cols {
                    let local_idx = row * cols + col;
                    let idx = global_idx + local_idx;
                    let cw = group.col_widths.get(col).copied().unwrap_or(tw / cols);

                    // Fill all character positions in this column with the same match
                    for cx in 0..cw {
                        let pos = row_start + x + cx;
                        if pos < grid_size {
                            if local_idx < group.count && idx < self.items.len() {
                                self.mtab[pos] = Some(idx);
                            } else {
                                self.mtab[pos] = None;
                            }
                            self.mgtab[pos] = group_idx;
                        }
                    }
                    x += cw;
                }
                display_row += 1;
            }

            global_idx += group.count;
        }

        self.layout_valid = true;
    }

    /// Check if only selection changed (can use singledraw optimization)
    /// Returns Some((old_idx, new_idx)) if only selection changed, None otherwise
    pub fn selection_changed_only(&self) -> Option<(Option<usize>, Option<usize>)> {
        // If viewport changed, need full redraw
        if self.prev_viewport_start != Some(self.viewport_start) {
            return None;
        }

        // If mline/mcol significantly changed, need full redraw
        // (This is a simplification - zsh has more complex logic)

        // For now, just track if we have a previous position
        let old_idx = if self.prev_mline < self.mlines && self.prev_mcol < self.mcols {
            let pos = self.prev_mline * self.mcols + self.prev_mcol;
            if pos < self.mtab.len() {
                self.mtab[pos]
            } else {
                None
            }
        } else {
            None
        };

        Some((old_idx, self.selected_idx))
    }

    /// Record current state for next singledraw check
    pub fn record_render_state(&mut self) {
        self.prev_viewport_start = Some(self.viewport_start);
        self.prev_mline = self.mline;
        self.prev_mcol = self.mcol;
    }

    /// Render just the changed cells (zsh singledraw)
    /// Returns cursor movement commands and the two lines to redraw
    pub fn render_singledraw(
        &mut self,
        old_idx: Option<usize>,
        new_idx: Option<usize>,
    ) -> Option<SingleDrawResult> {
        self.ensure_layout();

        // Need to find screen positions of old and new selection
        let old_pos = old_idx.and_then(|idx| {
            let (row, col) = self.idx_to_visual_row_col(idx);
            if row >= self.viewport_start && row < self.viewport_start + self.available_rows {
                Some((row - self.viewport_start, col, idx))
            } else {
                None
            }
        });

        let new_pos = new_idx.and_then(|idx| {
            let (row, col) = self.idx_to_visual_row_col(idx);
            if row >= self.viewport_start && row < self.viewport_start + self.available_rows {
                Some((row - self.viewport_start, col, idx))
            } else {
                None
            }
        });

        // Render the specific items
        let old_rendered = old_pos.map(|(screen_row, col, idx)| {
            if let Some(item) = self.items.get(idx) {
                let group_color = self
                    .groups
                    .get(item.group_idx)
                    .map(|g| g.color.as_str())
                    .unwrap_or("0");
                let col_width = self
                    .groups
                    .get(item.group_idx)
                    .and_then(|g| g.col_widths.first().copied())
                    .unwrap_or(20);

                let mut line = MenuLine::new();
                // Render without selection (old position)
                let saved_sel = self.selected_idx;
                self.selected_idx = None; // Temporarily clear to render unselected
                self.render_item(&mut line, item, idx, col, col_width, false, group_color);
                self.selected_idx = saved_sel;

                (screen_row, col, line)
            } else {
                (screen_row, col, MenuLine::new())
            }
        });

        let new_rendered = new_pos.map(|(screen_row, col, idx)| {
            if let Some(item) = self.items.get(idx) {
                let group_color = self
                    .groups
                    .get(item.group_idx)
                    .map(|g| g.color.as_str())
                    .unwrap_or("0");
                let col_width = self
                    .groups
                    .get(item.group_idx)
                    .and_then(|g| g.col_widths.first().copied())
                    .unwrap_or(20);

                let mut line = MenuLine::new();
                self.render_item(&mut line, item, idx, col, col_width, false, group_color);

                (screen_row, col, line)
            } else {
                (screen_row, col, MenuLine::new())
            }
        });

        // Update mline/mcol from new selection
        if let Some(idx) = new_idx {
            let (row, col) = self.idx_to_visual_row_col(idx);
            self.mline = row;
            self.mcol = col;
        }

        self.record_render_state();

        Some(SingleDrawResult {
            old_cell: old_rendered,
            new_cell: new_rendered,
        })
    }

    /// Render the menu to displayable lines
    pub fn render(&mut self) -> MenuRendering {
        self.ensure_layout();

        let mut rendering = MenuRendering {
            term_width: self.term_width,
            term_height: self.term_height,
            total_matches: self.items.len(),
            cols: self.cached_cols,
            total_rows: self.cached_total_rows,
            row_start: self.viewport_start,
            row_end: (self.viewport_start + self.available_rows).min(self.cached_total_rows),
            selected_idx: self.selected_idx,
            selected_row: 0,
            selected_col: 0,
            lines: Vec::new(),
            status: None,
            scrollable: self.cached_total_rows > self.available_rows,
        };

        if self.items.is_empty() {
            return rendering;
        }

        // Build display rows - each group has its own column layout
        let mut display_row = 0;
        let mut global_idx = 0;

        for group in &self.groups {
            if group.count == 0 {
                continue;
            }

            let cols = group.cols.max(1);

            // Header row - use group color for cyberpunk effect
            if self.show_headers && group.explanation.is_some() {
                if display_row >= self.viewport_start && display_row < rendering.row_end {
                    let header_text = group.explanation.as_deref().unwrap_or(&group.name);
                    // Bold + group color for header
                    let header_color = format!("1;{}", group.color);
                    let mut line = MenuLine::header(header_text, &header_color);
                    line.pad_to(self.term_width);
                    rendering.lines.push(line);
                }
                display_row += 1;
            }

            // Item rows - ROW MAJOR (left to right, then down)
            for row in 0..group.row_count {
                if display_row >= self.viewport_start && display_row < rendering.row_end {
                    let mut line = MenuLine::new();

                    if group.has_descriptions {
                        // Multi-column with descriptions - each entry padded to same width
                        let entry_width =
                            group.col_widths.first().copied().unwrap_or(self.term_width);
                        for col in 0..group.cols {
                            let local_idx = row * group.cols + col;
                            let idx = global_idx + local_idx;

                            if local_idx < group.count {
                                if let Some(item) = self.items.get(idx) {
                                    self.render_item_with_desc_column(
                                        &mut line,
                                        item,
                                        idx,
                                        group.comp_col_width,
                                        entry_width,
                                        &group.color,
                                    );
                                }
                            }
                        }
                    } else {
                        // Multi-column layout without descriptions (like zsh clprintm)
                        for col in 0..cols {
                            let local_idx = row * cols + col;
                            let idx = global_idx + local_idx;
                            let cw = group
                                .col_widths
                                .get(col)
                                .copied()
                                .unwrap_or(self.term_width / cols);

                            if local_idx < group.count {
                                if let Some(item) = self.items.get(idx) {
                                    self.render_item(
                                        &mut line,
                                        item,
                                        idx,
                                        col,
                                        cw,
                                        display_row % 2 == 1,
                                        &group.color,
                                    );
                                }
                            } else {
                                // Empty cell - just pad with spaces
                                for _ in 0..cw {
                                    line.content.push(' ');
                                }
                                line.width += cw;
                            }
                        }
                    }
                    rendering.lines.push(line);
                }
                display_row += 1;
            }

            global_idx += group.count;
        }

        // Status line - zsh MENUPROMPT style with %m, %l, %p escapes
        // Default: "%SScrolling active: current selection at %p%s"
        rendering.status = Some(self.format_status_line(rendering.row_start + 1));

        rendering
    }

    /// Format status line with zsh MENUPROMPT escapes
    /// %n - number of matches in current group
    /// %m - selected match number / total matches (e.g. "5/47")
    /// %M - same as %m but left-padded to 9 chars
    /// %l - current line / total lines (e.g. "3/12")
    /// %L - same as %l but left-padded to 9 chars  
    /// %p - position indicator (Top/Bottom/XX%)
    /// %P - same as %p but padded
    /// %S/%s - standout mode on/off
    /// %B/%b - bold on/off
    /// %U/%u - underline on/off
    fn format_status_line(&self, current_line: usize) -> String {
        let total_matches = self.items.len();
        let total_lines = self.cached_total_rows;
        let sel_num = self.selected_idx.map(|i| i + 1).unwrap_or(0);

        // Position indicator
        let position = if current_line >= total_lines {
            "Bottom".to_string()
        } else if self.viewport_start == 0 {
            "Top".to_string()
        } else {
            format!("{}%", (current_line * 100) / total_lines.max(1))
        };

        // Check mode for special status
        match self.mode {
            MenuMode::Interactive => {
                format!("interactive: [{}] ({} matches)", self.search, total_matches)
            }
            MenuMode::ForwardSearch | MenuMode::BackwardSearch => {
                let dir = if self.mode == MenuMode::BackwardSearch {
                    " backward"
                } else {
                    ""
                };
                let state = if self.search_state.failed {
                    "failed "
                } else if self.search_state.wrapped {
                    "wrapped "
                } else {
                    ""
                };
                format!(
                    "{}{}isearch{}: {}",
                    state,
                    dir,
                    if self.mode == MenuMode::BackwardSearch {
                        ""
                    } else {
                        ""
                    },
                    self.search
                )
            }
            MenuMode::Normal => {
                if self.cached_total_rows > self.available_rows {
                    // Scrolling active
                    format!("\x1b[7mScrolling active: current selection at {}\x1b[0m  {}/{}  line {}/{}",
                        position, sel_num, total_matches, current_line, total_lines)
                } else {
                    // Not scrolling, just show match info
                    format!(
                        "{}/{}  line {}/{}",
                        sel_num, total_matches, current_line, total_lines
                    )
                }
            }
        }
    }

    /// Render a single item into a line
    fn render_item(
        &self,
        line: &mut MenuLine,
        item: &MenuItem,
        idx: usize,
        _col: usize,
        col_width: usize,
        _secondary: bool,
        group_color: &str,
    ) {
        let is_selected = self.selected_idx == Some(idx);
        let is_marked = self.marked_indices.contains(&idx);
        let available = col_width.saturating_sub(1); // 1 char spacing

        if available == 0 {
            return;
        }

        // Completion text (possibly truncated)
        let display = if item.comp_width > available {
            truncate_with_ellipsis(&item.display, available)
        } else {
            item.display.clone()
        };

        // Calculate prefix match length (case-insensitive)
        let prefix_len = if !self.prefix.is_empty() {
            let prefix_lower = self.prefix.to_lowercase();
            let display_lower = display.to_lowercase();
            if display_lower.starts_with(&prefix_lower) {
                self.prefix.chars().count()
            } else {
                0
            }
        } else {
            0
        };

        // Split into prefix (highlighted) and rest
        let (prefix_part, rest_part) = if prefix_len > 0 {
            let char_boundary: usize = display.chars().take(prefix_len).map(|c| c.len_utf8()).sum();
            (&display[..char_boundary], &display[char_boundary..])
        } else {
            ("", display.as_str())
        };

        // Determine effective color - use LS_COLORS for file completions
        let effective_color = self.get_item_color(item, group_color);

        // Marked items get special color (zsh COL_DU - "duplicate/multi")
        // Selected highlight - use parsed ma= color from zstyle
        if is_selected {
            line.content
                .push_str(&ansi::from_codes(&self.selection_color));
            line.content.push_str(prefix_part);
            line.content.push_str(rest_part);
            line.content.push_str(ansi::RESET);
        } else if is_marked {
            // Marked but not selected: underline + dim (zsh COL_DU style)
            line.content.push_str("\x1b[4;2m"); // underline + dim
            line.content.push_str(prefix_part);
            line.content.push_str(rest_part);
            line.content.push_str(ansi::RESET);
        } else {
            // zsh style: prefix is BOLD WHITE to stand out, rest uses group color
            if !prefix_part.is_empty() {
                line.content.push_str("\x1b[1;37m"); // Bold white for prefix
                line.content.push_str(prefix_part);
                line.content.push_str(ansi::RESET);
            }
            // Item color for the rest
            line.content.push_str(&ansi::from_codes(&effective_color));
            line.content.push_str(rest_part);
            line.content.push_str(ansi::RESET);
        }

        let disp_width = display_width(&display);

        // Pad to column width (like zsh clprintm lines 1890-1896)
        let pad = col_width.saturating_sub(disp_width);
        for _ in 0..pad {
            line.content.push(' ');
        }
        line.width += col_width;
    }

    /// Get the color for an item, using LS_COLORS for file completions
    /// Supports LC_FOLLOW_SYMLINKS behavior: color symlinks by target type
    fn get_item_color(&self, item: &MenuItem, group_color: &str) -> String {
        // Check if this looks like a file completion
        let display = &item.display;
        let is_dir = display.ends_with('/') || item.completion.modec == '/';
        let is_link = item.completion.modec == '@';
        let is_exec = item.completion.modec == '*';

        // Check if this is a file-related group (use LS_COLORS for all items)
        let group_name = self
            .groups
            .get(item.group_idx)
            .map(|g| g.name.as_str())
            .unwrap_or("");
        let is_file_group = matches!(
            group_name,
            "files"
                | "file"
                | "all-files"
                | "globbed-files"
                | "local-directories"
                | "directories"
                | "directory"
                | "path"
                | "paths"
        );

        // Use LS_COLORS for file groups or items with file mode set
        if is_file_group || is_dir || is_link || is_exec {
            // For symlinks with LC_FOLLOW_SYMLINKS: resolve target and color by target type
            if is_link {
                let path = display.trim_end_matches('@');
                if let Some(target_color) = self.get_symlink_target_color(path) {
                    return target_color;
                }
                // Fall through to normal symlink color
            }

            let color = crate::zpwr_colors::ls_color_for_file(display, is_dir, is_exec, is_link);
            if !color.is_empty() {
                return color;
            }
        }

        // Fall back to group color
        group_color.to_string()
    }

    /// Get color for symlink based on its target (LC_FOLLOW_SYMLINKS behavior)
    /// Returns None if symlink resolution fails or we should use default symlink color
    fn get_symlink_target_color(&self, path: &str) -> Option<String> {
        // Only follow symlinks if enabled
        if !self.follow_symlinks {
            return None;
        }

        // Try to resolve the symlink target
        let path = std::path::Path::new(path);

        // Use fs::metadata which follows symlinks (vs symlink_metadata which doesn't)
        match std::fs::metadata(path) {
            Ok(meta) => {
                let is_dir = meta.is_dir();
                let is_exec = {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        meta.permissions().mode() & 0o111 != 0
                    }
                    #[cfg(not(unix))]
                    {
                        false
                    }
                };

                // Color by target type
                let filename = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

                Some(crate::zpwr_colors::ls_color_for_file(
                    filename, is_dir, is_exec, false,
                ))
            }
            Err(_) => {
                // Broken symlink - use orphan color (typically red)
                Some("1;31".to_string()) // Bold red for broken symlinks
            }
        }
    }

    /// Render item with properly aligned description column (zsh-style)
    fn render_item_with_desc_column(
        &self,
        line: &mut MenuLine,
        item: &MenuItem,
        idx: usize,
        comp_col_width: usize,
        entry_width: usize,
        group_color: &str,
    ) {
        let is_selected = self.selected_idx == Some(idx);
        let display = &item.display;

        // Calculate prefix match for highlighting
        let prefix_len = if !self.prefix.is_empty() {
            let prefix_lower = self.prefix.to_lowercase();
            let disp_lower = display.to_lowercase();
            if disp_lower.starts_with(&prefix_lower) {
                self.prefix.chars().count()
            } else {
                0
            }
        } else {
            0
        };

        let (prefix_part, rest_part) = if prefix_len > 0 {
            let char_boundary: usize = display.chars().take(prefix_len).map(|c| c.len_utf8()).sum();
            (&display[..char_boundary], &display[char_boundary..])
        } else {
            ("", display.as_str())
        };

        // Determine effective color - use LS_COLORS for file completions
        let effective_color = self.get_item_color(item, group_color);

        // Render completion with color
        if is_selected {
            line.content
                .push_str(&ansi::from_codes(&self.selection_color));
            line.content.push_str(prefix_part);
            line.content.push_str(rest_part);
            line.content.push_str(ansi::RESET);
        } else {
            // zsh style: prefix is BOLD WHITE to stand out, rest uses group color
            if !prefix_part.is_empty() {
                line.content.push_str("\x1b[1;37m"); // Bold white for prefix
                line.content.push_str(prefix_part);
                line.content.push_str(ansi::RESET);
            }
            // Rest in item color
            line.content.push_str(&ansi::from_codes(&effective_color));
            line.content.push_str(rest_part);
            line.content.push_str(ansi::RESET);
        }

        // Pad completion to computed column width (aligned columns!)
        let comp_width = display_width(display);
        for _ in comp_width..comp_col_width {
            line.content.push(' ');
        }

        line.width = comp_col_width;

        // Separator from zstyle (ZPWR_CHAR_LOGO)
        let separator = &self.list_separator;
        line.content.push_str("\x1b[2m");
        line.content.push_str(&separator);
        line.content.push_str(ansi::RESET);
        line.content.push(' ');
        line.width += UnicodeWidthStr::width(separator.as_str()) + 1;

        // Description column (cyan)
        if !item.description.is_empty() {
            line.content
                .push_str(&ansi::from_codes(&self.colors.description));
            line.content.push_str(&item.description);
            line.content.push_str(ansi::RESET);
            line.width += item.desc_width;
        }

        // Pad to entry width for aligned multi-column layout
        for _ in line.width..entry_width {
            line.content.push(' ');
        }
        line.width = entry_width;
    }

    /// Accept current selection and continue (multi-select)
    pub fn accept_and_continue(&mut self) -> Option<&Completion> {
        if let Some(idx) = self.selected_idx {
            self.navigate(MenuMotion::Next);
            return self.items.get(idx).map(|m| &m.completion);
        }
        None
    }

    /// Clear search
    pub fn search_clear(&mut self) {
        self.search.clear();
        self.search_active = false;
        self.items = self.unfiltered_items.clone();
        self.layout_valid = false;
    }

    /// Filter items by search string (used by interactive mode)
    fn filter_by_search(&mut self) {
        self.search_active = !self.search.is_empty();
        if self.search.is_empty() {
            self.items = self.unfiltered_items.clone();
            self.layout_valid = false;
            return;
        }

        let search_lower = self.search.to_lowercase();
        self.items = self
            .unfiltered_items
            .iter()
            .filter(|item| {
                item.display.to_lowercase().contains(&search_lower)
                    || item.description.to_lowercase().contains(&search_lower)
            })
            .cloned()
            .collect();

        self.layout_valid = false;

        // Reset selection if current is filtered out
        if let Some(idx) = self.selected_idx {
            if idx >= self.items.len() {
                self.selected_idx = if self.items.is_empty() { None } else { Some(0) };
            }
        }
    }
}

/// Calculate display width of a string using unicode-width
fn display_width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Make a color code bold by prepending "1;" if not already bold
/// e.g., "32" -> "1;32", "1;32" -> "1;32"
#[allow(dead_code)]
fn make_bold(color: &str) -> String {
    if color.is_empty() {
        return "1".to_string();
    }
    // Check if already has bold (starts with "1;" or contains ";1;" or is just "1")
    if color == "1" || color.starts_with("1;") || color.contains(";1;") || color.contains(";1") {
        color.to_string()
    } else {
        format!("1;{}", color)
    }
}

/// Truncate string with ellipsis to fit width
fn truncate_with_ellipsis(s: &str, max_width: usize) -> String {
    if max_width == 0 {
        return String::new();
    }
    let width = display_width(s);
    if width <= max_width {
        return s.to_string();
    }
    if max_width <= 1 {
        return "…".to_string();
    }

    let mut result = String::new();
    let mut current_width = 0;
    for c in s.chars() {
        let char_width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if current_width + char_width >= max_width {
            break;
        }
        result.push(c);
        current_width += char_width;
    }
    result.push('…');
    result
}

/// Key actions for menu completion (maps to zsh widget bindings)
///
/// These actions match zsh's Src/Zle/complist.c behavior exactly:
/// - Navigation (Up/Down/Left/Right) moves in the menu grid with wrapping
/// - Accept exits the menu and inserts the selection
/// - AcceptAndMenuComplete accepts current, then runs a NEW completion cycle
/// - AcceptAndInferNextHistory same as above (both push state stack, call do_menucmp)
/// - ReverseMenuComplete cycles to previous match via completion (not just prev item)
/// - ToggleInteractive enters/exits MM_INTER mode for type-to-filter
/// - SearchForward/Backward enters MM_FSEARCH/MM_BSEARCH incremental search
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    /// Exit menu, accept current selection (accept-line)
    Accept,
    /// Accept current + trigger NEW completion on modified line (accept-and-menu-complete)
    /// This is NOT "advance to next item" - it runs a fresh completion cycle
    AcceptAndMenuComplete,
    /// Accept current + trigger NEW completion (accept-and-infer-next-history)
    /// Same behavior as AcceptAndMenuComplete in menuselect context
    AcceptAndInferNextHistory,
    /// Exit menu without accepting (send-break)
    Cancel,
    /// Move up in menu grid, wrap to bottom+left at top (up-history)
    Up,
    /// Move down in menu grid, wrap to top+right at bottom (down-history)
    Down,
    /// Move left in menu grid, wrap to prev row at col 0 (vi-backward-char)
    Left,
    /// Move right in menu grid, wrap to next row at end (vi-forward-char)
    Right,
    /// Page up by screenful (vi-backward-word)
    PageUp,
    /// Page down by screenful (vi-forward-word)
    PageDown,
    /// Jump to first item (beginning-of-history)
    Beginning,
    /// Jump to last item (end-of-history)
    End,
    /// Jump to beginning of current row (vi-beginning-of-line)
    BeginningOfLine,
    /// Jump to end of current row (vi-end-of-line)
    EndOfLine,
    /// Cycle to next match via completion (menu-complete)
    Next,
    /// Cycle to previous match via completion with zmult=-1 (reverse-menu-complete)
    /// This calls do_menucmp(0) with negative multiplier
    Prev,
    /// Jump to next group (vi-forward-blank-word)
    NextGroup,
    /// Jump to previous group (vi-backward-blank-word)
    PrevGroup,
    /// Toggle interactive/filter mode MM_INTER (vi-insert)
    ToggleInteractive,
    /// Enter forward incremental search MM_FSEARCH (history-incremental-search-forward)
    SearchForward,
    /// Enter backward incremental search MM_BSEARCH (history-incremental-search-backward)
    SearchBackward,
    /// Undo last accept (pops from menu stack)
    Undo,
    /// Force redisplay
    Redisplay,
    /// Generic search (alias for SearchForward)
    Search,
    /// Clear current search string
    ClearSearch,
    /// Insert character (in interactive/search mode)
    Insert(char),
    /// Delete last character (in interactive/search mode)
    Backspace,
}

/// Result of processing a menu action
#[derive(Clone, Debug)]
pub enum MenuResult {
    /// Stay in menu, continue processing
    Continue,
    /// Exit menu, insert this string
    Accept(String),
    /// Accept this string AND advance to next item in SAME menu (accept-and-menu-complete)
    /// The caller should: 1) insert the string, 2) stay in menu mode showing next item
    AcceptAndHold(String),
    /// Exit menu without inserting anything
    Cancel,
    /// Force redisplay
    Redisplay,
    /// Undo requested - caller should pop state stack and restore previous line
    UndoRequested,
    /// No action taken
    None,
}

impl MenuState {
    /// Process a menu action and return the result
    ///
    /// This matches zsh's Src/Zle/complist.c menuselect behavior:
    /// - Accept: exit menu, insert selection
    /// - AcceptAndMenuComplete/AcceptAndInferNextHistory: accept + request NEW completion
    /// - Navigation: move in grid with proper wrapping
    /// - ToggleInteractive: enter/exit filter mode
    /// - SearchForward/Backward: enter incremental search
    pub fn process_action(&mut self, action: MenuAction) -> MenuResult {
        match action {
            MenuAction::Accept => {
                // zsh: sets acc=1, breaks loop - exit menu with current selection
                if let Some(comp) = self.selected() {
                    let insert = comp.insert_str();
                    self.stop();
                    return MenuResult::Accept(insert);
                }
                MenuResult::Cancel
            }
            MenuAction::AcceptAndMenuComplete | MenuAction::AcceptAndInferNextHistory => {
                // zsh: accept_last() then do_menucmp(0) - accept current, advance to next
                // This accepts current match AND moves to next item in SAME menu
                // The menu stays open showing the next item
                if let Some(idx) = self.selected_idx {
                    if let Some(item) = self.items.get(idx) {
                        let insert = item.completion.insert_str();
                        // Advance to next item (wrap around)
                        self.selected_idx = Some((idx + 1) % self.items.len());
                        self.ensure_selection_visible();
                        return MenuResult::AcceptAndHold(insert);
                    }
                }
                MenuResult::None
            }
            MenuAction::Cancel => {
                // zsh: send-break - exit without accepting
                self.stop();
                MenuResult::Cancel
            }
            MenuAction::Up => {
                // zsh: up-history - move up in grid, wrap to bottom+left at top
                self.navigate(MenuMotion::Up);
                MenuResult::Continue
            }
            MenuAction::Down => {
                // zsh: down-history - move down in grid, wrap to top+right at bottom
                self.navigate(MenuMotion::Down);
                MenuResult::Continue
            }
            MenuAction::Left => {
                // zsh: vi-backward-char - move left, wrap to prev row at col 0
                self.navigate(MenuMotion::Left);
                MenuResult::Continue
            }
            MenuAction::Right => {
                // zsh: vi-forward-char - move right, wrap to next row at end
                self.navigate(MenuMotion::Right);
                MenuResult::Continue
            }
            MenuAction::PageUp => {
                // zsh: vi-backward-word - page up by screenful
                self.navigate(MenuMotion::PageUp);
                MenuResult::Continue
            }
            MenuAction::PageDown => {
                // zsh: vi-forward-word - page down by screenful
                self.navigate(MenuMotion::PageDown);
                MenuResult::Continue
            }
            MenuAction::Beginning => {
                // zsh: beginning-of-history - jump to first item
                self.navigate(MenuMotion::First);
                MenuResult::Continue
            }
            MenuAction::End => {
                // zsh: end-of-history - jump to last item
                self.navigate(MenuMotion::Last);
                MenuResult::Continue
            }
            MenuAction::BeginningOfLine => {
                // zsh: vi-beginning-of-line - jump to start of current row
                self.navigate_to_row_start();
                MenuResult::Continue
            }
            MenuAction::EndOfLine => {
                // zsh: vi-end-of-line - jump to end of current row
                self.navigate_to_row_end();
                MenuResult::Continue
            }
            MenuAction::Next => {
                // zsh: menu-complete - this actually cycles via completion
                // For simplicity we just advance in menu; true zsh calls do_menucmp
                self.navigate(MenuMotion::Next);
                MenuResult::Continue
            }
            MenuAction::Prev => {
                // zsh: reverse-menu-complete - cycles backward via do_menucmp with zmult=-1
                // For simplicity we just go back in menu
                self.navigate(MenuMotion::Prev);
                MenuResult::Continue
            }
            MenuAction::NextGroup => {
                // zsh: vi-forward-blank-word - jump to next completion group
                self.navigate_to_next_group();
                MenuResult::Continue
            }
            MenuAction::PrevGroup => {
                // zsh: vi-backward-blank-word - jump to previous completion group
                self.navigate_to_prev_group();
                MenuResult::Continue
            }
            MenuAction::ToggleInteractive => {
                // zsh: vi-insert - toggle MM_INTER mode (type-to-filter)
                self.interactive_mode = !self.interactive_mode;
                if self.interactive_mode {
                    self.search_active = true;
                } else {
                    self.search_clear();
                }
                MenuResult::Continue
            }
            MenuAction::SearchForward | MenuAction::SearchBackward | MenuAction::Search => {
                // zsh: history-incremental-search-forward/backward - enter search mode
                self.search_active = true;
                self.search_direction = if matches!(action, MenuAction::SearchBackward) {
                    SearchDirection::Backward
                } else {
                    SearchDirection::Forward
                };
                MenuResult::Continue
            }
            MenuAction::ClearSearch => {
                self.search_clear();
                MenuResult::Continue
            }
            MenuAction::Undo => {
                // zsh: undo - pops from Menustack, restores previous line/state
                MenuResult::UndoRequested
            }
            MenuAction::Redisplay => MenuResult::Redisplay,
            MenuAction::Insert(c) => {
                if self.search_active || self.interactive_mode {
                    self.search_input(c);
                }
                MenuResult::Continue
            }
            MenuAction::Backspace => {
                if self.search_active || self.interactive_mode {
                    self.search_backspace();
                }
                MenuResult::Continue
            }
        }
    }

    /// Get the string to insert for the currently selected item
    pub fn current_insert_string(&self) -> Option<String> {
        self.selected().map(|c| c.insert_str())
    }

    /// Get info for status line: (selected_num, total, groups)
    pub fn status_info(&self) -> (usize, usize, usize) {
        (
            self.selected_idx.map(|i| i + 1).unwrap_or(0),
            self.items.len(),
            self.groups.len(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_menu_navigation() {
        let mut menu = MenuState::new();
        menu.set_term_size(80, 24);
        menu.set_available_rows(10);

        let mut group = CompletionGroup::new("test");
        // Long descriptions force single-column layout, so Down = next item
        for i in 0..20 {
            let mut comp = Completion::new(format!("item{:02}", i));
            comp.desc = Some(format!(
                "A long description for item {} that takes up most of the line width",
                i
            ));
            group.matches.push(comp);
        }

        menu.set_completions(&[group]);

        assert_eq!(menu.count(), 20);
        assert!(!menu.is_active());

        menu.start();
        assert!(menu.is_active());
        assert_eq!(menu.selected_index(), Some(0));

        // In single-column, Down moves to next row = next item
        menu.navigate(MenuMotion::Down);
        assert_eq!(menu.selected_index(), Some(1));

        for _ in 0..5 {
            menu.navigate(MenuMotion::Next);
        }
        assert_eq!(menu.selected_index(), Some(6));

        menu.navigate(MenuMotion::Deselect);
        assert!(!menu.is_active());
    }

    #[test]
    fn test_multi_column_navigation() {
        // Test navigation in multi-column layout (no descriptions = multi-col)
        let mut menu = MenuState::new();
        menu.set_term_size(80, 24);
        menu.set_available_rows(10);

        let mut group = CompletionGroup::new("test");
        // 20 items with longer names to force multiple rows
        for i in 0..20 {
            let comp = Completion::new(format!("longer_item_{:02}", i));
            group.matches.push(comp);
        }

        menu.set_completions(&[group]);
        menu.start();

        // Debug: print layout info
        eprintln!("Groups: {:?}", menu.groups);
        eprintln!("Initial selected_idx: {:?}", menu.selected_index());

        let cols = menu.groups[0].cols;
        let rows = menu.groups[0].row_count;
        eprintln!("Layout: {} cols x {} rows", cols, rows);

        // With 80-width and ~15 char items, we should get ~4-5 columns
        // So 20 items / 5 cols = 4 rows
        // Row 0: items 0-4
        // Row 1: items 5-9
        // etc.

        let initial_idx = menu.selected_index().unwrap();
        let (initial_row, initial_col) = menu.idx_to_visual_row_col(initial_idx);
        eprintln!(
            "Initial: idx={}, row={}, col={}",
            initial_idx, initial_row, initial_col
        );

        // Only test if we actually have multiple rows
        if rows > 1 {
            menu.navigate(MenuMotion::Down);

            let after_idx = menu.selected_index().unwrap();
            let (after_row, after_col) = menu.idx_to_visual_row_col(after_idx);
            eprintln!(
                "After Down: idx={}, row={}, col={}",
                after_idx, after_row, after_col
            );

            // Down should move exactly one row
            assert_eq!(after_row, initial_row + 1, "Down should move exactly 1 row");
            assert_eq!(after_col, initial_col, "Down should keep same column");

            // Expected: if we have 5 cols, item 0 -> item 5
            let expected_idx = cols;
            assert_eq!(after_idx, expected_idx, "Should move by number of columns");
        }
    }

    #[test]
    fn test_multi_group_navigation() {
        // Test navigation across multiple groups
        let mut menu = MenuState::new();
        menu.set_term_size(80, 24);
        menu.set_available_rows(10);

        // Group 1: files (short names)
        let mut group1 = CompletionGroup::new("files");
        for i in 0..15 {
            let comp = Completion::new(format!("file_{:02}.txt", i));
            group1.matches.push(comp);
        }

        // Group 2: directories (short names)
        let mut group2 = CompletionGroup::new("directories");
        for i in 0..8 {
            let comp = Completion::new(format!("dir_{}/", i));
            group2.matches.push(comp);
        }

        menu.set_completions(&[group1, group2]);
        menu.start();

        eprintln!("Groups: {:?}", menu.groups);

        // Navigate through first group
        let initial_idx = menu.selected_index().unwrap();
        let (r0, c0) = menu.idx_to_visual_row_col(initial_idx);
        eprintln!("Start: idx={}, row={}, col={}", initial_idx, r0, c0);

        menu.navigate(MenuMotion::Down);
        let (r1, c1) = menu.idx_to_visual_row_col(menu.selected_index().unwrap());
        eprintln!(
            "After Down: idx={}, row={}, col={}",
            menu.selected_index().unwrap(),
            r1,
            c1
        );
        assert_eq!(r1, r0 + 1, "Down should move 1 row");

        // Move to first group's last row, then down into second group
        let rows_g1 = menu.groups[0].row_count;
        eprintln!("Group 1 rows: {}", rows_g1);

        // Keep going down to reach group boundary
        for _ in 0..(rows_g1 - r1) {
            menu.navigate(MenuMotion::Down);
        }
        let (row_after, _col_after) = menu.idx_to_visual_row_col(menu.selected_index().unwrap());
        eprintln!(
            "After reaching group boundary: idx={}, row={}",
            menu.selected_index().unwrap(),
            row_after
        );
    }

    #[test]
    fn test_varied_column_groups() {
        // Reproduce user's 'a<TAB>' scenario with varied column counts
        let mut menu = MenuState::new();
        menu.set_term_size(200, 24); // Wide terminal
        menu.set_available_rows(20);

        // Group 1: external commands - 20 items, ~4 columns = 5 rows
        let mut group1 = CompletionGroup::new("external command");
        for i in 0..20 {
            let comp = Completion::new(format!("ext_cmd_{:02}_longish_name", i));
            group1.matches.push(comp);
        }

        // Group 2: alias - 1 item with description = 1 row, 1 column
        let mut group2 = CompletionGroup::new("alias");
        let mut alias = Completion::new("ai");
        alias.desc = Some("forDirZipRar.zsh && mountInstall.zsh".to_string());
        group2.matches.push(alias);

        // Group 3: shell functions - 9 short items = 1 row, ~9 columns
        let mut group3 = CompletionGroup::new("shell function");
        for name in [
            "a",
            "add-zle-hook",
            "add-zsh-hook",
            "after",
            "age",
            "allopt",
            "apz",
            "asg",
            "aws_comp",
        ] {
            let comp = Completion::new(name);
            group3.matches.push(comp);
        }

        // Group 4: builtins - 2 items = 1 row, 2 columns
        let mut group4 = CompletionGroup::new("builtin command");
        group4.matches.push(Completion::new("alias"));
        group4.matches.push(Completion::new("autoload"));

        menu.set_completions(&[group1, group2, group3, group4]);
        menu.start();

        eprintln!("\nGroups layout:");
        for (i, g) in menu.groups.iter().enumerate() {
            eprintln!(
                "  Group {}: '{}' - {} items, {} cols, {} rows, start_row={}",
                i, g.name, g.count, g.cols, g.row_count, g.start_row
            );
        }

        let total_rows: usize = menu.groups.iter().map(|g| g.row_count).sum();

        // Test: Starting at first item, navigate down multiple times
        // Each Down should move exactly 1 visual row
        eprintln!("\n=== Testing Down navigation ===");
        let mut prev_row = 0;
        for step in 0..10 {
            let idx = menu.selected_index().unwrap();
            let (row, col) = menu.idx_to_visual_row_col(idx);
            eprintln!("Step {}: idx={}, row={}, col={}", step, idx, row, col);

            if step > 0 {
                // Should have moved exactly 1 row (or wrapped from last to first)
                let expected = (prev_row + 1) % total_rows;
                assert_eq!(
                    row, expected,
                    "Down should move exactly 1 row (step {})",
                    step
                );
            }
            prev_row = row;

            menu.navigate(MenuMotion::Down);
        }

        // Test Up navigation too
        eprintln!("\n=== Testing Up navigation ===");
        // Reset to start
        menu.navigate(MenuMotion::First);
        prev_row = 0;
        for step in 0..10 {
            let idx = menu.selected_index().unwrap();
            let (row, col) = menu.idx_to_visual_row_col(idx);
            eprintln!("Step {}: idx={}, row={}, col={}", step, idx, row, col);

            if step > 0 {
                // Up should move exactly 1 row back (or wrap from 0 to last)
                let expected = if prev_row == 0 {
                    total_rows - 1
                } else {
                    prev_row - 1
                };
                assert_eq!(
                    row, expected,
                    "Up should move exactly 1 row back (step {})",
                    step
                );
            }
            prev_row = row;

            menu.navigate(MenuMotion::Up);
        }
    }

    #[test]
    fn test_truncate_ellipsis() {
        assert_eq!(truncate_with_ellipsis("hello", 10), "hello");
        assert_eq!(truncate_with_ellipsis("hello world", 8), "hello w…");
        assert_eq!(truncate_with_ellipsis("hi", 1), "…");
        assert_eq!(truncate_with_ellipsis("", 5), "");
    }

    #[test]
    fn test_unicode_width() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width("日本語"), 6); // 3 chars * 2 width each
        assert_eq!(display_width("café"), 4);
    }

    #[test]
    fn test_render() {
        let mut menu = MenuState::new();
        menu.set_term_size(80, 24);
        menu.set_available_rows(10);
        menu.set_show_headers(false);

        let mut group = CompletionGroup::new("test");
        for i in 0..5 {
            let mut comp = Completion::new(format!("item{}", i));
            comp.desc = Some(format!("desc{}", i));
            group.matches.push(comp);
        }

        menu.set_completions(&[group]);
        menu.start();

        let rendering = menu.render();
        assert_eq!(rendering.total_matches, 5);
        assert!(rendering.selected_idx.is_some());
        assert!(!rendering.lines.is_empty());
    }

    #[test]
    fn test_search_filter() {
        let mut menu = MenuState::new();
        menu.set_term_size(80, 24);
        menu.set_available_rows(10);

        let mut group = CompletionGroup::new("test");
        group.matches.push(Completion::new("apple"));
        group.matches.push(Completion::new("banana"));
        group.matches.push(Completion::new("apricot"));
        group.matches.push(Completion::new("cherry"));

        menu.set_completions(&[group]);
        menu.start();
        assert_eq!(menu.count(), 4);

        // Use interactive mode for filtering (toggles filtering behavior)
        menu.toggle_interactive();
        menu.search_input('a');
        menu.search_input('p');
        assert_eq!(menu.count(), 2); // apple, apricot

        menu.search_clear();
        assert_eq!(menu.count(), 4);
    }

    #[test]
    fn test_group_headers() {
        let mut menu = MenuState::new();
        menu.set_term_size(80, 24);
        menu.set_available_rows(10);
        menu.set_show_headers(true);

        let mut group1 = CompletionGroup::new("files");
        group1.explanation = Some("Files".to_string());
        group1.matches.push(Completion::new("file1.txt"));
        group1.matches.push(Completion::new("file2.txt"));

        let mut group2 = CompletionGroup::new("dirs");
        group2.explanation = Some("Directories".to_string());
        group2.matches.push(Completion::new("dir1/"));
        group2.matches.push(Completion::new("dir2/"));

        menu.set_completions(&[group1, group2]);
        menu.start();

        let rendering = menu.render();
        assert_eq!(rendering.total_matches, 4);

        // Should have header lines
        let header_count = rendering.lines.iter().filter(|l| l.is_header).count();
        assert_eq!(header_count, 2);
    }
}

// === KEYMAP SUPPORT ===

/// Search direction
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SearchDirection {
    #[default]
    Forward,
    Backward,
}

/// Terminal key sequence
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct KeySequence(pub Vec<u8>);

impl KeySequence {
    pub fn from_zsh_notation(s: &str) -> Self {
        let mut bytes = Vec::new();
        let mut chars = s.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '^' {
                if let Some(&next) = chars.peek() {
                    if next == '[' {
                        bytes.push(0x1b);
                        chars.next();
                    } else if next == '?' {
                        bytes.push(127);
                        chars.next();
                    } else if next == '@' {
                        bytes.push(0);
                        chars.next();
                    } else {
                        let ctrl = next.to_ascii_uppercase();
                        if ctrl >= 'A' && ctrl <= '_' {
                            bytes.push((ctrl as u8) - b'A' + 1);
                        } else {
                            bytes.push(ctrl as u8);
                        }
                        chars.next();
                    }
                }
            } else {
                bytes.push(c as u8);
            }
        }
        Self(bytes)
    }
}

/// Default menuselect keymap bindings
///
/// Maps zsh widget names to MenuAction based on Src/Zle/complist.c behavior
pub fn default_menuselect_bindings() -> Vec<(&'static str, MenuAction)> {
    vec![
        // Exit and accept
        ("accept-line", MenuAction::Accept),
        (".accept-line", MenuAction::Accept),
        ("accept-search", MenuAction::Accept),
        ("send-break", MenuAction::Cancel),
        // Accept + recompute (NOT advance to next item!)
        ("accept-and-hold", MenuAction::AcceptAndMenuComplete),
        (
            "accept-and-menu-complete",
            MenuAction::AcceptAndMenuComplete,
        ),
        (
            "accept-and-infer-next-history",
            MenuAction::AcceptAndInferNextHistory,
        ),
        // Vertical navigation
        ("down-history", MenuAction::Down),
        ("down-line-or-history", MenuAction::Down),
        ("down-line-or-search", MenuAction::Down),
        ("vi-down-line-or-history", MenuAction::Down),
        ("up-history", MenuAction::Up),
        ("up-line-or-history", MenuAction::Up),
        ("up-line-or-search", MenuAction::Up),
        ("vi-up-line-or-history", MenuAction::Up),
        // Horizontal navigation
        ("forward-char", MenuAction::Right),
        ("vi-forward-char", MenuAction::Right),
        ("backward-char", MenuAction::Left),
        ("vi-backward-char", MenuAction::Left),
        // Page navigation (by screenful)
        ("forward-word", MenuAction::PageDown),
        ("vi-forward-word", MenuAction::PageDown),
        ("vi-forward-word-end", MenuAction::PageDown),
        ("emacs-forward-word", MenuAction::PageDown),
        ("backward-word", MenuAction::PageUp),
        ("vi-backward-word", MenuAction::PageUp),
        ("emacs-backward-word", MenuAction::PageUp),
        // Group navigation
        ("vi-forward-blank-word", MenuAction::NextGroup),
        ("vi-backward-blank-word", MenuAction::PrevGroup),
        // Jump to first/last
        ("beginning-of-history", MenuAction::Beginning),
        ("beginning-of-buffer-or-history", MenuAction::Beginning),
        ("end-of-history", MenuAction::End),
        ("end-of-buffer-or-history", MenuAction::End),
        // Row navigation
        ("vi-beginning-of-line", MenuAction::BeginningOfLine),
        ("beginning-of-line", MenuAction::BeginningOfLine),
        ("beginning-of-line-hist", MenuAction::BeginningOfLine),
        ("vi-end-of-line", MenuAction::EndOfLine),
        ("end-of-line", MenuAction::EndOfLine),
        ("end-of-line-hist", MenuAction::EndOfLine),
        // Completion cycling (these call do_menucmp in zsh)
        ("complete-word", MenuAction::Next),
        ("menu-complete", MenuAction::Next),
        ("expand-or-complete", MenuAction::Next),
        ("menu-expand-or-complete", MenuAction::Next),
        ("reverse-menu-complete", MenuAction::Prev),
        // Interactive mode (MM_INTER)
        ("vi-insert", MenuAction::ToggleInteractive),
        // Incremental search (MM_FSEARCH/MM_BSEARCH)
        (
            "history-incremental-search-forward",
            MenuAction::SearchForward,
        ),
        (
            "history-incremental-search-backward",
            MenuAction::SearchBackward,
        ),
        // Undo (pops from menu stack)
        ("undo", MenuAction::Undo),
        ("backward-delete-char", MenuAction::Backspace),
        // Redisplay
        ("redisplay", MenuAction::Redisplay),
        ("clear-screen", MenuAction::Redisplay),
    ]
}

/// Menuselect keymap
#[derive(Clone, Debug, Default)]
pub struct MenuKeymap {
    key_to_widget: Vec<(KeySequence, String)>,
    widget_to_action: std::collections::HashMap<String, MenuAction>,
}

impl MenuKeymap {
    pub fn new() -> Self {
        let mut km = Self::default();
        for (w, a) in default_menuselect_bindings() {
            km.widget_to_action.insert(w.to_string(), a);
        }
        km.bind("^@", "accept-line");
        km.bind("^D", "accept-and-menu-complete");
        km.bind("^F", "accept-and-infer-next-history");
        km.bind("^H", "vi-backward-char");
        km.bind("^I", "vi-forward-char");
        km.bind("^J", "down-history");
        km.bind("^K", "up-history");
        km.bind("^L", "vi-forward-char");
        km.bind("^M", "accept-line");
        km.bind("^N", "vi-forward-word");
        km.bind("^P", "vi-backward-word");
        km.bind("^S", "reverse-menu-complete");
        km.bind("^V", "vi-insert");
        km.bind("^X", "history-incremental-search-forward");
        km.bind("^[OA", "up-line-or-history");
        km.bind("^[OB", "down-line-or-history");
        km.bind("^[OC", "forward-char");
        km.bind("^[OD", "backward-char");
        km.bind("^[[A", "up-line-or-history");
        km.bind("^[[B", "down-line-or-history");
        km.bind("^[[C", "forward-char");
        km.bind("^[[D", "backward-char");
        km.bind("^[[1~", "vi-beginning-of-line");
        km.bind("^[[4~", "vi-end-of-line");
        km.bind("^[[5~", "vi-backward-word");
        km.bind("^[[6~", "vi-forward-word");
        km.bind("^[[Z", "reverse-menu-complete");
        km.bind("^?", "undo");
        km.bind("?", "history-incremental-search-backward");
        km.bind("/", "history-incremental-search-forward");
        km.bind("^[", "send-break");
        km.bind("^G", "send-break");
        km.bind("{", "vi-backward-blank-word");
        km.bind("}", "vi-forward-blank-word");
        km
    }

    pub fn bind(&mut self, key: &str, widget: &str) {
        let seq = KeySequence::from_zsh_notation(key);
        self.key_to_widget.retain(|(k, _)| k != &seq);
        self.key_to_widget.push((seq, widget.to_string()));
    }

    pub fn lookup(&self, input: &[u8]) -> Option<(MenuAction, usize)> {
        let mut best: Option<(&str, usize)> = None;
        for (seq, widget) in &self.key_to_widget {
            if input.starts_with(&seq.0) && best.map(|(_, l)| seq.0.len() > l).unwrap_or(true) {
                best = Some((widget.as_str(), seq.0.len()));
            }
        }
        if let Some((w, len)) = best {
            if let Some(&a) = self.widget_to_action.get(w) {
                return Some((a, len));
            }
        }
        if !input.is_empty() && input[0] >= 0x20 && input[0] < 0x7f {
            return Some((MenuAction::Insert(input[0] as char), 1));
        }
        None
    }

    pub fn get_widget(&self, key: &str) -> Option<&str> {
        let seq = KeySequence::from_zsh_notation(key);
        self.key_to_widget
            .iter()
            .find(|(k, _)| k == &seq)
            .map(|(_, w)| w.as_str())
    }
}

pub fn parse_bindkey_output(output: &str, keymap: &mut MenuKeymap) {
    for line in output.lines() {
        let line = line
            .trim()
            .strip_prefix("bindkey")
            .map(|s| s.trim_start())
            .and_then(|s| s.strip_prefix("-M"))
            .map(|s| s.trim_start())
            .and_then(|s| s.strip_prefix("menuselect"))
            .map(|s| s.trim_start())
            .unwrap_or(line.trim());
        if let Some(rest) = line.strip_prefix('"') {
            if let Some(end) = rest.find('"') {
                let (key, widget) = (&rest[..end], rest[end + 1..].trim());
                if !widget.is_empty() {
                    keymap.bind(key, widget);
                }
            }
        }
    }
}

impl MenuState {
    /// Check if interactive filter mode is active (MM_INTER)
    pub fn is_interactive(&self) -> bool {
        self.interactive_mode
    }

    /// Check if incremental search is active (MM_FSEARCH or MM_BSEARCH)
    pub fn is_search_active(&self) -> bool {
        self.search_active
    }

    /// Get current search/filter string
    pub fn search_string(&self) -> &str {
        &self.search
    }

    /// Get search direction
    pub fn search_direction(&self) -> SearchDirection {
        self.search_direction
    }
}
