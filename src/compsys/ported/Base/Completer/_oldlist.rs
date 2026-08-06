//! Port of `_oldlist` from `Completion/Base/Completer/_oldlist`.
//!
//! Full upstream body (57 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  [[ _matcher_num -gt 1 || $_lastcomp[nmatches] -eq 0 ]] && return 1
//! sh: 5  local list
//! sh: 7  zstyle -s ":completion:${curcontext}:" old-list list
//! sh:16  if [[ -n $compstate[old_list] && $list != never &&
//! sh:17        $LASTWIDGET != _complete_help && $WIDGET != _complete_help ]]; then
//! sh:18    if [[ $WIDGETSTYLE = *list* && ( $list = always || $list != shown ) ]]; then
//! sh:19      compstate[old_list]=keep
//! sh:20      return 0
//! sh:21    elif [[ $list = *${_lastcomp[completer]}* ]]; then
//! sh:22      [[ "$_lastcomp[insert]" = unambig* ]] && compstate[to_end]=single
//! sh:23      compstate[old_list]=keep
//! sh:24      if [[ -o automenu ]]; then
//! sh:25        compstate[insert]=menu
//! sh:26      else
//! sh:27        compadd -Qs "$SUFFIX" - "$PREFIX"
//! sh:28      fi
//! sh:29      return 0
//! sh:30    fi
//! sh:31  fi
//! sh:37  if [[ -z $compstate[old_insert] && -n $compstate[old_list] &&
//! sh:38        ( $_lastcomp[nmatches] -ne 0 || $WIDGET != $LASTWIDGET ) &&
//! sh:39        $LASTWIDGET != _complete_help && $WIDGET != _complete_help ]]; then
//! sh:40    compstate[old_list]=keep
//! sh:41    return 0
//! sh:42  elif [[ $WIDGETSTYLE = *complete(|-prefix|-word) ]] &&
//! sh:43       zstyle -T ":completion:${curcontext}:" old-menu; then
//! sh:44    if [[ -n $compstate[old_insert] ]]; then
//! sh:45      compstate[old_list]=keep
//! sh:46      if [[ $WIDGETSTYLE = *reverse* ]]; then
//! sh:47        compstate[insert]=$(( compstate[old_insert] - 1 ))
//! sh:48      else
//! sh:49        compstate[insert]=$(( compstate[old_insert] + 1 ))
//! sh:50      fi
//! sh:51    else
//! sh:52      return 1
//! sh:53    fi
//! sh:54    return 0
//! sh:55  fi
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

    // sh:16
    if !old_list.is_empty()
        && list != "never"
        && lastwidget != "_complete_help"
        && widget != "_complete_help"
    {
        // sh:18
        let style_force_list = list == "always" || list != "shown";
        if widgetstyle.contains("list") && style_force_list {
            set_compstate_str("old_list", "keep");
            return 0;
        }
        // sh:21
        let completer = lastcomp_get("completer").unwrap_or_default();
        if !completer.is_empty() && list.contains(&completer) {
            // sh:22
            let insert_kind = lastcomp_get("insert").unwrap_or_default();
            if insert_kind.starts_with("unambig") {
                set_compstate_str("to_end", "single");
            }
            set_compstate_str("old_list", "keep");
            // sh:24-28
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

    // sh:37
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

    // sh:42
    let widget_is_complete = widgetstyle.contains("complete")
        || widgetstyle.contains("complete-prefix")
        || widgetstyle.contains("complete-word");
    let old_menu_on = testforstyle(&format!(":completion:{}:", curcontext), "old-menu") == 0;
    if widget_is_complete && old_menu_on {
        if !old_insert.is_empty() {
            set_compstate_str("old_list", "keep");
            let oi: i64 = old_insert.parse().unwrap_or(0);
            // sh:46
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
