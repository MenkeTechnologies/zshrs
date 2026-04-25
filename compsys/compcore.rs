//! Completion core - main entry point and match processing
//!
//! Ported from zsh Src/Zle/compcore.c with patterns from fish complete.rs

use crate::completion::{Completion, CompletionFlags, CompletionGroup};
use crate::state::CompParams;
use crate::zstyle::ZStyleStore;

/// Completion request options (inspired by fish)
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionRequestOptions {
    /// This is for autosuggestion (single best match)
    pub autosuggestion: bool,
    /// Generate descriptions for completions
    pub descriptions: bool,
    /// Allow fuzzy matching
    pub fuzzy_match: bool,
}

impl CompletionRequestOptions {
    pub fn autosuggest() -> Self {
        Self {
            autosuggestion: true,
            descriptions: false,
            fuzzy_match: false,
        }
    }

    pub fn normal() -> Self {
        Self {
            autosuggestion: false,
            descriptions: true,
            fuzzy_match: true,
        }
    }
}

/// Completion mode flags (from fish, maps to zsh options)
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionMode {
    /// Skip file completions
    pub no_files: bool,
    /// Force file completions even with other completions
    pub force_files: bool,
    /// Require a parameter after completion
    pub requires_param: bool,
}

/// Ambiguous match info (zsh ainfo/fainfo)
#[derive(Clone, Debug, Default)]
pub struct AmbiguousInfo {
    /// Unambiguous prefix
    pub prefix: String,
    /// Unambiguous suffix  
    pub suffix: String,
    /// Length of prefix
    pub prefix_len: usize,
    /// Length of suffix
    pub suffix_len: usize,
    /// Whether there's an exact match
    pub exact: bool,
    /// The exact match string
    pub exact_string: String,
}

/// Menu completion info
#[derive(Clone, Debug, Default)]
pub struct MenuInfo {
    /// Current match index (if in menu mode)
    pub current: Option<usize>,
    /// Group of current match
    pub group: Option<String>,
    /// Whether user was asked about listing
    pub asked: bool,
}

/// Global completion state during a completion operation
#[derive(Debug)]
pub struct CompletionState {
    /// The matches organized by group
    pub groups: Vec<CompletionGroup>,
    /// Total number of matches
    pub nmatches: usize,
    /// Number of matches to display (excludes hidden)
    pub smatches: usize,
    /// Whether there are different matches (not all identical)
    pub diff_matches: bool,
    /// Number of messages/explanations
    pub nmessages: usize,
    /// Ambiguous info for normal matches
    pub ainfo: AmbiguousInfo,
    /// Ambiguous info for fignore matches
    pub fainfo: AmbiguousInfo,
    /// Menu completion state
    pub minfo: MenuInfo,
    /// Has pattern matching
    pub has_pattern: bool,
    /// Has exact match
    pub has_exact: bool,
    /// Current completion mode
    pub mode: CompletionMode,
    /// Style store for zstyle lookups
    pub styles: ZStyleStore,
    /// Completion parameters
    pub params: CompParams,
    /// Ignored match count
    pub ignored: usize,
}

impl CompletionState {
    pub fn new() -> Self {
        Self {
            groups: Vec::new(),
            nmatches: 0,
            smatches: 0,
            diff_matches: false,
            nmessages: 0,
            ainfo: AmbiguousInfo::default(),
            fainfo: AmbiguousInfo::default(),
            minfo: MenuInfo::default(),
            has_pattern: false,
            has_exact: false,
            mode: CompletionMode::default(),
            styles: ZStyleStore::new(),
            params: CompParams::new(),
            ignored: 0,
        }
    }

    /// Initialize completion state from command line
    pub fn from_line(line: &str, cursor: usize) -> Self {
        let mut state = Self::new();
        state.params = CompParams::from_line(line, cursor);
        state
    }

    /// Get the current completion context string for zstyle
    pub fn context_string(&self) -> String {
        format!(
            ":completion:{}:{}:",
            self.params.compstate.context.as_str(),
            "" // completer name would go here
        )
    }

    /// Begin a new completion group
    pub fn begin_group(&mut self, name: &str, sorted: bool) {
        // Check if group already exists
        if let Some(group) = self.groups.iter_mut().find(|g| g.name == name) {
            group.sorted = sorted;
            return;
        }
        let group = if sorted {
            CompletionGroup::new(name)
        } else {
            CompletionGroup::new_unsorted(name)
        };
        self.groups.push(group);
    }

    /// End current group (finalize)
    pub fn end_group(&mut self) {
        // Sort if needed
        if let Some(group) = self.groups.last_mut() {
            if group.sorted {
                group.matches.sort_by(|a, b| a.str_.cmp(&b.str_));
            }
        }
    }

