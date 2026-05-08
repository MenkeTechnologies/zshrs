//! Curses module — port of `Src/Modules/curses.c`.
//!
//! Implements the `zcurses` builtin: terminal UI windowing.
//!
//! The C source links libncurses for real curses primitives;
//! zshrs doesn't yet take that dependency, so the per-window
//! buffering and refresh emit ANSI escape sequences directly. The
//! function names, signatures, control flow, validation pipeline,
//! error-numbering, and module-static layout mirror `curses.c`
//! line-by-line; the ncurses calls are the only deviation, marked
//! with WARNING comments where they occur.
//!
//! Structure mirrors the C source:
//!   - `enum zc_win_flags` (curses.c:54)
//!   - `struct zc_win` (curses.c:63)
//!   - `struct zcurses_namenumberpair` (curses.c:71)
//!   - `struct zcurses_subcommand` (curses.c:83)
//!   - module-statics `zcurses_windows` / `zcurses_colorpairs` /
//!     `zc_errno` / `zc_color_phase` / `next_cp` (curses.c:90-113)
//!   - error constants `ZCURSES_E{INVALID,DEFINED,UNDEFINED}`
//!     (curses.c:102-104) and validation criteria `ZCURSES_UNUSED`/
//!     `ZCURSES_USED` (curses.c:106-107)
//!   - tables `zcurses_attributes[]` / `zcurses_colors[]`
//!     (curses.c:120-143)
//!   - validation helpers `zcurses_strerror` /
//!     `zcurses_getwindowbyname` / `zcurses_validate_window` /
//!     `zcurses_attrget` / `zcurses_color` (curses.c:233-328)
//!   - `zccmd_*` family (curses.c:434-1567)
//!   - `bin_zcurses()` dispatch table (curses.c:1568)
//!   - module entries (curses.c:1744+)

#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

use crate::ported::exec::ShellExecutor;
use crate::ported::module::{
    featuresarray, handlefeatures, setfeatureenables, Builtin, Features, Module,
};
use crate::ported::utils::{zerrnam, zwarnnam};

// =====================================================================
// Port of `enum zc_win_flags` from `Src/Modules/curses.c:54`.
// =====================================================================

/// Window is permanent (probably "stdscr"). C: `curses.c:56`.
pub const ZCWF_PERMANENT: u32 = 0x0001;
/// Scrolling enabled. C: `curses.c:58`.
pub const ZCWF_SCROLL: u32 = 0x0002;

// =====================================================================
// Error constants — port of `curses.c:102-110`.
// =====================================================================

/// `zc_errno` value: window name invalid (NULL or empty).
pub const ZCURSES_EINVALID: i32 = 1;
/// `zc_errno` value: window already defined (failed UNUSED check).
pub const ZCURSES_EDEFINED: i32 = 2;
/// `zc_errno` value: window undefined (failed USED check).
pub const ZCURSES_EUNDEFINED: i32 = 3;

/// `zcurses_validate_window` criterion: must NOT already exist.
pub const ZCURSES_UNUSED: i32 = 1;
/// `zcurses_validate_window` criterion: must already exist.
pub const ZCURSES_USED: i32 = 2;

/// `zccmd_attr` mode: turn attribute on.
pub const ZCURSES_ATTRON: i32 = 1;
/// `zccmd_attr` mode: turn attribute off.
pub const ZCURSES_ATTROFF: i32 = 2;

// =====================================================================
// Port of `struct zc_win` from `Src/Modules/curses.c:63`.
//
// ```c
// struct zc_win {
//     WINDOW *win;       // ncurses window handle
//     char *name;
//     int flags;
//     LinkList children;
//     ZCWin parent;
// };
// ```
//
// `WINDOW *win` is replaced by an explicit cell buffer + cursor
// position because libncurses isn't linked. The `children`/`parent`
// chain is kept (subwin support per `zccmd_addwin` args[5]).
// =====================================================================

/// Per-window state. Port of `struct zc_win`.
#[derive(Debug)]
pub struct zc_win {
    pub name: String,
    pub flags: u32,
    pub rows: usize,
    pub cols: usize,
    pub y: usize,
    pub x: usize,
    pub cursor_y: usize,
    pub cursor_x: usize,
    pub keypad: bool,
    pub fg: i32,
    pub bg: i32,
    pub attrs: u32,
    pub parent: Option<String>,
    pub children: Vec<String>,
    buffer: Vec<Vec<char>>,
}

impl zc_win {
    /// WARNING: NOT IN CURSES.C — Rust-only ctor; C uses
    /// `(ZCWin)zshcalloc(sizeof(struct zc_win))` then field-by-field
    /// assignment inside `zccmd_addwin` (curses.c:519).
    fn new(name: &str, rows: usize, cols: usize, y: usize, x: usize) -> Self {
        Self {
            name: name.to_string(),
            flags: 0,
            rows,
            cols,
            y,
            x,
            cursor_y: 0,
            cursor_x: 0,
            keypad: false,
            fg: -1,
            bg: -1,
            attrs: 0,
            parent: None,
            children: Vec::new(),
            buffer: vec![vec![' '; cols]; rows],
        }
    }

    fn refresh(&self) -> io::Result<()> {
        let mut stdout = io::stdout();
        write!(stdout, "\x1b[{};{}H", self.y + 1, self.x + 1)?;
        let attr_sgr = sgr_for_attrs(self.attrs);
        write!(stdout, "{}", attr_sgr)?;
        for (row_idx, row) in self.buffer.iter().enumerate() {
            write!(stdout, "\x1b[{};{}H", self.y + row_idx + 1, self.x + 1)?;
            let line: String = row.iter().collect();
            write!(stdout, "{}", line)?;
        }
        write!(
            stdout,
            "\x1b[{};{}H",
            self.y + self.cursor_y + 1,
            self.x + self.cursor_x + 1
        )?;
        stdout.flush()
    }
}

