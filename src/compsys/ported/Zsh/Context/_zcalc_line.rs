//! Port of `_zcalc_line` from
//! `Completion/Zsh/Context/_zcalc_line`.
//!
//! Full upstream body (81 lines, faithful 1:1):
//! ```text
//! sh: 1  #compdef -zcalc-line-
//! sh: 5  _zcalc_line_escapes() {
//! sh: 7    cmds=( "!:shell escape" "q:quit" … "function:…" )
//! sh:18    cmds=("\:"${^cmds})           # prefix each with a single \:
//! sh:19    _describe -t command-escapes "command escape" cmds -Q
//! sh:22  _zcalc_line() {
//! sh:25    if [[ CURRENT -eq 1 && $words[1] != ":"(\\|)"!"* ]]; then
//! sh:27      (|:*)      command-escapes alternative
//! sh:30      (|[^:]*)   math alternative
//! sh:33      _alternative $alts
//! sh:37    case $words[1] in
//! sh:38    (":"(\\|)"!"*)  shift/compset then _normal
//! sh:49    (:function)     math-functions via compadd else _math
//! sh:61    (:local)        _parameter
//! sh:65    (:(fix|sci|eng)) precision _message ;& fallthrough
//! sh:71    (:*)            "no more arguments" _message
//! sh:75    ([^:]*)         _math
//! sh:78    esac
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getiparam, setaparam, setiparam};
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

/// sh:5 — describe the catalogue of `:cmd` escapes, each prefixed with
/// a single `\:` (sh:18 `cmds=("\:"${^cmds})`).
fn _zcalc_line_escapes() -> i32 {
    // sh:7-17
    let base = [
        "!:shell escape",
        "q:quit",
        "norm:normal output format",
        "sci:scientific output format",
        "fix:fixed point output format",
        "eng:engineering (power of 1000) output format",
        "raw:raw output format",
        "local:make variables local",
        "function:define math function (also \\:func or \\:f)",
    ];
    // sh:18  cmds=("\:"${^cmds})  — rc-expand prepends "\:" to each.
    let cmds: Vec<String> = base.iter().map(|c| format!("\\:{}", c)).collect();
    setaparam("cmds", cmds);
    // sh:19
    dispatch_function_call(
        "_describe",
        &[
            "-t".to_string(),
            "command-escapes".to_string(),
            "command escape".to_string(),
            "cmds".to_string(),
            "-Q".to_string(),
        ],
    )
    .unwrap_or(1)
}

/// sh:49-56 — `${${(k)functions:#^zsh_math_func_*}##zsh_math_func_}`:
/// user math-function names (defined functions whose name begins with
/// `zsh_math_func_`, with that prefix stripped).
///
/// `$functions` is NOT `shfunctab`: it is `scanpmfunctions`
/// (`Src/Modules/parameter.c:530`) → `scanfunctions(…, dis = 0)`, whose
/// walk is gated at c:482
///
/// ```c
/// if (dis ? (hn->flags & DISABLED) : !(hn->flags & DISABLED)) {
/// ```
///
/// so a `disable -f`'d function is absent from `$functions` and present in
/// `$dis_functions` (c:537-541) instead. Walking `shfunctab` sees both —
/// `shfunc_table::iter` has no such filter, unlike its `get`
/// (`hashtable.rs`, `gethashnode` c:239). Measured with
/// `zsh_math_func_aaa`, `zsh_math_func_bbb` and
/// `disable -f zsh_math_func_bbb`, zsh 5.9.2 completes `:function ` inside
/// `zcalc` to `aaa` outright; offering `bbb` too would make it ambiguous.
/// Both shells already agree on the shell-level expression, so only this
/// port diverged.
fn user_math_functions() -> Vec<String> {
    const PFX: &str = "zsh_math_func_";
    // The `$functions` scan itself (with the c:482 NOT-DISABLED gate) is
    // shared with the two other call sites that spell it the same way;
    // only the `##zsh_math_func_` strip is local to sh:55.
    crate::compsys::ported::shared::shfunc_names_with_prefix(PFX)
        .iter()
        .filter_map(|k| k.strip_prefix(PFX).map(|s| s.to_string()))
        .collect()
}

