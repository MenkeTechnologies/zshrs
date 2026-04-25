//! Completion utility functions for ZLE
//!
//! Port from zsh/Src/Zle/computil.c (5,180 lines)
//!
//! The full utility library is in compsys/computil.rs (674 lines).
//! This module provides _describe, _values, _alternative, _combination,
//! and the compdescribe/comparguments/compvalues builtins.
//!
//! Key C functions and their Rust locations:
//! - bin_compdescribe  → compsys::describe::describe()
//! - bin_comparguments → compsys::arguments (full _arguments)
//! - bin_compvalues    → compsys::computil::compvalues()
//! - bin_comptags      → compsys::state::comptags()
//! - bin_comptry       → compsys::state::comptry()

use std::collections::HashMap;

/// Completion description set (from computil.c CDSet)
#[derive(Debug, Clone)]
pub struct CompDescSet {
    pub tag: String,
    pub group: String,
    pub items: Vec<CompDescItem>,
    pub options: DescOptions,
}

/// A single completion with description
#[derive(Debug, Clone)]
pub struct CompDescItem {
    pub word: String,
    pub description: String,
    pub hidden: bool,
}

/// Options for _describe (from computil.c)
#[derive(Debug, Clone, Default)]
pub struct DescOptions {
    pub verbose: bool,
    pub sort: bool,
    pub unique: bool,
    pub group_name: Option<String>,
    pub separator: String,
}

impl Default for CompDescSet {
    fn default() -> Self {
        CompDescSet {
            tag: String::new(),
            group: String::new(),
            items: Vec::new(),
            options: DescOptions {
                separator: " -- ".to_string(),
                ..Default::default()
            },
        }
    }
}

/// Parse "word:description" format (from computil.c cd_get)
pub fn cd_get(spec: &str) -> CompDescItem {
    if let Some((word, desc)) = spec.split_once(':') {
        CompDescItem {
            word: word.to_string(),
            description: desc.to_string(),
            hidden: false,
        }
    } else {
        CompDescItem {
            word: spec.to_string(),
            description: String::new(),
            hidden: false,
        }
    }
}

/// Parse multiple specs into a description set (from computil.c cd_init)
pub fn cd_init(specs: &[String], tag: &str, group: &str) -> CompDescSet {
    let items: Vec<CompDescItem> = specs.iter().map(|s| cd_get(s)).collect();
    CompDescSet {
        tag: tag.to_string(),
        group: group.to_string(),
        items,
        ..Default::default()
    }
}

/// Sort items in a description set (from computil.c cd_sort)
pub fn cd_sort(set: &mut CompDescSet) {
    set.items.sort_by(|a, b| a.word.cmp(&b.word));
}

/// Calculate display widths (from computil.c cd_calc)
pub fn cd_calc(items: &[CompDescItem], separator: &str) -> (usize, usize) {
    let max_word = items.iter().map(|i| i.word.len()).max().unwrap_or(0);
    let max_desc = items.iter().map(|i| i.description.len()).max().unwrap_or(0);
    (max_word, max_word + separator.len() + max_desc)
}

/// Format items for display (from computil.c cd_prep)
pub fn cd_prep(items: &[CompDescItem], separator: &str) -> Vec<String> {
    let (max_word, _) = cd_calc(items, separator);
    items
        .iter()
        .map(|item| {
            if item.description.is_empty() {
                item.word.clone()
            } else {
                format!(
                    "{:<width$}{}{}",
                    item.word,
                    separator,
                    item.description,
                    width = max_word
                )
            }
        })
        .collect()
}

/// Check if groups want sorting (from computil.c cd_groups_want_sorting)
pub fn cd_groups_want_sorting(sets: &[CompDescSet]) -> bool {
    sets.iter().all(|s| s.options.sort)
}

/// Concatenate arrays from description sets (from computil.c cd_arrcat)
pub fn cd_arrcat(sets: &[CompDescSet]) -> Vec<String> {
    sets.iter()
        .flat_map(|s| s.items.iter().map(|i| i.word.clone()))
        .collect()
}

