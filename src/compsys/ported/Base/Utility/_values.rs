//! Port of `_values` from `Completion/Base/Utility/_values`
//! (spec: `/usr/share/zsh/5.9/functions/_values`).
//!
//! Faithful 1:1 translation. `_values` drives the C builtin
//! `compvalues` (ported at `crate::ported::zle::computil::bin_compvalues`)
//! — it is NOT routed through `_alternative`. The engine is:
//!
//! ```text
//! sh:  1  #autoload
//! sh:  6  zparseopts -D -a garbage s+:=keep S+:=keep w+=keep C=usecc O:=subopts …
//! sh:  9  (( $#subopts )) && subopts=( "${(@P)subopts[2]}" )
//! sh: 11  if compvalues -i "$keep[@]" "$@"; then       # parse value defs
//! sh: 16    compvalues -S argsep                        # value/arg separator
//! sh: 17    compvalues -s sep && test="[^sep]#"         # list separator
//! sh: 19    if ! compvalues -D descr action; then       # completing NAMES
//! sh: 21      _tags values
//! sh: 25      compvalues -V noargs args opts            # active value sets
//! sh: 27      if PREFIX = *argsep test; then            # typing `name=<arg>`
//! sh: 31        compvalues -L name descr action  (or disambiguate via compadd -D)
//! sh: 52      else _describe … noargs args opts; return # list value names
//! sh: 69    else compvalues -C subc                     # completing an ARG
//! sh: 74    _tags arguments; _description arguments expl "$descr"
//! sh: 88    if action = ->state: compvalues -v val_args; set $state; return 1
//! sh:104    else action dispatch (empty / ((…)) / (…) / {…} / ` …` / cmd)
//! sh:155    [[ nm -ne $compstate[nmatches] ]]
//! sh:156  else curcontext="$oldcontext"; return 1
//! ```
//!
//! Approximations (see inline comments): `compadd -D array` array
//! pruning (sh:42) is reproduced in Rust because the `computil`
//! port does not yet write the `dpar` array back; the multi-group
//! `_describe … -- … -- …` call (sh:60) is passed verbatim to the
//! `_describe` port, which currently reads the group array names and
//! ignores the per-group `-S`/`-qS`/`-r` flags.

use crate::compsys::ported::_all_labels::all_labels_byname;
use crate::compsys::ported::_describe::{_describe, describe_byname};
use crate::compsys::ported::_description::description_byname;
use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_next_label::next_label_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::ported::exec::{dispatch_function_call, execute_script};
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zle::computil::bin_compvalues;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// Call `compvalues <sub> <params…>`. Returns the builtin exit status
/// (0 = success). The named params are read back by the caller via
/// `getsparam`/`getaparam`/`gethparam`.
fn compvalues(argv: &[&str]) -> i32 {
    let v: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compvalues("compvalues", &v, &make_ops(), 0)
}

/// `${ctx%:*}:new` — replace the last `:`-delimited field of a
/// curcontext string (sh:23,50,71,93).
fn replace_last_field(ctx: &str, new: &str) -> String {
    let base = match ctx.rfind(':') {
        Some(i) => &ctx[..i],
        None => ctx,
    };
    format!("{}:{}", base, new)
}

/// `${s%%sep*}` — text before the first occurrence of `sep` (sh:30).
fn before_first<'a>(s: &'a str, sep: &str) -> &'a str {
    match s.find(sep) {
        Some(i) => &s[..i],
        None => s,
    }
}

/// `${s#*sep}` — text after the first occurrence of `sep` (sh:33,37).
fn after_first(s: &str, sep: &str) -> String {
    match s.find(sep) {
        Some(i) => s[i + sep.len()..].to_string(),
        None => s.to_string(),
    }
}

