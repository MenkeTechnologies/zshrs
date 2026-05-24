//! Port of `_external_pwds` — complete from other shells' PWDs.
//!
//! Local shell reference:
//! `compsys/functions/Base/Completer/_external_pwds`
//! (system copy `/opt/homebrew/share/zsh/functions/_external_pwds`).
//!
//! Upstream shell source (key lines):
//! ```text
//! 16  case $OSTYPE in
//! 17    solaris*) dirs=( /proc/*(N:A) /proc/*/path/cwd(N:A) ) ;;
//! 18    linux*)   dirs=( /proc/[0-9]*/cwd(N:A) ) ;;
//! 19    *)        dirs=( ) ;;
//! 20  esac
//! 22  compadd -V cwd -a dirs
//! ```
//!
//! Upstream walks `/proc/*/cwd` (or Solaris-equivalent) to discover
//! directories that OTHER shell processes are currently in, so the
//! user can `cd` directly to peer shells' PWDs.
//!
//! Simplified Rust port: emits only the CURRENT process's cwd as a
//! candidate. Walking `/proc/*/cwd` requires permission to read
//! other-uid procfs entries which is restricted on hardened Linux
//! and unavailable on macOS — left as a runtime-side feature.

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
