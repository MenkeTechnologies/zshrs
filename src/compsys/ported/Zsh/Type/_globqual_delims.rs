//! Port of `_globqual_delims` from
//! `Completion/Zsh/Type/_globqual_delims`.
//!
//! Full upstream body (24 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 5  # Helper for _globquals.  Sets delim to delimiter to match.
//! sh: 7  # don't restore special parameters
//! sh: 8  compstate[restore]=no
//! sh:10  delim=$PREFIX[1]
//! sh:11  compset -p 1
//! sh:13  # One of matching brackets?
//! sh:15  local matchl="<({[" matchr=">)}]"
//! sh:16  integer ind=${matchl[(I)$delim]}
//! sh:18  (( ind )) && delim=$matchr[ind]
//! sh:20  if compset -P "[^$delim]#$delim"; then
//! sh:22    # Completely matched.
//! sh:23    return 0
//! sh:24  else
//! sh:25    # Still in delimiter
//! sh:26    return 1
//! sh:27  fi
//! ```
//!
//! Helper for `_globquals` — derives the closing delimiter from the
//! 1st char of `$PREFIX`, supporting `<>`/`()`/`{}`/`[]` matched
//! pairs. Reports whether the delimited region is fully closed.

use crate::ported::params::{getsparam, setsparam};
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

/// `_globqual_delims` — set `$delim` to the closing delimiter for
/// the current glob-qualifier region; return 0 if fully matched, 1
/// if still inside.
pub fn _globqual_delims() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_globqual_delims");
    // sh:8
    set_compstate_str("restore", "no");

    // sh:10
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let mut delim = prefix.chars().next().unwrap_or(' ').to_string();

    // sh:11
    let _ = bin_compset(
        "compset",
        &["-p".to_string(), "1".to_string()],
        &make_ops(),
        0,
    );

    // sh:13-18  bracket-pair mirror
    let matchl = "<({[";
    let matchr = ">)}]";
    if let Some(idx) = matchl.find(delim.as_str()) {
        if let Some(closing) = matchr.chars().nth(idx) {
            delim = closing.to_string();
        }
    }

    // Publish delim for the caller (`_globquals` reads it via dynamic
    //   scoping in shell; we use the shell-side param table).
    let _ = setsparam("delim", &delim);

    // sh:20  compset -P "[^$delim]#$delim"
    let pat = format!("[^{}]#{}", delim, delim);
    if bin_compset("compset", &["-P".to_string(), pat], &make_ops(), 0) == 0 {
        // sh:23
        0
    } else {
        // sh:26
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_paren_sets_closing_paren_as_delim() {
        // sh:18 — `(` should map to `)` via the bracket-pair table.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "(foo");
        let _ = _globqual_delims();
        assert_eq!(getsparam("delim").as_deref(), Some(")"));
    }

    #[test]
    fn non_bracket_delim_stays_as_first_char() {
        // sh:10 — for non-bracket delim (e.g. `:`), stays as-is.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", ":foo");
        let _ = _globqual_delims();
        assert_eq!(getsparam("delim").as_deref(), Some(":"));
    }
}