/// Glob test for `[[ "$PREFIX" = *${argsep}${~test} ]]` (sh:27).
/// `test` is `*` (unrestricted) or `[^sep]#` (restricted to a run of
/// non-separator chars). The pattern matches iff there is an `argsep`
/// occurrence whose tail satisfies `test`; for the restricted form the
/// last `argsep`'s tail must contain no separator char (an earlier
/// occurrence can only have a longer, superset tail).
fn prefix_is_arg(prefix: &str, argsep: &str, restricted: bool, sep_char: &str) -> bool {
    if argsep.is_empty() {
        return false;
    }
    if !restricted {
        // test == '*'  →  PREFIX matches `*argsep*`
        return prefix.contains(argsep);
    }
    match prefix.rfind(argsep) {
        Some(i) => {
            let tail = &prefix[i + argsep.len()..];
            sep_char.is_empty() || !tail.contains(sep_char)
        }
        None => false,
    }
}

/// `_values` — generic value completion. Parses `name[desc]:msg:action`
/// specs into the `compvalues` engine and completes either the value
/// names (for a list) or the argument of one named value.
pub fn _values(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_values");
    let r = values_impl(args);
    // sh: the locals `noargs args opts descr action expl sep argsep subc
    // test ws val_args` are function-scoped in zsh and vanish on return.
    // The port has no local-param scope, so unset the by-name scratch
    // params here (state / state_descr / context / curcontext / PREFIX /
    // SUFFIX / IPREFIX / compstate persist for the caller).
    for p in [
        "noargs", "args", "opts", "descr", "action", "expl", "sep", "argsep", "subc", "val_args",
        "ws",
    ] {
        unsetparam(p);
    }
    r
}

