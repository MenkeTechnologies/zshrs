//! Curses module — port of `Src/Modules/curses.c`.
//!
//! Implements the `zcurses` builtin: terminal UI windowing.
//!
//! The C source links libncurses for real curses primitives;
//! zshrs doesn't yet take that dependency, so the per-window
//! buffering and refresh emit ANSI escape sequences directly. The
//! function names, signatures, control flow, and module-static
//! layout mirror `curses.c` line-by-line; the ncurses calls are
//! the only deviation, marked with WARNING comments where they
//! occur.
//!
//! Structure mirrors the C source:
//!   - `struct ZCWin` (curses.c — port as `Window`)
//!   - module-statics `zcurses_windows` / `colorpairs` /
//!     `next_pair` (curses.c, file-scope)
//!   - `zccmd_*` family (curses.c:434-1567)
//!   - `bin_zcurses()` (curses.c:1568)
//!   - module entries (curses.c:1744-)

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};

use crate::ported::exec::ShellExecutor;
use crate::ported::module::{
    featuresarray, handlefeatures, setfeatureenables, Builtin, Features, Module,
};
use crate::ported::utils::zwarnnam;

// =====================================================================
// Attribute / Color tables (curses.c uses ncurses constants A_BOLD,
// COLOR_RED, etc. — Rust port re-exports the names with the same
// shape).
// =====================================================================

/// Port of the attribute-name table used by `zccmd_attr()`
/// (curses.c:843). Maps user-facing names (`bold`, `dim`,
/// `underline`, `blink`, `reverse`, `standout`) onto ncurses's
/// `A_BOLD` / etc. The Rust port emits ANSI SGR codes directly
/// rather than calling `wattron(A_BOLD)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attribute {
    Normal,
    Bold,
    Dim,
    Underline,
    Blink,
    Reverse,
    Standout,
}

impl Attribute {
    /// WARNING: NOT IN CURSES.C — Rust-only ANSI emitter. C calls
    /// `wattron(win, A_BOLD)` etc.; Rust port writes the matching
    /// SGR sequence to stdout because libncurses isn't linked.
    pub fn to_ansi(&self) -> &'static str {
        match self {
            Attribute::Normal => "\x1b[0m",
            Attribute::Bold => "\x1b[1m",
            Attribute::Dim => "\x1b[2m",
            Attribute::Underline => "\x1b[4m",
            Attribute::Blink => "\x1b[5m",
            Attribute::Reverse => "\x1b[7m",
            Attribute::Standout => "\x1b[7m",
        }
    }

    /// Port of the `zcurses_attrget()` lookup in
    /// `Src/Modules/curses.c:302`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "normal" => Some(Attribute::Normal),
            "bold" => Some(Attribute::Bold),
            "dim" => Some(Attribute::Dim),
            "underline" => Some(Attribute::Underline),
            "blink" => Some(Attribute::Blink),
            "reverse" => Some(Attribute::Reverse),
            "standout" => Some(Attribute::Standout),
            _ => None,
        }
    }
}

/// Port of the eight-color palette `zcurses_color()`
/// (curses.c:318) recognises plus the `default` terminal-default
/// code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Default,
}

impl Color {
    // WARNING: NOT IN CURSES.C — ANSI SGR fg/bg encoder. C uses
    // ncurses's `init_pair(...)` + `wattron(COLOR_PAIR(n))` to
    // address terminal colors; Rust port writes the SGR `30..47`
    // numbers directly.
    pub fn fg_code(&self) -> u8 {
        match self {
            Color::Black => 30,
            Color::Red => 31,
            Color::Green => 32,
            Color::Yellow => 33,
            Color::Blue => 34,
            Color::Magenta => 35,
            Color::Cyan => 36,
            Color::White => 37,
            Color::Default => 39,
        }
    }

    pub fn bg_code(&self) -> u8 {
        match self {
            Color::Black => 40,
            Color::Red => 41,
            Color::Green => 42,
            Color::Yellow => 43,
            Color::Blue => 44,
            Color::Magenta => 45,
            Color::Cyan => 46,
            Color::Cyan => 46,
            Color::White => 47,
            Color::Default => 49,
        }
    }

    /// Port of `zcurses_color()` from `Src/Modules/curses.c:318`.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "black" => Some(Color::Black),
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "yellow" => Some(Color::Yellow),
            "blue" => Some(Color::Blue),
            "magenta" => Some(Color::Magenta),
            "cyan" => Some(Color::Cyan),
            "white" => Some(Color::White),
            "default" => Some(Color::Default),
            _ => None,
        }
    }
}

