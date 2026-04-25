//! Completion state management - the $compstate hash and special parameters

use std::collections::HashMap;

/// Completion context type (compstate[context])
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CompletionContext {
    #[default]
    Command,
    /// In argument position
    Argument,
    /// In redirection
    Redirect,
    /// In array subscript
    Subscript,
    /// In math context
    Math,
    /// In parameter assignment
    Parameter,
    /// In brace parameter ${...}
    BraceParameter,
    /// In condition [[ ]]
    Condition,
    /// Array value assignment
    ArrayValue,
    /// Scalar value assignment
    Value,
}

impl CompletionContext {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Argument => "argument",
            Self::Redirect => "redirect",
            Self::Subscript => "subscript",
            Self::Math => "math",
            Self::Parameter => "parameter",
            Self::BraceParameter => "brace_parameter",
            Self::Condition => "condition",
            Self::ArrayValue => "array_value",
            Self::Value => "value",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "command" => Some(Self::Command),
            "argument" => Some(Self::Argument),
            "redirect" => Some(Self::Redirect),
            "subscript" => Some(Self::Subscript),
            "math" => Some(Self::Math),
            "parameter" => Some(Self::Parameter),
            "brace_parameter" => Some(Self::BraceParameter),
            "condition" => Some(Self::Condition),
            "array_value" => Some(Self::ArrayValue),
            "value" => Some(Self::Value),
            _ => None,
        }
    }
}

/// The $compstate associative array
#[derive(Clone, Debug, Default)]
pub struct CompState {
    /// Context type
    pub context: CompletionContext,
    /// Parameter name if in assignment
    pub parameter: String,
    /// Redirection string
    pub redirect: String,
    /// Quote character (' " or $')
    pub quote: String,
    /// Quoting type (single, double, dollars)
    pub quoting: String,
    /// Restore mode (auto or empty)
    pub restore: String,
    /// List mode (list, autolist, ambiguous)
    pub list: String,
    /// Insert mode (menu, unambiguous, automenu, or N for Nth match)
    pub insert: String,
    /// Exact match mode (accept or empty)
    pub exact: String,
    /// Exact match string
    pub exact_string: String,
    /// Pattern matching mode (* or empty)
    pub pattern_match: String,
    /// Pattern insert mode
    pub pattern_insert: String,
    /// Unambiguous prefix (readonly)
    pub unambiguous: String,
    /// Cursor position in unambiguous (readonly)
    pub unambiguous_cursor: i32,
    /// Unambiguous positions (readonly)
    pub unambiguous_positions: String,
    /// Insert positions (readonly)
    pub insert_positions: String,
    /// Maximum list entries
    pub list_max: i32,
    /// Last prompt setting
    pub last_prompt: String,
    /// Move to end mode (single, match, always)
    pub to_end: String,
    /// Old list state (yes, shown)
    pub old_list: String,
    /// Old insert state (keep)
    pub old_insert: String,
    /// Vared buffer name
    pub vared: String,
    /// Number of list lines (readonly)
    pub list_lines: i32,
    /// All quotes stack (readonly)
    pub all_quotes: String,
    /// Number of ignored matches (readonly)
    pub ignored: i32,
    /// Number of matches (readonly)
    pub nmatches: i32,
}

impl CompState {
    pub fn new() -> Self {
        Self {
            list: "autolist".to_string(),
            insert: "unambiguous".to_string(),
            to_end: "match".to_string(),
            ..Default::default()
        }
    }