fn values_impl(args: &[String]) -> i32 {
    // sh:6-9 — zparseopts. `keep` collects the `-s SEP` / `-S SEP` / `-w`
    // flags that feed `compvalues -i`; `-C` → usecc; `-O ARR` → subopts
    // expands the named array (`${(@P)subopts[2]}`); everything else
    // (`-M -J -V -1 -2 -o -n -F -X`) goes to `garbage` (discarded).
    let mut keep: Vec<String> = Vec::new();
    let mut usecc = false;
    let mut subopts: Vec<String> = Vec::new();
    let mut idx = 0usize;

    while idx < args.len() {
        let a = &args[idx];
        let b = a.as_bytes();
        if b.len() != 2 || b[0] != b'-' {
            break;
        }
        match b[1] {
            // s+:=keep / S+:=keep — flag + arg, kept for `compvalues -i`.
            b's' | b'S' if idx + 1 < args.len() => {
                keep.push(a.clone());
                keep.push(args[idx + 1].clone());
                idx += 2;
            }
            // w+=keep — flag, no arg.
            b'w' => {
                keep.push(a.clone());
                idx += 1;
            }
            // C=usecc — no arg.
            b'C' => {
                usecc = true;
                idx += 1;
            }
            // O:=subopts — arg is an array name; expand it into subopts.
            b'O' if idx + 1 < args.len() => {
                subopts = getaparam(&args[idx + 1]).unwrap_or_default();
                idx += 2;
            }
            // garbage, arg-taking: M: J: V: o+: F: X:
            b'M' | b'J' | b'V' | b'o' | b'F' | b'X' if idx + 1 < args.len() => {
                idx += 2;
            }
            // garbage, no arg: 1 2 n
            b'1' | b'2' | b'n' => {
                idx += 1;
            }
            _ => break,
        }
    }

    // sh:11 — `compvalues -i "$keep[@]" "$@"`. Remaining positionals are
    // the description + value specs.
    let mut cvi: Vec<String> = vec!["-i".to_string()];
    cvi.extend(keep.iter().cloned());
    cvi.extend(args[idx..].iter().cloned());
    if bin_compvalues("compvalues", &cvi, &make_ops(), 0) != 0 {
        // sh:156-159 — `oldcontext` is declared at sh:14 inside the
        // taken branch only; on this path it is unset, so zsh expands
        // `curcontext="$oldcontext"` to the empty string.
        let _ = setsparam("curcontext", "");
        return 1;
    }

    // sh:14 — capture curcontext for restoration.
    let oldcontext = getsparam("curcontext").unwrap_or_default();

    // sh:16 — value/argument separator (default '=').
    compvalues(&["-S", "argsep"]);
    let argsep = getsparam("argsep").unwrap_or_default();

    // sh:17 — list separator; `test="[^sep]#"` when a non-empty sep exists.
    let has_sep = compvalues(&["-s", "sep"]) == 0;
    let sep_char = if has_sep {
        getsparam("sep").unwrap_or_default()
    } else {
        String::new()
    };
    let test_restricted = has_sep && !sep_char.is_empty();

    // sh:19 — `if ! compvalues -D descr action` → completing value NAMES.
    if compvalues(&["-D", "descr", "action"]) != 0 {
        // sh:21
        if tags_byname(&["values".to_string()]) != 0 {
            return 1;
        }
        // sh:23
        let _ = setsparam("curcontext", &replace_last_field(&oldcontext, "values"));
        // sh:25 — active value sets by argument type.
        compvalues(&["-V", "noargs", "args", "opts"]);

        let prefix = getsparam("PREFIX").unwrap_or_default();
        // sh:27 — `[[ -n "$argsep" && "$PREFIX" = *${argsep}${~test} ]]`.
        if !argsep.is_empty() && prefix_is_arg(&prefix, &argsep, test_restricted, &sep_char) {
            // Completing the ARGUMENT of `name=<arg>`.
            // sh:30 — name = text before the first argsep.
            let name = before_first(&prefix, &argsep).to_string();
            // sh:31 — try the value directly.
            if compvalues(&["-L", name.as_str(), "descr", "action"]) == 0 {
                // sh:32-33 — shift the `name=` prefix out of PREFIX.
                let ipref = getsparam("IPREFIX").unwrap_or_default();
                let _ = setsparam("IPREFIX", &format!("{}{}{}", ipref, name, argsep));
                let _ = setsparam("PREFIX", &after_first(&prefix, &argsep));
            } else {
                // sh:34-51 — ambiguous partial name; disambiguate.
                let prefix_after = after_first(&prefix, &argsep); // sh:37
                let suffix = getsparam("SUFFIX").unwrap_or_default(); // sh:38
                let _ = setsparam("PREFIX", &name); // sh:39
                let _ = setsparam("SUFFIX", ""); // sh:40
                                                 // sh:41 — args=( args opts ).
                let mut combined = getaparam("args").unwrap_or_default();
                combined.extend(getaparam("opts").unwrap_or_default());
                let _ = setaparam("args", combined.clone());
                // sh:42 — compadd -M … -D args - "${(@)args[@]%%:*}".
                // Adds nothing (with -D, compadd only prunes the array),
                // it filters `args` down to the entries whose name matches
                // the word on the line.
                let names: Vec<String> = combined
                    .iter()
                    .map(|e| before_first(e, ":").to_string())
                    .collect();
                let mut cadd = vec![
                    "-M".to_string(),
                    "r:|[_-]=* r:|=*".to_string(),
                    "-D".to_string(),
                    "args".to_string(),
                    "-".to_string(),
                ];
                cadd.extend(names);
                bin_compadd("compadd", &cadd, &make_ops(), 0);
                // APPROXIMATION: the `computil`/`compcore` port collects
                // `-D` targets (`dpar`) but does not yet write the pruned
                // array back. Reproduce `compadd -D`'s documented pruning
                // (drop entries whose name does not match the typed word)
                // so the `$#args -ne 1` disambiguation below works.
                let pruned: Vec<String> = combined
                    .into_iter()
                    .filter(|e| before_first(e, ":").starts_with(&name))
                    .collect();
                let _ = setaparam("args", pruned.clone());
                // sh:44 — need exactly one surviving value to proceed.
                if pruned.len() != 1 {
                    return 1;
                }
                // sh:46-48
                let _ = setsparam("PREFIX", &prefix_after);
                let _ = setsparam("SUFFIX", &suffix);
                let matched = before_first(&pruned[0], ":").to_string();
                let ipref = getsparam("IPREFIX").unwrap_or_default();
                let _ = setsparam("IPREFIX", &format!("{}{}{}", ipref, matched, argsep));
                // sh:49 — fetch descr/action/subc for the resolved value.
                compvalues(&["-L", matched.as_str(), "descr", "action", "subc"]);
                let subc = getsparam("subc").unwrap_or_default();
                // sh:50
                let _ = setsparam("curcontext", &replace_last_field(&oldcontext, &subc));
            }
            // Falls through to the arguments dispatch (sh:74).
        } else {
            // sh:52-67 — list the value NAMES via `_describe` and return.
            compvalues(&["-d", "descr"]); // sh:53
            let descr_v = getsparam("descr").unwrap_or_default();
            // sh:54-58 — sep=( -qS <char> ) or ().
            let sep_group: Vec<String> = if compvalues(&["-s", "sep"]) == 0 {
                vec!["-qS".to_string(), getsparam("sep").unwrap_or_default()]
            } else {
                Vec::new()
            };
            let sep2 = sep_group.get(1).cloned().unwrap_or_default();
            // sh:60-63 — three value groups (noargs / args / opts) with
            // per-group separators and the value-name matcher.
            let mut dargv: Vec<String> = vec![descr_v];
            dargv.push("noargs".to_string());
            dargv.extend(sep_group.iter().cloned());
            dargv.extend([
                "-M".to_string(),
                "r:|[_-]=* r:|=*".to_string(),
                "--".to_string(),
            ]);
            dargv.extend([
                "args".to_string(),
                "-S".to_string(),
                argsep.clone(),
                "-M".to_string(),
                "r:|[_-]=* r:|=*".to_string(),
                "--".to_string(),
            ]);
            dargv.extend([
                "opts".to_string(),
                "-qS".to_string(),
                argsep.clone(),
                "-r".to_string(),
                format!("{}{} \\t\\n\\-", argsep, sep2),
                "-M".to_string(),
                "r:|[_-]=* r:|=*".to_string(),
            ]);
            // NOTE: the `_describe` port reads the group array names but
            // ignores the per-group `-S`/`-qS`/`-r`/`--` flags; the value
            // names are still emitted (behavioral approximation).
            // sh:60-63 — reached BY NAME, not as a direct Rust call. Upstream
            // writes `_describe "$descr" …` as a bare command word, so (a) a
            // user's own `_describe` earlier in `$fpath` autoloads instead of
            // the port, and (b) the call gets its own `doshfunc` frame. A plain
            // `_describe(&dargv)` skipped both: the fpath copy was inert, and
            // `_describe`'s `declare_locals` list (`_opt … OPTIND OPTARG`,
            // _describe.rs:160-192) landed in _values' OWN param scope, so
            // doshfunc's `zoptind = funcsave->zoptind` restore (c:Src/exec.c:6060,
            // exec.rs:6283) wrote into that shadow and `endparamscope` then put
            // the caller's OPTIND back to the 1 doshfunc had stamped on entry
            // (exec.rs:6016) — `local OPTIND=9; _values …` returned OPTIND=1
            // where zsh returns 9.
            let _ = crate::compsys::ported::shared::call_compfn("_describe", &dargv, || {
                describe_byname(&dargv)
            });
            let _ = setsparam("curcontext", &oldcontext); // sh:65
                                                          // sh:67 is a BARE `return`, so it yields `$?` of the LAST command
                                                          // executed — the plain assignment at sh:65, not the `_describe` at
                                                          // sh:60-63. A simple command consisting only of assignments always
                                                          // exits 0, so zsh's value-name arm returns 0 even when `_describe`
                                                          // matched nothing.
                                                          //
                                                          // Propagating `_describe`'s status instead made `_values` fail
                                                          // whenever the value-name list came back empty. That escaped through
                                                          // `_tar`'s CURRENT==2 branch (`_values -s '' 'tar function' …`,
                                                          // _tar:169) into `_complete`, so the user's `_megacomplete`
                                                          // completer took its `ret == 1` arm and built a `last args` group out
                                                          // of shell history — zshrs listed history matches on `tar -<TAB>`
                                                          // where zsh shows "No Matches for `tar function'".
            return 0; // sh:67
        }
    } else {
        // sh:69-72 — completing the ARGUMENT of an already-recognized value.
        compvalues(&["-C", "subc"]); // sh:70
        let subc = getsparam("subc").unwrap_or_default();
        let _ = setsparam("curcontext", &replace_last_field(&oldcontext, &subc));
        // sh:71
    }

    // sh:74 — arguments dispatch (reached with descr/action set for a value).
    if tags_byname(&["arguments".to_string()]) != 0 {
        let _ = setsparam("curcontext", &oldcontext); // sh:75
        return 1;
    }

    let descr = getsparam("descr").unwrap_or_default();
    // sh:79
    let _ = description_byname(&["arguments".to_string(), "expl".to_string(), descr.clone()]);

    // sh:84-86 — append the list separator as an autoremovable suffix
    // unless exactly one value remains. `snames`/`names`/`onames` are not
    // set by this function (sum 0 ≠ 1), so the suffix is added when a
    // separator exists.
    let mut sep_group: Vec<String> = Vec::new();
    let cnt = getaparam("snames").map(|v| v.len()).unwrap_or(0)
        + getaparam("names").map(|v| v.len()).unwrap_or(0)
        + getaparam("onames").map(|v| v.len()).unwrap_or(0);
    if cnt != 1 && compvalues(&["-s", "sep"]) == 0 {
        let s = getsparam("sep").unwrap_or_default();
        let flag = format!("-qS{}", s);
        let mut expl = getaparam("expl").unwrap_or_default();
        expl.insert(0, flag.clone());
        let _ = setaparam("expl", expl);
        sep_group = vec![flag];
    }

    let action = getsparam("action").unwrap_or_default();

    // sh:88 — `->state` form.
    if action.starts_with("->") {
        compvalues(&["-v", "val_args"]); // sh:89
                                         // sh:90 — state = action minus `->`, whitespace-trimmed.
        let state = action[2..].trim().to_string();
        let _ = setsparam("state", &state);
        let _ = setsparam("state_descr", &descr); // sh:91
        let subc = getsparam("subc").unwrap_or_default();
        if usecc {
            // sh:93
            let _ = setsparam("curcontext", &replace_last_field(&oldcontext, &subc));
        } else {
            // sh:95
            let _ = setsparam("context", &subc);
        }
        set_compstate_str("restore", ""); // sh:97
        return 1; // sh:98
    }

    // sh:100-102 — `typeset -A val_args; compvalues -v val_args`.
    compvalues(&["-v", "val_args"]);

    if action.trim().is_empty() {
        // sh:104-109 — empty action: just show the description.
        let _ = message_byname(&["-e".to_string(), "arguments".to_string(), descr.clone()]);
        return 1;
    } else if action.starts_with("((") && action.ends_with("))") {
        // sh:111-118 — ((val:descr …)) literal set with descriptions.
        let body = &action[2..action.len() - 2];
        let ws: Vec<String> = body.split_whitespace().map(|s| s.to_string()).collect();
        let _ = setaparam("ws", ws);
        let mut d = vec![
            descr.clone(),
            "ws".to_string(),
            "-M".to_string(),
            "r:|[_-]=* r:|=*".to_string(),
        ];
        d.extend(subopts.iter().cloned());
        d.extend(sep_group.iter().cloned());
        // sh:118 — same by-name contract as sh:60-63 above.
        let _ = crate::compsys::ported::shared::call_compfn("_describe", &d, || _describe(&d));
    } else if action.starts_with('(') && action.ends_with(')') {
        // sh:120-126 — (val …) added directly.
        let body = &action[1..action.len() - 1];
        let ws: Vec<String> = body.split_whitespace().map(|s| s.to_string()).collect();
        let _ = setaparam("ws", ws);
        let mut a = vec![
            "arguments".to_string(),
            "expl".to_string(),
            descr.clone(),
            "compadd".to_string(),
        ];
        a.extend(subopts.iter().cloned());
        a.extend(sep_group.iter().cloned());
        a.extend(["-a".to_string(), "-".to_string(), "ws".to_string()]);
        let _ = all_labels_byname(&a);
    } else if action.starts_with('{') && action.ends_with('}') {
        // sh:127-133 — {body} evaluated per label.
        let body = &action[1..action.len() - 1];
        loop {
            if next_label_byname(&["arguments".to_string(), "expl".to_string(), descr.clone()]) != 0
            {
                break;
            }
            let _ = execute_script(body);
        }
    } else if action.starts_with(' ') {
        // sh:138 — `eval "action=( $action )"; "$action[@]"` (quote-respecting).
        let parts: Vec<String> = crate::compsys::ported::eval_action_words(&action);
        loop {
            if next_label_byname(&["arguments".to_string(), "expl".to_string(), descr.clone()]) != 0
            {
                break;
            }
            if let Some((cmd, rest)) = parts.split_first() {
                if cmd == "compadd" {
                    bin_compadd("compadd", rest, &make_ops(), 0);
                } else {
                    let _ = dispatch_function_call(cmd, rest);
                }
            }
        }
    } else {
        // sh:146 — `eval "action=( $action )"` then cmd+args (quote-respecting).
        let parts: Vec<String> = crate::compsys::ported::eval_action_words(&action);
        if let Some((cmd, rest)) = parts.split_first() {
            loop {
                if next_label_byname(&["arguments".to_string(), "expl".to_string(), descr.clone()])
                    != 0
                {
                    break;
                }
                let expl_now = getaparam("expl").unwrap_or_default();
                let mut call = subopts.clone();
                call.extend(expl_now);
                call.extend(rest.iter().cloned());
                if cmd == "compadd" {
                    bin_compadd("compadd", &call, &make_ops(), 0);
                } else {
                    let _ = dispatch_function_call(cmd, &call);
                }
            }
        }
    }

    // sh:153
    let _ = setsparam("curcontext", &oldcontext);
    // sh:155 — success iff matches were added since the caller's `nm`.
    let nm = getiparam("nm");
    let nmatches = get_compstate_str("nmatches")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    if nm != nmatches {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // No specs → `compvalues -i` fails (also gated on incompfunc);
        // outer else clears curcontext and returns 1.
        assert_eq!(_values(&[]), 1);
    }

    #[test]
    fn compvalues_i_failure_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // Outside a completion function `compvalues -i` reports "can only
        // be called from completion function" and returns 1, so `_values`
        // takes the sh:156 else branch and returns 1 without ever
        // reaching `_alternative` (the old reimplementation's crash path).
        let r = _values(&[
            "-s".to_string(),
            ",".to_string(),
            "option name".to_string(),
            "alpha[first]:arg:(a b)".to_string(),
            "beta".to_string(),
        ]);
        assert_eq!(r, 1);
    }

    #[test]
    fn flag_parse_consumes_options_before_specs() {
        let _g = crate::test_util::global_state_lock();
        // `-C`, `-s SEP`, `-S SEP`, `-w` and a garbage `-V x` must all be
        // consumed; the run still bottoms out at the `compvalues -i`
        // incompfunc gate and returns 1. This pins that option parsing
        // never treats a spec as a flag (or vice versa).
        let r = _values(&[
            "-C".to_string(),
            "-s".to_string(),
            ",".to_string(),
            "-S".to_string(),
            "=".to_string(),
            "-w".to_string(),
            "-V".to_string(),
            "grp".to_string(),
            "descr".to_string(),
            "val:msg:action".to_string(),
        ]);
        assert_eq!(r, 1);
    }

    #[test]
    fn helper_replace_last_field() {
        assert_eq!(replace_last_field(":a:b:c", "x"), ":a:b:x");
        assert_eq!(replace_last_field("noc", "x"), "noc:x");
    }

    #[test]
    fn helper_prefix_is_arg() {
        // Unrestricted: any argsep present.
        assert!(prefix_is_arg("name=val", "=", false, ""));
        assert!(!prefix_is_arg("noeq", "=", false, ""));
        // Restricted: tail after last argsep must have no list separator.
        assert!(prefix_is_arg("name=val", "=", true, ","));
        assert!(!prefix_is_arg("name=a,b", "=", true, ","));
        // Trailing argsep → empty tail matches.
        assert!(prefix_is_arg("name=", "=", true, ","));
    }

    #[test]
    fn helper_before_after_first() {
        assert_eq!(before_first("a=b=c", "="), "a");
        assert_eq!(after_first("a=b=c", "="), "b=c");
        assert_eq!(before_first("noeq", "="), "noeq");
        assert_eq!(after_first("noeq", "="), "noeq");
    }
}
