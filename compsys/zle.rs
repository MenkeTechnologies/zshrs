//! ZLE (Zsh Line Editor) completion widget implementations
//!
//! These functions implement the completion-related ZLE widgets from `man zshzle`:
//! - complete-word
//! - expand-or-complete  
//! - expand-or-complete-prefix
//! - menu-complete
//! - reverse-menu-complete
//! - accept-and-menu-complete
//! - delete-char-or-list
//! - list-choices
//! - list-expand
//! - expand-word
//! - expand-cmd-path
//! - expand-history
//! - magic-space
//! - menu-expand-or-complete
//! - end-of-list

use crate::{CompletionGroup, MenuAction, MenuResult, MenuState};

/// ZLE completion state - tracks menu completion across widget calls
#[derive(Debug, Clone)]
pub struct ZleCompletionState {
    /// Menu state for interactive completion
    pub menu: MenuState,
    /// Whether menu completion is active
    pub menu_active: bool,
    /// Original word before completion started
    pub original_word: String,
    /// Current completions
    pub completions: Vec<CompletionGroup>,
    /// Last completion action
    pub last_action: Option<ZleAction>,
    /// Whether list is currently displayed
    pub list_displayed: bool,
}

/// ZLE completion action results
#[derive(Debug, Clone)]
pub enum ZleAction {
    /// No completions found
    NoMatch,
    /// Single completion - inserted directly
    SingleMatch(String),
    /// Multiple completions - show menu or list
    MultipleMatches,
    /// Completion inserted, menu still active for cycling
    MenuCycle,
    /// Completion accepted, menu closed
    MenuAccept(String),
    /// Display completion list (don't insert)
    ListOnly,
    /// Expansion performed
    Expanded(String),
    /// Beep (ambiguous or error)
    Beep,
    /// Refresh display
    Refresh,
}

impl Default for ZleCompletionState {
    fn default() -> Self {
        Self::new()
    }
}

impl ZleCompletionState {
    pub fn new() -> Self {
        Self {
            menu: MenuState::new(),
            menu_active: false,
            original_word: String::new(),
            completions: Vec::new(),
            last_action: None,
            list_displayed: false,
        }
    }

    /// Reset completion state (called when line changes significantly)
    pub fn reset(&mut self) {
        self.menu_active = false;
        self.original_word.clear();
        self.completions.clear();
        self.last_action = None;
        self.list_displayed = false;
    }

    /// Set terminal size for menu rendering
    pub fn set_term_size(&mut self, width: usize, height: usize) {
        self.menu.set_term_size(width, height);
    }

    /// Set available rows for completion display
    pub fn set_available_rows(&mut self, rows: usize) {
        self.menu.set_available_rows(rows);
    }
}

/// ZLE completion widget implementations
pub struct ZleWidgets;

impl ZleWidgets {
    // =========================================================================
    // complete-word
    // Attempt completion on the current word.
    // =========================================================================
    pub fn complete_word(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        completions: Vec<CompletionGroup>,
    ) -> (ZleAction, Option<String>) {
        let word = Self::word_at_cursor(buffer, cursor);
        state.original_word = word.clone();
        state.completions = completions;

        let total_matches: usize = state.completions.iter().map(|g| g.matches.len()).sum();

        if total_matches == 0 {
            state.last_action = Some(ZleAction::NoMatch);
            return (ZleAction::NoMatch, None);
        }

        if total_matches == 1 {
            let completion = &state.completions[0].matches[0];
            let insert = completion.insert_str();
            state.last_action = Some(ZleAction::SingleMatch(insert.clone()));
            return (ZleAction::SingleMatch(insert.clone()), Some(insert));
        }

        // Multiple matches - find common prefix
        let common = Self::find_common_prefix(&state.completions);
        if common.len() > word.len() {
            // Can extend with common prefix
            state.last_action = Some(ZleAction::Expanded(common.clone()));
            return (ZleAction::Expanded(common.clone()), Some(common));
        }

        // Ambiguous - start menu or list
        state.menu.set_prefix(&word);
        state.menu.set_completions(&state.completions);
        state.menu.set_show_headers(true);
        state.list_displayed = true;
        state.last_action = Some(ZleAction::MultipleMatches);
        (ZleAction::MultipleMatches, None)
    }

