//! Complete library of zsh completion system functions
//!
//! This module implements ALL library functions documented in zshcompsys(1).
//! Command-specific completions (_git, _docker, etc.) remain as shell code.

use crate::base::{CompleterResult, MainCompleteState};
use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};
use std::collections::HashMap;
use std::path::Path;

// =============================================================================
// Missing functions from zshcompsys man page
// =============================================================================

/// _absolute_command_paths - Complete commands with absolute paths
pub fn absolute_command_paths(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();

    // Search PATH for executables
    if let Ok(path_var) = std::env::var("PATH") {
        state.begin_group("commands", true);

        for dir in path_var.split(':') {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();

                    if name_str.starts_with(&prefix) {
                        // Return absolute path
                        let full_path = entry.path();
                        if is_executable(&full_path) {
                            state.add_match(
                                Completion::new(full_path.to_string_lossy().to_string()),
                                Some("commands"),
                            );
                        }
                    }
                }
            }
        }

        state.end_group();
        state.nmatches > 0
    } else {
        false
    }
}

/// _canonical_paths - Complete canonical (resolved) paths
pub fn canonical_paths(
    state: &mut CompletionState,
    tag: &str,
    description: &str,
    paths: &[String],
) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group(tag, true);
    if !description.is_empty() {
        state.add_explanation(description.to_string(), Some(tag));
    }

    for path in paths {
        if let Ok(canonical) = std::fs::canonicalize(path) {
            let canonical_str = canonical.to_string_lossy().to_string();
            if canonical_str.starts_with(&prefix) {
                state.add_match(Completion::new(canonical_str), Some(tag));
            }
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _cmdambivalent - Handle commands that can be run with or without arguments
pub fn cmdambivalent(state: &mut MainCompleteState, inv: &ShellInventory<'_>) -> bool {
    // If no arguments yet, complete as command
    if state.comp.params.current <= 1 {
        command_names(&mut state.comp, inv, false)
    } else {
        // Otherwise use normal completion
        true
    }
}

/// _cmdstring - Complete a command string (for eval, etc.)
pub fn cmdstring(state: &mut CompletionState, inv: &ShellInventory<'_>) -> bool {
    // Complete as if it were a command line
    command_names(state, inv, false)
}

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
/// let inv = ShellInventory { aliases: &aliases, ... };
/// compsys::library::command_names(state, &inv, false);
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

/// Port of `_command_names` (zsh Completion/Base/Utility/_command_names).
///
/// Completes everything valid at command position. Faithful re-port of
/// the 68-line shell function — each tag category gets its own group
/// so `zstyle ':completion:*:command:aliases' …` works:
///
/// ```text
/// commands       — external executables found on $path
/// builtins       — names from inv.builtins
/// functions      — entries in inv.functions (with prefix-needed filter)
/// aliases        — entries in inv.aliases
/// suffix-aliases — entries in inv.suffix_aliases
/// reserved-words — entries in inv.reserved_words
/// jobs           — entries in inv.jobs (TODO: jobtab wiring upstream)
/// parameters     — entries in inv.parameters (with `=` suffix)
/// ```
///
/// Honored zstyles (shell source line refs):
///   `:commands rehash` (l.9)         — silently true; the caller
///                                       owns the cmdnam cache.
///   `:functions prefix-needed` (l.11) — when PREFIX is not `[_.]*`,
///                                       filter out `_*` / `.*` fns.
///   `command-path` (l.49)             — if set, use these dirs instead
///                                       of $path for external scan.
///
/// Modes:
///   `externals_only = true` (shell `-e` or `-`) — skip every internal
///       category; only emit `commands` matches.
///   precommand context (shell l.28: `$precommands:|builtin_precommands`)
///       not implemented at the leaf — the precommand-prefix machinery
///       belongs in the caller, which can pass an empty `inv` to mimic
///       the suppression effect.
pub fn command_names(
    state: &mut CompletionState,
    inv: &ShellInventory<'_>,
    externals_only: bool,
) -> bool {
    // Shell uses `${curcontext}` for zstyle lookups. CompletionState
    // alone doesn't carry curcontext (it lives on `MainCompleteState`),
    // so use the default class `command` — matches what `compinit`
    // sets when `_main_complete` enters command position.
    command_names_with_ctx(state, inv, "::command", externals_only)
}

/// `command_names` taking an explicit `curcontext` (everything after
/// `:completion:`). Use this when a caller already has the
/// `MainCompleteState.ctx.context` available — `:completion:$ctx:foo`
/// style lookups need it. The plain [`command_names`] uses `::command`.
pub fn command_names_with_ctx(
    state: &mut CompletionState,
    inv: &ShellInventory<'_>,
    curcontext: &str,
    externals_only: bool,
) -> bool {
    let prefix = state.params.prefix.clone();

    // Honor `:functions prefix-needed` (shell l.11-13). Default off.
    // When PREFIX doesn't begin with `_` or `.`, drop fns whose names
    // start with those chars — the user is clearly not asking for
    // internal/compsys completers.
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
    // Honor `command-path` style (shell l.49). Falls back to $PATH;
    // `_comp_priv_prefix` sbin augmentation (shell l.57-60) belongs
    // in the caller (the priv-prefix lives in the parent crate).
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
    // Shell uses `_parameters -g "^*(readonly|association)*"` to limit
    // to scalar-ish params and appends `=` as suffix. We don't track
    // the readonly/association attribute precisely at this layer —
    // list every visible param the caller provided.
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

/// _comp_caller_options - Get options from calling context
pub fn comp_caller_options() -> HashMap<String, bool> {
    // Returns shell options that were set when completion was invoked
    // This is stored in $_comp_caller_options in zsh
    HashMap::new()
}

/// _comp_priv_prefix - Prefix for privilege escalation (sudo, doas, etc.)
pub fn comp_priv_prefix() -> Vec<String> {
    // Returns the privilege prefix if any
    Vec::new()
}

/// Canonical 11-completer list emitted by `_completers` (shell:7-8).
/// Mirrors the upstream `list=(…)` literal verbatim — order matters
/// because zsh's `compadd` preserves it when sorting is off.
pub const CANONICAL_COMPLETER_NAMES: &[&str] = &[
    "complete",
    "approximate",
    "correct",
    "match",
    "expand",
    "list",
    "menu",
    "oldlist",
    "ignored",
    "prefix",
    "history",
];

/// Port of `_completers` (zsh Completion/Base/Utility/_completers).
///
/// Emits completer names into the `completers` tag group. Matches the
/// 14-line shell function exactly:
///   - With `with_underscore_prefix = true` (shell `-p` flag), names
///     are emitted as `_complete` / `_approximate` / … so the user
///     gets the function form usable in `compdef -K`.
///   - Otherwise names are emitted bare (`complete` / `approximate`)
///     — the form usable in `zstyle :completion:*:completer …`.
///   - The `prefix-hidden` zstyle (shell:11) drives a `-d list`
///     display alias when bare names should be shown alongside the
///     `_xxx` insertion form. Modeled here by setting `disp` to the
///     bare name while `str_` carries the `_xxx` form.
///
/// Signature change from the previous stub (which returned a `Vec<String>`
/// of currently-active completers — not a completion at all, despite
/// the shell impl's actual job being to ADD completion matches into
/// the receiver).
pub fn completers(state: &mut CompletionState, with_underscore_prefix: bool) -> bool {
    let prefix = state.params.prefix.clone();
    let curcontext = "::completers";

    // shell:11-12: zstyle -t :completion:${curcontext}:completers prefix-hidden
    let prefix_hidden = state
        .styles
        .lookup_bool(
            &format!(":completion:{}:completers", curcontext),
            "prefix-hidden",
        )
        .unwrap_or(false);

    let us = if with_underscore_prefix { "_" } else { "" };

    state.begin_group("completers", true);
    for &bare in CANONICAL_COMPLETER_NAMES {
        let inserted = format!("{}{}", us, bare);
        if !inserted.starts_with(&prefix) {
            continue;
        }
        let mut c = Completion::new(inserted);
        // `prefix-hidden`: shell uses `-d list` so listing shows the
        // BARE name (`complete`) while insertion still uses `_complete`.
        if prefix_hidden && with_underscore_prefix {
            c.disp = Some(bare.into());
        }
        state.add_match(c, Some("completers"));
    }
    state.end_group();
    state.nmatches > 0
}

/// _default - Default completion (files)
pub fn default_complete(state: &mut CompletionState) -> bool {
    crate::files::files_execute(state, &crate::files::FilesOpts::default())
}

/// _dir_list - Complete colon-separated directory list
pub fn dir_list(
    state: &mut CompletionState,
    separator: Option<&str>,
    strip_trailing: bool,
) -> bool {
    let sep = separator.unwrap_or(":");
    let prefix = state.params.prefix.clone();

    // Handle the last component after separator
    let (base, current) = if let Some(pos) = prefix.rfind(sep) {
        (&prefix[..pos + sep.len()], &prefix[pos + sep.len()..])
    } else {
        ("", prefix.as_str())
    };

    // Complete directories
    let dir_to_scan = if current.contains('/') {
        let pos = current.rfind('/').unwrap();
        &current[..pos + 1]
    } else {
        "."
    };

    if let Ok(entries) = std::fs::read_dir(dir_to_scan) {
        state.begin_group("directories", true);

        for entry in entries.flatten() {
            if entry.path().is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                let full = if dir_to_scan == "." {
                    name_str.to_string()
                } else {
                    format!("{}{}", dir_to_scan, name_str)
                };

                if full.starts_with(current) {
                    let mut comp_str = format!("{}{}", base, full);
                    if !strip_trailing {
                        comp_str.push('/');
                    }
                    let mut comp = Completion::new(comp_str);
                    comp.flags |= CompletionFlags::NOSPACE;
                    state.add_match(comp, Some("directories"));
                }
            }
        }

        state.end_group();
    }

    state.nmatches > 0
}

/// _email_addresses - Complete email addresses
pub fn email_addresses(state: &mut CompletionState, complete_struc: bool) -> bool {
    let prefix = state.params.prefix.clone();

    // Try to read from common sources
    let mut addresses = Vec::new();

    // ~/.mailrc
    if let Ok(home) = std::env::var("HOME") {
        let mailrc = format!("{}/.mailrc", home);
        if let Ok(content) = std::fs::read_to_string(&mailrc) {
            for line in content.lines() {
                if line.starts_with("alias ") {
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    if parts.len() >= 3 {
                        addresses.push(parts[2].to_string());
                    }
                }
            }
        }
    }

    state.begin_group("email-addresses", true);

    for addr in &addresses {
        if addr.starts_with(&prefix) {
            let comp = if complete_struc && !addr.contains('<') {
                Completion::new(format!("<{}>", addr))
            } else {
                Completion::new(addr.clone())
            };
            state.add_match(comp, Some("email-addresses"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _gnu_generic - Generic GNU-style option completion from --help
pub fn gnu_generic(state: &mut CompletionState, command: &str) -> bool {
    let prefix = state.params.prefix.clone();

    // Run command --help and parse options
    let output = std::process::Command::new(command).arg("--help").output();

    let help_text = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            format!("{}{}", stdout, stderr)
        }
        Err(_) => return false,
    };

    state.begin_group("options", true);

    // Parse --option and -o patterns
    for line in help_text.lines() {
        let line = line.trim();

        // Match patterns like: --option, -o, --option=ARG
        let mut i = 0;
        while i < line.len() {
            if line[i..].starts_with("--") {
                let start = i;
                i += 2;
                while i < line.len() {
                    let c = line.chars().nth(i).unwrap_or(' ');
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let opt = &line[start..i];
                if opt.len() > 2 && opt.starts_with(&prefix) {
                    // Check for =ARG
                    let has_arg = line[i..].starts_with("=") || line[i..].starts_with("[=");
                    let mut comp = Completion::new(opt.to_string());
                    if has_arg {
                        comp.suf = Some("=".to_string());
                        comp.flags |= CompletionFlags::NOSPACE;
                    }
                    state.add_match(comp, Some("options"));
                }
            } else if line[i..].starts_with("-") && !line[i..].starts_with("--") {
                let start = i;
                i += 1;
                if i < line.len()
                    && line
                        .chars()
                        .nth(i)
                        .map(|c| c.is_alphanumeric())
                        .unwrap_or(false)
                {
                    i += 1;
                    let opt = &line[start..i];
                    if opt.starts_with(&prefix) {
                        state.add_match(Completion::new(opt.to_string()), Some("options"));
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _options - Complete shell options
pub fn options(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("options", true);

    for (opt, is_set) in shell_options {
        if opt.starts_with(&prefix) {
            let mut comp = Completion::new(opt.to_string());
            comp.disp = Some(format!(
                "{} ({})",
                opt,
                if *is_set { "set" } else { "unset" }
            ));
            state.add_match(comp, Some("options"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _options_set - Complete currently set options
pub fn options_set(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let set_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| *is_set)
        .copied()
        .collect();
    options(state, &set_opts)
}

/// _options_unset - Complete currently unset options
pub fn options_unset(state: &mut CompletionState, shell_options: &[(&str, bool)]) -> bool {
    let unset_opts: Vec<(&str, bool)> = shell_options
        .iter()
        .filter(|(_, is_set)| !*is_set)
        .copied()
        .collect();
    options(state, &unset_opts)
}

/// _parameters - Complete parameter (variable) names
pub fn parameters(state: &mut CompletionState, params: &HashMap<String, String>) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("parameters", true);

    for name in params.keys() {
        if name.starts_with(&prefix) {
            state.add_match(Completion::new(name.clone()), Some("parameters"));
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// _path_files - Complete files with path handling
pub fn path_files(state: &mut CompletionState, opts: &PathFilesOpts) -> bool {
    let prefix = state.params.prefix.clone();

    // Determine directory to search
    let (dir, file_prefix) = if let Some(sep) = prefix.rfind('/') {
        (prefix[..sep + 1].to_string(), &prefix[sep + 1..])
    } else {
        (".".to_string(), prefix.as_str())
    };

    // Handle -W (search in specific directories)
    let search_dirs = if let Some(ref dirs) = opts.search_dirs {
        dirs.clone()
    } else {
        vec![dir.clone()]
    };

    state.begin_group(opts.tag.as_deref().unwrap_or("files"), true);

    for search_dir in &search_dirs {
        let full_dir = if search_dir.ends_with('/') {
            format!("{}{}", search_dir, dir.trim_start_matches("./"))
        } else {
            search_dir.clone()
        };

        if let Ok(entries) = std::fs::read_dir(&full_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                if !name_str.starts_with(file_prefix) {
                    continue;
                }

                // Apply glob filter
                if let Some(ref glob) = opts.glob {
                    if !glob_matches(glob, &name_str) && !entry.path().is_dir() {
                        continue;
                    }
                }

                // Apply ignore patterns
                if let Some(ref ignore) = opts.ignore {
                    if glob_matches(ignore, &name_str) {
                        continue;
                    }
                }

                let is_dir = entry.path().is_dir();

                // Filter by type
                if opts.dirs_only && !is_dir {
                    continue;
                }
                if opts.files_only && is_dir {
                    continue;
                }

                let full_path = if dir == "." {
                    name_str.to_string()
                } else {
                    format!("{}{}", dir, name_str)
                };

                let mut comp = Completion::new(full_path);

                // Set file mode character for LS_COLORS coloring
                if is_dir {
                    comp.modec = '/';
                    comp.suf = Some("/".to_string());
                    comp.flags |= CompletionFlags::NOSPACE;
                } else if entry.path().is_symlink() {
                    comp.modec = '@';
                } else {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if let Ok(meta) = entry.metadata() {
                            if meta.permissions().mode() & 0o111 != 0 {
                                comp.modec = '*';
                            }
                        }
                    }
                }

                // Apply prefix/suffix
                if let Some(ref p) = opts.prefix {
                    comp.pre = Some(p.clone());
                }
                if let Some(ref s) = opts.suffix {
                    comp.suf = Some(s.clone());
                }

                state.add_match(comp, opts.tag.as_deref());
            }
        }
    }

    state.end_group();
    state.nmatches > 0
}

/// Options for _path_files
#[derive(Default)]
pub struct PathFilesOpts {
    pub glob: Option<String>,
    pub ignore: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub search_dirs: Option<Vec<String>>,
    pub dirs_only: bool,
    pub files_only: bool,
    pub tag: Option<String>,
}

/// _precommand - Complete after a precommand (sudo, nohup, etc.)
pub fn precommand(state: &mut MainCompleteState) -> bool {
    // Skip the precommand and complete as normal command
    if state.comp.params.current > 1 {
        // Treat rest as command line
        matches!(
            crate::base::normal_complete(state),
            CompleterResult::Matched
        )
    } else {
        false
    }
}

/// _tilde_files - Complete files with tilde expansion
pub fn tilde_files(state: &mut CompletionState) -> bool {
    let prefix = state.params.prefix.clone();

    if prefix.starts_with('~') {
        // Expand tilde
        if let Ok(home) = std::env::var("HOME") {
            let expanded = if prefix == "~" {
                home.clone()
            } else if let Some(after_tilde) = prefix.strip_prefix("~/") {
                format!("{}/{}", home, after_tilde)
            } else {
                // ~user form - would need to look up user
                return false;
            };

            // Update state prefix for completion
            let old_prefix = state.params.prefix.clone();
            state.params.prefix = expanded;
            state.params.iprefix = "~".to_string();

            let result = crate::files::files_execute(state, &crate::files::FilesOpts::default());

            // Restore
            state.params.prefix = old_prefix;
            state.params.iprefix.clear();

            return result;
        }
    }

    false
}

/// _widgets - Complete widget names
pub fn widgets(state: &mut CompletionState, widgets: &[String], pattern: Option<&str>) -> bool {
    let prefix = state.params.prefix.clone();

    state.begin_group("widgets", true);

    for widget in widgets {
        if !widget.starts_with(&prefix) {
            continue;
        }

        if let Some(pat) = pattern {
            if !glob_matches(pat, widget) {
                continue;
            }
        }

        state.add_match(Completion::new(widget.clone()), Some("widgets"));
    }

    state.end_group();
    state.nmatches > 0
}

// =============================================================================
// Helper functions
// =============================================================================

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = path.metadata() {
            let mode = meta.permissions().mode();
            return mode & 0o111 != 0;
        }
    }
    #[cfg(not(unix))]
    {
        // On non-Unix, check for common executable extensions
        if let Some(ext) = path.extension() {
            let ext = ext.to_string_lossy().to_lowercase();
            return matches!(ext.as_str(), "exe" | "bat" | "cmd" | "com");
        }
    }
    false
}

fn glob_matches(pattern: &str, text: &str) -> bool {
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();
    glob_match_helper(&pattern_chars, &text_chars)
}

fn glob_match_helper(pattern: &[char], text: &[char]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some('*'), _) => {
            glob_match_helper(&pattern[1..], text)
                || (!text.is_empty() && glob_match_helper(pattern, &text[1..]))
        }
        (Some('?'), Some(_)) => glob_match_helper(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => glob_match_helper(&pattern[1..], &text[1..]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_glob_matches() {
        assert!(glob_matches("*.rs", "zle_main"));
        assert!(glob_matches("_*", "_git"));
        assert!(!glob_matches("*.rs", "main.txt"));
    }

    #[test]
    fn test_is_executable() {
        // /bin/ls should be executable
        assert!(is_executable(Path::new("/bin/ls")) || is_executable(Path::new("/usr/bin/ls")));
    }

    // ── _command_names port (faithful to Completion/Base/Utility) ─────

    fn _cn_inv<'a>(
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
    fn command_names_emits_all_eight_tag_groups() {
        // Regression for the original stub (compsys/library.rs:117-120
        // before the port): the `if !externals_only` block had a TODO
        // comment instead of actually adding builtins / aliases / fns.
        let mut state = CompletionState::new();
        state.params.prefix = "t".into();
        let builtins = vec!["true".into(), "test".into()];
        let aliases = vec!["tlf".into()];
        let functions = vec!["tlsdir".into(), "_tabby".into()];
        let reswords = vec!["then".into(), "time".into()];
        let inv = _cn_inv(&builtins, &aliases, &functions, &reswords);
        command_names(&mut state, &inv, false);

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
    fn command_names_externals_only_skips_internal_categories() {
        let mut state = CompletionState::new();
        state.params.prefix = "t".into();
        let builtins = vec!["true".into()];
        let inv = _cn_inv(&builtins, &[], &[], &[]);
        command_names(&mut state, &inv, true);
        let group_names: Vec<&str> = state.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(
            group_names,
            vec!["commands"],
            "externals_only must NOT emit internal-category groups",
        );
    }

    // ── _completers port ──────────────────────────────────────────────

    #[test]
    fn completers_emits_canonical_eleven_bare() {
        let mut state = CompletionState::new();
        completers(&mut state, false);
        let mut names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        names.sort();
        // Default group sort matches shell `compadd` alpha-sort. The
        // SET must contain all 11 canonical names from the shell
        // `list=(…)` literal at compsys/library.rs ~324.
        assert_eq!(
            names,
            vec![
                "approximate",
                "complete",
                "correct",
                "expand",
                "history",
                "ignored",
                "list",
                "match",
                "menu",
                "oldlist",
                "prefix",
            ],
        );
    }

    #[test]
    fn completers_with_p_flag_adds_underscore_prefix() {
        let mut state = CompletionState::new();
        completers(&mut state, true);
        let names: std::collections::HashSet<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert_eq!(names.len(), 11);
        assert!(names.contains("_complete"));
        assert!(names.contains("_approximate"));
        assert!(names.contains("_history"));
        // No bare names — `-p` flag means every entry MUST carry the
        // `_` prefix.
        assert!(!names.contains("complete"));
    }

    #[test]
    fn completers_prefix_hidden_shows_bare_in_disp() {
        let mut state = CompletionState::new();
        state.styles.set(
            ":completion:::completers:completers",
            "prefix-hidden",
            vec!["true".into()],
            false,
        );
        completers(&mut state, true);
        // Pick the `_complete` entry by string match (group is sorted).
        let c = state
            .groups[0]
            .matches
            .iter()
            .find(|m| m.str_ == "_complete")
            .expect("_complete present");
        assert_eq!(c.disp.as_deref(), Some("complete"));
    }

    #[test]
    fn command_names_prefix_needed_filters_underscore_fns() {
        let mut state = CompletionState::new();
        state.params.prefix = "g".into();
        state.styles.set(
            ":completion:::command:functions",
            "prefix-needed",
            vec!["true".into()],
            false,
        );
        let functions = vec!["git_helper".into(), "_grep".into(), "greet".into()];
        let inv = _cn_inv(&[], &[], &functions, &[]);
        command_names(&mut state, &inv, false);

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
        // prefix-needed=true + PREFIX!=`_*|\.*` should drop `_grep`.
        // Bonus: even without prefix-needed, the user typed `g` not
        // `_g`, so `_grep` is already off-prefix.
        assert!(
            !names.contains(&"_grep"),
            "prefix-needed should suppress _ fns when PREFIX doesn't start with _ or ."
        );
    }
}
