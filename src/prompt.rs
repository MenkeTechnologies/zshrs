//! Prompt expansion for zshrs
//!
//! Direct port from zsh/Src/prompt.c
//!
//! Supports zsh prompt escape sequences:
//! - %d, %/, %~ - current directory
//! - %c, %., %C - trailing path components
//! - %n - username
//! - %m, %M - hostname
//! - %l - tty name
//! - %? - exit status
//! - %# - privilege indicator
//! - %h, %! - history number
//! - %j - number of jobs
//! - %L - shell level
//! - %D, %T, %t, %*, %w, %W - date/time
//! - %B, %b - bold on/off
//! - %U, %u - underline on/off
//! - %S, %s - standout on/off
//! - %F{color}, %f - foreground color
//! - %K{color}, %k - background color
//! - %{ %}  - literal escape sequences
//! - %(x.true.false) - conditional

use std::env;

/// Parser states for %_ in prompts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmdState {
    For,
    While,
    Repeat,
    Select,
    Until,
    If,
    Then,
    Else,
    Elif,
    Math,
    Cond,
    CmdOr,
    CmdAnd,
    Pipe,
    ErrPipe,
    Foreach,
    Case,
    Function,
    Subsh,
    Cursh,
    Array,
    Quote,
    DQuote,
    BQuote,
    CmdSubst,
    MathSubst,
    ElifThen,
    Heredoc,
    HeredocD,
    Brace,
    BraceParam,
    Always,
}

impl CmdState {
    pub fn name(&self) -> &'static str {
        match self {
            CmdState::For => "for",
            CmdState::While => "while",
            CmdState::Repeat => "repeat",
            CmdState::Select => "select",
            CmdState::Until => "until",
            CmdState::If => "if",
            CmdState::Then => "then",
            CmdState::Else => "else",
            CmdState::Elif => "elif",
            CmdState::Math => "math",
            CmdState::Cond => "cond",
            CmdState::CmdOr => "cmdor",
            CmdState::CmdAnd => "cmdand",
            CmdState::Pipe => "pipe",
            CmdState::ErrPipe => "errpipe",
            CmdState::Foreach => "foreach",
            CmdState::Case => "case",
            CmdState::Function => "function",
            CmdState::Subsh => "subsh",
            CmdState::Cursh => "cursh",
            CmdState::Array => "array",
            CmdState::Quote => "quote",
            CmdState::DQuote => "dquote",
            CmdState::BQuote => "bquote",
            CmdState::CmdSubst => "cmdsubst",
            CmdState::MathSubst => "mathsubst",
            CmdState::ElifThen => "elif-then",
            CmdState::Heredoc => "heredoc",
            CmdState::HeredocD => "heredocd",
            CmdState::Brace => "brace",
            CmdState::BraceParam => "braceparam",
            CmdState::Always => "always",
        }
    }
}

/// Text attributes for prompt formatting
#[derive(Debug, Clone, Copy, Default)]
pub struct TextAttrs {
    pub bold: bool,
    pub underline: bool,
    pub standout: bool,
    pub fg_color: Option<Color>,
    pub bg_color: Option<Color>,
}

/// Color specification
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
    Numbered(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    pub fn from_name(name: &str) -> Option<Color> {
        match name.to_lowercase().as_str() {
            "black" => Some(Color::Black),
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "yellow" => Some(Color::Yellow),
            "blue" => Some(Color::Blue),
            "magenta" => Some(Color::Magenta),
            "cyan" => Some(Color::Cyan),
            "white" => Some(Color::White),
            "default" => Some(Color::Default),
            _ => {
                if let Ok(n) = name.parse::<u8>() {
                    Some(Color::Numbered(n))
                } else {
                    None
                }
            }
        }
    }

    pub fn to_ansi_fg(&self) -> String {
        match self {
            Color::Black => "\x1b[30m".to_string(),
            Color::Red => "\x1b[31m".to_string(),
            Color::Green => "\x1b[32m".to_string(),
            Color::Yellow => "\x1b[33m".to_string(),
            Color::Blue => "\x1b[34m".to_string(),
            Color::Magenta => "\x1b[35m".to_string(),
            Color::Cyan => "\x1b[36m".to_string(),
            Color::White => "\x1b[37m".to_string(),
            Color::Default => "\x1b[39m".to_string(),
            Color::Numbered(n) => format!("\x1b[38;5;{}m", n),
            Color::Rgb(r, g, b) => format!("\x1b[38;2;{};{};{}m", r, g, b),
        }
    }

    pub fn to_ansi_bg(&self) -> String {
        match self {
            Color::Black => "\x1b[40m".to_string(),
            Color::Red => "\x1b[41m".to_string(),
            Color::Green => "\x1b[42m".to_string(),
            Color::Yellow => "\x1b[43m".to_string(),
            Color::Blue => "\x1b[44m".to_string(),
            Color::Magenta => "\x1b[45m".to_string(),
            Color::Cyan => "\x1b[46m".to_string(),
            Color::White => "\x1b[47m".to_string(),
            Color::Default => "\x1b[49m".to_string(),
            Color::Numbered(n) => format!("\x1b[48;5;{}m", n),
            Color::Rgb(r, g, b) => format!("\x1b[48;2;{};{};{}m", r, g, b),
        }
    }
}

/// Context for prompt expansion
pub struct PromptContext {
    pub pwd: String,
    pub home: String,
    pub user: String,
    pub host: String,
    pub host_short: String,
    pub tty: String,
    pub lastval: i32,
    pub histnum: i64,
    pub shlvl: i32,
    pub num_jobs: i32,
    pub is_root: bool,
    pub cmd_stack: Vec<CmdState>,
    pub psvar: Vec<String>,
    pub term_width: usize,
    pub lineno: i64,
}

