//! Completion utilities - compdescribe, comparguments, compvalues, comptags
//!
//! Ported from zsh Src/Zle/computil.c
//! These are the C builtins that support shell functions like _describe, _arguments, _values

use crate::compcore::CompletionState;
use crate::completion::Completion;
use crate::state::CompParams;
use std::collections::HashSet;

/// Tag management for completion
///
/// comptags manages the "tag" system - tags are categories of completions
/// like "files", "directories", "commands", etc.
#[derive(Clone, Debug, Default)]
pub struct CompTags {
    /// All available tags for this completion
    offered: Vec<String>,
    /// Tags that have been tried in order
    tried: Vec<Vec<String>>,
    /// Current try index
    current_try: usize,
    /// Tags for current try
    current_tags: HashSet<String>,
}

impl CompTags {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize tags (comptags -i)
    pub fn init(&mut self, _context: &str, tags: &[String]) {
        self.offered = tags.to_vec();
        self.tried.clear();
        self.current_try = 0;
        self.current_tags.clear();
    }

    /// Try a set of tags (comptry)
    /// Returns true if any of the tags are available
    pub fn try_tags(&mut self, tags: &[String]) -> bool {
        let available: Vec<String> = tags
            .iter()
            .filter(|t| self.offered.contains(t))
            .cloned()
            .collect();

        if available.is_empty() {
            return false;
        }

        self.tried.push(available.clone());
        for tag in available {
            self.current_tags.insert(tag);
        }
        true
    }

    /// Move to next set of tags (comptags -N)
    /// Returns true if there are more tags to try
    pub fn next(&mut self) -> bool {
        self.current_try += 1;
        self.current_tags.clear();

        if self.current_try < self.tried.len() {
            for tag in &self.tried[self.current_try] {
                self.current_tags.insert(tag.clone());
            }
            true
        } else {
            false
        }
    }

    /// Check if a tag is currently active (comptags -T)
    pub fn is_set(&self, tag: &str) -> bool {
        self.current_tags.contains(tag)
    }

    /// Get current active tags
    pub fn current(&self) -> &HashSet<String> {
        &self.current_tags
    }
}

/// Argument specification for _arguments
#[derive(Clone, Debug)]
pub struct ArgSpec {
    /// Option string (e.g., "-v", "--verbose")
    pub option: String,
    /// Description
    pub description: String,
    /// Action to perform (completion spec)
    pub action: String,
    /// Whether option is exclusive with others
    pub exclusive: Vec<String>,
    /// Whether option can be repeated
    pub repeated: bool,
}

impl ArgSpec {
    pub fn parse(spec: &str) -> Option<Self> {
        // Parse specs like:
        // '-v[verbose mode]'
        // '--help[show help]'
        // '(-v --verbose)'{-v,--verbose}'[be verbose]'
        // '*:file:_files'

        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        // Check for exclusion prefix
        let (exclusive, rest) = if spec.starts_with('(') {
            if let Some(end) = spec.find(')') {
                let excl: Vec<String> = spec[1..end].split_whitespace().map(String::from).collect();
                (excl, &spec[end + 1..])
            } else {
                (Vec::new(), spec)
            }
        } else {
            (Vec::new(), spec)
        };

        // Check for repeated prefix
        let (repeated, rest) = if rest.starts_with('*') {
            (true, &rest[1..])
        } else {
            (false, rest)
        };

        // Parse option and description
        let (option, description, action) = if rest.starts_with('-') {
            // Option spec
            if let Some(bracket_start) = rest.find('[') {
                if let Some(bracket_end) = rest.find(']') {
                    let opt = rest[..bracket_start].to_string();
                    let desc = rest[bracket_start + 1..bracket_end].to_string();
                    let act = if bracket_end + 1 < rest.len() {
                        rest[bracket_end + 1..].trim_start_matches(':').to_string()
                    } else {
                        String::new()
                    };
                    (opt, desc, act)
                } else {
                    (rest.to_string(), String::new(), String::new())
                }
            } else {
                (rest.to_string(), String::new(), String::new())
            }
        } else if rest.starts_with(':') || rest.starts_with('*') {
            // Argument spec
            let parts: Vec<&str> = rest.splitn(3, ':').collect();
            let desc = parts.get(1).unwrap_or(&"").to_string();
            let act = parts.get(2).unwrap_or(&"").to_string();
            (String::new(), desc, act)
        } else {
            return None;
        };

        Some(Self {
            option,
            description,
            action,
            exclusive,
            repeated,
        })
    }
}

