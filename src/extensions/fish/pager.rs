//! fish's pager (fish 4.9 `pager.rs`): the list of completions TAB shows
//! under the line in `zshrs --fish`, laid out and worded as fish does it.
//!
//! zsh draws the list. Each cell goes in as the display string (`compadd -d`)
//! of the match it stands for, already padded to its column, and `calclist`
//! takes the number of rows from [`forced_rows`] instead of choosing its own.
//! fish decides rows from at most six columns; zsh fills as many as fit.

use unicode_width::UnicodeWidthChar;

/// fish `PAGER_SPACER_STRING`: the space between adjacent columns. zsh adds it
/// itself (`CM_SPACE`), so it is only counted here.
const SPACER: usize = 2;
/// fish `PAGER_UNDISCLOSED_MAX_ROWS`: rows shown at least before disclosure.
const UNDISCLOSED_MAX_ROWS: usize = 4;
/// fish `PAGER_MIN_WIDTH` / `PAGER_MIN_HEIGHT`: below these nothing is listed.
const MIN_WIDTH: usize = 16;
const MIN_HEIGHT: usize = 4;
/// fish `PAGER_MAX_COLS`.
const MAX_COLS: usize = 6;
/// fish reader.rs `PREFIX_MAX_LEN`: a longer pager prefix is shortened.
const PREFIX_MAX_LEN: usize = 9;
const ELLIPSIS: char = '…';

/// One completion as the pager shows it: its text (pager prefix included)
/// and its description.
pub struct Item {
    pub text: String,
    pub desc: String,
}

/// What the list looks like.
#[derive(Debug, Default, PartialEq)]
pub struct Page {
    /// Per item, the cell it is shown as; empty for an item that is not
    /// listed (joined into an earlier item's cell, or in a row not disclosed).
    pub cells: Vec<String>,
    /// Rows shown.
    pub rows: usize,
    /// fish's progress line (`…and 3 more rows`), empty when there is none.
    pub progress: String,
}

/// fish `PagerComp`: one cell, the completions sharing a description.
struct Cell {
    texts: Vec<String>,
    /// the items it stands for, the first one listed
    first: usize,
    desc: String,
    comp_width: usize,
    desc_width: usize,
}

impl Cell {
    /// `completion  (description)`: two spaces and the parentheses.
    fn desc_punctuated_width(&self) -> usize {
        self.desc_width + if self.desc_width != 0 { 4 } else { 0 }
    }
    fn preferred_width(&self) -> usize {
        self.comp_width + self.desc_punctuated_width()
    }
}

#[derive(Clone, Copy, Default)]
struct Column {
    width: usize,
    desc_align: usize,
}

/// fish `wcswidth_rendered`: a non-printable character takes no room.
fn width(s: &str) -> usize {
    s.chars().filter_map(UnicodeWidthChar::width).sum()
}

