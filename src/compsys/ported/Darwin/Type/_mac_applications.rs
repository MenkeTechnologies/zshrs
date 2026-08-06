//! Port of `_mac_applications` from `Completion/Darwin/Type/_mac_applications`.
//!
//! Full upstream body (7 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  _retrieve_mac_apps
//! sh: 4
//! sh: 5  local expl
//! sh: 6  _wanted commands expl 'macOS application' \
//! sh: 7      compadd "$@" - "${(@)${_mac_apps[@]:t}%.app}"
//! ```
//!
//! `_retrieve_mac_apps` (`Completion/Darwin/Type/_retrieve_mac_apps`) is not
//! yet ported to Rust — it is invoked via the shell-fallback
//! `dispatch_function_call`, matching the pattern used for predicate/filter
//! callbacks in sibling ports (e.g. `_baudrates::_baudrates`'s `-f`
//! handling). It populates the global `_mac_apps` array param with
//! application bundle paths.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getaparam;

/// sh:7 — `${(@)${_mac_apps[@]:t}%.app}`: basename (`:t`) each element of
/// `_mac_apps`, then strip a trailing `.app` suffix (`%.app`) from each.
fn basenames_stripped_app(mac_apps: &[String]) -> Vec<String> {
    mac_apps
        .iter()
        .map(|path| {
            let base = path.rsplit('/').next().unwrap_or(path.as_str());
            base.strip_suffix(".app").unwrap_or(base).to_string()
        })
        .collect()
}

/// `_mac_applications` — offer macOS `.app` bundle names (basenames, with
/// the `.app` suffix stripped) as completion candidates.
pub fn _mac_applications(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_mac_applications");
    // sh:3  _retrieve_mac_apps (shell fallback: not yet ported to Rust;
    //   populates the `_mac_apps` global array param).
    let _ = dispatch_function_call("_retrieve_mac_apps", &[]);

    // sh:5-7  local expl; _wanted commands expl 'macOS application' \
    //   compadd "$@" - "${(@)${_mac_apps[@]:t}%.app}"
    let mac_apps = getaparam("_mac_apps").unwrap_or_default();
    let names = basenames_stripped_app(&mac_apps);

    let mut wanted_args: Vec<String> = vec![
        "commands".to_string(),
        "expl".to_string(),
        "macOS application".to_string(),
        "compadd".to_string(),
    ];
    wanted_args.extend(args.iter().cloned());
    wanted_args.push("-".to_string());
    wanted_args.extend(names);
    _wanted(&wanted_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_dir_and_app_suffix() {
        // sh:7 — `:t` takes the basename, `%.app` strips a trailing
        //   `.app` suffix from each array element.
        let mac_apps = vec![
            "/Applications/Safari.app".to_string(),
            "/System/Applications/Utilities/Terminal.app".to_string(),
        ];
        assert_eq!(
            basenames_stripped_app(&mac_apps),
            vec!["Safari".to_string(), "Terminal".to_string()]
        );
    }

    #[test]
    fn leaves_non_app_basenames_untouched() {
        // `%.app` only strips a trailing match; entries without the
        //   suffix pass through unchanged.
        let mac_apps = vec!["/Applications/README".to_string()];
        assert_eq!(
            basenames_stripped_app(&mac_apps),
            vec!["README".to_string()]
        );
    }

    #[test]
    fn empty_mac_apps_yields_empty_names() {
        assert_eq!(basenames_stripped_app(&[]), Vec::<String>::new());
    }

    #[test]
    fn returns_one_without_completion_context() {
        // sh:6 — `_wanted` returns 1 when no tags are registered
        //   (no active completion context), matching shell semantics
        //   and the convention used by sibling ports (e.g.
        //   `_baudrates::returns_one_without_completion_context`).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_mac_applications(&[]), 1);
    }
}
