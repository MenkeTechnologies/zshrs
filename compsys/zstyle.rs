//! zstyle builtin implementation
//!
//! zstyle is the configuration system for zsh completion.
//! Patterns are matched against context strings like ':completion:*:*:*:*:*'

use std::collections::HashMap;

/// A single style definition
#[derive(Clone, Debug)]
pub struct ZStyle {
    /// Pattern to match context (e.g., ':completion:*:descriptions')
    pub pattern: String,
    /// Style name (e.g., 'format', 'menu', 'list-colors')
    pub name: String,
    /// Values for this style
    pub values: Vec<String>,
    /// Compiled pattern weight for matching precedence
    /// Higher weight = more specific = matched first
    pub weight: u64,
    /// Whether values should be evaluated (zstyle -e)
    pub eval: bool,
}

impl ZStyle {
    pub fn new(pattern: impl Into<String>, name: impl Into<String>, values: Vec<String>) -> Self {
        let pattern = pattern.into();
        let weight = calculate_weight(&pattern);
        Self {
            pattern,
            name: name.into(),
            values,
            weight,
            eval: false,
        }
    }

    pub fn with_eval(mut self) -> Self {
        self.eval = true;
        self
    }

    /// Get single value (first element)
    pub fn value(&self) -> Option<&str> {
        self.values.first().map(|s| s.as_str())
    }

    /// Get value as boolean
    pub fn as_bool(&self) -> Option<bool> {
        self.value()
            .map(|v| matches!(v.to_lowercase().as_str(), "yes" | "true" | "on" | "1"))
    }

    /// Get value as integer
    pub fn as_int(&self) -> Option<i64> {
        self.value().and_then(|v| v.parse().ok())
    }
}

/// Calculate pattern weight for matching precedence
/// Based on zsh's algorithm:
/// - More components = higher base weight
/// - Literal string component = 2 points
/// - Pattern component = 1 point
/// - Just '*' component = 0 points
fn calculate_weight(pattern: &str) -> u64 {
    let components: Vec<&str> = pattern.split(':').collect();
    let num_components = components.len() as u64;

    let mut specificity: u64 = 0;
    for comp in &components {
        if *comp == "*" || comp.is_empty() {
            // Just wildcard or empty = 0 points
        } else if comp.contains('*') || comp.contains('?') || comp.contains('[') {
            // Pattern = 1 point
            specificity += 1;
        } else {
            // Literal = 2 points
            specificity += 2;
        }
    }

    // Combine: high bits = num components, low bits = specificity
    (num_components << 32) | specificity
}

/// Storage for all styles
#[derive(Clone, Debug, Default)]
pub struct ZStyleStore {
    /// Styles grouped by name for faster lookup
    styles: HashMap<String, Vec<ZStyle>>,
}

