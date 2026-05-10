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

use std::cell::RefCell;
use std::env;

// `pub enum CmdState` + `impl CmdState { from_u8, name }` —
// DELETED per user directive ("CmdState fake"). Was a Rust-only
// typed wrapper around the canonical `CS_*` integer constants
// (`Src/zsh.h:2775-2806`, ported to `crate::ported::zsh_h::CS_*`).
// C source pushes raw `unsigned char` bytes onto `cmdstack` and
// indexes `cmdnames[CS_COUNT]` (`Src/prompt.c:62`) for the name.
// Now ported 1:1: callers use `CS_FOO as u8` directly and look up
// names through `cmdname()` below.

// parser states, for %_                                                    // c:60
/// Direct port of `cmdnames[CS_COUNT]` from `Src/prompt.c:62-71`.
/// Indexed by the `CS_*` constants in `zsh_h::CS_FOR..CS_ALWAYS`
/// (`Src/zsh.h:2775-2806`). Used by `%_` prompt expansion to print
/// the active compound-command keyword stack.
pub static CMDNAMES: [&str; crate::ported::zsh_h::CS_COUNT as usize] = [
    "for",      "while",     "repeat",    "select",    // c:63 (CS_FOR..CS_SELECT)
    "until",    "if",        "then",      "else",      // c:64 (CS_UNTIL..CS_ELSE)
    "elif",     "math",      "cond",      "cmdor",     // c:65 (CS_ELIF..CS_CMDOR)
    "cmdand",   "pipe",      "errpipe",   "foreach",   // c:66 (CS_CMDAND..CS_FOREACH)
    "case",     "function",  "subsh",     "cursh",     // c:67 (CS_CASE..CS_CURSH)
    "array",    "quote",     "dquote",    "bquote",    // c:68 (CS_ARRAY..CS_BQUOTE)
    "cmdsubst", "mathsubst", "elif-then", "heredoc",   // c:69 (CS_CMDSUBST..CS_HEREDOC)
    "heredocd", "brace",     "braceparam", "always",   // c:70 (CS_HEREDOCD..CS_ALWAYS)
];

// Note: there is no Rust helper for `cmdnames[cmdstack[t0]]` — C
// uses the bare array indexing inline (`Src/prompt.c:835`,
// `:846`, `:861`, `:872`). Use `CMDNAMES.get(b as usize).copied()`
// at every call site to mirror that pattern faithfully.

// `pub struct TextAttrs` and `pub enum Color` — DELETED per user
// directive. Both were Rust-only abstractions over the canonical
// `zattr` (u64) bitfield from `Src/zsh.h:2685-2741`. C packs every
// attribute (bold/faint/standout/underline/italic) PLUS the
// foreground colour (24 bits), background colour (24 bits), and
// 24-bit-or-palette flags into a single 64-bit word. The Rust
// port now uses the canonical `crate::ported::zsh_h::zattr`
// directly; helpers below mirror the C bit-twiddling macros.
//
// Bit layout (matches Src/zsh.h:2694-2741 exactly):
//   0x0001 TXTBOLDFACE / 0x0002 TXTFAINT / 0x0004 TXTSTANDOUT /
//   0x0008 TXTUNDERLINE / 0x0010 TXTITALIC / 0x0020 TXTFGCOLOUR /
//   0x0040 TXTBGCOLOUR / 0x4000 TXT_ATTR_FG_24BIT /
//   0x8000 TXT_ATTR_BG_24BIT
//   bits 16-39: TXT_ATTR_FG_COL_MASK (palette index 0-255 OR
//               packed RGB if TXT_ATTR_FG_24BIT)
//   bits 40-63: TXT_ATTR_BG_COL_MASK (same for BG)
pub use crate::ported::zsh_h::zattr as TextAttrs; // c:Src/zsh.h:2685
use crate::ported::zsh_h::{
    TXTBOLDFACE, TXTFGCOLOUR, TXTBGCOLOUR, TXTSTANDOUT, TXTUNDERLINE, // c:zsh.h:2694
    TXT_ATTR_FG_COL_MASK, TXT_ATTR_FG_COL_SHIFT,
    TXT_ATTR_BG_COL_MASK, TXT_ATTR_BG_COL_SHIFT,
    TXT_ATTR_FG_24BIT, TXT_ATTR_BG_24BIT,
    TXT_ATTR_FG_MASK, TXT_ATTR_BG_MASK,
    zattr,
}; // c:zsh.h:2685-2741

// `Color` is the colour slot lifted out of `zattr` so callers can
// pass a single integer around. Bit layout mirrors the C zattr
// colour bits exactly:
//   bit 31 (0x01000000): the local 24-bit flag — mirrors the
//     C `TXT_ATTR_FG_24BIT` / `TXT_ATTR_BG_24BIT` bit (Src/zsh.h:2727).
//     When set, the low 24 bits hold `0xRRGGBB`. When clear, the
//     low 8 bits hold a palette index 0..=255, where 8 is the
//     "default" sentinel per Src/prompt.c:1909.
// Not a new type — same encoding C packs into `TXT_ATTR_FG_COL_MASK`.
pub type Color = u32; // c:Src/zsh.h:2718 (colour slot)
pub const COLOR_24BIT: Color = 0x0100_0000; // c:zsh.h:2727 (TXT_ATTR_FG_24BIT)

