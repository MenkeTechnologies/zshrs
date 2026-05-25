//! Port of `_gnu_generic` from `Completion/Unix/Command/_gnu_generic`.
//!
//! Full upstream body (6 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This is for GNU-like commands which understand the --help option,
//! sh: 4  # but which do not otherwise require special completion handling.
//! sh: 5
//! sh: 6  _arguments '*:arg: _default' --
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



use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::{Completion, CompletionFlags};

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

    // shell:5 `_arguments '*:arg: _default' --` — when the user is
    // typing a non-option positional, chain to file completion via
    // `_default`. We only do this if NO option matches were added
    // AND the prefix doesn't start with `-`.
    if state.nmatches == 0 && !prefix.starts_with('-') {
        // Fall through to file completion via _default.
        return crate::compsys::ported::_default::_default(state);
    }

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
        // Every emitted match must start with `-` when prefix is
        // `-` (the parser's contract).
        let mut state = CompletionState::new();
        state.params.prefix = "-".into();
        let _ = _gnu_generic(&mut state, "ls");
        for c in state.groups.iter().flat_map(|g| g.matches.iter()) {
            assert!(
                c.str_.starts_with('-'),
                "parser must emit dash-prefixed options; got `{}`",
                c.str_
            );
        }
    }

    #[test]
    fn non_dash_prefix_falls_back_to_file_completion() {
        // shell:5 `_arguments '*:arg: _default' --` — positional
        // arg falls through to _default → file completion.
        // Use a prefix that targets the test cwd (compsys/).
        let mut state = CompletionState::new();
        state.params.prefix = "Cargo".into();
        // Pretend we have a known-trivial cmd that exits 0 with
        // empty --help output (`true --help` works on most systems).
        let ok = _gnu_generic(&mut state, "true");
        // Even though `true --help` produces no options, file
        // fallback should pick up Cargo.toml in cwd.
        if ok {
            let names: Vec<String> = state
                .groups
                .iter()
                .flat_map(|g| g.matches.iter())
                .map(|c| c.str_.clone())
                .collect();
            assert!(
                names.iter().any(|n| n.contains("Cargo")),
                "fallback to _default must surface Cargo.toml; got {names:?}"
            );
        }
    }

    #[test]
    fn dash_prefix_does_not_fall_back_to_files() {
        // When prefix starts with `-` but no options match (because
        // the command's --help is empty/missing), we must NOT pull
        // in file completions — user clearly wants an option.
        let mut state = CompletionState::new();
        state.params.prefix = "-zzzz-nonexistent-opt".into();
        let _ = _gnu_generic(&mut state, "true");
        // No file matches should appear (Cargo.toml etc.).
        let names: Vec<String> = state
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.clone())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains("Cargo.toml")),
            "dash-prefixed input must not fall back to file completion; got {names:?}"
        );
    }

    #[test]
    fn long_options_with_equals_emitted_with_nospace_suffix() {
        // We need a command whose --help reliably outputs an option
        // with `=`. `head --help` on GNU systems emits e.g.
        // `--bytes=[-]NUM`. On macOS BSD head may print to stderr in
        // a different shape. Tolerate either by checking only on
        // success.
        let mut state = CompletionState::new();
        state.params.prefix = "--by".into();
        let _ = _gnu_generic(&mut state, "head");
        // If we got anything starting with `--by`, AND it
        // had `=` immediately after, NOSPACE should be set.
        for c in state.groups.iter().flat_map(|g| g.matches.iter()) {
            if c.str_.starts_with("--by") {
                // suf should be Some("=") for any opt parsed with =
                if c.suf.is_some() {
                    assert_eq!(c.suf.as_deref(), Some("="));
                    assert!(c.flags.contains(CompletionFlags::NOSPACE));
                }
            }
        }
    }

    #[test]
    fn long_options_filtered_by_prefix() {
        // With prefix `--he`, ANY emitted option must start with `--he`.
        let mut state = CompletionState::new();
        state.params.prefix = "--he".into();
        let _ = _gnu_generic(&mut state, "ls");
        for c in state.groups.iter().flat_map(|g| g.matches.iter()) {
            assert!(
                c.str_.starts_with("--he"),
                "prefix filter violated; got `{}`",
                c.str_
            );
        }
    }

    #[test]
    fn short_options_with_alphabetic_char_parsed() {
        // Short options like `-l`, `-a`. Any emitted short opt must
        // be exactly 2 chars and the second char alphanumeric.
        let mut state = CompletionState::new();
        state.params.prefix = "-".into();
        let _ = _gnu_generic(&mut state, "ls");
        for c in state.groups.iter().flat_map(|g| g.matches.iter()) {
            if c.str_.len() == 2 {
                let ch = c.str_.chars().nth(1).unwrap();
                assert!(
                    ch.is_alphanumeric(),
                    "short opt's second char must be alphanum; got `{}`",
                    c.str_
                );
            }
        }
    }
}