/// State for comparguments builtin
#[derive(Clone, Debug, Default)]
pub struct CompArguments {
    /// Parsed argument specifications
    specs: Vec<ArgSpec>,
    /// Current state
    pub state: String,
    /// Context
    pub context: String,
    /// Options that have been used
    used_options: HashSet<String>,
}

impl CompArguments {
    pub fn new() -> Self {
        Self::default()
    }

    /// Initialize from argument specs (comparguments -i)
    pub fn init(&mut self, specs: &[String]) {
        self.specs.clear();
        self.used_options.clear();

        for spec in specs {
            if let Some(parsed) = ArgSpec::parse(spec) {
                self.specs.push(parsed);
            }
        }
    }

    /// Get current description and action
    pub fn get_current(&self, params: &CompParams) -> Option<(String, String)> {
        let current_word = params.current_word();

        // Check if completing an option
        if current_word.starts_with('-') {
            // Find matching options
            for spec in &self.specs {
                if !spec.option.is_empty() && spec.option.starts_with(&current_word) {
                    return Some((spec.description.clone(), spec.action.clone()));
                }
            }
        }

        // Check for argument specs (non-option)
        for spec in &self.specs {
            if spec.option.is_empty() {
                return Some((spec.description.clone(), spec.action.clone()));
            }
        }

        None
    }

    /// Get available options (not yet used, unless repeated)
    pub fn available_options(&self) -> Vec<&ArgSpec> {
        self.specs
            .iter()
            .filter(|s| {
                !s.option.is_empty() && (s.repeated || !self.used_options.contains(&s.option))
            })
            .collect()
    }

    /// Mark an option as used
    pub fn use_option(&mut self, opt: &str) {
        self.used_options.insert(opt.to_string());

        // Also mark exclusives
        for spec in &self.specs {
            if spec.option == opt {
                for excl in &spec.exclusive {
                    self.used_options.insert(excl.clone());
                }
            }
        }
    }
}

/// Value specification for _values
#[derive(Clone, Debug)]
pub struct ValueSpec {
    pub name: String,
    pub description: String,
    pub action: String,
    pub has_arg: bool,
}

impl ValueSpec {
    pub fn parse(spec: &str) -> Option<Self> {
        // Parse specs like:
        // 'name[description]'
        // 'name[description]:action'

        let spec = spec.trim();
        if spec.is_empty() {
            return None;
        }

        let (name, rest) = if let Some(bracket_start) = spec.find('[') {
            (spec[..bracket_start].to_string(), &spec[bracket_start..])
        } else if let Some(colon) = spec.find(':') {
            (spec[..colon].to_string(), &spec[colon..])
        } else {
            (spec.to_string(), "")
        };

        let (description, action, has_arg) = if rest.starts_with('[') {
            if let Some(bracket_end) = rest.find(']') {
                let desc = rest[1..bracket_end].to_string();
                let remaining = &rest[bracket_end + 1..];
                if remaining.starts_with(':') {
                    (desc, remaining[1..].to_string(), true)
                } else {
                    (desc, String::new(), false)
                }
            } else {
                (String::new(), String::new(), false)
            }
        } else if rest.starts_with(':') {
            (String::new(), rest[1..].to_string(), true)
        } else {
            (String::new(), String::new(), false)
        };

        Some(Self {
            name,
            description,
            action,
            has_arg,
        })
    }
}

/// State for compvalues builtin
#[derive(Clone, Debug, Default)]
pub struct CompValues {
    /// Value specifications
    specs: Vec<ValueSpec>,
    /// Separator between values (default ',')
    pub separator: char,
    /// Current state
    pub state: String,
}

impl CompValues {
    pub fn new() -> Self {
        Self {
            separator: ',',
            ..Default::default()
        }
    }

    /// Initialize from value specs (compvalues -i)
    pub fn init(&mut self, separator: char, specs: &[String]) {
        self.separator = separator;
        self.specs.clear();

        for spec in specs {
            if let Some(parsed) = ValueSpec::parse(spec) {
                self.specs.push(parsed);
            }
        }
    }

