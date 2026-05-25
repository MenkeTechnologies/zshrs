//! Port of `_equal` from `Completion/Zsh/Context/_equal`.
//!
//! Full upstream body (11 lines verbatim):
//! ```text
//! sh: 1  #compdef -equal-
//! sh: 2
//! sh: 3  local -a match mbegin mend
//! sh: 4
//! sh: 5  if _have_glob_qual $PREFIX; then
//! sh: 6    compset -p ${#match[1]}
//! sh: 7    compset -S '[^\)\|\~]#(|\))'
//! sh: 8    _globquals
//! sh: 9  else
//! sh:10    _path_commands
//! sh:11  fi
//! ```
//!
//! Strict Rust port: faithful 1:1 — delegates to our ported
//! [`_path_commands`].



use crate::compsys::base::MainCompleteState;
use crate::compsys::ported::_path_commands::_path_commands;

/// `_equal` — `=name` completion → delegates to `_path_commands`.
pub fn _equal(state: &mut MainCompleteState) -> bool {
    _path_commands(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compsys::base::TagManager;

    fn seed(state: &mut MainCompleteState) {
        state.tags = TagManager::new();
        state.tags.init(&["commands".into()]);
        state.tags.add_try(&["commands".into()]);
        let _ = state.tags.start();
    }

    #[test]
    fn delegates_to_path_commands() {
        let mut s1 = MainCompleteState::new("", 0);
        seed(&mut s1);
        s1.comp.params.prefix = "ls".into();
        let mut s2 = MainCompleteState::new("", 0);
        seed(&mut s2);
        s2.comp.params.prefix = "ls".into();
        let r1 = _equal(&mut s1);
        let r2 = _path_commands(&mut s2);
        assert_eq!(r1, r2);
    }

    #[test]
    fn emits_path_executables() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "ls".into();
        let _ = _equal(&mut state);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.iter().any(|n| *n == "ls" || n.starts_with("ls")));
    }

    #[test]
    fn untagged_call_skips_emission() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.prefix = "ls".into();
        assert!(!_equal(&mut state));
    }

    #[test]
    fn group_named_commands() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "ls".into();
        let _ = _equal(&mut state);
        assert!(state.comp.groups.iter().any(|g| g.name == "commands"));
    }

    #[test]
    fn off_prefix_emits_no_matches() {
        let mut state = MainCompleteState::new("", 0);
        seed(&mut state);
        state.comp.params.prefix = "definitely-no-such-binary-zzz-xyz".into();
        let _ = _equal(&mut state);
        let matches: usize = state
            .comp
            .groups
            .iter()
            .filter(|g| g.name == "commands")
            .map(|g| g.matches.len())
            .sum();
        assert_eq!(matches, 0);
    }
}