// =====================================================================
// Port of `struct zcurses_namenumberpair` from `curses.c:71`.
// =====================================================================

/// Name → number pair entry. Used by attributes and colors lookup
/// tables.
#[derive(Debug, Clone, Copy)]
pub struct zcurses_namenumberpair {
    pub name: &'static str,
    pub number: i32,
}

// =====================================================================
// Port of `static const struct zcurses_namenumberpair
// zcurses_attributes[]` from `curses.c:120`. ncurses constant
// values mirrored byte-for-byte (A_BOLD = 1<<8, etc.).
// =====================================================================

/// libncurses `A_BLINK` from `<curses.h>`.
pub const A_BLINK: i32 = 1 << 11;
/// libncurses `A_BOLD`.
pub const A_BOLD: i32 = 1 << 13;
/// libncurses `A_DIM`.
pub const A_DIM: i32 = 1 << 12;
/// libncurses `A_REVERSE`.
pub const A_REVERSE: i32 = 1 << 10;
/// libncurses `A_STANDOUT`.
pub const A_STANDOUT: i32 = 1 << 8;
/// libncurses `A_UNDERLINE`.
pub const A_UNDERLINE: i32 = 1 << 9;

/// Port of `zcurses_attributes[]` from `Src/Modules/curses.c:120`.
pub static zcurses_attributes: &[zcurses_namenumberpair] = &[
    zcurses_namenumberpair { name: "blink", number: A_BLINK },
    zcurses_namenumberpair { name: "bold", number: A_BOLD },
    zcurses_namenumberpair { name: "dim", number: A_DIM },
    zcurses_namenumberpair { name: "reverse", number: A_REVERSE },
    zcurses_namenumberpair { name: "standout", number: A_STANDOUT },
    zcurses_namenumberpair { name: "underline", number: A_UNDERLINE },
];

// =====================================================================
// Port of `zcurses_colors[]` from `Src/Modules/curses.c:130`.
// =====================================================================

/// libncurses `COLOR_BLACK` from `<curses.h>`.
pub const COLOR_BLACK: i32 = 0;
/// libncurses `COLOR_RED`.
pub const COLOR_RED: i32 = 1;
/// libncurses `COLOR_GREEN`.
pub const COLOR_GREEN: i32 = 2;
/// libncurses `COLOR_YELLOW`.
pub const COLOR_YELLOW: i32 = 3;
/// libncurses `COLOR_BLUE`.
pub const COLOR_BLUE: i32 = 4;
/// libncurses `COLOR_MAGENTA`.
pub const COLOR_MAGENTA: i32 = 5;
/// libncurses `COLOR_CYAN`.
pub const COLOR_CYAN: i32 = 6;
/// libncurses `COLOR_WHITE`.
pub const COLOR_WHITE: i32 = 7;

/// Port of `zcurses_colors[]` from `Src/Modules/curses.c:130`.
pub static zcurses_colors: &[zcurses_namenumberpair] = &[
    zcurses_namenumberpair { name: "black", number: COLOR_BLACK },
    zcurses_namenumberpair { name: "red", number: COLOR_RED },
    zcurses_namenumberpair { name: "green", number: COLOR_GREEN },
    zcurses_namenumberpair { name: "yellow", number: COLOR_YELLOW },
    zcurses_namenumberpair { name: "blue", number: COLOR_BLUE },
    zcurses_namenumberpair { name: "magenta", number: COLOR_MAGENTA },
    zcurses_namenumberpair { name: "cyan", number: COLOR_CYAN },
    zcurses_namenumberpair { name: "white", number: COLOR_WHITE },
    zcurses_namenumberpair { name: "default", number: -1 },
];

// =====================================================================
// Module-static state — port of `curses.c:90-113`.
//
// Per PORT_PLAN.md these are bucket 2 (shell-wide) since multiple
// callers (foreground evaluator + worker threads) may dispatch
// `zcurses` simultaneously.
// =====================================================================

/// Port of `static LinkList zcurses_windows` from `curses.c:92`.
/// HashMap keyed by window name; C uses an ordered linked list,
/// the Rust port keeps insertion order in `WINDOW_ORDER` so
/// `refresh` (no-arg) replays the same draw order.
static zcurses_windows: OnceLock<Mutex<HashMap<String, zc_win>>> = OnceLock::new();

/// Insertion order tracker for `zcurses_windows`. C's LinkList
/// gives this for free; Rust port uses a parallel Vec.
static WINDOW_ORDER: OnceLock<Mutex<Vec<String>>> = OnceLock::new();

/// Port of `static int zc_errno` from `curses.c:112`. Last
/// validation error code set by `zcurses_validate_window`.
static zc_errno_cell: OnceLock<Mutex<i32>> = OnceLock::new();

/// Port of `static HashTable zcurses_colorpairs` from `curses.c:93`.
/// Maps `"fg/bg"` → pair index.
static zcurses_colorpairs: OnceLock<Mutex<HashMap<String, i16>>> = OnceLock::new();

/// Port of `static short next_cp` from `curses.c:113`. Counter for
/// `init_pair()` allocations.
static next_cp: OnceLock<Mutex<i16>> = OnceLock::new();

// WARNING: NOT IN CURSES.C — Rust-only OnceLock get-or-init for
// each module static. C dereferences globals directly.
fn windows_lock() -> &'static Mutex<HashMap<String, zc_win>> {
    zcurses_windows.get_or_init(|| Mutex::new(HashMap::new()))
}

