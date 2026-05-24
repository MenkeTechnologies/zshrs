//! Port of `_command_names` (zsh Completion/Base/Utility/_command_names).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_command_names`.
//!
//! Completes everything valid at command position. Faithful re-port of
//! the 68-line shell function — each tag category gets its own group
//! so `zstyle ':completion:*:command:aliases' …` works:
//!
//! ```text
//! commands       — external executables found on $path
//! builtins       — names from inv.builtins
//! functions      — entries in inv.functions (with prefix-needed filter)
//! aliases        — entries in inv.aliases
//! suffix-aliases — entries in inv.suffix_aliases
//! reserved-words — entries in inv.reserved_words
//! jobs           — entries in inv.jobs (TODO: jobtab wiring upstream)
//! parameters     — entries in inv.parameters (with `=` suffix)
//! ```
//!
//! Honored zstyles (shell source line refs):
//!   `:commands rehash` (l.9)         — silently true; the caller
//!                                       owns the cmdnam cache.
//!   `:functions prefix-needed` (l.11) — when PREFIX is not `[_.]*`,
//!                                       filter out `_*` / `.*` ported.
//!   `command-path` (l.49)             — if set, use these dirs instead
//!                                       of $path for external scan.

use crate::compcore::CompletionState;
use crate::completion::Completion;
use std::path::Path;

use super::shared::is_executable;

/// Inventory of shell-table entries passed into [`command_names`]. The
/// compsys crate is a leaf — it can't reach into the parent zshrs
/// `aliastab` / `shfunctab` / `reswdtab` / `paramtab` etc. directly,
/// so the caller (LSP, ZLE completion, or test harness) populates
/// these slices from `crate::ported::hashtable::*_lock()` reads.
///
/// Caller-side population template (zshrs caller code):
/// ```ignore
/// let aliases: Vec<String> = aliastab_lock().read().unwrap()
///     .iter().map(|(k, _)| k.clone()).collect();
/// // ... same for the other six tables ...
/// let inv = ShellInventory { aliases: &aliases, ..Default::default() };
/// compsys::ported::_command_names(state, &inv, false);
/// ```
#[derive(Default)]
pub struct ShellInventory<'a> {
    pub builtins: &'a [String],
    pub functions: &'a [String],
    pub aliases: &'a [String],
    pub suffix_aliases: &'a [String],
    pub reserved_words: &'a [String],
    pub parameters: &'a [String],
    pub jobs: &'a [String],
}

/// Entry point — uses the default `::command` curcontext class.
pub fn _command_names(
    state: &mut CompletionState,
    inv: &ShellInventory<'_>,
    externals_only: bool,
) -> bool {
    _command_names_with_ctx(state, inv, "::command", externals_only)
}

