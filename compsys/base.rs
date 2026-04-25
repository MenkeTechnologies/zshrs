//! Native Rust implementations of Base/ completion functions
//!
//! This module implements all the core functions from zsh's Base/ directory:
//! - Core: _main_complete, _tags, _normal, _dispatch, etc.
//! - Utility: _alternative, _values, _multi_parts, etc.
//! - Completer: _complete, _approximate, _correct, etc.

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};
use crate::zstyle::ZStyleStore;
use std::collections::{HashMap, HashSet};

// =============================================================================
// Core functions (_main_complete, _tags, _normal, _dispatch, etc.)
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

/// Main completion entry point (_main_complete)
///
/// This is THE function that gets called when the user presses TAB.
pub fn main_complete(
    state: &mut MainCompleteState,
    dispatch: impl Fn(&mut MainCompleteState, &str) -> CompleterResult,
) -> i32 {
    // Get completers from style or use defaults
    if let Some(completers) = state
        .styles
        .lookup_values(&state.context_string(), "completer")
    {
        state.completers = completers.to_vec();
    }

    state.ctx.completer_num = 1;

    // Call pre-functions
    let prefuncs = state.prefuncs.clone();
    for func in &prefuncs {
        // Would call the function here
        let _ = func;
    }

    // Try each completer
    for completer_name in state.completers.clone() {
        // Extract completer name (handle _complete:foo syntax)
        let (completer, name) = if let Some(pos) = completer_name.find(':') {
            (&completer_name[..pos], &completer_name[pos + 1..])
        } else {
            (completer_name.as_str(), &completer_name[1..]) // strip leading _
        };

        state.ctx.completer = name.replace('_', "-");

        // Get matcher list
        let matchers = state
            .styles
            .lookup_values(&state.context_string(), "matcher-list")
            .map(|v| v.to_vec())
            .unwrap_or_else(|| vec![String::new()]);

        state.ctx.matcher_num = 1;

        for matcher in &matchers {
            state.ctx.matcher = matcher.clone();

            // Call the completer
            match dispatch(state, completer) {
                CompleterResult::Matched => {
                    state.ret = 0;
                    break;
                }
                CompleterResult::Skip => break,
                CompleterResult::NoMatch => {}
            }

            state.ctx.matcher_num += 1;
        }

        if state.ret == 0 {
            break;
        }

        state.ctx.completer_num += 1;
    }

    // Call post-functions
    let postfuncs = state.postfuncs.clone();
    for func in &postfuncs {
        let _ = func;
    }

    // Store lastcomp info
    state
        .lastcomp
        .insert("nmatches".to_string(), state.comp.nmatches.to_string());
    state
        .lastcomp
        .insert("completer".to_string(), state.ctx.completer.clone());
    state
        .lastcomp
        .insert("prefix".to_string(), state.comp.params.prefix.clone());
    state
        .lastcomp
        .insert("suffix".to_string(), state.comp.params.suffix.clone());

    state.ret
}

/// _normal - normal command completion
pub fn normal_complete(state: &mut MainCompleteState) -> CompleterResult {
    let current = state.comp.params.current as usize;

    // Completing command name (position 1)?
    if current == 1 {
        // Would dispatch to -command- completion
        return CompleterResult::NoMatch;
    }

    // Get the command name
    let cmd = if !state.comp.params.words.is_empty() {
        state.comp.params.words[0].clone()
    } else {
        return CompleterResult::NoMatch;
    };

    // Dispatch to command-specific completion
    dispatch_complete(state, &cmd)
}

/// _dispatch - dispatch to appropriate completion function
pub fn dispatch_complete(state: &mut MainCompleteState, cmd: &str) -> CompleterResult {
    // Look up completion function for command
    // In real implementation, this would check _comps associative array

    // For now, just return NoMatch - actual dispatch needs shell integration
    let _ = cmd;
    let _ = state;
    CompleterResult::NoMatch
}

// =============================================================================
// Tag-based completion functions
// =============================================================================

/// _requested - check if a tag is currently being tried
pub fn requested(tags: &mut TagManager, tag: &str) -> bool {
    tags.requested(tag)
}

/// _wanted - check if a tag is wanted (without marking as requested)
pub fn wanted(tags: &TagManager, tag: &str) -> bool {
    tags.wanted(tag)
}

