//! Port of `_complete` from `Completion/Base/Completer/_complete`.
//!
//! Full upstream body (145 lines, abridged — line numbers are the
//! vendored `Base/Completer/_complete` next to this file, byte-identical
//! to the installed `/opt/homebrew/share/zsh/functions/_complete`):
//! ```text
//! sh:  1  #autoload
//! sh:  7  local comp name oldcontext ret=1 service
//! sh:  8  typeset -T curcontext="$curcontext" ccarray
//! sh: 10  oldcontext="$curcontext"
//! sh: 14  if [[ -n "$compcontext" ]]; then  # custom context dispatch
//! sh: 16    type-of-compcontext (array | assoc | tag:descr:action | scalar)
//! sh: 85    ccarray[3]="$compcontext"
//! sh: 87    comp="$_comps[$compcontext]"
//! sh: 91    return
//! sh: 92  fi
//! sh: 96  comp="$_comps[-first-]"
//! sh: 97  if [[ -n "$comp" ]]; then
//! sh: 99    ccarray[3]=-first-
//! sh:100    … run -first-, exit if _compskip == all …
//! sh:110  [[ -n $compstate[vared] ]] && compstate[context]=vared
//! sh:115  if [[ "$compstate[context]" = command ]]; then
//! sh:116    curcontext="$oldcontext"
//! sh:117    _normal -s && ret=0
//! sh:118  else
//! sh:122    local cname="-${compstate[context]:s/_/-/}-"
//! sh:124    ccarray[3]="$cname"
//! sh:126    comp="$_comps[$cname]"
//! sh:131    if [[ -z "$comp" ]]; then
//! sh:136      comp="$_comps[-default-]"
//! sh:138    fi
//! sh:139    [[ -n "$comp" ]] && eval "$comp" && ret=0
//! sh:140  fi
//! sh:142  _compskip=
//! sh:144  return ret
//! ```
//!
//! Top-level completer dispatching to context-specific `$_comps`
//! entries. Heavy `compcontext`-based dispatch left as a thin
//! delegation since most live invocations skip that branch.

use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_normal::normal_byname;
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

/// `ccarray[n]=value` under sh:8's `typeset -T curcontext="$curcontext"
/// ccarray` — the array half of the tie, written back through the scalar.
///
/// A `-T` tie with no explicit separator uses `:`, so `ccarray` is just
/// `$curcontext` split on colons and `ccarray[3]` (1-based, hence
/// `n - 1` here) is the COMMAND field of
/// `<function>:<completer>:<command>:<argument>`. Assigning past the end
/// of a zsh array pads the gap with empty elements, then the tie rejoins
/// with `:` — so `curcontext=foo` + `ccarray[3]=bar` yields `foo::bar`,
/// which is what the pad loop below reproduces. An empty `curcontext`
/// splits to one empty field and pads to `::value`, matching zsh.
///
/// The write goes back into the PARAMETER, not a Rust local: every
/// `$_comps` entry dispatched below (and everything it calls —
/// `_tags`/`_description`/`lookupstyle`) reads `$curcontext` to build
/// the style context, which is the whole point of the field.
fn set_ccarray_field(n: usize, value: &str) {
    debug_assert!(n >= 1, "ccarray is 1-based");
    let cur = getsparam("curcontext").unwrap_or_default();
    let mut parts: Vec<&str> = cur.split(':').collect();
    while parts.len() < n {
        parts.push("");
    }
    parts[n - 1] = value;
    let _ = setsparam("curcontext", &parts.join(":"));
}

