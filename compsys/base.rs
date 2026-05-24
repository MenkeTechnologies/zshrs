//! Shared type definitions for the Base/ completion subsystem.
//!
//! Historically this module hosted both the type definitions
//! (`MainCompleteState`, `TagManager`, `CompletionContext`,
//! `CompleterResult`, `Value`, `Alternative`) AND inline ports of
//! every Base/ shell function (`_main_complete`, `_normal`,
//! `_alternative`, `_values`, `_description`, `_message`, etc.).
//! The per-function bodies have now been extracted into
//! `compsys/fns/Base/{Core,Completer,Utility}/_NAME.rs` to mirror
//! zsh's `Completion/Base/{Core,Completer,Utility,Widget}/` taxonomy
//! one-file-per-shell-function.
//!
//! What lives here now:
//!   * `MainCompleteState` — global state object passed through every
//!     completer.
//!   * `CompletionContext` — current `:completion:…:…:` context.
//!   * `TagManager` — tag offering / requested / wanted accounting.
//!   * `CompleterResult` — `Matched` / `NoMatch` / `Skip`.
//!   * `Value`, `Alternative` — spec parsers shared by `_values` /
//!     `_alternative`.
//!   * Re-exports of every extracted Base/ fn so external callers
//!     spelled `compsys::base::_main_complete` (and the
//!     `compsys::base::{…}` re-exports in `compsys/lib.rs`) continue
//!     to resolve.

use crate::compcore::CompletionState;
use std::collections::{HashMap, HashSet};

use crate::zstyle::ZStyleStore;

// =============================================================================
// Core context / state types
// =============================================================================

/// Completion context for tag-based completion
#[derive(Clone, Debug, Default)]
pub struct CompletionContext {
    /// Current context string (e.g., ":completion::complete:git:")
    pub context: String,
    /// Current completer being used
    pub completer: String,
    /// Completer index (1-based)
    pub completer_num: usize,
    /// Matcher specification
    pub matcher: String,
    /// Matcher index (1-based)
    pub matcher_num: usize,
}

/// Tag set management for completion
#[derive(Clone, Debug, Default)]
pub struct TagManager {
    /// All offered tags for this completion
    offered: Vec<String>,
    /// Tag sets to try, in order
    try_sets: Vec<Vec<String>>,
    /// Current try index
    current_try: usize,
    /// Tags currently being tried
    current_tags: HashSet<String>,
    /// Tags that have been requested
    requested: HashSet<String>,
}

impl TagManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize tags (_tags with arguments)
    /// Called at start of completion to declare available tags
    pub fn init(&mut self, tags: &[String]) {
        self.offered = tags.to_vec();
        self.try_sets.clear();
        self.current_try = 0;
        self.current_tags.clear();
        self.requested.clear();
    }

    /// Configure tag order from zstyle 'tag-order'
    /// Format: Each value is a space-separated list of tags to try together
    /// Special: "-" means don't try remaining tags
    /// Example: "files directories" "arguments" "-"
    pub fn configure_from_style(&mut self, tag_order: &[String]) {
        self.try_sets.clear();

        for group in tag_order {
            if group == "-" {
                break;
            }

            let tags: Vec<String> = group
                .split_whitespace()
                .filter(|t| self.offered.contains(&t.to_string()))
                .map(|s| s.to_string())
                .collect();

            if !tags.is_empty() {
                self.try_sets.push(tags);
            }
        }

        // If no tag-order or all filtered, use default (all offered at once)
        if self.try_sets.is_empty() {
            self.try_sets.push(self.offered.clone());
        }
    }

    /// Add a tag set to try (comptry)
    pub fn add_try(&mut self, tags: &[String]) {
        let available: Vec<String> = tags
            .iter()
            .filter(|t| self.offered.contains(t))
            .cloned()
            .collect();
        if !available.is_empty() {
            self.try_sets.push(available);
        }
    }

    /// Start trying tags - returns true if there are tags to try
    pub fn start(&mut self) -> bool {
        self.current_try = 0;
        self.load_current_set();
        !self.current_tags.is_empty()
    }

    /// Move to next tag set (_tags with no arguments)
    /// Returns true if there are more tags
    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> bool {
        self.current_try += 1;
        self.load_current_set();
        !self.current_tags.is_empty()
    }

    fn load_current_set(&mut self) {
        self.current_tags.clear();
        if self.current_try < self.try_sets.len() {
            for tag in &self.try_sets[self.current_try] {
                self.current_tags.insert(tag.clone());
            }
        }
    }

    /// Check if a tag is being tried (_requested)
    pub fn requested(&mut self, tag: &str) -> bool {
        if self.current_tags.contains(tag) {
            self.requested.insert(tag.to_string());
            true
        } else {
            false
        }
    }

    /// Check if a tag was requested without marking it (_wanted)
    pub fn wanted(&self, tag: &str) -> bool {
        self.current_tags.contains(tag)
    }

    /// Get all currently active tags
    pub fn current(&self) -> &HashSet<String> {
        &self.current_tags
    }
}