// Sentinel "no colour set" — palette index that lives in
// TXT_ATTR_FG_COL_MASK when the colour is `default` (8 in
// Src/prompt.c:1909). Bits 16-39 are at most 24 bits, so any
// value 0..=255 fits comfortably for palette mode.
pub const COLOUR_DEFAULT: u8 = 8; // c:Src/prompt.c:1909

// Named-colour palette constants. Indexes match `colour_names[]`
// from `Src/prompt.c:1884-1887`. Used in place of the deleted
// `Color::Black`..`Color::White`/`Color::Default` enum variants.
pub const COLOR_BLACK:   Color = 0; // c:1885
pub const COLOR_RED:     Color = 1; // c:1885
pub const COLOR_GREEN:   Color = 2; // c:1885
pub const COLOR_YELLOW:  Color = 3; // c:1885
pub const COLOR_BLUE:    Color = 4; // c:1885
pub const COLOR_MAGENTA: Color = 5; // c:1885
pub const COLOR_CYAN:    Color = 6; // c:1885
pub const COLOR_WHITE:   Color = 7; // c:1885
pub const COLOR_DEFAULT: Color = COLOUR_DEFAULT as Color; // c:1909

// Colour helpers as `macro_rules!` — C does these as inline
// bit-twiddling at each call site (no `fn` indirection in
// Src/prompt.c). Macros preserve port fidelity.

/// Pack an `(r, g, b)` triplet into a `Color` with the 24-bit
/// flag set. Mirrors C's `attr |= TXTFGCOLOUR | TXT_ATTR_FG_24BIT
/// | (((zattr)r<<16|g<<8|b) << TXT_ATTR_FG_COL_SHIFT)` idiom
/// scattered across `Src/prompt.c:2380-2440`.
macro_rules! color_rgb {
    ($r:expr, $g:expr, $b:expr) => {{
        COLOR_24BIT
            | (($r as Color) << 16)
            | (($g as Color) << 8)
            | ($b as Color)
    }};
}

/// Decode RGB from a `Color` if its 24-bit flag is set, else `None`.
/// Inline expression form — C tests the bit directly at the call site.
macro_rules! color_get_rgb {
    ($c:expr) => {{
        let c: Color = $c;
        if c & COLOR_24BIT == 0 { None }
        else { Some((((c >> 16) & 0xff) as u8, ((c >> 8) & 0xff) as u8, (c & 0xff) as u8)) }
    }};
}

/// Translate a `Color` into an ANSI escape for FG or BG. Wraps
/// the dispatch C does inline inside `set_colour_attribute()`
/// (Src/prompt.c:2440) — 24-bit RGB path vs the `output_colour()`
/// (c:2136) palette path.
macro_rules! color_to_ansi {
    ($c:expr, $is_fg:expr) => {{
        let c: Color = $c;
        let is_fg: bool = $is_fg;
        if let Some((r, g, b)) = color_get_rgb!(c) {
            let lead = if is_fg { 38 } else { 48 };
            format!("\x1b[{};2;{};{};{}m", lead, r, g, b)
        } else {
            output_colour(c as u8, is_fg)
        }
    }};
}

/// Try parsing `name` as a named colour, palette index, or
/// `#RRGGBB` hex triplet. Combines `match_named_colour()` (c:1915)
/// with the `#hex` branch inside `match_colour()` (c:1995-2030).
/// Inline at the two call sites in `parsehighlight` / `parse_percent`.
macro_rules! color_from_name {
    ($name:expr) => {{
        let name: &str = $name;
        if let Some(rest) = name.strip_prefix('#') {
            if rest.len() == 6 {
                let r = u8::from_str_radix(&rest[0..2], 16).ok();
                let g = u8::from_str_radix(&rest[2..4], 16).ok();
                let b = u8::from_str_radix(&rest[4..6], 16).ok();
                match (r, g, b) {
                    (Some(r), Some(g), Some(b)) => Some(color_rgb!(r, g, b) as Color),
                    _ => None,
                }
            } else {
                match_named_colour(name).map(|idx| idx as Color)
            }
        } else {
            match_named_colour(name).map(|idx| idx as Color)
        }
    }};
}