impl ZStyleStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add or replace a style
    pub fn set(&mut self, pattern: &str, name: &str, values: Vec<String>, eval: bool) {
        let style = if eval {
            ZStyle::new(pattern, name, values).with_eval()
        } else {
            ZStyle::new(pattern, name, values)
        };

        let entries = self.styles.entry(name.to_string()).or_default();

        // Remove existing entry with same pattern
        entries.retain(|s| s.pattern != pattern);

        // Insert and sort by weight descending
        entries.push(style);
        entries.sort_by(|a, b| b.weight.cmp(&a.weight));
    }

    /// Delete a style
    pub fn delete(&mut self, pattern: &str, name: Option<&str>) {
        if let Some(name) = name {
            if let Some(entries) = self.styles.get_mut(name) {
                entries.retain(|s| s.pattern != pattern);
                if entries.is_empty() {
                    self.styles.remove(name);
                }
            }
        } else {
            // Delete all styles with this pattern
            for entries in self.styles.values_mut() {
                entries.retain(|s| s.pattern != pattern);
            }
            self.styles.retain(|_, v| !v.is_empty());
        }
    }

    /// Lookup a style value for a context
    pub fn lookup(&self, context: &str, name: &str) -> Option<&ZStyle> {
        self.styles.get(name).and_then(|entries| {
            entries
                .iter()
                .find(|s| pattern_matches(&s.pattern, context))
        })
    }

    /// Lookup and return values
    pub fn lookup_values(&self, context: &str, name: &str) -> Option<&[String]> {
        self.lookup(context, name).map(|s| s.values.as_slice())
    }

    /// Lookup as single string value
    pub fn lookup_str(&self, context: &str, name: &str) -> Option<&str> {
        self.lookup(context, name).and_then(|s| s.value())
    }

    /// Lookup as boolean
    pub fn lookup_bool(&self, context: &str, name: &str) -> Option<bool> {
        self.lookup(context, name).and_then(|s| s.as_bool())
    }

    /// Test if a style exists for context (zstyle -t)
    pub fn test(&self, context: &str, name: &str, patterns: &[String]) -> bool {
        if let Some(style) = self.lookup(context, name) {
            if patterns.is_empty() {
                // Just test existence
                true
            } else {
                // Test if any value matches any pattern
                for val in &style.values {
                    for pat in patterns {
                        if pattern_matches(pat, val) {
                            return true;
                        }
                    }
                }
                false
            }
        } else {
            false
        }
    }

    /// Get all style names
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.styles.keys().map(|s| s.as_str())
    }

    /// Get all patterns for a style
    pub fn patterns(&self, name: &str) -> Vec<&str> {
        self.styles
            .get(name)
            .map(|v| v.iter().map(|s| s.pattern.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get all unique patterns
    pub fn all_patterns(&self) -> Vec<&str> {
        let mut patterns: Vec<&str> = self
            .styles
            .values()
            .flat_map(|v| v.iter().map(|s| s.pattern.as_str()))
            .collect();
        patterns.sort();
        patterns.dedup();
        patterns
    }

    /// Print all styles (for zstyle with no args or -L)
    pub fn print(&self, syntax: bool) -> Vec<String> {
        let mut output = Vec::new();

        // Collect all styles sorted
        let mut all: Vec<&ZStyle> = self.styles.values().flatten().collect();
        all.sort_by(|a, b| a.pattern.cmp(&b.pattern).then_with(|| a.name.cmp(&b.name)));

        for style in all {
            if syntax {
                let vals: Vec<String> = style.values.iter().map(|v| shell_quote(v)).collect();
                let eval_flag = if style.eval { "-e " } else { "" };
                output.push(format!(
                    "zstyle {}{} {} {}",
                    eval_flag,
                    shell_quote(&style.pattern),
                    shell_quote(&style.name),
                    vals.join(" ")
                ));
            } else {
                let vals: Vec<String> = style.values.iter().map(|v| shell_quote(v)).collect();
                let eval_mark = if style.eval { "(eval) " } else { "       " };
                output.push(format!(
                    "{}{}  {}",
                    eval_mark,
                    style.pattern,
                    vals.join(" ")
                ));
            }
        }

        output
    }
}

/// Match a pattern against a context string
/// Patterns use : as separator and * as wildcard
fn pattern_matches(pattern: &str, context: &str) -> bool {
    let pat_parts: Vec<&str> = pattern.split(':').collect();
    let ctx_parts: Vec<&str> = context.split(':').collect();

    pattern_match_parts(&pat_parts, &ctx_parts)
}

fn pattern_match_parts(pattern: &[&str], context: &[&str]) -> bool {
    let mut pi = 0;
    let mut ci = 0;

    while pi < pattern.len() && ci < context.len() {
        let pat = pattern[pi];
        let ctx = context[ci];

        if pat == "*" {
            // Wildcard matches any single component
            pi += 1;
            ci += 1;
        } else if glob_match_simple(ctx, pat) {
            pi += 1;
            ci += 1;
        } else {
            return false;
        }
    }

    // All pattern parts must be consumed
    // Context can have extra parts
    pi == pattern.len()
}

/// Simple glob matching for a single component
fn glob_match_simple(text: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        return text == pattern;
    }

    let text_chars: Vec<char> = text.chars().collect();
    let pat_chars: Vec<char> = pattern.chars().collect();
    glob_match_impl(&text_chars, &pat_chars)
}

fn glob_match_impl(text: &[char], pattern: &[char]) -> bool {
    let mut ti = 0;
    let mut pi = 0;
    let mut star_pi = None;
    let mut star_ti = 0;

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            ti += 1;
            pi += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star_pi = Some(pi);
            star_ti = ti;
            pi += 1;
        } else if let Some(sp) = star_pi {
            pi = sp + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }

    pi == pattern.len()
}

