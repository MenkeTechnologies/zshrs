//! Fish-style features for zshrs - native Rust implementations.
//!
//! **zshrs-original infrastructure — no C zsh source counterpart.**
//! C zsh has no built-in syntax highlighting, autosuggestion, or
//! abbreviation support; users layer these via plugins
//! (zsh-syntax-highlighting / zsh-autosuggestions / zsh-abbr) which
//! reimplement everything as zsh script. zshrs ships them as
//! native Rust to skip the interpreter overhead.
//!
//! Code adapted from fish-shell's Rust codebase. The fish-shell
//! origin gives us a structurally-validated implementation; the
//! zshrs port keeps the same shape so tests / fuzzers from upstream
//! still apply.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

// ============================================================================
// SYNTAX HIGHLIGHTING
// ============================================================================

/// Syntax-highlight role for one source token.
/// Adapted from fish-shell's `parse_constants::HighlightRole`. No
/// C zsh counterpart — C zsh's highlighting (`zle_highlight`
/// special parameter, Src/Zle/zle_refresh.c) only colors
/// match-positions; full token-classification highlighting is a
/// fish-shell innovation we port here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum HighlightRole {
    /// `Normal` variant.
    #[default]
    Normal,
    /// `Command` variant.
    Command,
    /// `Keyword` variant.
    Keyword,
    /// `Statement` variant.
    Statement,
    /// `Param` variant.
    Param,
    /// `Option` variant.
    Option,
    /// `Comment` variant.
    Comment,
    /// `Error` variant.
    Error,
    /// `String` variant.
    String,
    /// `Escape` variant.
    Escape,
    /// `Operator` variant.
    Operator,
    /// `Redirection` variant.
    Redirection,
    /// `Path` variant.
    Path,
    /// `PathValid` variant.
    PathValid,
    /// `Autosuggestion` variant.
    Autosuggestion,
    /// `Selection` variant.
    Selection,
    /// `Search` variant.
    Search,
    /// `Variable` variant.
    Variable,
    /// `Quote` variant.
    Quote,
}

/// A highlight specification for a character
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct HighlightSpec {
    /// `foreground` field.
    pub foreground: HighlightRole,
    /// `background` field.
    pub background: HighlightRole,
    /// `valid_path` field.
    pub valid_path: bool,
    /// `force_underline` field.
    pub force_underline: bool,
}

impl HighlightSpec {
    /// `with_fg` — see implementation.
    pub fn with_fg(fg: HighlightRole) -> Self {
        Self {
            foreground: fg,
            ..Default::default()
        }
    }
}

/// ANSI color codes for highlight roles
pub fn role_to_ansi(role: HighlightRole) -> &'static str {
    match role {
        HighlightRole::Normal => "\x1b[0m",
        HighlightRole::Command => "\x1b[1;32m", // Bold green
        HighlightRole::Keyword => "\x1b[1;34m", // Bold blue
        HighlightRole::Statement => "\x1b[1;35m", // Bold magenta
        HighlightRole::Param => "\x1b[0m",      // Normal
        HighlightRole::Option => "\x1b[36m",    // Cyan
        HighlightRole::Comment => "\x1b[90m",   // Gray
        HighlightRole::Error => "\x1b[1;31m",   // Bold red
        HighlightRole::String => "\x1b[33m",    // Yellow
        HighlightRole::Escape => "\x1b[1;33m",  // Bold yellow
        HighlightRole::Operator => "\x1b[1;37m", // Bold white
        HighlightRole::Redirection => "\x1b[35m", // Magenta
        HighlightRole::Path => "\x1b[4m",       // Underline
        HighlightRole::PathValid => "\x1b[4;32m", // Underline green
        HighlightRole::Autosuggestion => "\x1b[90m", // Gray
        HighlightRole::Selection => "\x1b[7m",  // Reverse
        HighlightRole::Search => "\x1b[1;43m",  // Bold yellow bg
        HighlightRole::Variable => "\x1b[1;36m", // Bold cyan
        HighlightRole::Quote => "\x1b[33m",     // Yellow
    }
}

/// Shell keywords
const KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "while", "until", "do", "done",
    "in", "function", "select", "time", "coproc", "{", "}", "[[", "]]", "!", "foreach", "end",
    "repeat", "always",
];

