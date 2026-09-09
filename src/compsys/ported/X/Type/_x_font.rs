//! Port of `_x_font` from `Completion/X/Type/_x_font`.
//!
//! Full upstream body (16 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _tags fonts || return 1
//! sh: 9  if (( ! $+_font_cache )); then
//! sh:10    typeset -gU _font_cache
//! sh:12    _font_cache=( ${(f)"$(_call_program fonts xlsfonts)"} )
//! sh:13  fi
//! sh:15  _wanted fonts expl font \
//! sh:16      compadd -M 'r:|-=* r:|=*' "$@" -a - _font_cache
//! ```
//!
//! sh:12 — `_call_program fonts xlsfonts` runs `xlsfonts` (via the
//! `fonts` style key), its captured stdout (`$REPLY`) is split on
//! newlines (`${(f)...}`) into the `_font_cache` array. `typeset -gU`
//! makes the array globally-scoped and duplicate-suppressing.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:12  `${(f)"$(...)"}` split + sh:10 `typeset -gU` dedupe —
/// pure helper split out for headless-testability.
fn dedupe_lines(stdout: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for line in stdout.lines() {
        if seen.insert(line.to_string()) {
            out.push(line.to_string());
        }
    }
    out
}

/// sh:12 — populate `_font_cache` from `xlsfonts` output, deduped
/// (`typeset -gU`) preserving first-seen order.
fn build_font_cache() -> Vec<String> {
    let _ = call_program_capture(&["fonts".to_string(), "xlsfonts".to_string()]);
    let stdout = getsparam("REPLY").unwrap_or_default();
    dedupe_lines(&stdout)
}

/// `_x_font` — complete X font names via `xlsfonts`, cached in the
/// global `_font_cache` array.
pub fn _x_font(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_font");
    // sh:3 — `local expl`.
    //
    // This port does not assign `expl` itself; it hands the NAME to
    // `_wanted`/`_description`, and `_description` fills it through
    // `setaparam` — `createparam(name, PM_SCALAR)` with no PM_LOCAL
    // (shared.rs:16-30). The array was therefore born at level 0 and
    // `endparamscope` had nothing to unwind, so one TAB left `expl`
    // in the user's shell. Measured through a pty, `${(t)expl}` and
    // the value read back at the next prompt after a single TAB on a
    // `_x_font` wrapper:
    //
    //   zsh  : [][]
    //   zshrs: [array][-J|-default-]
    //
    // kind 0, as sh:3 spells a bare `local`.
    crate::compsys::ported::shared::declare_locals(&["expl"], 0);
    // sh:5  _tags fonts || return 1
    if _tags(&["fonts".to_string()]) != 0 {
        return 1;
    }

    // sh:9-13 — populate/reuse the `_font_cache` param.
    if getaparam("_font_cache").is_none() {
        setaparam("_font_cache", build_font_cache());
        // sh:10 — `typeset -gU _font_cache`. `setaparam` creates the node
        // through `createparam(name, PM_SCALAR)` and carries no attribute
        // bits, so the `-U` was lost: `${(t)_font_cache}` read `array`
        // where zsh reads `array-unique`. Stamp PM_UNIQUE onto the node the
        // way `bin_typeset`'s attribute-only arm does
        // (`Src/builtin.c:2575`). It is not cosmetic — `_font_cache` is a
        // cross-invocation GLOBAL cache (that is what the `-g` is for), so
        // the flag governs every later append by any completer that touches
        // the name, not just the build below.
        if let Ok(mut tab) = crate::ported::params::paramtab().write() {
            if let Some(pm) = tab.get_mut("_font_cache") {
                pm.node.flags |= crate::compsys::ported::shared::PM_UNIQUE as i32;
            }
        }
    }

    // sh:15-16  _wanted fonts expl font \
    //   compadd -M 'r:|-=* r:|=*' "$@" -a - _font_cache
    let mut w: Vec<String> = vec![
        "fonts".to_string(),
        "expl".to_string(),
        "font".to_string(),
        "compadd".to_string(),
        "-M".to_string(),
        "r:|-=* r:|=*".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("_font_cache".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedupe_lines_preserves_first_seen_order() {
        // sh:10 `typeset -gU` + sh:12 `${(f)"$(...)"}` — duplicate
        //   lines from the captured `xlsfonts` stdout collapse,
        //   keeping the order of first occurrence.
        let out = dedupe_lines("a\nb\na\nc\nb");
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn dedupe_lines_empty_stdout_yields_empty() {
        assert_eq!(dedupe_lines(""), Vec::<String>::new());
    }

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_x_font(&[]), 1);
    }

    /// sh:3's `local expl` must produce a PARAMETER THAT GOES AWAY, even
    /// though this port never assigns `expl` itself.
    ///
    /// This is the shape 60 of the 63 measured leaks had: the port only hands
    /// the NAME to `_wanted`, and `_description` does the `setaparam`. The
    /// declaration is still the caller's job — that is exactly what sh:3 is —
    /// and without it the array `_description` builds is created at level 0
    /// and survives the completion.
    #[test]
    fn expl_is_function_local_and_unwinds() {
        let _g = crate::test_util::global_state_lock();
        // `expl` is process-global state shared with every other test in this
        // binary, and several of them leave one behind. Clear it first so the
        // absence assertion below measures THIS port, and clear it again after
        // so the completion state a `_tags` run leaves does not decide the
        // answer of whichever test cargo schedules next.
        crate::test_util::reset_completion_state();
        crate::ported::utils::inc_locallevel();
        let inner = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = _x_font(&[]);
            let ty = crate::ported::params::paramtab()
                .read()
                .ok()
                .and_then(|t| {
                    t.get("expl")
                        .map(|pm| crate::ported::modules::parameter::paramtypestr(pm))
                })
                .unwrap_or_default();
            assert!(
                ty.contains("local"),
                "`expl` reads `{ty}`, not `...-local`; sh:3 declares it local"
            );
        }));
        crate::ported::params::endparamscope();
        let still = crate::ported::params::paramtab()
            .read()
            .map(|t| t.get("expl").is_some())
            .unwrap_or(false);
        crate::test_util::reset_completion_state();
        if let Err(p) = inner {
            std::panic::resume_unwind(p);
        }
        assert!(
            !still,
            "`expl` survived endparamscope; one TAB leaves it in the shell and \
             `_parameters` then offers it as a match"
        );
    }
}
