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

// ── Base/Completer/ ───────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_all_matches.rs"] pub mod _all_matches;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_approximate.rs"] pub mod _approximate;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_complete.rs"] pub mod _complete;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_correct.rs"] pub mod _correct;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_expand.rs"] pub mod _expand;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_expand_alias.rs"] pub mod _expand_alias;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_extensions.rs"] pub mod _extensions;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_external_pwds.rs"] pub mod _external_pwds;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_history.rs"] pub mod _history;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_ignored.rs"] pub mod _ignored;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_list.rs"] pub mod _list;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_match.rs"] pub mod _match;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_menu.rs"] pub mod _menu;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_oldlist.rs"] pub mod _oldlist;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_prefix.rs"] pub mod _prefix;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_user_expand.rs"] pub mod _user_expand;

// ── Base/Core/ ────────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_all_labels.rs"] pub mod _all_labels;
// `_comp_caller_options` and `_comp_priv_prefix` are shell PARAMETERS,
// not shell functions — their registry-state lives in `compsys/state.rs`
// (see `comp_caller_options*` / `comp_priv_prefix*`).
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_description.rs"] pub mod _description;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_dispatch.rs"] pub mod _dispatch;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_main_complete.rs"] pub mod _main_complete;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_message.rs"] pub mod _message;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_next_label.rs"] pub mod _next_label;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_normal.rs"] pub mod _normal;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_requested.rs"] pub mod _requested;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_setup.rs"] pub mod _setup;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_tags.rs"] pub mod _tags;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_wanted.rs"] pub mod _wanted;

// ── Base/Utility/ ─────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_alternative.rs"] pub mod _alternative;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_arg_compile.rs"] pub mod _arg_compile;
#[path = "Base/Utility/_arguments.rs"] pub mod _arguments;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_as_if.rs"] pub mod _as_if;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_cache_invalid.rs"] pub mod _cache_invalid;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_call_function.rs"] pub mod _call_function;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_call_program.rs"] pub mod _call_program;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_cmdambivalent.rs"] pub mod _cmdambivalent;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_cmdstring.rs"] pub mod _cmdstring;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_combination.rs"] pub mod _combination;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_comp_locale.rs"] pub mod _comp_locale;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_complete_help_generic.rs"] pub mod _complete_help_generic;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_completers.rs"] pub mod _completers;
#[path = "Base/Utility/_describe.rs"] pub mod _describe;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_guard.rs"] pub mod _guard;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_multi_parts.rs"] pub mod _multi_parts;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_nothing.rs"] pub mod _nothing;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_numbers.rs"] pub mod _numbers;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_pick_variant.rs"] pub mod _pick_variant;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_regex_arguments.rs"] pub mod _regex_arguments;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_regex_words.rs"] pub mod _regex_words;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_retrieve_cache.rs"] pub mod _retrieve_cache;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_sep_parts.rs"] pub mod _sep_parts;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_sequence.rs"] pub mod _sequence;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_set_command.rs"] pub mod _set_command;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_shadow.rs"] pub mod _shadow;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_store_cache.rs"] pub mod _store_cache;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_sub_commands.rs"] pub mod _sub_commands;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_values.rs"] pub mod _values;

// ── Base/Widget/ ──────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_bash_completions.rs"] pub mod _bash_completions;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_complete_debug.rs"] pub mod _complete_debug;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_complete_help.rs"] pub mod _complete_help;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_complete_tag.rs"] pub mod _complete_tag;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_correct_filename.rs"] pub mod _correct_filename;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_correct_word.rs"] pub mod _correct_word;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_expand_word.rs"] pub mod _expand_word;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_generic.rs"] pub mod _generic;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_history_complete_word.rs"] pub mod _history_complete_word;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_most_recent_file.rs"] pub mod _most_recent_file;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_next_tags.rs"] pub mod _next_tags;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Widget/_read_comp.rs"] pub mod _read_comp;

// ── Unix/Type/ ────────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_absolute_command_paths.rs"] pub mod _absolute_command_paths;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_canonical_paths.rs"] pub mod _canonical_paths;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_command_names.rs"] pub mod _command_names;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_dir_list.rs"] pub mod _dir_list;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_directories.rs"] pub mod _directories;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_email_addresses.rs"] pub mod _email_addresses;
#[path = "Unix/Type/_files.rs"] pub mod _files;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Command/_gnu_generic.rs"] pub mod _gnu_generic;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_path_commands.rs"] pub mod _path_commands;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_path_files.rs"] pub mod _path_files;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Command/_precommand.rs"] pub mod _precommand;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_tilde_files.rs"] pub mod _tilde_files;

