//! Port of `_ignored` from `Completion/Base/Completer/_ignored`.
//!
//! Full upstream body (68 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  [[ _matcher_num -gt 1 || $compstate[ignored] -eq 0 ]] && return 1
//! sh: 7  local comp; integer ind
//! sh: 9  if ! zstyle -a … completer comp; then
//! sh:10    comp=( "${(@)_completers[1,_completer_num-1]}" )
//! sh:11    ind=${comp[(I)_ignored(|:*)]}
//! sh:12    (( ind )) && comp=("${(@)comp[ind,-1]}")
//! sh:13  fi
//! sh:15  local _comp_no_ignore=yes …
//! sh:20  for tmp in "$comp[@]"; do … completer chain dispatch …
//! sh:39      if zstyle -s … single-ignored tmp && … single match …; then
//! sh:42        case "$tmp" in
//! sh:43          show) compstate[insert]='' compstate[list]='list force' tmp='' ;;
//! sh:46          menu)
//! sh:47            compstate[insert]=menu
//! sh:48            _description original expl original
//! sh:49            compadd "$expl[@]" -S '' - "$PREFIX$SUFFIX"
//! sh:50            ;;
//! sh:51        esac
//! sh:55      return 0
//! sh:57    done
//! sh:59  done
//! sh:64  return 1
//! ```

use crate::compsys::ported::_description::_description;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getiparam, getsparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
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

/// `_ignored` — re-run prior completer chain with ignored-pattern
/// filter disabled, so previously-hidden matches reappear.
pub fn _ignored() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_ignored");
    // sh:5
    if getiparam("_matcher_num") > 1 {
        return 1;
    }
    let ignored: i64 = get_compstate_str("ignored")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if ignored == 0 {
        return 1;
    }

    // sh:9-13  completer chain selection
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let comp_style = lookupstyle(&format!(":completion:{}:", curcontext), "completer");
    let comp_list: Vec<String> = if !comp_style.is_empty() {
        comp_style
    } else {
        let completers = getaparam("_completers").unwrap_or_default();
        let comp_num = getiparam("_completer_num") as usize;
        let upto = comp_num.saturating_sub(1).min(completers.len());
        let slice = &completers[..upto];
        let ind = slice
            .iter()
            .position(|c| c == "_ignored" || c.starts_with("_ignored:"));
        match ind {
            Some(i) => slice[i..].to_vec(),
            None => slice.to_vec(),
        }
    };

    // sh:15  shell-local _comp_no_ignore=yes — set, restore at end
    let saved_no_ignore = getsparam("_comp_no_ignore").unwrap_or_default();
    let _ = setsparam("_comp_no_ignore", "yes");

    // sh:20-59  completer chain iteration
    for tmp in &comp_list {
        let bare = tmp.split(':').next().unwrap_or(tmp);
        if bare == "_ignored" {
            continue;
        }
        if dispatch_function_call(bare, &[]).unwrap_or(1) == 0 {
            // sh:39-55  single-ignored handling
            let single_ignored =
                lookupstyle(&format!(":completion:{}:", curcontext), "single-ignored")
                    .first()
                    .cloned()
                    .unwrap_or_default();
            let old_list = get_compstate_str("old_list").unwrap_or_default();
            let nmatches: i64 = get_compstate_str("nmatches")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            if !single_ignored.is_empty() && old_list != "shown" && nmatches == 1 {
                match single_ignored.as_str() {
                    "show" => {
                        set_compstate_str("insert", "");
                        set_compstate_str("list", "list force");
                    }
                    "menu" => {
                        set_compstate_str("insert", "menu");
                        let _ = _description(&[
                            "original".to_string(),
                            "expl".to_string(),
                            "original".to_string(),
                        ]);
                        let expl = getaparam("expl").unwrap_or_default();
                        let prefix = getsparam("PREFIX").unwrap_or_default();
                        let suffix = getsparam("SUFFIX").unwrap_or_default();
                        let mut compadd_argv: Vec<String> = expl;
                        compadd_argv.push("-S".to_string());
                        compadd_argv.push("".to_string());
                        compadd_argv.push("-".to_string());
                        compadd_argv.push(format!("{}{}", prefix, suffix));
                        let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
                    }
                    _ => {}
                }
            }
            // sh:55 — restore + return success
            let _ = setsparam("_comp_no_ignore", &saved_no_ignore);
            return 0;
        }
    }

    // sh:64 restore + fail
    let _ = setsparam("_comp_no_ignore", &saved_no_ignore);
    1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setiparam;

    #[test]
    fn matcher_num_gt_one_short_circuits() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 5);
        assert_eq!(_ignored(), 1);
        setiparam("_matcher_num", 0);
    }

    #[test]
    fn no_ignored_compstate_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setiparam("_matcher_num", 1);
        set_compstate_str("ignored", "0");
        assert_eq!(_ignored(), 1);
    }
}
