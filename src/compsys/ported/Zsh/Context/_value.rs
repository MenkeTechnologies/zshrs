//! Port of `_value` from `Completion/Zsh/Context/_value`.
//!
//! Full upstream body (50 lines verbatim, abridged):
//! ```text
//! sh: 1  #compdef -value- -array-value- -value-,-default-,-default-
//! sh: 9  if [[ "$service" != -value-,* ]]; then
//! sh:10    local strs ctx=
//! sh:12    strs=( -default- )
//! sh:14    if [[ "$compstate[context]" != *value && -n "$_comp_command1" ]]; then
//! sh:15      ctx="${_comp_command}"
//! sh:16      strs=( "${_comp_command1}" "$strs[@]" )
//! sh:17      [[ -n "$_comp_command2" ]] &&
//! sh:18          strs=( "${_comp_command2}" "$strs[@]" )
//! sh:19    fi
//! sh:21    _dispatch -value-,${compstate[parameter]},$ctx \
//! sh:22              -value-,{${compstate[parameter]},-default-},${^strs}
//! sh:23  else
//! sh:24    if [[ "$compstate[parameter]" != *-* &&
//! sh:25          "$compstate[context]" = array_value &&
//! sh:26          "${(Pt)${compstate[parameter]}}" = assoc* ]]; then
//! sh:28      if (( CURRENT & 1 )); then
//! sh:29        _wanted association-keys expl 'association key' \
//! sh:30            compadd -k "$compstate[parameter]"
//! sh:31      else
//! sh:32        compstate[parameter]="${compstate[parameter]}-${words[CURRENT-1]}"
//! sh:34        _dispatch -value-,${compstate[parameter]}, \
//! sh:35                  -value-,{${compstate[parameter]},-default-},-default-
//! sh:36      fi
//! sh:37    else
//! sh:38      local pats
//! sh:40      if { zstyle -a … assign-list pats && [[ … = pats ]] } ||
//! sh:42         [[ "$PREFIX$SUFFIX" = *:* ]]; then
//! sh:43        compset -P '*:'
//! sh:44        compset -S ':*'
//! sh:45        _default -r '\-\n\t /:' "$@"
//! sh:46      else
//! sh:47        _default "$@"
//! sh:48      fi
//! sh:49    fi
//! sh:50  fi
//! ```

use crate::compsys::ported::_default::_default;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getiparam, getsparam, setsparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
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

/// `_value` — `-value-` / `-array-value-` context entry.
pub fn _value(args: &[String]) -> i32 {
    let service = getsparam("service").unwrap_or_default();

    // sh:9 — outer service != -value-,* (top-level dispatch)
    if !service.starts_with("-value-,") {
        let mut strs: Vec<String> = vec!["-default-".to_string()];
        let mut ctx = String::new();

        let comp_context = get_compstate_str("context").unwrap_or_default();
        let cc1 = getsparam("_comp_command1").unwrap_or_default();
        let cc2 = getsparam("_comp_command2").unwrap_or_default();
        let cc = getsparam("_comp_command").unwrap_or_default();
        if !comp_context.ends_with("value") && !cc1.is_empty() {
            ctx = cc;
            strs.insert(0, cc1);
            if !cc2.is_empty() {
                strs.insert(0, cc2);
            }
        }

        // sh:21-22 — build dispatch argv by brace-expanding the two
        //   `-value-,{P,-default-},${^strs}` matrix.
        let param = get_compstate_str("parameter").unwrap_or_default();
        let mut argv: Vec<String> = vec![format!("-value-,{},{}", param, ctx)];
        for s in &strs {
            argv.push(format!("-value-,{},{}", param, s));
            argv.push(format!("-value-,-default-,{}", s));
        }
        return dispatch_function_call("_dispatch", &argv).unwrap_or(1);
    }

    // sh:23 — inner -value-,* dispatch
    let param = get_compstate_str("parameter").unwrap_or_default();
    let context = get_compstate_str("context").unwrap_or_default();
    if !param.contains('-') && context == "array_value" && param_is_assoc(&param) {
        // sh:28
        let current = getiparam("CURRENT");
        if current & 1 == 1 {
            // sh:29
            return _wanted(&[
                "association-keys".to_string(),
                "expl".to_string(),
                "association key".to_string(),
                "compadd".to_string(),
                "-k".to_string(),
                param,
            ]);
        }
        // sh:32 — extend param with previous word
        let words = getaparam("words").unwrap_or_default();
        let cur_idx = current as usize;
        let prev = if cur_idx >= 2 && cur_idx - 1 <= words.len() {
            words[cur_idx - 2].clone()
        } else {
            String::new()
        };
        let new_param = format!("{}-{}", param, prev);
        set_compstate_str("parameter", &new_param);
        let argv = vec![
            format!("-value-,{},", new_param),
            format!("-value-,{},-default-", new_param),
            "-value-,-default-,-default-".to_string(),
        ];
        return dispatch_function_call("_dispatch", &argv).unwrap_or(1);
    }

    // sh:38-47
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let pats = lookupstyle(&format!(":completion:{}:", curcontext), "assign-list");
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    let assign_match = if !pats.is_empty() {
        let joined = pats.join("|");
        match patcompile(&{ let mut __pat_tok = (&joined).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, 0, None) {
            Some(p) => pattry(&p, &param),
            None => false,
        }
    } else {
        false
    };
    let has_colon = format!("{}{}", prefix, suffix).contains(':');
    if assign_match || has_colon {
        // sh:43-45
        let _ = bin_compset(
            "compset",
            &["-P".to_string(), "*:".to_string()],
            &make_ops(),
            0,
        );
        let _ = bin_compset(
            "compset",
            &["-S".to_string(), ":*".to_string()],
            &make_ops(),
            0,
        );
        let mut a: Vec<String> = vec!["-r".to_string(), "\\-\\n\\t /:".to_string()];
        a.extend(args.iter().cloned());
        _default(&a)
    } else {
        _default(args)
    }
}

/// sh:26 — `(Pt)param` check for assoc-array type. Approximate via
/// the param table: if the named param exists as an assoc-backed
/// value in `paramtab_hashed_storage`, treat as assoc.
fn param_is_assoc(name: &str) -> bool {
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .map(|tab| tab.contains_key(name))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outer_dispatch_returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("service", "-value-");
        let _ = setsparam("_comp_command", "");
        set_compstate_str("context", "value");
        set_compstate_str("parameter", "myvar");
        assert_eq!(_value(&[]), 1);
    }
}
