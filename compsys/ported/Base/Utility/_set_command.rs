//! Port of `_set_command` — set the command being completed.
//!
//! Local shell reference: `compsys/functions/Base/Utility/_set_command`
//! (system copy `/opt/homebrew/share/zsh/functions/_set_command`).
//!
//! Upstream shell source (key lines):
//! ```text
//!  3  # Sets parameters _comp_command1, _comp_command2 and _comp_command
//!  9  command="$words[1]"
//! 13  if (( $+builtins[$command] + $+functions[$command] )); then
//! 14    _comp_command1="$command"
//! 15    _comp_command="$_comp_command1"
//! 16  elif [[ "$command[1]" = '=' ]]; then
//! 17    eval _comp_command2\=$command
//! ```
//!
//! Upstream sets THREE parameters (_comp_command1/2, _comp_command)
//! with a 5-way branch on the shape of `$words[1]`:
//!   1. builtin / function    → 1=name,    2=unset, dispatch=1
//!   2. `=name`               → 1=name[2,-1] (stripped), 2=abs path
//!                              after `=` eval, dispatch=2
//!   3. `..#/...` (no /-only) → 1=$PWD/cmd, 2=basename, dispatch=2
//!   4. `*/*` (path)          → 1=cmd,      2=basename, dispatch=2
//!   5. else (PATH lookup)    → 1=cmd,      2=$commands[cmd],
//!                              dispatch=1
//!
//! Strict Rust port: stores all three under `lastcomp["_comp_command1"]`,
//! `lastcomp["_comp_command2"]`, `lastcomp["_comp_command"]` keys.
//! Builtin/function detection consults the inventory passed in by the
//! caller (compsys can't reach the parent crate's builtin/fn table).

use crate::base::MainCompleteState;

/// Inventory hooks used by `_set_command` to classify a command name.
/// Caller supplies these so the leaf crate doesn't reach across into
/// the host shell's registries directly.
pub struct CommandClassifier<'a> {
    /// `$+builtins[$name]` — true iff `name` is a shell builtin.
    pub is_builtin: &'a dyn Fn(&str) -> bool,
    /// `$+functions[$name]` — true iff `name` is a shell function.
    pub is_function: &'a dyn Fn(&str) -> bool,
    /// `$commands[$name]` — full PATH lookup, returning the absolute
    /// path if `name` is an external executable found in $PATH (else
    /// empty string). Used for the case-5 fallback that wants the
    /// resolved path as `_comp_command2`.
    pub command_path: &'a dyn Fn(&str) -> String,
}

impl<'a> CommandClassifier<'a> {
    /// Sentinel classifier that treats nothing as builtin/function and
    /// resolves no PATH entries. Useful in tests + as the default for
    /// callers that don't yet have an inventory.
    pub const fn null() -> Self {
        Self {
            is_builtin: &|_: &str| false,
            is_function: &|_: &str| false,
            command_path: &|_: &str| String::new(),
        }
    }
}

/// _set_command - Set the command being completed.
pub fn _set_command(state: &mut MainCompleteState, cx: &CommandClassifier<'_>) {
    // shell:9-10 — `command="$words[1]"; [[ -z "$command" ]] && return`
    let command = match state.comp.params.words.first().cloned() {
        Some(c) if !c.is_empty() => c,
        _ => return,
    };

    let (c1, c2, dispatch_uses_c1): (String, String, bool) = if (cx.is_builtin)(&command)
        || (cx.is_function)(&command)
    {
        // shell:13-15
        (command.clone(), String::new(), true)
    } else if let Some(rest) = command.strip_prefix('=') {
        // shell:16-19. eval `_comp_command2=$command` runs the shell
        // `=foo` expansion which substitutes the full path of `foo`.
        // At our layer we approximate via PATH lookup.
        let abs = (cx.command_path)(rest);
        (
            rest.to_string(),
            if abs.is_empty() { rest.to_string() } else { abs },
            false,
        )
    } else if command.starts_with("..") && command.contains('/') {
        // shell:20-23 — `..#/*` pattern: relative path with `..`.
        let pwd = std::env::current_dir()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let abs = if pwd.is_empty() {
            command.clone()
        } else {
            format!("{}/{}", pwd, command)
        };
        let base = basename(&command);
        (abs, base, false)
    } else if command.contains('/') {
        // shell:24-27 — absolute / relative-with-slash path.
        let base = basename(&command);
        (command.clone(), base, false)
    } else {
        // shell:28-32 — bareword: try PATH.
        let path = (cx.command_path)(&command);
        (command.clone(), path, true)
    };

    let dispatch = if dispatch_uses_c1 { c1.clone() } else { c2.clone() };

    state.lastcomp.insert("_comp_command1".into(), c1);
    state.lastcomp.insert("_comp_command2".into(), c2);
    state.lastcomp.insert("_comp_command".into(), dispatch.clone());
    // Legacy key kept for callers that still ask for "command".
    state.lastcomp.insert("command".into(), dispatch);
}