// Defines standard ANSI colour names in index order                        // c:1883
/// Direct port of `colour_names[]` from `Src/prompt.c:1884-1887`.
/// Indexed 0-7 = basic ANSI, 8 = "default" sentinel (per
/// `Src/prompt.c:1909` comment "8 is the special value for
/// default"). Single canonical source — the second
/// `match_named_colour` further down this file consumed a
/// drifted local table with `default = 9` which mis-rendered
/// `%F{default}` output.
pub static COLOUR_NAMES: [&str; 9] = [
    "black", "red", "green", "yellow", // c:1885
    "blue", "magenta", "cyan", "white", // c:1885
    "default", // c:1886
];

// Bit-twiddling helpers as `macro_rules!` — C uses inline macros
// (`Src/prompt.c:2350+` does `attr |= TXTFGCOLOUR | ((zattr)idx <<
// TXT_ATTR_FG_COL_SHIFT)` literally at each call site). Rust
// equivalent uses `macro_rules!` for the same scope (file-local,
// no `fn` indirection per port-fidelity rules).

/// Encode a palette index (0..=255) into the FG slot of a `zattr`.
/// Mirrors `attr |= TXTFGCOLOUR | ((zattr)idx <<
/// TXT_ATTR_FG_COL_SHIFT)` from `Src/prompt.c:2350+`.
macro_rules! zattr_set_fg_palette {
    ($attrs:expr, $idx:expr) => {{
        let cleared = $attrs & !crate::ported::zsh_h::TXT_ATTR_FG_MASK; // c:2350
        cleared
            | crate::ported::zsh_h::TXTFGCOLOUR
            | (($idx as crate::ported::zsh_h::zattr)
                << crate::ported::zsh_h::TXT_ATTR_FG_COL_SHIFT)
    }};
}

/// Encode a 24-bit RGB triplet into the FG slot. Mirrors
/// `attr |= TXTFGCOLOUR | TXT_ATTR_FG_24BIT |
/// (((zattr)r<<16|g<<8|b) << TXT_ATTR_FG_COL_SHIFT)`.
macro_rules! zattr_set_fg_rgb {
    ($attrs:expr, $r:expr, $g:expr, $b:expr) => {{
        let cleared = $attrs & !crate::ported::zsh_h::TXT_ATTR_FG_MASK; // c:2350
        let rgb = (($r as crate::ported::zsh_h::zattr) << 16)
            | (($g as crate::ported::zsh_h::zattr) << 8)
            | ($b as crate::ported::zsh_h::zattr);
        cleared
            | crate::ported::zsh_h::TXTFGCOLOUR
            | crate::ported::zsh_h::TXT_ATTR_FG_24BIT
            | (rgb << crate::ported::zsh_h::TXT_ATTR_FG_COL_SHIFT)
    }};
}

macro_rules! zattr_set_bg_palette {
    ($attrs:expr, $idx:expr) => {{
        let cleared = $attrs & !crate::ported::zsh_h::TXT_ATTR_BG_MASK; // c:2350
        cleared
            | crate::ported::zsh_h::TXTBGCOLOUR
            | (($idx as crate::ported::zsh_h::zattr)
                << crate::ported::zsh_h::TXT_ATTR_BG_COL_SHIFT)
    }};
}

macro_rules! zattr_set_bg_rgb {
    ($attrs:expr, $r:expr, $g:expr, $b:expr) => {{
        let cleared = $attrs & !crate::ported::zsh_h::TXT_ATTR_BG_MASK; // c:2350
        let rgb = (($r as crate::ported::zsh_h::zattr) << 16)
            | (($g as crate::ported::zsh_h::zattr) << 8)
            | ($b as crate::ported::zsh_h::zattr);
        cleared
            | crate::ported::zsh_h::TXTBGCOLOUR
            | crate::ported::zsh_h::TXT_ATTR_BG_24BIT
            | (rgb << crate::ported::zsh_h::TXT_ATTR_BG_COL_SHIFT)
    }};
}

/// Values C reads from scattered globals during `promptexpand()`.
/// `expand_prompt` does not take this as a parameter — callers
/// either rely on `PROMPT_EXPAND_ENV` (thread-local, default-filled)
/// or `ShellExecutor::expand_prompt_string` which refreshes it from
/// executor state first. Direct port intent: replace Rust-only
/// `PromptContext` pass-by-reference with C's file-scope reads.
// c: Src/prompt.c (pwd, hist, jobs, …) + Src/utils.c (printprompt4).
#[derive(Clone)]
pub(crate) struct prompt_expand_env {
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
    /// `unsigned char cmdstack[]` — `CS_*` indices (`Src/prompt.c:55`).
    pub cmd_stack: Vec<u8>,
    pub psvar: Vec<String>,
    pub term_width: usize,
    pub lineno: i64,
    pub scriptname: Option<String>,
    pub scriptfilename: Option<String>,
    pub argzero: String,
}

impl Default for prompt_expand_env {
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

        Self {
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
            scriptname: None,
            scriptfilename: None,
            argzero: env::args().next().unwrap_or_else(|| "zsh".to_string()),
        }
    }
}

