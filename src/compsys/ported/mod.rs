//! Per-function ports of zsh compsys shell functions — one file per
//! function, organised under the upstream `Completion/` tree:
//!
//!   - `Base/Completer/`  ← top-level completer dispatchers
//!   - `Base/Core/`        ← tag-manager/setup/dispatch infrastructure
//!   - `Base/Utility/`     ← reusable helpers (_arguments, _describe, …)
//!   - `Base/Widget/`      ← ZLE widget entry points
//!   - `Unix/Type/`        ← Unix-specific type completers
//!   - `Zsh/Type/`         ← zsh-internal-type completers
//!
//! Module declarations use `#[path = "..."]` so the on-disk subdir
//! structure matches zsh exactly while module names stay flat
//! (`compsys::ported::_command_names`) so existing call sites need
//! no changes.

/// Split a completion-action string into an argv the way every compsys
/// dispatcher does it — `eval "action=( $action )"` (e.g. `_arguments`
/// sh:453/463, `_alternative` sh:61/69, `_values` sh:138/146, `_complete`
/// sh:61/72). The shell word-parser keeps quoted words with embedded spaces
/// whole and strips their quotes (`-M "r:|/=* r:|=*"` → one arg `r:|/=* r:|=*`)
/// and honours backslash escapes (`device\ path`). The ports previously used
/// `str::split_whitespace()`, a quote-blind split, which chopped
/// `-M "r:|/=* r:|=*"` into `-M`, `"r:|/=*`, `r:|=*"` — the stray leading `"`
/// made `compadd`'s matcher parser reject it (`unknown match specification
/// character '"'`, seen on `df -<TAB>` via `_umountable` → `_canonical_paths`).
/// The scratch globals are unset because zsh's `action` is a function local.
pub fn eval_action_words(action: &str) -> Vec<String> {
    let _ = crate::ported::params::setsparam("_cs_split_src", action);
    let _ = crate::ported::exec::execute_script("eval \"_cs_split_dst=( $_cs_split_src )\"");
    let out = crate::ported::params::getaparam("_cs_split_dst").unwrap_or_default();
    let _ = crate::ported::params::unsetparam("_cs_split_src");
    let _ = crate::ported::params::unsetparam("_cs_split_dst");
    out
}

// ── Base/Completer/ ───────────────────────────────────────────────────
/// `_all_matches` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_all_matches.rs"]
pub mod _all_matches;
/// `_approximate` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_approximate.rs"]
pub mod _approximate;
/// `_complete` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_complete.rs"]
pub mod _complete;
/// `_correct` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_correct.rs"]
pub mod _correct;
/// `_expand` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_expand.rs"]
pub mod _expand;
/// `_expand_alias` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_expand_alias.rs"]
pub mod _expand_alias;
/// `_extensions` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_extensions.rs"]
pub mod _extensions;
/// `_external_pwds` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_external_pwds.rs"]
pub mod _external_pwds;
/// `_history` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_history.rs"]
pub mod _history;
/// `_ignored` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_ignored.rs"]
pub mod _ignored;
/// `_list` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_list.rs"]
pub mod _list;
/// `_match` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_match.rs"]
pub mod _match;
/// `_menu` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_menu.rs"]
pub mod _menu;
/// `_oldlist` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_oldlist.rs"]
pub mod _oldlist;
/// `_prefix` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_prefix.rs"]
pub mod _prefix;
/// `_user_expand` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Completer/_user_expand.rs"]
pub mod _user_expand;

// ── Base/Core/ ────────────────────────────────────────────────────────
/// `_all_labels` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_all_labels.rs"]
pub mod _all_labels;
// `_comp_caller_options` and `_comp_priv_prefix` are shell PARAMETERS,
// not shell functions — their registry-state lives in `compsys/state.rs`
// (see `comp_caller_options*` / `comp_priv_prefix*`).
/// `_description` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_description.rs"]
pub mod _description;
/// `_dispatch` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_dispatch.rs"]
pub mod _dispatch;
/// `_main_complete` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_main_complete.rs"]
pub mod _main_complete;
/// `_message` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_message.rs"]
pub mod _message;
/// `_next_label` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_next_label.rs"]
pub mod _next_label;
/// `_normal` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_normal.rs"]
pub mod _normal;
/// `_requested` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_requested.rs"]
pub mod _requested;
/// `_setup` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_setup.rs"]
pub mod _setup;
/// `_tags` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_tags.rs"]
pub mod _tags;
/// `_wanted` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Core/_wanted.rs"]
pub mod _wanted;