/// sh:38-46 — strip a leading `:!` or `:\!` from `$words[1]`
/// (`${words[1]##:(\\|)\!}`).
fn strip_leading_bang(w: &str) -> String {
    if let Some(rest) = w.strip_prefix(":\\!") {
        rest.to_string()
    } else if let Some(rest) = w.strip_prefix(":!") {
        rest.to_string()
    } else {
        w.to_string()
    }
}

/// `_zcalc_line` — `-zcalc-line-` context: complete a `zcalc` command
/// line read via `vared`.
pub fn _zcalc_line() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_zcalc_line");
    // sh:23  local expl (bound at call sites via `_wanted … expl …`).
    let words = getaparam("words").unwrap_or_default();
    let current = getiparam("CURRENT");
    let word1 = words.first().cloned().unwrap_or_default();

    // sh:25  command position, not a shell escape.
    let is_bang = word1.starts_with(":!") || word1.starts_with(":\\!");
    if current == 1 && !is_bang {
        // sh:26
        let mut alts: Vec<String> = Vec::new();
        // sh:27  [[ $words[1] = (|:*) ]] — empty or starts with ':'.
        if word1.is_empty() || word1.starts_with(':') {
            alts.push("command-escapes:command escape:_zcalc_line_escapes".to_string());
        }
        // sh:30  [[ $words[1] = (|[^:]*) ]] — empty or not starting ':'.
        if word1.is_empty() || !word1.starts_with(':') {
            alts.push("math:math formula:_math".to_string());
        }
        // sh:33  _alternative $alts
        return dispatch_function_call("_alternative", &alts).unwrap_or(1);
    }

    // sh:37  case $words[1] in
    if is_bang {
        // sh:38  (":"(\\|)"!"*)
        if (word1 == ":!" || word1 == ":\\!") && current > 1 {
            // sh:40  shift words; (( CURRENT-- ))
            let mut w = words.clone();
            if !w.is_empty() {
                w.remove(0);
            }
            setaparam("words", w);
            let _ = setiparam("CURRENT", current - 1);
        } else {
            // sh:43  words[1]=${words[1]##:(\\|)\!}; compset -P ':(\\|)!'
            let stripped = strip_leading_bang(&word1);
            let mut w = words.clone();
            if w.is_empty() {
                w.push(stripped);
            } else {
                w[0] = stripped;
            }
            setaparam("words", w);
            // sh:44  compset -P ':(\\|)!' — the shell single-quoted
            //   pattern is the bytes `:(\\|)!` (optional backslash).
            let _ = bin_compset(
                "compset",
                &["-P".to_string(), ":(\\\\|)!".to_string()],
                &make_ops(),
                0,
            );
        }
        // sh:46  _normal
        return dispatch_function_call("_normal", &[]).unwrap_or(1);
    }

    if word1 == ":function" {
        // sh:49  (:function)
        if current == 2 {
            // sh:53-55  _wanted math-functions expl 'math function' \
            //   compadd -- <user math functions>
            let mut argv: Vec<String> = vec![
                "math-functions".to_string(),
                "expl".to_string(),
                "math function".to_string(),
                "compadd".to_string(),
                "--".to_string(),
            ];
            argv.extend(user_math_functions());
            return dispatch_function_call("_wanted", &argv).unwrap_or(1);
        } else {
            // sh:57  _math
            return dispatch_function_call("_math", &[]).unwrap_or(1);
        }
    }

    if word1 == ":local" {
        // sh:61  (:local)  _parameter
        return dispatch_function_call("_parameter", &[]).unwrap_or(1);
    }

    if word1 == ":fix" || word1 == ":sci" || word1 == ":eng" {
        // sh:65  (:(fix|sci|eng))
        if current == 2 {
            // sh:67  _message "precision"
            let _ = dispatch_function_call("_message", &["precision".to_string()]);
        }
        // sh:69  ;& — fall through to (:*).
        return dispatch_function_call("_message", &["no more arguments".to_string()]).unwrap_or(1);
    }

    if word1.starts_with(':') {
        // sh:71  (:*)  _message "no more arguments"
        return dispatch_function_call("_message", &["no more arguments".to_string()]).unwrap_or(1);
    }

    // sh:75  ([^:]*)  _math
    dispatch_function_call("_math", &[]).unwrap_or(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn command_position_returns_via_alternative() {
        // sh:25-33 — empty word in command position dispatches
        //   `_alternative` (returns 1 with no executor).
        let _g = crate::test_util::global_state_lock();
        setaparam("words", vec!["".to_string()]);
        let _ = setsparam("CURRENT", "1");
        assert_eq!(_zcalc_line(), 1);
    }

    #[test]
    fn escapes_prefixed_with_single_colon_escape() {
        // sh:18 — each escape is prefixed with a single `\:`, not `\::`.
        let _g = crate::test_util::global_state_lock();
        let _ = _zcalc_line_escapes();
        let cmds = getaparam("cmds").unwrap_or_default();
        assert_eq!(cmds.len(), 9);
        assert_eq!(cmds[0], "\\:!:shell escape");
        assert_eq!(cmds[5], "\\:eng:engineering (power of 1000) output format");
        assert_eq!(
            cmds[8],
            "\\:function:define math function (also \\:func or \\:f)"
        );
        // Never the doubled `\::` form.
        assert!(!cmds.iter().any(|c| c.starts_with("\\::")));
    }

    #[test]
    fn strip_leading_bang_forms() {
        // sh:43 — both `:!` and `:\!` prefixes are removed.
        assert_eq!(strip_leading_bang(":!ls"), "ls");
        assert_eq!(strip_leading_bang(":\\!ls"), "ls");
        assert_eq!(strip_leading_bang("plain"), "plain");
    }

    /// sh:53's source is `${(k)functions}`, which is the NOT-DISABLED half
    /// of `shfunctab` (c:Src/Modules/parameter.c:482). A `disable -f`'d
    /// math helper belongs to `$dis_functions` and must not be offered —
    /// zsh completes `:function ` to the one enabled name outright, and an
    /// extra candidate here makes it ambiguous instead.
    #[test]
    fn disabled_math_helper_is_not_offered() {
        let _g = crate::test_util::global_state_lock();
        const ON: &str = "zsh_math_func_zzqon";
        const OFF: &str = "zsh_math_func_zzqoff";

        {
            let mut tab = crate::ported::hashtable::shfunctab_lock().write().unwrap();
            for (nam, disabled) in [(ON, false), (OFF, true)] {
                tab.add(crate::ported::zsh_h::shfunc {
                    node: crate::ported::zsh_h::hashnode {
                        next: None,
                        nam: nam.to_string(),
                        flags: if disabled {
                            crate::ported::zsh_h::DISABLED
                        } else {
                            0
                        },
                    },
                    filename: None,
                    lineno: 0,
                    funcdef: None,
                    redir: None,
                    sticky: None,
                    body: None,
                    redir_text: None,
                });
            }
        }

        let names = user_math_functions();
        assert!(
            names.iter().any(|n| n == "zzqon"),
            "enabled helper missing: {:?}",
            names
        );
        assert!(
            !names.iter().any(|n| n == "zzqoff"),
            "DISABLED helper offered — `$functions` never lists it (c:482): {:?}",
            names
        );

        let mut tab = crate::ported::hashtable::shfunctab_lock().write().unwrap();
        tab.remove(ON);
        tab.remove(OFF);
    }
}
