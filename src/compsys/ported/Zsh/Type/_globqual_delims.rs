//! Port of `_globqual_delims` from
//! `Completion/Zsh/Type/_globqual_delims`.
//!
//! Full upstream body (24 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # Helper for _globquals.  Sets delim to delimiter to match.
//! sh: 5  # don't restore special parameters
//! sh: 6  compstate[restore]=no
//! sh: 8  delim=$PREFIX[1]
//! sh: 9  compset -p 1
//! sh:11  # One of matching brackets?
//! sh:13  local matchl="<({[" matchr=">)}]"
//! sh:14  integer ind=${matchl[(I)$delim]}
//! sh:16  (( ind )) && delim=$matchr[ind]
//! sh:18  if compset -P "[^$delim]#$delim"; then
//! sh:19    # Completely matched.
//! sh:20    return 0
//! sh:21  else
//! sh:22    # Still in delimiter
//! sh:23    return 1
//! sh:24  fi
//! ```
//!
//! Helper for `_globquals` — derives the closing delimiter from the
//! 1st char of `$PREFIX`, supporting `<>`/`()`/`{}`/`[]` matched
//! pairs. Reports whether the delimited region is fully closed.

use crate::ported::params::{getsparam, setsparam};
use crate::ported::subst::singsub;
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Reach `_globqual_delims` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `elif ! _globqual_delims; then` (Completion/Zsh/Type/_globquals sh:28) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_globqual_delims_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _globqual_delims() -> i32 {
    crate::compsys::ported::shared::call_compfn("_globqual_delims", &[], || _globqual_delims_impl())
}

/// `_globqual_delims` — set `$delim` to the closing delimiter for
/// the current glob-qualifier region; return 0 if fully matched, 1
/// if still inside.
pub fn _globqual_delims_impl() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_globqual_delims");
    // sh:6
    set_compstate_str("restore", "no");

    // sh:8 `delim=$PREFIX[1]` — empty when `$PREFIX` is. `delim` is not
    //   local: `_globquals` reads it back.
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let _ = setsparam(
        "delim",
        &prefix.chars().next().map(String::from).unwrap_or_default(),
    );

    // sh:9
    let _ = bin_compset(
        "compset",
        &["-p".to_string(), "1".to_string()],
        &make_ops(),
        0,
    );

    // sh:13 `local matchl="<({[" matchr=">)}]"`
    crate::compsys::ported::shared::declare_locals(&["matchl", "matchr"], 0);
    let _ = setsparam("matchl", "<({[");
    let _ = setsparam("matchr", ">)}]");
    // sh:14 `integer ind=${matchl[(I)$delim]}` — `(I)` takes `$delim` as a
    //   PATTERN, so the shell's own subscript search decides it: an empty or
    //   `*` delim gives 5 (the empty match past the end), `(` and `[` give 0
    //   (bad pattern), `?` gives 4. A literal `find` gets all three wrong.
    let ind: usize = singsub("${matchl[(I)$delim]}").parse().unwrap_or(0);
    // sh:16 `(( ind )) && delim=$matchr[ind]` — past the end reads empty.
    if ind > 0 {
        let closing = ">)}]".chars().nth(ind - 1).map(String::from).unwrap_or_default();
        let _ = setsparam("delim", &closing);
    }
    let delim = getsparam("delim").unwrap_or_default();

    // sh:18  compset -P "[^$delim]#$delim"
    let pat = format!("[^{}]#{}", delim, delim);
    if bin_compset("compset", &["-P".to_string(), pat], &make_ops(), 0) == 0 {
        // sh:20
        0
    } else {
        // sh:23
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delim_follows_the_shells_pattern_subscript_search() {
        // sh:14 `${matchl[(I)$delim]}` searches with `$delim` as a PATTERN.
        // Measured with zsh 5.9.2 over matchl="<({[": `<` -> 1, `{` -> 3,
        // `(` -> 0 (a lone `(` is a bad pattern), empty -> 5 (past the end).
        let _g = crate::test_util::global_state_lock();
        for (prefix, want) in [("<foo", ">"), ("{foo", "}"), ("(foo", "("), ("", "")] {
            let _ = setsparam("PREFIX", prefix);
            let _ = _globqual_delims_impl();
            assert_eq!(getsparam("delim").as_deref(), Some(want), "PREFIX={prefix:?}");
        }
    }

    #[test]
    fn non_bracket_delim_stays_as_first_char() {
        // sh: 8 — for non-bracket delim (e.g. `:`), stays as-is.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", ":foo");
        let _ = _globqual_delims_impl();
        assert_eq!(getsparam("delim").as_deref(), Some(":"));
    }
}