    /// Apply group-order style to reorder completion groups
    /// group-order values are group names in the desired order
    /// Groups not in the list appear at the end in original order
    pub fn apply_group_order(&mut self, order: &[String]) {
        if order.is_empty() {
            return;
        }

        let mut ordered: Vec<CompletionGroup> = Vec::with_capacity(self.groups.len());
        let mut remaining: Vec<CompletionGroup> = Vec::new();

        // First, add groups in the specified order
        for name in order {
            if let Some(pos) = self.groups.iter().position(|g| &g.name == name) {
                ordered.push(self.groups.remove(pos));
            }
        }

        // Then add remaining groups
        remaining.append(&mut self.groups);
        ordered.append(&mut remaining);

        self.groups = ordered;
    }

    /// Add a match to the current or specified group
    pub fn add_match(&mut self, comp: Completion, group_name: Option<&str>) {
        let group_name = group_name.unwrap_or("default");

        // Find or create group
        let group = if let Some(g) = self.groups.iter_mut().find(|g| g.name == group_name) {
            g
        } else {
            self.groups.push(CompletionGroup::new(group_name));
            self.groups.last_mut().unwrap()
        };

        // Track if matches differ
        if !group.matches.is_empty() && group.matches[0].str_ != comp.str_ {
            self.diff_matches = true;
        }

        // Add match
        if !comp.flags.contains(CompletionFlags::NOLIST) {
            self.smatches += 1;
        }
        self.nmatches += 1;
        group.add_match(comp);
    }

    /// Add an explanation/message to the current group
    pub fn add_explanation(&mut self, exp: String, group_name: Option<&str>) {
        let group_name = group_name.unwrap_or("default");
        if let Some(group) = self.groups.iter_mut().find(|g| g.name == group_name) {
            group.add_explanation(exp);
            self.nmessages += 1;
        }
    }

    /// Calculate the unambiguous prefix across all matches
    pub fn calculate_unambiguous(&mut self) {
        let all_matches: Vec<&Completion> =
            self.groups.iter().flat_map(|g| g.matches.iter()).collect();

        if all_matches.is_empty() {
            return;
        }

        if all_matches.len() == 1 {
            self.ainfo.prefix = all_matches[0].str_.clone();
            self.ainfo.prefix_len = self.ainfo.prefix.len();
            self.ainfo.exact = true;
            self.ainfo.exact_string = all_matches[0].str_.clone();
            return;
        }

        // Find common prefix
        let first = &all_matches[0].str_;
        let mut common_len = first.len();

        for m in &all_matches[1..] {
            let match_len = first
                .chars()
                .zip(m.str_.chars())
                .take_while(|(a, b)| a == b)
                .count();
            common_len = common_len.min(match_len);
        }

        self.ainfo.prefix = first.chars().take(common_len).collect();
        self.ainfo.prefix_len = common_len;

        // Check for exact match
        for m in &all_matches {
            if m.str_ == self.params.prefix {
                self.ainfo.exact = true;
                self.ainfo.exact_string = m.str_.clone();
                break;
            }
        }
    }

    /// Get all completions as a flat list
    pub fn all_completions(&self) -> Vec<&Completion> {
        self.groups.iter().flat_map(|g| g.matches.iter()).collect()
    }

    /// Update compstate based on current matches
    pub fn update_compstate(&mut self) {
        self.params.compstate.nmatches = self.nmatches as i32;
        self.params.compstate.ignored = self.ignored as i32;
        self.params.compstate.unambiguous = self.ainfo.prefix.clone();
        self.params.compstate.unambiguous_cursor = self.ainfo.prefix_len as i32;

        if self.ainfo.exact {
            self.params.compstate.exact_string = self.ainfo.exact_string.clone();
        }
    }
}

impl Default for CompletionState {
    fn default() -> Self {
        Self::new()
    }
}

/// Sort and prioritize completions (from fish, adapted for zsh groups)
pub fn sort_and_prioritize(groups: &mut [CompletionGroup], options: &CompletionRequestOptions) {
    for group in groups.iter_mut() {
        if group.matches.is_empty() {
            continue;
        }

        // Find best rank (if using fuzzy matching)
        if options.fuzzy_match {
            // For now, all matches have equal rank
            // TODO: implement fuzzy match ranking
        }

        // Deduplicate
        let mut seen = std::collections::HashSet::new();
        group.matches.retain(|c| seen.insert(c.str_.clone()));

        // Sort if the group is marked for sorting
        if group.sorted {
            group.matches.sort_by(|a, b| {
                // Files ending in ~ (autosave) sort last
                let a_tilde = a.str_.ends_with('~');
                let b_tilde = b.str_.ends_with('~');
                if a_tilde != b_tilde {
                    return a_tilde.cmp(&b_tilde);
                }
                a.str_.cmp(&b.str_)
            });
        }

        // Update lcount
        group.lcount = group
            .matches
            .iter()
            .filter(|m| !m.flags.contains(CompletionFlags::NOLIST))
            .count();
    }
}