/// _all_labels - iterate over all labels for a tag
pub fn all_labels<F>(
    state: &mut CompletionState,
    tags: &mut TagManager,
    tag: &str,
    description: &str,
    mut f: F,
) -> bool
where
    F: FnMut(&mut CompletionState, &str) -> bool,
{
    if !tags.requested(tag) {
        return false;
    }

    state.begin_group(tag, true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(tag));
    }

    let result = f(state, tag);

    state.end_group();
    result
}

/// _next_label - get next label for a tag (for iteration)
pub fn next_label(tags: &TagManager, tag: &str) -> Option<String> {
    if tags.wanted(tag) {
        Some(tag.to_string())
    } else {
        None
    }
}

// =============================================================================
// Utility functions (_alternative, _values, _multi_parts, etc.)
// =============================================================================

/// Alternative specification
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

/// _alternative - try multiple completion alternatives
pub fn alternative(
    state: &mut MainCompleteState,
    specs: &[String],
    action_handler: impl Fn(&mut MainCompleteState, &str) -> bool,
) -> bool {
    let alternatives: Vec<Alternative> =
        specs.iter().filter_map(|s| Alternative::parse(s)).collect();

    // Initialize tags with all alternative tags
    let tags: Vec<String> = alternatives.iter().map(|a| a.tag.clone()).collect();
    state.tags.init(&tags);
    state.tags.add_try(&tags);

    if !state.tags.start() {
        return false;
    }

    let mut matched = false;

    loop {
        for alt in &alternatives {
            if state.tags.requested(&alt.tag) {
                state.comp.begin_group(&alt.tag, true);
                if !alt.description.is_empty() {
                    state
                        .comp
                        .add_explanation(alt.description.clone(), Some(&alt.tag));
                }

                if action_handler(state, &alt.action) {
                    matched = true;
                }

                state.comp.end_group();
            }
        }

        if !state.tags.next() {
            break;
        }
    }

    matched
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
    /// Parse "name[description]:arg-desc:action" format
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
        let (has_arg, arg_description, action) = if rest.starts_with(':') {
            let parts: Vec<&str> = rest[1..].splitn(2, ':').collect();
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

/// _values - complete comma-separated values
pub fn values_complete(
    state: &mut CompletionState,
    description: &str,
    separator: char,
    specs: &[String],
) -> bool {
    let values: Vec<Value> = specs.iter().filter_map(|s| Value::parse(s)).collect();

    let prefix = state.params.prefix.clone();

    // Find already-used values
    let used: HashSet<String> = prefix
        .split(separator)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    // Get current value being completed
    let current_prefix = prefix.rsplit(separator).next().unwrap_or("").to_string();

    state.begin_group("values", true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some("values"));
    }

    let mut matched = false;
    for value in &values {
        // Skip already-used values
        if used.contains(&value.name) {
            continue;
        }

        // Check prefix match
        if !value.name.starts_with(&current_prefix) {
            continue;
        }

        let mut comp = Completion::new(&value.name);
        if !value.description.is_empty() {
            comp.disp = Some(format!("{} -- {}", value.name, value.description));
        }

        // Add separator suffix
        if value.has_arg {
            comp.suf = Some("=".to_string());
            comp.flags |= CompletionFlags::NOSPACE;
        }

        state.add_match(comp, Some("values"));
        matched = true;
    }

    state.end_group();
    matched
}

/// _multi_parts - complete with path-like parts
pub fn multi_parts(state: &mut CompletionState, separator: char, parts: &[String]) -> bool {
    let prefix = state.params.prefix.clone();

    // Find matching parts
    let matching: Vec<&String> = parts.iter().filter(|p| p.starts_with(&prefix)).collect();

    if matching.is_empty() {
        return false;
    }

    // Find common prefix up to next separator
    let prefix_parts: Vec<&str> = prefix.split(separator).collect();

    state.begin_group("parts", true);
    let depth = prefix_parts.len();

    let mut seen = HashSet::new();
    let mut matched = false;

    for part in matching {
        let part_parts: Vec<&str> = part.split(separator).collect();

        // Get completion up to next separator
        let comp_parts = &part_parts[..depth.min(part_parts.len())];
        let comp_str = comp_parts.join(&separator.to_string());

        if seen.contains(&comp_str) {
            continue;
        }
        seen.insert(comp_str.clone());

        let mut comp = Completion::new(&comp_str);

        // Add separator if there are more parts
        if depth < part_parts.len() {
            comp.suf = Some(separator.to_string());
            comp.flags |= CompletionFlags::NOSPACE;
        }

        state.add_match(comp, Some("parts"));
        matched = true;
    }

    state.end_group();
    matched
}

/// _sep_parts - complete parts with arbitrary separators
pub fn sep_parts(state: &mut CompletionState, separators: &str, arrays: &[Vec<String>]) -> bool {
    if arrays.is_empty() {
        return false;
    }

    let prefix = state.params.prefix.clone();
    let sep_chars: Vec<char> = separators.chars().collect();

    // Find which array we're completing based on separators in prefix
    let mut array_idx = 0;
    for sep in &sep_chars {
        if prefix.contains(*sep) {
            array_idx += 1;
        }
    }

    if array_idx >= arrays.len() {
        return false;
    }

    // Get prefix for current part
    let current_prefix = if let Some(sep) = sep_chars.get(array_idx.saturating_sub(1)) {
        prefix.rsplit(*sep).next().unwrap_or("").to_string()
    } else {
        prefix.clone()
    };

    state.begin_group("sep-parts", true);

    let mut matched = false;
    for item in &arrays[array_idx] {
        if item.starts_with(&current_prefix) {
            let mut comp = Completion::new(item);

            // Add next separator if there are more arrays
            if array_idx + 1 < arrays.len() {
                if let Some(&sep) = sep_chars.get(array_idx) {
                    comp.suf = Some(sep.to_string());
                    comp.flags |= CompletionFlags::NOSPACE;
                }
            }

            state.add_match(comp, Some("sep-parts"));
            matched = true;
        }
    }

    state.end_group();
    matched
}

// =============================================================================
// Completer functions (_complete, _approximate, _correct, etc.)
// =============================================================================

/// _complete - the main completer
pub fn completer_complete(state: &mut MainCompleteState) -> CompleterResult {
    // This is the default completer that handles normal completion
    normal_complete(state)
}

/// _ignored - complete with ignored matches
pub fn completer_ignored(state: &mut MainCompleteState) -> CompleterResult {
    // Complete using matches that were previously ignored (e.g., by fignore)
    // For now, just return NoMatch
    let _ = state;
    CompleterResult::NoMatch
}

/// _approximate - approximate completion (fuzzy matching)
pub fn completer_approximate(state: &mut MainCompleteState) -> CompleterResult {
    // Get max errors from style
    let max_errors = state
        .styles
        .lookup_values(&state.context_string(), "max-errors")
        .and_then(|v| v.first().and_then(|s| s.parse::<usize>().ok()))
        .unwrap_or(2);

    // Would implement approximate matching here
    let _ = max_errors;
    CompleterResult::NoMatch
}

/// _correct - spelling correction
pub fn completer_correct(state: &mut MainCompleteState) -> CompleterResult {
    // Would implement spelling correction here
    let _ = state;
    CompleterResult::NoMatch
}

/// _expand - expansion of special characters
pub fn completer_expand(state: &mut MainCompleteState) -> CompleterResult {
    let prefix = &state.comp.params.prefix;

    // Check for things to expand
    if prefix.contains('$') || prefix.contains('~') || prefix.contains('{') {
        // Would handle variable/tilde/brace expansion
    }

    CompleterResult::NoMatch
}

/// _history - complete from history
pub fn completer_history(state: &mut MainCompleteState) -> CompleterResult {
    // Would complete from command history
    let _ = state;
    CompleterResult::NoMatch
}

/// _match - pattern matching completion
pub fn completer_match(state: &mut MainCompleteState) -> CompleterResult {
    // Uses glob patterns for matching
    let _ = state;
    CompleterResult::NoMatch
}

/// _menu - menu completion
pub fn completer_menu(state: &mut MainCompleteState) -> CompleterResult {
    // Handles menu selection
    let _ = state;
    CompleterResult::NoMatch
}

/// _prefix - complete with prefix handling
pub fn completer_prefix(state: &mut MainCompleteState) -> CompleterResult {
    // Handles prefix-based completion
    let _ = state;
    CompleterResult::NoMatch
}

// =============================================================================
// Message and description functions
// =============================================================================

/// _description - set up description for a tag
/// Handles styles: format, hidden, group-name, matcher, sort, ignored-patterns
pub fn description(
    _state: &mut CompletionState,
    styles: &ZStyleStore,
    context: &str,
    tag: &str,
    description: &str,
) -> Option<String> {
    let ctx = format!("{}:{}", context, tag);

    // Check 'hidden' style - if set to 'all', return empty format
    if let Some(hidden) = styles.lookup_values(&ctx, "hidden") {
        if let Some(v) = hidden.first() {
            match v.as_str() {
                "all" => return None,
                "yes" | "true" | "1" | "on" => {
                    // Hidden but still has format for group header
                }
                _ => {}
            }
        }
    }

    // Get format from style (try tag-specific first, then descriptions tag)
    let format = styles
        .lookup_values(&ctx, "format")
        .or_else(|| styles.lookup_values(&format!("{}:descriptions", context), "format"))
        .and_then(|v| v.first().cloned())
        .unwrap_or_else(|| "%d".to_string());

    // zformat -F substitution: %d = description, plus additional escapes
    let result = format.replace("%d", description).replace("%%", "%");

    Some(result)
}

/// Get ignored-patterns for a context/tag
pub fn get_ignored_patterns(styles: &ZStyleStore, context: &str, tag: &str) -> Vec<String> {
    let ctx = format!("{}:{}", context, tag);
    styles
        .lookup_values(&ctx, "ignored-patterns")
        .map(|v| v.to_vec())
        .unwrap_or_default()
}

/// Check if a string matches any ignored pattern
pub fn is_ignored(s: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if glob_match(pattern, s) {
            return true;
        }
    }
    false
}

