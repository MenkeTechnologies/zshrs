//! Port of `_in_vared` from `Completion/Zsh/Context/_in_vared`.
//!
//! Full upstream body (35 lines verbatim):
//! ```text
//! sh: 1  #compdef -vared-
//! sh: 3  local also
//! sh: 7  if [[ $compstate[vared] = *\[* ]]; then
//! sh: 8    if [[ $compstate[vared] = *\]* ]]; then
//! sh: 9      # vared on an array-element
//! sh:10      compstate[parameter]=${${compstate[vared]%%\]*}//\[/-}
//! sh:11      compstate[context]=value
//! sh:12      also=-value-
//! sh:13    else
//! sh:14      # vared on an array-value
//! sh:15      compstate[parameter]=${compstate[vared]%%\[*}
//! sh:16      compstate[context]=value
//! sh:17      also=-value-
//! sh:18    fi
//! sh:19  else
//! sh:20    # vared on a parameter, let's see if it is an array
//! sh:21    compstate[parameter]=$compstate[vared]
//! sh:22    if [[ ${(tP)compstate[vared]} = *(array|assoc)* ]]; then
//! sh:23      compstate[context]=array_value
//! sh:24      also=-array-value-
//! sh:25    else
//! sh:26      compstate[context]=value
//! sh:27      also=-value-
//! sh:28    fi
//! sh:29  fi
//! sh:33  # Don't insert TAB in first column.
//! sh:34  compstate[insert]="${compstate[insert]//tab /}"
//! sh:36  _dispatch "$also" "$also"
//! ```

use crate::ported::exec_hooks::dispatch_function_call;
use crate::ported::params::getsparam;
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// `_in_vared` — `vared` widget context: classify the parameter
/// being edited (scalar / array element / whole array) and dispatch
/// the appropriate `-value-` / `-array-value-` context.
pub fn _in_vared() -> i32 {
    let vared = get_compstate_str("vared").unwrap_or_default();
    let also: String;

    // sh:7
    if vared.contains('[') {
        if vared.contains(']') {
            // sh:9-12  vared on array element
            let head = vared.splitn(2, ']').next().unwrap_or("").replace('[', "-");
            set_compstate_str("parameter", &head);
            set_compstate_str("context", "value");
            also = "-value-".to_string();
        } else {
            // sh:14-17  vared on array-value (mid-edit)
            let head = vared.splitn(2, '[').next().unwrap_or("");
            set_compstate_str("parameter", head);
            set_compstate_str("context", "value");
            also = "-value-".to_string();
        }
    } else {
        // sh:20-28  bare parameter
        set_compstate_str("parameter", &vared);
        // sh:22  ${(tP)compstate[vared]} — type-of (typeset-style) for
        //   the named param. Read directly from the param table:
        //   array param if `parameter` reports `array`/`association`.
        let raw_type = param_type(&vared);
        if raw_type.contains("array") || raw_type.contains("assoc") {
            set_compstate_str("context", "array_value");
            also = "-array-value-".to_string();
        } else {
            set_compstate_str("context", "value");
            also = "-value-".to_string();
        }
    }

    // sh:34
    let insert = get_compstate_str("insert").unwrap_or_default();
    let cleaned = insert.replace("tab ", "");
    set_compstate_str("insert", &cleaned);

    // sh:36
    dispatch_function_call("_dispatch", &[also.clone(), also]).unwrap_or(1)
}

/// sh:22 — return zsh-style typeset code for parameter `name` (or
/// empty if unset). Approximation: check whether the name exists as
/// an array param.
fn param_type(name: &str) -> String {
    if crate::ported::params::getaparam(name).is_some() {
        // Likely array; assoc detection requires the params crate to
        //   surface the type-flag bits, which it doesn't simply
        //   expose. "array" suffices for the sh:22 substring test.
        "array".to_string()
    } else if getsparam(name).is_some() {
        "scalar".to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setaparam;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("vared", "myvar");
        assert_eq!(_in_vared(), 1);
    }

    #[test]
    fn array_param_sets_array_value_context() {
        // sh:22-24
        let _g = crate::test_util::global_state_lock();
        setaparam("myarr", vec!["a".to_string(), "b".to_string()]);
        set_compstate_str("vared", "myarr");
        let _ = _in_vared();
        assert_eq!(
            get_compstate_str("context").as_deref(),
            Some("array_value")
        );
    }

    #[test]
    fn bracketed_vared_uses_value_context() {
        // sh:10-12
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("vared", "myarr[key]");
        let _ = _in_vared();
        assert_eq!(get_compstate_str("context").as_deref(), Some("value"));
        assert_eq!(get_compstate_str("parameter").as_deref(), Some("myarr-key"));
    }
}
