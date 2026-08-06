//! Port of `_redirect` from `Completion/Zsh/Context/_redirect`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #compdef -redirect-
//! sh: 2
//! sh: 3  local strs _comp_command1 _comp_command2 _comp_command
//! sh: 4
//! sh: 5  _set_command
//! sh: 6
//! sh: 7  strs=( -default- )
//! sh: 8
//! sh: 9  if [[ "$CURRENT" != "1" ]]; then
//! sh:10    strs=( "${_comp_command}" "$strs[@]" )
//! sh:11    if [[ -n "$_comp_command1" ]]; then
//! sh:12      strs=( "${_comp_command1}" "$strs[@]" )
//! sh:13      [[ -n "$_comp_command2" ]] &&
//! sh:14        strs=( "${_comp_command2}" "$strs[@]" )
//! sh:15    fi
//! sh:16  fi
//! sh:17
//! sh:18  _dispatch -redirect-,${compstate[redirect]},$_comp_command \
//! sh:19        -redirect-,{${compstate[redirect]},-default-},${^strs}
//! ```

use crate::compsys::ported::_set_command::set_command_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getsparam;
use crate::ported::zle::compcore::get_compstate_str;

/// `_redirect` — completion within a `>`/`<`/`|` redirection: try
/// per-command + per-redirect-target dispatch chain.
pub fn _redirect() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_redirect");
    // sh:5
    let _ = set_command_byname();

    // sh:7-15  build prefixes
    let mut strs: Vec<String> = vec!["-default-".to_string()];
    let current = getsparam("CURRENT").unwrap_or_default();
    if current != "1" {
        let cc = getsparam("_comp_command").unwrap_or_default();
        let cc1 = getsparam("_comp_command1").unwrap_or_default();
        let cc2 = getsparam("_comp_command2").unwrap_or_default();
        let mut prefix: Vec<String> = vec![cc];
        if !cc1.is_empty() {
            prefix.insert(0, cc1);
            if !cc2.is_empty() {
                prefix.insert(0, cc2);
            }
        }
        prefix.extend(strs);
        strs = prefix;
    }

    // sh:18-19  build the dispatch argv with brace-expansion done
    //   manually: `-redirect-,{X,Y},Z` → `-redirect-,X,Z` and
    //   `-redirect-,Y,Z`.
    let redir = get_compstate_str("redirect").unwrap_or_default();
    let cc = getsparam("_comp_command").unwrap_or_default();
    let mut argv: Vec<String> = vec![format!("-redirect-,{},{}", redir, cc)];
    for s in &strs {
        argv.push(format!("-redirect-,{},{}", redir, s));
        argv.push(format!("-redirect-,-default-,{}", s));
    }
    dispatch_function_call("_dispatch", &argv).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_redirect(), 1);
    }
}
