//! Port of `_complete` from `Completion/Base/Completer/_complete`.
//!
//! Full upstream body (144 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  7  local comp name oldcontext ret=1 service
//! sh: 12  if [[ -n "$compcontext" ]]; then  # custom context dispatch
//! sh: 14    type-of-compcontext (array | assoc | tag:descr:action | scalar)
//! sh: 92    return
//! sh: 95  fi
//! sh: 96  comp="$_comps[-first-]"
//! sh: 97  if [[ -n "$comp" ]]; then … run -first-, exit if _compskip == all …
//! sh:108  [[ -n $compstate[vared] ]] && compstate[context]=vared
//! sh:111  if [[ "$compstate[context]" = command ]]; then
//! sh:112    _normal -s && ret=0
//! sh:113  else
//! sh:115    local cname="-${compstate[context]:s/_/-/}-"
//! sh:117    comp="$_comps[$cname]"
//! sh:122    if [[ -z "$comp" ]]; then
//! sh:127      comp="$_comps[-default-]"
//! sh:130    fi
//! sh:131    [[ -n "$comp" ]] && eval "$comp" && ret=0
//! sh:132  fi
//! sh:134  _compskip=
//! sh:136  return ret
//! ```
//!
//! Top-level completer dispatching to context-specific `$_comps`
//! entries. Heavy `compcontext`-based dispatch left as a thin
//! delegation since most live invocations skip that branch.

use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_normal::_normal;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// Helper: flat assoc lookup.
fn assoc_get(name: &str, key: &str) -> Option<String> {
    // `_comps`/`_services` are PM_HASHED associative arrays, NOT flat arrays,
    // so `getaparam` returns nothing for them — read the hashed storage
    // directly (same fix as `_dispatch::assoc_get`). With the old getaparam
    // path, `$_comps[-parameter-]` came back empty during completion even
    // though the shell saw it set, so every non-command context (parameter,
    // brace_parameter, value, condition, …) fell through to nothing —
    // `$var<TAB>`, `${...}<TAB>`, `((expr<TAB>` all produced no matches.
    crate::ported::params::paramtab_hashed_storage()
        .lock()
        .ok()?
        .get(name)?
        .get(key)
        .cloned()
}