/// fish `escape_string` with `no_printables | no_quoted | symbolic`, as the
/// pager shows a completion: a control character becomes its Unicode
/// control picture (`␤` for a newline), anything else stays as it is.
pub fn escape_symbolic(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\x7f' => '␡',
            '\x00'..='\x1f' => char::from_u32(0x2400 + c as u32).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// fish reader.rs `handle_completions`: the pager prefix, shortened to its
/// last path component (`…/lib/`) or its last characters when long.
pub fn pager_prefix(full: &str) -> String {
    let n = full.chars().count();
    if n <= PREFIX_MAX_LEN {
        return full.to_string();
    }
    let truncated: String = full.chars().skip(n - PREFIX_MAX_LEN).collect();
    match truncated.rsplit_once('/') {
        None => format!("{ELLIPSIS}{truncated}"),
        Some((_, last)) => format!("{ELLIPSIS}/{last}"),
    }
}

/// fish `mangle_1_completion_description`: trim, and one space for every
/// run of whitespace.
fn mangle_description(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// fish `print_max_impl`: append `s` in at most `max` columns, ending it with
/// an ellipsis when it does not fit (or when it fits exactly but `has_more`).
/// Returns the columns used.
fn print_max(out: &mut String, s: &str, max: usize, has_more: bool) -> usize {
    let mut remaining = max;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        let Some(w) = c.width() else { continue };
        if w > remaining {
            break;
        }
        if w == remaining && (has_more || chars.peek().is_some()) {
            out.push(ELLIPSIS);
            remaining = remaining.saturating_sub(1);
            break;
        }
        out.push(c);
        remaining -= w;
    }
    max - remaining
}

fn spaces(out: &mut String, n: usize, max: usize) -> usize {
    print_max(out, &" ".repeat(n), max, false)
}

/// fish `completion_print_item`: one cell in its column's width.
fn print_cell(c: &Cell, col: Column) -> String {
    let comp_width = if c.preferred_width() <= col.width {
        c.comp_width
    } else {
        // no room for both: at most 2/3 of the column for the completion,
        // more when the description is short
        let w = col.width.saturating_sub(4);
        let two_thirds = (w / 3) * 2 + ((w % 3) * 2) / 3;
        let mut cw = c.comp_width.min(two_thirds);
        let desc = c.desc_punctuated_width();
        if col.width > desc {
            cw = cw.max(col.width - desc);
        }
        cw
    };
    let mut out = String::new();
    let mut comp_remaining = comp_width;
    for (i, t) in c.texts.iter().enumerate() {
        if i > 0 {
            comp_remaining -= print_max(&mut out, "  ", comp_remaining, true);
        }
        comp_remaining -= print_max(&mut out, t, comp_remaining, i + 1 < c.texts.len());
    }
    let mut desc_remaining = col.width - comp_width + comp_remaining;
    if c.desc_width > 0 && desc_remaining > 4 {
        desc_remaining -= print_max(&mut out, "  ", desc_remaining, false);
        // descriptions line up in the column unless one is too wide
        let pad = desc_remaining.saturating_sub(col.width.saturating_sub(col.desc_align).max(c.desc_width + 2));
        desc_remaining -= spaces(&mut out, pad, desc_remaining);
        desc_remaining -= print_max(&mut out, "(", 1, false);
        desc_remaining -= print_max(&mut out, &c.desc, desc_remaining - 1, false);
        desc_remaining -= print_max(&mut out, ")", 1, false);
    }
    spaces(&mut out, desc_remaining, desc_remaining);
    out
}

fn divide_round_up(n: usize, d: usize) -> usize {
    if n == 0 { 0 } else { n.div_ceil(d) }
}

/// Where fish's pager is in a run of TABs.
#[derive(Clone, Copy, PartialEq)]
pub enum Disclosure {
    /// The first TAB: at most half the terminal, then `…and N more rows`.
    First,
    /// The TAB after a list cut short: as many rows as the terminal holds,
    /// then `rows 1 to N of M` when there are more.
    Disclosed,
    /// Selecting in the list: every row is listed, so zsh's menu selection
    /// can reach every match (it scrolls a list taller than the terminal
    /// itself, where fish scrolls its pager).
    Selecting,
}

/// fish `Pager::render` and `completion_try_print`: lay the items out in a
/// `term_width` × `term_height` area under the command line. `prefix` is
/// the pager prefix: only under a bare `-` do options sharing a description
/// share a cell (fish `set_completions`).
pub fn render(items: &[Item], prefix: &str, term_width: usize, term_height: usize, state: Disclosure) -> Page {
    let fully_disclosed = state != Disclosure::First;
    // fish `process_completions_into_infos` + `join_completions`: one cell
    // per description, the completions sharing it side by side
    let mut cells: Vec<Cell> = Vec::new();
    let mut by_desc: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (i, item) in items.iter().enumerate() {
        let desc = mangle_description(&item.desc);
        if prefix == "-" && !desc.is_empty() {
            if let Some(&at) = by_desc.get(&desc) {
                cells[at].texts.push(item.text.clone());
                continue;
            }
            by_desc.insert(desc.clone(), cells.len());
        }
        cells.push(Cell { texts: vec![item.text.clone()], first: i, desc, comp_width: 0, desc_width: 0 });
    }
    // fish `measure_completion_infos`
    for c in &mut cells {
        c.comp_width = c.texts.iter().map(|t| width(t)).sum::<usize>() + SPACER * (c.texts.len() - 1);
        c.desc_width = width(&c.desc);
    }
    let mut page = Page { cells: vec![String::new(); items.len()], ..Page::default() };
    if cells.is_empty() || term_width < MIN_WIDTH || term_height < MIN_HEIGHT {
        return page;
    }
    let n = cells.len();
    for cols in (1..=MAX_COLS).rev() {
        let rows = divide_round_up(n, cols);
        // fewer columns with the same rows is better: the next try does it
        if cols > 1 && divide_round_up(n, rows) < cols {
            continue;
        }
        // a row is kept for the progress line
        let mut height = term_height - 1;
        if !fully_disclosed {
            height = height.min((height / 2).max(UNDISCLOSED_MAX_ROWS));
        }
        let mut remaining = if !fully_disclosed && rows > height { rows - height } else { 0 };
        // one row left to disclose is shown instead of a line saying so
        if remaining == 1 {
            height += 1;
            remaining = 0;
        }
        let columns: Vec<Column> = (0..cols)
            .map(|col| {
                let mut max_comp = 0;
                let mut max_desc = 0;
                for c in cells.iter().skip(col * rows).take(rows) {
                    max_comp = max_comp.max(c.comp_width);
                    max_desc = max_desc.max(c.desc_punctuated_width());
                }
                Column { width: (max_comp + max_desc).min(term_width), desc_align: max_comp + 2 }
            })
            .collect();
        if cols > 1 && columns.iter().map(|c| c.width).sum::<usize>() + (cols - 1) * SPACER > term_width {
            continue;
        }
        let shown = if state == Disclosure::Selecting { rows } else { rows.min(height) };
        for (idx, c) in cells.iter().enumerate() {
            if idx % rows < shown {
                page.cells[c.first] = print_cell(c, columns[idx / rows]);
            }
        }
        page.rows = shown;
        if remaining > 1 {
            page.progress = format!("{ELLIPSIS}and {remaining} more rows");
        } else if shown < rows {
            page.progress = format!("rows 1 to {shown} of {rows}");
        }
        return page;
    }
    page
}

/// The rows `calclist` gives the group of fish's matches, when the list is
/// fish's: in `--fish`, for the `fish` group, from `__fish_pager_rows` (set by
/// `__fish_complete_query`, read by every redraw of the list).
pub fn forced_rows(group: Option<&str>) -> Option<usize> {
    if group != Some("fish") || !crate::extensions::emulation_startup::fish_mode() {
        return None;
    }
    crate::ported::params::getsparam("__fish_pager_rows")?.parse().ok().filter(|&r| r > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(v: &[(&str, &str)]) -> Vec<Item> {
        v.iter().map(|(t, d)| Item { text: t.to_string(), desc: d.to_string() }).collect()
    }

    /// fish lists `tig <TAB>` in an 80-column terminal as three columns,
    /// descriptions in parentheses lined up per column, cut with `…`.
    #[test]
    fn columns_descriptions_and_ellipsis_as_fish() {
        let p = render(
            &items(&[
                ("-m", "Set file mode (as in chmod)"),
                ("-p", "Make parent directories as needed"),
                ("-v", "Print a message for each created directory"),
            ]),
            "-",
            200,
            40,
            Disclosure::First,
        );
        assert_eq!(p.rows, 1);
        assert_eq!(p.cells[0], "-m  (Set file mode (as in chmod))");
        assert_eq!(p.cells[2], "-v  (Print a message for each created directory)");
        // too narrow for the three side by side: one column, each cell as wide as the widest
        let p = render(&items(&[("-m", "Set file mode"), ("-p", "Make parents")]), "-", 20, 40, Disclosure::First);
        assert_eq!(p.rows, 2);
        assert_eq!(p.cells[0], "-m  (Set file mode)");
        assert_eq!(p.cells[1], "-p  (Make parents) ");
        let p = render(&items(&[("--long-option", "a long description here")]), "--", 20, 40, Disclosure::First);
        assert_eq!(p.cells[0], "--long-op…  (a lon…)");
    }

    /// Under a bare `-`, completions with the same description share a cell
    /// and the rest of them are not listed on their own; under `--co` (fish
    /// lists `--color` and `--color=` apart) they keep their own cells.
    #[test]
    fn same_description_joins_under_a_dash() {
        let v = items(&[("-p", "parents"), ("-v", "verbose"), ("--parents", "parents")]);
        let p = render(&v, "-", 80, 40, Disclosure::First);
        assert_eq!(p.cells[0], "-p  --parents  (parents)");
        assert_eq!(p.cells[2], "");
        let p = render(&v, "--co", 80, 40, Disclosure::First);
        assert_eq!(p.cells[2], "--parents  (parents)");
    }

    /// The first TAB shows at most half the terminal (4 rows at least) and
    /// says how many rows are left; once disclosed every row is shown.
    #[test]
    fn disclosure() {
        let many: Vec<Item> = (0..60).map(|i| Item { text: format!("item{i:02}-with-a-long-name-x"), desc: String::new() }).collect();
        let p = render(&many, "item", 80, 20, Disclosure::First);
        assert_eq!(p.rows, 9);
        assert_eq!(p.progress, "…and 11 more rows");
        assert_eq!(p.cells.iter().filter(|c| !c.is_empty()).count(), 27);
        let p = render(&many, "item", 80, 20, Disclosure::Disclosed);
        assert_eq!(p.rows, 19);
        assert_eq!(p.progress, "rows 1 to 19 of 20");
        let p = render(&many, "item", 80, 20, Disclosure::Selecting);
        assert_eq!(p.rows, 20);
        assert_eq!(p.progress, "");
    }

    #[test]
    fn prefix_is_shortened() {
        assert_eq!(pager_prefix("src/ex"), "src/ex");
        assert_eq!(pager_prefix("/usr/local/lib/"), "…/");
        assert_eq!(pager_prefix("/usr/local/lib/py"), "…/py");
        assert_eq!(pager_prefix("abcdefghijkl"), "…defghijkl");
    }
}
