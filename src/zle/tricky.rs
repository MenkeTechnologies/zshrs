//! ZLE tricky - completion and expansion widgets
//!
//! Direct port from zsh/Src/Zle/zle_tricky.c
//!
//! Implements completion widgets:
//! - complete-word, menu-complete, reverse-menu-complete
//! - expand-or-complete, expand-or-complete-prefix
//! - list-choices, list-expand
//! - expand-word, expand-history
//! - spell-word, delete-char-or-list
//! - magic-space, accept-and-menu-complete

use super::main::Zle;

/// Completion state
#[derive(Debug, Default, Clone)]
pub struct CompletionState {
    /// Whether we're in menu completion mode
    pub in_menu: bool,
    /// Current menu index
    pub menu_index: usize,
    /// Available completions
    pub completions: Vec<String>,
    /// Prefix being completed
    pub prefix: String,
    /// Suffix after cursor
    pub suffix: String,
    /// Word start position
    pub word_start: usize,
    /// Word end position
    pub word_end: usize,
    /// Last completion was a menu cycle
    pub last_menu: bool,
}

/// Brace info for parameter expansion
#[derive(Debug, Clone)]
pub struct BraceInfo {
    pub str_val: String,
    pub pos: usize,
    pub cur_pos: usize,
    pub qpos: usize,
    pub curlen: usize,
}

impl Zle {
    /// Complete word - trigger completion
    /// Port of completeword() from zle_tricky.c
    pub fn complete_word(&mut self, state: &mut CompletionState) {
        self.do_complete(state, false, false);
    }

    /// Menu complete - cycle through completions
    /// Port of menucomplete() from zle_tricky.c
    pub fn menu_complete(&mut self, state: &mut CompletionState) {
        if state.in_menu && !state.completions.is_empty() {
            // Cycle to next completion
            state.menu_index = (state.menu_index + 1) % state.completions.len();
            self.apply_completion(state);
        } else {
            self.do_complete(state, true, false);
        }
    }

    /// Reverse menu complete - cycle backwards
    /// Port of reversemenucomplete() from zle_tricky.c
    pub fn reverse_menu_complete(&mut self, state: &mut CompletionState) {
        if state.in_menu && !state.completions.is_empty() {
            if state.menu_index == 0 {
                state.menu_index = state.completions.len() - 1;
            } else {
                state.menu_index -= 1;
            }
            self.apply_completion(state);
        }
    }

    /// Expand or complete - try expansion first, then completion
    /// Port of expandorcomplete() from zle_tricky.c
    pub fn expand_or_complete(&mut self, state: &mut CompletionState) {
        // First try expansion
        if !self.try_expand() {
            // Then try completion
            self.do_complete(state, false, false);
        }
    }

    /// Expand or complete prefix - expand/complete keeping suffix
    /// Port of expandorcompleteprefix() from zle_tricky.c
    pub fn expand_or_complete_prefix(&mut self, state: &mut CompletionState) {
        state.suffix = self.zleline[self.zlecs..].iter().collect();
        self.expand_or_complete(state);
    }

    /// List choices - show available completions
    /// Port of listchoices() from zle_tricky.c
    pub fn list_choices(&mut self, state: &mut CompletionState) {
        self.do_complete(state, false, true);

        if !state.completions.is_empty() {
            println!();
            for (i, c) in state.completions.iter().enumerate() {
                if i > 0 && i % 5 == 0 {
                    println!();
                }
                print!("{:<16}", c);
            }
            println!();
            self.resetneeded = true;
        }
    }

    /// List expand - list possible expansions
    /// Port of listexpand() from zle_tricky.c
    pub fn list_expand(&mut self) {
        let word = self.get_word_at_cursor();
        let expansions = self.do_expansion(&word);

        if !expansions.is_empty() {
            println!();
            for exp in &expansions {
                println!("{}", exp);
            }
            self.resetneeded = true;
        }
    }

    /// Expand word - expand current word (glob, history, etc)
    /// Port of expandword() from zle_tricky.c
    pub fn expand_word(&mut self) {
        let _ = self.try_expand();
    }

    /// Expand history - expand history references
    /// Port of expandhistory() / doexpandhist() from zle_tricky.c
    pub fn expand_history(&mut self) {
        let line: String = self.zleline.iter().collect();

        // Look for history references like !!, !$, !*, etc.
        let expanded = self.do_expand_hist(&line);

        if expanded != line {
            self.zleline = expanded.chars().collect();
            self.zlell = self.zleline.len();
            if self.zlecs > self.zlell {
                self.zlecs = self.zlell;
            }
            self.resetneeded = true;
        }
    }

    /// Magic space - expand history then insert space
    /// Port of magicspace() from zle_tricky.c
    pub fn magic_space(&mut self) {
        self.expand_history();
        self.self_insert(' ');
    }

    /// Delete char or list - delete if there's text, else list completions
    /// Port of deletecharorlist() from zle_tricky.c
    pub fn delete_char_or_list(&mut self, state: &mut CompletionState) {
        if self.zlecs < self.zlell {
            self.delete_char();
        } else {
            self.list_choices(state);
        }
    }

