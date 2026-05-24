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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_current_directory_as_pwd_candidate() {
        let mut state = CompletionState::new();
        assert!(_external_pwds(&mut state));
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            names.contains(&cwd),
            "current dir must appear as a PWD candidate; got {names:?}"
        );
    }
}