    /// Get completions for values
    pub fn get_completions(&self, prefix: &str) -> Vec<Completion> {
        self.specs
            .iter()
            .filter(|s| s.name.starts_with(prefix))
            .map(|s| {
                let mut comp = Completion::new(&s.name);
                if !s.description.is_empty() {
                    comp.disp = Some(format!("{} -- {}", s.name, s.description));
                }
                if s.has_arg {
                    // Add = suffix
                    comp.suf = Some("=".to_string());
                }
                comp
            })
            .collect()
    }
}

/// Description handling for compdescribe builtin (supports _describe)
#[derive(Clone, Debug, Default)]
pub struct CompDescribe {
    /// Items with descriptions
    items: Vec<(String, String)>,
    /// Maximum match width
    pub max_width: usize,
    /// Separator between match and description
    pub separator: String,
}

impl CompDescribe {
    pub fn new() -> Self {
        Self {
            separator: " -- ".to_string(),
            max_width: 0,
            ..Default::default()
        }
    }

    /// Initialize from item arrays (compdescribe -I)
    pub fn init(&mut self, items: &[(String, String)], separator: &str, max_width: usize) {
        self.items = items.to_vec();
        self.separator = separator.to_string();
        self.max_width = max_width;
    }

    /// Parse items from "match:description" format
    pub fn parse_items(specs: &[String]) -> Vec<(String, String)> {
        specs
            .iter()
            .filter_map(|s| {
                let parts: Vec<&str> = s.splitn(2, ':').collect();
                if parts.len() == 2 {
                    Some((parts[0].to_string(), parts[1].to_string()))
                } else if !s.is_empty() {
                    Some((s.clone(), String::new()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Get formatted completions
    pub fn get_completions(&self, prefix: &str) -> Vec<Completion> {
        self.items
            .iter()
            .filter(|(name, _)| name.starts_with(prefix))
            .map(|(name, desc)| {
                let mut comp = Completion::new(name);
                if !desc.is_empty() {
                    let display = if self.max_width > 0 && name.len() < self.max_width {
                        let padding = " ".repeat(self.max_width - name.len());
                        format!("{}{}{}{}", name, padding, self.separator, desc)
                    } else {
                        format!("{}{}{}", name, self.separator, desc)
                    };
                    comp.disp = Some(display);
                }
                comp
            })
            .collect()
    }
}

/// Execute _describe-like completion
pub fn describe_execute(
    state: &mut CompletionState,
    tag: &str,
    description: &str,
    items: &[(String, String)],
    group_name: Option<&str>,
) {
    let group = group_name.unwrap_or(tag);
    state.begin_group(group, true);

    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(group));
    }

    let prefix = state.params.prefix.clone();
    for (name, desc) in items {
        if name.starts_with(&prefix) {
            let mut comp = Completion::new(name);
            if !desc.is_empty() {
                comp.disp = Some(format!("{} -- {}", name, desc));
            }
            state.add_match(comp, Some(group));
        }
    }

    state.end_group();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comptags() {
        let mut tags = CompTags::new();
        tags.init(
            "test",
            &[
                "files".to_string(),
                "directories".to_string(),
                "commands".to_string(),
            ],
        );

        assert!(tags.try_tags(&["files".to_string()]));
        assert!(tags.is_set("files"));
        assert!(!tags.is_set("directories"));

        assert!(tags.try_tags(&["directories".to_string(), "commands".to_string()]));
        assert!(tags.is_set("directories"));
    }

    #[test]
    fn test_argspec_parse() {
        let spec = ArgSpec::parse("-v[verbose mode]").unwrap();
        assert_eq!(spec.option, "-v");
        assert_eq!(spec.description, "verbose mode");

        let spec = ArgSpec::parse("--help[show help]:action").unwrap();
        assert_eq!(spec.option, "--help");
        assert_eq!(spec.action, "action");

        // Test exclusion parsing
        let spec = ArgSpec::parse("(-a -b)--all[select all]").unwrap();
        assert_eq!(spec.exclusive, vec!["-a", "-b"]);
        assert_eq!(spec.option, "--all");
    }

    #[test]
    fn test_valuespec_parse() {
        let spec = ValueSpec::parse("debug[enable debug mode]").unwrap();
        assert_eq!(spec.name, "debug");
        assert_eq!(spec.description, "enable debug mode");
        assert!(!spec.has_arg);

        let spec = ValueSpec::parse("level[set level]:number").unwrap();
        assert_eq!(spec.name, "level");
        assert!(spec.has_arg);
    }

    #[test]
    fn test_compdescribe() {
        let items = CompDescribe::parse_items(&[
            "foo:first option".to_string(),
            "bar:second option".to_string(),
        ]);

        assert_eq!(items.len(), 2);
        assert_eq!(items[0], ("foo".to_string(), "first option".to_string()));
    }
}

// =============================================================================
// compfiles builtin
// =============================================================================

/// compfiles - File completion helper builtin
///
/// This builtin helps with file path completion by providing:
/// - Path reduction and expansion
/// - Pattern matching on file names
/// - Prefix/suffix stripping
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct CompFiles {
    paths: Vec<String>,
    pattern: Option<String>,
    prefix: Option<String>,
    suffix: Option<String>,
}

impl CompFiles {
    pub fn new() -> Self {
        Self::default()
    }

    /// compfiles -p: Path reduction (reduce array to prefix)
    /// Returns common prefix of all paths
    pub fn reduce_paths(paths: &[String]) -> String {
        if paths.is_empty() {
            return String::new();
        }
        if paths.len() == 1 {
            return paths[0].clone();
        }

        let first = &paths[0];
        let mut common_len = first.len();

        for path in &paths[1..] {
            let shared = first
                .chars()
                .zip(path.chars())
                .take_while(|(a, b)| a == b)
                .count();
            common_len = common_len.min(shared);
        }

        // Cut at last /
        let prefix = &first[..common_len];
        if let Some(pos) = prefix.rfind('/') {
            first[..=pos].to_string()
        } else {
            String::new()
        }
    }

    /// compfiles -P: Pattern matching on paths
    /// Filter paths matching pattern
    pub fn match_paths(paths: &[String], pattern: &str) -> Vec<String> {
        paths
            .iter()
            .filter(|p| {
                let name = p.rsplit('/').next().unwrap_or(p);
                crate::compset::glob_match(pattern, name)
            })
            .cloned()
            .collect()
    }

    /// compfiles -i: Check if any path matches
    pub fn has_match(paths: &[String], pattern: &str) -> bool {
        paths.iter().any(|p| {
            let name = p.rsplit('/').next().unwrap_or(p);
            crate::compset::glob_match(pattern, name)
        })
    }
}

// =============================================================================
// compgroups builtin
// =============================================================================

/// compgroups - Manage completion groups
///
/// Creates multiple completion groups with different sorting/uniqueness options
/// for each named group.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct CompGroups {
    groups: Vec<CompGroupConfig>,
}

/// Configuration for a single completion group
#[derive(Clone, Debug)]
pub struct CompGroupConfig {
    pub name: String,
    pub no_sort: bool,
    pub unique_consecutive: bool,
    pub unique_all: bool,
}

impl CompGroups {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create standard group set for a name (as zsh does)
    /// Creates 6 groups with different options for flexibility
    pub fn create_groups(names: &[&str]) -> Vec<CompGroupConfig> {
        let mut groups = Vec::new();

        for name in names {
            // CGF_NOSORT|CGF_UNIQCON
            groups.push(CompGroupConfig {
                name: name.to_string(),
                no_sort: true,
                unique_consecutive: true,
                unique_all: false,
            });
            // CGF_UNIQALL
            groups.push(CompGroupConfig {
                name: name.to_string(),
                no_sort: false,
                unique_consecutive: false,
                unique_all: true,
            });
            // CGF_NOSORT|CGF_UNIQCON (duplicate for flexibility)
            groups.push(CompGroupConfig {
                name: name.to_string(),
                no_sort: true,
                unique_consecutive: true,
                unique_all: false,
            });
            // CGF_UNIQALL (duplicate)
            groups.push(CompGroupConfig {
                name: name.to_string(),
                no_sort: false,
                unique_consecutive: false,
                unique_all: true,
            });
            // CGF_NOSORT
            groups.push(CompGroupConfig {
                name: name.to_string(),
                no_sort: true,
                unique_consecutive: false,
                unique_all: false,
            });
            // No flags (sorted, all duplicates kept)
            groups.push(CompGroupConfig {
                name: name.to_string(),
                no_sort: false,
                unique_consecutive: false,
                unique_all: false,
            });
        }

        groups
    }
}