    // =========================================================================
    // expand-or-complete (TAB)
    // Attempt shell expansion on the current word. If that fails, attempt completion.
    // =========================================================================
    pub fn expand_or_complete(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        completions: Vec<CompletionGroup>,
        try_expand: impl FnOnce(&str) -> Option<String>,
    ) -> (ZleAction, Option<String>) {
        let word = Self::word_at_cursor(buffer, cursor);

        // First try expansion (glob, brace, tilde, etc.)
        if let Some(expanded) = try_expand(&word) {
            if expanded != word {
                state.last_action = Some(ZleAction::Expanded(expanded.clone()));
                return (ZleAction::Expanded(expanded.clone()), Some(expanded));
            }
        }

        // Fall back to completion
        Self::complete_word(state, buffer, cursor, completions)
    }

    // =========================================================================
    // expand-or-complete-prefix
    // Attempt shell expansion on the current word up to cursor.
    // =========================================================================
    pub fn expand_or_complete_prefix(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        completions: Vec<CompletionGroup>,
        try_expand: impl FnOnce(&str) -> Option<String>,
    ) -> (ZleAction, Option<String>) {
        // Get word up to cursor only (not including suffix after cursor)
        let word = Self::word_before_cursor(buffer, cursor);

        if let Some(expanded) = try_expand(&word) {
            if expanded != word {
                state.last_action = Some(ZleAction::Expanded(expanded.clone()));
                return (ZleAction::Expanded(expanded.clone()), Some(expanded));
            }
        }

        Self::complete_word(state, buffer, cursor, completions)
    }

    // =========================================================================
    // menu-complete
    // Like complete-word, except that menu completion is used.
    // =========================================================================
    pub fn menu_complete(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        completions: Vec<CompletionGroup>,
    ) -> (ZleAction, Option<String>) {
        if !state.menu_active {
            // Start menu completion
            let word = Self::word_at_cursor(buffer, cursor);
            state.original_word = word.clone();
            state.completions = completions;

            let total: usize = state.completions.iter().map(|g| g.matches.len()).sum();
            if total == 0 {
                return (ZleAction::NoMatch, None);
            }

            state.menu.set_prefix(&word);
            state.menu.set_completions(&state.completions);
            state.menu.start();
            state.menu_active = true;
        } else {
            // Cycle to next
            state.menu.process_action(MenuAction::Next);
        }

        // Get current selection
        if let Some(insert) = state.menu.selected_insert_string() {
            state.last_action = Some(ZleAction::MenuCycle);
            (ZleAction::MenuCycle, Some(insert))
        } else {
            (ZleAction::NoMatch, None)
        }
    }

    // =========================================================================
    // reverse-menu-complete
    // Perform menu completion, moving to previous completion.
    // =========================================================================
    pub fn reverse_menu_complete(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        completions: Vec<CompletionGroup>,
    ) -> (ZleAction, Option<String>) {
        if !state.menu_active {
            // Start menu completion at end
            let word = Self::word_at_cursor(buffer, cursor);
            state.original_word = word.clone();
            state.completions = completions;

            let total: usize = state.completions.iter().map(|g| g.matches.len()).sum();
            if total == 0 {
                return (ZleAction::NoMatch, None);
            }

            state.menu.set_prefix(&word);
            state.menu.set_completions(&state.completions);
            state.menu.start();
            state.menu.process_action(MenuAction::End); // Start at end
            state.menu_active = true;
        } else {
            // Cycle to previous
            state.menu.process_action(MenuAction::Prev);
        }

        if let Some(insert) = state.menu.selected_insert_string() {
            state.last_action = Some(ZleAction::MenuCycle);
            (ZleAction::MenuCycle, Some(insert))
        } else {
            (ZleAction::NoMatch, None)
        }
    }