/// Shell builtins (common ones)
const BUILTINS: &[&str] = &[
    "cd",
    "echo",
    "exit",
    "export",
    "alias",
    "unalias",
    "source",
    ".",
    "eval",
    "exec",
    "set",
    "unset",
    "shift",
    "return",
    "break",
    "continue",
    "read",
    "readonly",
    "declare",
    "local",
    "typeset",
    "let",
    "test",
    "[",
    "printf",
    "kill",
    "wait",
    "jobs",
    "fg",
    "bg",
    "disown",
    "trap",
    "umask",
    "ulimit",
    "hash",
    "type",
    "which",
    "builtin",
    "command",
    "enable",
    "help",
    "history",
    "fc",
    "pushd",
    "popd",
    "dirs",
    "pwd",
    "true",
    "false",
    ":",
    "getopts",
    "compgen",
    "complete",
    "compopt",
    "shopt",
    "bind",
    "autoload",
    "zmodload",
    "zstyle",
    "zle",
    "bindkey",
    "setopt",
    "unsetopt",
    "emulate",
    "whence",
    // ztest framework (src/extensions/ztest.rs) — shell-level unit
    // test builtins. Listed here so highlight_shell() colors them as
    // Command (not Error) at command position. EXT_BUILTIN_NAMES has
    // the same entries for LSP completion/hover; this hardcoded list
    // exists because highlight_shell predates EXT_BUILTIN_NAMES.
    "run_tests",
    "zassert_contains",
    "zassert_dies",
    "zassert_eq",
    "zassert_err",
    "zassert_false",
    "zassert_ge",
    "zassert_gt",
    "zassert_le",
    "zassert_lt",
    "zassert_match",
    "zassert_ne",
    "zassert_near",
    "zassert_ok",
    "zassert_true",
    "ztest_run",
    "ztest_skip",
];

/// Highlight a shell command line
/// Lex a shell line and produce per-character highlight specs.
/// Adapted from fish-shell's syntax highlighter (no C zsh
/// counterpart; this is the feature `zsh-syntax-highlighting` plugin
/// users layer on top of zsh, but native).
pub fn highlight_shell(line: &str) -> Vec<HighlightSpec> {
    let mut colors = vec![HighlightSpec::default(); line.len()];
    if line.is_empty() {
        return colors;
    }

    let mut in_string = false;
    let mut string_char = '"';
    let mut in_comment = false;
    let mut word_start: Option<usize> = None;
    let mut is_first_word = true;
    let mut after_pipe_or_semi = false;

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let byte_pos = line.char_indices().nth(i).map(|(p, _)| p).unwrap_or(0);

        // Handle comments
        if !in_string && c == '#' {
            in_comment = true;
        }
        if in_comment {
            if byte_pos < colors.len() {
                colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Comment);
            }
            i += 1;
            continue;
        }

        // Handle strings
        if !in_string && (c == '"' || c == '\'') {
            in_string = true;
            string_char = c;
            if byte_pos < colors.len() {
                colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Quote);
            }
            i += 1;
            continue;
        }
        if in_string {
            if c == string_char {
                in_string = false;
                if byte_pos < colors.len() {
                    colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Quote);
                }
            } else if c == '\\' && string_char == '"' && i + 1 < chars.len() {
                if byte_pos < colors.len() {
                    colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Escape);
                }
                i += 1;
                let next_byte = line.char_indices().nth(i).map(|(p, _)| p).unwrap_or(0);
                if next_byte < colors.len() {
                    colors[next_byte] = HighlightSpec::with_fg(HighlightRole::Escape);
                }
            } else if c == '$' {
                if byte_pos < colors.len() {
                    colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Variable);
                }
            } else {
                if byte_pos < colors.len() {
                    colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::String);
                }
            }
            i += 1;
            continue;
        }

        // Handle variables
        if c == '$' {
            if byte_pos < colors.len() {
                colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Variable);
            }
            i += 1;
            // Color the variable name
            while i < chars.len() {
                let vc = chars[i];
                if vc.is_alphanumeric() || vc == '_' || vc == '{' || vc == '}' {
                    let vbyte = line.char_indices().nth(i).map(|(p, _)| p).unwrap_or(0);
                    if vbyte < colors.len() {
                        colors[vbyte] = HighlightSpec::with_fg(HighlightRole::Variable);
                    }
                    i += 1;
                } else {
                    break;
                }
            }
            continue;
        }

        // Handle operators and redirections
        if c == '|' || c == '&' || c == ';' {
            if byte_pos < colors.len() {
                colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Operator);
            }
            is_first_word = true;
            after_pipe_or_semi = true;
            i += 1;
            continue;
        }
        if c == '>' || c == '<' {
            if byte_pos < colors.len() {
                colors[byte_pos] = HighlightSpec::with_fg(HighlightRole::Redirection);
            }
            // Handle >> or <<
            if i + 1 < chars.len() && (chars[i + 1] == '>' || chars[i + 1] == '<') {
                i += 1;
                let next_byte = line.char_indices().nth(i).map(|(p, _)| p).unwrap_or(0);
                if next_byte < colors.len() {
                    colors[next_byte] = HighlightSpec::with_fg(HighlightRole::Redirection);
                }
            }
            i += 1;
            continue;
        }

        // Handle word boundaries
        if c.is_whitespace() {
            if let Some(start) = word_start {
                // End of word - colorize it
                let word_end = i;
                let word: String = chars[start..word_end].iter().collect();
                colorize_word(
                    &word,
                    start,
                    &mut colors,
                    line,
                    is_first_word || after_pipe_or_semi,
                );
                is_first_word = false;
                after_pipe_or_semi = false;
            }
            word_start = None;
            i += 1;
            continue;
        }

        // Start of word
        if word_start.is_none() {
            word_start = Some(i);
        }

        i += 1;
    }

    // Handle last word
    if let Some(start) = word_start {
        let word: String = chars[start..].iter().collect();
        colorize_word(
            &word,
            start,
            &mut colors,
            line,
            is_first_word || after_pipe_or_semi,
        );
    }

    colors
}