/// Duplicate description set arrays (from computil.c cd_arrdup)
pub fn cd_arrdup(set: &CompDescSet) -> CompDescSet {
    set.clone()
}

/// Free description sets (from computil.c freecdsets) — no-op in Rust
pub fn freecdsets(_sets: Vec<CompDescSet>) {}

/// Group items by description (from computil.c cd_group)
pub fn cd_group(items: &[CompDescItem]) -> HashMap<String, Vec<CompDescItem>> {
    let mut groups: HashMap<String, Vec<CompDescItem>> = HashMap::new();
    for item in items {
        let key = if item.description.is_empty() {
            "(no description)".to_string()
        } else {
            item.description.clone()
        };
        groups.entry(key).or_default().push(item.clone());
    }
    groups
}

/// Compare arrays for equality (from computil.c arrcmp)
pub fn arrcmp(a: &[String], b: &[String]) -> bool {
    a == b
}

// --- _arguments support (from computil.c parse_caarg / alloc_cadef / set_cadef_opts) ---

/// Completion argument definition (from computil.c Caarg)
#[derive(Debug, Clone)]
pub struct CompArgDef {
    pub num: i32,       // Argument position (1-based, -1 for rest)
    pub action: String, // Action to take
    pub description: String,
    pub optional: bool,
    pub repeated: bool,
}

/// Completion option definition (from computil.c Caopt)
#[derive(Debug, Clone)]
pub struct CompOptDef {
    pub name: String, // Option name (e.g., "-v", "--verbose")
    pub description: String,
    pub has_arg: bool,          // Whether option takes an argument
    pub arg_desc: String,       // Argument description
    pub exclusive: Vec<String>, // Mutually exclusive options
}

/// Full completion definition for a command (from computil.c Cadef)
#[derive(Debug, Clone, Default)]
pub struct CompCommandDef {
    pub options: Vec<CompOptDef>,
    pub arguments: Vec<CompArgDef>,
    pub subcommands: HashMap<String, CompCommandDef>,
}

/// Parse a _arguments spec string (from computil.c parse_caarg)
pub fn parse_caarg(spec: &str) -> Option<CompArgDef> {
    // Format: "N:description:action" or "*:description:action"
    let parts: Vec<&str> = spec.splitn(3, ':').collect();
    if parts.is_empty() {
        return None;
    }

    let (num, optional) = if parts[0] == "*" {
        (-1, false)
    } else if parts[0].starts_with('?') {
        (parts[0][1..].parse().unwrap_or(0), true)
    } else {
        (parts[0].parse().unwrap_or(0), false)
    };

    Some(CompArgDef {
        num,
        description: parts.get(1).unwrap_or(&"").to_string(),
        action: parts.get(2).unwrap_or(&"").to_string(),
        optional,
        repeated: parts[0] == "*",
    })
}