    // =========================================================================
    // accept-and-menu-complete
    // In a menu completion, insert the current completion and advance to next.
    // =========================================================================
    pub fn accept_and_menu_complete(state: &mut ZleCompletionState) -> (ZleAction, Option<String>) {
        if !state.menu_active {
            return (ZleAction::NoMatch, None);
        }

        match state.menu.process_action(MenuAction::AcceptAndMenuComplete) {
            MenuResult::AcceptAndHold(s) => {
                state.last_action = Some(ZleAction::MenuAccept(s.clone()));
                (ZleAction::MenuAccept(s.clone()), Some(s))
            }
            _ => (ZleAction::NoMatch, None),
        }
    }

    // =========================================================================
    // delete-char-or-list (^D)
    // Delete the character under the cursor. If at end of line, list completions.
    // =========================================================================
    pub fn delete_char_or_list(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        at_eol: bool,
        completions: Vec<CompletionGroup>,
    ) -> (ZleAction, Option<String>) {
        if at_eol || cursor >= buffer.len() {
            // At end of line - list completions
            Self::list_choices(state, buffer, cursor, completions)
        } else {
            // Not at EOL - delete char (handled by caller)
            (ZleAction::Refresh, None)
        }
    }

    // =========================================================================
    // list-choices (ESC-^D)
    // List possible completions for the current word.
    // =========================================================================
    pub fn list_choices(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        completions: Vec<CompletionGroup>,
    ) -> (ZleAction, Option<String>) {
        let word = Self::word_at_cursor(buffer, cursor);
        state.completions = completions;

        let total: usize = state.completions.iter().map(|g| g.matches.len()).sum();
        if total == 0 {
            state.last_action = Some(ZleAction::NoMatch);
            return (ZleAction::NoMatch, None);
        }

        state.menu.set_prefix(&word);
        state.menu.set_completions(&state.completions);
        state.menu.set_show_headers(true);
        state.list_displayed = true;
        state.last_action = Some(ZleAction::ListOnly);
        (ZleAction::ListOnly, None)
    }

    // =========================================================================
    // list-expand (^Xg ^XG)
    // List the expansion of the current word.
    // =========================================================================
    pub fn list_expand(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        try_expand: impl FnOnce(&str) -> Vec<String>,
    ) -> (ZleAction, Option<String>) {
        let word = Self::word_at_cursor(buffer, cursor);
        let expansions = try_expand(&word);

        if expansions.is_empty() {
            return (ZleAction::NoMatch, None);
        }

        // Convert expansions to completion group
        let mut group = CompletionGroup::new("expansions");
        group.explanation = Some("expansions".to_string());
        for exp in expansions {
            group.matches.push(crate::Completion::new(exp));
        }

        state.completions = vec![group];
        state.menu.set_prefix(&word);
        state.menu.set_completions(&state.completions);
        state.list_displayed = true;
        state.last_action = Some(ZleAction::ListOnly);
        (ZleAction::ListOnly, None)
    }

    // =========================================================================
    // expand-word (^X*)
    // Attempt shell expansion on the current word.
    // =========================================================================
    pub fn expand_word(
        buffer: &str,
        cursor: usize,
        try_expand: impl FnOnce(&str) -> Option<String>,
    ) -> (ZleAction, Option<String>) {
        let word = Self::word_at_cursor(buffer, cursor);

        if let Some(expanded) = try_expand(&word) {
            if expanded != word {
                return (ZleAction::Expanded(expanded.clone()), Some(expanded));
            }
        }

        (ZleAction::NoMatch, None)
    }

    // =========================================================================
    // expand-cmd-path
    // Expand the current command to its full pathname.
    // =========================================================================
    pub fn expand_cmd_path(
        buffer: &str,
        cursor: usize,
        find_command: impl FnOnce(&str) -> Option<String>,
    ) -> (ZleAction, Option<String>) {
        let word = Self::word_at_cursor(buffer, cursor);

        if let Some(path) = find_command(&word) {
            if path != word {
                return (ZleAction::Expanded(path.clone()), Some(path));
            }
        }

        (ZleAction::NoMatch, None)
    }

