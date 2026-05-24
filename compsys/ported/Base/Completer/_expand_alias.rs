//! Port of `_expand_alias` — expand aliases.
//!
//! Local shell reference: `compsys/functions/Base/Completer/_expand_alias`
//! (system copy `/opt/homebrew/share/zsh/functions/_expand_alias`).
//!
//! Upstream shell source (50-line dispatch, key lines):
//! ```text
//! 24  zstyle -s ":completion:${curcontext}:" regular tmp || tmp=yes
//! 25  case $tmp in
//! 26    always)            sel=r;;
//! 27    yes|1|true|on)     [[ CURRENT -eq 1 ]] && sel=r;;
//! 29  zstyle -T ":completion:${curcontext}:" global && sel="g$sel"
//! 30  zstyle -t ":completion:${curcontext}:" disabled && sel="${sel}${(U)sel}"
//! 33  [[ $sel = *r* ]] && tmp=$aliases[$word]
//! 34  [[ -z $tmp && $sel = *g* ]] && tmp=$galiases[$word]
//! 35  [[ -z $tmp && $sel = *R* ]] && tmp=$dis_aliases[$word]
//! 36  [[ -z $tmp && $sel = *G* ]] && tmp=$dis_galiases[$word]
//! 41  if [[ $tmp[1] = [[:alnum:]_] ]]; then
//! 42    tmpa=(${(z)tmp})
//! 43    if [[ $tmpa[1] = $word && $tmp = $aliases[$word] ]]; then
//! 44      tmp="\\$tmp"   # quote to prevent re-expansion
//! 49  zstyle -T … add-space || suf=( -S '' )
//! 50  $pre _wanted aliases expl alias compadd -UQ "$suf[@]" -- $tmp
//! ```
//!
//! Strict Rust port: four kinds of alias tables (regular / global /
//! disabled regular / disabled global) keyed by the assembled
//! `IPREFIX+PREFIX+SUFFIX[+ISUFFIX]` word. Selector is built per
//! shell:24-30, then the four tables are queried in order. If the
//! resolved expansion starts with the SAME word the user typed and
//! came from the regular alias table, prepend `\\` to prevent
//! re-expansion (shell:43-46).

use std::collections::HashMap;

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// Four alias tables zsh exposes. Caller (shell bridge) populates
/// these from `aliases`, `galiases`, `dis_aliases`, `dis_galiases`.
pub struct AliasTables<'a> {
    pub regular: &'a HashMap<String, String>,
    pub global: &'a HashMap<String, String>,
    pub disabled_regular: &'a HashMap<String, String>,
    pub disabled_global: &'a HashMap<String, String>,
}

/// Options controlling `_expand_alias` behavior. Mirrors the
/// `regular` / `global` / `disabled` / `add-space` zstyles.
pub struct ExpandAliasOpts<'a> {
    /// `regular` zstyle: `always` / `yes,1,true,on` / `no,0,false,off`.
    /// Default `yes`.
    pub regular: &'a str,
    /// True iff `global` zstyle resolves to a truthy value
    /// (`zstyle -T`).
    pub global_enabled: bool,
    /// True iff `disabled` zstyle is truthy.
    pub disabled_enabled: bool,
    /// True iff user is at $CURRENT == 1 (first word in command line).
    /// Drives the `yes` branch of `regular`.
    pub at_first_word: bool,
    /// True iff `add-space` zstyle resolves truthy. When false we
    /// arm NOSPACE (`-S ''`) on the emitted match.
    pub add_space: bool,
    /// `_prefix`-parent funcstack mode — when true, the word
    /// excludes `$ISUFFIX` (shell:13).
    pub prefix_mode: bool,
}

impl<'a> ExpandAliasOpts<'a> {
    /// Defaults reflecting the upstream `|| tmp=yes` fallback
    /// (regular=yes, global enabled, no disabled, not in prefix mode).
    pub const fn defaults() -> Self {
        Self {
            regular: "yes",
            global_enabled: true,
            disabled_enabled: false,
            at_first_word: true,
            add_space: false,
            prefix_mode: false,
        }
    }
}

