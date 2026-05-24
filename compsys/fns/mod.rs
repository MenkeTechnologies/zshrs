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
//! (`compsys::fns::_command_names`) so existing call sites need
//! no changes.

// ── Base/Completer/ ───────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_all_matches.rs"] pub mod _all_matches;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Completer/_approximate.rs"] pub mod _approximate;
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
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_comp_caller_options.rs"] pub mod _comp_caller_options;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_comp_priv_prefix.rs"] pub mod _comp_priv_prefix;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_dispatch.rs"] pub mod _dispatch;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_setup.rs"] pub mod _setup;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_tags.rs"] pub mod _tags;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Core/_wanted.rs"] pub mod _wanted;

// ── Base/Utility/ ─────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_arg_compile.rs"] pub mod _arg_compile;
#[path = "Base/Utility/_arguments.rs"] pub mod _arguments;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_as_if.rs"] pub mod _as_if;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_cache_invalid.rs"] pub mod _cache_invalid;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_call_function.rs"] pub mod _call_function;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_call_program.rs"] pub mod _call_program;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_cmdambivalent.rs"] pub mod _cmdambivalent;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_cmdstring.rs"] pub mod _cmdstring;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_combination.rs"] pub mod _combination;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_comp_locale.rs"] pub mod _comp_locale;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_complete_help_generic.rs"] pub mod _complete_help_generic;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_completers.rs"] pub mod _completers;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_default.rs"] pub mod _default;
#[path = "Base/Utility/_describe.rs"] pub mod _describe;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_guard.rs"] pub mod _guard;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_multi_parts.rs"] pub mod _multi_parts;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_nothing.rs"] pub mod _nothing;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_numbers.rs"] pub mod _numbers;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_pick_variant.rs"] pub mod _pick_variant;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_regex_arguments.rs"] pub mod _regex_arguments;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_regex_words.rs"] pub mod _regex_words;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_retrieve_cache.rs"] pub mod _retrieve_cache;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_sequence.rs"] pub mod _sequence;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_set_command.rs"] pub mod _set_command;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_shadow.rs"] pub mod _shadow;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_store_cache.rs"] pub mod _store_cache;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Base/Utility/_sub_commands.rs"] pub mod _sub_commands;

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
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_command_names.rs"] pub mod _command_names;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_dir_list.rs"] pub mod _dir_list;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_email_addresses.rs"] pub mod _email_addresses;
#[path = "Unix/Type/_files.rs"] pub mod _files;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_gnu_generic.rs"] pub mod _gnu_generic;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_path_files.rs"] pub mod _path_files;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_precommand.rs"] pub mod _precommand;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Unix/Type/_tilde_files.rs"] pub mod _tilde_files;

// ── Zsh/Type/ ─────────────────────────────────────────────────────────
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_options.rs"] pub mod _options;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_options_set.rs"] pub mod _options_set;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_options_unset.rs"] pub mod _options_unset;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_parameters.rs"] pub mod _parameters;
#[allow(non_snake_case, non_camel_case_types)] #[path = "Zsh/Type/_widgets.rs"] pub mod _widgets;

pub(crate) mod shared;

/// `compinit` — top-level shell initialisation entry point. Lives at
/// `compsys/fns/compinit.rs` (no leading `_` because it's the shell
/// front-end, not a `_NAME` completion fn).
pub mod compinit;

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
pub use _widgets::_widgets;

// Simple one-symbol-per-module re-exports.
pub use _absolute_command_paths::_absolute_command_paths;
pub use _all_matches::_all_matches;
pub use _approximate::_approximate;
pub use _as_if::_as_if;
pub use _bash_completions::_bash_completions;
pub use _cache_invalid::_cache_invalid;
pub use _call_function::_call_function;
pub use _cmdambivalent::_cmdambivalent;
pub use _cmdstring::_cmdstring;
pub use _comp_caller_options::_comp_caller_options;
pub use _comp_locale::_comp_locale;
pub use _comp_priv_prefix::_comp_priv_prefix;
pub use _complete_debug::_complete_debug;
pub use _complete_help::_complete_help;
pub use _complete_help_generic::_complete_help_generic;
pub use _complete_tag::_complete_tag;
pub use _correct::_correct;
pub use _correct_filename::_correct_filename;
pub use _correct_word::_correct_word;
pub use _default::_default;
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
pub use _match::_match;
pub use _menu::_menu;
pub use _most_recent_file::_most_recent_file;
pub use _next_tags::_next_tags;
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
pub use _retrieve_cache::_retrieve_cache;
pub use _set_command::_set_command;
pub use _setup::_setup;
pub use _shadow::_shadow;
pub use _store_cache::_store_cache;
pub use _sub_commands::_sub_commands;
pub use _tilde_files::_tilde_files;
pub use _user_expand::_user_expand;
pub use _wanted::_wanted;
