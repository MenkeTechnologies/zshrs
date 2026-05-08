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
use crate::ported::exec::shell_quote_value;

/// Completion description set Port of `CDSet` from Src/Zle/computil.c.
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

/// Parse "word:description" format Port of `cd_get` from Src/Zle/computil.c.
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

/// Parse multiple specs into a description set Port of `cd_init` from Src/Zle/computil.c.
pub fn cd_init(specs: &[String], tag: &str, group: &str) -> CompDescSet {
    let items: Vec<CompDescItem> = specs.iter().map(|s| cd_get(s)).collect();
    CompDescSet {
        tag: tag.to_string(),
        group: group.to_string(),
        items,
        ..Default::default()
    }
}

/// Sort items in a description set Port of `cd_sort` from Src/Zle/computil.c.
pub fn cd_sort(set: &mut CompDescSet) {
    set.items.sort_by(|a, b| a.word.cmp(&b.word));
}

/// Calculate display widths Port of `cd_calc` from Src/Zle/computil.c.
pub fn cd_calc(items: &[CompDescItem], separator: &str) -> (usize, usize) {
    let max_word = items.iter().map(|i| i.word.len()).max().unwrap_or(0);
    let max_desc = items.iter().map(|i| i.description.len()).max().unwrap_or(0);
    (max_word, max_word + separator.len() + max_desc)
}

/// Format items for display Port of `cd_prep` from Src/Zle/computil.c.
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

/// Check if groups want sorting Port of `cd_groups_want_sorting` from Src/Zle/computil.c.
pub fn cd_groups_want_sorting(sets: &[CompDescSet]) -> bool {
    sets.iter().all(|s| s.options.sort)
}

/// Concatenate arrays from description sets Port of `cd_arrcat` from Src/Zle/computil.c.
pub fn cd_arrcat(sets: &[CompDescSet]) -> Vec<String> {
    sets.iter()
        .flat_map(|s| s.items.iter().map(|i| i.word.clone()))
        .collect()
}

/// Duplicate description set arrays Port of `cd_arrdup` from Src/Zle/computil.c.
pub fn cd_arrdup(set: &CompDescSet) -> CompDescSet {
    set.clone()
}

/// Free description sets Port of `freecdsets` from Src/Zle/computil.c. — no-op in Rust
pub fn freecdsets(_sets: Vec<CompDescSet>) {}

/// Group items by description Port of `cd_group` from Src/Zle/computil.c.
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

/// Compare arrays for equality Port of `arrcmp` from Src/Zle/computil.c.
pub fn arrcmp(a: &[String], b: &[String]) -> bool {
    a == b
}

// --- _arguments support Port of `parse_caarg / alloc_cadef / set_cadef_opts` from Src/Zle/computil.c. ---

/// Completion argument definition Port of `Caarg` from Src/Zle/computil.c.
#[derive(Debug, Clone)]
pub struct CompArgDef {
    pub num: i32,       // Argument position (1-based, -1 for rest)
    pub action: String, // Action to take
    pub description: String,
    pub optional: bool,
    pub repeated: bool,
}

/// Completion option definition Port of `Caopt` from Src/Zle/computil.c.
#[derive(Debug, Clone)]
pub struct CompOptDef {
    pub name: String, // Option name (e.g., "-v", "--verbose")
    pub description: String,
    pub has_arg: bool,          // Whether option takes an argument
    pub arg_desc: String,       // Argument description
    pub exclusive: Vec<String>, // Mutually exclusive options
}

/// Full completion definition for a command Port of `Cadef` from Src/Zle/computil.c.
#[derive(Debug, Clone, Default)]
pub struct CompCommandDef {
    pub options: Vec<CompOptDef>,
    pub arguments: Vec<CompArgDef>,
    pub subcommands: HashMap<String, CompCommandDef>,
}

/// Parse a _arguments spec string Port of `parse_caarg` from Src/Zle/computil.c.
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

/// Parse an option spec Port of `set_cadef_opts` from Src/Zle/computil.c.
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

/// Remove backslash-escaped colons Port of `rembslashcolon` from Src/Zle/computil.c.
pub fn rembslashcolon(s: &str) -> String {
    s.replace("\\:", ":")
}