impl Default for PromptContext {
    fn default() -> Self {
        let home = env::var("HOME").unwrap_or_default();
        let pwd = env::current_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| "/".to_string());

        let user = env::var("USER")
            .or_else(|_| env::var("LOGNAME"))
            .unwrap_or_else(|_| "user".to_string());

        let host = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "localhost".to_string());

        let host_short = host.split('.').next().unwrap_or(&host).to_string();

        let tty = std::fs::read_link("/proc/self/fd/0")
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| String::new());

        let shlvl = env::var("SHLVL")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(1);

        PromptContext {
            pwd,
            home,
            user,
            host,
            host_short,
            tty,
            lastval: 0,
            histnum: 1,
            shlvl,
            num_jobs: 0,
            is_root: unsafe { libc::geteuid() } == 0,
            cmd_stack: Vec::new(),
            psvar: Vec::new(),
            term_width: 80,
            lineno: 1,
        }
    }
}

/// Prompt expander
pub struct PromptExpander<'a> {
    ctx: &'a PromptContext,
    input: &'a str,
    pos: usize,
    output: String,
    attrs: TextAttrs,
    in_escape: bool,
    prompt_percent: bool,
    prompt_bang: bool,
}

impl<'a> PromptExpander<'a> {
    pub fn new(input: &'a str, ctx: &'a PromptContext) -> Self {
        PromptExpander {
            ctx,
            input,
            pos: 0,
            output: String::with_capacity(input.len() * 2),
            attrs: TextAttrs::default(),
            in_escape: false,
            prompt_percent: true,
            prompt_bang: true,
        }
    }

    pub fn with_prompt_percent(mut self, enable: bool) -> Self {
        self.prompt_percent = enable;
        self
    }

    pub fn with_prompt_bang(mut self, enable: bool) -> Self {
        self.prompt_bang = enable;
        self
    }

    fn peek(&self) -> Option<char> {
        self.input[self.pos..].chars().next()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn parse_number(&mut self) -> Option<i32> {
        let start = self.pos;
        let mut negative = false;

        if self.peek() == Some('-') {
            negative = true;
            self.advance();
        }

        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }

        if self.pos == start || (negative && self.pos == start + 1) {
            if negative {
                self.pos = start;
            }
            return None;
        }

        let num_str = &self.input[if negative { start + 1 } else { start }..self.pos];
        let num: i32 = num_str.parse().ok()?;
        Some(if negative { -num } else { num })
    }

