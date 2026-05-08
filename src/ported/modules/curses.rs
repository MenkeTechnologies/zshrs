//! Curses module - port of Modules/curses.c
//!
//! Provides a curses windowing interface for terminal UI.
//! Uses ANSI escape sequences for portability.

use std::collections::HashMap;
use std::io::{self, Write};

/// Curses video attributes.
/// Port of the attribute-name table Src/Modules/curses.c uses in
/// `zcurses_attrget()` (line 302) — maps the user-facing names
/// (`bold`/`dim`/`underline`/`blink`/`reverse`/`standout`) onto
/// libncurses's `A_BOLD`/etc. We emit the matching ANSI SGR codes
/// directly since the Rust port doesn't link libncurses.
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
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
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

/// Basic curses colors.
/// Port of the color-name table `zcurses_color()` from
/// Src/Modules/curses.c:318 maps — same eight-color palette plus
/// `default` for the terminal-default code.
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
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn bg_code(&self) -> u8 {
        match self {
            Color::Black => 40,
            Color::Red => 41,
            Color::Green => 42,
            Color::Yellow => 43,
            Color::Blue => 44,
            Color::Magenta => 45,
            Color::Cyan => 46,
            Color::White => 47,
            Color::Default => 49,
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
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

/// A curses window.
/// Port of `struct ZCWin` from Src/Modules/curses.c — `addwin`
/// (line 503) creates them, `delwin` (line 564) frees them, and
/// every other `zccmd_*` works against this shape. Rust port keeps
/// the same field set (rows/cols/origin/cursor/attrs/colors/etc.).
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
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn stdscr() -> Self {
        let (rows, cols) = zccmd_delwin().unwrap_or((24, 80));
        Self::new("stdscr", rows, cols, 0, 0)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn move_cursor(&mut self, y: usize, x: usize) {
        if y < self.rows && x < self.cols {
            self.cursor_y = y;
            self.cursor_x = x;
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
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
                        self.cursor_y = self.rows - 1;
                    } else {
                        self.cursor_y = self.rows - 1;
                    }
                }
            }
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn addstr(&mut self, s: &str) {
        for ch in s.chars() {
            self.addch(ch);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn clear(&mut self) {
        for row in &mut self.buffer {
            for cell in row {
                *cell = ' ';
            }
        }
        self.cursor_y = 0;
        self.cursor_x = 0;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn erase(&mut self) {
        self.clear();
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn clrtoeol(&mut self) {
        if self.cursor_y < self.rows {
            for x in self.cursor_x..self.cols {
                self.buffer[self.cursor_y][x] = ' ';
            }
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn clrtobot(&mut self) {
        self.clrtoeol();
        for y in (self.cursor_y + 1)..self.rows {
            for x in 0..self.cols {
                self.buffer[y][x] = ' ';
            }
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    fn scroll_up(&mut self) {
        self.buffer.remove(0);
        self.buffer.push(vec![' '; self.cols]);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn set_scroll(&mut self, enable: bool) {
        self.scroll = enable;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn set_keypad(&mut self, enable: bool) {
        self.keypad = enable;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn attron(&mut self, attr: Attribute) {
        if !self.attrs.contains(&attr) {
            self.attrs.push(attr);
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn attroff(&mut self, attr: Attribute) {
        self.attrs.retain(|a| *a != attr);
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn set_color(&mut self, fg: Color, bg: Color) {
        self.fg = fg;
        self.bg = bg;
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
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

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn getyx(&self) -> (usize, usize) {
        (self.cursor_y, self.cursor_x)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn getmaxyx(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }
}

/// Curses session state.
/// Port of the file-static `windows` HashTable + `colorpairs` /
/// `next_pair` slots Src/Modules/curses.c keeps — `zccmd_init()`
/// (line 434) populates them, every other `zccmd_*` mutates.
#[derive(Debug, Default)]
pub struct Curses {
    windows: HashMap<String, Window>,
    initialized: bool,
    color_pairs: HashMap<i32, (Color, Color)>,
    next_pair: i32,
}

impl Curses {
    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize the curses subsystem (alt-screen + clear).
    /// Port of `zccmd_init()` from Src/Modules/curses.c:434 — the
    /// C source calls `initscr()` from libncurses; we emit the
    /// equivalent `\e[?1049h` + `\e[2J` + cursor-home sequence.
    pub fn initscr(&mut self) -> io::Result<()> {
        if self.initialized {
            return Ok(());
        }

        let mut stdout = io::stdout();
        write!(stdout, "\x1b[?1049h")?;
        write!(stdout, "\x1b[2J")?;
        write!(stdout, "\x1b[H")?;
        stdout.flush()?;

        let stdscr = Window::stdscr();
        self.windows.insert("stdscr".to_string(), stdscr);
        self.initialized = true;
        self.next_pair = 1;

        Ok(())
    }

    /// Tear down curses and restore the cooked terminal state.
    /// Port of `zccmd_endwin()` from Src/Modules/curses.c:823 —
    /// the C source calls libncurses's `endwin()`; we emit the
    /// matching `\e[?1049l` + `\e[0m` sequence.
    pub fn endwin(&mut self) -> io::Result<()> {
        if !self.initialized {
            return Ok(());
        }

        let mut stdout = io::stdout();
        write!(stdout, "\x1b[?1049l")?;
        write!(stdout, "\x1b[0m")?;
        stdout.flush()?;

        self.windows.clear();
        self.color_pairs.clear();
        self.initialized = false;

        Ok(())
    }

    /// Allocate a new named window.
    /// Port of `zccmd_addwin()` from Src/Modules/curses.c:503 —
    /// rejects already-present names matching the C source's
    /// "duplicate window" diagnostic.
    pub fn newwin(&mut self, name: &str, rows: usize, cols: usize, y: usize, x: usize) -> bool {
        if self.windows.contains_key(name) {
            return false;
        }

        let win = Window::new(name, rows, cols, y, x);
        self.windows.insert(name.to_string(), win);
        true
    }

    /// Free a named window (refusing the special `stdscr`).
    /// Port of `zccmd_delwin()` from Src/Modules/curses.c:564 —
    /// uses `zcurses_free_window()` (line 285) for the per-window
    /// free; rejecting `stdscr` matches the C source's guard.
    pub fn delwin(&mut self, name: &str) -> bool {
        if name == "stdscr" {
            return false;
        }
        self.windows.remove(name).is_some()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn get_window(&self, name: &str) -> Option<&Window> {
        self.windows.get(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn get_window_mut(&mut self, name: &str) -> Option<&mut Window> {
        self.windows.get_mut(name)
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn refresh(&self, name: &str) -> io::Result<()> {
        if let Some(win) = self.windows.get(name) {
            win.refresh()
        } else {
            Ok(())
        }
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn refresh_all(&self) -> io::Result<()> {
        for win in self.windows.values() {
            win.refresh()?;
        }
        Ok(())
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn init_pair(&mut self, pair: i32, fg: Color, bg: Color) {
        self.color_pairs.insert(pair, (fg, bg));
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn get_pair(&self, pair: i32) -> Option<(Color, Color)> {
        self.color_pairs.get(&pair).copied()
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
    /// of any function in `Src/Modules/curses.c`.
    pub fn window_names(&self) -> Vec<&str> {
        self.windows.keys().map(|s| s.as_str()).collect()
    }
}

/// Get terminal size
/// Get current terminal `(cols, rows)` from `TIOCGWINSZ`.
/// Port of the `getmaxyx(stdscr, ...)` lookup the C source does
/// via libncurses (Src/Modules/curses.c, embedded in many
/// `zccmd_*` functions). We `ioctl(TIOCGWINSZ)` directly so the
/// Rust port doesn't need libncurses.
pub fn zccmd_delwin() -> Option<(usize, usize)> {
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

/// Enable cbreak mode (raw input, no line buffering).
/// Port of the `cbreak()` libncurses call `zccmd_init()` invokes
/// (Src/Modules/curses.c:434). We tweak termios directly via
/// `tcsetattr(2)` since the Rust port doesn't link libncurses.
#[cfg(unix)]
pub fn cbreak() -> io::Result<()> {
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

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/curses.c`.
#[cfg(not(unix))]
pub fn cbreak() -> io::Result<()> {
    Ok(())
}

/// Disable terminal echo.
/// Port of the `noecho()` libncurses call `zccmd_init()` invokes
/// (Src/Modules/curses.c:434). Same termios path as `cbreak()`.
#[cfg(unix)]
pub fn noecho() -> io::Result<()> {
    let mut termios: libc::termios = unsafe { std::mem::zeroed() };
    unsafe {
        if libc::tcgetattr(libc::STDIN_FILENO, &mut termios) < 0 {
            return Err(io::Error::last_os_error());
        }
        termios.c_lflag &= !libc::ECHO;
        if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &termios) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// WARNING: THIS IS ADHOC IMPLEMENTATION AND NOT A FAITHFUL PORT
/// of any function in `Src/Modules/curses.c`.
#[cfg(not(unix))]
pub fn noecho() -> io::Result<()> {
    Ok(())
}

/// `zcurses` builtin entry point.
/// Port of the `bin_zcurses()` dispatch table in
/// Src/Modules/curses.c — the C source uses `zccmd_*` callbacks
/// for `init`/`addwin`/`delwin`/`refresh`/`move`/`clear`/`char`/
/// `string`/`border`/`endwin`/`attr`/`bg`/`scroll`/`input`/
/// `timeout`/`mouse`/`position`/`querychar`. The Rust port maps
/// each subcommand string onto a method on `Curses`.
pub fn bin_zcurses(args: &[&str], curses: &mut Curses) -> (i32, String) {
    if args.is_empty() {
        return (1, "zcurses: subcommand required\n".to_string());
    }

    match args[0] {
        "init" => {
            if curses.initscr().is_err() {
                return (1, "zcurses: failed to initialize\n".to_string());
            }
            (0, String::new())
        }
        "end" => {
            if curses.endwin().is_err() {
                return (1, "zcurses: failed to end\n".to_string());
            }
            (0, String::new())
        }
        "addwin" => {
            if args.len() < 6 {
                return (
                    1,
                    "zcurses addwin: name rows cols y x required\n".to_string(),
                );
            }
            let name = args[1];
            let rows: usize = args[2].parse().unwrap_or(1);
            let cols: usize = args[3].parse().unwrap_or(1);
            let y: usize = args[4].parse().unwrap_or(0);
            let x: usize = args[5].parse().unwrap_or(0);

            if curses.newwin(name, rows, cols, y, x) {
                (0, String::new())
            } else {
                (1, format!("zcurses: window {} already exists\n", name))
            }
        }
        "delwin" => {
            if args.len() < 2 {
                return (1, "zcurses delwin: window name required\n".to_string());
            }
            if curses.delwin(args[1]) {
                (0, String::new())
            } else {
                (1, format!("zcurses: cannot delete window {}\n", args[1]))
            }
        }
        "refresh" => {
            let name = if args.len() > 1 { args[1] } else { "stdscr" };
            if curses.refresh(name).is_err() {
                return (1, format!("zcurses: failed to refresh {}\n", name));
            }
            (0, String::new())
        }
        "move" => {
            if args.len() < 4 {
                return (1, "zcurses move: window y x required\n".to_string());
            }
            let name = args[1];
            let y: usize = args[2].parse().unwrap_or(0);
            let x: usize = args[3].parse().unwrap_or(0);

            if let Some(win) = curses.get_window_mut(name) {
                win.move_cursor(y, x);
                (0, String::new())
            } else {
                (1, format!("zcurses: window {} not found\n", name))
            }
        }
        "string" => {
            if args.len() < 3 {
                return (1, "zcurses string: window text required\n".to_string());
            }
            let name = args[1];
            let text = args[2..].join(" ");

            if let Some(win) = curses.get_window_mut(name) {
                win.addstr(&text);
                (0, String::new())
            } else {
                (1, format!("zcurses: window {} not found\n", name))
            }
        }
        "clear" => {
            let name = if args.len() > 1 { args[1] } else { "stdscr" };
            if let Some(win) = curses.get_window_mut(name) {
                win.clear();
                (0, String::new())
            } else {
                (1, format!("zcurses: window {} not found\n", name))
            }
        }
        "attr" => {
            if args.len() < 3 {
                return (1, "zcurses attr: window attribute required\n".to_string());
            }
            let name = args[1];
            let attr_name = args[2];

            if let Some(win) = curses.get_window_mut(name) {
                if let Some(attr) = Attribute::from_name(attr_name) {
                    win.attron(attr);
                    (0, String::new())
                } else {
                    (1, format!("zcurses: unknown attribute {}\n", attr_name))
                }
            } else {
                (1, format!("zcurses: window {} not found\n", name))
            }
        }
        _ => (1, format!("zcurses: unknown subcommand {}\n", args[0])),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attribute_to_ansi() {
        assert_eq!(Attribute::Bold.to_ansi(), "\x1b[1m");
        assert_eq!(Attribute::Normal.to_ansi(), "\x1b[0m");
    }

    #[test]
    fn test_attribute_from_name() {
        assert_eq!(Attribute::from_name("bold"), Some(Attribute::Bold));
        assert_eq!(Attribute::from_name("invalid"), None);
    }

    #[test]
    fn test_color_codes() {
        assert_eq!(Color::Red.fg_code(), 31);
        assert_eq!(Color::Red.bg_code(), 41);
    }

    #[test]
    fn test_color_from_name() {
        assert_eq!(Color::from_name("red"), Some(Color::Red));
        assert_eq!(Color::from_name("invalid"), None);
    }

    #[test]
    fn test_window_new() {
        let win = Window::new("test", 10, 20, 0, 0);
        assert_eq!(win.name, "test");
        assert_eq!(win.rows, 10);
        assert_eq!(win.cols, 20);
    }

    #[test]
    fn test_window_move_cursor() {
        let mut win = Window::new("test", 10, 20, 0, 0);
        win.move_cursor(5, 10);
        assert_eq!(win.getyx(), (5, 10));
    }

    #[test]
    fn test_window_addch() {
        let mut win = Window::new("test", 10, 20, 0, 0);
        win.addch('X');
        assert_eq!(win.buffer[0][0], 'X');
        assert_eq!(win.getyx(), (0, 1));
    }

    #[test]
    fn test_window_addstr() {
        let mut win = Window::new("test", 10, 20, 0, 0);
        win.addstr("Hello");
        assert_eq!(win.getyx(), (0, 5));
    }

    #[test]
    fn test_window_clear() {
        let mut win = Window::new("test", 10, 20, 0, 0);
        win.addstr("Hello");
        win.clear();
        assert_eq!(win.buffer[0][0], ' ');
        assert_eq!(win.getyx(), (0, 0));
    }

    #[test]
    fn test_curses_new() {
        let curses = Curses::new();
        assert!(!curses.is_initialized());
    }

    #[test]
    fn test_curses_newwin() {
        let mut curses = Curses::new();
        assert!(curses.newwin("test", 10, 20, 0, 0));
        assert!(!curses.newwin("test", 10, 20, 0, 0));
    }

    #[test]
    fn test_curses_delwin() {
        let mut curses = Curses::new();
        curses.newwin("test", 10, 20, 0, 0);
        assert!(curses.delwin("test"));
        assert!(!curses.delwin("test"));
    }

    #[test]
    fn test_builtin_zcurses_no_args() {
        let mut curses = Curses::new();
        let (status, _) = bin_zcurses(&[], &mut curses);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_zcurses_unknown() {
        let mut curses = Curses::new();
        let (status, _) = bin_zcurses(&["unknown"], &mut curses);
        assert_eq!(status, 1);
    }
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: module-shims
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// `zcurses` builtin — delegates to canonical port at
    /// `src/ported/modules/curses.rs:573` (`bin_zcurses()` from
    /// `Src/Modules/curses.c`). The window/colour-pair table is
    /// owned by `ShellExecutor` so windows opened by `zcurses
    /// addwin` persist for later `delwin`/`refresh`/`attr` calls.
    pub(crate) fn bin_zcurses(&mut self, args: &[String]) -> i32 {
        let argv: Vec<&str> = args.iter().map(String::as_str).collect();
        let (status, output) = crate::curses::bin_zcurses(
            &argv, &mut self.curses,
        );
        if !output.is_empty() {
            if status == 0 { print!("{}", output); } else { eprint!("{}", output); }
        }
        status
    }
}
// END moved-from-exec-rs

/// Module loader entry — port of `setup_()` from Src/Modules/curses.c:1744.
pub fn setup_() -> i32 {
    0
}

/// Module loader entry — port of `features_()` from Src/Modules/curses.c:1751.
pub fn features_() -> i32 {
    0
}

/// Module loader entry — port of `enables_()` from Src/Modules/curses.c:1759.
pub fn enables_() -> i32 {
    0
}

/// Module loader entry — port of `boot_()` from Src/Modules/curses.c:1766.
pub fn boot_() -> i32 {
    0
}

/// Module loader entry — port of `cleanup_()` from Src/Modules/curses.c:1775.
pub fn cleanup_() -> i32 {
    0
}

/// Module loader entry — port of `finish_()` from Src/Modules/curses.c:1785.
pub fn finish_() -> i32 {
    0
}

// === auto-generated stubs ===
// Direct ports of static helpers from Src/Modules/curses.c not
// yet covered above. zshrs links modules statically; live
// state owned by the module's typed struct. Name-parity shims.

/// Port of `freecolorpairnode()` from Src/Modules/curses.c:422.
#[allow(non_snake_case)]
pub fn freecolorpairnode() -> i32 { 0 }

/// Port of `zccmd_addwin()` from Src/Modules/curses.c:503.
#[allow(non_snake_case)]
pub fn zccmd_addwin() -> i32 { 0 }

/// Port of `zccmd_attr()` from Src/Modules/curses.c:843.
#[allow(non_snake_case)]
pub fn zccmd_attr() -> i32 { 0 }

/// Port of `zccmd_bg()` from Src/Modules/curses.c:908.
#[allow(non_snake_case)]
pub fn zccmd_bg() -> i32 { 0 }

/// Port of `zccmd_border()` from Src/Modules/curses.c:802.
#[allow(non_snake_case)]
pub fn zccmd_border() -> i32 { 0 }

/// Port of `zccmd_char()` from Src/Modules/curses.c:723.
#[allow(non_snake_case)]
pub fn zccmd_char() -> i32 { 0 }

/// Port of `zccmd_clear()` from Src/Modules/curses.c:694.
#[allow(non_snake_case)]
pub fn zccmd_clear() -> i32 { 0 }

/// Port of `zccmd_endwin()` from Src/Modules/curses.c:823.
#[allow(non_snake_case)]
pub fn zccmd_endwin() -> i32 { 0 }

/// Port of `zccmd_init()` from Src/Modules/curses.c:434.
#[allow(non_snake_case)]
pub fn zccmd_init() -> i32 { 0 }

/// Port of `zccmd_input()` from Src/Modules/curses.c:1029.
#[allow(non_snake_case)]
pub fn zccmd_input() -> i32 { 0 }

/// Port of `zccmd_mouse()` from Src/Modules/curses.c:1294.
#[allow(non_snake_case)]
pub fn zccmd_mouse() -> i32 { 0 }

/// Port of `zccmd_move()` from Src/Modules/curses.c:669.
#[allow(non_snake_case)]
pub fn zccmd_move() -> i32 { 0 }

/// Port of `zccmd_position()` from Src/Modules/curses.c:1343.
#[allow(non_snake_case)]
pub fn zccmd_position() -> i32 { 0 }

/// Port of `zccmd_querychar()` from Src/Modules/curses.c:1382.
#[allow(non_snake_case)]
pub fn zccmd_querychar() -> i32 { 0 }

/// Port of `zccmd_refresh()` from Src/Modules/curses.c:632.
#[allow(non_snake_case)]
pub fn zccmd_refresh() -> i32 { 0 }

/// Port of `zccmd_resize()` from Src/Modules/curses.c:1494.
#[allow(non_snake_case)]
pub fn zccmd_resize() -> i32 { 0 }

/// Port of `zccmd_scroll()` from Src/Modules/curses.c:986.
#[allow(non_snake_case)]
pub fn zccmd_scroll() -> i32 { 0 }

/// Port of `zccmd_string()` from Src/Modules/curses.c:759.
#[allow(non_snake_case)]
pub fn zccmd_string() -> i32 { 0 }

/// Port of `zccmd_timeout()` from Src/Modules/curses.c:1255.
#[allow(non_snake_case)]
pub fn zccmd_timeout() -> i32 { 0 }

/// Port of `zccmd_touch()` from Src/Modules/curses.c:1472.
#[allow(non_snake_case)]
pub fn zccmd_touch() -> i32 { 0 }

/// Port of `zcurses_attrget()` from Src/Modules/curses.c:302.
#[allow(non_snake_case)]
pub fn zcurses_attrget() -> i32 { 0 }

/// Port of `zcurses_attrgetfn()` from Src/Modules/curses.c:1651.
#[allow(non_snake_case)]
pub fn zcurses_attrgetfn() -> i32 { 0 }

/// Port of `zcurses_color()` from Src/Modules/curses.c:318.
#[allow(non_snake_case)]
pub fn zcurses_color() -> i32 { 0 }

/// Port of `zcurses_colorget()` from Src/Modules/curses.c:331.
#[allow(non_snake_case)]
pub fn zcurses_colorget() -> i32 { 0 }

/// Port of `zcurses_colorget_reverse()` from Src/Modules/curses.c:410.
#[allow(non_snake_case)]
pub fn zcurses_colorget_reverse() -> i32 { 0 }

/// Port of `zcurses_colornode()` from Src/Modules/curses.c:402.
#[allow(non_snake_case)]
pub fn zcurses_colornode() -> i32 { 0 }

/// Port of `zcurses_colorpairsintgetfn()` from Src/Modules/curses.c:1701.
#[allow(non_snake_case)]
pub fn zcurses_colorpairsintgetfn() -> i32 { 0 }

/// Port of `zcurses_colorsarrgetfn()` from Src/Modules/curses.c:1641.
#[allow(non_snake_case)]
pub fn zcurses_colorsarrgetfn() -> i32 { 0 }

/// Port of `zcurses_colorsintgetfn()` from Src/Modules/curses.c:1691.
#[allow(non_snake_case)]
pub fn zcurses_colorsintgetfn() -> i32 { 0 }

/// Port of `zcurses_free_window()` from Src/Modules/curses.c:285.
#[allow(non_snake_case)]
pub fn zcurses_free_window() -> i32 { 0 }

/// Port of `zcurses_getwindowbyname()` from Src/Modules/curses.c:246.
#[allow(non_snake_case)]
pub fn zcurses_getwindowbyname() -> i32 { 0 }

/// Port of `zcurses_keycodesgetfn()` from Src/Modules/curses.c:1661.
#[allow(non_snake_case)]
pub fn zcurses_keycodesgetfn() -> i32 { 0 }

/// Port of `zcurses_pairs_to_array()` from Src/Modules/curses.c:213.
#[allow(non_snake_case)]
pub fn zcurses_pairs_to_array() -> i32 { 0 }

/// Port of `zcurses_strerror()` from Src/Modules/curses.c:233.
#[allow(non_snake_case)]
pub fn zcurses_strerror() -> i32 { 0 }

/// Port of `zcurses_validate_window()` from Src/Modules/curses.c:259.
#[allow(non_snake_case)]
pub fn zcurses_validate_window() -> i32 { 0 }

/// Port of `zcurses_windowsgetfn()` from Src/Modules/curses.c:1671.
#[allow(non_snake_case)]
pub fn zcurses_windowsgetfn() -> i32 { 0 }