/// _expand_alias - Expand aliases.
pub fn _expand_alias(
    state: &mut CompletionState,
    tables: &AliasTables<'_>,
    opts: &ExpandAliasOpts<'_>,
) -> bool {
    // shell:13-16 — assemble the lookup word.
    let word = if opts.prefix_mode {
        format!(
            "{}{}{}",
            state.params.iprefix, state.params.prefix, state.params.suffix
        )
    } else {
        format!(
            "{}{}{}{}",
            state.params.iprefix,
            state.params.prefix,
            state.params.suffix,
            state.params.isuffix
        )
    };

    // shell:24-30 — build the `sel` selector string.
    let mut sel = String::new();
    match opts.regular {
        "always" => sel.push('r'),
        "yes" | "1" | "true" | "on" if opts.at_first_word => sel.push('r'),
        _ => {}
    }
    if opts.global_enabled {
        let g = format!("g{}", sel);
        sel = g;
    }
    if opts.disabled_enabled {
        let upper = sel.to_uppercase();
        sel.push_str(&upper);
    }

    // shell:33-36 — table lookup in priority order.
    let mut tmp: Option<String> = None;
    let mut hit_regular = false;
    if sel.contains('r') {
        if let Some(v) = tables.regular.get(&word).cloned() {
            tmp = Some(v);
            hit_regular = true;
        }
    }
    if tmp.is_none() && sel.contains('g') {
        if let Some(v) = tables.global.get(&word).cloned() {
            tmp = Some(v);
        }
    }
    if tmp.is_none() && sel.contains('R') {
        if let Some(v) = tables.disabled_regular.get(&word).cloned() {
            tmp = Some(v);
        }
    }
    if tmp.is_none() && sel.contains('G') {
        if let Some(v) = tables.disabled_global.get(&word).cloned() {
            tmp = Some(v);
        }
    }

    let mut tmp = match tmp {
        Some(s) => s,
        None => return false,
    };

    // shell:38 — strip trailing blanks. The `%%[[:blank:]]##` shell
    // form drops a run of spaces/tabs from the end.
    while tmp.ends_with(|c: char| c == ' ' || c == '\t') {
        tmp.pop();
    }

    // shell:41-46 — if the expansion's first word equals the typed
    // word AND it came from the regular table, escape with `\` to
    // prevent re-expansion at execution.
    if hit_regular {
        let first_token = tmp.split_whitespace().next().unwrap_or("");
        if first_token == word {
            tmp.insert(0, '\\');
        }
    }

    // shell:50 — emit with -U (no munging) -Q (no quoting). NOSPACE
    // iff add-space was disabled.
    let mut comp = Completion::new(&tmp);
    if !opts.add_space {
        comp.flags |= CompletionFlags::NOSPACE;
    }
    state.add_match(comp, Some("aliases"));
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_table() -> HashMap<String, String> {
        HashMap::new()
    }

    fn tables_with_regular(t: &HashMap<String, String>) -> AliasTables<'_> {
        // Empty references for the unused tables to satisfy borrow
        // checker via separate empty maps.
        AliasTables {
            regular: t,
            global: Box::leak(Box::new(empty_table())),
            disabled_regular: Box::leak(Box::new(empty_table())),
            disabled_global: Box::leak(Box::new(empty_table())),
        }
    }

    #[test]
    fn known_regular_alias_emits_with_nospace() {
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let mut reg = HashMap::new();
        reg.insert("ll".into(), "ls -la".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts::defaults();
        assert!(_expand_alias(&mut state, &tables, &opts));
        let m = &state.groups[0].matches[0];
        assert_eq!(m.str_, "ls -la");
        assert!(m.flags.contains(CompletionFlags::NOSPACE));
    }

    #[test]
    fn unknown_word_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "unknown".into();
        let reg = empty_table();
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts::defaults();
        assert!(!_expand_alias(&mut state, &tables, &opts));
    }

    #[test]
    fn current_word_is_iprefix_plus_prefix_plus_suffix_plus_isuffix() {
        let mut state = CompletionState::new();
        state.params.iprefix = "git ".into();
        state.params.prefix = "co".into();
        state.params.suffix = "m".into();
        state.params.isuffix = " HEAD".into();
        let mut reg = HashMap::new();
        reg.insert("git com HEAD".into(), "git commit HEAD".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts::defaults();
        assert!(_expand_alias(&mut state, &tables, &opts));
    }

    #[test]
    fn prefix_mode_drops_isuffix_from_lookup() {
        // shell:13 — `_prefix` funcstack parent → exclude $ISUFFIX.
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        state.params.isuffix = "_garbage".into();
        let mut reg = HashMap::new();
        reg.insert("ll".into(), "ls -la".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts {
            prefix_mode: true,
            ..ExpandAliasOpts::defaults()
        };
        assert!(_expand_alias(&mut state, &tables, &opts));
    }

    #[test]
    fn regular_no_at_non_first_word_skips_regular_table() {
        // `regular=yes` requires CURRENT==1. Test at_first_word=false.
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let mut reg = HashMap::new();
        reg.insert("ll".into(), "ls -la".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts {
            at_first_word: false,
            ..ExpandAliasOpts::defaults()
        };
        assert!(!_expand_alias(&mut state, &tables, &opts));
    }

    #[test]
    fn regular_always_works_even_past_first_word() {
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let mut reg = HashMap::new();
        reg.insert("ll".into(), "ls -la".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts {
            regular: "always",
            at_first_word: false,
            ..ExpandAliasOpts::defaults()
        };
        assert!(_expand_alias(&mut state, &tables, &opts));
    }

    #[test]
    fn global_table_consulted_when_regular_misses() {
        let mut state = CompletionState::new();
        state.params.prefix = "L".into();
        let reg = empty_table();
        let mut global = HashMap::new();
        global.insert("L".into(), "| less".into());
        let tables = AliasTables {
            regular: &reg,
            global: &global,
            disabled_regular: &empty_table(),
            disabled_global: &empty_table(),
        };
        let opts = ExpandAliasOpts::defaults();
        assert!(_expand_alias(&mut state, &tables, &opts));
        assert_eq!(state.groups[0].matches[0].str_, "| less");
    }

    #[test]
    fn disabled_tables_only_consulted_when_disabled_enabled() {
        let mut state = CompletionState::new();
        state.params.prefix = "off".into();
        let reg = empty_table();
        let global = empty_table();
        let mut dis_reg = HashMap::new();
        dis_reg.insert("off".into(), "echo disabled".into());
        let tables = AliasTables {
            regular: &reg,
            global: &global,
            disabled_regular: &dis_reg,
            disabled_global: &empty_table(),
        };
        // Without disabled_enabled, the R/G selector bits aren't set.
        let opts_off = ExpandAliasOpts::defaults();
        assert!(!_expand_alias(&mut state, &tables, &opts_off));

        let mut state = CompletionState::new();
        state.params.prefix = "off".into();
        let opts_on = ExpandAliasOpts {
            disabled_enabled: true,
            ..ExpandAliasOpts::defaults()
        };
        assert!(_expand_alias(&mut state, &tables, &opts_on));
    }

    #[test]
    fn self_referential_regular_alias_escaped_with_backslash() {
        // shell:43-46 — if expansion's first word == the typed word
        // AND we used the regular table, prepend `\\`.
        let mut state = CompletionState::new();
        state.params.prefix = "git".into();
        let mut reg = HashMap::new();
        reg.insert("git".into(), "git extra".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts::defaults();
        assert!(_expand_alias(&mut state, &tables, &opts));
        assert_eq!(state.groups[0].matches[0].str_, "\\git extra");
    }

    #[test]
    fn trailing_whitespace_stripped_from_expansion() {
        // shell:38 — `${tmp%%[[:blank:]]##}` strips trailing blanks.
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let mut reg = HashMap::new();
        reg.insert("ll".into(), "ls -la   \t".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts::defaults();
        assert!(_expand_alias(&mut state, &tables, &opts));
        assert_eq!(state.groups[0].matches[0].str_, "ls -la");
    }

    #[test]
    fn add_space_disables_nospace_flag() {
        let mut state = CompletionState::new();
        state.params.prefix = "ll".into();
        let mut reg = HashMap::new();
        reg.insert("ll".into(), "ls -la".into());
        let tables = tables_with_regular(&reg);
        let opts = ExpandAliasOpts {
            add_space: true,
            ..ExpandAliasOpts::defaults()
        };
        assert!(_expand_alias(&mut state, &tables, &opts));
        assert!(!state.groups[0].matches[0].flags.contains(CompletionFlags::NOSPACE));
    }
}
