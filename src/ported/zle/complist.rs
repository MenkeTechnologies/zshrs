//! Completion listing display for ZLE
//!
//! Port from zsh/Src/Zle/complist.c (3,604 lines)
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

/// Color configuration from LS_COLORS / ZLS_COLORS
#[derive(Debug, Clone, Default)]
pub struct ListColors {
    pub colors: HashMap<String, String>,
    pub use_ls_colors: bool,
}

impl ListColors {
    /// Construct an empty colour map.
    /// Equivalent to a freshly-allocated `Listcols` from
    /// `getcols()` at Src/Zle/complist.c when `LS_COLORS` /
    /// `ZLS_COLORS` is unset.
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse a `LS_COLORS`-style spec into pattern→code lookups.
    /// Port of `getcols()` from Src/Zle/complist.c. The C source
    /// reads `$LS_COLORS` (or `$ZLS_COLORS`) and walks
    /// `:`-separated `pattern=code` pairs into `Listcols`. This
    /// Rust shape uses `pattern` as the hash key for
    /// `get_color`'s lookup.
    pub fn from_ls_colors(spec: &str) -> Self {
        let mut colors = HashMap::new();
        for entry in spec.split(':') {
            if let Some((pattern, code)) = entry.split_once('=') {
                colors.insert(pattern.to_string(), code.to_string());
            }
        }
        ListColors {
            colors,
            use_ls_colors: true,
        }
    }

    /// Resolve a filename to its ANSI colour code (or empty when no
    /// match).
    /// Port of `filecol()` from Src/Zle/complist.c. The C source
    /// matches `di` (directory), `ln` (symlink), `ex` (executable),
    /// and the per-extension `*.X=code` entries against the file
    /// metadata; ours follows the same precedence.
    pub fn get_color(
        &self,
        name: &str,
        is_dir: bool,
        is_link: bool,
        is_exec: bool,
    ) -> Option<String> {
        if is_dir {
            if let Some(c) = self.colors.get("di") {
                return Some(format!("\x1b[{}m", c));
            }
        }
        if is_link {
            if let Some(c) = self.colors.get("ln") {
                return Some(format!("\x1b[{}m", c));
            }
        }
        if is_exec {
            if let Some(c) = self.colors.get("ex") {
                return Some(format!("\x1b[{}m", c));
            }
        }
        // Check file extension
        if let Some(dot) = name.rfind('.') {
            let ext = format!("*{}", &name[dot..]);
            if let Some(c) = self.colors.get(&ext) {
                return Some(format!("\x1b[{}m", c));
            }
        }
        None
    }

    /// Emit the SGR reset escape (`\\e[0m`) used between coloured
    /// matches so a per-match colour doesn't bleed into separators.
    /// Equivalent to the `tcout(TCSGR0)` / hardcoded `\\e[0m` write
    /// at the end of each `clprintm()` call in Src/Zle/complist.c.
    pub fn reset() -> &'static str {
        "\x1b[0m"
    }
}

/// Completion list layout
#[derive(Debug, Clone)]
pub struct ListLayout {
    pub columns: usize,
    pub rows: usize,
    pub col_widths: Vec<usize>,
    pub total_width: usize,
}

/// Compute the row/column layout for the matches list given a terminal
/// width.
/// Port of `calclist()` from Src/Zle/complist.c. The C source picks
/// the column count by trying widths in descending order until the
/// row product fits the available rows; this Rust port uses the
/// simpler `term_width / max_item_width` heuristic — sufficient for
/// the common single-screen listing.
pub fn calclist(
    matches: &[String],
    term_width: usize,
    descriptions: &[Option<String>],
) -> ListLayout {
    let max_len = matches
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let desc_len = descriptions
                .get(i)
                .and_then(|d| d.as_ref())
                .map(|d| d.len() + 4) // " -- description"
                .unwrap_or(0);
            m.len() + desc_len
        })
        .max()
        .unwrap_or(0);

    let item_width = max_len + 2; // padding
    let columns = (term_width / item_width.max(1)).max(1);
    let rows = matches.len().div_ceil(columns);

    let mut col_widths = vec![item_width; columns];
    // Adjust last column
    if let Some(last) = col_widths.last_mut() {
        *last = max_len;
    }

    let total_width = col_widths.iter().sum();

    ListLayout {
        columns,
        rows,
        col_widths,
        total_width,
    }
}