/// Reach `_complete` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `if _complete; then` (Completion/Base/Completer/_approximate
/// sh:84) — so the normal function lookup runs.
///
/// A plain Rust call to the sibling port skips both of
/// [`crate::compsys::ported::shared::call_compfn`]'s effects: `$fpath` /
/// shfunc arbitration (the user's own copy of the function is inert) and
/// the `doshfunc` frame (no `FUNCSTACK` entry, and the callee's
/// `declare_locals` land in the CALLER's param scope instead of its own).
///
/// The direct call stays as the fallback: it runs only when neither a shell
/// function nor a registered port claims the name — i.e. in unit tests with
/// no executor installed.
pub fn complete_byname() -> i32 {
    crate::compsys::ported::shared::call_compfn("_complete", &[], || _complete())
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
    // sh:8 `typeset -T curcontext="$curcontext" ccarray` — a `typeset`
    // inside a function, so `curcontext` is LOCAL to `_complete` and
    // seeded from the enclosing scope's value (the completer-patched one
    // `_main_complete` built at `_main_complete.rs:736`). Every
    // `ccarray[3]=` below therefore has to be visible to the `$_comps`
    // entry it dispatches and unwound on return, exactly like
    // `_main_complete:31`'s own `curcontext="$curcontext"`
    // (`_main_complete.rs:432`).
    //
    // `LocalScope` rather than a bare `declare_locals_keeping_value`:
    // the value restore then holds on EVERY exit path (the four early
    // `return`s in the `compcontext` branch included) without depending
    // on `doshfunc`'s `endparamscope` firing, so a `-subscript-` written
    // here can never leak into the next completer in the chain.
    let mut _cc_scope = crate::compsys::ported::shared::LocalScope::declare(&[], 0);
    _cc_scope.also_keeping_value(&["curcontext"]);

    let mut ret: i32 = 1;
    // sh:10 `oldcontext="$curcontext"`.
    let oldcontext = getsparam("curcontext").unwrap_or_default();

    // sh:14  compcontext custom dispatch
    let compcontext = getsparam("compcontext").unwrap_or_default();
    tracing::debug!(
        target: "compsys_args",
        %compcontext,
        compskip = %getsparam("_compskip").unwrap_or_default(),
        context = %get_compstate_str("context").unwrap_or_default(),
        "_complete ENTER"
    );
    if !compcontext.is_empty() {
        // sh:32  tag:descr:action form
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
                return message_byname(&["-e".to_string(), tag, descr]);
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
        // sh:85  `ccarray[3]="$compcontext"` — a user-supplied context
        //   name becomes the command field before its `$_comps` entry runs.
        set_ccarray_field(3, &compcontext);
        // sh:87  scalar compcontext — look up $_comps[$compcontext]
        let comp = assoc_get("_comps", &compcontext).unwrap_or_default();
        if !comp.is_empty() {
            return dispatch_function_call(&comp, &[]).unwrap_or(1);
        }
        return 1;
    }

    // sh:96-105  -first- entry
    let first_comp = assoc_get("_comps", "-first-").unwrap_or_default();
    if !first_comp.is_empty() {
        // sh:98  `service="${_services[-first-]:--first-}"`.
        let service = assoc_get("_services", "-first-").unwrap_or_else(|| "-first-".to_string());
        let _ = setsparam("service", &service);
        // sh:99  `ccarray[3]=-first-`.
        set_ccarray_field(3, "-first-");
        if dispatch_function_call(&first_comp, &[]).unwrap_or(1) == 0 {
            ret = 0;
        }
        if getsparam("_compskip").as_deref() == Some("all") {
            let _ = setsparam("_compskip", "");
            return ret;
        }
    }

    // sh:110  vared override
    let vared = get_compstate_str("vared").unwrap_or_default();
    if !vared.is_empty() {
        set_compstate_str("context", "vared");
    }

    // sh:114-140
    let context = get_compstate_str("context").unwrap_or_default();
    if context == "command" {
        // sh:116 `curcontext="$oldcontext"` — undo any `ccarray[3]`
        //   written by the `-first-` branch before handing off to
        //   `_normal`, which builds its own command field.
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
        if normal_byname(&["-s".to_string()]) == 0 {
            ret = 0;
        }
    } else {
        // sh:122 — `local cname="-${compstate[context]:s/_/-/}-"`. The
        //   `:s` modifier substitutes the FIRST match only (`:gs` would
        //   be global), and no `compstate[context]` value carries more
        //   than one underscore (`brace_parameter`, `array_value`), so
        //   the two spellings agree on every live value.
        let cname = format!("-{}-", context.replacen('_', "-", 1));
        // sh:124 `ccarray[3]="$cname"` — THE fix: without this the
        //   command field of `curcontext` stays empty for every
        //   non-`command` context, so `:completion:*:*:-subscript-:*`,
        //   `-parameter-`, `-brace-parameter-`, `-math-`, `-condition-`,
        //   `-value-`, `-redirect-`, `-default-` styles are unreachable.
        //   Measured: `echo $path[<TAB>` reported context
        //   `:completion::megacomplete:::` instead of
        //   `:completion::megacomplete:-subscript-::`, so
        //   `zstyle ':completion:*:*:-subscript-:*' tag-order indexes
        //   parameters` never matched and `_subscript`'s two tags
        //   collapsed into a single pass.
        //
        //   (`-command-` was unaffected because `_normal` writes its own
        //   command field — sh:115's branch above restores `oldcontext`
        //   and hands off.)
        set_ccarray_field(3, &cname);
        let mut comp = assoc_get("_comps", &cname).unwrap_or_default();
        let service = assoc_get("_services", &cname).unwrap_or_else(|| cname.clone());
        let _ = setsparam("service", &service);
        // sh:131  `if [[ -z "$comp" ]]` — fall back to `-default-`.
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

    /// sh:124 `ccarray[3]="$cname"` on the shape `_main_complete` hands
    /// `_complete`: `:<completer>::` → `:<completer>:-subscript-:`, which
    /// is what makes `:completion:*:*:-subscript-:*` styles match.
    #[test]
    fn ccarray_field_three_is_the_command_field() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", ":megacomplete::");
        set_ccarray_field(3, "-subscript-");
        assert_eq!(
            getsparam("curcontext").as_deref(),
            Some(":megacomplete:-subscript-:")
        );
    }

    /// zsh pads a tied array assignment past its end with empty elements
    /// before rejoining on `:` — `curcontext=foo` + `ccarray[3]=bar` is
    /// `foo::bar`, and an empty scalar becomes `::bar`.
    #[test]
    fn ccarray_field_pads_short_context() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", "foo");
        set_ccarray_field(3, "bar");
        assert_eq!(getsparam("curcontext").as_deref(), Some("foo::bar"));

        let _ = setsparam("curcontext", "");
        set_ccarray_field(3, "bar");
        assert_eq!(getsparam("curcontext").as_deref(), Some("::bar"));
    }

    /// Fields past the third survive untouched — the argument field
    /// `_main_complete`/`_normal` may already have filled in must not be
    /// truncated by the command-field write.
    #[test]
    fn ccarray_field_preserves_trailing_fields() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("curcontext", ":complete:oldcmd:argfield");
        set_ccarray_field(3, "-parameter-");
        assert_eq!(
            getsparam("curcontext").as_deref(),
            Some(":complete:-parameter-:argfield")
        );
    }
}
