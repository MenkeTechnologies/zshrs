//! Native Rust implementation of _describe
//!
//! _describe adds completions with descriptions to the completion system.
//! It's called by most command-specific completion functions.

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// Options for _describe
#[derive(Clone, Debug, Default)]
pub struct DescribeOpts {
    /// Tag for this set of completions (-t)
    pub tag: Option<String>,
    /// Matcher spec (-M)
    pub matcher: Option<String>,
    /// Group name (-V or -J)
    pub group: Option<String>,
    /// Sorted group (-J)
    pub sorted: bool,
    /// Don't quote completions (-Q)
    pub no_quote: bool,
    /// Prefix (-P)
    pub prefix: Option<String>,
    /// Suffix (-S)
    pub suffix: Option<String>,
    /// Remove suffix on certain chars (-r)
    pub remove_suffix: Option<String>,
}

impl DescribeOpts {
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse _describe arguments
    /// Format: _describe [-t tag] description array ...
    pub fn parse(args: &[String]) -> (Self, String, Vec<String>) {
        let mut opts = Self::new();
        let mut i = 0;
        let mut description = String::new();
        let mut items = Vec::new();

        // Parse options
        while i < args.len() {
            match args[i].as_str() {
                "-t" => {
                    if i + 1 < args.len() {
                        opts.tag = Some(args[i + 1].clone());
                        i += 2;
                        continue;
                    }
                }
                "-M" => {
                    if i + 1 < args.len() {
                        opts.matcher = Some(args[i + 1].clone());
                        i += 2;
                        continue;
                    }
                }
                "-V" => {
                    if i + 1 < args.len() {
                        opts.group = Some(args[i + 1].clone());
                        opts.sorted = false;
                        i += 2;
                        continue;
                    }
                }
                "-J" => {
                    if i + 1 < args.len() {
                        opts.group = Some(args[i + 1].clone());
                        opts.sorted = true;
                        i += 2;
                        continue;
                    }
                }
                "-P" => {
                    if i + 1 < args.len() {
                        opts.prefix = Some(args[i + 1].clone());
                        i += 2;
                        continue;
                    }
                }
                "-S" => {
                    if i + 1 < args.len() {
                        opts.suffix = Some(args[i + 1].clone());
                        i += 2;
                        continue;
                    }
                }
                "-Q" => {
                    opts.no_quote = true;
                    i += 1;
                    continue;
                }
                "-r" => {
                    if i + 1 < args.len() {
                        opts.remove_suffix = Some(args[i + 1].clone());
                        i += 2;
                        continue;
                    }
                }
                arg if !arg.starts_with('-') => {
                    // First non-option is description
                    if description.is_empty() {
                        description = arg.to_string();
                    } else {
                        // Rest are item arrays
                        items.push(arg.to_string());
                    }
                }
                _ => {}
            }
            i += 1;
        }

        (opts, description, items)
    }
}

/// An item with optional description for _describe
#[derive(Clone, Debug)]
pub struct DescribeItem {
    /// The completion string
    pub value: String,
    /// Optional description
    pub description: String,
}

impl DescribeItem {
    /// Parse "value:description" format
    pub fn parse(s: &str) -> Self {
        if let Some(pos) = s.find(':') {
            Self {
                value: s[..pos].to_string(),
                description: s[pos + 1..].to_string(),
            }
        } else {
            Self {
                value: s.to_string(),
                description: String::new(),
            }
        }
    }

    /// Parse from escaped format "value\:with\:colons:description"
    pub fn parse_escaped(s: &str) -> Self {
        let mut value = String::new();
        let mut description = String::new();
        let mut in_value = true;
        let mut chars = s.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '\\' {
                if let Some(&next) = chars.peek() {
                    if next == ':' {
                        value.push(':');
                        chars.next();
                        continue;
                    }
                }
                if in_value {
                    value.push(c);
                } else {
                    description.push(c);
                }
            } else if c == ':' && in_value {
                in_value = false;
            } else if in_value {
                value.push(c);
            } else {
                description.push(c);
            }
        }

        Self { value, description }
    }
}

/// Execute _describe completion
pub fn describe_execute(
    state: &mut CompletionState,
    opts: &DescribeOpts,
    description: &str,
    items: &[DescribeItem],
) -> bool {
    let prefix = state.params.prefix.clone();
    let group_name = opts
        .group
        .as_deref()
        .or(opts.tag.as_deref())
        .unwrap_or("default");

    state.begin_group(group_name, opts.sorted);

    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(group_name));
    }

    let mut added = false;

    for item in items {
        // Check if matches prefix
        if !item.value.starts_with(&prefix) {
            continue;
        }

        let mut comp_str = item.value.clone();

        // Add prefix/suffix
        if let Some(ref pfx) = opts.prefix {
            comp_str = format!("{}{}", pfx, comp_str);
        }
        if let Some(ref sfx) = opts.suffix {
            comp_str.push_str(sfx);
        }

        let mut comp = Completion::new(&comp_str);

        // Set display with description
        if !item.description.is_empty() {
            comp.disp = Some(format!("{} -- {}", item.value, item.description));
        }

        if opts.no_quote {
            comp.flags |= CompletionFlags::NOQUOTE;
        }

        state.add_match(comp, Some(group_name));
        added = true;
    }

    state.end_group();
    added
}

/// Parse items from string array (for use with shell arrays)
pub fn parse_items(specs: &[String]) -> Vec<DescribeItem> {
    specs
        .iter()
        .map(|s| DescribeItem::parse_escaped(s))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_item() {
        let item = DescribeItem::parse("foo:description of foo");
        assert_eq!(item.value, "foo");
        assert_eq!(item.description, "description of foo");
    }

    #[test]
    fn test_parse_item_no_desc() {
        let item = DescribeItem::parse("foo");
        assert_eq!(item.value, "foo");
        assert_eq!(item.description, "");
    }

    #[test]
    fn test_parse_escaped() {
        let item = DescribeItem::parse_escaped(r"foo\:bar:description");
        assert_eq!(item.value, "foo:bar");
        assert_eq!(item.description, "description");
    }

    #[test]
    fn test_parse_opts() {
        let (opts, desc, items) = DescribeOpts::parse(&[
            "-t".to_string(),
            "commands".to_string(),
            "-J".to_string(),
            "git commands".to_string(),
            "command".to_string(),
            "items_array".to_string(),
        ]);

        assert_eq!(opts.tag, Some("commands".to_string()));
        assert_eq!(opts.group, Some("git commands".to_string()));
        assert!(opts.sorted);
        assert_eq!(desc, "command");
        assert_eq!(items, vec!["items_array"]);
    }
}