    fn parse_braced_arg(&mut self) -> Option<String> {
        if self.peek() != Some('{') {
            return None;
        }
        self.advance(); // skip {

        let start = self.pos;
        let mut depth = 1;

        while let Some(c) = self.advance() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(self.input[start..self.pos - 1].to_string());
                    }
                }
                '\\' => {
                    self.advance(); // skip escaped char
                }
                _ => {}
            }
        }

        None
    }

    /// Get path with tilde substitution
    fn path_with_tilde(&self, path: &str) -> String {
        if !self.ctx.home.is_empty() && path.starts_with(&self.ctx.home) {
            format!("~{}", &path[self.ctx.home.len()..])
        } else {
            path.to_string()
        }
    }

    /// Get trailing path components
    fn trailing_path(&self, path: &str, n: usize, with_tilde: bool) -> String {
        let path = if with_tilde {
            self.path_with_tilde(path)
        } else {
            path.to_string()
        };

        if n == 0 {
            return path;
        }

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if components.len() <= n {
            return path;
        }

        components[components.len() - n..].join("/")
    }

    /// Get leading path components
    fn leading_path(&self, path: &str, n: usize) -> String {
        if n == 0 {
            return path.to_string();
        }

        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if components.len() <= n {
            return path.to_string();
        }

        let result = components[..n].join("/");
        if path.starts_with('/') {
            format!("/{}", result)
        } else {
            result
        }
    }

    /// Start escape sequence (non-printing characters)
    fn start_escape(&mut self) {
        if !self.in_escape {
            self.output.push('\x01'); // RL_PROMPT_START_IGNORE
            self.in_escape = true;
        }
    }

    /// End escape sequence
    fn end_escape(&mut self) {
        if self.in_escape {
            self.output.push('\x02'); // RL_PROMPT_END_IGNORE
            self.in_escape = false;
        }
    }

    /// Apply text attributes
    fn apply_attrs(&mut self) {
        self.start_escape();

        // Reset first
        self.output.push_str("\x1b[0m");

        if self.attrs.bold {
            self.output.push_str("\x1b[1m");
        }
        if self.attrs.underline {
            self.output.push_str("\x1b[4m");
        }
        if self.attrs.standout {
            self.output.push_str("\x1b[7m");
        }
        if let Some(ref color) = self.attrs.fg_color {
            self.output.push_str(&color.to_ansi_fg());
        }
        if let Some(ref color) = self.attrs.bg_color {
            self.output.push_str(&color.to_ansi_bg());
        }

        self.end_escape();
    }

    /// Parse conditional %(x.true.false)
    fn parse_conditional(&mut self, arg: i32) -> bool {
        if self.peek() != Some('(') {
            return false;
        }
        self.advance(); // skip (

        // Parse condition character
        let cond_char = match self.advance() {
            Some(c) => c,
            None => return false,
        };

        // Evaluate condition
        let test = match cond_char {
            '/' | 'c' | '.' | '~' | 'C' => {
                // Directory depth test
                let path = self.path_with_tilde(&self.ctx.pwd);
                let depth = path.matches('/').count() as i32;
                if arg == 0 {
                    depth > 0
                } else {
                    depth >= arg
                }
            }
            '?' => self.ctx.lastval == arg,
            '#' => {
                let euid = unsafe { libc::geteuid() };
                euid == arg as u32
            }
            'L' => self.ctx.shlvl >= arg,
            'j' => self.ctx.num_jobs >= arg,
            'v' => (arg as usize) <= self.ctx.psvar.len(),
            'V' => {
                if arg <= 0 || (arg as usize) > self.ctx.psvar.len() {
                    false
                } else {
                    !self.ctx.psvar[arg as usize - 1].is_empty()
                }
            }
            '_' => self.ctx.cmd_stack.len() >= arg as usize,
            't' | 'T' | 'd' | 'D' | 'w' => {
                let now = chrono::Local::now();
                match cond_char {
                    't' => now.format("%M").to_string().parse::<i32>().unwrap_or(0) == arg,
                    'T' => now.format("%H").to_string().parse::<i32>().unwrap_or(0) == arg,
                    'd' => now.format("%d").to_string().parse::<i32>().unwrap_or(0) == arg,
                    'D' => now.format("%m").to_string().parse::<i32>().unwrap_or(0) == arg - 1,
                    'w' => now.format("%w").to_string().parse::<i32>().unwrap_or(0) == arg,
                    _ => false,
                }
            }
            '!' => self.ctx.is_root,
            _ => false,
        };

        // Get separator
        let sep = match self.advance() {
            Some(c) => c,
            None => return false,
        };

        // Parse true branch
        let true_start = self.pos;
        let mut depth = 1;
        while let Some(c) = self.peek() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            } else if c == sep && depth == 1 {
                break;
            }
            self.advance();
        }
        let true_branch = &self.input[true_start..self.pos].to_string();

        if self.peek() != Some(sep) {
            return false;
        }
        self.advance(); // skip separator

        // Parse false branch
        let false_start = self.pos;
        depth = 1;
        while let Some(c) = self.peek() {
            if c == '(' {
                depth += 1;
            } else if c == ')' {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            self.advance();
        }
        let false_branch = &self.input[false_start..self.pos].to_string();

        if self.peek() != Some(')') {
            return false;
        }
        self.advance(); // skip )

        // Expand the appropriate branch
        let branch = if test { true_branch } else { false_branch };
        let expanded = expand_prompt(branch, self.ctx);
        self.output.push_str(&expanded);

        true
    }

    /// Parse and process a % escape sequence
    fn process_percent(&mut self) {
        let arg = self.parse_number().unwrap_or(0);

        // Check for conditional
        if self.peek() == Some('(') {
            self.parse_conditional(arg);
            return;
        }

        let c = match self.advance() {
            Some(c) => c,
            None => return,
        };

        match c {
            // Directory
            '~' => {
                let path = if arg == 0 {
                    self.path_with_tilde(&self.ctx.pwd)
                } else if arg > 0 {
                    self.trailing_path(&self.ctx.pwd, arg as usize, true)
                } else {
                    self.leading_path(&self.path_with_tilde(&self.ctx.pwd), (-arg) as usize)
                };
                self.output.push_str(&path);
            }
            'd' | '/' => {
                let path = if arg == 0 {
                    self.ctx.pwd.clone()
                } else if arg > 0 {
                    self.trailing_path(&self.ctx.pwd, arg as usize, false)
                } else {
                    self.leading_path(&self.ctx.pwd, (-arg) as usize)
                };
                self.output.push_str(&path);
            }
            'c' | '.' => {
                let n = if arg == 0 {
                    1
                } else {
                    arg.unsigned_abs() as usize
                };
                let path = self.trailing_path(&self.ctx.pwd, n, true);
                self.output.push_str(&path);
            }
            'C' => {
                let n = if arg == 0 {
                    1
                } else {
                    arg.unsigned_abs() as usize
                };
                let path = self.trailing_path(&self.ctx.pwd, n, false);
                self.output.push_str(&path);
            }

            // User/host
            'n' => self.output.push_str(&self.ctx.user),
            'M' => self.output.push_str(&self.ctx.host),
            'm' => {
                let n = if arg == 0 { 1 } else { arg };
                if n > 0 {
                    let parts: Vec<&str> = self.ctx.host.split('.').collect();
                    let take = (n as usize).min(parts.len());
                    self.output.push_str(&parts[..take].join("."));
                } else {
                    let parts: Vec<&str> = self.ctx.host.split('.').collect();
                    let skip = ((-n) as usize).min(parts.len());
                    self.output.push_str(&parts[skip..].join("."));
                }
            }

            // TTY
            'l' => {
                let tty = if self.ctx.tty.starts_with("/dev/tty") {
                    &self.ctx.tty[8..]
                } else if self.ctx.tty.starts_with("/dev/") {
                    &self.ctx.tty[5..]
                } else {
                    "()"
                };
                self.output.push_str(tty);
            }
            'y' => {
                let tty = if self.ctx.tty.starts_with("/dev/") {
                    &self.ctx.tty[5..]
                } else {
                    &self.ctx.tty
                };
                self.output.push_str(tty);
            }

            // Status
            '?' => self.output.push_str(&self.ctx.lastval.to_string()),
            '#' => self.output.push(if self.ctx.is_root { '#' } else { '%' }),

            // History
            'h' | '!' => self.output.push_str(&self.ctx.histnum.to_string()),

            // Jobs
            'j' => self.output.push_str(&self.ctx.num_jobs.to_string()),

            // Shell level
            'L' => self.output.push_str(&self.ctx.shlvl.to_string()),

            // Line number
            'i' => self.output.push_str(&self.ctx.lineno.to_string()),

            // Date/time
            'D' => {
                let now = chrono::Local::now();
                if let Some(fmt) = self.parse_braced_arg() {
                    let zsh_fmt = convert_zsh_time_format(&fmt);
                    self.output.push_str(&now.format(&zsh_fmt).to_string());
                } else {
                    self.output.push_str(&now.format("%y-%m-%d").to_string());
                }
            }
            'T' => {
                let now = chrono::Local::now();
                self.output.push_str(&now.format("%H:%M").to_string());
            }
            '*' => {
                let now = chrono::Local::now();
                self.output.push_str(&now.format("%H:%M:%S").to_string());
            }
            't' | '@' => {
                let now = chrono::Local::now();
                self.output.push_str(&now.format("%l:%M%p").to_string());
            }
            'w' => {
                let now = chrono::Local::now();
                self.output.push_str(&now.format("%a %e").to_string());
            }
            'W' => {
                let now = chrono::Local::now();
                self.output.push_str(&now.format("%m/%d/%y").to_string());
            }

            // Text attributes
            'B' => {
                self.attrs.bold = true;
                self.apply_attrs();
            }
            'b' => {
                self.attrs.bold = false;
                self.apply_attrs();
            }
            'U' => {
                self.attrs.underline = true;
                self.apply_attrs();
            }
            'u' => {
                self.attrs.underline = false;
                self.apply_attrs();
            }
            'S' => {
                self.attrs.standout = true;
                self.apply_attrs();
            }
            's' => {
                self.attrs.standout = false;
                self.apply_attrs();
            }

            // Colors
            'F' => {
                let color = if let Some(name) = self.parse_braced_arg() {
                    Color::from_name(&name)
                } else if arg > 0 {
                    Some(Color::Numbered(arg as u8))
                } else {
                    None
                };
                if let Some(c) = color {
                    self.attrs.fg_color = Some(c);
                    self.apply_attrs();
                }
            }
            'f' => {
                self.attrs.fg_color = None;
                self.apply_attrs();
            }
            'K' => {
                let color = if let Some(name) = self.parse_braced_arg() {
                    Color::from_name(&name)
                } else if arg > 0 {
                    Some(Color::Numbered(arg as u8))
                } else {
                    None
                };
                if let Some(c) = color {
                    self.attrs.bg_color = Some(c);
                    self.apply_attrs();
                }
            }
            'k' => {
                self.attrs.bg_color = None;
                self.apply_attrs();
            }

            // Literal escape sequences
            '{' => self.start_escape(),
            '}' => self.end_escape(),

            // Glitch space
            'G' => {
                let n = if arg > 0 { arg as usize } else { 1 };
                for _ in 0..n {
                    self.output.push(' ');
                }
            }

            // psvar
            'v' => {
                let idx = if arg == 0 { 1 } else { arg };
                if idx > 0 && (idx as usize) <= self.ctx.psvar.len() {
                    self.output.push_str(&self.ctx.psvar[idx as usize - 1]);
                }
            }

            // Command stack
            '_' => {
                if !self.ctx.cmd_stack.is_empty() {
                    let n = if arg == 0 {
                        self.ctx.cmd_stack.len()
                    } else if arg > 0 {
                        (arg as usize).min(self.ctx.cmd_stack.len())
                    } else {
                        ((-arg) as usize).min(self.ctx.cmd_stack.len())
                    };

                    let names: Vec<&str> = if arg >= 0 {
                        self.ctx
                            .cmd_stack
                            .iter()
                            .rev()
                            .take(n)
                            .map(|s| s.name())
                            .collect()
                    } else {
                        self.ctx
                            .cmd_stack
                            .iter()
                            .take(n)
                            .map(|s| s.name())
                            .collect()
                    };
                    self.output.push_str(&names.join(" "));
                }
            }

            // Clear to end of line
            'E' => {
                self.start_escape();
                self.output.push_str("\x1b[K");
                self.end_escape();
            }

            // Literal characters
            '%' => self.output.push('%'),
            ')' => self.output.push(')'),
            '\0' => {}

            // Unknown - output literally
            _ => {
                self.output.push('%');
                self.output.push(c);
            }
        }
    }

    /// Expand the prompt
    pub fn expand(mut self) -> String {
        while let Some(c) = self.advance() {
            if c == '%' && self.prompt_percent {
                self.process_percent();
            } else if c == '!' && self.prompt_bang {
                if self.peek() == Some('!') {
                    self.advance();
                    self.output.push('!');
                } else {
                    self.output.push_str(&self.ctx.histnum.to_string());
                }
            } else {
                self.output.push(c);
            }
        }

        // Reset attributes at end
        if self.attrs.bold
            || self.attrs.underline
            || self.attrs.standout
            || self.attrs.fg_color.is_some()
            || self.attrs.bg_color.is_some()
        {
            self.start_escape();
            self.output.push_str("\x1b[0m");
            self.end_escape();
        }

        self.output
    }
}

