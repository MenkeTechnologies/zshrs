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
    // A Rust port stands in for the STOCK upstream function of that name.
    // When `$fpath` resolves the name to somebody else's file first — the
    // user's own `~/.../comp_utils/_parameters`, a plugin's copy — zsh would
    // autoload THAT, so the port must step aside or the customization is
    // silently dead. This is how `unset <TAB>` diverged: zsh ran his
    // `_parameters` (names + values, 496 entries), zshrs ran the port (bare
    // names, 197).
    if has_fpath_override(name) {
        return None;
    }
    rust_compsys_lookup(name)
}

/// True when `$fpath`'s FIRST file for `name` is not a stock zsh
/// completion-function file, i.e. the user (or a plugin) overrides it.
///
/// zshrs-original — C has no native compsys tree to arbitrate against.
/// Stock = the shipped `Completion/` tree, installed as
/// `…/share/zsh/<version>/functions` (and `/usr/share/zsh/functions`).
/// `site-functions` is NOT stock: third parties install there.
///
/// Memoized against the live `$fpath`; the map is rebuilt whenever fpath
/// changes, so a mid-session `fpath=(…)` is honored. Without the memo this
/// would stat up to `#fpath` directories on every completer call.
fn has_fpath_override(name: &str) -> bool {
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static MEMO: OnceLock<Mutex<(Vec<String>, HashMap<String, bool>)>> = OnceLock::new();

    let fpath = crate::ported::params::getaparam("fpath").unwrap_or_default();
    let memo = MEMO.get_or_init(|| Mutex::new((Vec::new(), HashMap::new())));
    let mut guard = match memo.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    if guard.0 != fpath {
        guard.0 = fpath.clone();
        guard.1.clear();
    }
    if let Some(hit) = guard.1.get(name) {
        return *hit;
    }
    let mut overridden = false;
    for dir in &fpath {
        let path = std::path::Path::new(dir).join(name);
        if path.is_file() {
            let stock = dir.contains("/share/zsh/")
                && std::path::Path::new(dir)
                    .file_name()
                    .map(|f| f == "functions")
                    .unwrap_or(false);
            overridden = !stock;
            break;
        }
    }
    guard.1.insert(name.to_string(), overridden);
    overridden
}

/// Dispatch compsys `_fn` `name` with `args`, honoring plugin overrides.
/// Precedence: a plugin-registered `CompFn` (ABI v4, `zmodload -R`) wins
/// over the built-in Rust port. Returns `Some(rc)` when either handled it,
/// `None` when neither does (caller falls through to shell autoload).
///
/// The plugin override intercepts even when `backend != Rust`: the user
/// installed it explicitly, so it should run regardless of the built-in
/// port toggle. The built-in port stays gated behind [`try_rust_dispatch`].
pub fn dispatch_compsys(name: &str, args: &[String]) -> Option<i32> {
    if let Some(rc) = crate::extensions::plugin_host::dispatch_compfn(name, args) {
        return Some(rc);
    }
    try_rust_dispatch(name).map(|f| f(args))
}