fn colorize_word(
    word: &str,
    char_start: usize,
    colors: &mut [HighlightSpec],
    line: &str,
    is_command_position: bool,
) {
    let role = if is_command_position {
        if KEYWORDS.contains(&word) {
            HighlightRole::Keyword
        } else if BUILTINS.contains(&word)
            || command_exists(word)
            || (word.contains('/') && std::path::Path::new(word).exists())
        {
            HighlightRole::Command
        } else {
            HighlightRole::Error
        }
    } else if word.starts_with('-') {
        HighlightRole::Option
    } else if std::path::Path::new(word).exists() {
        HighlightRole::PathValid
    } else {
        HighlightRole::Param
    };

    // Map char position to byte position and colorize
    for (ci, _) in word.char_indices() {
        let global_char_idx = char_start + word[..ci].chars().count();
        if let Some((byte_pos, _)) = line.char_indices().nth(global_char_idx) {
            if byte_pos < colors.len() {
                colors[byte_pos] = HighlightSpec::with_fg(role);
            }
        }
    }
    // Also color the last char
    let last_char_idx = char_start + word.chars().count() - 1;
    if let Some((byte_pos, _)) = line.char_indices().nth(last_char_idx) {
        if byte_pos < colors.len() {
            colors[byte_pos] = HighlightSpec::with_fg(role);
        }
    }
}

/// Check if a command exists in PATH
fn command_exists(cmd: &str) -> bool {
    if cmd.is_empty() {
        return false;
    }
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            let full_path = std::path::Path::new(dir).join(cmd);
            if full_path.is_file() {
                return true;
            }
        }
    }
    false
}

/// Convert highlight specs to ANSI-colored string
pub fn colorize_line(line: &str, colors: &[HighlightSpec]) -> String {
    let mut result = String::with_capacity(line.len() * 2);
    let mut last_role = HighlightRole::Normal;

    for (i, c) in line.chars().enumerate() {
        let byte_pos = line.char_indices().nth(i).map(|(p, _)| p).unwrap_or(i);
        let role = colors
            .get(byte_pos)
            .map(|s| s.foreground)
            .unwrap_or(HighlightRole::Normal);

        if role != last_role {
            result.push_str(role_to_ansi(role));
            last_role = role;
        }
        result.push(c);
    }

    if last_role != HighlightRole::Normal {
        result.push_str("\x1b[0m");
    }

    result
}

// ============================================================================
// ABBREVIATIONS
// ============================================================================

/// Position where abbreviation can expand
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Abbreviation expansion position.
/// Adapted from fish-shell. No C zsh counterpart — C zsh has
/// `setopt EXPANDABBREV` only via the `zsh-abbr` plugin, not natively.
pub enum AbbrPosition {
    /// `Command` variant.
    Command, // Only in command position
    /// `Anywhere` variant.
    Anywhere, // Anywhere in the line
}

