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

use crate::ported::exec::dispatch_function_call;
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
        assert_eq!(get_compstate_str("context").as_deref(), Some("array_value"));
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

    // ========================================================
    // param_type — array / scalar / unset classification
    // ========================================================

    #[test]
    fn param_type_returns_array_when_array_set() {
        let _g = crate::test_util::global_state_lock();
        setaparam("arr_x", vec!["a".to_string(), "b".to_string()]);
        assert_eq!(param_type("arr_x"), "array");
    }

    #[test]
    fn param_type_returns_scalar_when_only_scalar_set() {
        let _g = crate::test_util::global_state_lock();
        // Ensure no array shadow exists first.
        crate::ported::params::unsetparam("scal_x");
        let _ = crate::ported::params::setsparam("scal_x", "hello");
        assert_eq!(param_type("scal_x"), "scalar");
    }

    #[test]
    fn param_type_returns_empty_for_unset_name() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("never_set_param_ever");
        assert_eq!(param_type("never_set_param_ever"), "");
    }

    // ========================================================
    // _in_vared — half-bracketed (open without close) branch
    // ========================================================

    #[test]
    fn half_bracket_vared_strips_array_subscript_prefix() {
        // sh:14-17  vared="arr[partial" (mid-edit, no closing `]`)
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("vared", "arr[partial");
        let _ = _in_vared();
        assert_eq!(get_compstate_str("context").as_deref(), Some("value"));
        assert_eq!(get_compstate_str("parameter").as_deref(), Some("arr"));
    }

    #[test]
    fn scalar_param_sets_value_context_not_array_value() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("scalar_param_for_context");
        let _ = crate::ported::params::setsparam("scalar_param_for_context", "x");
        set_compstate_str("vared", "scalar_param_for_context");
        let _ = _in_vared();
        assert_eq!(get_compstate_str("context").as_deref(), Some("value"));
    }

    #[test]
    fn unset_param_falls_into_scalar_value_branch() {
        // sh:25-27  bare param, unset → param_type empty → scalar branch
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("doesnt_exist_anywhere");
        set_compstate_str("vared", "doesnt_exist_anywhere");
        let _ = _in_vared();
        // Context is "value" because the empty type doesn't contain
        // "array" or "assoc".
        assert_eq!(get_compstate_str("context").as_deref(), Some("value"));
    }

    #[test]
    fn vared_empty_string_sets_value_context() {
        // Empty vared field — falls into the bare-param branch with
        // empty name → param_type("") = empty → scalar.
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("");
        set_compstate_str("vared", "");
        let _ = _in_vared();
        assert_eq!(get_compstate_str("context").as_deref(), Some("value"));
    }

    // ========================================================
    // _in_vared — `tab ` removal from compstate[insert]
    // ========================================================

    #[test]
    fn insert_field_has_tab_token_stripped() {
        // sh:34  insert="${insert//tab /}" — every "tab " run drops.
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("vared", "");
        set_compstate_str("insert", "tab menu");
        let _ = _in_vared();
        assert_eq!(get_compstate_str("insert").as_deref(), Some("menu"));
    }

    #[test]
    fn insert_field_without_tab_unchanged() {
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("vared", "");
        set_compstate_str("insert", "menu only");
        let _ = _in_vared();
        assert_eq!(get_compstate_str("insert").as_deref(), Some("menu only"));
    }

    #[test]
    fn bracketed_with_close_inverts_brackets_to_dash() {
        // sh:10  parameter="${${compstate[vared]%%\]*}//\[/-}"
        // Multiple brackets in the prefix must each become `-`.
        let _g = crate::test_util::global_state_lock();
        set_compstate_str("vared", "outer[inner]extra]");
        let _ = _in_vared();
        // outer[inner]extra] → take pre-first-`]` = "outer[inner"
        // then replace `[` with `-` → "outer-inner".
        assert_eq!(
            get_compstate_str("parameter").as_deref(),
            Some("outer-inner")
        );
    }
}
