//! Port of `_default` — default completion (files). Moved from
//! `compsys/library.rs`. Renamed from `default_complete` to mirror zsh
//! shell function name `_default`.

use crate::compcore::CompletionState;

use super::_files::{files_execute, FilesOpts};

/// _default - Default completion (files)
pub fn _default(state: &mut CompletionState) -> bool {
    files_execute(state, &FilesOpts::default())
}