    /// Get a value by key name (for $compstate[key] access)
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "context" => Some(self.context.as_str().to_string()),
            "parameter" => Some(self.parameter.clone()),
            "redirect" => Some(self.redirect.clone()),
            "quote" => Some(self.quote.clone()),
            "quoting" => Some(self.quoting.clone()),
            "restore" => Some(self.restore.clone()),
            "list" => Some(self.list.clone()),
            "insert" => Some(self.insert.clone()),
            "exact" => Some(self.exact.clone()),
            "exact_string" => Some(self.exact_string.clone()),
            "pattern_match" => Some(self.pattern_match.clone()),
            "pattern_insert" => Some(self.pattern_insert.clone()),
            "unambiguous" => Some(self.unambiguous.clone()),
            "unambiguous_cursor" => Some(self.unambiguous_cursor.to_string()),
            "unambiguous_positions" => Some(self.unambiguous_positions.clone()),
            "insert_positions" => Some(self.insert_positions.clone()),
            "list_max" => Some(self.list_max.to_string()),
            "last_prompt" => Some(self.last_prompt.clone()),
            "to_end" => Some(self.to_end.clone()),
            "old_list" => Some(self.old_list.clone()),
            "old_insert" => Some(self.old_insert.clone()),
            "vared" => Some(self.vared.clone()),
            "list_lines" => Some(self.list_lines.to_string()),
            "all_quotes" => Some(self.all_quotes.clone()),
            "ignored" => Some(self.ignored.to_string()),
            "nmatches" => Some(self.nmatches.to_string()),
            _ => None,
        }
    }

    /// Set a value by key name (for $compstate[key]=value)
    pub fn set(&mut self, key: &str, value: &str) -> bool {
        match key {
            "context" => {
                if let Some(ctx) = CompletionContext::from_str(value) {
                    self.context = ctx;
                    true
                } else {
                    false
                }
            }
            "parameter" => {
                self.parameter = value.to_string();
                true
            }
            "redirect" => {
                self.redirect = value.to_string();
                true
            }
            "restore" => {
                self.restore = value.to_string();
                true
            }
            "list" => {
                self.list = value.to_string();
                true
            }
            "insert" => {
                self.insert = value.to_string();
                true
            }
            "exact" => {
                self.exact = value.to_string();
                true
            }
            "pattern_match" => {
                self.pattern_match = value.to_string();
                true
            }
            "pattern_insert" => {
                self.pattern_insert = value.to_string();
                true
            }
            "list_max" => {
                if let Ok(n) = value.parse() {
                    self.list_max = n;
                    true
                } else {
                    false
                }
            }
            "last_prompt" => {
                self.last_prompt = value.to_string();
                true
            }
            "to_end" => {
                self.to_end = value.to_string();
                true
            }
            "old_list" => {
                self.old_list = value.to_string();
                true
            }
            "old_insert" => {
                self.old_insert = value.to_string();
                true
            }
            "vared" => {
                self.vared = value.to_string();
                true
            }
            // Readonly fields
            "quote"
            | "quoting"
            | "unambiguous"
            | "unambiguous_cursor"
            | "unambiguous_positions"
            | "insert_positions"
            | "list_lines"
            | "all_quotes"
            | "ignored"
            | "nmatches"
            | "exact_string" => false,
            _ => false,
        }
    }

    /// Get all keys
    pub fn keys() -> &'static [&'static str] {
        &[
            "context",
            "parameter",
            "redirect",
            "quote",
            "quoting",
            "restore",
            "list",
            "insert",
            "exact",
            "exact_string",
            "pattern_match",
            "pattern_insert",
            "unambiguous",
            "unambiguous_cursor",
            "unambiguous_positions",
            "insert_positions",
            "list_max",
            "last_prompt",
            "to_end",
            "old_list",
            "old_insert",
            "vared",
            "list_lines",
            "all_quotes",
            "ignored",
            "nmatches",
        ]
    }

    /// Convert to HashMap for shell access
    pub fn to_hash(&self) -> HashMap<String, String> {
        Self::keys()
            .iter()
            .filter_map(|&k| self.get(k).map(|v| (k.to_string(), v)))
            .collect()
    }
}

/// Completion special parameters (the real params, not compstate keys)
/// These are: words, CURRENT, PREFIX, SUFFIX, IPREFIX, ISUFFIX, QIPREFIX, QISUFFIX
#[derive(Clone, Debug, Default)]
pub struct CompParams {
    /// $words - array of words on command line
    pub words: Vec<String>,
    /// $CURRENT - index of current word (1-based)
    pub current: i32,
    /// $PREFIX - part of current word before cursor
    pub prefix: String,
    /// $SUFFIX - part of current word after cursor
    pub suffix: String,
    /// $IPREFIX - ignored prefix (chars moved from PREFIX)
    pub iprefix: String,
    /// $ISUFFIX - ignored suffix (chars moved from SUFFIX)
    pub isuffix: String,
    /// $QIPREFIX - quoted ignored prefix (readonly)
    pub qiprefix: String,
    /// $QISUFFIX - quoted ignored suffix (readonly)
    pub qisuffix: String,
    /// $compstate - the completion state hash
    pub compstate: CompState,
}

impl CompParams {
    pub fn new() -> Self {
        Self {
            current: 1,
            compstate: CompState::new(),
            ..Default::default()
        }
    }

    /// Initialize from a command line and cursor position
    pub fn from_line(line: &str, cursor: usize) -> Self {
        let mut params = Self::new();

        // Parse the line into words
        let (words, current_idx, prefix, suffix) = parse_command_line(line, cursor);

        params.words = words;
        params.current = current_idx as i32;
        params.prefix = prefix;
        params.suffix = suffix;

        // Determine context
        params.compstate.context = if params.current == 1 {
            CompletionContext::Command
        } else {
            CompletionContext::Argument
        };

        params
    }

    /// Get the current word being completed
    pub fn current_word(&self) -> String {
        format!("{}{}", self.prefix, self.suffix)
    }