/// An abbreviation definition
#[derive(Debug, Clone)]
pub struct Abbreviation {
    /// `name` field.
    pub name: String,
    /// `key` field.
    pub key: String,
    /// `replacement` field.
    pub replacement: String,
    /// `position` field.
    pub position: AbbrPosition,
}

impl Abbreviation {
    /// `new` — see implementation.
    pub fn new(name: &str, key: &str, replacement: &str, position: AbbrPosition) -> Self {
        Self {
            name: name.to_string(),
            key: key.to_string(),
            replacement: replacement.to_string(),
            position,
        }
    }
    /// `matches` — see implementation.
    pub fn matches(&self, token: &str, is_command_position: bool) -> bool {
        let position_ok = match self.position {
            AbbrPosition::Anywhere => true,
            AbbrPosition::Command => is_command_position,
        };
        position_ok && self.key == token
    }
}

/// Global abbreviation set
static ABBRS: LazyLock<Mutex<AbbreviationSet>> =
    LazyLock::new(|| Mutex::new(AbbreviationSet::default()));
/// `with_abbrs` — see implementation.
pub fn with_abbrs<R>(cb: impl FnOnce(&AbbreviationSet) -> R) -> R {
    let abbrs = ABBRS.lock().unwrap();
    cb(&abbrs)
}
/// `with_abbrs_mut` — see implementation.
pub fn with_abbrs_mut<R>(cb: impl FnOnce(&mut AbbreviationSet) -> R) -> R {
    let mut abbrs = ABBRS.lock().unwrap();
    cb(&mut abbrs)
}
/// `AbbreviationSet` — see fields for layout.
#[derive(Default)]
pub struct AbbreviationSet {
    /// `abbrs` field.
    abbrs: Vec<Abbreviation>,
    /// `used_names` field.
    used_names: HashSet<String>,
}

impl AbbreviationSet {
    /// Find matching abbreviation for a token
    pub fn find_match(&self, token: &str, is_command_position: bool) -> Option<&Abbreviation> {
        // Later abbreviations take precedence
        self.abbrs
            .iter()
            .rev()
            .find(|a| a.matches(token, is_command_position))
    }

    /// Check if any abbreviation matches
    pub fn has_match(&self, token: &str, is_command_position: bool) -> bool {
        self.abbrs
            .iter()
            .any(|a| a.matches(token, is_command_position))
    }

    /// Add an abbreviation
    pub fn add(&mut self, abbr: Abbreviation) {
        if self.used_names.contains(&abbr.name) {
            self.abbrs.retain(|a| a.name != abbr.name);
        }
        self.used_names.insert(abbr.name.clone());
        self.abbrs.push(abbr);
    }

    /// Remove an abbreviation by name
    pub fn remove(&mut self, name: &str) -> bool {
        if self.used_names.remove(name) {
            self.abbrs.retain(|a| a.name != name);
            true
        } else {
            false
        }
    }

    /// List all abbreviations
    pub fn list(&self) -> &[Abbreviation] {
        &self.abbrs
    }
}

/// Expand abbreviations in a line at the current word
/// Expand an abbreviation at the cursor position.
/// Adapted from fish-shell. No C zsh counterpart.
pub fn expand_abbreviation(line: &str, cursor: usize) -> Option<(String, usize)> {
    // Find the word at cursor
    let before_cursor = &line[..cursor.min(line.len())];
    let word_start = before_cursor
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(0);
    let word = &before_cursor[word_start..];

    if word.is_empty() {
        return None;
    }

    // Check if we're in command position
    let is_command_position = before_cursor[..word_start].trim().is_empty()
        || before_cursor[..word_start]
            .trim()
            .ends_with(['|', ';', '&']);

    with_abbrs(|set| {
        set.find_match(word, is_command_position).map(|abbr| {
            let mut new_line = String::with_capacity(line.len() + abbr.replacement.len());
            new_line.push_str(&line[..word_start]);
            new_line.push_str(&abbr.replacement);
            new_line.push_str(&line[cursor..]);
            let new_cursor = word_start + abbr.replacement.len();
            (new_line, new_cursor)
        })
    })
}

// ============================================================================
// AUTOSUGGESTIONS
// ============================================================================

