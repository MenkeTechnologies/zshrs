//! Back-compat shim — every public function that used to live in
//! `compsys/functions.rs` has been split into a per-file module under
//! `compsys/fns/_<NAME>.rs`, mirroring zsh's one-file-per-function
//! layout (`/opt/homebrew/share/zsh/functions/_<NAME>`).
//!
//! This file now ONLY re-exports the moved symbols so existing
//! `compsys::functions::*` (and same-crate `crate::functions::*`)
//! call sites keep working. New code should call
//! `crate::fns::_<NAME>` directly.
//!
//! Where the old name lacked a leading `_` (e.g. `setup`, `dispatch`,
//! `wanted`, `call_function`, `as_if`, `comp_locale`,
//! `complete_help_generic`, `arg_compile`, `set_command`, `correct`,
//! `expand`, `read_comp`) the per-fn file uses the canonical
//! `_NAME` form and a back-compat alias is exported here under the
//! old short name.
//!
//! `edit_distance` and `glob_match` are not shell functions — they
//! live in [`crate::fns::shared`] but are re-exported here for
//! callers of the old `functions::` paths.
//!
//! `_numbers`, `_pick_variant`, `_sequence`, `_combination`,
//! `_canonical_paths`, `_dir_list`, `_email_addresses`, `_call_program`
//! had OLDER stubs here. Per the per-fn layout the proper ports live
//! under `crate::fns::` and the stubs are gone. The re-exports below
//! point at the proper ports. Note that some signatures changed (e.g.
//! `_numbers` now takes `NumbersOpts`).

pub use crate::fns::shared::{edit_distance, glob_match};

pub use crate::fns::{
    CallProgramOpts, CallProgramResult, CanonicalPathsOpts, CombinationOpts, CompiledArgSpec,
    DirListOpts, EmailAddressesOpts, MultiPartsOpts, NumbersOpts, PickVariantOpts,
    PickVariantResult, SequenceOpts, TagSortHook, TagsOpts, _all_matches, _approximate,
    _arg_compile, _as_if, _bash_completions, _cache_invalid, _call_function, _call_program,
    _canonical_paths, _combination, _combination_mcs, _comp_locale, _complete_debug,
    _complete_help, _complete_help_generic, _complete_tag, _correct, _correct_filename,
    _correct_word, _dir_list, _dispatch, _email_addresses, _expand, _expand_alias, _expand_word,
    _extensions, _external_pwds, _generic, _guard, _history, _history_complete_word, _ignored,
    _list, _match, _menu, _most_recent_file, _multi_parts, _next_tags, _nothing, _numbers,
    _oldlist, _pick_variant, _prefix, _read_comp, _regex_arguments, _regex_words, _retrieve_cache,
    _sequence, _set_command, _setup, _shadow, _store_cache, _sub_commands, _tags, _tags_mcs,
    _tags_next, _user_expand, _wanted,
};

/// Back-compat aliases for historical no-underscore call sites inside
/// this crate. Prefer the `_NAME` forms directly to mirror zsh.
pub use crate::fns::{
    _arg_compile as arg_compile, _as_if as as_if, _call_function as call_function,
    _comp_locale as comp_locale, _complete_help_generic as complete_help_generic,
    _correct as correct, _dispatch as dispatch, _expand as expand, _read_comp as read_comp,
    _set_command as set_command, _setup as setup, _wanted as wanted,
};
