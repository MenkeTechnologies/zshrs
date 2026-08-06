//! Port of `_match` from `Completion/Base/Completer/_match`.
//!
//! Full upstream body (81 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 9  local tmp opm="$compstate[pattern_match]" ret=1 orig ins
//! sh:10  local oms="$_old_match_string"
//! sh:11  local ocsi="$compstate[insert]" ocspi="$compstate[pattern_insert]"
//! sh:15  tmp="${${:-$PREFIX$SUFFIX}#[~=]}"
//! sh:16  [[ "$tmp:q" = "$tmp" ]] && return 1
//! sh:18  _old_match_string="$PREFIX$SUFFIX$HISTNO"
//! sh:20  _tags matches original
//! sh:22  zstyle -s … match-original orig
//! sh:23  zstyle -s … insert-unambiguous ins
//! sh:27  if [[ -n "$orig" ]]; then
//! sh:28    compstate[pattern_match]='-'
//! sh:29    _complete && ret=0
//! sh:30    compstate[pattern_match]="$opm"
//! sh:32    [[ ret -eq 1 && "$orig" = only ]] && return 1
//! sh:33  fi
//! sh:35  if (( ret )); then
//! sh:36    compstate[pattern_match]='*'
//! sh:37    _complete && ret=0
//! sh:38    compstate[pattern_match]="$opm"
//! sh:39  fi
//! sh:41  if (( ! ret )); then
//! sh:43    if [[ "$ins" = pattern && $compstate[nmatches] -gt 1 ]]; then
//! sh:46      [[ "$oms" = "$PREFIX$SUFFIX$HISTNO" &&
//! sh:47         "$compstate[insert]" = automenu-unambiguous ]] &&
//! sh:48          compstate[insert]=automenu
//! sh:49      [[ "$compstate[insert]" != *menu ]] &&
//! sh:50          compstate[pattern_insert]= compstate[insert]=
//! sh:65    fi
//! sh:67    if [[ "$ins" = (true|yes|on|1) && … ]] then …
//! sh:71    elif _requested original && { … }; then
//! sh:74      _description -V original expl original
//! sh:76      compadd "$expl[@]" -U -Q - "$PREFIX$SUFFIX"
//! sh:79  fi
//! sh:81  return ret
//! ```

use crate::compsys::ported::_description::description_byname;
use crate::compsys::ported::_requested::requested_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setsparam};
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

/// `_match` — pattern-matching completer: re-runs `_complete` with
/// `compstate[pattern_match]` set so the user's input can include
/// glob patterns.
pub fn _match() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_match");
    let opm = get_compstate_str("pattern_match").unwrap_or_default();
    let oms = getsparam("_old_match_string").unwrap_or_default();
    let _ocsi = get_compstate_str("insert").unwrap_or_default();
    let _ocspi = get_compstate_str("pattern_insert").unwrap_or_default();
    let mut ret: i32 = 1;

    // sh:15-16  short-circuit when prefix has no special meta chars
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let combined = format!("{}{}", prefix, suffix);
    let stripped = combined.trim_start_matches(|c| c == '~' || c == '=');
    // sh:16 `:q` no-op heuristic: if the trimmed string contains no
    //   glob meta (`* ? [`), return 1.
    if !stripped.chars().any(|c| matches!(c, '*' | '?' | '[')) {
        return 1;
    }

    // sh:18
    let histno = getsparam("HISTNO").unwrap_or_default();
    let _ = setsparam(
        "_old_match_string",
        &format!("{}{}{}", prefix, suffix, histno),
    );

    // sh:20
    let _ = tags_byname(&["matches".to_string(), "original".to_string()]);

    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    // sh:22
    let orig = lookupstyle(&ctx, "match-original")
        .first()
        .cloned()
        .unwrap_or_default();
    // sh:23
    let ins = lookupstyle(&ctx, "insert-unambiguous")
        .first()
        .cloned()
        .unwrap_or_default();

    // sh:27
    if !orig.is_empty() {
        set_compstate_str("pattern_match", "-");
        if dispatch_function_call("_complete", &[]).unwrap_or(1) == 0 {
            ret = 0;
        }
        set_compstate_str("pattern_match", &opm);
        // sh:32
        if ret == 1 && orig == "only" {
            return 1;
        }
    }

    // sh:35
    if ret != 0 {
        set_compstate_str("pattern_match", "*");
        if dispatch_function_call("_complete", &[]).unwrap_or(1) == 0 {
            ret = 0;
        }
        set_compstate_str("pattern_match", &opm);
    }

    // sh:41-79
    if ret == 0 {
        let nm: i64 = get_compstate_str("nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if ins == "pattern" && nm > 1 {
            let new_oms = format!("{}{}{}", prefix, suffix, histno);
            let cur_insert = get_compstate_str("insert").unwrap_or_default();
            if oms == new_oms && cur_insert == "automenu-unambiguous" {
                set_compstate_str("insert", "automenu");
            }
            let cur_insert2 = get_compstate_str("insert").unwrap_or_default();
            if !cur_insert2.ends_with("menu") {
                set_compstate_str("pattern_insert", "");
                set_compstate_str("insert", "");
            }
        }
        // sh:67
        let unambig = get_compstate_str("unambiguous").unwrap_or_default();
        if matches!(ins.as_str(), "true" | "yes" | "on" | "1") && unambig.len() >= combined.len() {
            set_compstate_str("pattern_insert", "unambiguous");
        } else if requested_byname(&["original".to_string()]) == 0 {
            let orig_style_on = lookupstyle(&ctx, "original")
                .first()
                .map(|v| matches!(v.as_str(), "yes" | "true" | "1" | "on"))
                .unwrap_or(false);
            if nm > 1 || orig_style_on {
                // sh:74
                let _ = description_byname(&[
                    "-V".to_string(),
                    "original".to_string(),
                    "expl".to_string(),
                    "original".to_string(),
                ]);
                let expl = getaparam("expl").unwrap_or_default();
                let mut compadd_argv = expl;
                compadd_argv.push("-U".to_string());
                compadd_argv.push("-Q".to_string());
                compadd_argv.push("-".to_string());
                compadd_argv.push(combined.clone());
                let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
            }
        }
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pattern_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "plain");
        let _ = setsparam("SUFFIX", "");
        assert_eq!(_match(), 1);
    }
}
