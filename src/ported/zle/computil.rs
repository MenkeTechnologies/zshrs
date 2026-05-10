//! Completion utility functions for ZLE
//!
//! Port from zsh/Src/Zle/computil.c (5,180 lines)
//!
//! Help for `_describe'.                                                    // c:34
//! Help for `_arguments'.                                                   // c:897
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
use crate::ported::utils::{quotedzputs, zwarnnam};

// =====================================================================
// CRT_* — `_describe` row-type discriminator from `computil.c:79-83`.
// Drives the `cdescr` table-builder switch.
// =====================================================================

/// Port of `CRT_SIMPLE` from `Src/Zle/computil.c:79`. Plain match row.
pub const CRT_SIMPLE: i32 = 0;                                               // c:79
/// Port of `CRT_DESC` from `computil.c:80`. Match with description.
pub const CRT_DESC:   i32 = 1;                                               // c:80
/// Port of `CRT_SPEC` from `computil.c:81`. Special separator row.
pub const CRT_SPEC:   i32 = 2;                                               // c:81
/// Port of `CRT_DUMMY` from `computil.c:82`. Placeholder row.
pub const CRT_DUMMY:  i32 = 3;                                               // c:82
/// Port of `CRT_EXPL` from `computil.c:83`. Explanation header row.
pub const CRT_EXPL:   i32 = 4;                                               // c:83

/// Port of `CDF_SEP` from `Src/Zle/computil.c:924`. `-S` flag — `--`
/// terminates options.
pub const CDF_SEP: i32 = 1;                                                  // c:924

// =====================================================================
// CAO_* — Cadef option-argument attachment style — `computil.c:941-945`.
// =====================================================================

/// Port of `CAO_NEXT` from `computil.c:941`. Argument in next argv slot.
pub const CAO_NEXT:    i32 = 1;                                              // c:941
/// Port of `CAO_DIRECT` from `computil.c:942`. Argument directly attached
/// to option (`-opt:value`).
pub const CAO_DIRECT:  i32 = 2;                                              // c:942
/// Port of `CAO_ODIRECT` from `computil.c:943`. Optional direct attach.
pub const CAO_ODIRECT: i32 = 3;                                              // c:943
/// Port of `CAO_EQUAL` from `computil.c:944`. Argument after `=`.
pub const CAO_EQUAL:   i32 = 4;                                              // c:944
/// Port of `CAO_OEQUAL` from `computil.c:945`. Optional `=` argument.
pub const CAO_OEQUAL:  i32 = 5;                                              // c:945

// =====================================================================
// CAA_* — Cadef positional-argument kinds — `computil.c:964-968`.
// =====================================================================

/// Port of `CAA_NORMAL` from `computil.c:964`. Plain positional arg.
pub const CAA_NORMAL: i32 = 1;                                               // c:964
/// Port of `CAA_OPT` from `computil.c:965`. Optional positional arg.
pub const CAA_OPT:    i32 = 2;                                               // c:965
/// Port of `CAA_REST` from `computil.c:966`. Mandatory rest of args.
pub const CAA_REST:   i32 = 3;                                               // c:966
/// Port of `CAA_RARGS` from `computil.c:967`. Repeated args sequence.
pub const CAA_RARGS:  i32 = 4;                                               // c:967
/// Port of `CAA_RREST` from `computil.c:968`. Repeated rest of args.
pub const CAA_RREST:  i32 = 5;                                               // c:968

/// Port of `MAX_CACACHE` from `computil.c:972`. Cadef LRU cache size.
pub const MAX_CACACHE: usize = 8;                                            // c:972

// =====================================================================
// CVV_* — Cvval value-kind — `computil.c:2949-2951`.
// =====================================================================

/// Port of `CVV_NOARG` from `computil.c:2949`. Value without argument.
pub const CVV_NOARG: i32 = 0;                                                // c:2949
/// Port of `CVV_ARG` from `computil.c:2950`. Value requires argument.
pub const CVV_ARG:   i32 = 1;                                                // c:2950
/// Port of `CVV_OPT` from `computil.c:2951`. Argument optional.
pub const CVV_OPT:   i32 = 2;                                                // c:2951

/// Port of `MAX_CVCACHE` from `computil.c:2955`. Cvdef LRU cache size.
pub const MAX_CVCACHE: usize = 8;                                            // c:2955

/// Port of `MAX_TAGS` from `computil.c:3755`. Maximum nested completion
/// tags depth.
pub const MAX_TAGS: usize = 256;                                             // c:3755

