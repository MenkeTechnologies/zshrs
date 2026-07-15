//! Compsys backend router — Rust ports vs upstream shell functions.
//!
//! **zshrs-original — no C source counterpart.** C zsh has only one
//! compsys backend: the shell-function tree at `Completion/`,
//! autoloaded via `fpath`. zshrs ships a parallel Rust-port tree
//! under `src/compsys/ported/` whose entry points are reachable
//! through this router.
//!
//! The user picks per-machine via `~/.config/zshrs/config.toml`:
//! ```toml
//! [compsys]
//! backend = "rust"   # default — route _NAME calls to compsys::ported
//! # backend = "shell"  # bypass — let standard shfunc dispatch win
//! ```
//!
//! `try_rust_dispatch(name, args)` returns `Some(rc)` when the Rust
//! backend handled the call; `None` to defer to the regular shfunc
//! path (the standard zsh-compatible chain).

use crate::extensions::config::{current as current_config, CompsysBackend};

/// Resolve a `_NAME` to its Rust port fn pointer, gated on the
/// `[compsys] backend = "rust"` config.
///
/// Returns `None` when:
/// - `backend = "shell"` (user opted out of Rust ports);
/// - the name doesn't start with `_`;
/// - or no Rust port is registered for the name (graceful per-name
///   degradation — partial coverage falls through to shell autoload).
///
/// The caller MUST run the returned fn pointer **inside doshfunc's
/// scope** (i.e. as the `body_runner` closure) so the prologue/
/// epilogue side effects (locallevel++, FUNCSTACK push, BREAKS/
/// CONTFLAG/LASTVAL save+restore, trap_state, noerrexit clear,
/// pipestats deep-copy) all apply to the Rust port the same way C
/// applies them to a wordcode shfunc body. Skipping doshfunc would
/// leak per-call scope state into the caller — a real bug, not just
/// a minor difference.
pub fn try_rust_dispatch(name: &str) -> Option<fn(&[String]) -> i32> {
    if current_config().compsys.backend != CompsysBackend::Rust {
        return None;
    }
    if !name.starts_with('_') {
        return None;
    }
    rust_compsys_lookup(name)
}

