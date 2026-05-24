//! Port of `_condition` — `[[ … ]]` test-condition context.
//!
//! Local shell reference:
//! `/opt/homebrew/share/zsh/functions/_condition`.
//!
//! Upstream shell source (~60 lines, key dispatch):
//! ```text
//! #compdef -condition-
//!
//! local prev="$words[CURRENT-1]" ret=1
//!
//! if [[ "$prev" = -o ]]; then
//!   _tags -C -o options && _options
//! elif [[ "$prev" = -([a-hkprsuwxLOGSN]|[no]t|ef) ]]; then
//!   _tags -C "$prev" files && _files
//! elif [[ "$prev" = -t ]]; then
//!   _file_descriptors
//! elif [[ "$prev" = -v ]]; then
//!   _parameters -r "\= \t\n\[\-"
//! else
//!   …operator suggestions…
//! fi
//! ```
//!
//! Strict Rust port: faithful dispatch based on `prev` (the
//! previous word on the line). Operators get their right-hand
//! side completed per the upstream branch table. Caller supplies
//! file/options/parameter handlers since `_options` /
//! `_file_descriptors` / `_parameters` need data injection.

use std::collections::HashMap;

use crate::base::MainCompleteState;
use crate::ported::_file_descriptors::_file_descriptors;
use crate::ported::_options::_options;
use crate::ported::_parameters::_parameters;

/// `_condition` — `-condition-` context dispatcher.
pub fn _condition(
    state: &mut MainCompleteState,
    shell_options: &[(&str, bool)],
    params: &HashMap<String, String>,
    files_completer: impl FnOnce(&mut MainCompleteState) -> bool,
) -> bool {
    let prev = if state.comp.params.current >= 1 {
        let idx = (state.comp.params.current - 1) as usize;
        state.comp.params.words.get(idx).cloned().unwrap_or_default()
    } else {
        String::new()
    };

    match prev.as_str() {
        // shell:5 — `-o` → options
        "-o" => _options(&mut state.comp, shell_options),
        // shell:9 — `-t` → file descriptors
        "-t" => _file_descriptors(&mut state.comp),
        // shell:11 — `-v` → parameters
        "-v" => _parameters(&mut state.comp, params),
        // shell:7 — file-test operators → files
        p if matches!(
            p,
            "-a" | "-b"
                | "-c"
                | "-d"
                | "-e"
                | "-f"
                | "-g"
                | "-h"
                | "-k"
                | "-p"
                | "-r"
                | "-s"
                | "-u"
                | "-w"
                | "-x"
                | "-L"
                | "-O"
                | "-G"
                | "-S"
                | "-N"
                | "-nt"
                | "-ot"
                | "-ef"
        ) =>
        {
            files_completer(state)
        }
        // Default — operator suggestions are caller-handled.
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nofiles(_: &mut MainCompleteState) -> bool {
        panic!("files_completer must not run for this test");
    }

    #[test]
    fn dash_o_dispatches_to_options() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-o".into()]; // prev = "-o"
        let opts: Vec<(&str, bool)> = vec![("EXTENDED_GLOB", true)];
        let _ = _condition(&mut state, &opts, &HashMap::new(), nofiles);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"EXTENDED_GLOB"));
    }

    #[test]
    fn dash_v_dispatches_to_parameters() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-v".into()];
        let mut p = HashMap::new();
        p.insert("X".into(), "scalar".into());
        let _ = _condition(&mut state, &[], &p, nofiles);
        let names: Vec<&str> = state
            .comp
            .groups
            .iter()
            .flat_map(|g| g.matches.iter())
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.contains(&"X"));
    }

    #[test]
    fn dash_f_dispatches_to_files_completer() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-f".into()];
        let fired = std::cell::Cell::new(false);
        let _ = _condition(&mut state, &[], &HashMap::new(), |_| {
            fired.set(true);
            true
        });
        assert!(fired.get());
    }

    #[test]
    fn dash_t_dispatches_to_file_descriptors() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "-t".into()];
        // _file_descriptors may emit nothing in sandboxed env; pin
        // no panic + correct group label.
        let _ = _condition(&mut state, &[], &HashMap::new(), nofiles);
        // Group is created even if no fds open.
        assert!(state
            .comp
            .groups
            .iter()
            .any(|g| g.name == "file-descriptors"));
    }

    #[test]
    fn unknown_prev_returns_false() {
        let mut state = MainCompleteState::new("", 0);
        state.comp.params.current = 2;
        state.comp.params.words = vec!["[[".into(), "??".into()];
        assert!(!_condition(&mut state, &[], &HashMap::new(), nofiles));
    }
}