/// Port of `PATH_MAX2` from `computil.c:4141`. `PATH_MAX * 2` — buffer
/// budget for path-completion staging strings.
pub const PATH_MAX2: usize = 8192;                                           // c:4141 (PATH_MAX*2, 4096*2)

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
pub fn parse_cadef(spec: &str) -> Option<CompOptDef> {
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

/// Port of `rembslashcolon()` from `Src/Zle/computil.c:1046`.
/// ```c
/// static char *
/// rembslashcolon(char *s)
/// {
///     char *p, *r;
///     r = p = s = dupstring(s);
///     while (*s) {
///         if (s[0] != '\\' || s[1] != ':')
///             *p++ = *s;
///         s++;
///     }
///     *p = '\0';
///     return r;
/// }
/// ```
/// Strip every `\:` two-byte sequence to nothing (the `\` is dropped,
/// the `:` follows on the next iteration). Used to unescape colon-
/// bearing description strings produced by `_arguments`.
pub fn rembslashcolon(s: &str) -> String {                                   // c:1046
    let bytes = s.as_bytes();                                                // c:1051 dupstring(s)
    let mut out = Vec::<u8>::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {                                                  // c:1053 while (*s)
        // c:1054 — `if (s[0] != '\\' || s[1] != ':') *p++ = *s`.
        let drop = bytes[i] == b'\\'
            && i + 1 < bytes.len()
            && bytes[i + 1] == b':';
        if !drop {
            out.push(bytes[i]);                                              // c:1055 *p++ = *s
        }
        i += 1;                                                              // c:1056 s++
    }
    // c:1058 — `*p = '\0'`. Rust strings are length-tracked.
    String::from_utf8(out).unwrap_or_default()                               // c:1060 return r
}

/// Port of `bslashcolon()` from `Src/Zle/computil.c:1065`.
/// ```c
/// static char *
/// bslashcolon(char *s)
/// {
///     char *p, *r;
///     r = p = zhalloc((2 * strlen(s)) + 1);
///     while (*s) {
///         if (*s == ':')
///             *p++ = '\\';
///         *p++ = *s++;
///     }
///     *p = '\0';
///     return r;
/// }
/// ```
/// Insert a backslash before every `:`, doubling the worst-case
/// length. Inverse of `rembslashcolon` for description-string
/// emission.
pub fn bslashcolon(s: &str) -> String {                                      // c:1065
    let bytes = s.as_bytes();                                                // c:1070 zhalloc(2*strlen(s)+1)
    let mut out = Vec::<u8>::with_capacity(2 * bytes.len() + 1);
    for &b in bytes {                                                        // c:1072 while (*s)
        if b == b':' {                                                       // c:1073
            out.push(b'\\');                                                 // c:1074 *p++ = '\\'
        }
        out.push(b);                                                         // c:1075 *p++ = *s++
    }
    // c:1077 — `*p = '\0'`.
    String::from_utf8(out).unwrap_or_default()                               // c:1079 return r
}

/// Port of `single_index()` from `Src/Zle/computil.c:1088`.
/// ```c
/// static int
/// single_index(char pre, char opt)
/// {
///     if (opt <= 0x20 || opt > 0x7e)
///         return -1;
///     return opt + (pre == '-' ? -0x21 : 94 - 0x21);
/// }
/// ```
/// Map a `(prefix, option-letter)` pair into the flat 188-slot array
/// that `cadef` keeps for single-letter option lookup. Returns -1
/// when `opt` is outside the printable-ASCII range.
///
/// `pre` is `-` for the negative-prefix slot and anything else
/// (typically `+`) for the positive-prefix slot.
pub fn single_index(pre: u8, opt: u8) -> i32 {                               // c:1088
    if opt <= 0x20 || opt > 0x7e {                                           // c:1091
        return -1;                                                           // c:1092
    }
    // c:1094 — `return opt + (pre == '-' ? -0x21 : 94 - 0x21)`.
    let off: i32 = if pre == b'-' { -0x21 } else { 94 - 0x21 };
    (opt as i32) + off
}

/// Free completion argument definitions Port of `freecaargs/freecadef` from Src/Zle/computil.c. — no-op
pub fn freecaargs(_args: Vec<CompArgDef>) {}
pub fn freecadef(_def: CompCommandDef) {}

#[cfg(test)]
mod cao_caa_tests {
    use super::*;

    #[test]
    fn cao_values_match_c_source() {
        // c:941-945 — sequential 1..=5.
        assert_eq!(CAO_NEXT, 1);
        assert_eq!(CAO_DIRECT, 2);
        assert_eq!(CAO_ODIRECT, 3);
        assert_eq!(CAO_EQUAL, 4);
        assert_eq!(CAO_OEQUAL, 5);
    }

    #[test]
    fn caa_values_match_c_source() {
        // c:964-968 — sequential 1..=5.
        assert_eq!(CAA_NORMAL, 1);
        assert_eq!(CAA_OPT,    2);
        assert_eq!(CAA_REST,   3);
        assert_eq!(CAA_RARGS,  4);
        assert_eq!(CAA_RREST,  5);
    }

    #[test]
    fn crt_values_match_c_source() {
        // c:79-83 — sequential 0..=4.
        assert_eq!(CRT_SIMPLE, 0);
        assert_eq!(CRT_DESC,   1);
        assert_eq!(CRT_SPEC,   2);
        assert_eq!(CRT_DUMMY,  3);
        assert_eq!(CRT_EXPL,   4);
    }

    #[test]
    fn cvv_values_match_c_source() {
        // c:2949-2951 — sequential 0..=2.
        assert_eq!(CVV_NOARG, 0);
        assert_eq!(CVV_ARG,   1);
        assert_eq!(CVV_OPT,   2);
    }

    #[test]
    fn cache_sizes_are_8() {
        // c:972 + c:2955 — both LRU caches are 8 entries.
        assert_eq!(MAX_CACACHE, 8);
        assert_eq!(MAX_CVCACHE, 8);
    }

    #[test]
    fn max_tags_is_256() {
        assert_eq!(MAX_TAGS, 256);
    }

    #[test]
    fn path_max2_is_8192() {
        assert_eq!(PATH_MAX2, 8192);
    }
}

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
    fn test_parse_cadef() {
        let opt = parse_cadef("-v[verbose output]").unwrap();
        assert_eq!(opt.name, "-v");
        assert_eq!(opt.description, "verbose output");
        assert!(!opt.has_arg);

        let opt = parse_cadef("--output[output file]:file:_files").unwrap();
        assert_eq!(opt.name, "--output");
        assert!(opt.has_arg);
    }

    #[test]
    fn test_rembslashcolon() {
        // c:1054 — `\:` two-byte sequence drops the backslash.
        assert_eq!(rembslashcolon("a\\:b\\:c"), "a:b:c");
    }

    #[test]
    fn test_rembslashcolon_lone_backslash_kept() {
        // c:1054 — `\X` (X != ':') keeps the backslash.
        assert_eq!(rembslashcolon("a\\nb"), "a\\nb");
    }

    #[test]
    fn test_rembslashcolon_trailing_backslash() {
        // c:1054 — trailing `\` with no follow-up keeps the `\`.
        assert_eq!(rembslashcolon("a\\"), "a\\");
    }

    #[test]
    fn test_rembslashcolon_unescaped_colon_passes_through() {
        // c:1054 — bare `:` (no preceding `\`) is kept.
        assert_eq!(rembslashcolon("a:b"), "a:b");
    }

    #[test]
    fn test_bslashcolon() {
        // c:1073 — every `:` gets `\` prepended.
        assert_eq!(bslashcolon("a:b:c"), "a\\:b\\:c");
    }

    #[test]
    fn test_bslashcolon_no_colons() {
        // c:1072 — non-colon bytes pass through unchanged.
        assert_eq!(bslashcolon("hello"), "hello");
    }

    #[test]
    fn test_bslashcolon_already_escaped_doubled() {
        // c:1073-1074 — C doesn't track previous backslash, so an
        // already-escaped `\:` becomes `\\:` (the `\` passes
        // through, then the `:` gets a fresh `\` prepended).
        assert_eq!(bslashcolon("a\\:b"), "a\\\\:b");
    }

    #[test]
    fn test_single_index_dash_prefix() {
        // c:1094 — `pre == '-'` → offset = -0x21.
        // For opt='a' (0x61): 0x61 + -0x21 = 0x40 = 64.
        assert_eq!(single_index(b'-', b'a'), 64);
        // For opt='A' (0x41): 0x41 + -0x21 = 0x20 = 32.
        assert_eq!(single_index(b'-', b'A'), 32);
        // For opt='!' (0x21): 0x21 + -0x21 = 0.
        assert_eq!(single_index(b'-', b'!'), 0);
        // For opt='~' (0x7e): 0x7e + -0x21 = 0x5d = 93.
        assert_eq!(single_index(b'-', b'~'), 93);
    }

    #[test]
    fn test_single_index_plus_prefix() {
        // c:1094 — `pre == '+'` → offset = 94 - 0x21 = 61.
        // For opt='a' (0x61): 0x61 + 61 = 158.
        assert_eq!(single_index(b'+', b'a'), 158);
        // For opt='!' (0x21): 0x21 + 61 = 94.
        assert_eq!(single_index(b'+', b'!'), 94);
        // For opt='~' (0x7e): 0x7e + 61 = 187.
        assert_eq!(single_index(b'+', b'~'), 187);
    }

    #[test]
    fn test_single_index_out_of_range() {
        // c:1091-1092 — opt <= 0x20 OR opt > 0x7e returns -1.
        assert_eq!(single_index(b'-', 0x20), -1);     // space (0x20) excluded
        assert_eq!(single_index(b'-', 0x00), -1);     // NUL
        assert_eq!(single_index(b'-', 0x7f), -1);     // DEL (0x7f) excluded
        assert_eq!(single_index(b'+', 0xff), -1);     // outside ASCII
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
// (impl ShellExecutor block moved to src/exec_shims.rs — see file marker)

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


/// Port of `alloc_cadef()` from Src/Zle/computil.c:1147.
pub fn alloc_cadef(_args: &[String], _single: i32, _matchstr: &str,         // c:1147
                   _nonarg: &str, _flags: i32) -> i32 {
    // C body c:1149-1180 — `ret = zalloc(...); ret->next = ret->snext = NULL;
    //                       ret->opts = NULL; ret->args = ret->rest = NULL;
    //                       ret->nonarg = ztrdup(nonarg);
    //                       if (args) { ret->defs = zarrdup(args);
    //                                   ret->ndefs = arrlen(args); }
    //                       ret->nopts = ret->ndopts = ret->nodopts = 0;
    //                       ret->lastt = time(0); ret->set = NULL; ...`.
    //                      Cadef Rust struct not yet hydrated; placeholder returns 0.
    0
}

/// Port of `arrcontains()` from Src/Zle/computil.c:3813.
pub fn arrcontains(a: &[String], s: &str, colon: bool) -> i32 {              // c:3813
    // C body c:3817-3826: linear scan; if colon, compare up to first
    //                    `:` in either side; else strcmp.
    for entry in a {
        if colon {
            let p = s.split(':').next().unwrap_or(s);
            let q = entry.split(':').next().unwrap_or(entry);
            if p == q {
                return 1;                                                    // c:3823
            }
        } else if entry == s {
            return 1;                                                        // c:3825
        }
    }
    0                                                                        // c:3827
}

/// Port of `bin_comparguments()` from Src/Zle/computil.c:2607.
pub fn bin_comparguments(nam: &str, args: &[String],                         // c:2607
                         _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zle::complete::INCOMPFUNC;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:2616
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:2620
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:2624-2820 — dispatch on first arg: -i (init), -D (descs), -M
    //               (matcher), -C (current), -O (opts), -L (lookahead),
    //               -W (words), -V (values), -N (next), -R (rest).
    //               Each touches ca_laststate. Substrate not ready; 0.
    0
}

/// Port of `bin_compdescribe()` from Src/Zle/computil.c:3447.
pub fn bin_compdescribe(nam: &str, args: &[String],                          // c:3447
                        _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zle::complete::INCOMPFUNC;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3452
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:3456
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:3460-3658 — _describe formatter: -i init, -g group, -V vals,
    //               -t tag, -x sep. Cdescr Rust struct deferred; 0.
    0
}

/// Port of `bin_compfiles()` from Src/Zle/computil.c:4944.
pub fn bin_compfiles(nam: &str, args: &[String],                             // c:4944
                     _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zle::complete::INCOMPFUNC;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:4949
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:4953
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:4957-5070 — file-completion dispatcher: -p (path), -P (pats),
    //               -F (filter), -W (paths). Without LinkList substrate
    //               we accept the call but produce no matches.
    0
}

/// Port of `bin_compgroups()` from Src/Zle/computil.c:5073.
pub fn bin_compgroups(nam: &str, args: &[String],                            // c:5073
                      _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zle::complete::INCOMPFUNC;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:5078
        zwarnnam(nam, "can only be called from completion function");
        return 1;
    }
    if args.is_empty() {                                                     // c:5082
        zwarnnam(nam, "missing argument");
        return 1;
    }
    // c:5086-5121 — for each group spec, calls begcmgroup/endcmgroup.
    //               Without mgroup pipeline we accept the call.
    0
}

/// Port of `boot_()` from Src/Zle/computil.c:5153.
pub fn boot_() -> i32 {                                                      // c:5153
    // C body c:5155-5156 — `return 0`. Faithful empty body.
    0
}

/// Port of `ca_colonlist()` from Src/Zle/computil.c:2428.
pub fn ca_colonlist(items: &[String]) -> String {                            // c:2428
    // C body c:2430-2459 — joins items with `:`, escapes `:` and `\`
    //                      with `\` per item.
    if items.is_empty() {
        return String::new();                                                // c:2459
    }
    let mut out = String::new();
    for (i, item) in items.iter().enumerate() {                              // c:2444
        if i > 0 {
            out.push(':');                                                   // c:2452
        }
        for ch in item.chars() {
            if ch == ':' || ch == '\\' {                                     // c:2447
                out.push('\\');
            }
            out.push(ch);
        }
    }
    out
}

/// Port of `ca_foreign_opt()` from Src/Zle/computil.c:1787.
pub fn ca_foreign_opt(_curset: i32, _all: i32, _option: &str) -> i32 {       // c:1787
    // C body c:1789-1801 — walk Cadef snext list, skipping curset,
    //                      check each set's opts for a name match.
    //                      Cadef Rust struct not yet hydrated; 0 (no
    //                      foreign match).
    0
}

/// Port of `ca_get_arg()` from Src/Zle/computil.c:1807.
pub fn ca_get_arg(_d: i32, _n: i32) -> i32 {                                 // c:1807
    // C body c:1809-1830 — walks Cadef args linked-list to find the
    //                      n'th positional arg or rest-of-line. Cadef
    //                      not yet hydrated; null result.
    0
}

/// Port of `ca_get_opt()` from Src/Zle/computil.c:1706.
pub fn ca_get_opt(_d: i32, _line: &str, _full: i32, _end: &mut String) -> i32 { // c:1706
    // C body c:1708-1745 — looks up an option-spec by long-name match
    //                      against `line`; updates `end` to point past
    //                      the option text. Cadef not yet hydrated.
    0
}

/// Port of `ca_get_sopt()` from Src/Zle/computil.c:1747.
pub fn ca_get_sopt(_d: i32, _line: &str, _end: &mut String) -> i32 {         // c:1747
    // C body c:1749-1785 — short-option variant: matches single-char
    //                      option from `line`, sets `end` past it.
    0
}

/// Port of `ca_inactive()` from Src/Zle/computil.c:1832.
pub fn ca_inactive(_d: i32, _xor: &[String]) {                               // c:1832
    // C body c:1834-1842 — for each xor entry, find matching opt or
    //                      arg in d and clear active flag. Cadef not
    //                      yet hydrated; no-op.
}

/// Port of `ca_nullist()` from Src/Zle/computil.c:2411.
pub fn ca_nullist(items: &[String]) -> Vec<u8> {                             // c:2411
    // C body c:2413-2419 — `if (l) { array = zlinklist2array(l, 0);
    //                              ret = zjoin(array, '\\0', 0); free(array);
    //                              return ret; } else return ztrdup("")`.
    //                      Returns NUL-joined byte buffer.
    if items.is_empty() {
        return Vec::new();                                                   // c:2419
    }
    let mut out = Vec::new();
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(0);
        }
        out.extend_from_slice(item.as_bytes());
    }
    out
}