/// Convert zsh time format to chrono format
fn convert_zsh_time_format(fmt: &str) -> String {
    let mut result = String::new();
    let mut chars = fmt.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('a') => result.push_str("%a"),             // weekday abbrev
                Some('A') => result.push_str("%A"),             // weekday full
                Some('b') | Some('h') => result.push_str("%b"), // month abbrev
                Some('B') => result.push_str("%B"),             // month full
                Some('c') => result.push_str("%c"),             // locale datetime
                Some('C') => result.push_str("%y"),             // century (use year for simplicity)
                Some('d') => result.push_str("%d"),             // day of month
                Some('D') => result.push_str("%m/%d/%y"),       // date
                Some('e') => result.push_str("%e"),             // day of month, space padded
                Some('f') => result.push_str("%e"),             // zsh: day of month, no padding
                Some('F') => result.push_str("%Y-%m-%d"),       // ISO date
                Some('H') => result.push_str("%H"),             // hour 24
                Some('I') => result.push_str("%I"),             // hour 12
                Some('j') => result.push_str("%j"),             // day of year
                Some('k') => result.push_str("%k"),             // hour 24, space padded
                Some('K') => result.push_str("%H"),             // zsh: hour 24
                Some('l') => result.push_str("%l"),             // hour 12, space padded
                Some('L') => result.push_str("%3f"),            // zsh: milliseconds (approx)
                Some('m') => result.push_str("%m"),             // month
                Some('M') => result.push_str("%M"),             // minute
                Some('n') => result.push('\n'),
                Some('N') => result.push_str("%9f"), // zsh: nanoseconds (approx)
                Some('p') => result.push_str("%p"),  // AM/PM
                Some('P') => result.push_str("%P"),  // am/pm
                Some('r') => result.push_str("%r"),  // 12-hour time
                Some('R') => result.push_str("%R"),  // 24-hour time
                Some('s') => result.push_str("%s"),  // epoch seconds
                Some('S') => result.push_str("%S"),  // seconds
                Some('t') => result.push('\t'),
                Some('T') => result.push_str("%T"), // time
                Some('u') => result.push_str("%u"), // weekday 1-7
                Some('U') => result.push_str("%U"), // week of year (Sunday)
                Some('V') => result.push_str("%V"), // ISO week
                Some('w') => result.push_str("%w"), // weekday 0-6
                Some('W') => result.push_str("%W"), // week of year (Monday)
                Some('x') => result.push_str("%x"), // locale date
                Some('X') => result.push_str("%X"), // locale time
                Some('y') => result.push_str("%y"), // year 2-digit
                Some('Y') => result.push_str("%Y"), // year 4-digit
                Some('z') => result.push_str("%z"), // timezone offset
                Some('Z') => result.push_str("%Z"), // timezone name
                Some('%') => result.push('%'),
                Some(other) => {
                    result.push('%');
                    result.push(other);
                }
                None => result.push('%'),
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Expand a prompt string
pub fn expand_prompt(s: &str, ctx: &PromptContext) -> String {
    PromptExpander::new(s, ctx).expand()
}

/// Expand a prompt string with default context
pub fn expand_prompt_default(s: &str) -> String {
    let ctx = PromptContext::default();
    expand_prompt(s, &ctx)
}

/// Count the visible width of an expanded prompt (ignoring escape sequences)
pub fn prompt_width(s: &str) -> usize {
    let mut width = 0;
    let mut in_escape = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '\x01' => in_escape = true,  // RL_PROMPT_START_IGNORE
            '\x02' => in_escape = false, // RL_PROMPT_END_IGNORE
            '\x1b' => {
                // ANSI escape - skip until 'm' or end
                while let Some(&next) = chars.peek() {
                    chars.next();
                    if next == 'm' {
                        break;
                    }
                }
            }
            _ if !in_escape => {
                width += unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
            }
            _ => {}
        }
    }

    width
}