// =====================================================================
// Port of `struct ZCWin` from Src/Modules/curses.c.
// =====================================================================

/// Per-window state. Port of `struct ZCWin` from
/// `Src/Modules/curses.c`. Same field set as C: name, dimensions,
/// origin, cursor position, scroll/keypad flags, color pair,
/// active attributes, plus the on-screen buffer ncurses owns
/// internally (Rust port keeps it explicit since libncurses isn't
/// linked).
#[derive(Debug)]
pub struct Window {
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    pub y: usize,
    pub x: usize,
    pub cursor_y: usize,
    pub cursor_x: usize,
    pub scroll: bool,
    pub keypad: bool,
    pub fg: Color,
    pub bg: Color,
    pub attrs: Vec<Attribute>,
    buffer: Vec<Vec<char>>,
}

impl Window {
    /// Port of the `zcurses_addwin()` per-window allocation in
    /// `Src/Modules/curses.c:503`.
    pub fn new(name: &str, rows: usize, cols: usize, y: usize, x: usize) -> Self {
        Self {
            name: name.to_string(),
            rows,
            cols,
            y,
            x,
            cursor_y: 0,
            cursor_x: 0,
            scroll: false,
            keypad: false,
            fg: Color::Default,
            bg: Color::Default,
            attrs: Vec::new(),
            buffer: vec![vec![' '; cols]; rows],
        }
    }

    /// Port of `stdscr` initialization inside `zccmd_init()`
    /// (curses.c:434). C calls libncurses's `initscr()` which
    /// returns the standard screen sized to `LINES` × `COLS`.
    pub fn stdscr() -> Self {
        let (rows, cols) = terminal_size().unwrap_or((24, 80));
        Self::new("stdscr", rows, cols, 0, 0)
    }

    pub fn move_cursor(&mut self, y: usize, x: usize) {
        if y < self.rows && x < self.cols {
            self.cursor_y = y;
            self.cursor_x = x;
        }
    }

    pub fn addch(&mut self, ch: char) {
        if self.cursor_y < self.rows && self.cursor_x < self.cols {
            self.buffer[self.cursor_y][self.cursor_x] = ch;
            self.cursor_x += 1;
            if self.cursor_x >= self.cols {
                self.cursor_x = 0;
                self.cursor_y += 1;
                if self.cursor_y >= self.rows {
                    if self.scroll {
                        self.scroll_up();
                    }
                    self.cursor_y = self.rows - 1;
                }
            }
        }
    }

    pub fn addstr(&mut self, s: &str) {
        for ch in s.chars() {
            self.addch(ch);
        }
    }

    pub fn clear(&mut self) {
        for row in &mut self.buffer {
            for cell in row {
                *cell = ' ';
            }
        }
        self.cursor_y = 0;
        self.cursor_x = 0;
    }

    fn scroll_up(&mut self) {
        self.buffer.remove(0);
        self.buffer.push(vec![' '; self.cols]);
    }

    pub fn refresh(&self) -> io::Result<()> {
        let mut stdout = io::stdout();
        write!(stdout, "\x1b[{};{}H", self.y + 1, self.x + 1)?;
        for attr in &self.attrs {
            write!(stdout, "{}", attr.to_ansi())?;
        }
        write!(stdout, "\x1b[{};{}m", self.fg.fg_code(), self.bg.bg_code())?;
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
// Module-static state — replaces the deleted `Curses` bag-of-globals
// struct. Per PORT_PLAN.md these mirror C's file-statics in
// `Src/Modules/curses.c` and are bucket 2 (shell-wide).
// =====================================================================

/// Port of `static LinkList zcurses_windows` from
/// `Src/Modules/curses.c`. Map keyed by window name; C uses a
/// linked list, the Rust port uses HashMap for O(1) lookup.
static WINDOWS: OnceLock<Mutex<HashMap<String, Window>>> = OnceLock::new();

/// Tracks whether `zccmd_init()` (curses.c:434) has been called.
/// C source dereferences `stdscr` (NULL → uninitialized).
static INITIALIZED: OnceLock<Mutex<bool>> = OnceLock::new();

/// Port of the `colorpairs` HashTable `zcurses_colorget()`
/// (curses.c:331) populates. Maps pair-index → (fg, bg).
static COLOR_PAIRS: OnceLock<Mutex<HashMap<i32, (Color, Color)>>> = OnceLock::new();

/// Port of `static int next_pair` from `Src/Modules/curses.c`.
/// Counter for `init_pair()` allocations.
static NEXT_PAIR: OnceLock<Mutex<i32>> = OnceLock::new();

// WARNING: NOT IN CURSES.C — Rust-only OnceLock get-or-init. C
// dereferences each global directly; Rust port factors the lock
// dance.
fn windows_lock() -> &'static Mutex<HashMap<String, Window>> {
    WINDOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

// WARNING: NOT IN CURSES.C — see windows_lock above.
fn initialized_lock() -> &'static Mutex<bool> {
    INITIALIZED.get_or_init(|| Mutex::new(false))
}

// WARNING: NOT IN CURSES.C — see windows_lock above.
fn color_pairs_lock() -> &'static Mutex<HashMap<i32, (Color, Color)>> {
    COLOR_PAIRS.get_or_init(|| Mutex::new(HashMap::new()))
}