// ── Zsh/Command/ ──────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Command/_command.rs"] pub mod _command;

// ── Zsh/Context/ ──────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_assign.rs"] pub mod _assign;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_autocd.rs"] pub mod _autocd;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_brace_parameter.rs"] pub mod _brace_parameter;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_condition.rs"] pub mod _condition;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_default.rs"] pub mod _default;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_dynamic_directory_name.rs"] pub mod _dynamic_directory_name;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_equal.rs"] pub mod _equal;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_first.rs"] pub mod _first;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_in_vared.rs"] pub mod _in_vared;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_math.rs"] pub mod _math;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_parameter.rs"] pub mod _parameter;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_redirect.rs"] pub mod _redirect;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_subscript.rs"] pub mod _subscript;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_tilde.rs"] pub mod _tilde;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_value.rs"] pub mod _value;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Context/_zcalc_line.rs"] pub mod _zcalc_line;

// ── Zsh/Type/ ─────────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_aliases.rs"] pub mod _aliases;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_arrays.rs"] pub mod _arrays;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_delimiters.rs"] pub mod _delimiters;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_directory_stack.rs"] pub mod _directory_stack;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_file_descriptors.rs"] pub mod _file_descriptors;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_functions.rs"] pub mod _functions;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_globflags.rs"] pub mod _globflags;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_globqual_delims.rs"] pub mod _globqual_delims;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_globquals.rs"] pub mod _globquals;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_history_modifiers.rs"] pub mod _history_modifiers;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_jobs.rs"] pub mod _jobs;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_jobs_bg.rs"] pub mod _jobs_bg;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_jobs_fg.rs"] pub mod _jobs_fg;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_limits.rs"] pub mod _limits;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_math_params.rs"] pub mod _math_params;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_module_math_func.rs"] pub mod _module_math_func;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_options.rs"] pub mod _options;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_options_set.rs"] pub mod _options_set;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_options_unset.rs"] pub mod _options_unset;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_parameters.rs"] pub mod _parameters;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_ps1234.rs"] pub mod _ps1234;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_suffix_alias_files.rs"] pub mod _suffix_alias_files;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_user_math_func.rs"] pub mod _user_math_func;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_vars.rs"] pub mod _vars;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_vcs_info_hooks.rs"] pub mod _vcs_info_hooks;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_widgets.rs"] pub mod _widgets;

pub mod shared;

/// `compinit` — top-level shell initialisation entry point. Lives at
/// `compsys/ported/compinit.rs` (no leading `_` because it's the shell
/// front-end, not a `_NAME` completion fn).
pub mod compinit;

/// `compdump` — `.zcompdump` cache writer / staleness checker. Split
/// out of `compinit.rs` to mirror upstream `Completion/compdump`.
pub mod compdump;

// ── Public re-exports ─────────────────────────────────────────────────
// Items with richer export shapes (Opts structs, consts, etc.):
pub use _arg_compile::{_arg_compile, CompiledArgSpec};
pub use _call_program::{CallProgramOpts, CallProgramResult, _call_program};
pub use _canonical_paths::{CanonicalPathsOpts, _canonical_paths};
pub use _combination::{CombinationOpts, _combination, _combination_mcs};
pub use _command_names::{ShellInventory, _command_names, _command_names_with_ctx};
pub use _completers::{CANONICAL_COMPLETER_NAMES, _completers};
pub use _dir_list::{DirListOpts, _dir_list};
pub use _email_addresses::{EmailAddressesOpts, _email_addresses};
pub use _multi_parts::{MultiPartsOpts, _multi_parts};
pub use _numbers::{NumbersOpts, _numbers};
pub use _path_files::{PathFilesOpts, _path_files};
pub use _pick_variant::{PickVariantOpts, PickVariantResult, _pick_variant};
pub use _sequence::{SequenceOpts, _sequence};
pub use _tags::{TagSortHook, TagsOpts, _tags, _tags_mcs, _tags_next};
pub use _aliases::{AliasTables as _AliasTables_Zsh, _aliases};
pub use _arrays::_arrays;
pub use _delimiters::_delimiters;
pub use _directory_stack::_directory_stack;
pub use _file_descriptors::_file_descriptors;
pub use _functions::_functions;
pub use _globflags::_globflags;
pub use _globqual_delims::{GlobQualDelimsResult, _globqual_delims};
pub use _globquals::_globquals;
pub use _history_modifiers::{ModifierContext, _history_modifiers};
pub use _jobs::{JobEntry, JobState, JobsFilter, _jobs};
pub use _jobs_bg::_jobs_bg;
pub use _jobs_fg::_jobs_fg;
pub use _limits::_limits;
pub use _math_params::_math_params;
pub use _module_math_func::_module_math_func;
pub use _ps1234::_ps1234;
pub use _suffix_alias_files::_suffix_alias_files;
pub use _user_math_func::_user_math_func;
pub use _vars::_vars;
pub use _vcs_info_hooks::_vcs_info_hooks;
pub use _widgets::_widgets;