/// Quote a string for shell output
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }

    // Check if quoting needed
    let needs_quote = s.chars().any(|c| {
        matches!(
            c,
            ' ' | '\t'
                | '\n'
                | '\''
                | '"'
                | '\\'
                | '$'
                | '`'
                | '!'
                | '*'
                | '?'
                | '['
                | ']'
                | '('
                | ')'
                | '{'
                | '}'
                | '<'
                | '>'
                | '|'
                | '&'
                | ';'
        )
    });

    if !needs_quote {
        return s.to_string();
    }

    // Use single quotes, escaping any single quotes
    let escaped = s.replace('\'', "'\\''");
    format!("'{}'", escaped)
}

/// Trait for easy style lookup
pub trait ZStyleLookup {
    fn lookup_style(&self, context: &str, name: &str) -> Option<&ZStyle>;
    fn lookup_style_str(&self, context: &str, name: &str) -> Option<&str>;
    fn lookup_style_bool(&self, context: &str, name: &str) -> Option<bool>;
    fn lookup_style_values(&self, context: &str, name: &str) -> Option<&[String]>;
}

impl ZStyleLookup for ZStyleStore {
    fn lookup_style(&self, context: &str, name: &str) -> Option<&ZStyle> {
        self.lookup(context, name)
    }

    fn lookup_style_str(&self, context: &str, name: &str) -> Option<&str> {
        self.lookup_str(context, name)
    }

    fn lookup_style_bool(&self, context: &str, name: &str) -> Option<bool> {
        self.lookup_bool(context, name)
    }

    fn lookup_style_values(&self, context: &str, name: &str) -> Option<&[String]> {
        self.lookup_values(context, name)
    }
}

// =============================================================================
// Standard zstyle names from zshcompsys(1)
// =============================================================================