/// History-based autosuggestion
pub struct Autosuggestion {
    /// `text` field.
    pub text: String,
    /// `is_from_history` field.
    pub is_from_history: bool,
}

impl Autosuggestion {
    /// `empty` — see implementation.
    pub fn empty() -> Self {
        Self {
            text: String::new(),
            is_from_history: false,
        }
    }
    /// `is_empty` — see implementation.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }
}

/// Generate autosuggestion from history
/// Build an autosuggestion from history (the `zsh-autosuggestions`
/// behavior, native).
/// Adapted from fish-shell's `autosuggestion::autosuggest_*` set —
/// no C zsh counterpart; users currently get this via the
/// zsh-autosuggestions zle plugin.
pub fn autosuggest_from_history(line: &str, history: &[String]) -> Autosuggestion {
    if line.is_empty() {
        return Autosuggestion::empty();
    }

    let line_lower = line.to_lowercase();

    // Search history in reverse (most recent first)
    for entry in history.iter().rev() {
        // Exact prefix match (case-sensitive)
        if entry.starts_with(line) && entry.len() > line.len() {
            return Autosuggestion {
                text: entry[line.len()..].to_string(),
                is_from_history: true,
            };
        }
    }

    // Case-insensitive prefix match
    for entry in history.iter().rev() {
        let entry_lower = entry.to_lowercase();
        if entry_lower.starts_with(&line_lower) && entry.len() > line.len() {
            return Autosuggestion {
                text: entry[line.len()..].to_string(),
                is_from_history: true,
            };
        }
    }

    Autosuggestion::empty()
}

/// Validate autosuggestion (check if command exists, paths valid, etc.)
pub fn validate_autosuggestion(suggestion: &str, current_line: &str) -> bool {
    if suggestion.is_empty() {
        return false;
    }

    // Get the full command that would result
    let full_line = format!("{}{}", current_line, suggestion);
    let words: Vec<&str> = full_line.split_whitespace().collect();

    if words.is_empty() {
        return true;
    }

    let cmd = words[0];

    // Check if command exists
    if !command_exists(cmd) && !BUILTINS.contains(&cmd) && !KEYWORDS.contains(&cmd) {
        // Check if it's a path
        if !cmd.contains('/') || !std::path::Path::new(cmd).exists() {
            return false;
        }
    }

    true
}

// ============================================================================
// KILLRING (Yank/Paste)
// ============================================================================

static KILLRING: LazyLock<Mutex<KillRing>> = LazyLock::new(|| Mutex::new(KillRing::new(100)));

/// Yank-pop kill ring.
/// Adapted from fish-shell. C zsh's equivalent lives in
/// Src/Zle/zle_misc.c — the `kringnum` array drives `yank-pop`.
/// We keep a separate kill-ring here for fish-style yank cycling.
pub struct KillRing {
    /// `entries` field.
    entries: Vec<String>,
    /// `max_size` field.
    max_size: usize,
    /// `yank_index` field.
    yank_index: usize,
}

impl KillRing {
    /// `new` — see implementation.
    pub fn new(max_size: usize) -> Self {
        Self {
            entries: Vec::with_capacity(max_size),
            max_size,
            yank_index: 0,
        }
    }

    /// Add text to killring
    pub fn add(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        // Remove duplicates
        self.entries.retain(|e| e != &text);
        self.entries.insert(0, text);
        if self.entries.len() > self.max_size {
            self.entries.pop();
        }
        self.yank_index = 0;
    }

    /// Replace last entry (for consecutive kills)
    pub fn replace(&mut self, text: String) {
        if text.is_empty() {
            return;
        }
        if self.entries.is_empty() {
            self.add(text);
        } else {
            self.entries[0] = text;
        }
    }

    /// Get current yank text
    pub fn yank(&self) -> Option<&str> {
        self.entries.get(self.yank_index).map(|s| s.as_str())
    }

    /// Rotate to next entry (yank-pop)
    pub fn rotate(&mut self) -> Option<&str> {
        if self.entries.is_empty() {
            return None;
        }
        self.yank_index = (self.yank_index + 1) % self.entries.len();
        self.yank()
    }

