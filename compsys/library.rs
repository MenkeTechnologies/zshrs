//! Back-compat shim — every public function that used to live in
//! `compsys/library.rs` has been split into a per-file module under
//! `compsys/fns/_<NAME>.rs`, mirroring zsh's one-file-per-function
//! layout (`/opt/homebrew/share/zsh/functions/_<NAME>`).
//!
//! This file now ONLY re-exports the moved symbols so existing
//! `compsys::library::*` (and same-crate `crate::library::*`) call
//! sites keep working. New code should call `crate::fns::_<NAME>`
//! directly.
//!
//! Mapping (old name → new module):
//!
//! | Old | New module |
//! |-----|------------|
//! | `_absolute_command_paths`  | [`crate::fns::_absolute_command_paths`] |
//! | `_cmdambivalent`           | [`crate::fns::_cmdambivalent`]          |
//! | `_cmdstring`               | [`crate::fns::_cmdstring`]              |
//! | `_comp_caller_options`     | [`crate::fns::_comp_caller_options`]    |
//! | `_comp_priv_prefix`        | [`crate::fns::_comp_priv_prefix`]       |
//! | `_default`                 | [`crate::fns::_default`]                |
//! | `_gnu_generic`             | [`crate::fns::_gnu_generic`]            |
//! | `_options` / `_set` / `_unset` | [`crate::fns::_options`] etc.      |
//! | `_parameters`              | [`crate::fns::_parameters`]             |
//! | `_path_files`              | [`crate::fns::_path_files`]             |
//! | `_precommand`              | [`crate::fns::_precommand`]             |
//! | `_tilde_files`             | [`crate::fns::_tilde_files`]            |
//! | `_command_names` / `_completers` / `_widgets` / `_dir_list` / `_canonical_paths` / `_email_addresses` | already lived in `fns/` — re-exported below |

pub use crate::fns::{
    CANONICAL_COMPLETER_NAMES, PathFilesOpts, ShellInventory, _absolute_command_paths,
    _canonical_paths, _cmdambivalent, _cmdstring, _command_names, _command_names_with_ctx,
    _comp_caller_options, _comp_priv_prefix, _completers, _default, _dir_list, _email_addresses,
    _gnu_generic, _options, _options_set, _options_unset, _parameters, _path_files, _precommand,
    _tilde_files, _widgets,
};

/// Back-compat aliases for the historical no-underscore call sites
/// inside this crate. Prefer the `_NAME` forms directly to mirror zsh.
pub use crate::fns::{
    _command_names as command_names, _command_names_with_ctx as command_names_with_ctx,
    _completers as completers, _widgets as widgets,
};
