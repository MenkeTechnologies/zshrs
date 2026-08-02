//! Port of `_prefix` from `Completion/Base/Completer/_prefix`.
//!
//! Full upstream body (62 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 5  [[ _matcher_num -gt 1 || -z "$SUFFIX" ]] && return 1
//! sh: 7  local comp curcontext="$curcontext" tmp suf="$SUFFIX" \
//! sh: 8        _completer _matcher _c_matcher _matchers _matcher_num
//! sh: 9  integer ind
//! sh:11  if ! zstyle -a ":completion:${curcontext}:" completer comp; then
//! sh:12    comp=( "${(@)_completers[1,_completer_num-1]}" )
//! sh:13    ind=${comp[(I)_prefix(|:*)]}
//! sh:14    (( ind )) && comp=("${(@)comp[ind,-1]}")
//! sh:15  fi
//! sh:17  if zstyle -t ":completion:${curcontext}:" add-space; then
//! sh:18    ISUFFIX=" $SUFFIX"
//! sh:19  else
//! sh:20    ISUFFIX="$SUFFIX"
//! sh:21  fi
//! sh:22  SUFFIX=''
//! sh:24  local _completer_num=1
//! sh:26  for tmp in "$comp[@]"; do
//! sh:43    if [[ "$tmp" != _prefix ]] && "$tmp"; then
//! sh:44      if [[ -n $compstate[old_list] || ${compstate[unambiguous]%$suf} == $PREFIX ]]; then
//! sh:45        compstate[to_end]=match
//! sh:46      fi
//! sh:47      return 0
//! sh:48    fi
//! sh:51  done
//! sh:53  return 1
//! ```
//!
//! The inner completer-list iteration (sh:26-51) dispatches each
//! completer name via `crate::ported::exec::dispatch_function_call`. The
//! style-list / matcher-list walks are simplified — we honor the
//! `completer` style if set, else use the prior-rounds slice of
//! `$_completers` per sh:12-14.

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getiparam, getsparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// `_prefix` — try the next completer chain after ignoring the
/// current `$SUFFIX`. Returns 0 on first successful completer.
pub fn _prefix() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_prefix");
    // sh:5
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    if getiparam("_matcher_num") > 1 || suffix.is_empty() {
        return 1;
    }

    // sh:7 snapshots for shell-local restore
    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let saved_isuffix = getsparam("ISUFFIX").unwrap_or_default();
    let saved_suffix = suffix.clone();

    // sh:11-15
    let ctx = format!(":completion:{}:", saved_curcontext);
    let comp_style = lookupstyle(&ctx, "completer");
    let comp_list: Vec<String> = if !comp_style.is_empty() {
        comp_style
    } else {
        let completers = getaparam("_completers").unwrap_or_default();
        let comp_num = getiparam("_completer_num") as usize;
        let upto = comp_num.saturating_sub(1).min(completers.len());
        let slice = &completers[..upto];
        // Skip everything before "_prefix(:.*)?"
        let ind = slice
            .iter()
            .position(|c| c == "_prefix" || c.starts_with("_prefix:"));
        match ind {
            Some(i) => slice[i..].to_vec(),
            None => slice.to_vec(),
        }
    };

    // sh:17-21
    if testforstyle(&ctx, "add-space") == 0 {
        let _ = setsparam("ISUFFIX", &format!(" {}", suffix));
    } else {
        let _ = setsparam("ISUFFIX", &suffix);
    }
    // sh:22
    let _ = setsparam("SUFFIX", "");

    // sh:26-51 — iterate completers
    for tmp in &comp_list {
        // Skip recursion into _prefix itself
        let bare = tmp.split(':').next().unwrap_or(tmp);
        if bare == "_prefix" {
            continue;
        }
        if dispatch_function_call(bare, &[]).unwrap_or(1) == 0 {
            // sh:44-46
            let old_list = get_compstate_str("old_list").unwrap_or_default();
            let unambig = get_compstate_str("unambiguous").unwrap_or_default();
            let unambig_no_suf = unambig.trim_end_matches(&saved_suffix as &str);
            let prefix = getsparam("PREFIX").unwrap_or_default();
            if !old_list.is_empty() || unambig_no_suf == prefix {
                set_compstate_str("to_end", "match");
            }
            // Restore + return success
            let _ = setsparam("curcontext", &saved_curcontext);
            let _ = setsparam("ISUFFIX", &saved_isuffix);
            let _ = setsparam("SUFFIX", &saved_suffix);
            return 0;
        }
    }

    // sh:53 restore + fail
    let _ = setsparam("curcontext", &saved_curcontext);
    let _ = setsparam("ISUFFIX", &saved_isuffix);
    let _ = setsparam("SUFFIX", &saved_suffix);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_suffix_returns_one() {
        // sh:5
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("SUFFIX", "");
        assert_eq!(_prefix(), 1);
    }

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("SUFFIX", "rest");
        crate::ported::params::setiparam("_matcher_num", 1);
        assert_eq!(_prefix(), 1);
    }
}