/// `command_names` taking an explicit `curcontext` (everything after
/// `:completion:`). Use this when a caller already has the
/// `MainCompleteState.ctx.context` available — `:completion:$ctx:foo`
/// style lookups need it.
pub fn _command_names_with_ctx(
    state: &mut CompletionState,
    inv: &ShellInventory<'_>,
    curcontext: &str,
    externals_only: bool,
) -> bool {
    let prefix = state.params.prefix.clone();

    // Honor `:functions prefix-needed` (shell l.11-13). Default off.
    let prefix_needed = state
        .styles
        .lookup_bool(
            &format!(":completion:{}:functions", curcontext),
            "prefix-needed",
        )
        .unwrap_or(false);
    let fn_starts_internal = prefix.starts_with('_') || prefix.starts_with('.');
    let skip_internal_fns = prefix_needed && !fn_starts_internal;

    // ── Tag: commands (external executables) ──────────────────────────
    let cmd_path: Vec<String> = state
        .styles
        .lookup_values(&format!(":completion:{}", curcontext), "command-path")
        .map(|v| v.to_vec())
        .unwrap_or_else(|| {
            std::env::var("PATH")
                .ok()
                .map(|p| p.split(':').map(String::from).collect())
                .unwrap_or_default()
        });

    state.begin_group("commands", true);
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for dir in &cmd_path {
        let dpath = Path::new(dir);
        if let Ok(entries) = std::fs::read_dir(dpath) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy().to_string();
                if !name_str.starts_with(&prefix) {
                    continue;
                }
                if !is_executable(&entry.path()) {
                    continue;
                }
                if seen.insert(name_str.clone()) {
                    state.add_match(Completion::new(name_str), Some("commands"));
                }
            }
        }
    }
    state.end_group();

    if externals_only {
        return state.nmatches > 0;
    }

    // ── Tag: builtins ─────────────────────────────────────────────────
    state.begin_group("builtins", true);
    for name in inv.builtins {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("builtins"));
        }
    }
    state.end_group();

    // ── Tag: functions ────────────────────────────────────────────────
    state.begin_group("functions", true);
    for name in inv.functions {
        if !name.starts_with(&prefix) {
            continue;
        }
        if skip_internal_fns && (name.starts_with('_') || name.starts_with('.')) {
            continue;
        }
        state.add_match(Completion::new(name.clone()), Some("functions"));
    }
    state.end_group();

    // ── Tag: aliases ──────────────────────────────────────────────────
    state.begin_group("aliases", true);
    for name in inv.aliases {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("aliases"));
        }
    }
    state.end_group();

    // ── Tag: suffix-aliases ───────────────────────────────────────────
    state.begin_group("suffix-aliases", true);
    for name in inv.suffix_aliases {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("suffix-aliases"));
        }
    }
    state.end_group();

    // ── Tag: reserved-words ───────────────────────────────────────────
    state.begin_group("reserved-words", true);
    for name in inv.reserved_words {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("reserved-words"));
        }
    }
    state.end_group();

    // ── Tag: jobs ─────────────────────────────────────────────────────
    state.begin_group("jobs", true);
    for name in inv.jobs {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("jobs"));
        }
    }
    state.end_group();

    // ── Tag: parameters ───────────────────────────────────────────────
    state.begin_group("parameters", true);
    for name in inv.parameters {
        if name.starts_with(&prefix) {
            let mut c = Completion::new(name.clone());
            c.suf = Some("=".into());
            state.add_match(c, Some("parameters"));
        }
    }
    state.end_group();

    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn _inv<'a>(
        builtins: &'a [String],
        aliases: &'a [String],
        functions: &'a [String],
        reswords: &'a [String],
    ) -> ShellInventory<'a> {
        ShellInventory {
            builtins,
            aliases,
            functions,
            reserved_words: reswords,
            ..Default::default()
        }
    }

    #[test]
    fn emits_all_eight_tag_groups() {
        let mut state = CompletionState::new();
        state.params.prefix = "t".into();
        let builtins = vec!["true".into(), "test".into()];
        let aliases = vec!["tlf".into()];
        let functions = vec!["tlsdir".into(), "_tabby".into()];
        let reswords = vec!["then".into(), "time".into()];
        let inv = _inv(&builtins, &aliases, &functions, &reswords);
        _command_names(&mut state, &inv, false);

        let group_names: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        for must in [
            "commands",
            "builtins",
            "functions",
            "aliases",
            "suffix-aliases",
            "reserved-words",
            "jobs",
            "parameters",
        ] {
            assert!(
                group_names.contains(&must),
                "missing tag group `{must}` (got {group_names:?})",
            );
        }
    }

    #[test]
    fn externals_only_skips_internal_categories() {
        let mut state = CompletionState::new();
        state.params.prefix = "t".into();
        let builtins = vec!["true".into()];
        let inv = _inv(&builtins, &[], &[], &[]);
        _command_names(&mut state, &inv, true);
        let group_names: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(group_names, vec!["commands"]);
    }

    #[test]
    fn prefix_needed_filters_underscore_fns() {
        let mut state = CompletionState::new();
        state.params.prefix = "g".into();
        state.styles.set(
            ":completion:::command:functions",
            "prefix-needed",
            vec!["true".into()],
            false,
        );
        let functions = vec!["git_helper".into(), "_grep".into(), "greet".into()];
        let inv = _inv(&[], &[], &functions, &[]);
        _command_names(&mut state, &inv, false);

        let fn_group = state
            .groups
            .iter()
            .find(|g| g.name == "functions")
            .expect("functions group present");
        let names: Vec<&str> = fn_group
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"git_helper"));
        assert!(names.contains(&"greet"));
        assert!(!names.contains(&"_grep"));
    }

    #[test]
    fn builtins_emitted_into_named_tag_group() {
        let mut state = CompletionState::new();
        state.params.prefix = "t".into();
        let builtins = vec!["true".into(), "trap".into(), "test".into(), "exit".into()];
        let inv = _inv(&builtins, &[], &[], &[]);
        _command_names(&mut state, &inv, false);
        let b = state
            .groups
            .iter()
            .find(|g| g.name == "builtins")
            .expect("builtins group");
        let names: Vec<&str> = b.matches.iter().map(|c| c.str_.as_str()).collect();
        assert!(names.contains(&"true"));
        assert!(names.contains(&"trap"));
        assert!(names.contains(&"test"));
        assert!(!names.contains(&"exit"));
    }

    #[test]
    fn aliases_emitted_into_aliases_group() {
        let mut state = CompletionState::new();
        state.params.prefix = "l".into();
        let aliases = vec!["ll".into(), "la".into(), "gst".into()];
        let inv = _inv(&[], &aliases, &[], &[]);
        _command_names(&mut state, &inv, false);
        let group = state
            .groups
            .iter()
            .find(|g| g.name == "aliases")
            .expect("aliases group");
        let names: Vec<&str> = group.matches.iter().map(|c| c.str_.as_str()).collect();
        assert!(names.contains(&"ll"));
        assert!(names.contains(&"la"));
        assert!(!names.contains(&"gst"));
    }

    #[test]
    fn empty_prefix_emits_all_inventory_entries() {
        let mut state = CompletionState::new();
        let builtins = vec!["true".into(), "false".into()];
        let aliases = vec!["ll".into()];
        let functions = vec!["myfn".into()];
        let inv = _inv(&builtins, &aliases, &functions, &[]);
        _command_names(&mut state, &inv, false);
        let by_group: std::collections::HashMap<&str, usize> = state
            .groups
            .iter()
            .map(|g| (g.name.as_str(), g.matches.len()))
            .collect();
        assert_eq!(by_group["builtins"], 2);
        assert_eq!(by_group["aliases"], 1);
        assert_eq!(by_group["functions"], 1);
    }

    #[test]
    fn externals_only_includes_path_commands_under_commands_tag() {
        // -e mode reads from PATH. We can't assert specific contents
        // (system-dependent) but the `commands` group MUST exist and
        // any non-empty inventory means SOME commands are emitted.
        let mut state = CompletionState::new();
        state.params.prefix = "ls".into();
        let inv = _inv(&[], &[], &[], &[]);
        _command_names(&mut state, &inv, true);
        assert!(state.groups.iter().any(|g| g.name == "commands"));
    }
}
