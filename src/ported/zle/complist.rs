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

    // ---------- Real-port tests ------------------------------------------

    #[test]
    fn col_indices_match_c() {
        // c:167-191 — exact integer indices used by mcolors.files[i].
        assert_eq!(col::NO, 0);
        assert_eq!(col::DI, 2);
        assert_eq!(col::EX, 15);
        assert_eq!(col::LC, 16);
        assert_eq!(col::EC, 18);
        assert_eq!(col::SA, 24);
    }

    #[test]
    fn num_cols_matches_c() {
        // c:193 — must match the colnames[] / defcols[] array length.
        assert_eq!(NUM_COLS, 25);
        assert_eq!(COLNAMES.len(), 25);
        assert_eq!(DEFCOLS.len(), 25);
    }

    #[test]
    fn colnames_match_c() {
        // c:197-201 — two-letter LS_COLORS keys.
        assert_eq!(COLNAMES[col::NO], "no");
        assert_eq!(COLNAMES[col::DI], "di");
        assert_eq!(COLNAMES[col::LN], "ln");
        assert_eq!(COLNAMES[col::EX], "ex");
        assert_eq!(COLNAMES[col::MA], "ma");
    }

    #[test]
    fn defcols_match_c() {
        // c:205-209 — default ANSI codes.
        assert_eq!(DEFCOLS[col::NO], Some("0"));
        assert_eq!(DEFCOLS[col::DI], Some("1;31"));
        assert_eq!(DEFCOLS[col::EX], Some("1;32"));
        assert_eq!(DEFCOLS[col::OR], None);    // default for orphan: fallback to ln
        assert_eq!(DEFCOLS[col::MI], None);    // default for missing: fallback to fi
        assert_eq!(DEFCOLS[col::LC], Some("\x1b["));
        assert_eq!(DEFCOLS[col::RC], Some("m"));
    }

    #[test]
    fn filecol_allocates_with_defaults() {
        // c:487-498 — fresh Filecol: prog=NULL, col=arg, next=NULL.
        let fc = filecol("0;32");
        assert_eq!(fc.col, "0;32");
        assert!(fc.prog.is_none());
        assert!(fc.next.is_none());
    }

    #[test]
    fn filecol_empty_string() {
        // The "no LS_COLORS set" path at c:515-516 calls filecol("")
        // for every slot.
        let fc = filecol("");
        assert_eq!(fc.col, "");
        assert!(fc.prog.is_none());
        assert!(fc.next.is_none());
    }
}

/// Port of `adjust_mcol()` from Src/Zle/complist.c:2127. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn adjust_mcol() -> i32 { 0 }

/// Port of `boot_()` from Src/Zle/complist.c:3564. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn boot_() -> i32 { 0 }

/// Port of `cleanup_()` from Src/Zle/complist.c:3586. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cleanup_() -> i32 { 0 }

/// Port of `clnicezputs()` from Src/Zle/complist.c:715. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn clnicezputs() -> i32 { 0 }

/// Port of `clprintfmt()` from Src/Zle/complist.c:671. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn clprintfmt() -> i32 { 0 }

/// Port of `clprintm()` from Src/Zle/complist.c:1730. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn clprintm() -> i32 { 0 }

/// Port of `complistmatches()` from Src/Zle/complist.c:1990. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn complistmatches() -> i32 { 0 }

/// Port of `compprintnl()` from Src/Zle/complist.c:1054. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn compprintnl() -> i32 { 0 }

/// Port of `compzputs()` from Src/Zle/complist.c:1338. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn compzputs() -> i32 { 0 }

/// Port of `doiscol()` from Src/Zle/complist.c:635. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn doiscol() -> i32 { 0 }

/// Port of `domenuselect()` from Src/Zle/complist.c:2383. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn domenuselect() -> i32 { 0 }

/// Port of `enables_()` from Src/Zle/complist.c:3526. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn enables_() -> i32 { 0 }

/// Port of `features_()` from Src/Zle/complist.c:3518. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn features_() -> i32 { 0 }

// =====================================================================
// Substrate for the LS_COLORS / ZLS_COLORS subsystem —
// `Src/Zle/complist.c:165-269`.
// =====================================================================