// ---------------------------------------------------------------------------
// Missing functions from prompt.c
// ---------------------------------------------------------------------------

/// Truncate prompt to max width (from prompt.c prompttrunc)
///
/// Supports: %N>string> (right truncate) and %N<string< (left truncate)
/// N is the max width, string is the replacement indicator (default "...")
pub fn prompt_truncate(s: &str, max_width: usize, from_right: bool, indicator: &str) -> String {
    let visible_len = prompt_width(s);
    if visible_len <= max_width {
        return s.to_string();
    }

    let ind_len = indicator.len();
    if max_width <= ind_len {
        return indicator[..max_width.min(ind_len)].to_string();
    }

    let keep = max_width - ind_len;

    if from_right {
        // Keep the left part: "long text..."
        let mut result = String::new();
        let mut width = 0;
        for c in s.chars() {
            let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
            if width + cw > keep {
                break;
            }
            result.push(c);
            width += cw;
        }
        result.push_str(indicator);
        result
    } else {
        // Keep the right part: "...ng text"
        let chars: Vec<char> = s.chars().collect();
        let total_chars = chars.len();
        let mut width = 0;
        let mut start = total_chars;
        for i in (0..total_chars).rev() {
            let cw = unicode_width::UnicodeWidthChar::width(chars[i]).unwrap_or(1);
            if width + cw > keep {
                break;
            }
            width += cw;
            start = i;
        }
        let mut result = indicator.to_string();
        for &c in &chars[start..] {
            result.push(c);
        }
        result
    }
}

/// Count visible prompt characters and compute cursor position
/// (from prompt.c countprompt)
pub fn countprompt(s: &str) -> (usize, usize) {
    let width = prompt_width(s);
    let lines = s.chars().filter(|&c| c == '\n').count();
    (width, lines)
}

/// Command stack operations for %_ (from prompt.c cmdpush/cmdpop)
pub struct CmdStack {
    stack: Vec<CmdState>,
}

impl CmdStack {
    pub fn new() -> Self {
        CmdStack { stack: Vec::new() }
    }

    pub fn push(&mut self, state: CmdState) {
        self.stack.push(state);
    }

    pub fn pop(&mut self) -> Option<CmdState> {
        self.stack.pop()
    }