/// Parse an option spec (from computil.c set_cadef_opts)
pub fn parse_caopt(spec: &str) -> Option<CompOptDef> {
    // Format: "-o[description]" or "--option[description]:arg_desc:action"
    // or "(-a -b)-c[description]"

    let spec = spec.trim();
    if spec.is_empty() {
        return None;
    }

    // Extract exclusions
    let (exclusive, rest) = if spec.starts_with('(') {
        if let Some(close) = spec.find(')') {
            let excl: Vec<String> = spec[1..close]
                .split_whitespace()
                .map(String::from)
                .collect();
            (excl, spec[close + 1..].trim())
        } else {
            (Vec::new(), spec)
        }
    } else {
        (Vec::new(), spec)
    };

    // Extract option name
    let (name, after_name) = if rest.starts_with("--") {
        let end = rest
            .find('[')
            .unwrap_or(rest.find(':').unwrap_or(rest.len()));
        (&rest[..end], &rest[end..])
    } else if rest.starts_with('-') {
        let end = if rest.len() > 2 { 2 } else { rest.len() };
        let end = rest[end..]
            .find('[')
            .map(|i| i + end)
            .unwrap_or(rest[end..].find(':').map(|i| i + end).unwrap_or(rest.len()));
        (&rest[..end], &rest[end..])
    } else {
        return None;
    };

    // Extract description from [...]
    let description = if let Some(start) = after_name.find('[') {
        if let Some(end) = after_name[start..].find(']') {
            after_name[start + 1..start + end].to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Check for argument
    let has_arg = after_name.contains(':');
    let arg_desc = if has_arg {
        after_name.rsplit(':').next().unwrap_or("").to_string()
    } else {
        String::new()
    };

    Some(CompOptDef {
        name: name.to_string(),
        description,
        has_arg,
        arg_desc,
        exclusive,
    })
}

/// Remove backslash-escaped colons (from computil.c rembslashcolon)
pub fn rembslashcolon(s: &str) -> String {
    s.replace("\\:", ":")
}

/// Add backslash before colons (from computil.c bslashcolon)
pub fn bslashcolon(s: &str) -> String {
    s.replace(':', "\\:")
}

/// Single index lookup (from computil.c single_index)
pub fn single_index(arr: &[String], val: &str) -> Option<usize> {
    arr.iter().position(|s| s == val)
}

/// Free completion argument definitions (from computil.c freecaargs/freecadef) — no-op
pub fn freecaargs(_args: Vec<CompArgDef>) {}
pub fn freecadef(_def: CompCommandDef) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cd_get() {
        let item = cd_get("commit:Record changes");
        assert_eq!(item.word, "commit");
        assert_eq!(item.description, "Record changes");

        let item = cd_get("plain");
        assert_eq!(item.word, "plain");
        assert_eq!(item.description, "");
    }

    #[test]
    fn test_cd_init() {
        let specs = vec!["a:first".into(), "b:second".into(), "c:third".into()];
        let set = cd_init(&specs, "options", "group1");
        assert_eq!(set.items.len(), 3);
        assert_eq!(set.tag, "options");
    }

    #[test]
    fn test_cd_sort() {
        let mut set = cd_init(
            &vec!["c:third".into(), "a:first".into(), "b:second".into()],
            "",
            "",
        );
        cd_sort(&mut set);
        assert_eq!(set.items[0].word, "a");
        assert_eq!(set.items[2].word, "c");
    }

    #[test]
    fn test_cd_prep() {
        let items = vec![
            CompDescItem {
                word: "short".into(),
                description: "A short one".into(),
                hidden: false,
            },
            CompDescItem {
                word: "longer".into(),
                description: "A longer one".into(),
                hidden: false,
            },
        ];
        let formatted = cd_prep(&items, " -- ");
        assert!(formatted[0].contains(" -- "));
        assert!(formatted[1].contains(" -- "));
    }

    #[test]
    fn test_parse_caarg() {
        let arg = parse_caarg("1:file:_files").unwrap();
        assert_eq!(arg.num, 1);
        assert_eq!(arg.description, "file");
        assert_eq!(arg.action, "_files");

        let arg = parse_caarg("*:rest args:_files").unwrap();
        assert_eq!(arg.num, -1);
        assert!(arg.repeated);
    }

    #[test]
    fn test_parse_caopt() {
        let opt = parse_caopt("-v[verbose output]").unwrap();
        assert_eq!(opt.name, "-v");
        assert_eq!(opt.description, "verbose output");
        assert!(!opt.has_arg);

        let opt = parse_caopt("--output[output file]:file:_files").unwrap();
        assert_eq!(opt.name, "--output");
        assert!(opt.has_arg);
    }

    #[test]
    fn test_rembslashcolon() {
        assert_eq!(rembslashcolon("a\\:b\\:c"), "a:b:c");
    }

    #[test]
    fn test_bslashcolon() {
        assert_eq!(bslashcolon("a:b:c"), "a\\:b\\:c");
    }

    #[test]
    fn test_cd_group() {
        let items = vec![
            CompDescItem {
                word: "a".into(),
                description: "group1".into(),
                hidden: false,
            },
            CompDescItem {
                word: "b".into(),
                description: "group1".into(),
                hidden: false,
            },
            CompDescItem {
                word: "c".into(),
                description: "group2".into(),
                hidden: false,
            },
        ];
        let groups = cd_group(&items);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups["group1"].len(), 2);
    }
}
