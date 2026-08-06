//! Port of `_read_comp` from
//! `Completion/Base/Widget/_read_comp`.
//!
//! Full upstream body (152 lines, abridged):
//! ```text
//! sh:  1  #compdef -k complete-word \C-x\C-r
//! sh: 29  typeset -g _read_comp
//! sh: 30  if [[ ${+NUMERIC} = 0 && -n $_read_comp ]]; then
//! sh: 30    if [[ $_read_comp = _* ]]; then eval $_read_comp
//! sh: 32    else eval "compadd $_read_comp"
//! sh: 36  return
//! sh: 40  read -k key + key loop building $str interactively
//! sh:130  _read_comp="$str"
//! sh:140  …
//! ```
//!
//! Interactive on-the-fly completion read. Faithful port requires
//! ZLE key input; this port honors the "re-use last $_read_comp"
//! short-circuit (sh:29-36) and degrades the read loop to a
//! one-shot dispatch.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getsparam;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_read_comp` — re-run the last-used `_read_comp` string. Without
/// an interactive read loop, returns 1 when `$_read_comp` is empty.
pub fn _read_comp() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_read_comp");
    let cached = getsparam("_read_comp").unwrap_or_default();
    if cached.is_empty() {
        return 1;
    }
    if cached.starts_with('_') {
        // sh:30 — dispatch the fn named in cached
        dispatch_function_call(&cached, &[]).unwrap_or(1)
    } else {
        // sh:32 — split into compadd argv
        let parts: Vec<String> = cached.split_whitespace().map(|s| s.to_string()).collect();
        bin_compadd("compadd", &parts, &make_ops(), 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_cached() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("_read_comp", "");
        assert_eq!(_read_comp(), 1);
    }
}