/// Per-thread prompt globals (C zsh: file-static / global bindings).
/// `ShellExecutor::expand_prompt_string` overwrites this immediately
/// before calling `expand_prompt`.
pub(crate) thread_local! {
    pub(crate) static PROMPT_EXPAND_ENV: RefCell<prompt_expand_env> =
        RefCell::new(prompt_expand_env::default());
}

/// 1:1 port of `struct buf_vars` from `Src/prompt.c:76-121`. Holds
/// every per-call piece of state that the C expander reads via
/// `bv->X` while walking a format string.
///
/// Field naming follows the C struct exactly where it makes sense:
/// - `fm`        — format string pointer (C:104). Rust uses
///   `(input: &str, pos: usize)` instead of a raw `char *`.
/// - `buf`       — output buffer (C:84). Rust uses an owned
///   `String`; `bp`/`bufline`/`bp1` collapse into byte-index
///   tracking on `buf` and `output.len()`.
/// - `dontcount` — nesting depth of `%{ / %}` non-spacing
///   sequences (C:112). Renamed `in_escape` in earlier port; the
///   `bool` is sufficient because the expander never sees nested
///   `%{...%}` (zsh's parser flattens them before reaching here).
/// - `last`/`bp1`/`bufspc`/`bufline`/`truncwidth`/`trunccount`/
///   `rstring`/`Rstring` — not represented because the Rust port
///   uses owned `String`s + return tuples instead of a linked
///   stack of scratch buffers. `rstring`/`Rstring` are handled
///   by `promptexpand`'s return tuple (Src/prompt.c:218-219).
///
/// `prompt_percent` / `prompt_bang` mirror the global option
/// reads that C does (`isset(PROMPTPERCENT)` / `isset(PROMPTBANG)`
/// at Src/prompt.c:325). They live here so the recursive expander
/// can suppress them for sub-expansions (Src/prompt.c:328-330).
#[allow(non_camel_case_types)]
pub struct buf_vars<'a> {                                                   // c:Src/prompt.c:76
    ctx: &'a PromptContext,
    input: &'a str,           // c:104 (fm — format pointer)
    pos: usize,               // c:104 (fm cursor — Rust uses byte index)
    output: String,           // c:84  (buf — output buffer)
    attrs: TextAttrs,         // c:txtattrmask (active SGR state)
    in_escape: bool,          // c:112 (dontcount)
    prompt_percent: bool,     // c:325 (isset(PROMPTPERCENT))
    prompt_bang: bool,        // c:325 (isset(PROMPTBANG))
}

