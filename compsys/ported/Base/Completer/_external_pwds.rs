//! Port of `_external_pwds` — complete from other shells' PWDs. Moved
//! from `compsys/functions.rs`.

use crate::compcore::CompletionState;
use crate::completion::Completion;

/// _external_pwds - Complete from other shell's PWDs
pub fn _external_pwds(state: &mut CompletionState) -> bool {
    // Would read from /proc/*/cwd or similar
    // Simplified: just add current directory
    if let Ok(pwd) = std::env::current_dir() {
        state.add_match(Completion::new(pwd.to_string_lossy().to_string()), None);
        true
    } else {
        false
    }
}
