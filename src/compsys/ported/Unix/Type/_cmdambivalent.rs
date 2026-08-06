//! Port of `_cmdambivalent` from `Completion/Unix/Type/_cmdambivalent`.
//!
//! Full upstream body (17 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  if (( CURRENT == 1 && ${#words} == 1 )); then
//! sh: 4    # Heuristics to decide whether to complete for system() or for execl().
//! sh: 5    local space=' '
//! sh: 6    if (( ${${words[CURRENT]}[(I)$space]} )); then
//! sh: 7      _cmdstring
//! sh: 8    elif [[ ${${compstate[all_quotes]}[1]} == (\'|\") ]]; then
//! sh: 9      _cmdstring
//! sh:10    else
//! sh:11      _command_names -e
//! sh:12    fi
//! sh:13  elif (( CURRENT == 1 )); then
//! sh:14    _command_names -e
//! sh:15  else
//! sh:16    _normal
//! sh:17  fi
//! ```

use crate::compsys::ported::_cmdstring::cmdstring_byname;
use crate::compsys::ported::_command_names::command_names_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam};
use crate::ported::zle::compcore::get_compstate_str;

/// `_cmdambivalent` — choose between cmdstring/command-names/normal
/// completion based on cursor position and quoting state.
pub fn _cmdambivalent() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_cmdambivalent");
    let current: usize = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let words = getaparam("words").unwrap_or_default();

    // sh:3
    if current == 1 && words.len() == 1 {
        // sh:6  $words[CURRENT] contains a space
        let curword = words.first().cloned().unwrap_or_default();
        if curword.contains(' ') {
            return cmdstring_byname();
        }
        // sh:8  compstate[all_quotes][1] is `'` or `"`
        let all_quotes = get_compstate_str("all_quotes").unwrap_or_default();
        if all_quotes.starts_with('\'') || all_quotes.starts_with('"') {
            return cmdstring_byname();
        }
        // sh:11 — bare command word; by name so `$fpath` arbitration runs.
        // `_command_names` IS overridden on a real zpwr host
        // (`~/.zpwr/autoload/comp_utils/_command_names`), which is the same
        // defect fixed for the direct dispatch path in b8e714f7be.
        let a = ["-e".to_string()];
        return crate::compsys::ported::shared::call_compfn("_command_names", &a, || {
            command_names_byname(&a)
        });
    }
    // sh:13 — likewise.
    if current == 1 {
        let a = ["-e".to_string()];
        return crate::compsys::ported::shared::call_compfn("_command_names", &a, || {
            command_names_byname(&a)
        });
    }
    // sh:16
    dispatch_function_call("_normal", &[]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{setaparam, setsparam};

    #[test]
    fn current_gt_one_dispatches_normal() {
        // sh:16 — when CURRENT > 1, defer to _normal (returns 1
        //   without executor).
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("CURRENT", "3");
        setaparam(
            "words",
            vec!["a".to_string(), "b".to_string(), "c".to_string()],
        );
        assert_eq!(_cmdambivalent(), 1);
    }
}