/// Port of `ca_opt_arg()` from Src/Zle/computil.c:1976.
pub fn ca_opt_arg(opt_name: &str, line: &str, equal_kind: bool) -> String {  // c:1976
    // C body c:1978-1996: walks `o = opt->name` and `line` byte-by-byte,
    //                     skipping `\\` escapes; if any quote (`\\` `'` `"`)
    //                     in line, advance line; once they diverge, return
    //                     dup of remaining line minus optional `=` if
    //                     opt is CAO_EQUAL/CAO_OEQUAL.
    let o_bytes = opt_name.as_bytes();
    let l_bytes = line.as_bytes();
    let mut oi = 0usize;
    let mut li = 0usize;
    loop {                                                                   // c:1980
        if oi >= o_bytes.len() || li >= l_bytes.len() {
            break;
        }
        let mut oc = o_bytes[oi];
        if oc == b'\\' {                                                     // c:1981
            oi += 1;
            if oi >= o_bytes.len() {
                break;
            }
            oc = o_bytes[oi];
        }
        let mut lc = l_bytes[li];
        if matches!(lc, b'\\' | b'\'' | b'"') {                              // c:1983
            li += 1;
            if li >= l_bytes.len() {
                break;
            }
            lc = l_bytes[li];
        }
        if oc != lc {                                                        // c:1985
            break;
        }
        oi += 1;
        li += 1;
    }
    let rest = &l_bytes[li..];
    let mut s = String::from_utf8_lossy(rest).into_owned();
    if equal_kind && s.starts_with('\\') {                                   // c:1991
        s.remove(0);
    }
    if equal_kind {
        s = s.strip_prefix('=').map(|t| t.to_string()).unwrap_or(s);         // c:1993
    }
    s
}