/// Per-name dispatch table — maps `_NAME` to a `fn(&[String]) -> i32`
/// pointer to the Rust port entry. Every `_NAME` port declared in
/// `compsys::ported` (`mod.rs`) is wired here so the completer chain
/// (`_main_complete` → `_complete` → `_normal`/`_files` →
/// `_path_files` → …) resolves end-to-end to Rust instead of falling
/// through `_ => None` to shell autoload. Ports whose entry takes no
/// args are adapted to the `fn(&[String]) -> i32` router signature via
/// a non-capturing closure (which coerces to a plain fn pointer).
fn rust_compsys_lookup(name: &str) -> Option<fn(&[String]) -> i32> {
    use crate::compsys::ported::*;
    match name {
        // Ports whose entry takes the arg slice directly.
        "_aliases" => Some(_aliases::_aliases),
        "_all_labels" => Some(_all_labels::_all_labels),
        "_alternative" => Some(_alternative::_alternative),
        "_approximate" => Some(_approximate::_approximate),
        "_arg_compile" => Some(_arg_compile::_arg_compile),
        "_arguments" => Some(_arguments::_arguments),
        "_arrays" => Some(_arrays::_arrays),
        "_as_if" => Some(_as_if::_as_if),
        "_cache_invalid" => Some(_cache_invalid::_cache_invalid),
        "_call_function" => Some(_call_function::_call_function),
        "_call_program" => Some(_call_program::_call_program),
        "_canonical_paths" => Some(_canonical_paths::_canonical_paths),
        "_combination" => Some(_combination::_combination),
        "_command_names" => Some(_command_names::_command_names),
        "_complete_debug" => Some(_complete_debug::_complete_debug),
        "_complete_help" => Some(_complete_help::_complete_help),
        "_help_sort_tags" => Some(_complete_help::_help_sort_tags),
        "_completers" => Some(_completers::_completers),
        "_correct_filename" => Some(_correct_filename::_correct_filename),
        "_default" => Some(_default::_default),
        "_delimiters" => Some(_delimiters::_delimiters),
        "_describe" => Some(_describe::_describe),
        "_description" => Some(_description::_description),
        "_dir_list" => Some(_dir_list::_dir_list),
        "_directories" => Some(_directories::_directories),
        "_directory_stack" => Some(_directory_stack::_directory_stack),
        "_dispatch" => Some(_dispatch::_dispatch),
        "_email_addresses" => Some(_email_addresses::_email_addresses),
        "_file_descriptors" => Some(_file_descriptors::_file_descriptors),
        "_files" => Some(_files::_files),
        "_functions" => Some(_functions::_functions),
        "_generic" => Some(_generic::_generic),
        "_guard" => Some(_guard::_guard),
        "_history_modifiers" => Some(_history_modifiers::_history_modifiers),
        "_jobs" => Some(_jobs::_jobs),
        "_jobs_bg" => Some(_jobs_bg::_jobs_bg),
        "_jobs_fg" => Some(_jobs_fg::_jobs_fg),
        "_limits" => Some(_limits::_limits),
        "_main_complete" => Some(_main_complete::_main_complete),
        "_message" => Some(_message::_message),
        "_multi_parts" => Some(_multi_parts::_multi_parts),
        "_next_label" => Some(_next_label::_next_label),
        "_normal" => Some(_normal::_normal),
        "_numbers" => Some(_numbers::_numbers),
        "_options" => Some(_options::_options),
        "_options_set" => Some(_options_set::_options_set),
        "_options_unset" => Some(_options_unset::_options_unset),
        "_parameters" => Some(_parameters::_parameters),
        "_path_commands" => Some(_path_commands::_path_commands),
        "_path_files" => Some(_path_files::_path_files),
        "_pick_variant" => Some(_pick_variant::_pick_variant),
        "_regex_arguments" => Some(_regex_arguments::_regex_arguments),
        "_regex_words" => Some(_regex_words::_regex_words),
        "_requested" => Some(_requested::_requested),
        "_retrieve_cache" => Some(_retrieve_cache::_retrieve_cache),
        "_sep_parts" => Some(_sep_parts::_sep_parts),
        "_sequence" => Some(_sequence::_sequence),
        "_setup" => Some(_setup::_setup),
        "_shadow" => Some(_shadow::_shadow),
        "_unshadow" => Some(|_: &[String]| _shadow::_unshadow()),
        "_store_cache" => Some(_store_cache::_store_cache),
        "_sub_commands" => Some(_sub_commands::_sub_commands),
        "_subscript" => Some(_subscript::_subscript),
        "_suffix_alias_files" => Some(_suffix_alias_files::_suffix_alias_files),
        "_tags" => Some(_tags::_tags),
        "_tilde" => Some(_tilde::_tilde),
        "_tilde_files" => Some(_tilde_files::_tilde_files),
        "_user_math_func" => Some(_user_math_func::_user_math_func),
        "_value" => Some(_value::_value),
        "_values" => Some(_values::_values),
        "_vars" => Some(_vars::_vars),
        "_vcs_info_hooks" => Some(_vcs_info_hooks::_vcs_info_hooks),
        "_wanted" => Some(_wanted::_wanted),
        "_widgets" => Some(_widgets::_widgets),
        // Zero-arg ports, adapted to the router sig via closure coercion.
        "_absolute_command_paths" => {
            Some(|_: &[String]| _absolute_command_paths::_absolute_command_paths())
        }
        "_all_matches" => Some(|_: &[String]| _all_matches::_all_matches()),
        "_assign" => Some(|_: &[String]| _assign::_assign()),
        "_autocd" => Some(|_: &[String]| _autocd::_autocd()),
        "_bash_completions" => Some(|_: &[String]| _bash_completions::_bash_completions()),
        "_brace_parameter" => Some(|_: &[String]| _brace_parameter::_brace_parameter()),
        "_cmdambivalent" => Some(|_: &[String]| _cmdambivalent::_cmdambivalent()),
        "_cmdstring" => Some(|_: &[String]| _cmdstring::_cmdstring()),
        "_command" => Some(|_: &[String]| _command::_command()),
        "_comp_locale" => Some(|_: &[String]| _comp_locale::_comp_locale()),
        "_complete" => Some(|_: &[String]| _complete::_complete()),
        "_complete_help_generic" => {
            Some(|_: &[String]| _complete_help_generic::_complete_help_generic())
        }
        "_complete_tag" => Some(|_: &[String]| _complete_tag::_complete_tag()),
        "_condition" => Some(|_: &[String]| _condition::_condition()),
        "_correct" => Some(|_: &[String]| _correct::_correct()),
        "_correct_word" => Some(|_: &[String]| _correct_word::_correct_word()),
        "_dynamic_directory_name" => {
            Some(|_: &[String]| _dynamic_directory_name::_dynamic_directory_name())
        }
        "_equal" => Some(|_: &[String]| _equal::_equal()),
        "_expand" => Some(|_: &[String]| _expand::_expand()),
        "_expand_alias" => Some(|_: &[String]| _expand_alias::_expand_alias()),
        "_expand_word" => Some(|_: &[String]| _expand_word::_expand_word()),
        "_extensions" => Some(|_: &[String]| _extensions::_extensions()),
        "_external_pwds" => Some(|_: &[String]| _external_pwds::_external_pwds()),
        "_first" => Some(|_: &[String]| _first::_first()),
        "_globflags" => Some(|_: &[String]| _globflags::_globflags()),
        "_globqual_delims" => Some(|_: &[String]| _globqual_delims::_globqual_delims()),
        "_globquals" => Some(|_: &[String]| _globquals::_globquals()),
        "_gnu_generic" => Some(|_: &[String]| _gnu_generic::_gnu_generic()),
        "_history" => Some(|_: &[String]| _history::_history()),
        "_history_complete_word" => {
            Some(|_: &[String]| _history_complete_word::_history_complete_word())
        }
        "_ignored" => Some(|_: &[String]| _ignored::_ignored()),
        "_in_vared" => Some(|_: &[String]| _in_vared::_in_vared()),
        "_list" => Some(|_: &[String]| _list::_list()),
        "_match" => Some(|_: &[String]| _match::_match()),
        "_math" => Some(|_: &[String]| _math::_math()),
        "_math_params" => Some(|_: &[String]| _math_params::_math_params()),
        "_menu" => Some(|_: &[String]| _menu::_menu()),
        "_module_math_func" => Some(|_: &[String]| _module_math_func::_module_math_func()),
        "_most_recent_file" => Some(|_: &[String]| _most_recent_file::_most_recent_file()),
        "_next_tags" => Some(|_: &[String]| _next_tags::_next_tags()),
        "_nothing" => Some(|_: &[String]| _nothing::_nothing()),
        "_oldlist" => Some(|_: &[String]| _oldlist::_oldlist()),
        "_parameter" => Some(|_: &[String]| _parameter::_parameter()),
        "_precommand" => Some(|_: &[String]| _precommand::_precommand()),
        "_prefix" => Some(|_: &[String]| _prefix::_prefix()),
        "_ps1234" => Some(|_: &[String]| _ps1234::_ps1234()),
        "_read_comp" => Some(|_: &[String]| _read_comp::_read_comp()),
        "_redirect" => Some(|_: &[String]| _redirect::_redirect()),
        "_set_command" => Some(|_: &[String]| _set_command::_set_command()),
        "_user_expand" => Some(|_: &[String]| _user_expand::_user_expand()),
        "_zcalc_line" => Some(|_: &[String]| _zcalc_line::_zcalc_line()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_underscore_names() {
        assert!(rust_compsys_lookup("foo").is_none());
        assert!(rust_compsys_lookup("complete-word").is_none());
    }

    #[test]
    fn returns_fn_pointer_for_registered_names() {
        assert!(rust_compsys_lookup("_main_complete").is_some());
        assert!(rust_compsys_lookup("_setup").is_some());
    }

    #[test]
    fn returns_none_for_unregistered_names() {
        // `_nosuch_xyz` has no Rust port — caller falls back to
        // shfunc autoload.
        assert!(rust_compsys_lookup("_nosuch_xyz").is_none());
    }
}