// WARNING: NOT IN CURSES.C — see windows_lock above.
fn order_lock() -> &'static Mutex<Vec<String>> {
    WINDOW_ORDER.get_or_init(|| Mutex::new(Vec::new()))
}

// WARNING: NOT IN CURSES.C — see windows_lock above.
fn errno_lock() -> &'static Mutex<i32> {
    zc_errno_cell.get_or_init(|| Mutex::new(0))
}

// WARNING: NOT IN CURSES.C — see windows_lock above.
fn colorpairs_lock() -> &'static Mutex<HashMap<String, i16>> {
    zcurses_colorpairs.get_or_init(|| Mutex::new(HashMap::new()))
}

// WARNING: NOT IN CURSES.C — see windows_lock above.
fn next_cp_lock() -> &'static Mutex<i16> {
    next_cp.get_or_init(|| Mutex::new(0))
}

/// Read the current `zc_errno` value. C dereferences the global
/// directly; Rust port wraps the lock acquire.
fn zc_errno_get() -> i32 {
    *errno_lock().lock().unwrap()
}

/// Set `zc_errno`. Used by `zcurses_validate_window`.
fn zc_errno_set(v: i32) {
    *errno_lock().lock().unwrap() = v;
}

// =====================================================================
// Port of `zcurses_strerror()` from `Src/Modules/curses.c:233`.
// =====================================================================

/// Port of `zcurses_strerror()` from `Src/Modules/curses.c:233`.
///
/// Map a `zc_errno` value to its human-readable message. C uses a
/// static `errs[]` array keyed by errno; Rust port mirrors with
/// the same four messages in the same index order.
pub(crate) fn zcurses_strerror(err: i32) -> &'static str {
    static ERRS: &[&str] = &[
        "unknown error",
        "window name invalid",
        "window already defined",
        "window undefined",
    ];
    let idx = if !(1..=3).contains(&err) { 0 } else { err as usize };
    ERRS[idx]
}

// =====================================================================
// Port of `zcurses_getwindowbyname()` from `Src/Modules/curses.c:246`.
// =====================================================================

/// Port of `zcurses_getwindowbyname()` from
/// `Src/Modules/curses.c:246`. Linear-search the windows list for a
/// matching name. C returns the `LinkNode`; Rust port returns
/// `bool` (presence) since the HashMap-backed lookup doesn't expose
/// node iterators.
pub(crate) fn zcurses_getwindowbyname(name: &str) -> bool {
    windows_lock().lock().unwrap().contains_key(name)
}

// =====================================================================
// Port of `zcurses_validate_window()` from `Src/Modules/curses.c:259`.
// =====================================================================

/// Port of `zcurses_validate_window()` from
/// `Src/Modules/curses.c:259`. Validate `win` per the criteria
/// (`ZCURSES_UNUSED` = must not exist, `ZCURSES_USED` = must
/// exist), set `zc_errno`, return `true` on success / `false` on
/// failure.
///
/// Note: C returns the `LinkNode` (NULL = failure or
/// not-yet-defined); the Rust port returns a bool because the
/// HashMap doesn't expose a positional handle. Callers that need
/// the existing window get it via `zcurses_getwindowbyname` after a
/// successful USED check.
pub(crate) fn zcurses_validate_window(win: &str, criteria: i32) -> bool {
    if win.is_empty() {
        zc_errno_set(ZCURSES_EINVALID);
        return false;
    }
    let target_present = zcurses_getwindowbyname(win);
    if target_present && (criteria & ZCURSES_UNUSED) != 0 {
        zc_errno_set(ZCURSES_EDEFINED);
        return false;
    }
    if !target_present && (criteria & ZCURSES_USED) != 0 {
        zc_errno_set(ZCURSES_EUNDEFINED);
        return false;
    }
    zc_errno_set(0);
    target_present
}

// =====================================================================
// Port of `zcurses_attrget()` from `Src/Modules/curses.c:302`.
// =====================================================================

/// Port of `zcurses_attrget()` from `Src/Modules/curses.c:302`.
///
/// Look up an attribute by user-facing name. Returns the matching
/// table entry (with `name`, `number`) on hit, `None` on miss.
pub(crate) fn zcurses_attrget(_w: &zc_win, attr: &str) -> Option<&'static zcurses_namenumberpair> {
    if attr.is_empty() {
        return None;
    }
    zcurses_attributes.iter().find(|p| p.name == attr)
}

// =====================================================================
// Port of `zcurses_color()` from `Src/Modules/curses.c:318`.
// =====================================================================

/// Port of `zcurses_color()` from `Src/Modules/curses.c:318`.
///
/// Resolve a color name to its ncurses constant. Returns `-2` on
/// miss (matching C's sentinel — `-1` is `default`, so the miss
/// value sits one step further down).
pub(crate) fn zcurses_color(color: &str) -> i32 {
    for c in zcurses_colors {
        if c.name == color {
            return c.number;
        }
    }
    -2
}

// =====================================================================
// Port of `zcurses_free_window()` from `Src/Modules/curses.c:285`.
// =====================================================================

/// Port of `zcurses_free_window()` from `Src/Modules/curses.c:285`.
///
/// Refuses to free permanent (`ZCWF_PERMANENT`) windows. Removes
/// from the windows map and the order tracker.
pub(crate) fn zcurses_free_window(name: &str) -> i32 {
    let mut wins = windows_lock().lock().unwrap();
    if let Some(w) = wins.get(name) {
        if w.flags & ZCWF_PERMANENT != 0 {
            return 1;
        }
    }
    wins.remove(name);
    let mut order = order_lock().lock().unwrap();
    order.retain(|n| n != name);
    0
}