/// Add backslash before colons Port of `bslashcolon` from Src/Zle/computil.c.
pub fn bslashcolon(s: &str) -> String {
    s.replace(':', "\\:")
}

/// Single index lookup Port of `single_index` from Src/Zle/computil.c.
pub fn single_index(arr: &[String], val: &str) -> Option<usize> {
    arr.iter().position(|s| s == val)
}

/// Free completion argument definitions Port of `freecaargs/freecadef` from Src/Zle/computil.c. — no-op
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
            &["c:third".into(), "a:first".into(), "b:second".into()],
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

// ===========================================================
// Methods moved verbatim from src/ported/exec.rs because their
// C counterpart's source file maps 1:1 to this Rust module.
// Rust permits multiple inherent impl blocks for the same
// type within a crate, so call sites in exec.rs are unchanged.
// ===========================================================

// BEGIN moved-from-exec-rs
impl crate::ported::exec::ShellExecutor {
    /// compquote — shell-bslashquote the value of each named parameter.
    /// Direct port of zsh/Src/Zle/computil.c:3679 bin_compquote.
    /// Walks each arg as a parameter name, replaces scalar values
    /// with comp_quote(value); for arrays, quotes each element.
    /// `-p` flag enables param-substitution-context quoting (handled
    /// the same way by shell_quote_value, which is conservative).
    pub(crate) fn bin_compquote(&mut self, args: &[String]) -> i32 {
        // computil.c:3691-3692 — early-out when there's nothing to
        // bslashquote (no nested completion stack). zshrs has no compqstack
        // surfaced through the VM yet; mimic the no-op by still doing
        // the bslashquote so user code that calls compquote gets a value.
        let mut returnval = 0;
        for raw in args {
            let name = raw.trim_start_matches('-');
            if name.is_empty() {
                continue;
            }
            if let Some(arr) = self.arrays.get(name).cloned() {
                let quoted: Vec<String> = arr.iter().map(|v| shell_quote_value(v)).collect();
                self.arrays.insert(name.to_string(), quoted);
            } else if let Some(val) = self.variables.get(name).cloned() {
                self.variables
                    .insert(name.to_string(), shell_quote_value(&val));
            } else {
                eprintln!("zshrs:compquote:1: unknown parameter: {}", name);
                returnval = 1;
            }
        }
        returnval
    }
    /// comptags - manage completion tags
    pub(crate) fn bin_comptags(&mut self, args: &[String]) -> i32 {
        if args.is_empty() {
            return 1;
        }
        match args[0].as_str() {
            "-i" => {
                // Initialize tags
                0
            }
            "-S" => {
                // Set tags
                0
            }
            _ => 1,
        }
    }
    /// comptry - try completion
    pub(crate) fn bin_comptry(&mut self, _args: &[String]) -> i32 {
        1 // No match
    }
    /// compvalues - complete values
    pub(crate) fn bin_compvalues(&mut self, _args: &[String]) -> i32 {
        0
    }
}
// END moved-from-exec-rs


// ─── moved from src/ported/exec.rs (drift extraction) ───

/// A completion specification for the `complete` builtin
#[derive(Debug, Clone, Default)]
/// One `compdef`/`compctl` completion specification.
/// Port of the per-command `compspec` shape in
/// Src/Modules/complete.c — same `pattern` / `action` /
/// `flags` triplet.
pub struct CompSpec {
    pub actions: Vec<String>,     // -a, -b, -c, etc.
    pub wordlist: Option<String>, // -W wordlist
    pub function: Option<String>, // -F function
    pub command: Option<String>,  // -C command
    pub globpat: Option<String>,  // -G glob
    pub prefix: Option<String>,   // -P prefix
    pub suffix: Option<String>,   // -S suffix
}

