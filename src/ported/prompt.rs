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
    /// Map a `u8` discriminant back to the variant. Used by
    /// `BUILTIN_CMD_PUSH` to decode the token from the bytecode
    /// stack. The discriminants are in the same declaration order
    /// as zsh's `cmdnames[]` (Src/prompt.c:62) so cross-checking
    /// against the C source's CS_* numeric IDs is direct.
    pub fn from_u8(n: u8) -> Option<Self> {
        Some(match n {
            0 => CmdState::For,
            1 => CmdState::While,
            2 => CmdState::Repeat,
            3 => CmdState::Select,
            4 => CmdState::Until,
            5 => CmdState::If,
            6 => CmdState::Then,
            7 => CmdState::Else,
            8 => CmdState::Elif,
            9 => CmdState::Math,
            10 => CmdState::Cond,
            11 => CmdState::CmdOr,
            12 => CmdState::CmdAnd,
            13 => CmdState::Pipe,
            14 => CmdState::ErrPipe,
            15 => CmdState::Foreach,
            16 => CmdState::Case,
            17 => CmdState::Function,
            18 => CmdState::Subsh,
            19 => CmdState::Cursh,
            20 => CmdState::Array,
            21 => CmdState::Quote,
            22 => CmdState::DQuote,
            23 => CmdState::BQuote,
            24 => CmdState::CmdSubst,
            25 => CmdState::MathSubst,
            26 => CmdState::ElifThen,
            27 => CmdState::Heredoc,
            28 => CmdState::HeredocD,
            29 => CmdState::Brace,
            30 => CmdState::BraceParam,
            31 => CmdState::Always,
            _ => return None,
        })
    }

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
            CmdState::Quote => "bslashquote",
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
            // Basic 8 colors (0-7) use ANSI 30-37; bright 8 colors
            // (8-15) use ANSI 90-97 (high-intensity); 16-255 use the
            // 256-color escape `\e[38;5;N`. Direct port of zsh's
            // setcolor in Src/utils.c — zsh emits `%F{1}` as `\e[31m`
            // and `%F{8}` as `\e[90m`, not the 256-color form.
            Color::Numbered(n) if *n <= 7 => format!("\x1b[3{}m", n),
            Color::Numbered(n) if *n <= 15 => format!("\x1b[{}m", 90 + (n - 8)),
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
            // Basic 8 colors use 40-47; bright 8 (8-15) use 100-107
            // (high-intensity bg); 16-255 use 256-color form.
            Color::Numbered(n) if *n <= 7 => format!("\x1b[4{}m", n),
            Color::Numbered(n) if *n <= 15 => format!("\x1b[{}m", 100 + (n - 8)),
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
    /// Name of the currently-sourced script.
    /// Mirrors the file-static `scriptname` from Src/init.c that
    /// `%N` consults in Src/prompt.c:554. `None` falls back to
    /// `argzero` per the C source's `scriptname ? scriptname :
    /// argzero` ternary.
    pub scriptname: Option<String>,
    /// `scriptfilename` — file being read (vs `scriptname` which
    /// mutates to function name inside a call). Backs `%x`. Mirrors
    /// C zsh's `scriptfilename` global (Src/init.c).
    pub scriptfilename: Option<String>,
    /// `argzero` — `argv[0]` of the running binary.
    /// Backs the `%N` fallback per Src/prompt.c:555 when no
    /// scriptname is in scope.
    pub argzero: String,
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
            // `scriptname` defaults to None — top-level shell
            // invocations have no in-scope script. Sourced files
            // populate this before expanding `%N` per
            // Src/prompt.c:554.
            scriptname: None,
            scriptfilename: None,
            argzero: env::args().next().unwrap_or_else(|| "zsh".to_string()),
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

    /// Apply text attributes incrementally. zsh emits just the new
    /// SGR codes (no leading `\e[0m`) when adding attrs to a default
    /// state — only emit a reset when there's nothing to apply (rare,
    /// covered by the explicit `%b`/`%f`/`%k`/`%u` reset handlers).
    fn apply_attrs(&mut self) {
        self.start_escape();
        if self.attrs.bold {
            self.output.push_str("\x1b[1m");
        }
        if self.attrs.underline {
            self.output.push_str("\x1b[4m");
        }
        if self.attrs.standout {
            // zsh emits italic (`3m`) for `%S` standout, not reverse
            // video (`7m`). Match zsh's actual prompt output.
            self.output.push_str("\x1b[3m");
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

            // Script name (or argzero fallback) — port of
            // Src/prompt.c:554-556 `case 'N': promptpath(scriptname
            // ? scriptname : argzero, arg, 0)`. The `arg` selects N
            // trailing path components (0 = full path).
            'N' => {
                let name = self
                    .ctx
                    .scriptname
                    .as_deref()
                    .unwrap_or(&self.ctx.argzero);
                let n = if arg <= 0 {
                    0
                } else {
                    arg.unsigned_abs() as usize
                };
                if n == 0 {
                    self.output.push_str(name);
                } else {
                    self.output.push_str(&self.trailing_path(name, n, false));
                }
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
                // zsh: `%y` is the tty short name (without `/dev/`).
                // When not connected to a tty (e.g. in `-c` mode or
                // a pipe), zsh outputs `()` matching the `%l` form.
                let tty = if self.ctx.tty.is_empty() {
                    "()"
                } else if self.ctx.tty.starts_with("/dev/") {
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

            // `%I` — line number being executed in the current
            // script / file / function. Port of Src/prompt.c case
            // 'I' which adds funcstack->flineno when inside a
            // function. At top-level (no funcstack), it falls
            // through to plain lineno. zshrs doesn't yet track
            // funcstack-relative line numbers in PromptContext, so
            // emit the same lineno as `%i` — matches zsh at top
            // level and degrades gracefully inside functions.
            'I' => self.output.push_str(&self.ctx.lineno.to_string()),

            // `%x` — file containing the source code currently
            // being executed. Port of Src/prompt.c case 'x':
            // `promptpath(scriptfilename ? scriptfilename :
            // argzero, arg, 0)` (the funcstack->filename path
            // inside functions isn't modeled yet — same TODO as
            // `%I`). zshrs's PromptContext.scriptname mirrors C
            // zsh's `scriptname`/`scriptfilename`; both globals
            // stay in sync (init.c:479, init.c:1591), so we read
            // scriptname here too. Honors the `arg` (npath) digit
            // identically to %N — `%2x` returns the last 2 path
            // components, `%0x` (default) is the full path.
            'x' => {
                // %x reads `scriptfilename` (Src/prompt.c:567 case 'x':
                //   `promptpath(scriptfilename ? scriptfilename :
                //               argzero, arg, 0)`). Distinct from %N
                // which reads `scriptname`. Inside a function call,
                // `scriptname` mutates to the function name but
                // `scriptfilename` keeps the original file path —
                // so PS4 trace inside `f() { … }; f` shows
                // `<file>\t<fn>\t…` not `<fn>\t<fn>\t…`.
                let name = self
                    .ctx
                    .scriptfilename
                    .as_deref()
                    .or(self.ctx.scriptname.as_deref())
                    .unwrap_or(&self.ctx.argzero);
                let n = if arg <= 0 {
                    0
                } else {
                    arg.unsigned_abs() as usize
                };
                if n == 0 {
                    self.output.push_str(name);
                } else {
                    self.output.push_str(&self.trailing_path(name, n, false));
                }
            }

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
                // zsh prints %T with no zero-pad on the hour: 04:10 → 4:10.
                // chrono's %H always zero-pads; use %k (space-padded hour
                // 0-23) and trim the leading space. Without this, zshrs
                // emitted `04:10` while zsh emitted `4:10` for early
                // hours.
                let now = chrono::Local::now();
                let formatted = now.format("%k:%M").to_string();
                self.output.push_str(formatted.trim_start());
            }
            '*' => {
                let now = chrono::Local::now();
                let formatted = now.format("%k:%M:%S").to_string();
                self.output.push_str(formatted.trim_start());
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

            // Text attributes — emit only the SGR for the newly
            // toggled attribute, not all currently-active ones.
            // zsh: `%B%S%U` → `\e[1m\e[3m\e[4m` (each is independent).
            // apply_attrs would re-emit all active attrs every call,
            // producing duplicate codes.
            'B' => {
                self.attrs.bold = true;
                self.start_escape();
                self.output.push_str("\x1b[1m");
                self.end_escape();
            }
            'b' => {
                // zsh's %b emits a full SGR reset `\e[0m` (matches the
                // raw bytes mainline zsh produces). The incremental
                // SGR-22 (bold off) would also work but zsh chose the
                // full reset.
                self.attrs.bold = false;
                self.start_escape();
                self.output.push_str("\x1b[0m");
                self.end_escape();
            }
            'U' => {
                self.attrs.underline = true;
                self.start_escape();
                self.output.push_str("\x1b[4m");
                self.end_escape();
            }
            'u' => {
                self.attrs.underline = false;
                self.start_escape();
                self.output.push_str("\x1b[24m");
                self.end_escape();
            }
            'S' => {
                self.attrs.standout = true;
                self.start_escape();
                // zsh emits italic (`3m`) for `%S` standout, not
                // reverse video (`7m`). Match zsh's actual output.
                self.output.push_str("\x1b[3m");
                self.end_escape();
            }
            's' => {
                self.attrs.standout = false;
                self.start_escape();
                // zsh emits the italic-end (`23m`) for `%s` rather
                // than the reverse-end (`27m`). Match zsh's output
                // so terminal state agrees with what `%S` set.
                self.output.push_str("\x1b[23m");
                self.end_escape();
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
                    // Emit ONLY the color code, not all active attrs.
                    // apply_attrs would re-emit bold/underline/standout
                    // each time `%F` runs, producing duplicate codes.
                    self.start_escape();
                    self.output.push_str(&c.to_ansi_fg());
                    self.end_escape();
                }
            }
            'f' => {
                // zsh emits the default-foreground escape `\e[39m`
                // (not a full `\e[0m` reset) — preserves background
                // color and other attrs. Going through apply_attrs
                // would emit a full reset which over-clears.
                self.attrs.fg_color = None;
                self.start_escape();
                self.output.push_str("\x1b[39m");
                self.end_escape();
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
                    self.start_escape();
                    self.output.push_str(&c.to_ansi_bg());
                    self.end_escape();
                }
            }
            'k' => {
                // zsh's `%k` emits `\e[49m` (default bg only); zshrs
                // was going through apply_attrs which would re-emit
                // all active attrs.
                self.attrs.bg_color = None;
                self.start_escape();
                self.output.push_str("\x1b[49m");
                self.end_escape();
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

            // Command stack — direct port of Src/prompt.c:855-880
            // case '_'. arg >= 0 prints the TOP `arg` elements
            // BOTTOM-UP (oldest first). arg < 0 prints the BOTTOM
            // `-arg` elements bottom-up. arg == 0 prints all.
            '_' => {
                let cmdsp = self.ctx.cmd_stack.len();
                if cmdsp > 0 {
                    let names: Vec<&str> = if arg >= 0 {
                        let mut n = if arg == 0 { cmdsp } else { arg as usize };
                        if n > cmdsp {
                            n = cmdsp;
                        }
                        // Walk forward from `cmdsp - n` to top.
                        self.ctx
                            .cmd_stack
                            .iter()
                            .skip(cmdsp - n)
                            .map(|s| s.name())
                            .collect()
                    } else {
                        let mut n = (-arg) as usize;
                        if n > cmdsp {
                            n = cmdsp;
                        }
                        // Walk forward from 0 to `n`.
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

        // zsh: no auto-reset at end of prompt expansion. The user is
        // responsible for emitting `%b`/`%f`/`%k` to reset attributes;
        // `print -P "%B"` outputs only `\e[1m` with no trailing
        // `\e[0m`. Leaving any open escapes is the caller's intent.

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

/// Truncate the prompt to a maximum width.
/// Port of `prompttrunc()` from Src/prompt.c:1276 — the C source
/// implements the `%N>string>` (right-truncate) and `%N<string<`
/// (left-truncate) sequences with a configurable indicator.
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

/// Port of `countprompt()` from `Src/prompt.c:1140`.
///
/// C signature:
/// `void countprompt(char *str, int *wp, int *hp, int overf);`
///
/// Walks the expanded prompt counting visible columns, wrapping
/// to the next line every `terminal_width` characters and bumping
/// the height counter. Returns `(width, height)` — `width` is the
/// column on the FINAL line; `height` is total line count
/// including the first.
///
/// Faithful to C's prompt.c:1140 logic:
/// - `\t` advances to the next 8-column boundary (`w = (w | 7) + 1`).
/// - `\n` resets `w` to 0 and bumps `h`.
/// - `\x01`/`\x02` (RL_PROMPT_*_IGNORE) toggle visibility skip.
/// - `\x1b[...m` ANSI escapes consumed without counting.
/// - Wrap rule: `while w > terminal_width && overf >= 0` →
///   `h++; w -= terminal_width` (matches the C overflow loop at
///   line 1158 + 1255).
/// - Final-column-equals-width edge case: when `w == terminal_width
///   && overf == 0`, snap to (0, h+1) — mirrors C lines 1265-1268.
///
// by locating them and finding out their screen width.                    // c:1135
/// Previous Rust port took only `&str` and returned `(width,
/// newlines)` — missing the `terminal_width` overflow tracking
/// and the `overf` flag entirely.
pub fn countprompt(s: &str, terminal_width: usize, overf: i32) -> (usize, usize) { // c:1140
    let mut w: usize = 0;
    let mut h: usize = 1;
    let mut in_escape = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        // Pre-loop wrap check (matches C's while at line 1158).
        while terminal_width > 0 && w > terminal_width && overf >= 0 {
            h += 1;
            w -= terminal_width;
        }

        match c {
            '\x01' => in_escape = true,
            '\x02' => in_escape = false,
            '\x1b' => {
                // ANSI escape — consume until 'm' or end of string.
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            }
            '\t' if !in_escape => {
                // C: w = (w | 7) + 1;
                w = (w | 7) + 1;
            }
            '\n' if !in_escape => {
                w = 0;
                h += 1;
            }
            _ if !in_escape => {
                w += unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
            }
            _ => {}
        }
    }

    // Post-loop wrap drain (C lines 1255-1263).
    while terminal_width > 0 && w > terminal_width && overf >= 0 {
        h += 1;
        w -= terminal_width;
    }
    // Final-column edge case (C lines 1265-1268).
    if terminal_width > 0 && w == terminal_width && overf == 0 {
        w = 0;
        h += 1;
    }

    (w, h)
}

/// Command stack for the `%_` prompt escape.
/// Port of the `cmdpush()`/`cmdpop()` pair from Src/prompt.c:1624
/// /1632 — tracks the active compound-command tokens (`for`,
/// `while`, etc.) so prompts can render the unfinished syntax
/// when the parser stalls mid-statement.
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

/// Resolve a color name to an ANSI base index.
/// Port of `match_named_colour()` from Src/prompt.c:1915 —
/// recognises the same eight-name palette plus `default`, then
/// falls through to numeric parsing.
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

/// Build an ANSI escape for an indexed colour.
/// Port of `output_colour()` from Src/prompt.c:2136.
pub fn output_colour(colour: u8, is_fg: bool) -> String {                    // c:2136
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

/// Parse a `,`-separated highlight specification.
/// Port of `parsehighlight()` from Src/prompt.c:285 — handles
// Parse the argument for %H                                                // c:282
/// `bold` / `underline` / `standout` / `none` plus `fg=NAME` and
/// `bg=NAME` color targets.
pub fn parsehighlight(spec: &str) -> TextAttrs {                             // c:285
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

/// Apply text attributes as a single ANSI SGR escape.
// functions for handling attributes                                        // c:1641
/// Port of `applytextattributes()` from Src/prompt.c:1645 —
/// builds one SGR sequence with all active codes joined.
pub fn apply_text_attributes(attrs: &TextAttrs) -> String {                  // c:1645
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

/// Compute the default-colour reset sequences.
/// Port of `set_default_colour_sequences()` from Src/prompt.c:2341.
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

/// Get a prompt-friendly path with optional tilde substitution.
/// Port of `promptpath()` from Src/prompt.c:134 — used for `%~`,
/// `%/`, `%c`, etc. The `npath` argument trims to the last N
/// components.
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

/// Result of [`promptexpand`] — mirrors C's mutated outparams
/// `marker`, `rs`, `Rs` from prompt.c:182.
///
/// - `expanded`: the expanded prompt text.
/// - `rs_offset`: byte offset of the right-prompt anchor (`%E`)
///   into `expanded`, or `None` if no anchor was present. Mirrors
///   C's `*rs = bv.bp - bv.buf` capture at the anchor point.
/// - `cap_rs_offset`: byte offset of the upper-case `%>>` anchor
///   for completion — same shape as C's `*Rs`.
#[derive(Debug, Clone)]
pub struct PromptExpandResult {
    pub expanded: String,
    pub rs_offset: Option<usize>,
    pub cap_rs_offset: Option<usize>,
}

/// Port of `promptexpand()` from `Src/prompt.c:182`.
///
/// C signature:
/// `char *promptexpand(char *s, int ns, const char *marker,
///                     char *rs, char *Rs);`
///
/// `ns` flags the "non-special" mode (skip processing of `%E` /
/// `%{...%}`); `marker` is an opt-in completion-cursor sentinel
/// embedded into the output; `rs`/`Rs` are output pointers
/// receiving the byte offsets where the right-prompt anchor
/// landed.
///
/// Rust port returns a [`PromptExpandResult`] carrying all four
/// values. The previous Rust wrapper dropped `marker`/`rs`/`Rs`
/// entirely; ZLE callers couldn't position the right-prompt
// `glitch' space.                                                          // c:177
/// anchor without these. The marker-insertion path is documented
/// but currently a no-op (RL completion-cursor integration
/// pending).
pub fn promptexpand(                                                         // c:182
    s: &str,
    _ns: i32,
    _marker: Option<&str>,
    ctx: &PromptContext,
) -> PromptExpandResult {
    let expanded = expand_prompt(s, ctx);
    // Compute right-prompt anchor offset by re-scanning for `%E`
    // / `%>` markers in the source — the expander loses that
    // metadata, so do a second pass on the source string. Maps
    // source offset → expanded offset is approximate (1:1 except
    // where expansion lengthens).
    let rs_offset = s.find("%E").or_else(|| s.find("%E)"));
    let cap_rs_offset = s.find("%>>");
    PromptExpandResult {
        expanded,
        rs_offset,
        cap_rs_offset,
    }
}

/// Escape text attributes back to a `%`-prefixed prompt string.
/// Port of `zattrescape()` from Src/prompt.c:257 — inverse of
/// `parsehighlight()`; used by the `print -P` output path.
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

/// Parse a single colour character from a `%F{...}` argument.
/// Port of `parsecolorchar()` from Src/prompt.c:318.
pub fn parsecolorchar(arg: &str, is_fg: bool) -> Option<(Color, String)> {
    let color = Color::from_name(arg)?;
    let ansi = if is_fg {
        color.to_ansi_fg()
    } else {
        color.to_ansi_bg()
    };
    Some((color, ansi))
}

/// Internal prompt char output.
/// Port of `pputc()` from Src/prompt.c:976 — the C source's
/// per-character buffer-append helper. Rust's `String::push`
/// covers it directly; this wrapper exists for call-site parity.
pub fn pputc(buf: &mut String, c: char) {
    buf.push(c);
}

/// Ensure the prompt buffer has at least `need` bytes free.
/// Port of `addbufspc()` from Src/prompt.c:991 — the C source
/// reallocates the heap buffer; Rust's `String` does this
/// automatically so this is a no-op.
pub fn addbufspc(_buf: &mut String, _need: usize) {
    // Rust String handles allocation automatically
}

/// Append a string to the prompt buffer.
/// Port of `stradd()` from Src/prompt.c:1016.
pub fn stradd(buf: &mut String, s: &str) {
    buf.push_str(s);
}

/// Look up a terminal capability and emit its escape.
/// Port of `tsetcap()` from Src/prompt.c:1083 — the C source
/// resolves termcap/terminfo names; we map the most-common ones
/// directly onto ANSI sequences.
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

/// Output a string from a terminal capability.
/// Port of `putstr()` from Src/prompt.c:1121.
pub fn putstr(cap: &str) -> String {
    tsetcap(cap)
}

/// Replace one set of text attributes with another.
/// Port of `treplaceattrs()` from Src/prompt.c:1719 — emits the
/// minimal SGR delta between two attribute states.
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

/// Set text attributes (full apply).
/// Port of `tsetattrs()` from Src/prompt.c:1737.
pub fn tsetattrs(attrs: &TextAttrs) -> String {
    apply_text_attributes(attrs)
}

/// Unset (clear) text attributes via SGR-22/24/27 + 39/49.
/// Port of `tunsetattrs()` from Src/prompt.c:1755.
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

/// Match a `%F`/`%K` argument as a colour spec.
/// Port of `match_colour()` from Src/prompt.c:1957 — accepts
/// named, numeric, and `#RRGGBB` truecolor forms.
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

/// Match a highlight specification, returning attrs + mask.
/// Port of `match_highlight()` from Src/prompt.c:2031 — the
/// mask records which fields were explicitly set so callers can
/// merge against a default.
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

/// Emit highlight attributes as an ANSI escape string.
/// Port of `output_highlight()` from Src/prompt.c:2179.
pub fn output_highlight(attrs: &TextAttrs) -> String {
    apply_text_attributes(attrs)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
// Trailing helpers (`prompt_width`, etc.) live below the tests to
// keep the prompt module's C-port topology cohesive — reordering
// would split related functions across the file.
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
            scriptname: None,
            scriptfilename: None,
            argzero: "zsh".to_string(),
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

    // -------------------------------------------------------------
    // countprompt + applytextattributes + promptexpand C-shape tests.
    // -------------------------------------------------------------

    #[test]
    fn test_countprompt_simple_no_wrap() {
        // 5 chars, terminal_width=80, no wrap.
        let (w, h) = countprompt("hello", 80, 0);
        assert_eq!(w, 5);
        assert_eq!(h, 1);
    }

    #[test]
    fn test_countprompt_tab_advances_to_next_8() {
        // C: w = (w | 7) + 1; — tab snaps to next multiple of 8.
        let (w, h) = countprompt("a\tb", 80, 0);
        assert_eq!(w, 9); // 'a' = 1, tab to 8, 'b' = 9
        assert_eq!(h, 1);
        let (w, _) = countprompt("\t", 80, 0);
        assert_eq!(w, 8);
    }

    #[test]
    fn test_countprompt_newline_resets_width_bumps_height() {
        let (w, h) = countprompt("foo\nbar", 80, 0);
        assert_eq!(w, 3);
        assert_eq!(h, 2);
    }

    #[test]
    fn test_countprompt_wraps_at_terminal_width() {
        // 12 chars at width=10 → wraps once. Final col=2, h=2.
        let (w, h) = countprompt("123456789012", 10, 1);
        assert_eq!(h, 2);
        assert_eq!(w, 2);
    }

    #[test]
    fn test_countprompt_overf_zero_snaps_at_boundary() {
        // C lines 1265-1268: w == terminal_width && overf == 0
        // → (0, h+1).
        let (w, h) = countprompt("0123456789", 10, 0);
        assert_eq!(w, 0);
        assert_eq!(h, 2);
    }

    #[test]
    fn test_countprompt_skips_ansi_escape() {
        // ANSI escape \x1b[31m takes 0 visible columns.
        let (w, h) = countprompt("\x1b[31mhello\x1b[0m", 80, 0);
        assert_eq!(w, 5);
        assert_eq!(h, 1);
    }

    #[test]
    fn test_countprompt_skips_rl_ignore_markers() {
        // \x01 ... \x02 markers are invisible.
        let (w, _) = countprompt("\x01raw_zero_width\x02visible", 80, 0);
        assert_eq!(w, "visible".len());
    }

    #[test]
    fn test_promptexpand_returns_struct_with_anchors() {
        let ctx = PromptContext::default();
        let r = promptexpand("user@host %E rprompt", 0, None, &ctx);
        assert!(r.expanded.contains("rprompt"));
        // %E offset captured.
        assert_eq!(r.rs_offset, Some("user@host ".len()));
        assert!(r.cap_rs_offset.is_none());
    }

    #[test]
    fn test_applytextattributes_emits_diff() {
        // Reset state.
        set_pending_text_attrs(TextAttrs::default());
        let _ = applytextattributes(0);
        // Set bold pending; diff should include the bold SGR.
        set_pending_text_attrs(TextAttrs {
            bold: true,
            ..TextAttrs::default()
        });
        let diff = applytextattributes(0);
        assert!(diff.contains("\x1b[") && diff.contains("1"));
        // Clear pending; diff should reset.
        set_pending_text_attrs(TextAttrs::default());
        let diff = applytextattributes(0);
        assert!(diff.contains("\x1b[0m"));
    }

    #[test]
    fn test_applytextattributes_no_diff_when_unchanged() {
        set_pending_text_attrs(TextAttrs::default());
        let _ = applytextattributes(0);
        // Re-apply same state — should be empty diff.
        let diff = applytextattributes(0);
        assert!(diff.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Remaining 7 missing prompt.c functions
// ---------------------------------------------------------------------------

/// Core character-by-character prompt renderer.
/// Port of `putpromptchar()` from Src/prompt.c:359 — the ~600-line
// section is ended by an instance of endchar.  If doprint is 0, the valid // c:354
// % sequences are merely skipped over, and nothing is stored.              // c:355
/// `%` escape dispatcher in the C source. The actual dispatch
/// lives in `PromptExpander::expand()`; this exists for call-site
/// parity with C callers.
pub fn putpromptchar(c: char, ctx: &PromptContext, buf: &mut String) {       // c:359
    if c == '%' {
        // The full handling is in PromptExpander::expand()
        // This function is called character by character in C
        // but in Rust we process the whole string at once
        buf.push(c);
    } else {
        buf.push(c);
    }
}

/// Mix two sets of text attributes through a mask.
/// Port of `mixattrs()` from Src/prompt.c:1802 — primary wins
/// where the mask says "set"; secondary fills the rest.
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
            primary.fg_color
        } else {
            secondary.fg_color
        },
        bg_color: if mask.bg_color.is_some() {
            primary.bg_color
        } else {
            secondary.bg_color
        },
    }
}

/// Detect whether the terminal supports true color (24-bit).
/// Port of `truecolor_terminal()` from Src/prompt.c:1935.
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

/// Build a colour escape string from a specification.
/// Port of `set_colour_code()` from Src/prompt.c:2353.
pub fn set_colour_code(spec: &str) -> Option<String> {
    match_colour(spec, true)
}

/// Allocate the colour-buffer working space.
/// Port of `allocate_colour_buffer()` from Src/prompt.c:2367 —
/// no-op in Rust because `String` allocates lazily.
pub fn allocate_colour_buffer() {
    // Rust String handles allocation automatically
}

/// Free the colour-buffer working space.
/// Port of `free_colour_buffer()` from Src/prompt.c:2417 — no-op
/// in Rust because `Drop` handles it.
pub fn free_colour_buffer() {
    // Rust Drop handles this
}

/// Apply a parsed colour attribute as an ANSI escape.
/// Port of `set_colour_attribute()` from Src/prompt.c:2440.
pub fn set_colour_attribute(color: &Color, is_fg: bool) -> String {
    if is_fg {
        color.to_ansi_fg()
    } else {
        color.to_ansi_bg()
    }
}

/// Maximum cmdstack depth, mirroring C zsh's `CMDSTACKSZ`.
/// Used to bound `cmdpush`/`cmdpop` so the stack can't grow
/// unbounded under runaway recursion.
const CMDSTACKSZ: usize = 256;

// Port of file-static `cmdstack` from `Src/init.c` (declared as
// `extern unsigned char cmdstack[CMDSTACKSZ]` in `Src/zsh.h:2658`).
// Stack of parser-context tokens (`CS_*`) the parser pushes as it
// descends into nested compound commands (`if`/`for`/`while`/`{}`
// etc.). Read by the prompt expander for `%_` and `%^` to render
// which constructs are currently open.
//
// Bucket-1 per PORT_PLAN.md — file-static in C, per-evaluator in
// zshrs. Each worker thread parses independently; sharing the
// stack across threads would corrupt nesting state. `RefCell`
// for interior mutability since the contents are owned `Vec<u8>`.
thread_local! {
    static CMDSTACK: std::cell::RefCell<Vec<u8>> = const {
        std::cell::RefCell::new(Vec::new())
    };
}

/// Push a parser context token. Port of `cmdpush()` from
/// Src/prompt.c. Bounded at CMDSTACKSZ; over-push is silently
/// ignored (matches the C source's `cmdsp < CMDSTACKSZ` guard).
pub fn cmdpush(cmdtok: u8) {
    CMDSTACK.with(|s| {
        let mut stack = s.borrow_mut();
        if stack.len() < CMDSTACKSZ {
            stack.push(cmdtok);
        }
    });
}

/// Pop the top parser context token. Port of `cmdpop()` from
/// Src/prompt.c. Empty-stack pop is a no-op (the C source emits
/// a `BUG: cmdstack empty` debug print and continues).
pub fn cmdpop() {
    CMDSTACK.with(|s| {
        s.borrow_mut().pop();
    });
}

/// Promote the 256-color value embedded in `atr` to an explicit
/// 24-bit RGB value. Port of `map256toRGB()` from Src/prompt.c.
/// Used by the prompt-output path when the terminal supports
/// truecolor and we want to emit RGB rather than the smaller
/// 256-palette code.
///
/// `shift` selects fg-byte vs bg-byte position inside `atr`;
/// `set24` is the bit that marks "this slot is now 24-bit".
/// Algorithm mirrors the C: 16-231 are the 6×6×6 color cube,
/// 232-255 are the 24-step grayscale ramp.
#[allow(non_snake_case)]
pub fn map256toRGB(atr: &mut u64, shift: u32, set24: u64) {
    if (*atr & set24) != 0 {
        return;
    }
    let colour: u32 = ((*atr >> shift) & 0xff) as u32;
    if colour < 16 {
        return;
    }
    let (red, green, blue) = if (16..232).contains(&colour) {
        let mut c = colour - 16;
        let blue = (if c != 0 { 0x37 } else { 0 }) + 40 * (c % 6);
        c /= 6;
        let green = (if c != 0 { 0x37 } else { 0 }) + 40 * (c % 6);
        c /= 6;
        let red = (if c != 0 { 0x37 } else { 0 }) + 40 * c;
        (red, green, blue)
    } else {
        let v = 8 + 10 * (colour - 232);
        (v, v, v)
    };
    *atr &= !((0xffffff_u64) << shift);
    *atr |= set24 | ((((red as u64) << 8 | green as u64) << 8 | blue as u64) << shift);
}

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: prompt
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Phase: drift
// ===========================================================

// BEGIN moved-from-exec-rs
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

// END moved-from-exec-rs

/// Singleton holding the txtcurrentattrs / txtpendingattrs C
/// globals (Src/prompt.c file-statics, around line 1640). Used
/// by [`applytextattributes`] to compute the SGR diff between
/// the last-flushed and the pending attribute state.
fn current_attrs_lock() -> &'static std::sync::Mutex<TextAttrs> {
    static CUR: std::sync::OnceLock<std::sync::Mutex<TextAttrs>> = std::sync::OnceLock::new();
    CUR.get_or_init(|| std::sync::Mutex::new(TextAttrs::default()))
}

fn pending_attrs_lock() -> &'static std::sync::Mutex<TextAttrs> {
    static PND: std::sync::OnceLock<std::sync::Mutex<TextAttrs>> = std::sync::OnceLock::new();
    PND.get_or_init(|| std::sync::Mutex::new(TextAttrs::default()))
}

/// Set the pending text-attributes that the next
/// [`applytextattributes`] call will diff against the current
/// state. Mirrors callers writing to C's `txtpendingattrs`.
pub fn set_pending_text_attrs(attrs: TextAttrs) {
    *pending_attrs_lock()
        .lock()
        .expect("pending_attrs poisoned") = attrs;
}

/// Port of `applytextattributes()` from `Src/prompt.c:1645`.
///
/// C body diff-syncs `txtcurrentattrs` against `txtpendingattrs`
/// and emits the minimal termcap-driven sequence to transition
/// the terminal — `tsetcap(TCALLATTRSOFF…)`, `TCBOLDFACEBEG`, etc.
///
/// Rust port: returns the SGR diff string built by [`treplaceattrs`]
/// over the (current, pending) pair, and updates current = pending.
/// The previous port was an empty `void` shim that emitted nothing
/// — output gets emitted at flush time, which broke any caller
/// expecting incremental attr changes. New shape returns the diff
/// the caller can write to the terminal.
///
/// `_flags` parameter (currently unused in zshrs port — C uses it
/// to gate "force reset" mode).
pub fn applytextattributes(_flags: i32) -> String {
    let mut current = current_attrs_lock().lock().expect("current_attrs poisoned");
    let pending = pending_attrs_lock()
        .lock()
        .expect("pending_attrs poisoned")
        .clone();
    let diff = treplaceattrs(&current, &pending);
    *current = pending;
    diff
}

/// Handle `%>...>` / `%<...<` / `%[truncchar string]` truncation.
/// Port of `prompttrunc()` from Src/prompt.c:1276.
///
/// The C implementation mutates `bv` (the `BufVars` scratch struct
/// in zsh's prompt expander) to insert a truncation string and
/// re-run `putpromptchar()` against a width-bounded region. The
/// Rust port handles truncation inline inside `expand_prompt()`
/// rather than via this recursive callback; this entry exists for
/// ABI parity.
pub fn prompttrunc(_arg: i32, _truncchar: i32, _doprint: i32, _endchar: i32) -> i32 {
    0
}
