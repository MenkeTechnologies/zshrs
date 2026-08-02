//! Port of `_oldlist` from `Completion/Base/Completer/_oldlist`.
//!
//! Full upstream body (57 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  [[ _matcher_num -gt 1 || $_lastcomp[nmatches] -eq 0 ]] && return 1
//! sh: 5  local list
//! sh: 7  zstyle -s ":completion:${curcontext}:" old-list list
//! sh:14  if [[ -n $compstate[old_list] && $list != never &&
//! sh:15        $LASTWIDGET != _complete_help && $WIDGET != _complete_help ]]; then
//! sh:16    if [[ $WIDGETSTYLE = *list* && ( $list = always || $list != shown ) ]]; then
//! sh:17      compstate[old_list]=keep
//! sh:18      return 0
//! sh:19    elif [[ $list = *${_lastcomp[completer]}* ]]; then
//! sh:20      [[ "$_lastcomp[insert]" = unambig* ]] && compstate[to_end]=single
//! sh:21      compstate[old_list]=keep
//! sh:22      if [[ -o automenu ]]; then
//! sh:23        compstate[insert]=menu
//! sh:24      else
//! sh:25        compadd -Qs "$SUFFIX" - "$PREFIX"
//! sh:26      fi
//! sh:27      return 0
//! sh:28    fi
//! sh:29  fi
//! sh:34  if [[ -z $compstate[old_insert] && -n $compstate[old_list] &&
//! sh:35        ( $_lastcomp[nmatches] -ne 0 || $WIDGET != $LASTWIDGET ) &&
//! sh:36        $LASTWIDGET != _complete_help && $WIDGET != _complete_help ]]; then
//! sh:37    compstate[old_list]=keep
//! sh:38    return 0
//! sh:39  elif [[ $WIDGETSTYLE = *complete(|-prefix|-word) ]] &&
//! sh:40       zstyle -T ":completion:${curcontext}:" old-menu; then
//! sh:41    if [[ -n $compstate[old_insert] ]]; then
//! sh:42      compstate[old_list]=keep
//! sh:43      if [[ $WIDGETSTYLE = *reverse* ]]; then
//! sh:44        compstate[insert]=$(( compstate[old_insert] - 1 ))
//! sh:45      else
//! sh:46        compstate[insert]=$(( compstate[old_insert] + 1 ))
//! sh:47      fi
//! sh:48    else
//! sh:49      return 1
//! sh:50    fi
//! sh:51    return 0
//! sh:52  fi
//! sh:57  return 1
//! ```

use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{isset, options, AUTOMENU, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:3-7 assoc helper for the flat `_lastcomp` layout
fn lastcomp_get(key: &str) -> Option<String> {
    let arr = getaparam("_lastcomp")?;
    arr.chunks(2)
        .find(|kv| kv.first().map(|k| k == key).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned())
}

/// `_oldlist` — reuse the previous completion list when possible.
pub fn _oldlist() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_oldlist");
    // sh:3
    if getiparam("_matcher_num") > 1 {
        return 1;
    }
    let lastcomp_nmatches: i64 = lastcomp_get("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if lastcomp_nmatches == 0 {
        return 1;
    }

    // sh:7
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let list = lookupstyle(&format!(":completion:{}:", curcontext), "old-list")
        .first()
        .cloned()
        .unwrap_or_default();

    let old_list = get_compstate_str("old_list").unwrap_or_default();
    let widget = getsparam("WIDGET").unwrap_or_default();
    let lastwidget = getsparam("LASTWIDGET").unwrap_or_default();
    let widgetstyle = getsparam("WIDGETSTYLE").unwrap_or_default();

    // sh:14
    if !old_list.is_empty()
        && list != "never"
        && lastwidget != "_complete_help"
        && widget != "_complete_help"
    {
        // sh:16
        let style_force_list = list == "always" || list != "shown";
        if widgetstyle.contains("list") && style_force_list {
            set_compstate_str("old_list", "keep");
            return 0;
        }
        // sh:19
        let completer = lastcomp_get("completer").unwrap_or_default();
        if !completer.is_empty() && list.contains(&completer) {
            // sh:20
            let insert_kind = lastcomp_get("insert").unwrap_or_default();
            if insert_kind.starts_with("unambig") {
                set_compstate_str("to_end", "single");
            }
            set_compstate_str("old_list", "keep");
            // sh:22-26
            if isset(AUTOMENU) {
                set_compstate_str("insert", "menu");
            } else {
                let prefix = getsparam("PREFIX").unwrap_or_default();
                let suffix = getsparam("SUFFIX").unwrap_or_default();
                let argv = vec!["-Qs".to_string(), suffix, "-".to_string(), prefix];
                let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
            }
            return 0;
        }
    }

    // sh:34
    let old_insert = get_compstate_str("old_insert").unwrap_or_default();
    if old_insert.is_empty()
        && !old_list.is_empty()
        && (lastcomp_nmatches != 0 || widget != lastwidget)
        && lastwidget != "_complete_help"
        && widget != "_complete_help"
    {
        set_compstate_str("old_list", "keep");
        return 0;
    }

    // sh:39
    let widget_is_complete = widgetstyle.contains("complete")
        || widgetstyle.contains("complete-prefix")
        || widgetstyle.contains("complete-word");
    let old_menu_on = testforstyle(&format!(":completion:{}:", curcontext), "old-menu") == 0;
    if widget_is_complete && old_menu_on {
        if !old_insert.is_empty() {
            set_compstate_str("old_list", "keep");
            let oi: i64 = old_insert.parse().unwrap_or(0);
            // sh:43
            if widgetstyle.contains("reverse") {
                set_compstate_str("insert", &(oi - 1).to_string());
            } else {
                set_compstate_str("insert", &(oi + 1).to_string());
            }
            return 0;
        } else {
            return 1;
        }
    }

    // sh:57
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
        assert_eq!(_oldlist(), 1);
        setiparam("_matcher_num", 0);
    }
}
