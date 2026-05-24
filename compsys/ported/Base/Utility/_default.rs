//! Port of `_default` — default completion (files). Moved from
//! `compsys/library.rs`. Renamed from `default_complete` to mirror zsh
//! shell function name `_default`.

use crate::compcore::CompletionState;

use super::_files::{files_execute, FilesOpts};

/// _default - Default completion (files)
pub fn _default(state: &mut CompletionState) -> bool {
    files_execute(state, &FilesOpts::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delegates_to_files_execute_default() {
        // Pin: _default IS files_execute(FilesOpts::default()).
        // Whether matches return depends on test cwd contents —
        // we only assert the call doesn't panic and exercises the
        // delegation path.
        let mut state = CompletionState::new();
        state.params.prefix = "Carg".into();
        let _ = _default(&mut state);
    }
}