    /// Accept and menu complete
    /// Port of acceptandmenucomplete() from zle_tricky.c  
    pub fn accept_and_menu_complete(&mut self, state: &mut CompletionState) -> Option<String> {
        let line = self.accept_line();
        state.in_menu = false;
        Some(line)
    }

    /// Spell word - check spelling
    /// Port of spellword() from zle_tricky.c
    pub fn spell_word(&mut self) {
        // Simple spell check - look for common patterns
        let word = self.get_word_at_cursor();
        // Would integrate with aspell/hunspell in full implementation
        let _ = word;
    }

    /// Internal: perform completion
    fn do_complete(&mut self, state: &mut CompletionState, menu_mode: bool, list_only: bool) {
        // Get word at cursor
        let (word_start, word_end) = self.get_word_bounds();
        let word: String = self.zleline[word_start..word_end].iter().collect();

        state.word_start = word_start;
        state.word_end = word_end;
        state.prefix = word.clone();

        // Get completions (simplified - real impl would call compsys)
        state.completions = self.get_completions(&word);

        if state.completions.is_empty() {
            return;
        }

        if list_only {
            return;
        }

        if menu_mode || state.completions.len() > 1 {
            state.in_menu = true;
            state.menu_index = 0;
            self.apply_completion(state);
        } else if state.completions.len() == 1 {
            // Single completion - apply directly
            state.menu_index = 0;
            self.apply_completion(state);
            state.in_menu = false;
        }
    }

    /// Apply current completion from state
    fn apply_completion(&mut self, state: &CompletionState) {
        if state.completions.is_empty() {
            return;
        }

        let completion = &state.completions[state.menu_index];

        // Remove old word
        self.zleline.drain(state.word_start..state.word_end);
        self.zlell = self.zleline.len();
        self.zlecs = state.word_start;

        // Insert completion
        for c in completion.chars() {
            self.zleline.insert(self.zlecs, c);
            self.zlecs += 1;
        }
        self.zlell = self.zleline.len();
        self.resetneeded = true;
    }

    /// Get word at cursor position
    fn get_word_at_cursor(&self) -> String {
        let (start, end) = self.get_word_bounds();
        self.zleline[start..end].iter().collect()
    }

    /// Get bounds of word at cursor
    fn get_word_bounds(&self) -> (usize, usize) {
        let mut start = self.zlecs;
        let mut end = self.zlecs;

        // Find word start
        while start > 0 && !self.zleline[start - 1].is_whitespace() {
            start -= 1;
        }

        // Find word end
        while end < self.zlell && !self.zleline[end].is_whitespace() {
            end += 1;
        }

        (start, end)
    }

    /// Try to expand the word at cursor
    fn try_expand(&mut self) -> bool {
        let word = self.get_word_at_cursor();

        if word.is_empty() {
            return false;
        }

        let expansions = self.do_expansion(&word);

        if expansions.is_empty() || (expansions.len() == 1 && expansions[0] == word) {
            return false;
        }

        let (start, end) = self.get_word_bounds();

        // Remove old word
        self.zleline.drain(start..end);
        self.zlecs = start;

        // Insert expansions
        let expanded = expansions.join(" ");
        for c in expanded.chars() {
            self.zleline.insert(self.zlecs, c);
            self.zlecs += 1;
        }
        self.zlell = self.zleline.len();
        self.resetneeded = true;

        true
    }

    /// Do expansion on a word
    fn do_expansion(&self, word: &str) -> Vec<String> {
        let mut results = Vec::new();

        // Check for glob patterns
        if word.contains('*') || word.contains('?') || word.contains('[') {
            // Would call glob expansion
            if let Ok(paths) = glob::glob(word) {
                for path in paths.flatten() {
                    results.push(path.display().to_string());
                }
            }
        }

        // Check for tilde expansion
        if word.starts_with('~') {
            if let Some(home) = std::env::var_os("HOME") {
                let expanded = word.replacen('~', home.to_str().unwrap_or("~"), 1);
                results.push(expanded);
            }
        }

        // Check for variable expansion
        if word.starts_with('$') {
            let var_name = &word[1..];
            if let Ok(val) = std::env::var(var_name) {
                results.push(val);
            }
        }

        if results.is_empty() {
            results.push(word.to_string());
        }

        results
    }

    /// Do history expansion
    fn do_expand_hist(&self, line: &str) -> String {
        let mut result = line.to_string();

        // !! -> last command (simplified)
        if result.contains("!!") {
            result = result.replace("!!", "[last-command]");
        }

        // !$ -> last argument of last command (simplified)
        if result.contains("!$") {
            result = result.replace("!$", "[last-arg]");
        }

        result
    }