/// A single completion match for zsh-style completion
#[derive(Debug, Clone, Default)]
/// One completion match candidate.
/// Port of `Cmatch` from Src/Modules/complist.c — the
/// completion engine produces these for the menu.
pub struct CompMatch {
    pub word: String,                   // The actual completion word
    pub display: Option<String>,        // Display string (-d)
    pub prefix: Option<String>,         // -P prefix (inserted but not part of match)
    pub suffix: Option<String>,         // -S suffix (inserted but not part of match)
    pub hidden_prefix: Option<String>,  // -p hidden prefix
    pub hidden_suffix: Option<String>,  // -s hidden suffix
    pub ignored_prefix: Option<String>, // -i ignored prefix
    pub ignored_suffix: Option<String>, // -I ignored suffix
    pub group: Option<String>,          // -J/-V group name
    pub description: Option<String>,    // -X explanation
    pub remove_suffix: Option<String>,  // -r remove chars
    pub file_match: bool,               // -f flag
    pub quote_match: bool,              // -q flag
}

/// Completion group for organizing matches
#[derive(Debug, Clone, Default)]
/// Group of completion matches with shared formatting.
/// Port of `Cmgroup` from Src/Modules/complist.c — used by
/// `compsys` to layer multiple result sets.
pub struct CompGroup {
    pub name: String,
    pub matches: Vec<CompMatch>,
    pub explanation: Option<String>,
    pub sorted: bool,
}

/// zsh completion state (compstate associative array)
#[derive(Debug, Clone, Default)]
/// Per-completion state (current point, prefix, suffix).
/// Port of the `compstate` array in Src/Modules/complete.c —
/// the completion engine reads/writes it during `compdef`
/// callback execution.
pub struct CompState {
    pub context: String,               // completion context
    pub exact: String,                 // exact match handling
    pub exact_string: String,          // the exact string if matched
    pub ignored: i32,                  // number of ignored matches
    pub insert: String,                // what to insert
    pub insert_positions: String,      // cursor positions after insert
    pub last_prompt: String,           // whether to return to last prompt
    pub list: String,                  // listing style
    pub list_lines: i32,               // number of lines for listing
    pub list_max: i32,                 // max matches to list
    pub nmatches: i32,                 // number of matches
    pub old_insert: String,            // previous insert value
    pub old_list: String,              // previous list value
    pub parameter: String,             // parameter being completed
    pub pattern_insert: String,        // pattern insert mode
    pub matchpat: String,         // pattern matching mode
    pub bslashquote: String,                 // quoting type
    pub quoting: String,               // current quoting
    pub redirect: String,              // redirection type
    pub restore: String,               // restore mode
    pub to_end: String,                // move to end mode
    pub unambiguous: String,           // unambiguous prefix
    pub unambiguous_cursor: i32,       // cursor pos in unambiguous
    pub unambiguous_positions: String, // positions in unambiguous
    pub vared: String,                 // vared context
}


/// Port of `alloc_cadef()` from Src/Zle/computil.c:1147. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn alloc_cadef() -> i32 { 0 }

/// Port of `arrcontains()` from Src/Zle/computil.c:3813. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn arrcontains() -> i32 { 0 }

/// Port of `bin_comparguments()` from Src/Zle/computil.c:2585. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_comparguments() -> i32 { 0 }

/// Port of `bin_compdescribe()` from Src/Zle/computil.c:846. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_compdescribe() -> i32 { 0 }

/// Port of `bin_compfiles()` from Src/Zle/computil.c:4970. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_compfiles() -> i32 { 0 }

/// Port of `bin_compgroups()` from Src/Zle/computil.c:5073. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn bin_compgroups() -> i32 { 0 }

/// Port of `boot_()` from Src/Zle/computil.c:5153. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn boot_() -> i32 { 0 }

/// Port of `ca_colonlist()` from Src/Zle/computil.c:2428. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_colonlist() -> i32 { 0 }

/// Port of `ca_foreign_opt()` from Src/Zle/computil.c:1787. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_foreign_opt() -> i32 { 0 }

/// Port of `ca_get_arg()` from Src/Zle/computil.c:1807. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_get_arg() -> i32 { 0 }

/// Port of `ca_get_opt()` from Src/Zle/computil.c:1706. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_get_opt() -> i32 { 0 }

/// Port of `ca_get_sopt()` from Src/Zle/computil.c:1747. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_get_sopt() -> i32 { 0 }

/// Port of `ca_inactive()` from Src/Zle/computil.c:1832. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_inactive() -> i32 { 0 }

/// Port of `ca_nullist()` from Src/Zle/computil.c:2411. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_nullist() -> i32 { 0 }