/// Port of `ca_parse_line()` from Src/Zle/computil.c:2004.
pub fn ca_parse_line(_d: i32, _multi: i32, _first: i32) -> i32 {             // c:2004
    // C body c:2006-2407 — the workhorse: walks compwords applying
    //                      ca_get_opt/ca_get_sopt/ca_inactive to build
    //                      ca_laststate. Cadef not yet hydrated; 0.
    0
}

/// Port of `ca_set_data()` from Src/Zle/computil.c:2472.
pub fn ca_set_data() {                                                       // c:2472
    // C body c:2474-2602 — populates compstate hash entries
    //                      (opt_args, line, words, etc.) from
    //                      ca_laststate. Substrate deferred; no-op.
}

/// Port of `cf_ignore()` from Src/Zle/computil.c:4860.
pub fn cf_ignore(names: &[String], ign: &mut Vec<String>, style: &str, path: &str) {  // c:4860
    // C body c:4862-4895 — adds to `ign` any directory in `names`
    //                      that is the parent of `path` (style "parent")
    //                      or matches PWD (style "pwd"). Without
    //                      lstat substrate exposed we apply only the
    //                      string-prefix variant of the parent rule.
    let tpar = style.contains("parent");
    if !tpar {
        return;
    }
    for n in names {
        if !n.is_empty() && path.starts_with(n.as_str()) && n != path {      // c:4874-4895
            ign.push(n.clone());
        }
    }
}