/// Main completion entry point
///
/// This is called when completion is requested on a command line.
/// Returns the number of matches found.
pub fn do_completion(
    line: &str,
    cursor: usize,
    state: &mut CompletionState,
    completion_func: impl FnOnce(&mut CompletionState),
) -> usize {
    // Initialize state
    state.params = CompParams::from_line(line, cursor);
    state.nmatches = 0;
    state.smatches = 0;
    state.nmessages = 0;
    state.ignored = 0;
    state.groups.clear();

    // Call the completion function (this is where shell functions like _main_complete would run)
    completion_func(state);

    // Calculate unambiguous prefix
    state.calculate_unambiguous();

    // Sort and prioritize
    sort_and_prioritize(&mut state.groups, &CompletionRequestOptions::normal());

    // Update compstate
    state.update_compstate();

    state.nmatches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_completion_state_basic() {
        let mut state = CompletionState::from_line("git ch", 6);

        assert_eq!(state.params.prefix, "ch");
        assert_eq!(state.params.current, 2);

        state.begin_group("commands", true);
        state.add_match(Completion::new("checkout"), Some("commands"));
        state.add_match(Completion::new("cherry-pick"), Some("commands"));

        assert_eq!(state.nmatches, 2);
        assert!(state.diff_matches);

        state.calculate_unambiguous();
        // "checkout" and "cherry-pick" share "che" as common prefix
        assert_eq!(state.ainfo.prefix, "che");
    }

    #[test]
    fn test_unambiguous_single_match() {
        let mut state = CompletionState::new();
        state.add_match(Completion::new("foobar"), None);
        state.calculate_unambiguous();

        assert_eq!(state.ainfo.prefix, "foobar");
        assert!(state.ainfo.exact);
    }

    #[test]
    fn test_unambiguous_multiple_matches() {
        let mut state = CompletionState::new();
        state.add_match(Completion::new("checkout"), None);
        state.add_match(Completion::new("cherry-pick"), None);
        state.add_match(Completion::new("clean"), None);
        state.calculate_unambiguous();

        assert_eq!(state.ainfo.prefix, "c");
        assert!(!state.ainfo.exact);
    }

    #[test]
    fn test_sort_and_prioritize() {
        let mut groups = vec![CompletionGroup::new("test")];
        groups[0].matches = vec![
            Completion::new("zebra"),
            Completion::new("alpha"),
            Completion::new("beta"),
            Completion::new("alpha"), // duplicate
        ];

        sort_and_prioritize(&mut groups, &CompletionRequestOptions::normal());

        assert_eq!(groups[0].matches.len(), 3); // deduplicated
        assert_eq!(groups[0].matches[0].str_, "alpha");
        assert_eq!(groups[0].matches[1].str_, "beta");
        assert_eq!(groups[0].matches[2].str_, "zebra");
    }

    #[test]
    fn test_apply_group_order() {
        let mut state = CompletionState::new();

        state.begin_group("files", true);
        state.add_match(Completion::new("file1"), Some("files"));
        state.end_group();

        state.begin_group("directories", true);
        state.add_match(Completion::new("dir1"), Some("directories"));
        state.end_group();

        state.begin_group("commands", true);
        state.add_match(Completion::new("cmd1"), Some("commands"));
        state.end_group();

        // Original order: files, directories, commands
        assert_eq!(state.groups[0].name, "files");
        assert_eq!(state.groups[1].name, "directories");
        assert_eq!(state.groups[2].name, "commands");

        // Apply custom order: commands first, then directories
        state.apply_group_order(&["commands".to_string(), "directories".to_string()]);

        // New order: commands, directories, files (files at end since not in order list)
        assert_eq!(state.groups[0].name, "commands");
        assert_eq!(state.groups[1].name, "directories");
        assert_eq!(state.groups[2].name, "files");
    }

    #[test]
    fn test_apply_group_order_empty() {
        let mut state = CompletionState::new();

        state.begin_group("files", true);
        state.add_match(Completion::new("file1"), Some("files"));
        state.end_group();

        state.begin_group("commands", true);
        state.add_match(Completion::new("cmd1"), Some("commands"));
        state.end_group();

        // Empty order should not change anything
        state.apply_group_order(&[]);

        assert_eq!(state.groups[0].name, "files");
        assert_eq!(state.groups[1].name, "commands");
    }

    #[test]
    fn test_apply_group_order_nonexistent_groups() {
        let mut state = CompletionState::new();

        state.begin_group("files", true);
        state.add_match(Completion::new("file1"), Some("files"));
        state.end_group();

        // Order includes non-existent group
        state.apply_group_order(&["nonexistent".to_string(), "files".to_string()]);

        // Should still work, just files at the start (from order)
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].name, "files");
    }

    #[test]
    fn test_completion_state_multiple_groups() {
        let mut state = CompletionState::new();

        state.begin_group("options", true);
        state.add_match(Completion::new("--help"), Some("options"));
        state.add_match(Completion::new("--version"), Some("options"));
        state.end_group();

        state.begin_group("files", true);
        state.add_match(Completion::new("foo.txt"), Some("files"));
        state.end_group();

        assert_eq!(state.groups.len(), 2);
        assert_eq!(state.nmatches, 3);
        assert_eq!(state.groups[0].matches.len(), 2);
        assert_eq!(state.groups[1].matches.len(), 1);
    }

    #[test]
    fn test_completion_state_add_to_existing_group() {
        let mut state = CompletionState::new();

        state.begin_group("files", true);
        state.add_match(Completion::new("a.txt"), Some("files"));
        state.end_group();

        // Add more to the same group
        state.begin_group("files", true);
        state.add_match(Completion::new("b.txt"), Some("files"));
        state.end_group();

        // Should be one group with two matches
        assert_eq!(state.groups.len(), 1);
        assert_eq!(state.groups[0].matches.len(), 2);
    }

    #[test]
    fn test_completion_state_explanation() {
        let mut state = CompletionState::new();

        state.begin_group("files", true);
        state.add_explanation("Select a file".to_string(), Some("files"));
        state.add_match(Completion::new("test.txt"), Some("files"));
        state.end_group();

        // add_explanation adds to explanations vec, not the explanation field
        assert!(state.groups[0]
            .explanations
            .contains(&"Select a file".to_string()));
    }

    #[test]
    fn test_do_completion() {
        let mut state = CompletionState::new();

        let matches = do_completion("git ch", 6, &mut state, |s| {
            s.add_match(Completion::new("checkout"), None);
            s.add_match(Completion::new("cherry-pick"), None);
        });

        assert_eq!(matches, 2);
        assert_eq!(state.ainfo.prefix, "che");
    }

    #[test]
    fn test_completion_mode_default() {
        let mode = CompletionMode::default();
        assert!(!mode.no_files);
        assert!(!mode.force_files);
        assert!(!mode.requires_param);
    }

    #[test]
    fn test_comp_params_from_line_empty() {
        let params = CompParams::from_line("", 0);
        assert_eq!(params.words.len(), 1);
        assert_eq!(params.words[0], "");
        assert_eq!(params.current, 1);
        assert_eq!(params.prefix, "");
        assert_eq!(params.suffix, "");
    }

    #[test]
    fn test_comp_params_from_line_single_word() {
        let params = CompParams::from_line("git", 3);
        assert_eq!(params.words, vec!["git"]);
        assert_eq!(params.current, 1);
        assert_eq!(params.prefix, "git");
    }

    #[test]
    fn test_comp_params_from_line_multiple_words() {
        let params = CompParams::from_line("git checkout main", 17);
        assert_eq!(params.words, vec!["git", "checkout", "main"]);
        assert_eq!(params.current, 3);
        assert_eq!(params.prefix, "main");
    }

    #[test]
    fn test_comp_params_cursor_mid_word() {
        let params = CompParams::from_line("git check", 7);
        assert_eq!(params.current, 2);
        assert_eq!(params.prefix, "che");
        assert_eq!(params.suffix, "ck");
    }

    #[test]
    fn test_comp_params_cursor_between_words() {
        let params = CompParams::from_line("git checkout ", 13);
        assert_eq!(params.current, 3);
        assert_eq!(params.prefix, "");
    }

    #[test]
    fn test_unambiguous_no_matches() {
        let mut state = CompletionState::new();
        state.calculate_unambiguous();

        assert_eq!(state.ainfo.prefix, "");
        assert!(!state.ainfo.exact);
    }

    #[test]
    fn test_unambiguous_common_prefix() {
        let mut state = CompletionState::new();
        state.add_match(Completion::new("foobar"), None);
        state.add_match(Completion::new("foobaz"), None);
        state.add_match(Completion::new("fooqux"), None);
        state.calculate_unambiguous();

        assert_eq!(state.ainfo.prefix, "foo");
        assert!(!state.ainfo.exact);
    }

    #[test]
    fn test_compstate_context_string() {
        let mut state = CompletionState::new();
        state.params.compstate.context = crate::state::CompletionContext::Command;

        let ctx = state.context_string();
        assert!(ctx.contains("completion"));
    }
}