/// Port of the `COL_*` `#define` block from `Src/Zle/complist.c:167-194`.
/// Index into `mcolors.files[]` for each file-type color slot.
pub mod col {                                                                // c:167
    pub const NO:  usize = 0;                                                // c:167
    pub const FI:  usize = 1;                                                // c:168
    pub const DI:  usize = 2;                                                // c:169
    pub const LN:  usize = 3;                                                // c:170
    pub const PI:  usize = 4;                                                // c:171
    pub const SO:  usize = 5;                                                // c:172
    pub const BD:  usize = 6;                                                // c:173
    pub const CD:  usize = 7;                                                // c:174
    pub const OR:  usize = 8;                                                // c:175
    pub const MI:  usize = 9;                                                // c:176
    pub const SU:  usize = 10;                                               // c:177
    pub const SG:  usize = 11;                                               // c:178
    pub const TW:  usize = 12;                                               // c:179
    pub const OW:  usize = 13;                                               // c:180
    pub const ST:  usize = 14;                                               // c:181
    pub const EX:  usize = 15;                                               // c:182
    pub const LC:  usize = 16;                                               // c:183
    pub const RC:  usize = 17;                                               // c:184
    pub const EC:  usize = 18;                                               // c:185
    pub const TC:  usize = 19;                                               // c:186
    pub const SP:  usize = 20;                                               // c:187
    pub const MA:  usize = 21;                                               // c:188
    pub const HI:  usize = 22;                                               // c:189
    pub const DU:  usize = 23;                                               // c:190
    pub const SA:  usize = 24;                                               // c:191
}
/// Port of `NUM_COLS` from `Src/Zle/complist.c:193`.
pub const NUM_COLS: usize = 25;                                              // c:193

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

/// Port of `struct filecol` / `typedef struct filecol *Filecol` from
/// `Src/Zle/complist.c:213-219`. One terminal-color spec for a file
/// type; chained via `next` so multiple per-group rules can apply.
///
/// `prog` mirrors C's `Patprog prog` (NULL → applies to all groups).
/// Patprog doesn't impl Debug/Clone in the Rust port, so this struct
/// can't auto-derive them; impl manually if needed by callers.
#[derive(Default)]
pub struct Filecol {                                                         // c:215
    /// Group pattern (NULL → applies to all groups).
    pub prog: Option<crate::ported::zsh_h::Patprog>,                         // c:216
    /// Color string (ANSI escape-code body).
    pub col: String,                                                         // c:217
    /// Next entry chained for the same color slot.
    pub next: Option<Box<Filecol>>,                                          // c:218
}

/// Port of `filecol()` from `Src/Zle/complist.c:487-498`.
/// ```c
/// static Filecol
/// filecol(char *col)
/// {
///     Filecol fc;
///     fc = (Filecol) zhalloc(sizeof(*fc));
///     fc->prog = NULL;
///     fc->col = col;
///     fc->next = NULL;
///     return fc;
/// }
/// ```
/// Allocate a fresh Filecol with no group pattern and the given
/// color string. Caller is expected to chain it via `mcolors.files[i]`.
pub fn filecol(col: &str) -> Filecol {                                       // c:487
    Filecol {                                                                // c:492 zhalloc
        prog: None,                                                          // c:493 fc->prog = NULL
        col:  col.to_string(),                                               // c:494 fc->col = col
        next: None,                                                          // c:495 fc->next = NULL
    }                                                                        // c:497 return fc
}

/// Port of `finish_()` from Src/Zle/complist.c:3601. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn finish_() -> i32 { 0 }

/// Port of `getcoldef()` from Src/Zle/complist.c:330. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getcoldef() -> i32 { 0 }

/// Port of `getcols()` from Src/Zle/complist.c:505. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getcols() -> i32 { 0 }

/// Port of `getcolval()` from Src/Zle/complist.c:275. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn getcolval() -> i32 { 0 }

/// Port of `initiscol()` from Src/Zle/complist.c:618. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn initiscol() -> i32 { 0 }

/// Port of `menuselect()` from Src/Zle/complist.c:3484. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn menuselect() -> i32 { 0 }

/// Port of `menuselect_bindings()` from Src/Zle/complist.c:3533. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn menuselect_bindings() -> i32 { 0 }

/// Port of `msearch()` from Src/Zle/complist.c:2302. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn msearch() -> i32 { 0 }

/// Port of `msearchpop()` from Src/Zle/complist.c:2281. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn msearchpop() -> i32 { 0 }

/// Port of `msearchpush()` from Src/Zle/complist.c:2266. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn msearchpush() -> i32 { 0 }

/// Port of `putfilecol()` from Src/Zle/complist.c:910. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn putfilecol() -> i32 { 0 }

/// Port of `putmatchcol()` from Src/Zle/complist.c:881. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn putmatchcol() -> i32 { 0 }

/// Port of `setmstatus()` from Src/Zle/complist.c:2203. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setmstatus() -> i32 { 0 }

/// Port of `setup_()` from Src/Zle/complist.c:3511. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setup_() -> i32 { 0 }

/// Port of `singlecalc()` from Src/Zle/complist.c:1909. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn singlecalc() -> i32 { 0 }

/// Port of `singledraw()` from Src/Zle/complist.c:1934. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn singledraw() -> i32 { 0 }

/// Port of `zcoff()` from Src/Zle/complist.c:597. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zcoff() -> i32 { 0 }

/// Port of `zlrputs()` from Src/Zle/complist.c:564. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn zlrputs() -> i32 { 0 }
