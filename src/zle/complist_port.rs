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
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse LS_COLORS format: "*.rs=1;32:*.c=0;33:di=1;34:..."
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

    /// Get ANSI color code for a file pattern
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

/// Calculate optimal column layout for matches (from complist.c calclist)
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
    let rows = (matches.len() + columns - 1) / columns;

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

/// Format completion list for display (from complist.c compprintlist)
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

/// Ask if user wants to scroll (from complist.c asklistscroll)
pub fn asklistscroll(total: usize, shown: usize) -> String {
    let remaining = total - shown;
    format!("--More--({}/{})", shown, total)
}

/// Format completion group header (from complist.c compprintfmt)
pub fn compprintfmt(format: &str, matches_count: usize, group: &str) -> String {
    format
        .replace("%d", &matches_count.to_string())
        .replace("%g", group)
        .replace("%%", "%")
}

/// Clear to end of line (from complist.c cleareol)
pub fn cleareol() -> &'static str {
    "\x1b[K"
}

/// Print with color for completion (from complist.c zcputs)
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
        assert_eq!(layout.columns * layout.rows >= matches.len(), true);
    }

    #[test]
    fn test_compprintfmt() {
        assert_eq!(
            compprintfmt("Showing %d matches in %g", 42, "files"),
            "Showing 42 matches in files"
        );
    }
}