// ── Base/Utility/ ─────────────────────────────────────────────────────
/// `_alternative` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_alternative.rs"]
pub mod _alternative;
/// `_arg_compile` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_arg_compile.rs"]
pub mod _arg_compile;
/// `_arguments` submodule.
#[path = "Base/Utility/_arguments.rs"]
pub mod _arguments;
/// `_as_if` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_as_if.rs"]
pub mod _as_if;
/// `_cache_invalid` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_cache_invalid.rs"]
pub mod _cache_invalid;
/// `_call_function` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_call_function.rs"]
pub mod _call_function;
/// `_call_program` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_call_program.rs"]
pub mod _call_program;
/// `_cmdambivalent` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_cmdambivalent.rs"]
pub mod _cmdambivalent;
/// `_cmdstring` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_cmdstring.rs"]
pub mod _cmdstring;
/// `_combination` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_combination.rs"]
pub mod _combination;
/// `_comp_locale` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_comp_locale.rs"]
pub mod _comp_locale;
/// `_complete_help_generic` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_complete_help_generic.rs"]
pub mod _complete_help_generic;
/// `_completers` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_completers.rs"]
pub mod _completers;
/// `_describe` submodule.
#[path = "Base/Utility/_describe.rs"]
pub mod _describe;
/// `_guard` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_guard.rs"]
pub mod _guard;
/// `_multi_parts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_multi_parts.rs"]
pub mod _multi_parts;
/// `_nothing` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_nothing.rs"]
pub mod _nothing;
/// `_numbers` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_numbers.rs"]
pub mod _numbers;
/// `_pick_variant` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_pick_variant.rs"]
pub mod _pick_variant;
/// `_regex_arguments` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_regex_arguments.rs"]
pub mod _regex_arguments;
/// `_regex_words` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_regex_words.rs"]
pub mod _regex_words;
/// `_retrieve_cache` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_retrieve_cache.rs"]
pub mod _retrieve_cache;
/// `_sep_parts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_sep_parts.rs"]
pub mod _sep_parts;
/// `_sequence` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_sequence.rs"]
pub mod _sequence;
/// `_set_command` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_set_command.rs"]
pub mod _set_command;
/// `_shadow` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_shadow.rs"]
pub mod _shadow;
/// `_store_cache` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_store_cache.rs"]
pub mod _store_cache;
/// `_sub_commands` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_sub_commands.rs"]
pub mod _sub_commands;
/// `_values` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Utility/_values.rs"]
pub mod _values;

// ── Base/Widget/ ──────────────────────────────────────────────────────
/// `_bash_completions` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_bash_completions.rs"]
pub mod _bash_completions;
/// `_complete_debug` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_complete_debug.rs"]
pub mod _complete_debug;
/// `_complete_help` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_complete_help.rs"]
pub mod _complete_help;
/// `_complete_tag` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_complete_tag.rs"]
pub mod _complete_tag;
/// `_correct_filename` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_correct_filename.rs"]
pub mod _correct_filename;
/// `_correct_word` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_correct_word.rs"]
pub mod _correct_word;
/// `_expand_word` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_expand_word.rs"]
pub mod _expand_word;
/// `_generic` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_generic.rs"]
pub mod _generic;
/// `_history_complete_word` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_history_complete_word.rs"]
pub mod _history_complete_word;
/// `_most_recent_file` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_most_recent_file.rs"]
pub mod _most_recent_file;
/// `_next_tags` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_next_tags.rs"]
pub mod _next_tags;
/// `_read_comp` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Base/Widget/_read_comp.rs"]
pub mod _read_comp;

// ── Unix/Type/ ────────────────────────────────────────────────────────
/// `_absolute_command_paths` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_absolute_command_paths.rs"]
pub mod _absolute_command_paths;
/// `_canonical_paths` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_canonical_paths.rs"]
pub mod _canonical_paths;
/// `_command_names` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_command_names.rs"]
pub mod _command_names;
/// `_dir_list` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_dir_list.rs"]
pub mod _dir_list;
/// `_directories` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_directories.rs"]
pub mod _directories;
/// `_email_addresses` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_email_addresses.rs"]
pub mod _email_addresses;
/// `_files` submodule.
#[path = "Unix/Type/_files.rs"]
pub mod _files;
/// `_gnu_generic` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Command/_gnu_generic.rs"]
pub mod _gnu_generic;
/// `_path_commands` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_path_commands.rs"]
pub mod _path_commands;
/// `_path_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_path_files.rs"]
pub mod _path_files;
/// `_precommand` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Command/_precommand.rs"]
pub mod _precommand;
/// `_tilde_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_tilde_files.rs"]
pub mod _tilde_files;