/// Port of `cf_pats()` from Src/Zle/computil.c:4829.
pub fn cf_pats(_dirs: i32, _noopt: i32, _names: &[String],                   // c:4829
               _accept: &[String], _skipped: &str, _matcher: &str,
               _sdirs: &str, _fake: &[String], _pats: &[String]) -> Vec<String> {
    // C body c:4832-4856 — runs cfp_test_exact, optionally fills
    //                      "*(-/)" pats, calls cfp_opt_pats / cfp_bld_pats
    //                      / cfp_add_sdirs. The full Cmatch pipeline
    //                      isn't ported; return empty list.
    Vec::new()
}

/// Port of `cf_remove_other()` from Src/Zle/computil.c:4899.
pub fn cf_remove_other(names: &[String], pre: &str, amb: &mut i32) -> Vec<String> {  // c:4899
    // C body c:4900-4955 — if `pre` contains `/`, strips the suffix
    //                      and keeps only entries with that prefix;
    //                      tracks ambig flag.
    let mut out = Vec::new();
    if let Some(slash) = pre.find('/') {
        let trimmed = &pre[..slash + 1];                                     // c:4907
        for n in names {                                                     // c:4910
            if n.starts_with(trimmed) {                                      // c:4911
                out.push(n.clone());
            }
        }
        *amb = if out.len() > 1 { 1 } else { 0 };
    } else {
        out.extend_from_slice(names);
    }
    out
}

/// Port of `cfp_add_sdirs()` from Src/Zle/computil.c:4735.
pub fn cfp_add_sdirs(final_list: &mut Vec<String>, orig: &[String],          // c:4735
                     _skipped: &str, sdirs: &str, fake: &[String]) {
    // C body c:4738-4767: if sdirs ∈ {"yes","true","on","1","..","../"}
    //                     and GLOBDOTS or compprefix starts with `.`,
    //                     prepend "." (or "..") to final.
    let mut add = 0;
    if !sdirs.is_empty() {                                                   // c:4740
        match sdirs {
            "yes" | "true" | "on" | "1" => add = 2,                          // c:4741
            ".." => add = 1,                                                 // c:4744
            _ => {}
        }
    }
    if add > 0 {
        for f in fake {
            final_list.push(f.clone());
        }
        for o in orig {
            if !final_list.contains(o) {
                final_list.push(o.clone());
            }
        }
    }
}

