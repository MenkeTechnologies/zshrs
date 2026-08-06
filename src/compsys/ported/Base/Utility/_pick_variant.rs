//! Port of `_pick_variant` from
//! `Completion/Base/Utility/_pick_variant`.
//!
//! Full upstream body (49 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 9  zparseopts -D -A opts b: c: r:
//! sh:10  : ${opts[-c]:=$words[1]}
//! sh:12  while [[ $1 = *=* ]]; do
//! sh:13    var+=( "${1%%\=*}" "${1#*=}" )
//! sh:14    shift
//! sh:15  done
//! sh:17  if (( ${#precommands:|builtin_precommands} )); then
//! sh:18    pre=command
//! sh:17  elif (( $+opts[-b] && (...) )); then
//! sh:21    return 0
//! sh:22  elif (( $precommands[(I)builtin] )); then
//! sh:23    pre=builtin
//! sh:24  else
//! sh:27    pre=
//! sh:28  fi
//! sh:30  if [[ $pre != builtin ]] && (( $+_cmd_variant[$opts[-c]] )); then
//! sh:31    (( $+opts[-r] )) && : ${(P)opts[-r]::=${_cmd_variant[$opts[-c]]}}
//! sh:32    [[ $_cmd_variant[$opts[-c]] = "$1" ]] && return 1
//! sh:33    return 0
//! sh:34  fi
//! sh:36  output="$(_call_program variant $pre $opts[-c] "${@[2,-1]}" </dev/null 2>&1)"
//! sh:38  for cmd pat in "$var[@]"; do
//! sh:39    if [[ $output = *$~pat* ]]; then
//! sh:40      (( $+opts[-r] )) && : ${(P)opts[-r]::=$cmd}
//! sh:41      _cmd_variant[$opts[-c]]="$cmd"
//! sh:42      return 0
//! sh:43    fi
//! sh:44  done
//! sh:49  return 1
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:9 — `zparseopts -D -A opts b: c: r:`. `-A` makes opts an
/// assoc; we use the flat key/value layout.
fn run_zparseopts_pick_variant(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("opts_flat", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "opts_flat".to_string(),
            "b:".to_string(),
            "c:".to_string(),
            "r:".to_string(),
        ],
        &make_ops(),
        0,
    );
    let opts_flat = getaparam("opts_flat").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, opts_flat)
}

/// Look up `key` in a flat [k, v, k, v, ...] options array.
fn opt(opts_flat: &[String], key: &str) -> Option<String> {
    let mut i = 0;
    while i + 1 < opts_flat.len() {
        if opts_flat[i] == key {
            return Some(opts_flat[i + 1].clone());
        }
        i += 2;
    }
    None
}

/// `_pick_variant` — detect which variant of a command is installed
/// by running it (cached in `$_cmd_variant`) and matching its output
/// against caller-supplied `name=pattern` specs.
pub fn _pick_variant(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_pick_variant");
    // sh:9
    let (argv, opts_flat) = run_zparseopts_pick_variant(args);

    // sh:10 — opts[-c] defaults to $words[1]
    let cmd_name = match opt(&opts_flat, "-c") {
        Some(v) => v,
        None => getaparam("words")
            .unwrap_or_default()
            .first()
            .cloned()
            .unwrap_or_default(),
    };

    // sh:10-13 — extract `name=pattern` pairs from argv head
    let mut var: Vec<(String, String)> = Vec::new();
    let mut argv = argv;
    while let Some(first) = argv.first() {
        if let Some(eq) = first.find('=') {
            let (n, p) = first.split_at(eq);
            var.push((n.to_string(), p[1..].to_string()));
            argv.remove(0);
        } else {
            break;
        }
    }

    // sh:30  cached?
    let cmd_variant_arr = getaparam("_cmd_variant").unwrap_or_default();
    let cached: Option<String> = cmd_variant_arr
        .chunks(2)
        .find(|kv| kv.first().map(|k| k == &cmd_name).unwrap_or(false))
        .and_then(|kv| kv.get(1).cloned());
    if let Some(cached_v) = cached {
        if let Some(r) = opt(&opts_flat, "-r") {
            let _ = setsparam(&r, &cached_v);
        }
        let dflt = argv.first().cloned().unwrap_or_default();
        if cached_v == dflt {
            return 1;
        }
        return 0;
    }

    // sh:36 — `output="$(_call_program variant … "${@[2,-1]}" </dev/null 2>&1)"`.
    // The shell captures the probe command's output with STDERR MERGED (`2>&1`)
    // and stdin from /dev/null. Native `_call_program` publishes only the
    // command's STDOUT to `$REPLY`, and native `_pick_variant` calls it directly
    // (no `$(… 2>&1)` wrapper), so a probe that prints its usage/version to
    // STDERR — e.g. `nc -h` (BSD netcat writes 86 lines to stderr, nothing to
    // stdout) — yielded an EMPTY `$REPLY`, no pattern matched, and the DEFAULT
    // (last) variant was picked: `nc <TAB>` completed `nedit` options
    // (`-iconic`/`-line`/…) instead of netcat's `-b`/`-i`/`-l`. `_call_program`
    // runs `sh -c <joined args>`, so append the redirects as command words to
    // reproduce the `</dev/null 2>&1` the shell puts on the `$()`.
    let mut call_args: Vec<String> = vec!["variant".to_string(), cmd_name.clone()];
    if argv.len() > 1 {
        call_args.extend(argv[1..].iter().cloned());
    }
    call_args.push("</dev/null".to_string());
    call_args.push("2>&1".to_string());
    let _ = _call_program(&call_args);
    let output = getsparam("REPLY").unwrap_or_default();

    // sh:38-43 — for each (name, pattern), test output match
    for (name, pat) in &var {
        // sh:39 — `if [[ $output = *$~pat* ]]`: the pattern is matched as a
        // SUBSTRING (`*…*` on both sides), not anchored to the whole output.
        // Without the wrapping `*`, `pattry` did a full-string match, so e.g.
        // `grep (BSD grep, GNU compatible) 2.6.0-FreeBSD` never matched the
        // `gpl2='(2.5.1|GNU compatible)'` spec → `_pick_variant` fell through
        // to the `unix` default → wrong (reduced) option set for `grep`/etc.
        let matched = match patcompile(
            &{
                let mut __pat_tok = format!("*{}*", pat);
                crate::ported::glob::tokenize(&mut __pat_tok);
                __pat_tok
            },
            0,
            None,
        ) {
            Some(prog) => pattry(&prog, &output),
            None => output.contains(pat),
        };
        if matched {
            if let Some(r) = opt(&opts_flat, "-r") {
                let _ = setsparam(&r, name);
            }
            // Append to _cmd_variant
            let mut arr = getaparam("_cmd_variant").unwrap_or_default();
            arr.push(cmd_name.clone());
            arr.push(name.clone());
            setaparam("_cmd_variant", arr);
            return 0;
        }
    }

    // sh:46-47
    let dflt = argv.first().cloned().unwrap_or_default();
    if let Some(r) = opt(&opts_flat, "-r") {
        let _ = setsparam(&r, &dflt);
    }
    let mut arr = getaparam("_cmd_variant").unwrap_or_default();
    arr.push(cmd_name);
    arr.push(dflt);
    setaparam("_cmd_variant", arr);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_cmd_variant", Vec::new());
        assert_eq!(_pick_variant(&[]), 1);
    }
}