    // =========================================================================
    // expand-history (ESC-space ESC-!)
    // Perform history expansion on the edit buffer.
    // =========================================================================
    pub fn expand_history(
        buffer: &str,
        expand_fn: impl FnOnce(&str) -> Option<String>,
    ) -> (ZleAction, Option<String>) {
        if let Some(expanded) = expand_fn(buffer) {
            if expanded != buffer {
                return (ZleAction::Expanded(expanded.clone()), Some(expanded));
            }
        }
        (ZleAction::NoMatch, None)
    }

    // =========================================================================
    // magic-space
    // Perform history expansion and insert a space into the buffer.
    // =========================================================================
    pub fn magic_space(
        buffer: &str,
        expand_fn: impl FnOnce(&str) -> Option<String>,
    ) -> (ZleAction, Option<String>) {
        if let Some(expanded) = expand_fn(buffer) {
            let with_space = format!("{} ", expanded);
            return (ZleAction::Expanded(with_space.clone()), Some(with_space));
        }
        // No expansion - just insert space
        (ZleAction::Expanded(" ".to_string()), Some(" ".to_string()))
    }

    // =========================================================================
    // menu-expand-or-complete
    // Like expand-or-complete, except that menu completion is used.
    // =========================================================================
    pub fn menu_expand_or_complete(
        state: &mut ZleCompletionState,
        buffer: &str,
        cursor: usize,
        completions: Vec<CompletionGroup>,
        try_expand: impl FnOnce(&str) -> Option<String>,
    ) -> (ZleAction, Option<String>) {
        let word = Self::word_at_cursor(buffer, cursor);

        // First try expansion
        if let Some(expanded) = try_expand(&word) {
            if expanded != word {
                return (ZleAction::Expanded(expanded.clone()), Some(expanded));
            }
        }

        // Fall back to menu completion
        Self::menu_complete(state, buffer, cursor, completions)
    }

    // =========================================================================
    // end-of-list
    // When a previous completion displayed a list below the prompt,
    // move the prompt below the list.
    // =========================================================================
    pub fn end_of_list(state: &mut ZleCompletionState) -> ZleAction {
        if state.list_displayed {
            // Signal to move prompt below list
            ZleAction::Refresh
        } else {
            ZleAction::NoMatch
        }
    }

    // =========================================================================
    // Helper functions
    // =========================================================================

    /// Extract word at cursor position
    fn word_at_cursor(buffer: &str, cursor: usize) -> String {
        let cursor = cursor.min(buffer.len());

        // Find word start
        let start = buffer[..cursor]
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);

        // Find word end
        let end = buffer[cursor..]
            .find(|c: char| c.is_whitespace())
            .map(|i| cursor + i)
            .unwrap_or(buffer.len());

        buffer[start..end].to_string()
    }

    /// Extract word before cursor (for prefix expansion)
    fn word_before_cursor(buffer: &str, cursor: usize) -> String {
        let cursor = cursor.min(buffer.len());

        let start = buffer[..cursor]
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(0);

        buffer[start..cursor].to_string()
    }

    /// Find longest common prefix among all completions
    fn find_common_prefix(groups: &[CompletionGroup]) -> String {
        let all_matches: Vec<&str> = groups
            .iter()
            .flat_map(|g| g.matches.iter().map(|m| m.str_.as_str()))
            .collect();

        if all_matches.is_empty() {
            return String::new();
        }

        if all_matches.len() == 1 {
            return all_matches[0].to_string();
        }

        let first = all_matches[0];
        let mut prefix_len = first.len();

        for s in &all_matches[1..] {
            let common = first
                .chars()
                .zip(s.chars())
                .take_while(|(a, b)| a.eq_ignore_ascii_case(b))
                .count();
            prefix_len = prefix_len.min(common);
        }

        first[..prefix_len].to_string()
    }
}

