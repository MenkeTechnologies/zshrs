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
//! sh:24    described=( … keys of $parameters matching pattern … )
//! sh:26    compadd … -D described -a - described
//! sh:34    _describe -t parameters parameter verbose …
//! sh:38    normal=( … )
//! sh:41  else
//! sh:42    normal=( ${(k)parameters[(R)${~pattern[2]}~*local*]:#$~pfilt} )
//! sh:43  fi
//! sh:45  if zstyle -a ":completion:${curcontext}:" fake-parameters tmp; then …
//! sh:55  compadd "$@" "$expl[@]" - "$normal[@]" "${(@)fakes:|described}" \
//! sh:56      "${(@)${(@)${(@M)faked:#${~pattern[2]}}%%:*}:|described}"
//! sh:58  (( compstate[nmatches] > nm ))
//! ```
//!
//! `$parameters` is the shell-side assoc-array mapping param name
//! to its zsh-type string ("integer", "array", "scalar-export", …).
//! We enumerate from `paramtab` directly; the `(R)pattern` glob
//! filters values, `$~pfilt` excludes names matching that pattern,
//! `*local*` exclusion stops local-scoped names from leaking.

use crate::compsys::ported::_description::_description;
use crate::ported::modules::zutil::{bin_zparseopts, lookupstyle, testforstyle};
use crate::ported::params::{getaparam, getsparam, paramtab, setaparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS, PM_LOCAL, PM_TYPE};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Iterate paramtab, returning (name, zsh-type-string) pairs.
/// Approximates `$parameters` assoc enumeration.
fn enumerate_params() -> Vec<(String, String, i32)> {
    let mut out: Vec<(String, String, i32)> = Vec::new();
    if let Ok(tab) = paramtab().read() {
        for (name, pm) in tab.iter() {
            let flags = pm.node.flags as i32;
            let pmtype = PM_TYPE(flags as u32);
            // Map PM_TYPE bits to zsh's `$parameters` value-string
            //   convention. Approximation suitable for `(R)pattern`
            //   matching.
            let ty = if pmtype & crate::ported::zsh_h::PM_INTEGER != 0 {
                "integer"
            } else if pmtype & crate::ported::zsh_h::PM_ARRAY != 0 {
                "array"
            } else if pmtype & crate::ported::zsh_h::PM_HASHED != 0 {
                "association"
            } else if pmtype & (crate::ported::zsh_h::PM_EFLOAT | crate::ported::zsh_h::PM_FFLOAT)
                != 0
            {
                "float"
            } else {
                "scalar"
            };
            out.push((name.clone(), ty.to_string(), flags));
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
    // sh:11
    let mut pattern_seed: Vec<String> = vec!["-g".to_string(), "*".to_string()];

    // sh:14-16  prefix-needed handling
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let prefix_needed = testforstyle(
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
    setaparam(src, args.to_vec());
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
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
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

    let all_params = enumerate_params();
    let mut normal: Vec<String> = Vec::new();
    for (name, ty, flags) in all_params {
        // sh:42 exclusion: `*local*` removed
        if (flags & PM_LOCAL as i32) != 0 {
            continue;
        }
        // (R)pattern — value must match
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
        normal.push(name);
    }
    normal.sort();

    // sh:45-54  fake-parameters
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

    // sh:55-56 — combine `normal` + fakes + faked-name parts, excluding
    //   any already in `described` (we don't model extra-verbose here)
    let mut combined: Vec<String> = normal;
    combined.extend(fakes);
    combined.extend(faked_matching);
    combined.sort();
    combined.dedup();

    // sh:55  compadd "$@" "$expl[@]" - <combined>
    let expl = getaparam("expl").unwrap_or_default();
    let mut compadd_argv = argv;
    compadd_argv.extend(expl);
    compadd_argv.push("-".to_string());
    let nm_before = combined.is_empty();
    compadd_argv.extend(combined);
    let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);

    // sh:58 — return: did nmatches grow? approximate with "any added"
    let _ = get_compstate_str("nmatches");
    if nm_before {
        1
    } else {
        0
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