// WARNING: NOT IN CURSES.C — see windows_lock above.
fn next_pair_lock() -> &'static Mutex<i32> {
    NEXT_PAIR.get_or_init(|| Mutex::new(1))
}

// WARNING: NOT IN CURSES.C — Rust-only TIOCGWINSZ probe. C uses
// libncurses's `LINES` / `COLS` globals which are populated by
// `initscr()` from terminfo; Rust port queries the kernel
// directly since libncurses isn't linked.
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

// WARNING: NOT IN CURSES.C — Rust-only termios shim. C calls
// libncurses's `cbreak()` which manipulates the same termios
// fields under the hood.
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

// =====================================================================
// `zccmd_*` family — port of curses.c:434-1567. Each takes
// `(nam: &str, args: &[String]) -> i32` matching C's
// `zccmd_*(const char *nam, char **args)` signature.
// =====================================================================

/// Port of `zccmd_init()` from `Src/Modules/curses.c:434`.
pub(crate) fn zccmd_init(_nam: &str, _args: &[String]) -> i32 {
    let mut init = initialized_lock().lock().unwrap();
    if *init {
        return 0;
    }
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b[?1049h\x1b[2J\x1b[H");
    let _ = stdout.flush();
    let _ = cbreak();
    let stdscr = Window::stdscr();
    windows_lock().lock().unwrap().insert("stdscr".into(), stdscr);
    *init = true;
    *next_pair_lock().lock().unwrap() = 1;
    0
}

/// Port of `zccmd_addwin()` from `Src/Modules/curses.c:503`.
pub(crate) fn zccmd_addwin(nam: &str, args: &[String]) -> i32 {
    if args.len() < 5 {
        zwarnnam(nam, "addwin: name rows cols y x required");
        return 1;
    }
    let name = args[0].as_str();
    let rows: usize = args[1].parse().unwrap_or(1);
    let cols: usize = args[2].parse().unwrap_or(1);
    let y: usize = args[3].parse().unwrap_or(0);
    let x: usize = args[4].parse().unwrap_or(0);
    let mut wins = windows_lock().lock().unwrap();
    if wins.contains_key(name) {
        zwarnnam(nam, &format!("window {} already exists", name));
        return 1;
    }
    wins.insert(name.to_string(), Window::new(name, rows, cols, y, x));
    0
}

/// Port of `zccmd_delwin()` from `Src/Modules/curses.c:564`.
pub(crate) fn zccmd_delwin(nam: &str, args: &[String]) -> i32 {
    if args.is_empty() {
        zwarnnam(nam, "delwin: window name required");
        return 1;
    }
    let name = args[0].as_str();
    if name == "stdscr" {
        zwarnnam(nam, "cannot delete stdscr");
        return 1;
    }
    if windows_lock().lock().unwrap().remove(name).is_none() {
        zwarnnam(nam, &format!("window {} not found", name));
        return 1;
    }
    0
}

/// Port of `zccmd_refresh()` from `Src/Modules/curses.c:632`.
pub(crate) fn zccmd_refresh(_nam: &str, args: &[String]) -> i32 {
    let wins = windows_lock().lock().unwrap();
    if args.is_empty() {
        // C: refresh all windows in the order they were created.
        for win in wins.values() {
            let _ = win.refresh();
        }
    } else if let Some(win) = wins.get(args[0].as_str()) {
        let _ = win.refresh();
    }
    0
}

/// Port of `zccmd_move()` from `Src/Modules/curses.c:669`.
pub(crate) fn zccmd_move(nam: &str, args: &[String]) -> i32 {
    if args.len() < 3 {
        zwarnnam(nam, "move: window y x required");
        return 1;
    }
    let name = args[0].as_str();
    let y: usize = args[1].parse().unwrap_or(0);
    let x: usize = args[2].parse().unwrap_or(0);
    let mut wins = windows_lock().lock().unwrap();
    match wins.get_mut(name) {
        Some(w) => {
            w.move_cursor(y, x);
            0
        }
        None => {
            drop(wins);
            zwarnnam(nam, &format!("window {} not found", name));
            1
        }
    }
}

/// Port of `zccmd_clear()` from `Src/Modules/curses.c:694`.
pub(crate) fn zccmd_clear(nam: &str, args: &[String]) -> i32 {
    let name = args.first().map(|s| s.as_str()).unwrap_or("stdscr");
    let mut wins = windows_lock().lock().unwrap();
    match wins.get_mut(name) {
        Some(w) => {
            w.clear();
            0
        }
        None => {
            drop(wins);
            zwarnnam(nam, &format!("window {} not found", name));
            1
        }
    }
}

/// Port of `zccmd_string()` from `Src/Modules/curses.c:759`.
pub(crate) fn zccmd_string(nam: &str, args: &[String]) -> i32 {
    if args.len() < 2 {
        zwarnnam(nam, "string: window text required");
        return 1;
    }
    let name = args[0].as_str();
    let text = args[1..].join(" ");
    let mut wins = windows_lock().lock().unwrap();
    match wins.get_mut(name) {
        Some(w) => {
            w.addstr(&text);
            0
        }
        None => {
            drop(wins);
            zwarnnam(nam, &format!("window {} not found", name));
            1
        }
    }
}

/// Port of `zccmd_attr()` from `Src/Modules/curses.c:843`.
pub(crate) fn zccmd_attr(nam: &str, args: &[String]) -> i32 {
    if args.len() < 2 {
        zwarnnam(nam, "attr: window attribute required");
        return 1;
    }
    let name = args[0].as_str();
    let attr_name = args[1].as_str();
    let attr = match Attribute::from_name(attr_name) {
        Some(a) => a,
        None => {
            zwarnnam(nam, &format!("unknown attribute {}", attr_name));
            return 1;
        }
    };
    let mut wins = windows_lock().lock().unwrap();
    match wins.get_mut(name) {
        Some(w) => {
            if !w.attrs.contains(&attr) {
                w.attrs.push(attr);
            }
            0
        }
        None => {
            drop(wins);
            zwarnnam(nam, &format!("window {} not found", name));
            1
        }
    }
}

/// Port of `zccmd_endwin()` from `Src/Modules/curses.c:823`.
pub(crate) fn zccmd_endwin(_nam: &str, _args: &[String]) -> i32 {
    let mut init = initialized_lock().lock().unwrap();
    if !*init {
        return 0;
    }
    let mut stdout = io::stdout();
    let _ = write!(stdout, "\x1b[?1049l\x1b[0m");
    let _ = stdout.flush();
    windows_lock().lock().unwrap().clear();
    color_pairs_lock().lock().unwrap().clear();
    *init = false;
    0
}

// =====================================================================
// Port of `bin_zcurses()` from Src/Modules/curses.c:1568.
// =====================================================================

/// Port of `bin_zcurses()` from `Src/Modules/curses.c:1568`.
///
/// Subcommand dispatcher. C uses a static lookup table mapping
/// names (`init`, `addwin`, `refresh`, etc.) to `zccmd_*` function
/// pointers; Rust port mirrors with a match.
pub(crate) fn bin_zcurses(_s: &mut ShellExecutor, nam: &str, args: &[String], _func: i32) -> i32 {
    let cmd = match args.first() {
        Some(c) => c.as_str(),
        None => {
            zwarnnam(nam, "subcommand required");
            return 1;
        }
    };
    let rest = &args[1..];
    match cmd {
        "init" => zccmd_init(nam, rest),
        "addwin" => zccmd_addwin(nam, rest),
        "delwin" => zccmd_delwin(nam, rest),
        "refresh" => zccmd_refresh(nam, rest),
        "move" => zccmd_move(nam, rest),
        "clear" => zccmd_clear(nam, rest),
        "string" => zccmd_string(nam, rest),
        "attr" => zccmd_attr(nam, rest),
        "end" | "endwin" => zccmd_endwin(nam, rest),
        // Remaining subcommands are stubbed pending the libncurses
        // FFI port. C handles them in zccmd_char/border/bg/scroll/
        // input/timeout/mouse/position/querychar/touch/resize.
        // WARNING: NOT IN CURSES.C — bin_notavail-equivalent stub.
        "char" | "border" | "bg" | "scroll" | "input" | "timeout" | "mouse" | "position"
        | "querychar" | "touch" | "resize" => {
            zwarnnam(nam, &format!("subcommand {} not yet ported (needs libncurses FFI)", cmd));
            1
        }
        _ => {
            zwarnnam(nam, &format!("unknown subcommand {}", cmd));
            1
        }
    }
}

// =====================================================================
// Module paraphernalia (curses.c module_features).
// =====================================================================

/// Port of `static struct builtin bintab[]` from `curses.c`.
///
/// ```c
/// BUILTIN("zcurses", 0, bin_zcurses, 1, -1, 0, NULL, NULL),
/// ```
static BINTAB: &[Builtin] = &[Builtin {
    name: "zcurses",
    flags: 0,
    minargs: 1,
    maxargs: -1,
    funcid: 0,
    optstr: None,
    defopts: None,
}];

/// Port of `static struct features module_features` from `curses.c`.
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
        *initialized_lock().lock().unwrap_or_else(|e| {
            initialized_lock().clear_poison();
            e.into_inner()
        }) = false;
        color_pairs_lock().lock().unwrap_or_else(|e| {
            color_pairs_lock().clear_poison();
            e.into_inner()
        }).clear();
        guard
    }

    #[test]
    fn test_attribute_from_name() {
        assert_eq!(Attribute::from_name("bold"), Some(Attribute::Bold));
        assert_eq!(Attribute::from_name("invalid"), None);
    }

    #[test]
    fn test_color_from_name() {
        assert_eq!(Color::from_name("red"), Some(Color::Red));
        assert_eq!(Color::from_name("invalid"), None);
    }

    #[test]
    fn test_window_addch() {
        let mut win = Window::new("test", 10, 20, 0, 0);
        win.addch('X');
        assert_eq!(win.buffer[0][0], 'X');
        assert_eq!(win.cursor_x, 1);
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
    fn test_enables_get_then_set() {
        let m = Module::new("zsh/curses");
        let mut enables: Option<Vec<i32>> = None;
        let rc = enables_(&m, &mut enables);
        assert_eq!(rc, 0);
        let v = enables.as_ref().unwrap();
        assert_eq!(v.len(), 1);
        let rc = enables_(&m, &mut enables);
        assert_eq!(rc, 0);
    }

    #[test]
    fn test_cleanup_returns_zero() {
        let m = Module::new("zsh/curses");
        assert_eq!(cleanup_(&m), 0);
    }

    #[test]
    fn test_zccmd_addwin_then_delwin() {
        let _g = reset();
        let rc = zccmd_addwin("zcurses", &[
            "win1".into(),
            "10".into(),
            "20".into(),
            "0".into(),
            "0".into(),
        ]);
        assert_eq!(rc, 0);
        assert_eq!(windows_lock().lock().unwrap().len(), 1);
        let rc = zccmd_delwin("zcurses", &["win1".into()]);
        assert_eq!(rc, 0);
        assert!(windows_lock().lock().unwrap().is_empty());
    }

    #[test]
    fn test_zccmd_addwin_duplicate_rejected() {
        let _g = reset();
        let args: Vec<String> = vec![
            "win1".into(),
            "10".into(),
            "20".into(),
            "0".into(),
            "0".into(),
        ];
        assert_eq!(zccmd_addwin("zcurses", &args), 0);
        assert_eq!(zccmd_addwin("zcurses", &args), 1);
    }

    #[test]
    fn test_zccmd_delwin_stdscr_rejected() {
        let _g = reset();
        let rc = zccmd_delwin("zcurses", &["stdscr".into()]);
        assert_eq!(rc, 1);
    }

    #[test]
    fn test_bin_zcurses_no_args() {
        let _g = reset();
        let mut s = ShellExecutor::new();
        assert_eq!(bin_zcurses(&mut s, "zcurses", &[], 0), 1);
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
}