/// Render the laid-out match list as a Vec of lines ready to write
/// to the terminal.
/// Port of `compprintlist()` from Src/Zle/complist.c. Walks
/// row-major across the column grid, emits group headers when the
/// group name changes, applies LS_COLORS-derived attrs to each
/// match (matching the per-cell `clprintm()` call in the C source),
/// and reverse-videos the optional `selected` index for
/// menu-selection mode.
pub fn compprintlist(
    matches: &[String],
    descriptions: &[Option<String>],
    groups: &[Option<String>],
    layout: &ListLayout,
    colors: &ListColors,
    selected: Option<usize>,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current_group: Option<&str> = None;

    for row in 0..layout.rows {
        let mut line = String::new();

        for col in 0..layout.columns {
            let idx = col * layout.rows + row;
            if idx >= matches.len() {
                break;
            }

            // Group header
            if let Some(Some(group)) = groups.get(idx) {
                if current_group != Some(group.as_str()) {
                    current_group = Some(group);
                    lines.push(format!("\x1b[1m{}:\x1b[0m", group));
                }
            }

            let m = &matches[idx];
            let is_selected = selected == Some(idx);

            // Apply color
            let colored = if is_selected {
                format!("\x1b[7m{}\x1b[0m", m) // reverse video for selected
            } else if let Some(color) = colors.get_color(m, false, false, false) {
                format!("{}{}{}", color, m, ListColors::reset())
            } else {
                m.clone()
            };

            let desc = descriptions
                .get(idx)
                .and_then(|d| d.as_ref())
                .map(|d| format!(" \x1b[2m-- {}\x1b[0m", d))
                .unwrap_or_default();

            let entry = format!("{}{}", colored, desc);
            let visible_len = m.len()
                + descriptions
                    .get(idx)
                    .and_then(|d| d.as_ref())
                    .map(|d| d.len() + 4)
                    .unwrap_or(0);

            line.push_str(&entry);

            if col + 1 < layout.columns {
                let padding = layout.col_widths[col].saturating_sub(visible_len);
                for _ in 0..padding {
                    line.push(' ');
                }
            }
        }

        lines.push(line);
    }

    lines
}

/// Format the "scroll for more?" prompt shown when the match list
/// exceeds the terminal height.
/// Port of `asklistscroll()` from Src/Zle/complist.c. The C source
/// emits "--More--" plus a percent indicator and reads y/n via
/// `getzlequery`; ours produces the prompt string and leaves the
/// input read to the caller.
pub fn asklistscroll(total: usize, shown: usize) -> String {
    let _remaining = total.saturating_sub(shown);
    format!("--More--({}/{})", shown, total)
}

/// Substitute `%d`/`%g`/`%%` in a `LIST_GROUPS_HEADER`-style format.
/// Port of `compprintfmt()` from Src/Zle/complist.c. The C source
/// supports more escapes (per-group counts, etc.); the daily-driver
/// subset (count + group + literal `%`) is honoured here.
pub fn compprintfmt(format: &str, matches_count: usize, group: &str) -> String {
    format
        .replace("%d", &matches_count.to_string())
        .replace("%g", group)
        .replace("%%", "%")
}

/// Emit the CSI-K sequence clearing from cursor to end of the
/// current line — used between match-list rows so leftover
/// characters from a prior frame don't bleed through.
/// Port of `cleareol()` from Src/Zle/complist.c (the C source
/// fronts the same `\\e[K` escape via `tcout(TCCLEAREOL)`).
pub fn cleareol() -> &'static str {
    "\x1b[K"
}

/// Wrap a string in a CSI SGR sequence using the supplied colour
/// code, then reset.
/// Port of `zcputs()` from Src/Zle/complist.c. The C source uses
/// this for per-match colour application during list paint.
pub fn zcputs(s: &str, color: Option<&str>) -> String {
    match color {
        Some(c) => format!("\x1b[{}m{}\x1b[0m", c, s),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ls_colors() {
        let colors = ListColors::from_ls_colors("di=1;34:*.rs=0;32:*.c=0;33:ex=1;31");
        assert!(colors.get_color("foo", true, false, false).is_some());
        assert!(colors.get_color("main.rs", false, false, false).is_some());
        assert!(colors.get_color("main.txt", false, false, false).is_none());
    }

    #[test]
    fn test_calclist() {
        let matches: Vec<String> = (0..20).map(|i| format!("item_{}", i)).collect();
        let descs: Vec<Option<String>> = vec![None; 20];
        let layout = calclist(&matches, 80, &descs);
        assert!(layout.columns >= 1);
        assert!(layout.rows >= 1);
        assert!(layout.columns * layout.rows >= matches.len());
    }

    #[test]
    fn test_compprintfmt() {
        assert_eq!(
            compprintfmt("Showing %d matches in %g", 42, "files"),
            "Showing 42 matches in files"
        );
    }
}