/// Result from a completer function
#[derive(Clone, Debug)]
pub enum CompleterResult {
    /// Matches were added
    Matched,
    /// No matches, but not an error
    NoMatch,
    /// Skip remaining completers
    Skip,
}

/// Completer function type
pub type CompleterFn = fn(&mut MainCompleteState) -> CompleterResult;

/// State for _main_complete
#[derive(Debug)]
pub struct MainCompleteState {
    /// Completion state
    pub comp: CompletionState,
    /// Style store
    pub styles: ZStyleStore,
    /// Tag manager
    pub tags: TagManager,
    /// Context
    pub ctx: CompletionContext,
    /// Completers to use
    pub completers: Vec<String>,
    /// Last completion info
    pub lastcomp: HashMap<String, String>,
    /// Pre-completion functions
    pub prefuncs: Vec<String>,
    /// Post-completion functions
    pub postfuncs: Vec<String>,
    /// Return value
    pub ret: i32,
}

impl MainCompleteState {
    pub fn new(line: &str, cursor: usize) -> Self {
        Self {
            comp: CompletionState::from_line(line, cursor),
            styles: ZStyleStore::new(),
            tags: TagManager::new(),
            ctx: CompletionContext::default(),
            completers: vec!["_complete".to_string(), "_ignored".to_string()],
            lastcomp: HashMap::new(),
            prefuncs: Vec::new(),
            postfuncs: Vec::new(),
            ret: 1,
        }
    }

    /// Get the current context string for zstyle lookups
    pub fn context_string(&self) -> String {
        format!(":completion:{}:{}:", self.ctx.context, self.ctx.completer)
    }
}

// =============================================================================
// Spec types used by _alternative / _values
// =============================================================================

/// Alternative specification — parses "tag:description:action".
#[derive(Clone, Debug)]
pub struct Alternative {
    pub tag: String,
    pub description: String,
    pub action: String,
}

impl Alternative {
    /// Parse "tag:description:action" format
    pub fn parse(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.splitn(3, ':').collect();
        if parts.len() < 3 {
            return None;
        }
        Some(Self {
            tag: parts[0].to_string(),
            description: parts[1].to_string(),
            action: parts[2].to_string(),
        })
    }
}

/// Value with optional argument for _values
#[derive(Clone, Debug)]
pub struct Value {
    pub name: String,
    pub description: String,
    pub has_arg: bool,
    pub arg_description: String,
    pub action: String,
}

impl Value {
    /// Parse "name\[description\]:arg-desc:action" format
    pub fn parse(spec: &str) -> Option<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        // Parse name[description]
        let (name, rest) = if let Some(bracket_start) = spec.find('[') {
            if let Some(bracket_end) = spec[bracket_start..].find(']') {
                let name = spec[..bracket_start].to_string();
                let desc = spec[bracket_start + 1..bracket_start + bracket_end].to_string();
                let rest = &spec[bracket_start + bracket_end + 1..];
                (name, (desc, rest))
            } else {
                (spec.to_string(), (String::new(), ""))
            }
        } else if let Some(colon) = spec.find(':') {
            (spec[..colon].to_string(), (String::new(), &spec[colon..]))
        } else {
            (spec.to_string(), (String::new(), ""))
        };

        let (description, rest) = rest;

        // Parse :arg-desc:action
        let (has_arg, arg_description, action) = if let Some(after_colon) = rest.strip_prefix(':') {
            let parts: Vec<&str> = after_colon.splitn(2, ':').collect();
            (
                true,
                parts.first().unwrap_or(&"").to_string(),
                parts.get(1).unwrap_or(&"").to_string(),
            )
        } else {
            (false, String::new(), String::new())
        };

        Some(Self {
            name,
            description,
            has_arg,
            arg_description,
            action,
        })
    }
}