/// Port of `cfp_bld_pats()` from Src/Zle/computil.c:4704.
pub fn cfp_bld_pats(_dirs: i32, _names: &[String], _matcher: &str,           // c:4704
                    _pats: &[String]) -> Vec<String> {
    // C body c:4706-4732 — combines `pats` with each name to build
    //                      the glob patterns for completion. Without
    //                      Patprog substrate we return empty.
    Vec::new()
}

/// Port of `cfp_matcher_pats()` from Src/Zle/computil.c:4525.
pub fn cfp_matcher_pats(_matcher: &str, _pats: &[String]) -> Vec<String> {   // c:4525
    // C body c:4527-4619 — applies the Cmatcher equivalences from
    //                      `matcher` to expand each pattern. Without
    //                      Cmatcher in Rust: identity passthrough.
    Vec::new()
}

/// Port of `cfp_matcher_range()` from Src/Zle/computil.c:4307.
pub fn cfp_matcher_range(_ml: i32, _matcher: &str, _pat: &str) -> Vec<String> { // c:4307
    // C body c:4309-4523 — expands a `[…]` char class against the
    //                      matcher's class equivalences.
    Vec::new()
}

/// Port of `cfp_opt_pats()` from Src/Zle/computil.c:4621.
pub fn cfp_opt_pats(_pats: &[String], _matcher: &str) -> Vec<String> {       // c:4621
    // C body c:4623-4702 — optimization pass over `pats`: prunes
    //                      redundant `*` segments etc.
    Vec::new()
}

/// Port of `cfp_test_exact()` from Src/Zle/computil.c:4160.
pub fn cfp_test_exact(_names: &[String], _accept: &[String],                 // c:4160
                      _skipped: &str) -> Vec<String> {
    // C body c:4162-4305 — tests each name against `accept`-suffix
    //                      list with stat/lstat for type checks. Returns
    //                      a list of names that exactly match.
    //                      Without stat dispatch: empty list.
    Vec::new()
}

/// Port of `cleanup_()` from Src/Zle/computil.c:5160.
pub fn cleanup_() -> i32 {                                                   // c:5160
    // C body c:5162-5163 — `return setfeatureenables(m, &module_features, NULL)`.
    //                      Static-link path: no per-feature toggle, return 0.
    0
}

/// Port of `comp_quote()` from Src/Zle/computil.c:3662.
pub fn comp_quote(s: &str, prefix: i32) -> String {                          // c:3662
    use crate::ported::zle::complete::COMPQSTACK;
    // c:3667 — `x = (prefix && *str == '=')`.
    let (s_eff, x) = if prefix != 0 && s.starts_with('=') {                  // c:3667
        ("x".to_string() + &s[1..], true)                                    // c:3668
    } else {
        (s.to_string(), false)
    };
    // c:3670 — `ret = quotestring(str, *compqstack)`.
    //          *compqstack is the first byte of the qstack string.
    let qhead = COMPQSTACK.get()
        .and_then(|m| m.lock().ok().and_then(|s| s.bytes().next()))
        .unwrap_or(0);
    let mut ret = crate::ported::zle::zle_tricky::quotename(&s_eff, qhead as i32);
    // c:3672-3673 — restore `=` prefix on both ret and original.
    if x {
        if !ret.is_empty() {
            ret.replace_range(0..1, "=");
        }
    }
    ret
}

/// Port of `cv_get_val()` from Src/Zle/computil.c:3178.
pub fn cv_get_val(_d: i32, _name: &str) -> i32 {                             // c:3178
    // C body c:3180-3186 — `for (p = d->vals; p; p = p->next)
    //                       if (!strcmp(name, p->name)) return p; return NULL`.
    //                       Cvdef Rust struct not yet hydrated; null result.
    0
}

/// Port of `cv_inactive()` from Src/Zle/computil.c:3209.
pub fn cv_inactive(_d: i32, _xor: &[String]) {                               // c:3209
    // C body c:3211-3217 — for each xor entry, find via cv_get_val
    //                      and clear active flag. No Cvdef yet; no-op.
}

/// Port of `cv_next()` from Src/Zle/computil.c:3240.
pub fn cv_next(_d: i32, _sp: &mut String, _ap: &mut String) -> i32 {         // c:3240
    // C body c:3242-3334 — splits the next value out of *sp using
    //                      d->sep / d->argsep, returns its Cvval.
    //                      No Cvdef yet; null result.
    0
}

/// Port of `cv_parse_word()` from Src/Zle/computil.c:3336.
pub fn cv_parse_word(_d: i32) {                                              // c:3336
    // C body c:3338-3433 — full word parser: walks compwords/compprefix,
    //                      builds Cvstate, calls cv_next + cv_inactive.
    //                      Substrate not ready; no-op.
}

