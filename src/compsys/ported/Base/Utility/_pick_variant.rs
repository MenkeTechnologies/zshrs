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
//! sh:19  elif (( $+opts[-b] && ( $precommands[(I)builtin] || $+builtins[$opts[-c]] ) )); then
//! sh:20    (( $+opts[-r] )) && : ${(P)opts[-r]::=$opts[-b]}
//! sh:21    return 0
//! sh:22  elif (( $precommands[(I)builtin] )); then
//! sh:23    pre=builtin
//! sh:24  else
//! sh:25    # Neither builtin nor command-forcing precommand specified,
//! sh:26    # so no prefix is needed.
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
//! sh:46  (( $+opts[-r] )) && : ${(P)opts[-r]::=$1}
//! sh:47  [[ $pre != builtin ]] && _cmd_variant[$opts[-c]]="$1"
//! sh:49  return 1
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zsh_h::{options, MAX_OPS, PM_UNSET};

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

/// sh:19 — `$+builtins[$opts[-c]]`.
///
/// `builtins` is the `zsh/parameter` special associative array; `$+`
/// on one of its elements resolves through the module's `getnode`
/// hook, which for `builtins` is `getpmbuiltin`
/// (`src/ported/builtin.rs:8567` dispatches exactly this way). A name
/// that is not in `builtintab` still yields a Param, but flagged
/// `PM_UNSET` (`Src/Modules/parameter.c:790-792`), which is what makes
/// `$+builtins[nosuchthing]` evaluate to 0. Go through the same
/// accessor rather than probing `BUILTINS` directly so the DISABLED /
/// auto-load-stub semantics `getbuiltin` already models are honoured.
fn plus_builtins(name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    match crate::ported::modules::parameter::getpmbuiltin(std::ptr::null_mut(), name) {
        Some(pm) => (pm.node.flags as u32 & PM_UNSET) == 0,
        None => false,
    }
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

    // sh:17-28 — decide the command PREFIX (`$pre`) for the probe, and
    // short-circuit entirely when `-b` says "this is a shell builtin".
    //
    // This whole block used to be MISSING from the port: `_pick_variant`
    // went straight from arg parsing to the `_cmd_variant` cache and the
    // `_call_program` probe, so the `-b` fast path (sh:19-21) never fired.
    // `Completion/Unix/Command/_echo:6` calls
    // `_pick_variant -r variant -b zsh gnu='Free Soft' $OSTYPE --version`
    // — `echo` IS a builtin, so upstream returns `zsh` at sh:21 without
    // running anything. Skipping that meant the probe ran, matched no
    // `name=pattern`, and fell through to the DEFAULT `$1` = `$OSTYPE`.
    // On macOS that is `darwin25.4.0`, selecting `_echo`'s `darwin*` arm
    // (sh:24-26) which strips `-e`/`-E` as well as `--*`, leaving `-n` as
    // the ONLY match — so `echo -<TAB>` inserted `-n` instead of listing
    // `-E`/`-e`/`-n`. Every `_pick_variant -b` caller was affected the
    // same way.
    let precommands = getaparam("precommands").unwrap_or_default();
    let builtin_precommands = getaparam("builtin_precommands").unwrap_or_default();
    // sh:17 — `(( ${#precommands:|builtin_precommands} ))`. `${a:|b}`
    // is array-difference, so this is "some precommand is NOT one of
    // the builtin-preserving ones" (same computation as
    // `_command_names.rs:126`, sh:28 there).
    let precmd_diff_nonempty = precommands.iter().any(|p| !builtin_precommands.contains(p));
    // sh:19/sh:22 — `$precommands[(I)builtin]`, the index of the LAST
    // `builtin` element (0 when absent), used purely as a boolean.
    let has_builtin_precommand = precommands.iter().any(|p| p == "builtin");
    let pre: &str;
    if precmd_diff_nonempty {
        pre = "command"; // sh:18
    } else if opt(&opts_flat, "-b").is_some()
        && (has_builtin_precommand || plus_builtins(&cmd_name))
    {
        // sh:20 — `(( $+opts[-r] )) && : ${(P)opts[-r]::=$opts[-b]}`
        if let Some(r) = opt(&opts_flat, "-r") {
            let _ = setsparam(&r, &opt(&opts_flat, "-b").unwrap_or_default());
        }
        return 0; // sh:21
    } else if has_builtin_precommand {
        pre = "builtin"; // sh:23
    } else {
        // sh:25-27 — Neither builtin nor command-forcing precommand
        // specified, so no prefix is needed.
        pre = "";
    }

    // sh:30  cached?  — guarded by `[[ $pre != builtin ]]`: a `builtin
    // foo` command line must not consume (or later populate, sh:47) the
    // cache entry keyed on the bare command name, since that entry
    // describes the EXTERNAL `foo`.
    let cmd_variant_arr = getaparam("_cmd_variant").unwrap_or_default();
    let cached: Option<String> = if pre == "builtin" {
        None
    } else {
        cmd_variant_arr
            .chunks(2)
            .find(|kv| kv.first().map(|k| k == &cmd_name).unwrap_or(false))
            .and_then(|kv| kv.get(1).cloned())
    };
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
    // sh:36 — `$pre` is UNQUOTED in the shell, so an empty `$pre`
    // contributes no word at all; `command`/`builtin` become a real
    // prefix word ahead of `$opts[-c]`.
    let mut call_args: Vec<String> = vec!["variant".to_string()];
    if !pre.is_empty() {
        call_args.push(pre.to_string());
    }
    call_args.push(cmd_name.clone());
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
    // sh:47 — `[[ $pre != builtin ]] && _cmd_variant[$opts[-c]]="$1"`.
    if pre != "builtin" {
        let mut arr = getaparam("_cmd_variant").unwrap_or_default();
        arr.push(cmd_name);
        arr.push(dflt);
        setaparam("_cmd_variant", arr);
    }
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