/// All standard completion styles documented in zshcompsys(1)
/// These are the style names that can be used with zstyle ':completion:*' STYLE VALUE
pub const STANDARD_STYLES: &[(&str, &str)] = &[
    // Boolean styles
    (
        "accept-exact",
        "Accept exact match immediately without further completion",
    ),
    (
        "accept-exact-dirs",
        "Accept exact directory match without completing further",
    ),
    (
        "add-space",
        "Add space after completed word (used by _expand)",
    ),
    (
        "ambiguous",
        "Leave cursor after first ambiguous component in paths",
    ),
    (
        "call-command",
        "Whether to call external command to generate matches",
    ),
    (
        "complete",
        "Used by _expand_alias for completing vs expanding",
    ),
    ("complete-options", "Complete options for cd/pushd after -"),
    ("disabled", "Include disabled aliases in completion"),
    ("expand", "Control glob expansion in _expand completer"),
    (
        "extra-verbose",
        "Show even more verbose completion information",
    ),
    (
        "fake-always",
        "Always add fake entries even with real matches",
    ),
    (
        "gain-privileges",
        "Attempt privilege escalation for completion",
    ),
    ("glob", "Enable glob expansion in _expand"),
    ("global", "Include global aliases in _expand_alias"),
    ("hidden", "Hide matches for this context from listing"),
    (
        "ignore-line",
        "Ignore words already on line (true/current/other)",
    ),
    ("insert", "Insert all-matches string unconditionally"),
    ("insert-tab", "Insert tab when nothing to complete"),
    ("insert-unambiguous", "Only menu if no unambiguous prefix"),
    (
        "last-prompt",
        "Return cursor to prompt after completion list",
    ),
    ("list", "Control listing behavior"),
    ("list-dirs-first", "List directories before files"),
    ("list-grouped", "Group matches in listing"),
    ("list-packed", "Pack completion list tightly"),
    ("list-rows-first", "List completions by rows first"),
    ("list-suffixes", "Show suffixes in completion list"),
    ("match-original", "Match original string before corrections"),
    ("old-list", "Reuse previous completion list"),
    ("old-matches", "Reuse previous matches"),
    ("old-menu", "Reuse previous menu state"),
    ("original", "Include original string in corrections"),
    (
        "path-completion",
        "Enable path completion (set false to disable)",
    ),
    ("prefix-hidden", "Hide common prefix in listing"),
    ("prefix-needed", "Require prefix before completing"),
    ("preserve-prefix", "Keep prefix when expanding"),
    ("recursive-files", "Complete files recursively"),
    ("regular", "Include regular aliases (not global/suffix)"),
    ("remove-all-dups", "Remove all duplicate matches"),
    ("separate-sections", "Separate man pages by section"),
    ("show-ambiguity", "Highlight ambiguous part of completion"),
    ("show-completer", "Display which completer is being tried"),
    ("single-ignored", "Show single ignored match"),
    ("special-dirs", "Complete . and .. directories"),
    ("squeeze-slashes", "Remove duplicate slashes in paths"),
    ("strip-comments", "Strip comments from completion"),
    (
        "subst-globs-only",
        "Only substitute globs, not other expansions",
    ),
    ("substitute", "Enable substitution in _expand"),
    ("use-cache", "Enable completion caching"),
    ("use-compctl", "Use old compctl system as fallback"),
    ("verbose", "Show verbose completion information"),
    // String styles
    (
        "auto-description",
        "Format for auto-generated descriptions (%d = desc)",
    ),
    ("cache-path", "Directory for completion cache files"),
    ("cache-policy", "Function to check cache validity"),
    ("command", "Override command for generating matches"),
    ("command-path", "Directories to search for commands"),
    ("condition", "Condition for including matches"),
    ("format", "Format string for completion headers"),
    ("group-name", "Name for grouping matches (empty = by tag)"),
    ("list-prompt", "Prompt shown when paging through list"),
    (
        "list-separator",
        "Separator between completion and description",
    ),
    ("local", "Local part of URL for completion"),
    ("mail-directory", "Directory containing mail folders"),
    ("max-matches-width", "Maximum width for matches in listing"),
    ("menu", "Menu selection (yes/no/select/interactive/search)"),
    ("pine-directory", "Pine mail directory"),
    ("select-prompt", "Prompt shown during menu selection"),
    ("select-scroll", "Scroll behavior in menu selection"),
    // Integer styles
    (
        "file-sort",
        "Sort order for files (name/size/time/links/access/inode/modification)",
    ),
    ("force-list", "Force listing when >= N matches"),
    ("insert-ids", "Insert process IDs (menu/single)"),
    ("insert-sections", "Insert man page section numbers"),
    ("max-errors", "Maximum errors for approximate matching"),
    // Array styles
    (
        "assign-list",
        "Patterns for colon-separated assignment values",
    ),
    ("avoid-completer", "Completers to skip for all-matches"),
    ("commands", "Default subcommands for init scripts"),
    ("completer", "List of completer functions to use"),
    ("delimiters", "Word delimiters for completion"),
    ("domains", "Network domains for completion"),
    ("environ", "Environment variables for external commands"),
    ("fake", "Fake entries to add to completion"),
    ("fake-files", "Fake files to add in directory completion"),
    ("fake-parameters", "Fake parameters to add"),
    ("file-patterns", "Patterns for file completion grouping"),
    (
        "file-split-chars",
        "Characters that split filename completion",
    ),
    ("filter", "Filter for LDAP completion"),
    ("group-order", "Order of completion groups"),
    ("groups", "UNIX groups for completion"),
    ("hosts", "Hostnames for completion"),
    ("hosts-ports", "host:port pairs for completion"),
    ("ignored-patterns", "Patterns to ignore in completion"),
    ("known-hosts-files", "SSH known_hosts files to read"),
    (
        "list-colors",
        "Colors for completion listing (ZLS_COLORS format)",
    ),
    ("matcher", "Matcher specification for current context"),
    ("matcher-list", "List of matcher specs to try in order"),
    ("packageset", "Package sets for completion"),
    (
        "remote-access",
        "Whether to access remote systems for completion",
    ),
    ("tag-order", "Order in which tags are tried"),
    ("urls", "URLs for completion"),
    ("users", "Usernames for completion"),
    ("users-hosts", "user@host pairs for completion"),
    ("users-hosts-ports", "user@host:port triples for completion"),
];