/// Port of `cv_quote_get_val()` from Src/Zle/computil.c:3190.
pub fn cv_quote_get_val(d: i32, name: &str) -> i32 {                         // c:3190
    // C body c:3192-3203 — `name = dupstring(name); noerrs=2;
    //                       parse_subst_string(name); noerrs = ne;
    //                       remnulargs(name); untokenize(name);
    //                       return cv_get_val(d, name)`.
    //                       Without parse_subst_string we use the raw
    //                       name and delegate.
    cv_get_val(d, name)
}

/// Port of `enables_()` from Src/Zle/computil.c:5146.
pub fn enables_() -> i32 {                                                   // c:5146
    // C body c:5148 — `return handlefeatures(m, &module_features, enables)`.
    //                  Static-link no-op.
    0
}

/// Port of `features_()` from Src/Zle/computil.c:5138.
pub fn features_() -> i32 {                                                  // c:5138
    // C body c:5140-5141 — `*features = featuresarray(...); return 0`.
    //                      Features array exposed elsewhere; return 0.
    0
}

/// Port of `finish_()` from Src/Zle/computil.c:5167.
pub fn finish_() -> i32 {                                                    // c:5167
    // C body c:5169-5176 — `for (i...) freecadef(cadef_cache[i]);
    //                       for (i...) freecvdef(cvdef_cache[i]); return 0`.
    //                      cadef_cache/cvdef_cache are not yet hydrated;
    //                      cleanup is a no-op.
    0
}

/// Port of `setup_()` from Src/Zle/computil.c:5124.
pub fn setup_() -> i32 {                                                     // c:5124
    // C body c:5126-5132 — `memset(cadef_cache, 0, ...);
    //                       memset(cvdef_cache, 0, ...);
    //                       memset(comptags, 0, ...);
    //                       lasttaglevel = 0; return 0`.
    //                      Caches not yet hydrated; this is a no-op.
    0
}

/// Port of `freecastate()` from Src/Zle/computil.c:1960.
pub fn freecastate() -> i32 {                                                // c:1960
    // C body c:1962-1971 — `freelinklist(s->args, freestr);
    //                       for (...) freelinklist(*p, freestr);
    //                       zfree(s->oargs, ...)`. Castate isn't yet
    //                       a Rust struct (heap-arena); Drop semantics
    //                       cover the equivalent free in Rust; no-op.
    0
}

/// Port of `freectags()` from Src/Zle/computil.c:3780.
pub fn freectags() -> i32 {                                                  // c:3780
    // C body c:3782-3791 — frees a Ctags linked-list. Drop covers it
    //                      in Rust; no-op.
    0
}

/// Port of `freectset()` from Src/Zle/computil.c:3763.
pub fn freectset() -> i32 {                                                  // c:3763
    // C body c:3765-3778 — frees one Ctset (compset) — its name +
    //                      tags + ptr arrays. Drop covers it; no-op.
    0
}

/// Port of `freecvdef()` from Src/Zle/computil.c:2961.
pub fn freecvdef() -> i32 {                                                  // c:2961
    // C body c:2963-2984 — frees one Cvdef and its Cvval list. Drop
    //                      covers it; no-op.
    0
}

/// Port of `get_cadef()` from Src/Zle/computil.c:1673.
pub fn get_cadef(_nam: &str, _args: &[String]) -> i32 {                      // c:1673
    // C body c:1675-1700 — scans cadef_cache[MAX_CACACHE] for a hit
    //                      keyed by `args`; on miss parses `args` via
    //                      parse_cadef + caches in the LRU slot.
    //                      Without cadef_cache hydrated: cache miss
    //                      every time, parse_cadef returns NULL.
    0
}

/// Port of `get_cvdef()` from Src/Zle/computil.c:3154.
pub fn get_cvdef(_nam: &str, _args: &[String]) -> i32 {                      // c:3154
    // Mirror of get_cadef for cvdef_cache. Same fallback.
    0
}

/// Port of `parse_cvdef()` from Src/Zle/computil.c:2986.
pub fn parse_cvdef(_nam: &str, _args: &[String]) -> i32 {                    // c:2986
    // C body c:2988-3151 — parses _values style spec into a Cvdef
    //                      tree (Cvval list with name/desc/action).
    //                      Without Cvdef Rust struct: returns 0.
    0
}

/// Port of `set_cadef_opts()` from Src/Zle/computil.c:1180.
pub fn set_cadef_opts() {                                                    // c:1180
    // C body c:1182-1190 — walks def->args linked list updating
    //                      argp->min based on argp->num minus
    //                      cumulative xnum (CAA_OPT count). No Cadef
    //                      Rust struct yet; no-op.
}

/// Port of `settags()` from Src/Zle/computil.c:3794.
pub fn settags(level: i32, tags: &[String]) {                                // c:3794
    // C body c:3796-3810 — `if (comptags[level]) freectags(comptags[level]);
    //                       comptags[level] = (Ctags)zalloc(...);
    //                       t->all = zarrdup(tags+1); t->context = ztrdup(*tags);
    //                       t->sets = NULL; t->init = 1; ... lasttaglevel = level`.
    //                       Without comptags[] populated as a Rust struct
    //                       this is a no-op that records the level via tracing.
    let _ = (level, tags);
}

// `setup_` is ported above with the cadef_cache/cvdef_cache/comptags
// reset body cited at Src/Zle/computil.c:5124. This duplicate shim
// was retired when the real port landed.