/// `_complete` — primary `completer` entry: dispatches to per-context
/// `$_comps` entries based on `$compstate[context]`.
pub fn _complete() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_complete");
    // sh:7 `local comp name oldcontext ret=1 service`.
    //
    // `comp`/`name`/`oldcontext`/`ret` are Rust locals here, but `service`
    // is a real shell parameter: the port writes it with `setsparam` below
    // and every `$_comps` entry reads it. Without the declaration that write
    // landed at level 0, so `service` survived the completion AND read back
    // as `scalar` instead of `scalar-local` — `_parameters` filters on
    // `[(R)…~*local*]`, so `${<TAB>` / `$fpath[<TAB>` offered `service` as a
    // parameter name and zsh does not. The other four are declared for the
    // same reason `_main_complete` declares its whole `local` line: a
    // `$parameters`/`typeset -p` read during completion must see what zsh
    // sees. `_complete` is reached through `dispatch_function_call`, so
    // `doshfunc`'s `endparamscope` performs the unwind.
    {
        use crate::compsys::ported::shared::declare_locals;
        declare_locals(&["comp", "name", "oldcontext", "ret", "service"], 0);
    }
    let mut ret: i32 = 1;
    let oldcontext = getsparam("curcontext").unwrap_or_default();

    // sh:12  compcontext custom dispatch
    let compcontext = getsparam("compcontext").unwrap_or_default();
    tracing::debug!(
        target: "compsys_args",
        %compcontext,
        compskip = %getsparam("_compskip").unwrap_or_default(),
        context = %get_compstate_str("context").unwrap_or_default(),
        "_complete ENTER"
    );
    if !compcontext.is_empty() {
        // sh:30  tag:descr:action form
        if compcontext.matches(':').count() >= 2 {
            let mut parts = compcontext.splitn(3, ':');
            let tag = parts
                .next()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "values".to_string());
            let descr = parts
                .next()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "value".to_string());
            let action = parts.next().unwrap_or("").to_string();
            if action.trim().is_empty() {
                return _message(&["-e".to_string(), tag, descr]);
            }
            // Fall through to the generic dispatch via _alternative-
            //   style action: treat `action` as a command.
            // C _complete sh:61/72 `eval ws=( "$action" )` — quote-respecting split.
            let parts: Vec<String> = crate::compsys::ported::eval_action_words(&action);
            if let Some((cmd, rest)) = parts.split_first() {
                return dispatch_function_call(cmd, rest).unwrap_or(1);
            }
            return 1;
        }
        // sh:86  scalar compcontext — look up $_comps[$compcontext]
        let comp = assoc_get("_comps", &compcontext).unwrap_or_default();
        if !comp.is_empty() {
            return dispatch_function_call(&comp, &[]).unwrap_or(1);
        }
        return 1;
    }

    // sh:96-107  -first- entry
    let first_comp = assoc_get("_comps", "-first-").unwrap_or_default();
    if !first_comp.is_empty() {
        let service = assoc_get("_services", "-first-").unwrap_or_else(|| "-first-".to_string());
        let _ = setsparam("service", &service);
        if dispatch_function_call(&first_comp, &[]).unwrap_or(1) == 0 {
            ret = 0;
        }
        if getsparam("_compskip").as_deref() == Some("all") {
            let _ = setsparam("_compskip", "");
            return ret;
        }
    }

    // sh:108  vared override
    let vared = get_compstate_str("vared").unwrap_or_default();
    if !vared.is_empty() {
        set_compstate_str("context", "vared");
    }

    // sh:111-132
    let context = get_compstate_str("context").unwrap_or_default();
    if context == "command" {
        let _ = setsparam("curcontext", &oldcontext);
        // Direct Rust call, not a shell-function dispatch: `_normal`
        // therefore contributes no `$funcstack` entry and no
        // `$zsh_eval_context` frames, which is two of the six frames
        // zshrs is short inside a live completion. Deliberate — see
        // docs/COMPLETION_DISPATCH.md, Divergence C — and the two
        // stacks stay consistent with each other precisely BECAUSE
        // neither is faked here.
        //
        // Known consequence: an fpath `_normal` (user or plugin
        // override) is bypassed, because `router::try_rust_dispatch`'s
        // `has_fpath_override` gate only runs on the shell-function
        // dispatch path. Same bug class as the `_command_names`
        // override fix; tracked separately from Divergence C.
        if _normal(&["-s".to_string()]) == 0 {
            ret = 0;
        }
    } else {
        // sh:115 — `-${context//_/-}-`
        let cname = format!("-{}-", context.replace('_', "-"));
        let mut comp = assoc_get("_comps", &cname).unwrap_or_default();
        let service = assoc_get("_services", &cname).unwrap_or_else(|| cname.clone());
        let _ = setsparam("service", &service);
        // sh:122
        if comp.is_empty() {
            let cs = getsparam("_compskip").unwrap_or_default();
            if cs.contains("default") {
                let _ = setsparam("_compskip", "");
                return 1;
            }
            comp = assoc_get("_comps", "-default-").unwrap_or_default();
            let default_service =
                assoc_get("_services", "-default-").unwrap_or_else(|| "-default-".to_string());
            let _ = setsparam("service", &default_service);
        }
        if !comp.is_empty() {
            if dispatch_function_call(&comp, &[]).unwrap_or(1) == 0 {
                ret = 0;
            }
        }
    }
    let _ = setsparam("_compskip", "");
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("compcontext", "");
        set_compstate_str("context", "command");
        crate::ported::params::setaparam("_comps", Vec::new());
        let _r = _complete();
    }
}