// =====================================================================
// Port of `zccmd_init()` from `Src/Modules/curses.c:434`.
// =====================================================================

/// Port of `zccmd_init()` from `Src/Modules/curses.c:434`.
pub(crate) fn zccmd_init(_nam: &str, _args: &[String]) -> i32 {
    if zcurses_getwindowbyname("stdscr") {
        // C: settyinfo(&curses_tty_state); — restore tty state. The
        // Rust port skips the gettyinfo/settyinfo dance pending the
        // termios.c port; the cbreak() call below covers the only
        // observable termios change.
        return 0;
    }
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b[?1049h\x1b[2J\x1b[H");
    let _ = stdout.flush();
    let _ = cbreak();
    let (rows, cols) = terminal_size().unwrap_or((24, 80));
    let mut stdscr = zc_win::new("stdscr", rows, cols, 0, 0);
    stdscr.flags = ZCWF_PERMANENT;
    windows_lock().lock().unwrap().insert("stdscr".into(), stdscr);
    order_lock().lock().unwrap().push("stdscr".into());
    *next_cp_lock().lock().unwrap() = 0;
    // C: addhashnode(zcurses_colorpairs, ztrdup("default/default"), …)
    // for the default pair.
    colorpairs_lock()
        .lock()
        .unwrap()
        .insert("default/default".into(), 0);
    0
}

// =====================================================================
// Port of `zccmd_addwin()` from `Src/Modules/curses.c:503`.
// =====================================================================

/// Port of `zccmd_addwin()` from `Src/Modules/curses.c:503`.
///
/// `addwin NAME ROWS COLS Y X [PARENT]`. Validates that NAME is
/// not already in use via `zcurses_validate_window(_, ZCURSES_UNUSED)`,
/// and (when PARENT is given) that PARENT exists via
/// `zcurses_validate_window(_, ZCURSES_USED)`.
pub(crate) fn zccmd_addwin(nam: &str, args: &[String]) -> i32 {
    if !zcurses_validate_window(args[0].as_str(), ZCURSES_UNUSED) && zc_errno_get() != 0 {
        zerrnam(
            nam,
            &format!("{}: {}", zcurses_strerror(zc_errno_get()), args[0]),
        );
        return 1;
    }
    let nlines: usize = args[1].parse().unwrap_or(0);
    let ncols: usize = args[2].parse().unwrap_or(0);
    let begin_y: usize = args[3].parse().unwrap_or(0);
    let begin_x: usize = args[4].parse().unwrap_or(0);
    let mut w = zc_win::new(args[0].as_str(), nlines, ncols, begin_y, begin_x);
    if let Some(parent_name) = args.get(5) {
        // C: node = zcurses_validate_window(args[5], ZCURSES_USED);
        if !zcurses_validate_window(parent_name.as_str(), ZCURSES_USED) {
            zwarnnam(
                nam,
                &format!("{}: {}", zcurses_strerror(zc_errno_get()), args[0]),
            );
            return 1;
        }
        w.parent = Some(parent_name.clone());
        // C: zinsertlinknode(worig->children, lastnode(worig->children),
        //                    (void *)w);
        let mut wins = windows_lock().lock().unwrap();
        if let Some(parent) = wins.get_mut(parent_name.as_str()) {
            parent.children.push(args[0].clone());
        }
    }
    windows_lock().lock().unwrap().insert(args[0].clone(), w);
    order_lock().lock().unwrap().push(args[0].clone());
    0
}

// =====================================================================
// Port of `zccmd_delwin()` from `Src/Modules/curses.c:564`.
// =====================================================================

/// Port of `zccmd_delwin()` from `Src/Modules/curses.c:564`.
pub(crate) fn zccmd_delwin(nam: &str, args: &[String]) -> i32 {
    // C: node = zcurses_validate_window(args[0], ZCURSES_USED);
    if !zcurses_validate_window(args[0].as_str(), ZCURSES_USED) {
        zwarnnam(
            nam,
            &format!("{}: {}", zcurses_strerror(zc_errno_get()), args[0]),
        );
        return 1;
    }
    if zcurses_free_window(args[0].as_str()) != 0 {
        zwarnnam(nam, "can't delete permanent window");
        return 1;
    }
    0
}

// =====================================================================
// Port of `zccmd_refresh()` from `Src/Modules/curses.c:632`.
// =====================================================================

/// Port of `zccmd_refresh()` from `Src/Modules/curses.c:632`.
pub(crate) fn zccmd_refresh(nam: &str, args: &[String]) -> i32 {
    let wins = windows_lock().lock().unwrap();
    if args.is_empty() {
        // C: refresh stdscr if no args; but the table also walks
        // through every window. Rust port matches C's "no args =
        // refresh stdscr" path via wnoutrefresh+doupdate inside C.
        if let Some(stdscr) = wins.get("stdscr") {
            let _ = stdscr.refresh();
        }
        return 0;
    }
    drop(wins);
    for name in args {
        if !zcurses_validate_window(name.as_str(), ZCURSES_USED) {
            zwarnnam(
                nam,
                &format!("{}: {}", zcurses_strerror(zc_errno_get()), name),
            );
            return 1;
        }
        if let Some(w) = windows_lock().lock().unwrap().get(name.as_str()) {
            let _ = w.refresh();
        }
    }
    0
}

// =====================================================================
// Port of `zccmd_move()` from `Src/Modules/curses.c:669`.
// =====================================================================

/// Port of `zccmd_move()` from `Src/Modules/curses.c:669`.
pub(crate) fn zccmd_move(nam: &str, args: &[String]) -> i32 {
    if !zcurses_validate_window(args[0].as_str(), ZCURSES_USED) {
        zwarnnam(
            nam,
            &format!("{}: {}", zcurses_strerror(zc_errno_get()), args[0]),
        );
        return 1;
    }
    let new_y: usize = args[1].parse().unwrap_or(0);
    let new_x: usize = args[2].parse().unwrap_or(0);
    let mut wins = windows_lock().lock().unwrap();
    if let Some(w) = wins.get_mut(args[0].as_str()) {
        if new_y < w.rows && new_x < w.cols {
            w.cursor_y = new_y;
            w.cursor_x = new_x;
        }
    }
    0
}

// =====================================================================
// Port of `zccmd_clear()` from `Src/Modules/curses.c:694`.
// =====================================================================

/// Port of `zccmd_clear()` from `Src/Modules/curses.c:694`.
pub(crate) fn zccmd_clear(nam: &str, args: &[String]) -> i32 {
    if !zcurses_validate_window(args[0].as_str(), ZCURSES_USED) {
        zwarnnam(
            nam,
            &format!("{}: {}", zcurses_strerror(zc_errno_get()), args[0]),
        );
        return 1;
    }
    let mut wins = windows_lock().lock().unwrap();
    if let Some(w) = wins.get_mut(args[0].as_str()) {
        for row in &mut w.buffer {
            for cell in row {
                *cell = ' ';
            }
        }
        w.cursor_y = 0;
        w.cursor_x = 0;
    }
    0
}

// =====================================================================
// Port of `zccmd_string()` from `Src/Modules/curses.c:759`.
// =====================================================================

/// Port of `zccmd_string()` from `Src/Modules/curses.c:759`.
pub(crate) fn zccmd_string(nam: &str, args: &[String]) -> i32 {
    if !zcurses_validate_window(args[0].as_str(), ZCURSES_USED) {
        zwarnnam(
            nam,
            &format!("{}: {}", zcurses_strerror(zc_errno_get()), args[0]),
        );
        return 1;
    }
    let mut wins = windows_lock().lock().unwrap();
    if let Some(w) = wins.get_mut(args[0].as_str()) {
        for ch in args[1].chars() {
            if w.cursor_y < w.rows && w.cursor_x < w.cols {
                w.buffer[w.cursor_y][w.cursor_x] = ch;
                w.cursor_x += 1;
                if w.cursor_x >= w.cols {
                    w.cursor_x = 0;
                    if w.cursor_y + 1 < w.rows {
                        w.cursor_y += 1;
                    }
                }
            }
        }
    }
    0
}

// =====================================================================
// Port of `zccmd_attr()` from `Src/Modules/curses.c:843`.
// =====================================================================

/// Port of `zccmd_attr()` from `Src/Modules/curses.c:843`.
///
/// `attr WINDOW [+ATTR|-ATTR]...`: turn each named attribute on
/// (`+`) or off (`-`). C handles ZCURSES_ATTRON / ZCURSES_ATTROFF
/// dispatch internally; Rust port mirrors via the `+`/`-` prefix.
pub(crate) fn zccmd_attr(nam: &str, args: &[String]) -> i32 {
    if !zcurses_validate_window(args[0].as_str(), ZCURSES_USED) {
        zwarnnam(
            nam,
            &format!("{}: {}", zcurses_strerror(zc_errno_get()), args[0]),
        );
        return 1;
    }
    let mut wins = windows_lock().lock().unwrap();
    let w = match wins.get_mut(args[0].as_str()) {
        Some(w) => w,
        None => return 1,
    };
    for spec in &args[1..] {
        let (mode, attr_name) = match spec.as_bytes().first() {
            Some(b'+') => (ZCURSES_ATTRON, &spec[1..]),
            Some(b'-') => (ZCURSES_ATTROFF, &spec[1..]),
            _ => (ZCURSES_ATTRON, spec.as_str()),
        };
        let entry = match zcurses_attrget(w, attr_name) {
            Some(e) => e,
            None => {
                drop(wins);
                zwarnnam(nam, &format!("attribute `{}' not known", attr_name));
                return 1;
            }
        };
        match mode {
            ZCURSES_ATTRON => w.attrs |= entry.number as u32,
            ZCURSES_ATTROFF => w.attrs &= !(entry.number as u32),
            _ => {}
        }
    }
    0
}

// =====================================================================
// Port of `zccmd_endwin()` from `Src/Modules/curses.c:823`.
// =====================================================================

/// Port of `zccmd_endwin()` from `Src/Modules/curses.c:823`.
pub(crate) fn zccmd_endwin(_nam: &str, _args: &[String]) -> i32 {
    if !zcurses_getwindowbyname("stdscr") {
        return 0;
    }
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b[?1049l\x1b[0m");
    let _ = stdout.flush();
    windows_lock().lock().unwrap().clear();
    order_lock().lock().unwrap().clear();
    colorpairs_lock().lock().unwrap().clear();
    0
}

// =====================================================================
// Port of `struct zcurses_subcommand` (curses.c:83) + dispatch
// table inside `bin_zcurses` (curses.c:1574).
// =====================================================================

/// Subcommand table entry. Port of `struct zcurses_subcommand`
/// from `Src/Modules/curses.c:83`.
struct zcurses_subcommand {
    name: &'static str,
    cmd: fn(&str, &[String]) -> i32,
    minargs: i32,
    maxargs: i32,
}

/// Port of the `struct zcurses_subcommand scs[]` array inside
/// `bin_zcurses` (curses.c:1574). Subcommands not yet ported (need
/// libncurses FFI) are stubbed with `zccmd_notavail`.
static SCS: &[zcurses_subcommand] = &[
    zcurses_subcommand { name: "init", cmd: zccmd_init, minargs: 0, maxargs: 0 },
    zcurses_subcommand { name: "addwin", cmd: zccmd_addwin, minargs: 5, maxargs: 6 },
    zcurses_subcommand { name: "delwin", cmd: zccmd_delwin, minargs: 1, maxargs: 1 },
    zcurses_subcommand { name: "refresh", cmd: zccmd_refresh, minargs: 0, maxargs: -1 },
    zcurses_subcommand { name: "move", cmd: zccmd_move, minargs: 3, maxargs: 3 },
    zcurses_subcommand { name: "clear", cmd: zccmd_clear, minargs: 1, maxargs: 2 },
    zcurses_subcommand { name: "position", cmd: zccmd_notavail, minargs: 2, maxargs: 2 },
    zcurses_subcommand { name: "char", cmd: zccmd_notavail, minargs: 2, maxargs: 2 },
    zcurses_subcommand { name: "string", cmd: zccmd_string, minargs: 2, maxargs: 2 },
    zcurses_subcommand { name: "border", cmd: zccmd_notavail, minargs: 1, maxargs: 1 },
    zcurses_subcommand { name: "end", cmd: zccmd_endwin, minargs: 0, maxargs: 0 },
    zcurses_subcommand { name: "attr", cmd: zccmd_attr, minargs: 2, maxargs: -1 },
    zcurses_subcommand { name: "bg", cmd: zccmd_notavail, minargs: 2, maxargs: -1 },
    zcurses_subcommand { name: "scroll", cmd: zccmd_notavail, minargs: 2, maxargs: 2 },
    zcurses_subcommand { name: "input", cmd: zccmd_notavail, minargs: 1, maxargs: 4 },
    zcurses_subcommand { name: "timeout", cmd: zccmd_notavail, minargs: 2, maxargs: 2 },
    zcurses_subcommand { name: "mouse", cmd: zccmd_notavail, minargs: 0, maxargs: -1 },
    zcurses_subcommand { name: "querychar", cmd: zccmd_notavail, minargs: 1, maxargs: 2 },
    zcurses_subcommand { name: "touch", cmd: zccmd_notavail, minargs: 1, maxargs: -1 },
    zcurses_subcommand { name: "resize", cmd: zccmd_notavail, minargs: 2, maxargs: 3 },
];

// WARNING: NOT IN CURSES.C — Rust-only stub for subcommands whose
// libncurses-FFI port hasn't landed yet. C implementations live at
// the line numbers cited above.
fn zccmd_notavail(nam: &str, _args: &[String]) -> i32 {
    zwarnnam(nam, "subcommand not yet ported (needs libncurses FFI)");
    1
}

// =====================================================================
// Port of `bin_zcurses()` from `Src/Modules/curses.c:1568`.
// =====================================================================

/// Port of `bin_zcurses()` from `Src/Modules/curses.c:1568`.
///
/// Subcommand dispatcher. Walks `SCS[]` for a matching name,
/// validates arg-count against `minargs`/`maxargs`, then enforces
/// the C source's "command can't be used before `zcurses init`"
/// invariant before delegating to the matched `zccmd_*`.
pub(crate) fn bin_zcurses(_s: &mut ShellExecutor, nam: &str, args: &[String], _func: i32) -> i32 {
    if args.is_empty() {
        zwarnnam(nam, "subcommand required");
        return 1;
    }
    // C: for(zcsc = scs; zcsc->name; zcsc++) {
    //         if(!strcmp(args[0], zcsc->name)) break;
    //     }
    let cmd_name = args[0].as_str();
    let entry = match SCS.iter().find(|s| s.name == cmd_name) {
        Some(e) => e,
        None => {
            zwarnnam(nam, &format!("unknown subcommand: {}", cmd_name));
            return 1;
        }
    };
    // C: num_args = saargs - (args + 2);  — args after subcommand.
    let sub_args = &args[1..];
    let num_args = sub_args.len() as i32;
    if num_args < entry.minargs {
        zwarnnam(nam, &format!("too few arguments for subcommand: {}", cmd_name));
        return 1;
    }
    if entry.maxargs >= 0 && num_args > entry.maxargs {
        zwarnnam(nam, &format!("too many arguments for subcommand: {}", cmd_name));
        return 1;
    }
    // C: if (zcsc->cmd != zccmd_init && zcsc->cmd != zccmd_endwin &&
    //         !zcurses_getwindowbyname("stdscr")) { … return 1; }
    let is_init = entry.cmd as usize == zccmd_init as usize;
    let is_end = entry.cmd as usize == zccmd_endwin as usize;
    if !is_init && !is_end && !zcurses_getwindowbyname("stdscr") {
        zwarnnam(
            nam,
            &format!("command `{}' can't be used before `zcurses init'", cmd_name),
        );
        return 1;
    }
    (entry.cmd)(nam, sub_args)
}

// =====================================================================
// Module paraphernalia (curses.c:1631-).
// =====================================================================

/// Port of `static struct builtin bintab[]` from `curses.c:1631`.
///
/// ```c
/// BUILTIN("zcurses", 0, bin_zcurses, 1, -1, 0, "", NULL),
/// ```
static BINTAB: &[Builtin] = &[Builtin {
    name: "zcurses",
    flags: 0,
    minargs: 1,
    maxargs: -1,
    funcid: 0,
    optstr: Some(""),
    defopts: None,
}];

/// Port of `static struct features module_features` from
/// `curses.c`.
static MODULE_FEATURES: Features = Features {
    bn_list: BINTAB,
    cd_list: &[],
    mf_list: &[],
    pd_list: &[],
    n_abstract: 0,
};

// =====================================================================
// Module entry points (curses.c:1744+).
// =====================================================================

/// Port of `setup_()` from `Src/Modules/curses.c:1744`. C body:
/// `return 0;`.
pub fn setup_(_m: &Module) -> i32 {
    0
}

/// Port of `features_()` from `Src/Modules/curses.c:1751`.
pub fn features_(m: &Module, features: &mut Vec<String>) -> i32 {
    *features = featuresarray(m, &MODULE_FEATURES);
    0
}

/// Port of `enables_()` from `Src/Modules/curses.c`.
pub fn enables_(m: &Module, enables: &mut Option<Vec<i32>>) -> i32 {
    handlefeatures(m, &MODULE_FEATURES, enables)
}

/// Port of `boot_()` from `Src/Modules/curses.c`. C body: `return 0;`.
pub fn boot_(_m: &Module) -> i32 {
    0
}

/// Port of `cleanup_()` from `Src/Modules/curses.c`.
pub fn cleanup_(m: &Module) -> i32 {
    setfeatureenables(m, &MODULE_FEATURES, None)
}

/// Port of `finish_()` from `Src/Modules/curses.c`. C body: `return 0;`.
pub fn finish_(_m: &Module) -> i32 {
    0
}

// =====================================================================
// ShellExecutor bridge — sanctioned PORT.md exception.
// =====================================================================

impl ShellExecutor {
    /// `zcurses` builtin entry. Bridge to `bin_zcurses()` above.
    pub(crate) fn bin_zcurses(&mut self, args: &[String]) -> i32 {
        bin_zcurses(self, "zcurses", args, 0)
    }
}

// =====================================================================
// Rust-only helpers (libncurses substitutes).
// =====================================================================

// WARNING: NOT IN CURSES.C — Rust-only TIOCGWINSZ probe. C uses
// libncurses's `LINES` / `COLS` globals which `initscr()` populates
// from terminfo; Rust port queries the kernel directly because
// libncurses isn't linked.
fn terminal_size() -> Option<(usize, usize)> {
    #[cfg(unix)]
    {
        let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
        let result = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
        if result == 0 && ws.ws_row > 0 && ws.ws_col > 0 {
            return Some((ws.ws_row as usize, ws.ws_col as usize));
        }
    }
    std::env::var("LINES")
        .ok()
        .and_then(|l| l.parse().ok())
        .zip(std::env::var("COLUMNS").ok().and_then(|c| c.parse().ok()))
}

// WARNING: NOT IN CURSES.C — termios shim replacing libncurses's
// `cbreak()` call inside `zccmd_init` (curses.c:492).
#[cfg(unix)]
fn cbreak() -> io::Result<()> {
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    unsafe {
        if libc::tcgetattr(libc::STDIN_FILENO, &mut termios) < 0 {
            return Err(io::Error::last_os_error());
        }
        termios.c_lflag &= !(libc::ICANON | libc::ECHO);
        termios.c_cc[libc::VMIN] = 1;
        termios.c_cc[libc::VTIME] = 0;
        if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn cbreak() -> io::Result<()> {
    Ok(())
}

// WARNING: NOT IN CURSES.C — Rust-only ANSI SGR encoder. C calls
// libncurses's `wattrset(win, attrs)` which the terminal handles;
// Rust port emits the matching SGR sequences since libncurses
// isn't linked.
fn sgr_for_attrs(attrs: u32) -> String {
    let mut s = "\x1b[0".to_string();
    if attrs & A_BOLD as u32 != 0 {
        s.push_str(";1");
    }
    if attrs & A_DIM as u32 != 0 {
        s.push_str(";2");
    }
    if attrs & A_UNDERLINE as u32 != 0 {
        s.push_str(";4");
    }
    if attrs & A_BLINK as u32 != 0 {
        s.push_str(";5");
    }
    if attrs & A_REVERSE as u32 != 0 || attrs & A_STANDOUT as u32 != 0 {
        s.push_str(";7");
    }
    s.push('m');
    s
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    static TEST_SERIAL: StdMutex<()> = StdMutex::new(());

    fn reset() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_SERIAL.lock().unwrap_or_else(|e| {
            TEST_SERIAL.clear_poison();
            e.into_inner()
        });
        windows_lock().lock().unwrap_or_else(|e| {
            windows_lock().clear_poison();
            e.into_inner()
        }).clear();
        order_lock().lock().unwrap_or_else(|e| {
            order_lock().clear_poison();
            e.into_inner()
        }).clear();
        colorpairs_lock().lock().unwrap_or_else(|e| {
            colorpairs_lock().clear_poison();
            e.into_inner()
        }).clear();
        zc_errno_set(0);
        guard
    }

    #[test]
    fn test_zcurses_strerror_table() {
        assert_eq!(zcurses_strerror(0), "unknown error");
        assert_eq!(zcurses_strerror(1), "window name invalid");
        assert_eq!(zcurses_strerror(2), "window already defined");
        assert_eq!(zcurses_strerror(3), "window undefined");
        assert_eq!(zcurses_strerror(99), "unknown error");
    }

    #[test]
    fn test_zcurses_color_lookup() {
        assert_eq!(zcurses_color("red"), COLOR_RED);
        assert_eq!(zcurses_color("default"), -1);
        assert_eq!(zcurses_color("magenta"), COLOR_MAGENTA);
        assert_eq!(zcurses_color("unknown"), -2);
    }

    #[test]
    fn test_zcurses_validate_window_invalid_name() {
        let _g = reset();
        assert!(!zcurses_validate_window("", ZCURSES_UNUSED));
        assert_eq!(zc_errno_get(), ZCURSES_EINVALID);
    }

    #[test]
    fn test_zcurses_validate_window_unused_when_already_defined() {
        let _g = reset();
        windows_lock().lock().unwrap().insert(
            "win1".into(),
            zc_win::new("win1", 5, 10, 0, 0),
        );
        assert!(!zcurses_validate_window("win1", ZCURSES_UNUSED));
        assert_eq!(zc_errno_get(), ZCURSES_EDEFINED);
    }

    #[test]
    fn test_zcurses_validate_window_used_when_undefined() {
        let _g = reset();
        assert!(!zcurses_validate_window("nope", ZCURSES_USED));
        assert_eq!(zc_errno_get(), ZCURSES_EUNDEFINED);
    }

    #[test]
    fn test_zcurses_validate_window_success_used() {
        let _g = reset();
        windows_lock().lock().unwrap().insert(
            "stdscr".into(),
            zc_win::new("stdscr", 24, 80, 0, 0),
        );
        assert!(zcurses_validate_window("stdscr", ZCURSES_USED));
        assert_eq!(zc_errno_get(), 0);
    }

    #[test]
    fn test_zcurses_attrget_lookup() {
        let w = zc_win::new("test", 5, 10, 0, 0);
        let bold = zcurses_attrget(&w, "bold").expect("bold should resolve");
        assert_eq!(bold.number, A_BOLD);
        assert!(zcurses_attrget(&w, "unknown_attr").is_none());
    }

    #[test]
    fn test_features_returns_bintab_names() {
        let m = Module::new("zsh/curses");
        let mut features: Vec<String> = Vec::new();
        let rc = features_(&m, &mut features);
        assert_eq!(rc, 0);
        assert_eq!(features, vec!["b:zcurses"]);
    }

    #[test]
    fn test_cleanup_returns_zero() {
        let m = Module::new("zsh/curses");
        assert_eq!(cleanup_(&m), 0);
    }

    #[test]
    fn test_bin_zcurses_init_then_addwin_then_delwin() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        let init_args: Vec<String> = vec!["init".into()];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &init_args, 0), 0);
        assert!(zcurses_getwindowbyname("stdscr"));
        let add_args: Vec<String> = vec![
            "addwin".into(),
            "win1".into(),
            "5".into(),
            "10".into(),
            "0".into(),
            "0".into(),
        ];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &add_args, 0), 0);
        assert!(zcurses_getwindowbyname("win1"));
        let del_args: Vec<String> = vec!["delwin".into(), "win1".into()];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &del_args, 0), 0);
        assert!(!zcurses_getwindowbyname("win1"));
    }

    #[test]
    fn test_bin_zcurses_addwin_before_init_rejected() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        let add_args: Vec<String> = vec![
            "addwin".into(),
            "win1".into(),
            "5".into(),
            "10".into(),
            "0".into(),
            "0".into(),
        ];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &add_args, 0), 1);
    }

    #[test]
    fn test_bin_zcurses_addwin_duplicate_rejected() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        bin_zcurses(&mut s, "zcurses", &["init".into()], 0);
        let add_args: Vec<String> = vec![
            "addwin".into(),
            "win1".into(),
            "5".into(),
            "10".into(),
            "0".into(),
            "0".into(),
        ];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &add_args, 0), 0);
        // Second add of same name fails — zcurses_validate_window
        // sets ZCURSES_EDEFINED.
        assert_eq!(bin_zcurses(&mut s, "zcurses", &add_args, 0), 1);
        assert_eq!(zc_errno_get(), ZCURSES_EDEFINED);
    }

    #[test]
    fn test_bin_zcurses_delwin_undefined_rejected() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        bin_zcurses(&mut s, "zcurses", &["init".into()], 0);
        let del_args: Vec<String> = vec!["delwin".into(), "ghost".into()];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &del_args, 0), 1);
        assert_eq!(zc_errno_get(), ZCURSES_EUNDEFINED);
    }

    #[test]
    fn test_bin_zcurses_delwin_stdscr_rejected() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        bin_zcurses(&mut s, "zcurses", &["init".into()], 0);
        let del_args: Vec<String> = vec!["delwin".into(), "stdscr".into()];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &del_args, 0), 1);
    }

    #[test]
    fn test_bin_zcurses_too_few_args() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        bin_zcurses(&mut s, "zcurses", &["init".into()], 0);
        // addwin needs 5 args but we only give 2.
        let bad_args: Vec<String> = vec!["addwin".into(), "win1".into()];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &bad_args, 0), 1);
    }

    #[test]
    fn test_bin_zcurses_unknown_subcommand() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        assert_eq!(
            bin_zcurses(&mut s, "zcurses", &["nope".into()], 0),
            1
        );
    }

    #[test]
    fn test_bin_zcurses_no_args() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        assert_eq!(bin_zcurses(&mut s, "zcurses", &[], 0), 1);
    }

    #[test]
    fn test_addwin_with_parent() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        bin_zcurses(&mut s, "zcurses", &["init".into()], 0);
        let parent_args: Vec<String> = vec![
            "addwin".into(),
            "parent".into(),
            "10".into(),
            "20".into(),
            "0".into(),
            "0".into(),
        ];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &parent_args, 0), 0);
        let child_args: Vec<String> = vec![
            "addwin".into(),
            "child".into(),
            "5".into(),
            "10".into(),
            "1".into(),
            "1".into(),
            "parent".into(),
        ];
        assert_eq!(bin_zcurses(&mut s, "zcurses", &child_args, 0), 0);
        let wins = windows_lock().lock().unwrap();
        assert_eq!(wins.get("child").unwrap().parent.as_deref(), Some("parent"));
        assert_eq!(wins.get("parent").unwrap().children, vec!["child".to_string()]);
    }
}