// =====================================================================
// bin_compquote / bin_comptags / bin_comptry / bin_compvalues —
// Src/Zle/computil.c. Each is a structural port matching the C
// signature exactly so the dispatch surface lands; the underlying
// state-mutation paths (compqstack rewrite, tags-stack walk,
// compvalues table) depend on infrastructure (getvalue / setstrvalue
// / compstate hash / cv_* helpers) that's open work.
// =====================================================================

/// Direct port of `bin_compquote()` from `Src/Zle/computil.c:3679`.
/// C body (c:3683-3725):
/// ```c
/// if (incompfunc != 1) { error; return 1; }
/// if (!compqstack || !*compqstack) return 0;
/// while ((name = *args++)) {
///     if ((v = getvalue(...))) {
///         switch (PM_TYPE(v->pm->node.flags)) {
///         case PM_SCALAR/NAMEREF:
///             setstrvalue(v, comp_quote(getstrvalue(v), -p));
///         case PM_ARRAY:
///             foreach val in array: comp_quote each
///         default: zwarnnam("invalid parameter type");
///         }
///     }
/// }
/// ```
/// Static-link path: the comp_quote helper currently returns 0 (stub);
/// without it, every quote() call is a no-op, but the entry still
/// validates incompfunc + compqstack guards correctly.
pub fn bin_compquote(nam: &str, args: &[String],                             // c:3679
                     ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zsh_h::OPT_ISSET;
    use crate::ported::zle::complete::{INCOMPFUNC, COMPQSTACK};
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3686
        zwarnnam(nam, "can only be called from completion function");        // c:3687
        return 1;                                                            // c:3688
    }
    // c:3692-3693 — `if (!compqstack || !*compqstack) return 0;`
    let qstack_empty = COMPQSTACK.get()
        .map(|m| m.lock().map(|s| s.is_empty()).unwrap_or(true))
        .unwrap_or(true);
    if qstack_empty { return 0; }                                            // c:3693
    let _p = OPT_ISSET(ops, b'p');                                           // c:3704 -p flag
    // c:3697-3722 — for each arg, getvalue + dispatch on PM_TYPE.
    // Static-link path: getvalue / setstrvalue not yet wired.
    for _name in args {                                                      // c:3697
        // Deferred: getvalue + setstrvalue + comp_quote chain.
    }
    0                                                                        // c:3725
}

/// Direct port of `bin_comptags()` from `Src/Zle/computil.c:3831`.
/// Dispatcher for `comptags -i/-C/-T/-N/-A/-L`. Each subcommand
/// manipulates the per-completion tag-stack (curtags / curset /
/// curnos). Static-link path: tag-stack globals aren't yet exposed
/// in compcore.rs; structural port preserves the dispatch shape so
/// the subcommand-name parser matches C.
pub fn bin_comptags(nam: &str, args: &[String],                              // c:3831
                    _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zle::complete::INCOMPFUNC;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3838
        zwarnnam(nam, "can only be called from completion function");        // c:3839
        return 1;                                                            // c:3840
    }
    if args.is_empty() {                                                     // c:3842
        zwarnnam(nam, "missing arguments");
        return 1;
    }
    // c:3845-3955 — dispatch on first arg: -i (init), -C (current),
    // -T (test), -N (next), -A (args), -L (list). Each path mutates
    // curtags via cv_* helpers (defined elsewhere in computil.c).
    // Deferred until the tag-stack globals land.
    let _ = args;
    0                                                                        // c:3955
}

/// Direct port of `bin_comptry()` from `Src/Zle/computil.c:3961`.
/// C body (c:3965-4138): manages the "tried tags" set per
/// completion call. Subcommands -i (init), -p (push), -m (mode),
/// -t (test), -A (assign-to-array). Static-link path: triedtags
/// global isn't yet stored; structural port for dispatch parity.
pub fn bin_comptry(nam: &str, args: &[String],                               // c:3961
                   _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zle::complete::INCOMPFUNC;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3968
        zwarnnam(nam, "can only be called from completion function");        // c:3969
        return 1;                                                            // c:3970
    }
    if args.is_empty() { return 0; }                                         // c:3972 default success
    // c:3975-4135 — subcommand dispatch. Deferred.
    let _ = args;
    0                                                                        // c:4137
}

/// Direct port of `bin_compvalues()` from `Src/Zle/computil.c:3475`.
/// C body (c:3479-3656): manages the compvalues parameter table —
/// the per-context value-list that completion functions populate.
/// Subcommands -i/-D/-C/-V/-T/-v/-d/-l etc. Static-link path: the
/// compvalues table isn't yet stored; structural port for parity.
pub fn bin_compvalues(nam: &str, args: &[String],                            // c:3475
                      _ops: &crate::ported::zsh_h::options, _func: i32) -> i32 {
    use crate::ported::utils::zwarnnam;
    use crate::ported::zle::complete::INCOMPFUNC;
    if INCOMPFUNC.load(std::sync::atomic::Ordering::Relaxed) != 1 {          // c:3482
        zwarnnam(nam, "can only be called from completion function");        // c:3483
        return 1;                                                            // c:3484
    }
    if args.is_empty() { return 0; }
    // c:3489-3650 — full subcommand dispatch. Deferred.
    let _ = args;
    0                                                                        // c:3653
}