/// Standard completion tags documented in zshcompsys(1)
pub const STANDARD_TAGS: &[(&str, &str)] = &[
    ("accounts", "User accounts (for users-hosts style)"),
    (
        "all-expansions",
        "All expansions from _expand as single string",
    ),
    ("all-files", "All files (vs specific subset)"),
    ("arguments", "Command arguments"),
    ("arrays", "Array parameter names"),
    ("association-keys", "Keys of associative arrays"),
    ("bookmarks", "Bookmarks (URLs, zftp)"),
    ("builtins", "Builtin command names"),
    ("characters", "Single characters"),
    ("colormapids", "X colormap IDs"),
    ("colors", "Color names"),
    ("commands", "External command names"),
    ("contexts", "zstyle contexts"),
    ("corrections", "Spelling corrections"),
    ("cursors", "X cursor names"),
    ("default", "Default fallback tag"),
    ("descriptions", "For format style lookups"),
    ("devices", "Device special files"),
    ("directories", "Directory names"),
    ("directory-stack", "Directory stack entries"),
    ("displays", "X display names"),
    ("domains", "Network domains"),
    ("email-plugin", "Email addresses"),
    ("expansions", "Individual expansions from _expand"),
    ("extensions", "X server extensions"),
    ("file-descriptors", "Open file descriptors"),
    ("files", "Generic filenames"),
    ("fonts", "X font names"),
    ("fstypes", "Filesystem types"),
    ("functions", "Function names"),
    ("globbed-files", "Files matching glob pattern"),
    ("groups", "User group names"),
    ("history-words", "Words from history"),
    ("hosts", "Hostnames"),
    ("indexes", "Array indexes"),
    ("interfaces", "Network interfaces"),
    ("jobs", "Job identifiers"),
    ("keymaps", "Zsh keymap names"),
    ("keysyms", "X keysym names"),
    ("libraries", "Library names"),
    ("local-directories", "Directories relative to cdpath"),
    ("mailboxstrstrstrstrstrstrfolders", "Mail folders"),
    ("manuals", "Manual pages"),
    ("maps", "NIS maps"),
    ("messages", "For format style lookups"),
    ("modifiers", "History modifiers"),
    ("modules", "Zsh module names"),
    ("my-accounts", "User's own accounts"),
    ("named-directories", "Named directories (~name)"),
    ("names", "Generic names"),
    ("nicknames", "NIS nicknames"),
    ("options", "Command options"),
    ("original", "Original (uncorrected) string"),
    ("other-accounts", "Other user accounts"),
    ("packages", "Package names"),
    ("parameters", "Parameter names"),
    ("paths", "Path components"),
    ("pods", "Perl POD files"),
    ("ports", "Network ports"),
    ("prefixes", "Completion prefixes"),
    ("printers", "Printer names"),
    ("processes", "Process IDs"),
    ("processes-names", "Process names"),
    ("sequences", "Sequence numbers"),
    ("sessions", "Terminal sessions"),
    ("signals", "Signal names"),
    ("strings", "Generic strings"),
    ("styles", "zstyle style names"),
    ("suffixes", "Filename suffixes"),
    ("tags", "Completion tags"),
    ("targets", "Make/build targets"),
    ("time-zones", "Time zone names"),
    ("types", "Type names"),
    ("urls", "URLs"),
    ("users", "Usernames"),
    ("values", "Generic values"),
    ("variants", "Command variants"),
    ("visuals", "X visual types"),
    ("warnings", "For format style lookups"),
    ("widgets", "Zsh widget names"),
    ("windows", "X window IDs"),
    ("zsh-options", "Zsh option names"),
];

