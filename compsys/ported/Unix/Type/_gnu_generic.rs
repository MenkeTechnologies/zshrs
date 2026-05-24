//! Port of `_gnu_generic` — generic GNU-style option completion
//! from `--help` output.
//!
//! Local shell reference: `compsys/functions/Unix/Type/_gnu_generic`
//! (system copy `/opt/homebrew/share/zsh/functions/_gnu_generic`).
//!
//! Upstream shell source (the WHOLE file, 5 lines):
//! ```text
//!  3  # For GNU-like commands which understand the --help option,
//!  4  # but which do not otherwise require special completion handling.
//!  5  _arguments '*:arg: _default' --
//! ```
//!
//! Upstream lets `_arguments` handle the `--help` parsing — it
//! recognises `--option` and `-o` from the cmd's own help output
//! and offers them as completions, plus `_default` for positional
//! args.
//!
//! Simplified Rust port: forks `<command> --help`, parses
//! dash-prefixed options from the output (`--option`, `--option=`,
//! `-o`) and emits them. Doesn't yet delegate positional args to
//! `_files` like the shell's `_default` would.

use crate::compcore::CompletionState;
use crate::completion::{Completion, CompletionFlags};

/// _gnu_generic - Generic GNU-style option completion from --help
pub fn _gnu_generic(state: &mut CompletionState, command: &str) -> bool {
    let prefix = state.params.prefix.clone();

    // Run command --help and parse options
    let output = std::process::Command::new(command).arg("--help").output();

    let help_text = match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            format!("{}{}", stdout, stderr)
        }
        Err(_) => return false,
    };

    state.begin_group("options", true);

    // Parse --option and -o patterns
    for line in help_text.lines() {
        let line = line.trim();

        // Match patterns like: --option, -o, --option=ARG
        let mut i = 0;
        while i < line.len() {
            if line[i..].starts_with("--") {
                let start = i;
                i += 2;
                while i < line.len() {
                    let c = line.chars().nth(i).unwrap_or(' ');
                    if c.is_alphanumeric() || c == '-' || c == '_' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let opt = &line[start..i];
                if opt.len() > 2 && opt.starts_with(&prefix) {
                    // Check for =ARG
                    let has_arg = line[i..].starts_with("=") || line[i..].starts_with("[=");
                    let mut comp = Completion::new(opt.to_string());
                    if has_arg {
                        comp.suf = Some("=".to_string());
                        comp.flags |= CompletionFlags::NOSPACE;
                    }
                    state.add_match(comp, Some("options"));
                }
            } else if line[i..].starts_with("-") && !line[i..].starts_with("--") {
                let start = i;
                i += 1;
                if i < line.len()
                    && line
                        .chars()
                        .nth(i)
                        .map(|c| c.is_alphanumeric())
                        .unwrap_or(false)
                {
                    i += 1;
                    let opt = &line[start..i];
                    if opt.starts_with(&prefix) {
                        state.add_match(Completion::new(opt.to_string()), Some("options"));
                    }
                }
            } else {
                i += 1;
            }
        }
    }

    state.end_group();
    state.nmatches > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonexistent_command_returns_false() {
        let mut state = CompletionState::new();
        assert!(!_gnu_generic(&mut state, "/no/such/binary/at/all/zshrs"));
    }

    #[test]
    fn parses_dash_options_from_help_output() {
        // Any cmd with --help; we don't know which options it lists
        // without committing to a specific cmd, so just pin: every
        // emitted match must START WITH `-` (the parser's contract).
        let mut state = CompletionState::new();
        state.params.prefix = "-".into();
        let _ = _gnu_generic(&mut state, "ls");
        for c in state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
        {
            assert!(
                c.str_.starts_with('-'),
                "parser must emit dash-prefixed options; got `{}`",
                c.str_
            );
        }
    }
}
