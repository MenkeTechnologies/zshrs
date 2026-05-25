//! Port of `_command` from `Completion/Zsh/Command/_command`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh: 1  #compdef command
//! sh: 2
//! sh: 3  _arguments \
//! sh: 4    '-v[indicate result of command search]:*:command:_path_commands' \
//! sh: 5    '-V[show result of command search in verbose form]:*:command:_path_commands' \
//! sh: 6    '(-)-p[use default PATH to find command]' \
//! sh: 7    '*:: : _normal -p $service'
//! ```
//!
//! Completion for the POSIX `command` builtin. Three optspecs and a
//! catch-all that dispatches the rest of the line to `_normal` with the
//! `-p` flag (which tells `_normal` to treat the next word as the
//! effective command name for arg-completion lookup).
//!
//! Faithful port: parses out which `command` flag is being completed
//! and what action the caller should take. compsys is a leaf crate
//! and can't itself invoke `_path_commands` or `_normal` (those need
//! the parent shell's path/cmdnam tables), so this port surfaces the
//! decision in a structured `CommandStage` enum and lets the caller
//! dispatch. `_arguments` itself isn't re-run here — we directly model
//! the four-clause _arguments table since it's tiny and immutable.



use crate::compsys::compcore::CompletionState;

/// What `_command` decided the caller should complete next.
///
/// Mirrors the four `_arguments` clauses verbatim:
///   - `Flag` — user is on a `-v` / `-V` / `-p` option (or has typed
///     the leading `-`); offer the three flags.
///   - `FlagArg(flag)` — user is past `-v` / `-V` and needs to type the
///     command name (`_path_commands`).
///   - `Normal { service }` — user is past the `command` builtin and
///     should get normal-command completion (`_normal -p <service>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandStage {
    /// Offer the three flags (`-v`, `-V`, `-p`).
    Flag,
    /// User is past `-v` or `-V`; complete a command name.
    /// The original flag is carried so callers can preserve context.
    FlagArg(char),
    /// User is past the `command` builtin proper; dispatch via
    /// `_normal -p` with the supplied service name.
    Normal { service: String },
}

/// _command — completion for the `command` builtin.
///
/// `state.params.words[0]` is `command` itself; subsequent words are
/// flag(s) and the wrapped command. `service` is the curcontext service
/// (zsh sets it to the function name being run — for shell `_command`
/// this is `"command"`). Returns `true` if a dispatch decision was
/// made, `false` if there's nothing to complete (no further words and
/// no `-`).
///
/// The decision tree mirrors the four `_arguments` clauses literally:
/// 1. Current word is `-v` or `-V` and there's no operand yet → caller
///    should run `_path_commands` for that flag.
/// 2. Current word starts with `-` (or is empty in arg position 2) →
///    offer the flag set.
/// 3. Otherwise → return [`CommandStage::Normal`] with the service name
///    so the caller dispatches to `_normal -p`.
pub fn _command(state: &CompletionState, service: &str) -> Option<CommandStage> {
    let words = &state.params.words;
    let current = state.params.current as usize;
    let prefix = &state.params.prefix;

    // The optspec `(-)-p` means -p is exclusive with positional args.
    // Once the user is past flags, dispatch to _normal.

    // Look back: did the previous word complete one of -v / -V?
    if current >= 2 {
        if let Some(prev) = words.get(current.saturating_sub(2)) {
            if prev == "-v" {
                return Some(CommandStage::FlagArg('v'));
            }
            if prev == "-V" {
                return Some(CommandStage::FlagArg('V'));
            }
        }
    }

    // Current word starts with `-` → user is typing/completing a flag.
    if prefix.starts_with('-') || (prefix.is_empty() && current == 2) {
        return Some(CommandStage::Flag);
    }

    // Anything else → defer to _normal with service for arg-completion.
    Some(CommandStage::Normal {
        service: service.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(words: &[&str], current: i32, prefix: &str) -> CompletionState {
        let mut s = CompletionState::new();
        s.params.words = words.iter().map(|w| w.to_string()).collect();
        s.params.current = current;
        s.params.prefix = prefix.into();
        s
    }

    #[test]
    fn dash_prefix_offers_flags() {
        let s = st(&["command", "-"], 2, "-");
        assert_eq!(_command(&s, "command"), Some(CommandStage::Flag));
    }

    #[test]
    fn empty_at_position_two_offers_flags() {
        // Before user has typed anything past `command`, position 2 is
        // a free choice between flags and a wrapped command — the
        // shell `_arguments` exposes flags here.
        let s = st(&["command", ""], 2, "");
        assert_eq!(_command(&s, "command"), Some(CommandStage::Flag));
    }

    #[test]
    fn after_dash_v_completes_command_name() {
        // words: command -v <cursor>  →  current=3, prev=-v
        let s = st(&["command", "-v", ""], 3, "");
        assert_eq!(_command(&s, "command"), Some(CommandStage::FlagArg('v')));
    }

    #[test]
    fn after_dash_big_v_completes_command_name() {
        let s = st(&["command", "-V", ""], 3, "");
        assert_eq!(_command(&s, "command"), Some(CommandStage::FlagArg('V')));
    }

    #[test]
    fn plain_word_dispatches_to_normal() {
        // command ls <cursor>  →  fall through to _normal -p
        let s = st(&["command", "ls", ""], 3, "");
        assert_eq!(
            _command(&s, "command"),
            Some(CommandStage::Normal {
                service: "command".into()
            })
        );
    }

    #[test]
    fn after_dash_p_falls_through_to_normal() {
        // -p doesn't take an arg, so the next word is treated as the
        // command name → Normal dispatch.
        let s = st(&["command", "-p", "ls"], 3, "ls");
        // current=3, words[1]="-p", prefix="ls" (no leading dash)
        // → Normal branch.
        assert_eq!(
            _command(&s, "command"),
            Some(CommandStage::Normal {
                service: "command".into()
            })
        );
    }

    #[test]
    fn service_string_is_forwarded_verbatim() {
        let s = st(&["command", "ls", ""], 3, "");
        if let Some(CommandStage::Normal { service }) = _command(&s, "my-service") {
            assert_eq!(service, "my-service");
        } else {
            panic!("expected Normal stage");
        }
    }

    #[test]
    fn dash_v_with_arg_typed_completes_command_with_prefix() {
        // command -v gi<cursor>  → still FlagArg('v') because prev=-v.
        let s = st(&["command", "-v", "gi"], 3, "gi");
        assert_eq!(_command(&s, "command"), Some(CommandStage::FlagArg('v')));
    }

    #[test]
    fn dash_partial_typed_still_offers_flags() {
        // User typed `-v` halfway; still in flag mode.
        let s = st(&["command", "-v"], 2, "-v");
        assert_eq!(_command(&s, "command"), Some(CommandStage::Flag));
    }
}