// Zsh/Context
pub use _assign::_assign;
pub use _autocd::_autocd;
pub use _brace_parameter::_brace_parameter;
pub use _condition::_condition;
pub use _dynamic_directory_name::_dynamic_directory_name;
pub use _equal::_equal;
pub use _first::_first;
pub use _in_vared::_in_vared;
pub use _math::_math;
pub use _parameter::_parameter;
pub use _redirect::_redirect;
pub use _subscript::{SubscriptTarget, _subscript};
pub use _tilde::_tilde;
pub use _value::_value;
pub use _zcalc_line::{ZCALC_ESCAPES, _zcalc_line};

// Unix/Type — newly-added
pub use _path_commands::_path_commands;

// Parameters opts (re-export so wrappers in other crates can build
// the same flag-set the shell `_parameters` accepts).
pub use _parameters::{ParametersOpts, _parameters_with_opts};

// Simple one-symbol-per-module re-exports.
pub use _absolute_command_paths::_absolute_command_paths;
pub use _all_labels::_all_labels;
pub use _all_matches::_all_matches;
pub use _alternative::_alternative;
pub use _approximate::_approximate;
pub use _as_if::_as_if;
pub use _bash_completions::_bash_completions;
pub use _cache_invalid::_cache_invalid;
pub use _call_function::_call_function;
pub use _cmdambivalent::_cmdambivalent;
pub use _cmdstring::_cmdstring;
pub use _command::{CommandStage, _command};
pub use _comp_locale::_comp_locale;
pub use _complete::_complete;
pub use _complete_debug::_complete_debug;
pub use _complete_help::_complete_help;
pub use _complete_help_generic::_complete_help_generic;
pub use _complete_tag::_complete_tag;
pub use _correct::_correct;
pub use _correct_filename::_correct_filename;
pub use _correct_word::_correct_word;
pub use _default::_default;
pub use _description::_description;
pub use _directories::_directories;
pub use _dispatch::_dispatch;
pub use _expand::_expand;
pub use _expand_alias::_expand_alias;
pub use _expand_word::_expand_word;
pub use _extensions::_extensions;
pub use _external_pwds::_external_pwds;
pub use _generic::_generic;
pub use _gnu_generic::_gnu_generic;
pub use _guard::_guard;
pub use _history::_history;
pub use _history_complete_word::_history_complete_word;
pub use _ignored::_ignored;
pub use _list::_list;
pub use _main_complete::_main_complete;
pub use _match::_match;
pub use _menu::_menu;
pub use _message::_message;
pub use _most_recent_file::_most_recent_file;
pub use _next_label::_next_label;
pub use _next_tags::_next_tags;
pub use _normal::_normal;
pub use _nothing::_nothing;
pub use _oldlist::_oldlist;
pub use _options::_options;
pub use _options_set::_options_set;
pub use _options_unset::_options_unset;
pub use _parameters::_parameters;
pub use _precommand::_precommand;
pub use _prefix::_prefix;
pub use _read_comp::_read_comp;
pub use _regex_arguments::_regex_arguments;
pub use _regex_words::_regex_words;
pub use _requested::{RequestedAction, RequestedFlags, _requested};
pub use _retrieve_cache::_retrieve_cache;
pub use _sep_parts::_sep_parts;
pub use _set_command::_set_command;
pub use _setup::_setup;
pub use _shadow::_shadow;
pub use _store_cache::_store_cache;
pub use _sub_commands::_sub_commands;
pub use _tilde_files::_tilde_files;
pub use _user_expand::_user_expand;
pub use _values::_values;
pub use _wanted::_wanted;
pub use shared::{get_ignored_patterns, is_ignored};
