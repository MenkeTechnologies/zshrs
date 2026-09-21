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

use crate::compsys::ported::_set_command::_set_command;
use crate::compsys::ported::shared::dispatch_action_command;
use crate::ported::params::getsparam;
use crate::ported::zle::compcore::get_compstate_str;

/// `_redirect` — completion within a `>`/`<`/`|` redirection: try
/// per-command + per-redirect-target dispatch chain.
pub fn _redirect() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_redirect");
    // sh:5
    let _ = _set_command();

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
    // sh:18 is a COMMAND WORD, so `dispatch_action_command`
    // (shared.rs:1407) resolves it exactly as `execcmd` does:
    // shfunc/port/plugin (c:Src/exec.c:3105-3109), then builtin, then
    // `$PATH`, then c:903's `command not found` with c:908's 127. The
    // `.unwrap_or(1)` this replaces had NO not-found arm, so a name
    // that resolved nowhere returned in silence.
    dispatch_action_command("_dispatch", &argv, 18)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    /// With no executor wired the command word this path ends in resolves
    /// to no shell function, no builtin and nothing on `$PATH` — the
    /// `Src/exec.c:903` case — so it reports `command not found` and the
    /// status is c:908's 127. It used to return a silent 1.
    fn unresolvable_command_word_reports_not_found() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_redirect(), 127);
    }
}
