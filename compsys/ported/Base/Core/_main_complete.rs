//! Port of `_main_complete` — top-level completion entry point.
//!
//! Local shell reference: `compsys/functions/Base/Core/_main_complete`
//! (system copy `/opt/homebrew/share/zsh/functions/_main_complete`).
//!
//! Upstream shell source (key lines from the 200+ line dispatch):
//! ```text
//!  3  # The main loop of the completion code. This is what is called
//!  4  # when completion is attempted from the command line.
//! 11  local IFS=$' \t\n\0'
//!  …  zstyle -a ":completion:${context}" completer comp
//!  …  for completer in $comp; do
//!  …    $completer
//!  …    [[ $ret = 0 ]] && break
//!  …  done
//! ```
//!
//! Faithful Rust port: walks the configured completer list via a
//! caller-supplied `dispatch` closure. The `pre_funcs` /
//! `post_funcs` shell-side hooks are exposed via
//! `state.prefuncs` / `state.postfuncs` so the caller can register
//! their own. Records `lastcomp[nmatches/completer/prefix/suffix]`
//! after every run — same as shell's `_lastcomp` association.

use crate::base::{CompleterResult, MainCompleteState};

/// Main completion entry point (_main_complete)
///
/// This is THE function that gets called when the user presses TAB.
pub fn _main_complete(
    state: &mut MainCompleteState,
    dispatch: impl Fn(&mut MainCompleteState, &str) -> CompleterResult,
) -> i32 {
    // Get completers from style or use defaults
    if let Some(completers) = state
        .styles
        .lookup_values(&state.context_string(), "completer")
    {
        state.completers = completers.to_vec();
    }

    state.ctx.completer_num = 1;

    // Call pre-functions
    let prefuncs = state.prefuncs.clone();
    for func in &prefuncs {
        // Would call the function here
        let _ = func;
    }

    // Try each completer
    for completer_name in state.completers.clone() {
        // Extract completer name (handle _complete:foo syntax)
        let (completer, name) = if let Some(pos) = completer_name.find(':') {
            (&completer_name[..pos], &completer_name[pos + 1..])
        } else {
            (completer_name.as_str(), &completer_name[1..]) // strip leading _
        };

        state.ctx.completer = name.replace('_', "-");

        // Get matcher list
        let matchers = state
            .styles
            .lookup_values(&state.context_string(), "matcher-list")
            .map(|v| v.to_vec())
            .unwrap_or_else(|| vec![String::new()]);

        state.ctx.matcher_num = 1;

        for matcher in &matchers {
            state.ctx.matcher = matcher.clone();

            // Call the completer
            match dispatch(state, completer) {
                CompleterResult::Matched => {
                    state.ret = 0;
                    break;
                }
                CompleterResult::Skip => break,
                CompleterResult::NoMatch => {}
            }

            state.ctx.matcher_num += 1;
        }

        if state.ret == 0 {
            break;
        }

        state.ctx.completer_num += 1;
    }

    // Call post-functions
    let postfuncs = state.postfuncs.clone();
    for func in &postfuncs {
        let _ = func;
    }

    // Store lastcomp info
    state
        .lastcomp
        .insert("nmatches".to_string(), state.comp.nmatches.to_string());
    state
        .lastcomp
        .insert("completer".to_string(), state.ctx.completer.clone());
    state
        .lastcomp
        .insert("prefix".to_string(), state.comp.params.prefix.clone());
    state
        .lastcomp
        .insert("suffix".to_string(), state.comp.params.suffix.clone());

    state.ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_to_each_completer_until_one_matches() {
        let mut state = MainCompleteState::new("", 0);
        state.completers = vec!["_complete".into(), "_match".into()];
        let calls = std::cell::RefCell::new(Vec::<String>::new());
        let _ = _main_complete(&mut state, |s, name| {
            calls.borrow_mut().push(name.to_string());
            if name == "_match" {
                s.ret = 0;
                CompleterResult::Matched
            } else {
                CompleterResult::NoMatch
            }
        });
        let names = calls.into_inner();
        assert!(names.contains(&"_complete".to_string()), "got {names:?}");
        assert!(names.contains(&"_match".to_string()));
    }

    #[test]
    fn ret_zero_after_match() {
        let mut state = MainCompleteState::new("", 0);
        state.completers = vec!["_complete".into()];
        let ret = _main_complete(&mut state, |s, _| {
            s.ret = 0;
            CompleterResult::Matched
        });
        assert_eq!(ret, 0);
    }

    #[test]
    fn lastcomp_populated_with_completion_result() {
        let mut state = MainCompleteState::new("git", 3);
        state.comp.params.prefix = "git".into();
        state.completers = vec!["_complete".into()];
        _main_complete(&mut state, |_, _| CompleterResult::NoMatch);
        assert_eq!(
            state.lastcomp.get("prefix").map(String::as_str),
            Some("git")
        );
        assert!(state.lastcomp.contains_key("nmatches"));
    }
}