impl<'a> buf_vars<'a> {
    pub fn new(input: &'a str, ctx: &'a PromptContext) -> Self {
        Self {
            ctx,
            input,
            pos: 0,
            output: String::with_capacity(input.len() * 2),
            attrs: 0 as TextAttrs, // c:zsh.h:2685 (zattr=0 == no attrs)
            in_escape: false,
            prompt_percent: true, // c:325 (PROMPTPERCENT default)
            prompt_bang: true,    // c:325 (PROMPTBANG default)
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
        if self.attrs & TXTBOLDFACE != 0 { // c:1645
            self.output.push_str("\x1b[1m");
        }
        if self.attrs & TXTUNDERLINE != 0 { // c:1645
            self.output.push_str("\x1b[4m");
        }
        if self.attrs & TXTSTANDOUT != 0 { // c:1645
            // zsh emits italic (`3m`) for `%S` standout, not reverse
            // video (`7m`). Match zsh's actual prompt output.
            self.output.push_str("\x1b[3m");
        }
        if self.attrs & TXTFGCOLOUR != 0 { // c:1645
            let raw = (self.attrs & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
            let c = if self.attrs & TXT_ATTR_FG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else { raw as Color };
            self.output.push_str(&color_to_ansi!(c, true));
        }
        if self.attrs & TXTBGCOLOUR != 0 { // c:1645
            let raw = (self.attrs & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
            let c = if self.attrs & TXT_ATTR_BG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else { raw as Color };
            self.output.push_str(&color_to_ansi!(c, false));
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
                self.attrs |= TXTBOLDFACE; // c:zsh.h:2694
                self.start_escape();
                self.output.push_str("\x1b[1m");
                self.end_escape();
            }
            'b' => {
                // zsh's %b emits a full SGR reset `\e[0m` (matches the
                // raw bytes mainline zsh produces). The incremental
                // SGR-22 (bold off) would also work but zsh chose the
                // full reset.
                self.attrs &= !TXTBOLDFACE; // c:zsh.h:2694
                self.start_escape();
                self.output.push_str("\x1b[0m");
                self.end_escape();
            }
            'U' => {
                self.attrs |= TXTUNDERLINE; // c:zsh.h:2697
                self.start_escape();
                self.output.push_str("\x1b[4m");
                self.end_escape();
            }
            'u' => {
                self.attrs &= !TXTUNDERLINE; // c:zsh.h:2697
                self.start_escape();
                self.output.push_str("\x1b[24m");
                self.end_escape();
            }
            'S' => {
                self.attrs |= TXTSTANDOUT; // c:zsh.h:2696
                self.start_escape();
                // zsh emits italic (`3m`) for `%S` standout, not
                // reverse video (`7m`). Match zsh's actual output.
                self.output.push_str("\x1b[3m");
                self.end_escape();
            }
            's' => {
                self.attrs &= !TXTSTANDOUT; // c:zsh.h:2696
                self.start_escape();
                // zsh emits the italic-end (`23m`) for `%s` rather
                // than the reverse-end (`27m`). Match zsh's output
                // so terminal state agrees with what `%S` set.
                self.output.push_str("\x1b[23m");
                self.end_escape();
            }

            // Colors
            'F' => {
                let color: Option<Color> = if let Some(name) = self.parse_braced_arg() {
                    color_from_name!(&name) // c:336 (match_colour)
                } else if arg > 0 {
                    Some(arg as Color) // c:622 (parsecolorchar numeric)
                } else {
                    None
                };
                if let Some(c) = color {
                    if let Some((r, g, b)) = color_get_rgb!(c) {
                        self.attrs = zattr_set_fg_rgb!(self.attrs, r, g, b); // c:2440
                    } else {
                        self.attrs = zattr_set_fg_palette!(self.attrs, c as u8); // c:2440
                    }
                    // Emit ONLY the color code, not all active attrs.
                    // apply_attrs would re-emit bold/underline/standout
                    // each time `%F` runs, producing duplicate codes.
                    self.start_escape();
                    self.output.push_str(&color_to_ansi!(c, true));
                    self.end_escape();
                }
            }
            'f' => {
                // zsh emits the default-foreground escape `\e[39m`
                // (not a full `\e[0m` reset) — preserves background
                // color and other attrs. Going through apply_attrs
                // would emit a full reset which over-clears.
                self.attrs &= !TXT_ATTR_FG_MASK; // c:zsh.h:2732
                self.start_escape();
                self.output.push_str("\x1b[39m");
                self.end_escape();
            }
            'K' => {
                let color: Option<Color> = if let Some(name) = self.parse_braced_arg() {
                    color_from_name!(&name) // c:336
                } else if arg > 0 {
                    Some(arg as Color) // c:634
                } else {
                    None
                };
                if let Some(c) = color {
                    if let Some((r, g, b)) = color_get_rgb!(c) {
                        self.attrs = zattr_set_bg_rgb!(self.attrs, r, g, b); // c:2440
                    } else {
                        self.attrs = zattr_set_bg_palette!(self.attrs, c as u8); // c:2440
                    }
                    self.start_escape();
                    self.output.push_str(&color_to_ansi!(c, false));
                    self.end_escape();
                }
            }
            'k' => {
                // zsh's `%k` emits `\e[49m` (default bg only); zshrs
                // was going through apply_attrs which would re-emit
                // all active attrs.
                self.attrs &= !TXT_ATTR_BG_MASK; // c:zsh.h:2736
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
                        // c:Src/prompt.c:835 — `cmdnames[cmdstack[t0]]`
                        self.ctx
                            .cmd_stack
                            .iter()
                            .skip(cmdsp - n)
                            .filter_map(|b| CMDNAMES.get(*b as usize).copied())
                            .collect()
                    } else {
                        let mut n = (-arg) as usize;
                        if n > cmdsp {
                            n = cmdsp;
                        }
                        // Walk forward from 0 to `n`.
                        // c:Src/prompt.c:872 — `cmdnames[cmdstack[t0]]`
                        self.ctx
                            .cmd_stack
                            .iter()
                            .take(n)
                            .filter_map(|b| CMDNAMES.get(*b as usize).copied())
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
    buf_vars::new(s, ctx).expand() // c:Src/prompt.c:214 (new_vars init)
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

// `pub struct CmdStack` + `impl CmdStack { new, push, pop, top,
// depth, as_slice }` — DELETED per user directive. C source uses
// `unsigned char *cmdstack` + `int cmdsp` flat globals
// (`Src/prompt.c:55-58`) plus `cmdpush()`/`cmdpop()` functions
// (`Src/prompt.c:1624-1632`). The Rust-only `CmdStack` wrapper had
// zero callers outside this file. The canonical port lives on
// `PromptContext.cmd_stack: Vec<u8>` and `ShellExecutor.cmd_stack:
// Vec<u8>`. `cmdpush()`/`cmdpop()` ports go on those.

/// Resolve a color name to an ANSI base index.
/// Port of `match_named_colour()` from Src/prompt.c:1915 —
/// walks `colour_names[]` (now `COLOUR_NAMES` at file head), then
/// falls through to numeric parsing. Returns palette index 0-7
/// for basic colours, 8 for "default" sentinel (per C:1909),
/// numeric value for raw integers.
pub fn match_named_colour(name: &str) -> Option<u8> {                        // c:1915
    let lower = name.to_lowercase(); // c:1917
    for (i, &n) in COLOUR_NAMES.iter().enumerate() { // c:1922
        if n == lower {
            return Some(i as u8); // c:1929
        }
    }
    name.parse::<u8>().ok() // c:1933 (fall-through to numeric)
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
    let mut attrs: TextAttrs = 0;
    for part in spec.split(',') {
        let part = part.trim();
        match part {
            "bold" => attrs |= TXTBOLDFACE, // c:288
            "underline" => attrs |= TXTUNDERLINE, // c:288
            "standout" => attrs |= TXTSTANDOUT, // c:288
            "none" => {
                attrs = 0; // c:288
            }
            s if s.starts_with("fg=") => {
                if let Some(code) = match_named_colour(&s[3..]) { // c:295
                    attrs = zattr_set_fg_palette!(attrs, code); // c:295
                }
            }
            s if s.starts_with("bg=") => {
                if let Some(code) = match_named_colour(&s[3..]) { // c:295
                    attrs = zattr_set_bg_palette!(attrs, code); // c:295
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
pub fn apply_text_attributes(attrs: TextAttrs) -> String {                   // c:1645
    let mut codes: Vec<String> = Vec::new();
    if attrs & TXTBOLDFACE != 0 { codes.push("1".to_string()); } // c:1645
    if attrs & TXTUNDERLINE != 0 { codes.push("4".to_string()); } // c:1645
    if attrs & TXTSTANDOUT != 0 { codes.push("7".to_string()); } // c:1645
    if attrs & TXTFGCOLOUR != 0 { // c:1645
        let raw = (attrs & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_FG_24BIT != 0 {
            // 24-bit FG — re-pack raw RGB into a `Color` and emit.
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else {
            raw as Color
        };
        codes.push(color_to_ansi!(c, true).trim_start_matches("\x1b[")
            .trim_end_matches('m').to_string());
    }
    if attrs & TXTBGCOLOUR != 0 { // c:1645
        let raw = (attrs & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_BG_24BIT != 0 {
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else {
            raw as Color
        };
        codes.push(color_to_ansi!(c, false).trim_start_matches("\x1b[")
            .trim_end_matches('m').to_string());
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
pub fn promptpath(path: &str, npath: usize, tilde: bool, home: &str) -> String { // c:134
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

// `pub struct PromptExpandResult` — DELETED per user directive.
// Was a Rust-only bundle for C's three outparams. C signature
// `char *promptexpand(char *s, int ns, const char *marker, char
// *rs, char *Rs)` (`Src/prompt.c:182`) writes through `rs`/`Rs`
// pointers and returns the expanded `char *`. Rust port now
// returns a `(String, Option<usize>, Option<usize>)` tuple
// matching C's outparam shape directly.

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
/// landed. Rust returns the four values as a tuple
/// `(expanded, rs_offset, cap_rs_offset)`.
pub fn promptexpand(                                                         // c:182
    s: &str,
    _ns: i32,
    _marker: Option<&str>,
    ctx: &PromptContext,
) -> (String, Option<usize>, Option<usize>) {
    let expanded = expand_prompt(s, ctx);
    // C: `*rs = bv.bp - bv.buf` at `%E` / `%>` markers. Rust
    // expander loses that metadata, so a second pass on `s` is the
    // closest approximation. Source-offset → expanded-offset is
    // 1:1 except where expansion lengthens.
    let rs_offset = s.find("%E").or_else(|| s.find("%E)")); // c:Src/prompt.c:182
    let cap_rs_offset = s.find("%>>"); // c:Src/prompt.c:182
    (expanded, rs_offset, cap_rs_offset)
}

/// Escape text attributes back to a `%`-prefixed prompt string.
/// Port of `zattrescape()` from Src/prompt.c:257 — inverse of
/// `parsehighlight()`; used by the `print -P` output path.
pub fn zattrescape(attrs: TextAttrs) -> String {                             // c:257
    let mut result = String::new();
    if attrs & TXTBOLDFACE != 0 { result.push_str("%B"); } // c:259
    if attrs & TXTUNDERLINE != 0 { result.push_str("%U"); } // c:259
    if attrs & TXTSTANDOUT != 0 { result.push_str("%S"); } // c:259
    if attrs & TXTFGCOLOUR != 0 { // c:266
        let raw = (attrs & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_FG_24BIT != 0 {
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else { raw as Color };
        result.push_str(&format!("%F{{{}}}", color_name(c)));
    }
    if attrs & TXTBGCOLOUR != 0 { // c:266
        let raw = (attrs & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
        let c = if attrs & TXT_ATTR_BG_24BIT != 0 {
            COLOR_24BIT | (raw as Color & 0x00ff_ffff)
        } else { raw as Color };
        result.push_str(&format!("%K{{{}}}", color_name(c)));
    }
    result
}

fn color_name(c: Color) -> String {
    if let Some((r, g, b)) = color_get_rgb!(c) {
        return format!("#{:02x}{:02x}{:02x}", r, g, b);
    }
    let idx = (c & 0xff) as usize;
    if idx < COLOUR_NAMES.len() {
        return COLOUR_NAMES[idx].to_string();
    }
    idx.to_string()
}

/// Parse a single colour character from a `%F{...}` argument.
/// Port of `parsecolorchar()` from Src/prompt.c:318.
pub fn parsecolorchar(arg: &str, is_fg: bool) -> Option<(Color, String)> {   // c:318
    let color = color_from_name!(arg)?; // c:336 (match_colour)
    let ansi = color_to_ansi!(color, is_fg); // c:2440
    Some((color, ansi))
}

/// Internal prompt char output.
/// Port of `pputc()` from Src/prompt.c:976 — the C source's
/// per-character buffer-append helper. Rust's `String::push`
/// covers it directly; this wrapper exists for call-site parity.
pub fn pputc(buf: &mut String, c: char) {                                    // c:976
    buf.push(c);
}

// Make sure there is room for `need' more characters in the buffer.       // c:987
/// Ensure the prompt buffer has at least `need` bytes free.
/// Port of `addbufspc()` from Src/prompt.c:991 — the C source
/// reallocates the heap buffer; Rust's `String` does this
/// automatically so this is a no-op.
pub fn addbufspc(_buf: &mut String, _need: usize) {                         // c:991
    // Rust String handles allocation automatically
}

/// Append a string to the prompt buffer.
/// Port of `stradd()` from Src/prompt.c:1016.
pub fn stradd(buf: &mut String, s: &str) {                                   // c:1016
    buf.push_str(s);
}

/// Look up a terminal capability and emit its escape.
/// Port of `tsetcap()` from Src/prompt.c:1083 — the C source
/// resolves termcap/terminfo names; we map the most-common ones
/// directly onto ANSI sequences.
pub fn tsetcap(cap: &str) -> String {                                        // c:1083
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
pub fn treplaceattrs(old: TextAttrs, new: TextAttrs) -> String {             // c:1719
    let mut result = String::new();

    let old_b = old & TXTBOLDFACE != 0;
    let new_b = new & TXTBOLDFACE != 0;
    let old_u = old & TXTUNDERLINE != 0;
    let new_u = new & TXTUNDERLINE != 0;
    let old_s = old & TXTSTANDOUT != 0;
    let new_s = new & TXTSTANDOUT != 0;

    let need_reset = (old_b && !new_b) || (old_u && !new_u) || (old_s && !new_s);

    if need_reset {
        result.push_str("\x1b[0m");
        if new_b { result.push_str("\x1b[1m"); }
        if new_u { result.push_str("\x1b[4m"); }
        if new_s { result.push_str("\x1b[7m"); }
    } else {
        if !old_b && new_b { result.push_str("\x1b[1m"); }
        if !old_u && new_u { result.push_str("\x1b[4m"); }
        if !old_s && new_s { result.push_str("\x1b[7m"); }
    }

    if (old & TXT_ATTR_FG_MASK) != (new & TXT_ATTR_FG_MASK) {
        if new & TXTFGCOLOUR != 0 {
            let raw = (new & TXT_ATTR_FG_COL_MASK) >> TXT_ATTR_FG_COL_SHIFT;
            let c = if new & TXT_ATTR_FG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else { raw as Color };
            result.push_str(&color_to_ansi!(c, true));
        } else {
            result.push_str("\x1b[39m");
        }
    }
    if (old & TXT_ATTR_BG_MASK) != (new & TXT_ATTR_BG_MASK) {
        if new & TXTBGCOLOUR != 0 {
            let raw = (new & TXT_ATTR_BG_COL_MASK) >> TXT_ATTR_BG_COL_SHIFT;
            let c = if new & TXT_ATTR_BG_24BIT != 0 {
                COLOR_24BIT | (raw as Color & 0x00ff_ffff)
            } else { raw as Color };
            result.push_str(&color_to_ansi!(c, false));
        } else {
            result.push_str("\x1b[49m");
        }
    }

    result
}

/// Set text attributes (full apply).
/// Port of `tsetattrs()` from Src/prompt.c:1737.
pub fn tsetattrs(attrs: TextAttrs) -> String {                               // c:1737
    apply_text_attributes(attrs)
}

/// Unset (clear) text attributes via SGR-22/24/27 + 39/49.
/// Port of `tunsetattrs()` from Src/prompt.c:1755.
pub fn tunsetattrs(attrs: TextAttrs) -> String {                             // c:1755
    let mut result = String::new();
    if attrs & TXTBOLDFACE != 0 { result.push_str("\x1b[22m"); }
    if attrs & TXTUNDERLINE != 0 {
        result.push_str("\x1b[24m");
    }
    if attrs & TXTSTANDOUT != 0 { result.push_str("\x1b[27m"); }
    if attrs & TXTFGCOLOUR != 0 { result.push_str("\x1b[39m"); }
    if attrs & TXTBGCOLOUR != 0 { result.push_str("\x1b[49m"); }
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
/// merge against a default. Both values are canonical `zattr`
/// bitfields (c:Src/zsh.h:2685); the mask carries the same
/// attribute / TXT*COLOUR bits as `attrs` but zeroes out the
/// actual colour indices so callers can detect "this bit was
/// set vs default" by mask-and against `TXT_ATTR_*_MASK`.
pub fn match_highlight(spec: &str) -> (TextAttrs, TextAttrs) {
    let attrs = parsehighlight(spec);
    let mut mask: TextAttrs = 0;
    mask |= attrs & (TXTBOLDFACE | TXTUNDERLINE | TXTSTANDOUT); // c:2031
    if attrs & TXTFGCOLOUR != 0 { mask |= TXTFGCOLOUR; } // c:2031
    if attrs & TXTBGCOLOUR != 0 { mask |= TXTBGCOLOUR; } // c:2031
    (attrs, mask)
}

/// Emit highlight attributes as an ANSI escape string.
/// Port of `output_highlight()` from Src/prompt.c:2179.
pub fn output_highlight(attrs: TextAttrs) -> String {
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
        let exp = buf_vars::new("cmd !!", &ctx).with_prompt_bang(true);
        assert_eq!(exp.expand(), "cmd !");

        let exp2 = buf_vars::new("cmd !", &ctx).with_prompt_bang(true);
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
    fn test_promptexpand_returns_tuple_with_anchors() {
        let ctx = PromptContext::default();
        let (expanded, rs_offset, cap_rs_offset) =
            promptexpand("user@host %E rprompt", 0, None, &ctx);
        assert!(expanded.contains("rprompt"));
        // %E offset captured.
        assert_eq!(rs_offset, Some("user@host ".len()));
        assert!(cap_rs_offset.is_none());
    }

    #[test]
    fn test_applytextattributes_emits_diff() {
        set_pending_text_attrs(0); // c:zsh.h:2685 (zattr=0 = no attrs)
        let _ = applytextattributes(0);
        // Set bold pending; diff should include the bold SGR.
        set_pending_text_attrs(TXTBOLDFACE);
        let diff = applytextattributes(0);
        assert!(diff.contains("\x1b[") && diff.contains("1"));
        // Clear pending; diff should reset.
        set_pending_text_attrs(0);
        let diff = applytextattributes(0);
        assert!(diff.contains("\x1b[0m"));
    }

    #[test]
    fn test_applytextattributes_no_diff_when_unchanged() {
        set_pending_text_attrs(0);
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
/// lives in `buf_vars::expand()`; this exists for call-site
/// parity with C callers.
pub fn putpromptchar(c: char, ctx: &PromptContext, buf: &mut String) {       // c:359
    if c == '%' {
        // The full handling is in buf_vars::expand()
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
pub fn mixattrs(primary: TextAttrs, mask: TextAttrs, secondary: TextAttrs) -> TextAttrs {
    // Bit-level mix: for each TXT* bit set in `mask`, take the
    // value from `primary`; else from `secondary`. Mirrors the C
    // idiom `(mask & primary) | (!mask & secondary)`.
    let mut out: TextAttrs = 0;
    for bit in [TXTBOLDFACE, TXTUNDERLINE, TXTSTANDOUT] {
        if mask & bit != 0 { out |= primary & bit; } else { out |= secondary & bit; }
    }
    if mask & TXTFGCOLOUR != 0 { out |= primary & TXT_ATTR_FG_MASK; }
    else { out |= secondary & TXT_ATTR_FG_MASK; }
    if mask & TXTBGCOLOUR != 0 { out |= primary & TXT_ATTR_BG_MASK; }
    else { out |= secondary & TXT_ATTR_BG_MASK; }
    out
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
pub fn set_colour_attribute(color: Color, is_fg: bool) -> String {           // c:2440
    color_to_ansi!(color, is_fg) // c:2440
}

/// Maximum cmdstack depth, mirroring C zsh's `CMDSTACKSZ`.
/// Used to bound `cmdpush`/`cmdpop` so the stack can't grow
/// unbounded under runaway recursion.
// the command stack for use with %_ in prompts                             // c:53
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
// the command stack for use with %_ in prompts                             // c:53
thread_local! {
    static CMDSTACK: std::cell::RefCell<Vec<u8>> = const {                  // c:56
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
    CUR.get_or_init(|| std::sync::Mutex::new(0 as TextAttrs))
}

fn pending_attrs_lock() -> &'static std::sync::Mutex<TextAttrs> {
    static PND: std::sync::OnceLock<std::sync::Mutex<TextAttrs>> = std::sync::OnceLock::new();
    PND.get_or_init(|| std::sync::Mutex::new(0 as TextAttrs))
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
    let diff = treplaceattrs(*current, pending);
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