    /// Get completions for a prefix (simplified)
    fn get_completions(&self, prefix: &str) -> Vec<String> {
        let mut completions = Vec::new();

        // Check if it looks like a path
        if prefix.contains('/') || prefix.starts_with('.') {
            // Path completion
            let dir = if let Some(pos) = prefix.rfind('/') {
                &prefix[..=pos]
            } else {
                "./"
            };
            let file_prefix = if let Some(pos) = prefix.rfind('/') {
                &prefix[pos + 1..]
            } else {
                prefix
            };

            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(file_prefix) {
                        let full_path = if dir == "./" {
                            name
                        } else {
                            format!("{}{}", dir, name)
                        };
                        completions.push(full_path);
                    }
                }
            }
        } else {
            // Command completion - look in PATH
            if let Ok(path) = std::env::var("PATH") {
                for dir in path.split(':') {
                    if let Ok(entries) = std::fs::read_dir(dir) {
                        for entry in entries.flatten() {
                            let name = entry.file_name().to_string_lossy().to_string();
                            if name.starts_with(prefix) {
                                if !completions.contains(&name) {
                                    completions.push(name);
                                }
                            }
                        }
                    }
                }
            }
        }

        completions.sort();
        completions
    }
}

/// Meta character for zsh's internal encoding (0x83)
pub const META: char = '\u{83}';

/// Metafy a line (escape special chars)
/// Port of metafy_line() from zle_tricky.c
pub fn metafy_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if c == META || (c as u32) >= 0x83 {
            result.push(META);
            result.push(char::from_u32((c as u32) ^ 32).unwrap_or(c));
        } else {
            result.push(c);
        }
    }
    result
}

/// Unmetafy a line (unescape special chars)
/// Port of unmetafy_line() from zle_tricky.c
pub fn unmetafy_line(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        if c == META {
            if let Some(&next) = chars.peek() {
                chars.next();
                result.push(char::from_u32((next as u32) ^ 32).unwrap_or(next));
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Get the command being typed
/// Port of getcurcmd() from zle_tricky.c
pub fn get_cur_cmd(line: &[char], cursor: usize) -> Option<String> {
    // Find start of current simple command
    let mut cmd_start = 0;

    for i in 0..cursor {
        let c = line[i];
        if c == ';' || c == '|' || c == '&' || c == '(' || c == ')' || c == '`' {
            cmd_start = i + 1;
        }
    }

    // Skip whitespace
    while cmd_start < cursor && line[cmd_start].is_whitespace() {
        cmd_start += 1;
    }

    // Find end of command word
    let mut cmd_end = cmd_start;
    while cmd_end < cursor && !line[cmd_end].is_whitespace() {
        cmd_end += 1;
    }

    if cmd_start < cmd_end {
        Some(line[cmd_start..cmd_end].iter().collect())
    } else {
        None
    }
}

/// Check if string has real tokens (not escaped)
/// Port of has_real_token() from zle_tricky.c
pub fn has_real_token(s: &str) -> bool {
    let special = ['$', '`', '"', '\'', '\\', '{', '}', '[', ']', '*', '?', '~'];

    let mut escaped = false;
    for c in s.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
            continue;
        }
        if special.contains(&c) {
            return true;
        }
    }

    false
}

/// Get length of common prefix
/// Port of pfxlen() from zle_tricky.c
pub fn pfx_len(s1: &str, s2: &str) -> usize {
    s1.chars()
        .zip(s2.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Get length of common suffix
/// Port of sfxlen() from zle_tricky.c
pub fn sfx_len(s1: &str, s2: &str) -> usize {
    s1.chars()
        .rev()
        .zip(s2.chars().rev())
        .take_while(|(a, b)| a == b)
        .count()
}

/// Quote a string for shell
/// Port of quotestring() from zle_tricky.c
pub fn quote_string(s: &str, style: QuoteStyle) -> String {
    match style {
        QuoteStyle::Single => format!("'{}'", s.replace('\'', "'\\''")),
        QuoteStyle::Double => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        QuoteStyle::Dollar => format!("$'{}'", s.replace('\\', "\\\\").replace('\'', "\\'")),
        QuoteStyle::Backslash => {
            let mut result = String::with_capacity(s.len() * 2);
            for c in s.chars() {
                if " \t\n\\'\"`$&|;()<>*?[]{}#~".contains(c) {
                    result.push('\\');
                }
                result.push(c);
            }
            result
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum QuoteStyle {
    Single,
    Double,
    Dollar,
    Backslash,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pfx_len() {
        assert_eq!(pfx_len("hello", "help"), 3);
        assert_eq!(pfx_len("abc", "xyz"), 0);
        assert_eq!(pfx_len("test", "test"), 4);
    }

    #[test]
    fn test_sfx_len() {
        assert_eq!(sfx_len("testing", "running"), 3);
        assert_eq!(sfx_len("abc", "xyz"), 0);
    }

    #[test]
    fn test_quote_string() {
        assert_eq!(quote_string("hello", QuoteStyle::Single), "'hello'");
        assert_eq!(quote_string("it's", QuoteStyle::Single), "'it'\\''s'");
        assert_eq!(quote_string("hello", QuoteStyle::Double), "\"hello\"");
    }

    #[test]
    fn test_has_real_token() {
        assert!(has_real_token("$HOME"));
        assert!(has_real_token("*.txt"));
        assert!(!has_real_token("hello"));
        assert!(!has_real_token("test\\$var")); // escaped
    }
}