    /// Reset yank index
    pub fn reset_yank(&mut self) {
        self.yank_index = 0;
    }
}
/// `kill_add` — see implementation.
pub fn kill_add(text: String) {
    KILLRING.lock().unwrap().add(text);
}
/// `kill_replace` — see implementation.
pub fn kill_replace(text: String) {
    KILLRING.lock().unwrap().replace(text);
}
/// `kill_yank` — see implementation.
pub fn kill_yank() -> Option<String> {
    KILLRING.lock().unwrap().yank().map(|s| s.to_string())
}
/// `kill_yank_rotate` — see implementation.
pub fn kill_yank_rotate() -> Option<String> {
    KILLRING.lock().unwrap().rotate().map(|s| s.to_string())
}

// ============================================================================
// COMMAND VALIDATION
// ============================================================================

/// Validate a command line for errors
pub fn validate_command(line: &str) -> ValidationStatus {
    if line.trim().is_empty() {
        return ValidationStatus::Valid;
    }

    // Check for unclosed quotes
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    for c in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            _ => {}
        }
    }

    if in_single || in_double {
        return ValidationStatus::Incomplete;
    }

    // Check for incomplete commands (trailing | or &&)
    let trimmed = line.trim();
    if trimmed.ends_with('|') || trimmed.ends_with("&&") || trimmed.ends_with("||") {
        return ValidationStatus::Incomplete;
    }

    // Check for unclosed braces/brackets
    let mut brace_count = 0i32;
    let mut bracket_count = 0i32;
    let mut paren_count = 0i32;

    for c in line.chars() {
        match c {
            '{' => brace_count += 1,
            '}' => brace_count -= 1,
            '[' => bracket_count += 1,
            ']' => bracket_count -= 1,
            '(' => paren_count += 1,
            ')' => paren_count -= 1,
            _ => {}
        }
        if brace_count < 0 || bracket_count < 0 || paren_count < 0 {
            return ValidationStatus::Invalid("Unmatched closing bracket".into());
        }
    }

    if brace_count > 0 || bracket_count > 0 || paren_count > 0 {
        return ValidationStatus::Incomplete;
    }

    ValidationStatus::Valid
}
/// `ValidationStatus` — see variants.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationStatus {
    /// `Valid` variant.
    Valid,
    /// `Incomplete` variant.
    Incomplete,
    /// `Invalid` variant.
    Invalid(String),
}

// ============================================================================
// PRIVATE MODE
// ============================================================================

static PRIVATE_MODE: LazyLock<Mutex<bool>> = LazyLock::new(|| Mutex::new(false));
/// `is_private_mode` — see implementation.
pub fn is_private_mode() -> bool {
    *PRIVATE_MODE.lock().unwrap()
}
/// `set_private_mode` — see implementation.
pub fn set_private_mode(enabled: bool) {
    *PRIVATE_MODE.lock().unwrap() = enabled;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_highlight_command() {
        let _g = crate::test_util::global_state_lock();
        let line = "ls -la /tmp";
        let colors = highlight_shell(line);
        assert!(!colors.is_empty());
    }

    #[test]
    fn test_abbreviation() {
        let _g = crate::test_util::global_state_lock();
        with_abbrs_mut(|set| {
            set.add(Abbreviation::new("g", "g", "git", AbbrPosition::Command));
            set.add(Abbreviation::new(
                "ga",
                "ga",
                "git add",
                AbbrPosition::Command,
            ));
        });

        let result = expand_abbreviation("g", 1);
        assert!(result.is_some());
        let (new_line, _) = result.unwrap();
        assert_eq!(new_line, "git");
    }

    #[test]
    fn test_autosuggestion() {
        let _g = crate::test_util::global_state_lock();
        let history = vec![
            "ls -la".to_string(),
            "git status".to_string(),
            "git commit -m 'test'".to_string(),
        ];

        let suggestion = autosuggest_from_history("git s", &history);
        assert!(!suggestion.is_empty());
        assert_eq!(suggestion.text, "tatus");
    }

    #[test]
    fn test_killring() {
        let _g = crate::test_util::global_state_lock();
        kill_add("first".to_string());
        kill_add("second".to_string());

        assert_eq!(kill_yank(), Some("second".to_string()));
        assert_eq!(kill_yank_rotate(), Some("first".to_string()));
    }

    #[test]
    fn test_validation() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(validate_command("echo hello"), ValidationStatus::Valid);
        assert_eq!(
            validate_command("echo \"unclosed"),
            ValidationStatus::Incomplete
        );
        assert_eq!(validate_command("ls |"), ValidationStatus::Incomplete);
    }
}