// ── Zsh/Command/ ──────────────────────────────────────────────────────
/// `_command` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Command/_command.rs"]
pub mod _command;

// ── Zsh/Context/ ──────────────────────────────────────────────────────
/// `_assign` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_assign.rs"]
pub mod _assign;
/// `_autocd` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_autocd.rs"]
pub mod _autocd;
/// `_brace_parameter` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_brace_parameter.rs"]
pub mod _brace_parameter;
/// `_condition` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_condition.rs"]
pub mod _condition;
/// `_default` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_default.rs"]
pub mod _default;
/// `_dynamic_directory_name` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_dynamic_directory_name.rs"]
pub mod _dynamic_directory_name;
/// `_equal` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_equal.rs"]
pub mod _equal;
/// `_first` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_first.rs"]
pub mod _first;
/// `_in_vared` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_in_vared.rs"]
pub mod _in_vared;
/// `_math` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_math.rs"]
pub mod _math;
/// `_parameter` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_parameter.rs"]
pub mod _parameter;
/// `_redirect` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_redirect.rs"]
pub mod _redirect;
/// `_subscript` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_subscript.rs"]
pub mod _subscript;
/// `_tilde` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_tilde.rs"]
pub mod _tilde;
/// `_value` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_value.rs"]
pub mod _value;
/// `_zcalc_line` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Context/_zcalc_line.rs"]
pub mod _zcalc_line;

// ── Zsh/Type/ ─────────────────────────────────────────────────────────
/// `_aliases` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_aliases.rs"]
pub mod _aliases;
/// `_arrays` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_arrays.rs"]
pub mod _arrays;
/// `_delimiters` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_delimiters.rs"]
pub mod _delimiters;
/// `_directory_stack` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_directory_stack.rs"]
pub mod _directory_stack;
/// `_file_descriptors` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_file_descriptors.rs"]
pub mod _file_descriptors;
/// `_functions` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_functions.rs"]
pub mod _functions;
/// `_globflags` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_globflags.rs"]
pub mod _globflags;
/// `_globqual_delims` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_globqual_delims.rs"]
pub mod _globqual_delims;
/// `_globquals` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_globquals.rs"]
pub mod _globquals;
/// `_history_modifiers` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_history_modifiers.rs"]
pub mod _history_modifiers;
/// `_jobs` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_jobs.rs"]
pub mod _jobs;
/// `_jobs_bg` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_jobs_bg.rs"]
pub mod _jobs_bg;
/// `_jobs_fg` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_jobs_fg.rs"]
pub mod _jobs_fg;
/// `_limits` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_limits.rs"]
pub mod _limits;
/// `_math_params` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_math_params.rs"]
pub mod _math_params;
/// `_module_math_func` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_module_math_func.rs"]
pub mod _module_math_func;
/// `_options` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_options.rs"]
pub mod _options;
/// `_options_set` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_options_set.rs"]
pub mod _options_set;
/// `_options_unset` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_options_unset.rs"]
pub mod _options_unset;
/// `_parameters` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_parameters.rs"]
pub mod _parameters;
/// `_ps1234` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_ps1234.rs"]
pub mod _ps1234;
/// `_suffix_alias_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_suffix_alias_files.rs"]
pub mod _suffix_alias_files;
/// `_user_math_func` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_user_math_func.rs"]
pub mod _user_math_func;
/// `_vars` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_vars.rs"]
pub mod _vars;
/// `_vcs_info_hooks` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_vcs_info_hooks.rs"]
pub mod _vcs_info_hooks;
/// `_widgets` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_widgets.rs"]
pub mod _widgets;
/// `shared` submodule.
pub mod shared;

/// `compinit` — top-level shell initialisation entry point. Lives at
/// `compsys/ported/compinit.rs` (no leading `_` because it's the shell
/// front-end, not a `_NAME` completion fn).
pub mod compinit;

/// `compdump` — `.zcompdump` cache writer / staleness checker. Split
/// out of `compinit.rs` to mirror upstream `Completion/compdump`.
pub mod compdump;

/// `compaudit` — fpath security check. Faithful port of
/// `Completion/compaudit` (sh:1-176). Lives in its own file per
/// upstream's layout.
pub mod compaudit;
// ── Public re-exports ─────────────────────────────────────────────────
// Items with richer export shapes (Opts structs, consts, etc.):

// Zsh/Context

// Unix/Type — newly-added

// Parameters opts (re-export so wrappers in other crates can build
// the same flag-set the shell `_parameters` accepts).

// Simple one-symbol-per-module re-exports.
pub use shared::{get_ignored_patterns, is_ignored};