    /// Get word at index (1-based, like zsh)
    pub fn word_at(&self, idx: i32) -> Option<&String> {
        if idx < 1 {
            return None;
        }
        self.words.get((idx - 1) as usize)
    }
}

/// Parse a command line into words and find current word/prefix/suffix
fn parse_command_line(line: &str, cursor: usize) -> (Vec<String>, usize, String, String) {
    let mut words = Vec::new();
    let mut current_word = String::new();
    let current_idx;
    let mut in_word = false;
    let mut word_start = 0;
    let mut cursor_in_word: Option<usize> = None; // word index where cursor is
    let mut cursor_offset_in_word = 0; // byte offset within that word

    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;
    let mut byte_pos = 0;

    while i < chars.len() {
        let ch = chars[i];
        let char_len = ch.len_utf8();

        if ch.is_whitespace() {
            if in_word {
                // End of word - check if cursor was in this word
                if cursor_in_word.is_none() && cursor <= byte_pos && cursor >= word_start {
                    cursor_in_word = Some(words.len());
                    cursor_offset_in_word = cursor - word_start;
                }
                words.push(current_word.clone());
                current_word.clear();
                in_word = false;
            } else if cursor_in_word.is_none() && cursor == byte_pos {
                // Cursor in whitespace - points to next word (empty)
                cursor_in_word = Some(words.len());
                cursor_offset_in_word = 0;
            }
        } else {
            if !in_word {
                word_start = byte_pos;
                in_word = true;
            }
            current_word.push(ch);
        }

        byte_pos += char_len;
        i += 1;
    }

    // Handle final word
    if in_word {
        if cursor_in_word.is_none() && cursor >= word_start {
            cursor_in_word = Some(words.len());
            cursor_offset_in_word = cursor - word_start;
        }
        words.push(current_word);
    } else if cursor_in_word.is_none() {
        // Cursor is at end after whitespace
        cursor_in_word = Some(words.len());
        cursor_offset_in_word = 0;
        words.push(String::new());
    }

    // Now compute prefix/suffix from the word containing cursor
    let (prefix, suffix) = if let Some(word_idx) = cursor_in_word {
        if word_idx < words.len() {
            let word = &words[word_idx];
            let word_chars: Vec<char> = word.chars().collect();
            let mut char_offset = 0;
            let mut char_count = 0;
            for wc in &word_chars {
                if char_offset >= cursor_offset_in_word {
                    break;
                }
                char_offset += wc.len_utf8();
                char_count += 1;
            }
            let pre: String = word_chars[..char_count].iter().collect();
            let suf: String = word_chars[char_count..].iter().collect();
            (pre, suf)
        } else {
            (String::new(), String::new())
        }
    } else {
        (String::new(), String::new())
    };

    current_idx = cursor_in_word.map(|i| i + 1).unwrap_or(1);
    (words, current_idx, prefix, suffix)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_line() {
        let (words, idx, pre, suf) = parse_command_line("git commit -m", 13);
        assert_eq!(words, vec!["git", "commit", "-m"]);
        assert_eq!(idx, 3);
        assert_eq!(pre, "-m");
        assert_eq!(suf, "");

        // "git com" = bytes: g(0) i(1) t(2) ' '(3) c(4) o(5) m(6)
        // cursor=5 means after 'o', so prefix="co" suffix="m"
        // word_start=4, cursor_offset_in_word = 5-4 = 1
        // But that gives prefix="c"... the offset should count chars not bytes
        // Actually wait: in "com", c is at offset 0, o at 1, m at 2
        // If cursor_offset_in_word=1, we get prefix="c" (chars 0..1 exclusive = "c")
        // We need cursor_offset_in_word=2 to get "co"
        // cursor=5, word_start=4, 5-4=1 is correct for bytes
        // But char_offset counts bytes too, so this should work...
        // Let me check: cursor=5 means cursor is BEFORE byte 5 (the 'o')
        // Actually cursor=5 means we've typed 5 bytes. So prefix is bytes 4..5 = "c"
        // If we want prefix="co", cursor should be 6
        let (words, idx, pre, suf) = parse_command_line("git com", 6);
        assert_eq!(words, vec!["git", "com"]);
        assert_eq!(idx, 2);
        assert_eq!(pre, "co");
        assert_eq!(suf, "m");

        let (words, idx, pre, suf) = parse_command_line("git ", 4);
        assert_eq!(words, vec!["git", ""]);
        assert_eq!(idx, 2);
        assert_eq!(pre, "");
        assert_eq!(suf, "");
    }

    #[test]
    fn test_compstate_get_set() {
        let mut state = CompState::new();

        assert!(state.set("insert", "menu"));
        assert_eq!(state.get("insert"), Some("menu".to_string()));

        // Readonly should fail
        assert!(!state.set("nmatches", "42"));
    }
}
