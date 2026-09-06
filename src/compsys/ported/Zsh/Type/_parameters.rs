//! Port of `_parameters` from `Completion/Zsh/Type/_parameters`.
//!
//! Full upstream body (58 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 9  local i pfilt
//! sh:10  local -i nm=$compstate[nmatches]
//! sh:11  local -a expl pattern=( -g \* ) normal described verbose faked fakes tmp
//! sh:14  zstyle -t ":completion:${curcontext}:parameters" prefix-needed &&
//! sh:15      [[ $PREFIX != [_.]* ]] &&
//! sh:16          pfilt='[_.]*'
//! sh:18  [[ $IPREFIX = *\$ ]] && pfilt+='|*.*'
//! sh:20  _description parameters expl parameter
//! sh:21  zparseopts -D -K -E g:=pattern
//! sh:23  if zstyle -t ":completion:${curcontext}:parameters" extra-verbose; then
//! sh:24    described=(
//! sh:25        ${(k)parameters[(R)$~pattern[2]~*(hideval|local|special)*]:#$~pfilt}
//! sh:26    )
//! sh:27    compadd "$@" "$expl[@]" -D described -a - described
//! sh:28    if (( $#described )); then
//! sh:33      verbose=(
//! sh:34          ${${${(f@)"$( typeset -m ${(@b)described} )"}/=/:}[@]//'\'/'\\'}
//! sh:35      )
//! sh:36      _describe -t parameters parameter verbose "$@" "$expl[@]"
//! sh:37    fi
//! sh:39    normal=(
//! sh:40        ${(k)parameters[(R)$~pattern[2]~^(*(hideval|special)*)~*local*]:#$~pfilt}
//! sh:41    )
//! sh:42  else
//! sh:43    normal=( ${(k)parameters[(R)${~pattern[2]}~*local*]:#$~pfilt} )
//! sh:44  fi
//! sh:46  if zstyle -a ":completion:${curcontext}:" fake-parameters tmp; then …
//! sh:55  compadd "$@" "$expl[@]" - "$normal[@]" "${(@)fakes:|described}" \
//! sh:56      "${(@)${(@)${(@M)faked:#${~pattern[2]}}%%:*}:|described}"
//! sh:58  (( compstate[nmatches] > nm ))
//! ```
//!
//! `$parameters` is the shell-side assoc-array mapping param name
//! to its zsh-type string ("integer", "array", "scalar-export", …).
//! We enumerate from `paramtab` directly, rendering each value with the
//! REAL `paramtypestr` (`Src/Modules/parameter.c:43`) — the same function
//! that backs the `parameter` module's `$parameters` — so the `(R)pattern`
//! glob sees the full modifier suffix chain (`-local`, `-readonly`,
//! `-export`, `-hideval`, `-special`, …). A previous revision hand-rolled a
//! bare `PM_TYPE`-only string; every upstream filter that keys on a
//! modifier (`~*(hideval|local|special)*` at sh:25, `~^(*(hideval|special)*)`
//! at sh:40, `_command_names` sh:39's `^*(readonly|association)*`) silently
//! matched the wrong set against it.
//!
//! `$~pfilt` excludes names matching that pattern.

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::shared::zstyle_t;
use crate::ported::modules::zutil::{bin_zparseopts, lookupstyle};
use crate::ported::params::{getaparam, getsparam, paramtab, setaparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zle::compcore::get_compstate_str;
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

/// Iterate paramtab, returning (name, zsh-type-string, flags) triples —
/// the Rust view of the `$parameters` assoc the shell source reads.
///
/// The type string comes from the REAL
/// [`crate::ported::modules::parameter::paramtypestr`]
/// (`Src/Modules/parameter.c:43`), which is what `getpmparameter`
/// (c:116) stores as each `$parameters` value. The modifier suffixes it
/// appends (`-local` c:63, `-readonly` c:75, `-export` c:81,
/// `-hideval` c:87, `-special` c:89, …) are load-bearing: sh:25 and
/// sh:40 partition the parameter set on exactly those substrings.
///
/// c:48 — a PM_UNSET param renders as the empty string; c:49 — a
/// PM_AUTOLOAD one as "undefined". Both fall out of `paramtypestr`
/// itself, so no filtering happens here.
fn enumerate_params() -> Vec<(String, String, i32)> {
    let mut out: Vec<(String, String, i32)> = Vec::new();
    if let Ok(tab) = paramtab().read() {
        for (name, pm) in tab.iter() {
            let flags = pm.node.flags as i32;
            let ty = crate::ported::modules::parameter::paramtypestr(pm);
            out.push((name.clone(), ty, flags));
        }
    }
    out
}

/// Call `_parameters` by NAME, the way the upstream shell code does.
///
/// The shell contexts that end in `_parameters` (`_brace_parameter`,
/// `_subscript`, `_parameter`, …) write a plain command word, so `$fpath`
/// arbitration applies: a user's or plugin's own `_parameters` file is
/// autoloaded instead of the stock one. `dispatch_function_call` runs that
/// arbitration (`compsys::router::try_rust_dispatch` → `has_fpath_override`);
/// calling [`_parameters`] as a Rust fn skips it and pins the port, which
/// silently kills the override. Falls back to the port when there is no
/// executor in scope (unit tests).
pub fn call_parameters(args: &[String]) -> i32 {
    crate::ported::exec::dispatch_function_call("_parameters", args)
        .unwrap_or_else(|| _parameters(args))
}

/// `_parameters` — complete non-local parameter names. `-g <pat>`
/// filters by parameter type-string.
///
/// Callers inside other ported completers must use [`call_parameters`], not
/// this fn, so an `$fpath` override still wins.
pub fn _parameters(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_parameters");
    // sh:10  local -i nm=$compstate[nmatches]
    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    // sh:11
    let mut pattern_seed: Vec<String> = vec!["-g".to_string(), "*".to_string()];

    // sh:14-16  prefix-needed handling
    let curcontext = getsparam("curcontext").unwrap_or_default();
    // sh:18 — `zstyle -t … prefix-needed`, a VALUE test; see [`zstyle_t`].
    let prefix_needed = zstyle_t(
        &format!(":completion:{}:parameters", curcontext),
        "prefix-needed",
    ) == 0;
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let mut pfilt = String::new();
    if prefix_needed && !prefix.starts_with('_') && !prefix.starts_with('.') {
        pfilt = "[_.]*".to_string();
    }
    // sh:18
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    if iprefix.ends_with('$') {
        if pfilt.is_empty() {
            pfilt = "*.*".to_string();
        } else {
            pfilt.push_str("|*.*");
        }
    }

    // sh:20
    let _ = _description(&[
        "parameters".to_string(),
        "expl".to_string(),
        "parameter".to_string(),
    ]);

    // sh:21  zparseopts -D -K -E g:=pattern
    let src = "__compsys_argv";
    crate::compsys::ported::shared::set_bridge_argv(src, args);
    setaparam("pattern", pattern_seed.clone());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-K".to_string(),
            "-E".to_string(),
            "-v".to_string(),
            src.to_string(),
            "g:=pattern".to_string(),
        ],
        &make_ops(),
        0,
    );
    pattern_seed = getaparam("pattern").unwrap_or_default();
    let pattern_val = pattern_seed
        .get(1)
        .cloned()
        .unwrap_or_else(|| "*".to_string());
    let argv = getaparam(src).unwrap_or_default();
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);

    // Build the filter against (R)pattern + excl PM_LOCAL + pfilt.
    let pat_prog = patcompile(
        &{
            let mut __pat_tok = (&pattern_val).to_string();
            crate::ported::glob::tokenize(&mut __pat_tok);
            __pat_tok
        },
        0,
        None,
    );
    let pfilt_prog = if pfilt.is_empty() {
        None
    } else {
        patcompile(
            &{
                let mut __pat_tok = (&pfilt).to_string();
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            0,
            None,
        )
    };

    // sh:23-41 — the `extra-verbose` branch is NOT ported.
    //
    //   Under it, sh:25 splits the parameter set in two: the plain names go
    //   through `_describe` at sh:36 carrying `name:value` descriptions built
    //   by sh:34's `$( typeset -m ${(@b)described} )`, and only the
    //   hideval/special ones reach the sh:55 `compadd`. A faithful port of
    //   that split was written and measured against zsh
    //   (`comptab_parity.py --case 'unset ' --sequences tab1`, stock `$fpath`,
    //   `extra-verbose on`): zsh offered 148 matches, the port 124, against a
    //   sh:43-branch baseline of 171 for the same shell. The loss is on the
    //   `typeset -m` → `_describe` leg, not in the sh:25/sh:40 partition, so
    //   the branch would have traded a documented gap for a silent
    //   match-dropping one. It is left unported until that leg is understood
    //   rather than shipped half-working.
    //
    //   sh:43 (the `else` arm) is what runs below, unconditionally.
    let all_params = enumerate_params();
    let mut normal: Vec<String> = Vec::new();
    for (name, ty, _flags) in all_params {
        // (R)$~pattern[2] — the assoc subscript matches the VALUE, i.e. the
        //   `paramtypestr` string, not the parameter name.
        let val_matches = match pat_prog.as_ref() {
            Some(p) => pattry(p, &ty),
            None => ty == pattern_val,
        };
        if !val_matches {
            continue;
        }
        // :#$~pfilt — name must NOT match pfilt
        if let Some(prog) = pfilt_prog.as_ref() {
            if pattry(prog, &name) {
                continue;
            }
        }
        // sh:43's `~*local*` is a plain substring test against the type
        //   string. The previous port tested `flags & PM_LOCAL` instead —
        //   which never matches, because `createparam` CLEARS that bit once
        //   it has used it to stamp `pm->level` (`Src/params.c:1155`). Locals
        //   were therefore never filtered at all.
        if ty.contains("local") {
            continue;
        }
        normal.push(name);
    }
    normal.sort();

    let expl = getaparam("expl").unwrap_or_default();
    // sh:11's `described` is only ever filled by the unported sh:23 branch, so
    // the sh:55-56 `${…:|described}` set-differences below are no-ops here.
    let described_final: Vec<String> = Vec::new();

    // sh:46-54  fake-parameters
    let fake_vals = lookupstyle(&format!(":completion:{}:", curcontext), "fake-parameters");
    let mut faked: Vec<String> = Vec::new();
    let mut fakes: Vec<String> = Vec::new();
    for v in fake_vals {
        if v.contains(':') {
            faked.push(v);
        } else {
            fakes.push(v);
        }
    }
    // Faked names whose declared type (after `:`) matches `pattern_val`
    let faked_matching: Vec<String> = faked
        .iter()
        .filter_map(|s| {
            let mut parts = s.splitn(2, ':');
            let name = parts.next()?.to_string();
            let ty = parts.next()?;
            let matches = match pat_prog.as_ref() {
                Some(p) => pattry(p, ty),
                None => ty == pattern_val,
            };
            if matches {
                Some(name)
            } else {
                None
            }
        })
        .collect();

    // sh:55  compadd "$@" "$expl[@]" - "$normal[@]" "${(@)fakes:|described}" \
    // sh:56      "${(@)${(@)${(@M)faked:#${~pattern[2]}}%%:*}:|described}"
    //   `${a:|b}` is the set DIFFERENCE — elements of `a` that are not in
    //   `b`. Under extra-verbose the described names were already offered by
    //   `_describe` at sh:36, so re-adding a fake by the same name would
    //   double-list it.
    let mut combined: Vec<String> = normal;
    combined.extend(fakes.into_iter().filter(|f| !described_final.contains(f)));
    combined.extend(
        faked_matching
            .into_iter()
            .filter(|f| !described_final.contains(f)),
    );
    combined.sort();
    combined.dedup();

    let mut compadd_argv = argv;
    compadd_argv.extend(expl);
    compadd_argv.push("-".to_string());
    compadd_argv.extend(combined);
    let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);

    // sh:58  (( compstate[nmatches] > nm )) — the real counter, not "did
    //   this call pass a non-empty list". `_describe` at sh:36 can be the
    //   only thing that added, and an empty `normal` at sh:55 does not make
    //   the function fail in that case.
    let nm_after = get_compstate_str("nmatches")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if nm_after > nm {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    #[test]
    fn enumerate_params_returns_some_entries() {
        // Param table should never be empty at runtime (zshrs init
        //   creates many special params).
        let _g = crate::test_util::global_state_lock();
        let entries = enumerate_params();
        assert!(!entries.is_empty(), "paramtab unexpectedly empty");
    }

    #[test]
    fn returns_one_or_zero_no_panic() {
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let _r = _parameters(&[]);
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }
}
