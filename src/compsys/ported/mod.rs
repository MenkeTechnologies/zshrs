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
///
/// Every scratch name is declared LOCAL first. Upstream has no counterpart
/// to declare against — the name it evaluates into is its own function-local
/// `action` — so the requirement is not "match an `sh:NN local` line" but
/// "do not create a global". Under `WARN_CREATE_GLOBAL` (a completer may set
/// it for its own body: `_mc:36` `setopt localoptions warncreateglobal
/// typesetsilent`) an undeclared assignment prints `_arguments:411: scalar
/// parameter _cs_split_src created globally in function _arguments` per call,
/// which lands on the terminal instead of the match list. The names still have
/// to exist in `paramtab`: the `eval` reads `$_cs_split_src` by name, and that
/// indirection IS the semantics being ported — zsh expands `$action` and
/// re-parses the RESULT, so splicing the text into the eval string instead
/// would parse it one time too few.
pub fn eval_action_words(action: &str) -> Vec<String> {
    eval_action_words_status(action).unwrap_or_default()
}

/// [`eval_action_words`], but reporting whether the `eval` SUCCEEDED.
///
/// Upstream evaluates into `action` ITSELF (`eval "action=( $action )"`), so
/// the eval's outcome is observable on the very next line. When the action
/// text is not a parseable word list the eval is a PARSE ERROR, nothing is
/// assigned, and `action` KEEPS its scalar value — after which `$action[1]`
/// subscripts a SCALAR and yields its first CHARACTER, and `${(@)action[2,-1]}`
/// the rest of the string. zsh therefore runs a one-character command name.
/// Measured, zsh 5.9 and this shell agreeing on the scalar semantics:
///
/// ```text
/// action='app or factory:fn; env: UVICORN_APP):'
/// eval "action=( $action )"   → (eval):1: parse error near `)'  rc=1
/// $action[1]                  → a
/// ${(@)action[2,-1]}          → pp or factory:fn; env: UVICORN_APP):
/// ```
///
/// which is how `uvicorn <TAB>` reaches `_arguments:465: command not found: a`
/// — `_uvicorn`'s last spec is
/// `1:ASGI app import path (e.g. mymodule:app or factory:fn; env: UVICORN_APP):`
/// and the unescaped colons in that MESSAGE put `app or factory:fn; env:
/// UVICORN_APP):` in the ACTION field.
///
/// Evaluating into a Rust-side scratch array instead loses that, because a
/// failed eval and an empty action produce the same empty array. Callers that
/// subscript `action` after the eval need the distinction, so they get it here:
///
///   * `Ok(words)`  — the eval assigned; `action` is now an ARRAY of `words`.
///   * `Err(text)`  — the eval FAILED; `action` is still the SCALAR `text`.
pub fn eval_action_words_status(action: &str) -> Result<Vec<String>, String> {
    crate::compsys::ported::shared::declare_locals(&["_cs_split_src", "_cs_split_rc"], 0);
    crate::compsys::ported::shared::declare_locals(
        &["_cs_split_dst"],
        crate::compsys::ported::shared::PM_ARRAY,
    );
    let _ = crate::ported::params::setsparam("_cs_split_src", action);
    let _ = crate::ported::exec::execute_script(
        "eval \"_cs_split_dst=( $_cs_split_src )\"\n_cs_split_rc=$?\n",
    );
    let out = crate::ported::params::getaparam("_cs_split_dst").unwrap_or_default();
    let rc = crate::ported::params::getsparam("_cs_split_rc").unwrap_or_default();
    let _ = crate::ported::params::unsetparam("_cs_split_src");
    let _ = crate::ported::params::unsetparam("_cs_split_dst");
    let _ = crate::ported::params::unsetparam("_cs_split_rc");
    if rc == "0" {
        Ok(out)
    } else {
        Err(action.to_string())
    }
}