// =============================================================================
// Back-compat re-exports — every Base/ fn that used to live here now
// resolves to its per-file port under `crate::fns::`. Names match the
// upstream zsh shell-function names. Pre-rename aliases
// (`completer_correct` → `_correct`, `next_label` → `_next_label`,
// `message` → `_message`, `_description` → `base_description` already
// at the lib.rs layer) are kept so `compsys/lib.rs`'s `pub use
// base::{…}` continues to compile.
// =============================================================================

pub use crate::fns::{
    _all_labels, _alternative, _approximate, _complete, _description, _dispatch, _ignored,
    _main_complete, _message, _multi_parts, _next_label, _normal, _sep_parts, _values, _wanted,
    get_ignored_patterns, is_ignored,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_manager() {
        let mut tags = TagManager::new();
        tags.init(&[
            "files".to_string(),
            "directories".to_string(),
            "commands".to_string(),
        ]);

        tags.add_try(&["files".to_string(), "directories".to_string()]);
        tags.add_try(&["commands".to_string()]);

        assert!(tags.start());
        assert!(tags.wanted("files"));
        assert!(tags.wanted("directories"));
        assert!(!tags.wanted("commands"));

        assert!(tags.next());
        assert!(!tags.wanted("files"));
        assert!(tags.wanted("commands"));

        assert!(!tags.next());
    }

    #[test]
    fn test_tag_manager_configure_from_style() {
        let mut tags = TagManager::new();
        tags.init(&[
            "files".to_string(),
            "directories".to_string(),
            "commands".to_string(),
            "options".to_string(),
        ]);

        // Configure with tag-order style values
        tags.configure_from_style(&[
            "commands options".to_string(),
            "files directories".to_string(),
        ]);

        assert!(tags.start());
        assert!(tags.wanted("commands"));
        assert!(tags.wanted("options"));
        assert!(!tags.wanted("files"));

        assert!(tags.next());
        assert!(tags.wanted("files"));
        assert!(tags.wanted("directories"));
        assert!(!tags.wanted("commands"));

        assert!(!tags.next());
    }

    #[test]
    fn test_tag_manager_configure_with_dash_stop() {
        let mut tags = TagManager::new();
        tags.init(&[
            "files".to_string(),
            "directories".to_string(),
            "commands".to_string(),
        ]);

        // "-" should stop processing remaining tag groups
        tags.configure_from_style(&[
            "files".to_string(),
            "-".to_string(),
            "commands".to_string(), // Should be ignored
        ]);

        assert!(tags.start());
        assert!(tags.wanted("files"));
        assert!(!tags.wanted("commands"));

        assert!(!tags.next()); // No more groups
    }

    #[test]
    fn test_tag_manager_requested_marks_tag() {
        let mut tags = TagManager::new();
        tags.init(&["files".to_string(), "commands".to_string()]);
        tags.add_try(&["files".to_string(), "commands".to_string()]);
        tags.start();

        // wanted() doesn't mark as requested
        assert!(tags.wanted("files"));
        assert!(!tags.requested.contains("files"));

        // requested() marks as requested
        assert!(tags.requested("files"));
        assert!(tags.requested.contains("files"));
    }

    #[test]
    fn test_alternative_parse() {
        let alt = Alternative::parse("files:file:_files").unwrap();
        assert_eq!(alt.tag, "files");
        assert_eq!(alt.description, "file");
        assert_eq!(alt.action, "_files");
    }

    #[test]
    fn test_alternative_parse_with_special_chars() {
        let alt = Alternative::parse("urls:URL:_urls -f").unwrap();
        assert_eq!(alt.tag, "urls");
        assert_eq!(alt.description, "URL");
        assert_eq!(alt.action, "_urls -f");
    }

    #[test]
    fn test_alternative_parse_empty_description() {
        let alt = Alternative::parse("files::_files").unwrap();
        assert_eq!(alt.tag, "files");
        assert_eq!(alt.description, "");
        assert_eq!(alt.action, "_files");
    }

    #[test]
    fn test_alternative_parse_invalid() {
        assert!(Alternative::parse("invalid").is_none());
        assert!(Alternative::parse("only:two").is_none());
        assert!(Alternative::parse("").is_none());
    }

    #[test]
    fn test_value_parse() {
        let val = Value::parse("debug[enable debugging]").unwrap();
        assert_eq!(val.name, "debug");
        assert_eq!(val.description, "enable debugging");
        assert!(!val.has_arg);

        let val = Value::parse("level[set level]:number:").unwrap();
        assert_eq!(val.name, "level");
        assert!(val.has_arg);
        assert_eq!(val.arg_description, "number");
    }

    #[test]
    fn test_value_parse_no_description() {
        let val = Value::parse("verbose").unwrap();
        assert_eq!(val.name, "verbose");
        assert_eq!(val.description, "");
        assert!(!val.has_arg);
    }

    #[test]
    fn test_value_parse_with_action() {
        let val = Value::parse("file[select file]:filename:_files").unwrap();
        assert_eq!(val.name, "file");
        assert_eq!(val.description, "select file");
        assert!(val.has_arg);
        assert_eq!(val.arg_description, "filename");
        assert_eq!(val.action, "_files");
    }

    #[test]
    fn test_main_complete_state() {
        let state = MainCompleteState::new("git checkout", 12);
        assert_eq!(state.comp.params.prefix, "checkout");
    }

    #[test]
    fn test_main_complete_state_empty() {
        let state = MainCompleteState::new("", 0);
        assert_eq!(state.comp.params.prefix, "");
        assert_eq!(state.comp.params.current, 1);
    }

    #[test]
    fn test_main_complete_state_mid_word() {
        let state = MainCompleteState::new("git che", 7);
        assert_eq!(state.comp.params.prefix, "che");
    }

    #[test]
    fn test_context_string() {
        let mut state = MainCompleteState::new("git checkout", 12);
        state.ctx.context = "complete".to_string();
        state.ctx.completer = "complete".to_string();
        assert_eq!(state.context_string(), ":completion:complete:complete:");
    }

    #[test]
    fn test_is_ignored() {
        let patterns = vec![
            "*.pyc".to_string(),
            "__pycache__".to_string(),
            ".git*".to_string(),
        ];

        assert!(is_ignored("file.pyc", &patterns));
        assert!(is_ignored("__pycache__", &patterns));
        assert!(is_ignored(".git", &patterns));
        assert!(is_ignored(".gitignore", &patterns));
        assert!(!is_ignored("main.py", &patterns));
        assert!(!is_ignored("git", &patterns));
    }

    #[test]
    fn test_is_ignored_empty_patterns() {
        let patterns: Vec<String> = vec![];
        assert!(!is_ignored("anything", &patterns));
    }

    #[test]
    fn test_description_basic() {
        let mut state = CompletionState::new();
        let styles = ZStyleStore::new();

        let result = _description(&mut state, &styles, ":completion:", "files", "file");
        assert_eq!(result, Some("file".to_string())); // Default format is %d
    }

    #[test]
    fn test_description_with_format() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::files",
            "format",
            vec!["-- %d --".to_string()],
            false,
        );

        let result = _description(&mut state, &styles, ":completion:", "files", "file");
        assert_eq!(result, Some("-- file --".to_string()));
    }

    #[test]
    fn test_description_with_hidden_all() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::files",
            "hidden",
            vec!["all".to_string()],
            false,
        );

        let result = _description(&mut state, &styles, ":completion:", "files", "file");
        assert_eq!(result, None);
    }

    #[test]
    fn test_description_percent_escape() {
        let mut state = CompletionState::new();
        let mut styles = ZStyleStore::new();
        styles.set(
            ":completion::files",
            "format",
            vec!["100%% %d".to_string()],
            false,
        );

        let result = _description(&mut state, &styles, ":completion:", "files", "complete");
        assert_eq!(result, Some("100% complete".to_string()));
    }

    #[test]
    fn test_completer_result_variants() {
        let matched = CompleterResult::Matched;
        let no_match = CompleterResult::NoMatch;
        let skip = CompleterResult::Skip;

        // Just verify they're distinct (for match arms)
        assert!(matches!(matched, CompleterResult::Matched));
        assert!(matches!(no_match, CompleterResult::NoMatch));
        assert!(matches!(skip, CompleterResult::Skip));
    }

    #[test]
    fn test_completion_context_default() {
        let ctx = CompletionContext::default();
        assert_eq!(ctx.context, "");
        assert_eq!(ctx.completer, "");
        assert_eq!(ctx.completer_num, 0);
        assert_eq!(ctx.matcher, "");
        assert_eq!(ctx.matcher_num, 0);
    }
}