/// Standard completers that can appear in the 'completer' style
pub const STANDARD_COMPLETERS: &[(&str, &str)] = &[
    ("_complete", "Standard completion"),
    ("_approximate", "Approximate/fuzzy matching"),
    ("_correct", "Spelling correction"),
    ("_expand", "Expand globs/variables/history"),
    ("_expand_alias", "Expand aliases"),
    ("_extensions", "Complete by file extension"),
    ("_external_pwds", "Complete external working directories"),
    ("_history", "Complete from command history"),
    ("_ignored", "Restore previously ignored matches"),
    ("_list", "Control listing behavior"),
    ("_match", "Pattern matching completion"),
    ("_menu", "Menu completion control"),
    ("_oldlist", "Reuse previous completion list"),
    ("_prefix", "Complete prefix before cursor"),
    ("_user_expand", "User-defined expansions"),
    ("_all_matches", "Add string with all matches"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weight_calculation() {
        // More specific patterns should have higher weight
        let w1 = calculate_weight(":completion:*");
        let w2 = calculate_weight(":completion:*:descriptions");
        let w3 = calculate_weight(":completion:*:*:*:*:descriptions");

        assert!(w2 > w1, "more components = higher weight");
        assert!(w3 > w2, "even more components = even higher weight");

        let wa = calculate_weight(":completion:*:default");
        let wb = calculate_weight(":completion:*:*");
        assert!(wa > wb, "literal > wildcard");
    }

    #[test]
    fn test_pattern_matching() {
        assert!(pattern_matches(":completion:*", ":completion:foo"));
        assert!(pattern_matches(":completion:*:*", ":completion:foo:bar"));
        assert!(pattern_matches(
            ":completion:*:descriptions",
            ":completion:foo:descriptions"
        ));
        assert!(!pattern_matches(
            ":completion:*:descriptions",
            ":completion:foo:messages"
        ));
    }

    #[test]
    fn test_style_store() {
        let mut store = ZStyleStore::new();

        store.set(":completion:*", "menu", vec!["select".to_string()], false);
        store.set(
            ":completion:*:descriptions",
            "format",
            vec!["%d".to_string()],
            false,
        );

        assert_eq!(
            store.lookup_str(":completion:anything", "menu"),
            Some("select")
        );
        assert_eq!(
            store.lookup_str(":completion:foo:descriptions", "format"),
            Some("%d")
        );
        assert_eq!(store.lookup_str(":completion:foo:messages", "format"), None);
    }

    #[test]
    fn test_specificity() {
        let mut store = ZStyleStore::new();

        // Less specific
        store.set(":completion:*", "menu", vec!["no".to_string()], false);
        // More specific
        store.set(
            ":completion:*:*:*:default",
            "menu",
            vec!["yes".to_string()],
            false,
        );

        // More specific should win
        assert_eq!(
            store.lookup_str(":completion:foo:bar:baz:default", "menu"),
            Some("yes")
        );
        // Falls back to less specific
        assert_eq!(store.lookup_str(":completion:foo", "menu"), Some("no"));
    }

    #[test]
    fn test_shell_quote() {
        assert_eq!(shell_quote("simple"), "simple");
        assert_eq!(shell_quote("with space"), "'with space'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    #[test]
    fn test_zstyle_new() {
        let style = ZStyle::new(":completion:*", "menu", vec!["select".to_string()]);
        assert_eq!(style.pattern, ":completion:*");
        assert_eq!(style.name, "menu");
        assert_eq!(style.values, vec!["select"]);
        assert!(!style.eval);
        assert!(style.weight > 0);
    }

    #[test]
    fn test_zstyle_with_eval() {
        let style = ZStyle::new(":completion:*", "hosts", vec!["$myhosts".to_string()]).with_eval();
        assert!(style.eval);
    }

    #[test]
    fn test_zstyle_value() {
        let style = ZStyle::new(
            ":completion:*",
            "menu",
            vec!["select".to_string(), "interactive".to_string()],
        );
        assert_eq!(style.value(), Some("select"));
    }

    #[test]
    fn test_zstyle_value_empty() {
        let style = ZStyle::new(":completion:*", "menu", vec![]);
        assert_eq!(style.value(), None);
    }

    #[test]
    fn test_zstyle_as_bool_true() {
        for val in &["true", "yes", "on", "1"] {
            let style = ZStyle::new(":completion:*", "verbose", vec![val.to_string()]);
            assert_eq!(style.as_bool(), Some(true), "Failed for {}", val);
        }
    }

    #[test]
    fn test_zstyle_as_bool_false() {
        for val in &["false", "no", "off", "0"] {
            let style = ZStyle::new(":completion:*", "verbose", vec![val.to_string()]);
            assert_eq!(style.as_bool(), Some(false), "Failed for {}", val);
        }
    }

    #[test]
    fn test_zstyle_as_int() {
        let style = ZStyle::new(":completion:*", "max-errors", vec!["3".to_string()]);
        assert_eq!(style.as_int(), Some(3));

        let style2 = ZStyle::new(
            ":completion:*",
            "max-errors",
            vec!["not-a-number".to_string()],
        );
        assert_eq!(style2.as_int(), None);
    }

    #[test]
    fn test_lookup_values() {
        let mut store = ZStyleStore::new();
        store.set(
            ":completion:*",
            "completer",
            vec!["_complete".to_string(), "_approximate".to_string()],
            false,
        );

        let values = store.lookup_values(":completion:foo", "completer");
        assert!(values.is_some());
        assert_eq!(values.unwrap().len(), 2);
    }

    #[test]
    fn test_lookup_bool() {
        let mut store = ZStyleStore::new();
        store.set(":completion:*", "verbose", vec!["yes".to_string()], false);

        assert_eq!(store.lookup_bool(":completion:foo", "verbose"), Some(true));
        assert_eq!(store.lookup_bool(":completion:foo", "nonexistent"), None);
    }

    #[test]
    fn test_lookup_as_int() {
        let mut store = ZStyleStore::new();
        store.set(":completion:*", "max-errors", vec!["5".to_string()], false);

        let style = store.lookup(":completion:foo", "max-errors");
        assert!(style.is_some());
        assert_eq!(style.unwrap().as_int(), Some(5));
    }

    #[test]
    fn test_pattern_matches_wildcard_middle() {
        assert!(pattern_matches(
            ":completion:*:default",
            ":completion:complete:default"
        ));
        assert!(pattern_matches(
            ":completion:*:*:default",
            ":completion:complete:git:default"
        ));
    }

    #[test]
    fn test_pattern_matches_exact() {
        assert!(pattern_matches(
            ":completion:complete:git",
            ":completion:complete:git"
        ));
        assert!(!pattern_matches(
            ":completion:complete:git",
            ":completion:complete:docker"
        ));
    }

    #[test]
    fn test_pattern_matches_empty() {
        assert!(pattern_matches("", ""));
        assert!(!pattern_matches("", "foo"));
    }

    #[test]
    fn test_calculate_weight_all_wildcards() {
        let w = calculate_weight(":*:*:*");
        assert!(w > 0); // Has components but low specificity
    }

    #[test]
    fn test_calculate_weight_all_literals() {
        let w1 = calculate_weight(":completion:complete:git:argument:files");
        let w2 = calculate_weight(":*:*:*:*:*");
        assert!(w1 > w2); // Literals have higher specificity
    }

    #[test]
    fn test_standard_styles_not_empty() {
        assert!(!STANDARD_STYLES.is_empty());
        assert!(STANDARD_STYLES.len() > 50);

        // Check a few known styles exist
        assert!(STANDARD_STYLES.iter().any(|(name, _)| *name == "menu"));
        assert!(STANDARD_STYLES.iter().any(|(name, _)| *name == "completer"));
        assert!(STANDARD_STYLES.iter().any(|(name, _)| *name == "format"));
    }

    #[test]
    fn test_standard_tags_not_empty() {
        assert!(!STANDARD_TAGS.is_empty());
        assert!(STANDARD_TAGS.len() > 50);

        // Check a few known tags exist
        assert!(STANDARD_TAGS.iter().any(|(name, _)| *name == "files"));
        assert!(STANDARD_TAGS.iter().any(|(name, _)| *name == "commands"));
        assert!(STANDARD_TAGS.iter().any(|(name, _)| *name == "options"));
    }

    #[test]
    fn test_standard_completers_not_empty() {
        assert!(!STANDARD_COMPLETERS.is_empty());

        // Check known completers exist
        assert!(STANDARD_COMPLETERS
            .iter()
            .any(|(name, _)| *name == "_complete"));
        assert!(STANDARD_COMPLETERS
            .iter()
            .any(|(name, _)| *name == "_approximate"));
        assert!(STANDARD_COMPLETERS
            .iter()
            .any(|(name, _)| *name == "_expand"));
    }

    #[test]
    fn test_style_override() {
        let mut store = ZStyleStore::new();

        store.set(":completion:*", "menu", vec!["no".to_string()], false);
        assert_eq!(store.lookup_str(":completion:foo", "menu"), Some("no"));

        // Override with same pattern
        store.set(":completion:*", "menu", vec!["yes".to_string()], false);
        assert_eq!(store.lookup_str(":completion:foo", "menu"), Some("yes"));
    }

    #[test]
    fn test_multiple_styles_same_pattern() {
        let mut store = ZStyleStore::new();

        store.set(":completion:*", "menu", vec!["select".to_string()], false);
        store.set(":completion:*", "verbose", vec!["yes".to_string()], false);
        store.set(":completion:*", "format", vec!["%d".to_string()], false);

        assert_eq!(store.lookup_str(":completion:foo", "menu"), Some("select"));
        assert_eq!(store.lookup_str(":completion:foo", "verbose"), Some("yes"));
        assert_eq!(store.lookup_str(":completion:foo", "format"), Some("%d"));
    }

    #[test]
    fn test_shell_quote_special_chars() {
        assert_eq!(shell_quote("$var"), "'$var'");
        assert_eq!(shell_quote("a;b"), "'a;b'");
        assert_eq!(shell_quote("a|b"), "'a|b'");
        assert_eq!(shell_quote("a&b"), "'a&b'");
    }

    #[test]
    fn test_shell_quote_newline() {
        assert_eq!(shell_quote("a\nb"), "'a\nb'");
    }
}