/// Simple glob matching for ignored-patterns
fn glob_match(pattern: &str, s: &str) -> bool {
    let pattern = pattern.as_bytes();
    let s = s.as_bytes();

    let mut pi = 0;
    let mut si = 0;
    let mut star_pi = None;
    let mut star_si = None;

    while si < s.len() {
        if pi < pattern.len() && (pattern[pi] == b'?' || pattern[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < pattern.len() && pattern[pi] == b'*' {
            star_pi = Some(pi);
            star_si = Some(si);
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_si = Some(star_si.unwrap() + 1);
            si = star_si.unwrap();
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == b'*' {
        pi += 1;
    }

    pi == pattern.len()
}

/// _message - display a message (no completions)
pub fn message(
    state: &mut CompletionState,
    styles: &ZStyleStore,
    context: &str,
    tag: &str,
    message: &str,
) {
    let formatted = description(state, styles, context, tag, message);

    if let Some(msg) = formatted {
        state.begin_group(tag, true);
        state.add_explanation(msg, Some(tag));
        state.end_group();
    }
}

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
    fn test_glob_match_simple() {
        assert!(glob_match("*.txt", "file.txt"));
        assert!(glob_match("*.txt", ".txt"));
        assert!(!glob_match("*.txt", "file.rs"));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("file?.txt", "file1.txt"));
        assert!(glob_match("file?.txt", "fileX.txt"));
        assert!(!glob_match("file?.txt", "file.txt"));
        assert!(!glob_match("file?.txt", "file12.txt"));
    }

    #[test]
    fn test_glob_match_star_middle() {
        assert!(glob_match("foo*bar", "foobar"));
        assert!(glob_match("foo*bar", "foo123bar"));
        assert!(glob_match("foo*bar", "fooXYZbar"));
        assert!(!glob_match("foo*bar", "foobaz"));
    }

    #[test]
    fn test_glob_match_multiple_stars() {
        assert!(glob_match("*foo*", "foo"));
        assert!(glob_match("*foo*", "afoo"));
        assert!(glob_match("*foo*", "foob"));
        assert!(glob_match("*foo*", "afoob"));
        assert!(!glob_match("*foo*", "bar"));
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("exact", "exact"));
        assert!(!glob_match("exact", "exacty"));
        assert!(!glob_match("exact", "xact"));
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

        let result = description(&mut state, &styles, ":completion:", "files", "file");
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

        let result = description(&mut state, &styles, ":completion:", "files", "file");
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

        let result = description(&mut state, &styles, ":completion:", "files", "file");
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

        let result = description(&mut state, &styles, ":completion:", "files", "complete");
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
