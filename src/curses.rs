//! Curses module - port of Modules/curses.c
//!
//! Provides a curses windowing interface for terminal UI.
//! Uses ANSI escape sequences for portability.

use std::collections::HashMap;
use std::io::{self, Write};

/// Window attributes
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

/// Basic colors
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
            Color::White => 47,
            Color::Default => 49,
        }
    }

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

/// A curses window
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
                        self.cursor_y = self.rows - 1;
                    } else {
                        self.cursor_y = self.rows - 1;
                    }
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

    pub fn erase(&mut self) {
        self.clear();
    }

    pub fn clrtoeol(&mut self) {
        if self.cursor_y < self.rows {
            for x in self.cursor_x..self.cols {
                self.buffer[self.cursor_y][x] = ' ';
            }
        }
    }

    pub fn clrtobot(&mut self) {
        self.clrtoeol();
        for y in (self.cursor_y + 1)..self.rows {
            for x in 0..self.cols {
                self.buffer[y][x] = ' ';
            }
        }
    }

    fn scroll_up(&mut self) {
        self.buffer.remove(0);
        self.buffer.push(vec![' '; self.cols]);
    }

    pub fn set_scroll(&mut self, enable: bool) {
        self.scroll = enable;
    }

    pub fn set_keypad(&mut self, enable: bool) {
        self.keypad = enable;
    }

    pub fn attron(&mut self, attr: Attribute) {
        if !self.attrs.contains(&attr) {
            self.attrs.push(attr);
        }
    }

    pub fn attroff(&mut self, attr: Attribute) {
        self.attrs.retain(|a| *a != attr);
    }

    pub fn set_color(&mut self, fg: Color, bg: Color) {
        self.fg = fg;
        self.bg = bg;
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

    pub fn getyx(&self) -> (usize, usize) {
        (self.cursor_y, self.cursor_x)
    }

    pub fn getmaxyx(&self) -> (usize, usize) {
        (self.rows, self.cols)
    }
}

/// Curses state manager
#[derive(Debug, Default)]
pub struct Curses {
    windows: HashMap<String, Window>,
    initialized: bool,
    color_pairs: HashMap<i32, (Color, Color)>,
    next_pair: i32,
}

impl Curses {
    pub fn new() -> Self {
        Self::default()
    }

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

    pub fn newwin(&mut self, name: &str, rows: usize, cols: usize, y: usize, x: usize) -> bool {
        if self.windows.contains_key(name) {
            return false;
        }

        let win = Window::new(name, rows, cols, y, x);
        self.windows.insert(name.to_string(), win);
        true
    }

    pub fn delwin(&mut self, name: &str) -> bool {
        if name == "stdscr" {
            return false;
        }
        self.windows.remove(name).is_some()
    }

    pub fn get_window(&self, name: &str) -> Option<&Window> {
        self.windows.get(name)
    }

    pub fn get_window_mut(&mut self, name: &str) -> Option<&mut Window> {
        self.windows.get_mut(name)
    }

    pub fn refresh(&self, name: &str) -> io::Result<()> {
        if let Some(win) = self.windows.get(name) {
            win.refresh()
        } else {
            Ok(())
        }
    }

    pub fn refresh_all(&self) -> io::Result<()> {
        for win in self.windows.values() {
            win.refresh()?;
        }
        Ok(())
    }

    pub fn init_pair(&mut self, pair: i32, fg: Color, bg: Color) {
        self.color_pairs.insert(pair, (fg, bg));
    }

    pub fn get_pair(&self, pair: i32) -> Option<(Color, Color)> {
        self.color_pairs.get(&pair).copied()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn window_names(&self) -> Vec<&str> {
        self.windows.keys().map(|s| s.as_str()).collect()
    }
}

/// Get terminal size
pub fn terminal_size() -> Option<(usize, usize)> {
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

/// Raw mode for input
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

#[cfg(not(unix))]
pub fn cbreak() -> io::Result<()> {
    Ok(())
}

/// Disable echo
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

#[cfg(not(unix))]
pub fn noecho() -> io::Result<()> {
    Ok(())
}

/// Hide cursor
pub fn curs_set(visible: bool) -> io::Result<()> {
    let mut stdout = io::stdout();
    if visible {
        write!(stdout, "\x1b[?25h")?;
    } else {
        write!(stdout, "\x1b[?25l")?;
    }
    stdout.flush()
}

/// Execute zcurses builtin
pub fn builtin_zcurses(args: &[&str], curses: &mut Curses) -> (i32, String) {
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
        let (status, _) = builtin_zcurses(&[], &mut curses);
        assert_eq!(status, 1);
    }

    #[test]
    fn test_builtin_zcurses_unknown() {
        let mut curses = Curses::new();
        let (status, _) = builtin_zcurses(&["unknown"], &mut curses);
        assert_eq!(status, 1);
    }
}