/// True if a plugin override (ABI v4) OR a built-in Rust port handles
/// `name`. Used where a caller must know whether the completer is
/// intercepted natively (e.g. to skip the shell-autoload prelude) without
/// invoking it.
pub fn is_intercepted(name: &str) -> bool {
    crate::extensions::plugin_host::compfn_override(name).is_some()
        || try_rust_dispatch(name).is_some()
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
        "_dns_types" => Some(_dns_types::_dns_types),
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
        "_my_accounts" => Some(_my_accounts::_my_accounts),
        "_next_label" => Some(_next_label::_next_label),
        "_normal" => Some(_normal::_normal),
        "_other_accounts" => Some(_other_accounts::_other_accounts),
        "_numbers" => Some(_numbers::_numbers),
        "_options" => Some(_options::_options),
        "_options_set" => Some(_options_set::_options_set),
        "_options_unset" => Some(_options_unset::_options_unset),
        "_parameters" => Some(_parameters::_parameters),
        "_path_commands" => Some(_path_commands::_path_commands),
        "_path_files" => Some(_path_files::_path_files),
        "_pdf" => Some(_pdf::_pdf),
        "_pick_variant" => Some(_pick_variant::_pick_variant),
        "_postscript" => Some(_postscript::_postscript),
        "_pspdf" => Some(_pspdf::_pspdf),
        "_ra_comp" => Some(_regex_arguments::_ra_comp),
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
        "_texi" => Some(_texi::_texi),
        "_arch_archives" => Some(_arch_archives::_arch_archives),
        "_arch_namespace" => Some(_arch_namespace::_arch_namespace),
        "_baudrates" => Some(_baudrates::_baudrates),
        "_bind_addresses" => Some(_bind_addresses::_bind_addresses),
        "_bpf_filters" => Some(_bpf_filters::_bpf_filters),
        "_ctags_tags" => Some(_ctags_tags::_ctags_tags),
        "_date_formats" => Some(_date_formats::_date_formats),
        "_dates" => Some(_dates::_dates),
        "_dict_words" => Some(_dict_words::_dict_words),
        "_diff_options" => Some(_diff_options::_diff_options),
        "_domains" => Some(_domains::_domains),
        "_file_modes" => Some(_file_modes::_file_modes),
        "_file_systems" => Some(_file_systems::_file_systems),
        "_find_net_interfaces" => Some(_find_net_interfaces::_find_net_interfaces),
        "_global_tags" => Some(_global_tags::_global_tags),
        "_groups" => Some(_groups::_groups),
        "_have_glob_qual" => Some(_have_glob_qual::_have_glob_qual),
        "_hosts" => Some(_hosts::_hosts),
        "_java_class" => Some(_java_class::_java_class),
        "_ld_debug" => Some(_ld_debug::_ld_debug),
        "_ldap_attributes" => Some(_ldap_attributes::_ldap_attributes),
        "_ldap_filters" => Some(_ldap_filters::_ldap_filters),
        "_list_files" => Some(_list_files::_list_files),
        "_locales" => Some(_locales::_locales),
        "_mailboxes" => Some(_mailboxes::_mailboxes),
        "_mime_types" => Some(_mime_types::_mime_types),
        "_net_interfaces" => Some(_net_interfaces::_net_interfaces),
        "_newsgroups" => Some(_newsgroups::_newsgroups),
        "_object_files" => Some(_object_files::_object_files),
        "_perl_basepods" => Some(_perl_basepods::_perl_basepods),
        "_perl_modules" => Some(_perl_modules::_perl_modules),
        "_perl_modules_caching_policy" => Some(_perl_modules::_perl_modules_caching_policy),
        "_pgids" => Some(_pgids::_pgids),
        "_pids" => Some(_pids::_pids),
        "_ports" => Some(_ports::_ports),
        "_printers" => Some(_printers::_printers),
        "_process_names" => Some(_process_names::_process_names),
        "_python_modules" => Some(_python_modules::_python_modules),
        "_remote_files" => Some(_remote_files::_remote_files),
        "_services" => Some(_services::_services),
        "_signals" => Some(_signals::_signals),
        "_ssh_hosts" => Some(_ssh_hosts::_ssh_hosts),
        "_sys_calls" => Some(_sys_calls::_sys_calls),
        "_terminals" => Some(_terminals::_terminals),
        "_time_zone" => Some(_time_zone::_time_zone),
        "_ttys" => Some(_ttys::_ttys),
        "_umountable" => Some(_umountable::_umountable),
        "_urls" => Some(_urls::_urls),
        "_user_at_host" => Some(_user_at_host::_user_at_host),
        "_users" => Some(_users::_users),
        "_users_on" => Some(_users_on::_users_on),
        "_zfs_dataset" => Some(_zfs_dataset::_zfs_dataset),
        "_zfs_pool" => Some(_zfs_pool::_zfs_pool),
        "_diff_palette" => Some(_diff_options::_diff_palette),
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
        "_expand" => Some(|args: &[String]| _expand::_expand_with(args)),
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
        "_logical_volumes" => Some(_logical_volumes::_logical_volumes),
        "_object_classes" => Some(_object_classes::_object_classes),
        "_physical_volumes" => Some(_physical_volumes::_physical_volumes),
        "_volume_groups" => Some(_volume_groups::_volume_groups),
        "_bsd_disks" => Some(_bsd_disks::_bsd_disks),
        "_fbsd_architectures" => Some(_fbsd_architectures::_fbsd_architectures),
        "_fbsd_device_types" => Some(_fbsd_device_types::_fbsd_device_types),
        "_file_flags" => Some(_file_flags::_file_flags),
        "_jails" => Some(_jails::_jails),
        "_ktrace_points" => Some(_ktrace_points::_ktrace_points),
        "_login_classes" => Some(_login_classes::_login_classes),
        "_nbsd_architectures" => Some(_nbsd_architectures::_nbsd_architectures),
        "_obsd_architectures" => Some(_obsd_architectures::_obsd_architectures),
        "_routing_domains" => Some(_routing_domains::_routing_domains),
        "_routing_tables" => Some(_routing_tables::_routing_tables),
        "_mac_applications" => Some(_mac_applications::_mac_applications),
        "_mac_files_for_application" => {
            Some(_mac_files_for_application::_mac_files_for_application)
        }
        "_deb_architectures" => Some(_deb_architectures::_deb_architectures),
        "_deb_codenames" => Some(_deb_codenames::_deb_codenames),
        "_debbugs_bugnumber" => Some(_debbugs_bugnumber::_debbugs_bugnumber),
        "_capabilities" => Some(_capabilities::_capabilities),
        "_fuse_arguments" => Some(_fuse_arguments::_fuse_arguments),
        "_fuse_values" => Some(_fuse_values::_fuse_values),
        "_selinux_roles" => Some(_selinux_roles::_selinux_roles),
        "_selinux_types" => Some(_selinux_types::_selinux_types),
        "_selinux_users" => Some(_selinux_users::_selinux_users),
        "_be_name" => Some(_be_name::_be_name),
        "_zones" => Some(_zones::_zones),
        "_x_borderwidth" => Some(_x_borderwidth::_x_borderwidth),
        "_retrieve_mac_apps" => Some(_retrieve_mac_apps::_retrieve_mac_apps),
        "_deb_files" => Some(_deb_files::_deb_files),
        "_deb_packages" => Some(_deb_packages::_deb_packages),
        "_selinux_contexts" => Some(_selinux_contexts::_selinux_contexts),
        "_wakeup_capable_devices" => Some(_wakeup_capable_devices::_wakeup_capable_devices),
        "_svcs_fmri" => Some(_svcs_fmri::_svcs_fmri),
        "_x_color" => Some(_x_color::_x_color),
        "_x_colormapid" => Some(_x_colormapid::_x_colormapid),
        "_x_cursor" => Some(_x_cursor::_x_cursor),
        "_x_display" => Some(_x_display::_x_display),
        "_x_extension" => Some(_x_extension::_x_extension),
        "_x_font" => Some(_x_font::_x_font),
        "_x_geometry" => Some(_x_geometry::_x_geometry),
        "_x_keysym" => Some(_x_keysym::_x_keysym),
        "_x_locale" => Some(_x_locale::_x_locale),
        "_x_modifier" => Some(_x_modifier::_x_modifier),
        "_x_name" => Some(_x_name::_x_name),
        "_x_resource" => Some(_x_resource::_x_resource),
        "_x_selection_timeout" => Some(_x_selection_timeout::_x_selection_timeout),
        "_x_title" => Some(_x_title::_x_title),
        "_x_visual" => Some(_x_visual::_x_visual),
        "_x_window" => Some(_x_window::_x_window),
        "_xft_fonts" => Some(_xft_fonts::_xft_fonts),
        "_xt_session_id" => Some(_xt_session_id::_xt_session_id),
        "_x_arguments" => Some(_x_arguments::_x_arguments),
        "_xt_arguments" => Some(_xt_arguments::_xt_arguments),
        "__arguments" => Some(__arguments::__arguments),
        "_add-zle-hook-widget" => Some(_add_zle_hook_widget::_add_zle_hook_widget),
        "_add-zsh-hook" => Some(_add_zsh_hook::_add_zsh_hook),
        "_vcs_info" => Some(_vcs_info::_vcs_info),
        "_zargs" => Some(_zargs::_zargs),
        "_zcalc" => Some(_zcalc::_zcalc),
        "_zsh-mime-handler" => Some(_zsh_mime_handler::_zsh_mime_handler),
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

    /// A Rust port replaces the STOCK function of that name only. When
    /// `$fpath` resolves the name to a user's or plugin's own file first,
    /// zsh autoloads THAT, so `try_rust_dispatch` must decline.
    ///
    /// Regression: his `~/.zpwr/autoload/comp_utils/_parameters` (parameter
    /// names WITH values) was shadowed by the port (bare names), so
    /// `unset <TAB>` listed 197 entries where zsh listed 496.
    #[test]
    fn user_fpath_file_beats_the_rust_port() {
        let _g = crate::test_util::global_state_lock();
        let base = std::env::temp_dir().join("zshrs_router_override_test");
        let user_dir = base.join("mine");
        let stock_dir = base.join("share/zsh/5.9/functions");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::create_dir_all(&stock_dir).unwrap();
        std::fs::write(stock_dir.join("_setup"), "#autoload\n").unwrap();

        let saved = crate::ported::params::getaparam("fpath").unwrap_or_default();

        // Stock dir only: the port answers.
        crate::ported::params::setaparam("fpath", vec![stock_dir.to_string_lossy().into_owned()]);
        assert!(try_rust_dispatch("_setup").is_some(), "stock file → port");

        // The user's own copy earlier in fpath: the port steps aside.
        std::fs::write(user_dir.join("_setup"), "#autoload\n# mine\n").unwrap();
        crate::ported::params::setaparam(
            "fpath",
            vec![
                user_dir.to_string_lossy().into_owned(),
                stock_dir.to_string_lossy().into_owned(),
            ],
        );
        assert!(
            try_rust_dispatch("_setup").is_none(),
            "user file earlier in fpath must win"
        );

        crate::ported::params::setaparam("fpath", saved);
        let _ = std::fs::remove_dir_all(&base);
    }
}