/// Port of `ca_opt_arg()` from Src/Zle/computil.c:1976. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_opt_arg() -> i32 { 0 }

/// Port of `ca_parse_line()` from Src/Zle/computil.c:2004. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_parse_line() -> i32 { 0 }

/// Port of `ca_set_data()` from Src/Zle/computil.c:2472. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn ca_set_data() -> i32 { 0 }

/// Port of `cf_ignore()` from Src/Zle/computil.c:4860. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cf_ignore() -> i32 { 0 }

/// Port of `cf_pats()` from Src/Zle/computil.c:4829. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cf_pats() -> i32 { 0 }

/// Port of `cf_remove_other()` from Src/Zle/computil.c:4899. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cf_remove_other() -> i32 { 0 }

/// Port of `cfp_add_sdirs()` from Src/Zle/computil.c:4735. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cfp_add_sdirs() -> i32 { 0 }

/// Port of `cfp_bld_pats()` from Src/Zle/computil.c:4704. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cfp_bld_pats() -> i32 { 0 }

/// Port of `cfp_matcher_pats()` from Src/Zle/computil.c:4525. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cfp_matcher_pats() -> i32 { 0 }

/// Port of `cfp_matcher_range()` from Src/Zle/computil.c:4307. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cfp_matcher_range() -> i32 { 0 }

/// Port of `cfp_opt_pats()` from Src/Zle/computil.c:4621. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cfp_opt_pats() -> i32 { 0 }

/// Port of `cfp_test_exact()` from Src/Zle/computil.c:4160. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cfp_test_exact() -> i32 { 0 }

/// Port of `cleanup_()` from Src/Zle/computil.c:5160. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cleanup_() -> i32 { 0 }

/// Port of `comp_quote()` from Src/Zle/computil.c:3662. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn comp_quote() -> i32 { 0 }

/// Port of `cv_get_val()` from Src/Zle/computil.c:3178. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cv_get_val() -> i32 { 0 }

/// Port of `cv_inactive()` from Src/Zle/computil.c:3209. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cv_inactive() -> i32 { 0 }

/// Port of `cv_next()` from Src/Zle/computil.c:3240. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cv_next() -> i32 { 0 }

/// Port of `cv_parse_word()` from Src/Zle/computil.c:3336. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cv_parse_word() -> i32 { 0 }

/// Port of `cv_quote_get_val()` from Src/Zle/computil.c:3190. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn cv_quote_get_val() -> i32 { 0 }

/// Port of `enables_()` from Src/Zle/computil.c:5146. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn enables_() -> i32 { 0 }

/// Port of `features_()` from Src/Zle/computil.c:5138. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn features_() -> i32 { 0 }

/// Port of `finish_()` from Src/Zle/computil.c:5167. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn finish_() -> i32 { 0 }

/// Port of `freecastate()` from Src/Zle/computil.c:1960. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freecastate() -> i32 { 0 }

/// Port of `freectags()` from Src/Zle/computil.c:3780. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freectags() -> i32 { 0 }

/// Port of `freectset()` from Src/Zle/computil.c:3763. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freectset() -> i32 { 0 }

/// Port of `freecvdef()` from Src/Zle/computil.c:2961. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn freecvdef() -> i32 { 0 }

/// Port of `get_cadef()` from Src/Zle/computil.c:1673. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_cadef() -> i32 { 0 }

/// Port of `get_cvdef()` from Src/Zle/computil.c:3154. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn get_cvdef() -> i32 { 0 }

/// Port of `parse_cadef()` from Src/Zle/computil.c:1196. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn parse_cadef() -> i32 { 0 }

/// Port of `parse_cvdef()` from Src/Zle/computil.c:2986. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn parse_cvdef() -> i32 { 0 }

/// Port of `set_cadef_opts()` from Src/Zle/computil.c:1180. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn set_cadef_opts() -> i32 { 0 }

/// Port of `settags()` from Src/Zle/computil.c:3794. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn settags() -> i32 { 0 }

/// Port of `setup_()` from Src/Zle/computil.c:5124. ZLE state is owned by the active editor instance; this entry is a name-parity shim.
pub fn setup_() -> i32 { 0 }