/// The `"$action[1]" … "${(@)action[2,-1]}"` of `_arguments` sh:465 applied to
/// an action whose `eval` FAILED, i.e. to a SCALAR: character 1 is the command
/// word and characters 2..-1 are the single remaining word. Empty for an empty
/// scalar, which has no character 1 to run.
pub fn scalar_action_call(text: &str) -> Vec<String> {
    let mut it = text.chars();
    let Some(first) = it.next() else {
        return Vec::new();
    };
    // `"${(@)action[2,-1]}"` on a one-character scalar is one EMPTY word, not
    // no word — the double quotes keep it.
    vec![first.to_string(), it.collect()]
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
/// `_arch_archives` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_arch_archives.rs"]
pub mod _arch_archives;
/// `_arch_namespace` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_arch_namespace.rs"]
pub mod _arch_namespace;
/// `_baudrates` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_baudrates.rs"]
pub mod _baudrates;
/// `_bind_addresses` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_bind_addresses.rs"]
pub mod _bind_addresses;
/// `_bpf_filters` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_bpf_filters.rs"]
pub mod _bpf_filters;
/// `_canonical_paths` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_canonical_paths.rs"]
pub mod _canonical_paths;
/// `_command_names` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Type/_command_names.rs"]
pub mod _command_names;
/// `_ctags_tags` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_ctags_tags.rs"]
pub mod _ctags_tags;
/// `_date_formats` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_date_formats.rs"]
pub mod _date_formats;
/// `_dates` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_dates.rs"]
pub mod _dates;
/// `_dict_words` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_dict_words.rs"]
pub mod _dict_words;
/// `_diff_options` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_diff_options.rs"]
pub mod _diff_options;
/// `_dir_list` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_dir_list.rs"]
pub mod _dir_list;
/// `_directories` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_directories.rs"]
pub mod _directories;
/// `_dns_types` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_dns_types.rs"]
pub mod _dns_types;
/// `_domains` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_domains.rs"]
pub mod _domains;
/// `_email_addresses` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_email_addresses.rs"]
pub mod _email_addresses;
/// `_file_modes` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_file_modes.rs"]
pub mod _file_modes;
/// `_file_systems` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_file_systems.rs"]
pub mod _file_systems;
/// `_files` submodule.
#[path = "Unix/Type/_files.rs"]
pub mod _files;
/// `_find_net_interfaces` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_find_net_interfaces.rs"]
pub mod _find_net_interfaces;
/// `_global_tags` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_global_tags.rs"]
pub mod _global_tags;
/// `_gnu_generic` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Command/_gnu_generic.rs"]
pub mod _gnu_generic;
/// `_groups` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_groups.rs"]
pub mod _groups;
/// `_have_glob_qual` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_have_glob_qual.rs"]
pub mod _have_glob_qual;
/// `_hosts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_hosts.rs"]
pub mod _hosts;
/// `_java_class` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_java_class.rs"]
pub mod _java_class;
/// `_ld_debug` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_ld_debug.rs"]
pub mod _ld_debug;
/// `_ldap_attributes` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_ldap_attributes.rs"]
pub mod _ldap_attributes;
/// `_ldap_filters` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_ldap_filters.rs"]
pub mod _ldap_filters;
/// `_list_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_list_files.rs"]
pub mod _list_files;
/// `_locales` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_locales.rs"]
pub mod _locales;
/// `_mailboxes` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_mailboxes.rs"]
pub mod _mailboxes;
/// `_mime_types` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_mime_types.rs"]
pub mod _mime_types;
/// `_my_accounts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_my_accounts.rs"]
pub mod _my_accounts;
/// `_net_interfaces` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_net_interfaces.rs"]
pub mod _net_interfaces;
/// `_newsgroups` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_newsgroups.rs"]
pub mod _newsgroups;
/// `_object_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_object_files.rs"]
pub mod _object_files;
/// `_other_accounts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_other_accounts.rs"]
pub mod _other_accounts;
/// `_path_commands` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_path_commands.rs"]
pub mod _path_commands;
/// `_path_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_path_files.rs"]
pub mod _path_files;
/// `_pdf` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_pdf.rs"]
pub mod _pdf;
/// `_perl_basepods` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_perl_basepods.rs"]
pub mod _perl_basepods;
/// `_perl_modules` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_perl_modules.rs"]
pub mod _perl_modules;
/// `_pgids` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_pgids.rs"]
pub mod _pgids;
/// `_pids` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_pids.rs"]
pub mod _pids;
/// `_ports` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_ports.rs"]
pub mod _ports;
/// `_postscript` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_postscript.rs"]
pub mod _postscript;
/// `_precommand` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Command/_precommand.rs"]
pub mod _precommand;
/// `_printers` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_printers.rs"]
pub mod _printers;
/// `_process_names` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_process_names.rs"]
pub mod _process_names;
/// `_pspdf` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_pspdf.rs"]
pub mod _pspdf;
/// `_python_modules` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_python_modules.rs"]
pub mod _python_modules;
/// `_remote_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_remote_files.rs"]
pub mod _remote_files;
/// `_services` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_services.rs"]
pub mod _services;
/// `_signals` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_signals.rs"]
pub mod _signals;
/// `_ssh_hosts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_ssh_hosts.rs"]
pub mod _ssh_hosts;
/// `_sys_calls` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_sys_calls.rs"]
pub mod _sys_calls;
/// `_terminals` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_terminals.rs"]
pub mod _terminals;
/// `_texi` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_texi.rs"]
pub mod _texi;
/// `_tilde_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_tilde_files.rs"]
pub mod _tilde_files;
/// `_time_zone` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_time_zone.rs"]
pub mod _time_zone;
/// `_ttys` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_ttys.rs"]
pub mod _ttys;
/// `_umountable` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_umountable.rs"]
pub mod _umountable;
/// `_urls` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_urls.rs"]
pub mod _urls;
/// `_user_at_host` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_user_at_host.rs"]
pub mod _user_at_host;
/// `_users` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_users.rs"]
pub mod _users;
/// `_users_on` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_users_on.rs"]
pub mod _users_on;
/// `_zfs_dataset` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_zfs_dataset.rs"]
pub mod _zfs_dataset;
/// `_zfs_pool` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Unix/Type/_zfs_pool.rs"]
pub mod _zfs_pool;

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
// ── Non-leaf ports (OS-specific Type) batch 1 ──
/// `_be_name` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Solaris/Type/_be_name.rs"]
pub mod _be_name;
/// `_bsd_disks` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_bsd_disks.rs"]
pub mod _bsd_disks;
/// `_capabilities` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_capabilities.rs"]
pub mod _capabilities;
/// `_deb_architectures` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Debian/Type/_deb_architectures.rs"]
pub mod _deb_architectures;
/// `_deb_codenames` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Debian/Type/_deb_codenames.rs"]
pub mod _deb_codenames;
/// `_debbugs_bugnumber` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Debian/Type/_debbugs_bugnumber.rs"]
pub mod _debbugs_bugnumber;
/// `_fbsd_architectures` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_fbsd_architectures.rs"]
pub mod _fbsd_architectures;
/// `_fbsd_device_types` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_fbsd_device_types.rs"]
pub mod _fbsd_device_types;
/// `_file_flags` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_file_flags.rs"]
pub mod _file_flags;
/// `_fuse_arguments` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_fuse_arguments.rs"]
pub mod _fuse_arguments;
/// `_fuse_values` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_fuse_values.rs"]
pub mod _fuse_values;
/// `_jails` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_jails.rs"]
pub mod _jails;
/// `_ktrace_points` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_ktrace_points.rs"]
pub mod _ktrace_points;
/// `_logical_volumes` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "AIX/Type/_logical_volumes.rs"]
pub mod _logical_volumes;
/// `_login_classes` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_login_classes.rs"]
pub mod _login_classes;
/// `_mac_applications` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Darwin/Type/_mac_applications.rs"]
pub mod _mac_applications;
/// `_mac_files_for_application` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Darwin/Type/_mac_files_for_application.rs"]
pub mod _mac_files_for_application;
/// `_nbsd_architectures` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_nbsd_architectures.rs"]
pub mod _nbsd_architectures;
/// `_object_classes` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "AIX/Type/_object_classes.rs"]
pub mod _object_classes;
/// `_obsd_architectures` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_obsd_architectures.rs"]
pub mod _obsd_architectures;
/// `_physical_volumes` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "AIX/Type/_physical_volumes.rs"]
pub mod _physical_volumes;
/// `_routing_domains` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_routing_domains.rs"]
pub mod _routing_domains;
/// `_routing_tables` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "BSD/Type/_routing_tables.rs"]
pub mod _routing_tables;
/// `_selinux_roles` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_selinux_roles.rs"]
pub mod _selinux_roles;
/// `_selinux_types` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_selinux_types.rs"]
pub mod _selinux_types;
/// `_selinux_users` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_selinux_users.rs"]
pub mod _selinux_users;
/// `_volume_groups` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "AIX/Type/_volume_groups.rs"]
pub mod _volume_groups;
/// `_x_borderwidth` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_borderwidth.rs"]
pub mod _x_borderwidth;
/// `_zones` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Solaris/Type/_zones.rs"]
pub mod _zones;
// ── Non-leaf ports (X + Zsh/Function + stragglers) batch 2 ──
/// `__arguments` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Function/__arguments.rs"]
pub mod __arguments;
/// `_add-zle-hook-widget` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Function/_add-zle-hook-widget.rs"]
pub mod _add_zle_hook_widget;
/// `_add-zsh-hook` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Function/_add-zsh-hook.rs"]
pub mod _add_zsh_hook;
/// `_deb_files` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Debian/Type/_deb_files.rs"]
pub mod _deb_files;
/// `_deb_packages` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Debian/Type/_deb_packages.rs"]
pub mod _deb_packages;
/// `_retrieve_mac_apps` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Darwin/Type/_retrieve_mac_apps.rs"]
pub mod _retrieve_mac_apps;
/// `_selinux_contexts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_selinux_contexts.rs"]
pub mod _selinux_contexts;
/// `_svcs_fmri` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Solaris/Type/_svcs_fmri.rs"]
pub mod _svcs_fmri;
/// `_vcs_info` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Function/_vcs_info.rs"]
pub mod _vcs_info;
/// `_wakeup_capable_devices` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Linux/Type/_wakeup_capable_devices.rs"]
pub mod _wakeup_capable_devices;
/// `_x_arguments` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Utility/_x_arguments.rs"]
pub mod _x_arguments;
/// `_x_color` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_color.rs"]
pub mod _x_color;
/// `_x_colormapid` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_colormapid.rs"]
pub mod _x_colormapid;
/// `_x_cursor` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_cursor.rs"]
pub mod _x_cursor;
/// `_x_display` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_display.rs"]
pub mod _x_display;
/// `_x_extension` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_extension.rs"]
pub mod _x_extension;
/// `_x_font` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_font.rs"]
pub mod _x_font;
/// `_x_geometry` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_geometry.rs"]
pub mod _x_geometry;
/// `_x_keysym` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_keysym.rs"]
pub mod _x_keysym;
/// `_x_locale` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_locale.rs"]
pub mod _x_locale;
/// `_x_modifier` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_modifier.rs"]
pub mod _x_modifier;
/// `_x_name` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_name.rs"]
pub mod _x_name;
/// `_x_resource` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_resource.rs"]
pub mod _x_resource;
/// `_x_selection_timeout` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_selection_timeout.rs"]
pub mod _x_selection_timeout;
/// `_x_title` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_title.rs"]
pub mod _x_title;
/// `_x_visual` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_visual.rs"]
pub mod _x_visual;
/// `_x_window` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_x_window.rs"]
pub mod _x_window;
/// `_xft_fonts` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_xft_fonts.rs"]
pub mod _xft_fonts;
/// `_xt_arguments` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Utility/_xt_arguments.rs"]
pub mod _xt_arguments;
/// `_xt_session_id` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "X/Type/_xt_session_id.rs"]
pub mod _xt_session_id;
/// `_zargs` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Function/_zargs.rs"]
pub mod _zargs;
/// `_zcalc` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Function/_zcalc.rs"]
pub mod _zcalc;
/// `_zsh-mime-handler` submodule.
#[allow(non_snake_case, non_camel_case_types)]
#[path = "Zsh/Function/_zsh-mime-handler.rs"]
pub mod _zsh_mime_handler;
// ── Public re-exports ─────────────────────────────────────────────────
// Items with richer export shapes (Opts structs, consts, etc.):

// Zsh/Context

// Unix/Type — newly-added

// Parameters opts (re-export so wrappers in other crates can build
// the same flag-set the shell `_parameters` accepts).

// Simple one-symbol-per-module re-exports.
pub use shared::{get_ignored_patterns, is_ignored};