/// `${command:t}` — strip everything up to the last `/`. Empty if
/// the path ends in `/` (matches zsh modifier behavior).
fn basename(p: &str) -> String {
    match p.rfind('/') {
        Some(i) => p[i + 1..].to_string(),
        None => p.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn null_cx() -> CommandClassifier<'static> {
        CommandClassifier::null()
    }

    #[test]
    fn empty_words_does_not_insert() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.words.clear();
        _set_command(&mut state, &null_cx());
        assert!(state.lastcomp.get("_comp_command").is_none());
        assert!(state.lastcomp.get("command").is_none());
    }

    #[test]
    fn empty_first_word_does_not_insert() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.words = vec!["".into()];
        _set_command(&mut state, &null_cx());
        assert!(state.lastcomp.get("_comp_command").is_none());
    }

    #[test]
    fn builtin_branch_dispatches_via_command1() {
        // shell:13-15 — builtin → c1=name, c2 unset, dispatch=c1
        let mut state = MainCompleteState::new("cd /tmp", 7);
        state.comp.params.words = vec!["cd".into(), "/tmp".into()];
        let cx = CommandClassifier {
            is_builtin: &|n| n == "cd",
            is_function: &|_| false,
            command_path: &|_| String::new(),
        };
        _set_command(&mut state, &cx);
        assert_eq!(state.lastcomp.get("_comp_command1").unwrap(), "cd");
        assert_eq!(state.lastcomp.get("_comp_command2").unwrap(), "");
        assert_eq!(state.lastcomp.get("_comp_command").unwrap(), "cd");
    }

    #[test]
    fn function_branch_dispatches_via_command1() {
        let mut state = MainCompleteState::new("myfn x", 6);
        state.comp.params.words = vec!["myfn".into(), "x".into()];
        let cx = CommandClassifier {
            is_builtin: &|_| false,
            is_function: &|n| n == "myfn",
            command_path: &|_| String::new(),
        };
        _set_command(&mut state, &cx);
        assert_eq!(state.lastcomp.get("_comp_command").unwrap(), "myfn");
    }

    #[test]
    fn equals_form_strips_prefix_and_dispatches_via_command2() {
        // shell:16-19 — `=ls` form
        let mut state = MainCompleteState::new("=ls", 3);
        state.comp.params.words = vec!["=ls".into()];
        let cx = CommandClassifier {
            is_builtin: &|_| false,
            is_function: &|_| false,
            command_path: &|n| {
                if n == "ls" {
                    "/bin/ls".into()
                } else {
                    String::new()
                }
            },
        };
        _set_command(&mut state, &cx);
        assert_eq!(state.lastcomp.get("_comp_command1").unwrap(), "ls");
        assert_eq!(state.lastcomp.get("_comp_command2").unwrap(), "/bin/ls");
        assert_eq!(state.lastcomp.get("_comp_command").unwrap(), "/bin/ls");
    }

    #[test]
    fn equals_form_with_unresolved_path_falls_back_to_name() {
        let mut state = MainCompleteState::new("=nonexistent", 12);
        state.comp.params.words = vec!["=nonexistent".into()];
        _set_command(&mut state, &null_cx());
        // Both c1 and c2 collapse to the stripped name, dispatch=c2.
        assert_eq!(state.lastcomp.get("_comp_command1").unwrap(), "nonexistent");
        assert_eq!(state.lastcomp.get("_comp_command2").unwrap(), "nonexistent");
    }

    #[test]
    fn slash_path_uses_basename_as_command2() {
        // shell:24-27 — `*/*` branch
        let mut state = MainCompleteState::new("/usr/bin/git st", 15);
        state.comp.params.words = vec!["/usr/bin/git".into(), "st".into()];
        _set_command(&mut state, &null_cx());
        assert_eq!(state.lastcomp.get("_comp_command1").unwrap(), "/usr/bin/git");
        assert_eq!(state.lastcomp.get("_comp_command2").unwrap(), "git");
        assert_eq!(state.lastcomp.get("_comp_command").unwrap(), "git");
    }

    #[test]
    fn bareword_with_path_lookup_stores_resolved_path_in_command2() {
        // shell:28-32 — bareword fallback
        let mut state = MainCompleteState::new("ls", 2);
        state.comp.params.words = vec!["ls".into()];
        let cx = CommandClassifier {
            is_builtin: &|_| false,
            is_function: &|_| false,
            command_path: &|n| {
                if n == "ls" {
                    "/bin/ls".into()
                } else {
                    String::new()
                }
            },
        };
        _set_command(&mut state, &cx);
        assert_eq!(state.lastcomp.get("_comp_command1").unwrap(), "ls");
        assert_eq!(state.lastcomp.get("_comp_command2").unwrap(), "/bin/ls");
        // dispatch is c1 in this branch.
        assert_eq!(state.lastcomp.get("_comp_command").unwrap(), "ls");
    }

    #[test]
    fn does_not_alter_other_lastcomp_entries() {
        let mut state = MainCompleteState::new("git", 3);
        state.comp.params.words = vec!["git".into()];
        state.lastcomp.insert("prefix".to_string(), "g".into());
        _set_command(&mut state, &null_cx());
        assert_eq!(state.lastcomp.get("prefix").unwrap(), "g");
    }

    #[test]
    fn basename_helper_handles_trailing_slash() {
        assert_eq!(basename("a/b/c"), "c");
        assert_eq!(basename("nope"), "nope");
        assert_eq!(basename("a/b/"), "");
        assert_eq!(basename(""), "");
    }
}