/// Menu navigation for ZLE
impl ZleCompletionState {
    /// Process a menu navigation action
    pub fn menu_navigate(&mut self, action: MenuAction) -> Option<String> {
        if !self.menu_active {
            return None;
        }

        match self.menu.process_action(action) {
            MenuResult::Accept(s) => {
                self.menu_active = false;
                Some(s)
            }
            MenuResult::AcceptAndHold(s) => Some(s),
            MenuResult::Cancel => {
                self.menu_active = false;
                None
            }
            MenuResult::Continue => self.menu.selected_insert_string(),
            _ => None,
        }
    }

    /// Get current menu selection for display
    pub fn current_selection(&self) -> Option<String> {
        if self.menu_active {
            self.menu.selected_insert_string()
        } else {
            None
        }
    }

    /// Render the completion menu
    pub fn render_menu(&mut self) -> crate::MenuRendering {
        self.menu.render()
    }

    /// Check if menu is active
    pub fn is_menu_active(&self) -> bool {
        self.menu_active
    }

    /// Cancel menu completion
    pub fn cancel_menu(&mut self) {
        self.menu_active = false;
        self.list_displayed = false;
    }

    /// Get completion count
    pub fn completion_count(&self) -> usize {
        self.menu.count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_word_at_cursor() {
        assert_eq!(ZleWidgets::word_at_cursor("hello world", 5), "hello");
        assert_eq!(ZleWidgets::word_at_cursor("hello world", 6), "world");
        assert_eq!(ZleWidgets::word_at_cursor("hello world", 11), "world");
        assert_eq!(ZleWidgets::word_at_cursor("hello", 3), "hello");
        assert_eq!(ZleWidgets::word_at_cursor("", 0), "");
    }

    #[test]
    fn test_word_before_cursor() {
        assert_eq!(ZleWidgets::word_before_cursor("hello world", 5), "hello");
        assert_eq!(ZleWidgets::word_before_cursor("hello world", 8), "wo");
        assert_eq!(ZleWidgets::word_before_cursor("hello", 3), "hel");
    }

    #[test]
    fn test_find_common_prefix() {
        let mut g = CompletionGroup::new("test");
        g.matches.push(crate::Completion::new("hello"));
        g.matches.push(crate::Completion::new("help"));
        g.matches.push(crate::Completion::new("helicopter"));

        assert_eq!(ZleWidgets::find_common_prefix(&[g]), "hel");
    }

    #[test]
    fn test_complete_word_no_matches() {
        let mut state = ZleCompletionState::new();
        let (action, insert) = ZleWidgets::complete_word(&mut state, "xyz", 3, vec![]);
        assert!(matches!(action, ZleAction::NoMatch));
        assert!(insert.is_none());
    }

    #[test]
    fn test_complete_word_single_match() {
        let mut state = ZleCompletionState::new();
        let mut g = CompletionGroup::new("test");
        g.matches.push(crate::Completion::new("hello"));

        let (action, insert) = ZleWidgets::complete_word(&mut state, "hel", 3, vec![g]);
        assert!(matches!(action, ZleAction::SingleMatch(_)));
        assert_eq!(insert, Some("hello".into()));
    }

    #[test]
    fn test_menu_complete_cycle() {
        let mut state = ZleCompletionState::new();
        let mut g = CompletionGroup::new("test");
        g.matches.push(crate::Completion::new("aaa"));
        g.matches.push(crate::Completion::new("aab"));
        g.matches.push(crate::Completion::new("aac"));

        // First call starts menu
        let (_, insert1) = ZleWidgets::menu_complete(&mut state, "aa", 2, vec![g.clone()]);
        assert!(state.menu_active);
        assert_eq!(insert1, Some("aaa".into()));

        // Second call cycles
        let (_, insert2) = ZleWidgets::menu_complete(&mut state, "aa", 2, vec![g.clone()]);
        assert_eq!(insert2, Some("aab".into()));

        // Third call cycles
        let (_, insert3) = ZleWidgets::menu_complete(&mut state, "aa", 2, vec![g]);
        assert_eq!(insert3, Some("aac".into()));
    }
}