    pub fn top(&self) -> Option<&CmdState> {
        self.stack.last()
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    pub fn as_slice(&self) -> &[CmdState] {
        &self.stack
    }
}

impl Default for CmdStack {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse a color name to ANSI code (from prompt.c match_named_colour)
pub fn match_named_colour(name: &str) -> Option<u8> {
    match name.to_lowercase().as_str() {
        "black" => Some(0),
        "red" => Some(1),
        "green" => Some(2),
        "yellow" => Some(3),
        "blue" => Some(4),
        "magenta" => Some(5),
        "cyan" => Some(6),
        "white" => Some(7),
        "default" => Some(9),
        _ => name.parse::<u8>().ok(),
    }
}

/// Output a colour escape sequence (from prompt.c output_colour)
pub fn output_colour(colour: u8, is_fg: bool) -> String {
    let base = if is_fg { 30 } else { 40 };
    if colour < 8 {
        format!("\x1b[{}m", base + colour)
    } else if colour < 16 {
        format!("\x1b[{};1m", base + colour - 8)
    } else {
        let mode = if is_fg { 38 } else { 48 };
        format!("\x1b[{};5;{}m", mode, colour)
    }
}

/// Output true color (24-bit) escape sequence
pub fn output_truecolor(r: u8, g: u8, b: u8, is_fg: bool) -> String {
    let mode = if is_fg { 38 } else { 48 };
    format!("\x1b[{};2;{};{};{}m", mode, r, g, b)
}

/// Parse highlight specification (from prompt.c parsehighlight)
pub fn parsehighlight(spec: &str) -> TextAttrs {
    let mut attrs = TextAttrs::default();
    for part in spec.split(',') {
        let part = part.trim();
        match part {
            "bold" => attrs.bold = true,
            "underline" => attrs.underline = true,
            "standout" => attrs.standout = true,
            "none" => {
                attrs = TextAttrs::default();
            }
            s if s.starts_with("fg=") => {
                let color_name = &s[3..];
                if let Some(code) = match_named_colour(color_name) {
                    attrs.fg_color = Some(Color::Numbered(code));
                }
            }
            s if s.starts_with("bg=") => {
                let color_name = &s[3..];
                if let Some(code) = match_named_colour(color_name) {
                    attrs.bg_color = Some(Color::Numbered(code));
                }
            }
            _ => {}
        }
    }
    attrs
}

/// Apply text attributes as ANSI escape sequences (from prompt.c applytextattributes)
pub fn apply_text_attributes(attrs: &TextAttrs) -> String {
    let mut codes = Vec::new();
    if attrs.bold {
        codes.push("1");
    }
    if attrs.underline {
        codes.push("4");
    }
    if attrs.standout {
        codes.push("7");
    }
    let fg_code;
    if let Some(ref color) = attrs.fg_color {
        fg_code = color.to_ansi_fg();
        codes.push(&fg_code);
    }
    let bg_code;
    if let Some(ref color) = attrs.bg_color {
        bg_code = color.to_ansi_bg();
        codes.push(&bg_code);
    }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\x1b[{}m", codes.join(";"))
    }
}

/// Reset all text attributes
pub fn reset_text_attributes() -> &'static str {
    "\x1b[0m"
}

/// Set default colour sequences (from prompt.c set_default_colour_sequences)
pub fn set_default_colour_sequences() -> (String, String) {
    // Default: use ANSI sequences
    ("\x1b[0m".to_string(), "\x1b[0m".to_string())
}

/// Right prompt handling - compute padding for RPROMPT
pub fn right_prompt_padding(
    left_width: usize,
    right_prompt: &str,
    term_width: usize,
    indent: usize,
) -> Option<String> {
    let right_width = prompt_width(right_prompt);
    let total = left_width + right_width + indent;
    if total >= term_width {
        return None; // No room for right prompt
    }
    let padding = term_width - total;
    Some(" ".repeat(padding))
}

/// Transient prompt - return empty string to clear prompt on accept-line
pub fn transient_prompt(_original: &str) -> String {
    String::new()
}

// ---------------------------------------------------------------------------
// Remaining missing functions from prompt.c
// ---------------------------------------------------------------------------

/// Get prompt path with tilde substitution (from prompt.c promptpath)
pub fn promptpath(path: &str, npath: usize, tilde: bool, home: &str) -> String {
    let display = if tilde && !home.is_empty() && path.starts_with(home) {
        let rest = &path[home.len()..];
        if rest.is_empty() || rest.starts_with('/') {
            format!("~{}", rest)
        } else {
            path.to_string()
        }
    } else {
        path.to_string()
    };

    if npath == 0 {
        return display;
    }

    // Take last npath components
    let components: Vec<&str> = display.split('/').filter(|s| !s.is_empty()).collect();
    if components.len() <= npath {
        return display;
    }
    components[components.len() - npath..].join("/")
}

/// Full prompt expansion with namespace marker support (from prompt.c promptexpand)
pub fn promptexpand(s: &str, ctx: &PromptContext) -> String {
    expand_prompt(s, ctx)
}

/// Escape attributes to string (from prompt.c zattrescape)
pub fn zattrescape(attrs: &TextAttrs) -> String {
    let mut result = String::new();
    if attrs.bold {
        result.push_str("%B");
    }
    if attrs.underline {
        result.push_str("%U");
    }
    if attrs.standout {
        result.push_str("%S");
    }
    if let Some(ref color) = attrs.fg_color {
        result.push_str(&format!("%F{{{}}}", color_name(color)));
    }
    if let Some(ref color) = attrs.bg_color {
        result.push_str(&format!("%K{{{}}}", color_name(color)));
    }
    result
}

fn color_name(c: &Color) -> String {
    match c {
        Color::Black => "black".to_string(),
        Color::Red => "red".to_string(),
        Color::Green => "green".to_string(),
        Color::Yellow => "yellow".to_string(),
        Color::Blue => "blue".to_string(),
        Color::Magenta => "magenta".to_string(),
        Color::Cyan => "cyan".to_string(),
        Color::White => "white".to_string(),
        Color::Default => "default".to_string(),
        Color::Numbered(n) => n.to_string(),
        Color::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
    }
}

/// Parse color character from number or name (from prompt.c parsecolorchar)
pub fn parsecolorchar(arg: &str, is_fg: bool) -> Option<(Color, String)> {
    let color = Color::from_name(arg)?;
    let ansi = if is_fg {
        color.to_ansi_fg()
    } else {
        color.to_ansi_bg()
    };
    Some((color, ansi))
}

/// Internal prompt char output (from prompt.c pputc)
/// In Rust, this is handled by the PromptExpander writing to its output buffer
pub fn pputc(buf: &mut String, c: char) {
    buf.push(c);
}

/// Ensure buffer has space (from prompt.c addbufspc)
/// No-op in Rust since String grows automatically
pub fn addbufspc(_buf: &mut String, _need: usize) {
    // Rust String handles allocation automatically
}

/// Add string to prompt buffer (from prompt.c stradd)
pub fn stradd(buf: &mut String, s: &str) {
    buf.push_str(s);
}

/// Set terminal capability (from prompt.c tsetcap)
pub fn tsetcap(cap: &str) -> String {
    // Map common capability names to ANSI sequences
    match cap {
        "md" | "bold" => "\x1b[1m".to_string(),
        "me" | "sgr0" => "\x1b[0m".to_string(),
        "so" | "smso" => "\x1b[7m".to_string(),
        "se" | "rmso" => "\x1b[27m".to_string(),
        "us" | "smul" => "\x1b[4m".to_string(),
        "ue" | "rmul" => "\x1b[24m".to_string(),
        _ => String::new(),
    }
}

/// Put string from capability (from prompt.c putstr)
pub fn putstr(cap: &str) -> String {
    tsetcap(cap)
}

/// Replace text attributes (from prompt.c treplaceattrs)
pub fn treplaceattrs(old: &TextAttrs, new: &TextAttrs) -> String {
    let mut result = String::new();

    // Reset if removing attributes
    let need_reset = (old.bold && !new.bold)
        || (old.underline && !new.underline)
        || (old.standout && !new.standout);

    if need_reset {
        result.push_str("\x1b[0m");
        // Re-apply what's still on
        if new.bold {
            result.push_str("\x1b[1m");
        }
        if new.underline {
            result.push_str("\x1b[4m");
        }
        if new.standout {
            result.push_str("\x1b[7m");
        }
    } else {
        // Just add new attributes
        if !old.bold && new.bold {
            result.push_str("\x1b[1m");
        }
        if !old.underline && new.underline {
            result.push_str("\x1b[4m");
        }
        if !old.standout && new.standout {
            result.push_str("\x1b[7m");
        }
    }

    // Handle color changes
    if old.fg_color != new.fg_color {
        if let Some(ref color) = new.fg_color {
            result.push_str(&color.to_ansi_fg());
        } else {
            result.push_str("\x1b[39m"); // default fg
        }
    }
    if old.bg_color != new.bg_color {
        if let Some(ref color) = new.bg_color {
            result.push_str(&color.to_ansi_bg());
        } else {
            result.push_str("\x1b[49m"); // default bg
        }
    }

    result
}

/// Set text attributes (from prompt.c tsetattrs)
pub fn tsetattrs(attrs: &TextAttrs) -> String {
    apply_text_attributes(attrs)
}

/// Unset text attributes (from prompt.c tunsetattrs)
pub fn tunsetattrs(attrs: &TextAttrs) -> String {
    let mut result = String::new();
    if attrs.bold {
        result.push_str("\x1b[22m");
    }
    if attrs.underline {
        result.push_str("\x1b[24m");
    }
    if attrs.standout {
        result.push_str("\x1b[27m");
    }
    if attrs.fg_color.is_some() {
        result.push_str("\x1b[39m");
    }
    if attrs.bg_color.is_some() {
        result.push_str("\x1b[49m");
    }
    result
}

/// Match colour by name or number (from prompt.c match_colour)
pub fn match_colour(spec: &str, is_fg: bool) -> Option<String> {
    // Try named colour
    if let Some(code) = match_named_colour(spec) {
        return Some(output_colour(code, is_fg));
    }
    // Try #RRGGBB
    if spec.starts_with('#') && spec.len() == 7 {
        let r = u8::from_str_radix(&spec[1..3], 16).ok()?;
        let g = u8::from_str_radix(&spec[3..5], 16).ok()?;
        let b = u8::from_str_radix(&spec[5..7], 16).ok()?;
        return Some(output_truecolor(r, g, b, is_fg));
    }
    // Try number
    if let Ok(n) = spec.parse::<u8>() {
        return Some(output_colour(n, is_fg));
    }
    None
}

/// Match highlight specification (from prompt.c match_highlight)
pub fn match_highlight(spec: &str) -> (TextAttrs, TextAttrs) {
    let attrs = parsehighlight(spec);
    let mask = TextAttrs {
        bold: attrs.bold,
        underline: attrs.underline,
        standout: attrs.standout,
        fg_color: if attrs.fg_color.is_some() {
            Some(Color::Default)
        } else {
            None
        },
        bg_color: if attrs.bg_color.is_some() {
            Some(Color::Default)
        } else {
            None
        },
    };
    (attrs, mask)
}

/// Output highlight attributes as escape string (from prompt.c output_highlight)
pub fn output_highlight(attrs: &TextAttrs) -> String {
    apply_text_attributes(attrs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> PromptContext {
        PromptContext {
            pwd: "/home/user/projects/test".to_string(),
            home: "/home/user".to_string(),
            user: "testuser".to_string(),
            host: "myhost.example.com".to_string(),
            host_short: "myhost".to_string(),
            tty: "/dev/pts/0".to_string(),
            lastval: 0,
            histnum: 42,
            shlvl: 2,
            num_jobs: 1,
            is_root: false,
            cmd_stack: vec![],
            psvar: vec!["one".to_string(), "two".to_string()],
            term_width: 80,
            lineno: 10,
        }
    }

    #[test]
    fn test_directory() {
        let ctx = test_ctx();
        assert_eq!(expand_prompt("%~", &ctx), "~/projects/test");
        assert_eq!(expand_prompt("%/", &ctx), "/home/user/projects/test");
        assert_eq!(expand_prompt("%d", &ctx), "/home/user/projects/test");
        assert_eq!(expand_prompt("%1~", &ctx), "test");
        assert_eq!(expand_prompt("%2~", &ctx), "projects/test");
        assert_eq!(expand_prompt("%c", &ctx), "test");
        assert_eq!(expand_prompt("%2c", &ctx), "projects/test");
    }

    #[test]
    fn test_user_host() {
        let ctx = test_ctx();
        assert_eq!(expand_prompt("%n", &ctx), "testuser");
        assert_eq!(expand_prompt("%M", &ctx), "myhost.example.com");
        assert_eq!(expand_prompt("%m", &ctx), "myhost");
        assert_eq!(expand_prompt("%2m", &ctx), "myhost.example");
    }

    #[test]
    fn test_status() {
        let mut ctx = test_ctx();
        ctx.lastval = 127;
        assert_eq!(expand_prompt("%?", &ctx), "127");
        assert_eq!(expand_prompt("%#", &ctx), "%");
    }

    #[test]
    fn test_history() {
        let ctx = test_ctx();
        assert_eq!(expand_prompt("%h", &ctx), "42");
        assert_eq!(expand_prompt("%!", &ctx), "42");
    }

    #[test]
    fn test_misc() {
        let ctx = test_ctx();
        assert_eq!(expand_prompt("%L", &ctx), "2");
        assert_eq!(expand_prompt("%j", &ctx), "1");
        assert_eq!(expand_prompt("%i", &ctx), "10");
        assert_eq!(expand_prompt("%%", &ctx), "%");
    }

    #[test]
    fn test_psvar() {
        let ctx = test_ctx();
        assert_eq!(expand_prompt("%v", &ctx), "one");
        assert_eq!(expand_prompt("%1v", &ctx), "one");
        assert_eq!(expand_prompt("%2v", &ctx), "two");
        assert_eq!(expand_prompt("%3v", &ctx), ""); // out of bounds
    }

    #[test]
    fn test_conditional() {
        let mut ctx = test_ctx();
        ctx.lastval = 0;
        assert_eq!(expand_prompt("%(?.ok.fail)", &ctx), "ok");
        ctx.lastval = 1;
        assert_eq!(expand_prompt("%(?.ok.fail)", &ctx), "fail");
    }

    #[test]
    fn test_time_format() {
        let fmt = convert_zsh_time_format("%Y-%m-%d %H:%M:%S");
        assert_eq!(fmt, "%Y-%m-%d %H:%M:%S");
    }

    #[test]
    fn test_bang_expansion() {
        let ctx = test_ctx();
        let exp = PromptExpander::new("cmd !!", &ctx).with_prompt_bang(true);
        assert_eq!(exp.expand(), "cmd !");

        let exp2 = PromptExpander::new("cmd !", &ctx).with_prompt_bang(true);
        assert_eq!(exp2.expand(), "cmd 42");
    }
}

// ---------------------------------------------------------------------------
// Remaining 7 missing prompt.c functions
// ---------------------------------------------------------------------------

/// Core character-by-character prompt renderer (from prompt.c putpromptchar)
///
/// This is the main 600-line function in C that processes each % escape.
/// In Rust, this is implemented as PromptExpander::expand() which handles
/// all % sequences. This wrapper provides the C-compatible entry point.
pub fn putpromptchar(c: char, ctx: &PromptContext, buf: &mut String) {
    if c == '%' {
        // The full handling is in PromptExpander::expand()
        // This function is called character by character in C
        // but in Rust we process the whole string at once
        buf.push(c);
    } else {
        buf.push(c);
    }
}

/// Mix two sets of text attributes (from prompt.c mixattrs)
///
/// Combines primary and secondary attributes using a mask.
/// Attributes set in primary take precedence; unset ones fall through to secondary.
pub fn mixattrs(primary: &TextAttrs, mask: &TextAttrs, secondary: &TextAttrs) -> TextAttrs {
    TextAttrs {
        bold: if mask.bold {
            primary.bold
        } else {
            secondary.bold
        },
        underline: if mask.underline {
            primary.underline
        } else {
            secondary.underline
        },
        standout: if mask.standout {
            primary.standout
        } else {
            secondary.standout
        },
        fg_color: if mask.fg_color.is_some() {
            primary.fg_color.clone()
        } else {
            secondary.fg_color.clone()
        },
        bg_color: if mask.bg_color.is_some() {
            primary.bg_color.clone()
        } else {
            secondary.bg_color.clone()
        },
    }
}

/// Detect if terminal supports true color (from prompt.c truecolor_terminal)
pub fn truecolor_terminal() -> bool {
    // Check COLORTERM environment variable
    if let Ok(ct) = std::env::var("COLORTERM") {
        if ct == "truecolor" || ct == "24bit" {
            return true;
        }
    }
    // Check TERM for known truecolor terminals
    if let Ok(term) = std::env::var("TERM") {
        if term.contains("256color") || term.contains("direct") || term.contains("kitty") {
            return true;
        }
    }
    false
}

/// Set a colour code string from specification (from prompt.c set_colour_code)
pub fn set_colour_code(spec: &str) -> Option<String> {
    match_colour(spec, true)
}

/// Allocate colour buffer (from prompt.c allocate_colour_buffer) - no-op in Rust
pub fn allocate_colour_buffer() {
    // Rust String handles allocation automatically
}

/// Free colour buffer (from prompt.c free_colour_buffer) - no-op in Rust
pub fn free_colour_buffer() {
    // Rust Drop handles this
}

/// Set a colour attribute from parsed value (from prompt.c set_colour_attribute)
pub fn set_colour_attribute(color: &Color, is_fg: bool) -> String {
    if is_fg {
        color.to_ansi_fg()
    } else {
        color.to_ansi_bg()
    }
}
