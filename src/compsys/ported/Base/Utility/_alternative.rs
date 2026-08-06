//! Port of `_alternative` from
//! `Completion/Base/Utility/_alternative`.
//!
//! Full upstream body (83 lines verbatim, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local tags def expl descr action mesgs nm subopts
//! sh: 7  while getopts 'O:C:' opt; do
//! sh: 8    case "$opt" in
//! sh: 9    O) subopts=( "${(@P)OPTARG}" ) ;;
//! sh:10    C) curcontext="${curcontext%:*}:$OPTARG" ;;
//! sh:14  shift OPTIND-1
//! sh:16  [[ "$1" = -(|-) ]] && shift
//! sh:18  mesgs=()
//! sh:20  _tags "${(@)argv%%:*}"
//! sh:22  while _tags; do
//! sh:23    for def; do
//! sh:24      if _requested "${def%%:*}"; then
//! sh:25        descr="${${def#*:}%%:*}"
//! sh:26        action="${def#*:*:}"
//! sh:28        _description "${def%%:*}" expl "$descr"
//! sh:30        if [[ "$action" = \ # ]]; then  # empty action → mesgs
//! sh:35        elif [[ "$action" = \(\(*\)\) ]]; then  # describe-style
//! sh:43        elif [[ "$action" = \(*\) ]]; then  # compadd literal list
//! sh:51        elif [[ "$action" = \{*\} ]]; then  # eval body
//! sh:57        elif [[ "$action" = \ * ]]; then  # bare-call ` cmd args`
//! sh:64        else  # action with description-args
//! sh:65          while _next_label …; do "$action[1]" …; done
//! sh:73    [[ nm -ne compstate[nmatches] ]] && return 0
//! sh:74  done
//! sh:77  for descr in "$mesgs[@]"; do
//! sh:78    _message -e "${descr%%:*}" "${descr#*:}"
//! sh:79  done
//! sh:81  return 1
//! ```
//!
//! Dispatches a list of `tag:description:action` specs, building
//! per-tag completion via `_description` + `_next_label` + the
//! action. Five action forms: empty (message-only), `((…))`
//! describe-style, `(…)` literal list, `{…}` eval-body, bare `…`
//! command, and `cmd args…` with desc passthrough.

use crate::compsys::ported::_description::description_byname;
use crate::compsys::ported::_message::message_byname;
use crate::compsys::ported::_next_label::next_label_byname;
use crate::compsys::ported::_requested::requested_byname;
use crate::compsys::ported::_tags::tags_byname;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
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

/// Reach `_alternative` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_alternative \` (Completion/Debian/Command/_bts sh:55) — so the normal function lookup runs.
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
pub fn alternative_byname(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_alternative", args, || _alternative(args))
}

/// `_alternative` — try each `tag:descr:action` spec until one
/// produces matches. Returns 0 on first success, 1 if none match.
pub fn _alternative(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_alternative");
    // sh:5
    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let mut subopts: Vec<String> = Vec::new();
    let mut curcontext = saved_curcontext.clone();
    let mut idx = 0usize;

    // sh:7-13  getopts O:/C:
    while idx < args.len() {
        let a = &args[idx];
        if a == "-O" && idx + 1 < args.len() {
            // Read the named array `${(@P)OPTARG}`
            subopts = getaparam(&args[idx + 1]).unwrap_or_default();
            idx += 2;
        } else if a == "-C" && idx + 1 < args.len() {
            // Replace last `:`-field of curcontext
            if let Some(i) = curcontext.rfind(':') {
                curcontext.truncate(i);
            }
            curcontext.push(':');
            curcontext.push_str(&args[idx + 1]);
            let _ = setsparam("curcontext", &curcontext);
            idx += 2;
        } else if a.starts_with('-') && a != "-" && a != "--" {
            // Unknown option terminator
            break;
        } else {
            break;
        }
    }

    // sh:16
    if idx < args.len() && (args[idx] == "-" || args[idx] == "--") {
        idx += 1;
    }

    let defs: Vec<String> = args[idx..].to_vec();
    let mut mesgs: Vec<String> = Vec::new();

    // sh:20  _tags "${(@)argv%%:*}"
    let tag_names: Vec<String> = defs
        .iter()
        .map(|d| d.splitn(2, ':').next().unwrap_or("").to_string())
        .collect();
    let _ = tags_byname(&tag_names);

    let nm_initial: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // sh:22  while _tags …
    loop {
        if tags_byname(&[]) != 0 {
            break;
        }

        // sh:23
        for def in &defs {
            let mut parts = def.splitn(3, ':');
            let tag = parts.next().unwrap_or("").to_string();
            let descr = parts.next().unwrap_or("").to_string();
            let action = parts.next().unwrap_or("").to_string();

            // sh:24
            if requested_byname(&[tag.clone()]) != 0 {
                continue;
            }

            // sh:28
            let _ = description_byname(&[tag.clone(), "expl".to_string(), descr.clone()]);

            // sh:30 action dispatch
            if action.trim().is_empty() {
                // sh:30  Empty action → defer to messages collection.
                mesgs.push(format!("{}:{}", tag, descr));
            } else if action.starts_with("((") && action.ends_with("))") {
                // sh:35  ((value:desc value:desc …)) → describe-style.
                // sh:39 does `eval ws=( ${action[3,-3]} )` — the `eval`
                // strips backslash escapes so `a\:all` becomes `a:all`,
                // letting `_describe` split value from description on the
                // (now unescaped) colon. `split_whitespace().to_string()`
                // kept the backslash, so `_describe` saw `a\:all`, treated
                // the `\:` as a literal (non-separator) colon, and rendered
                // the raw `a:all` instead of `a  -- all` (e.g. `chmod <TAB>`
                // via `_file_modes` → `_alternative 'who:who:((a\:all …))'`).
                let body = &action[2..action.len() - 2];
                let items: Vec<String> = body
                    .split_whitespace()
                    .map(|s| {
                        let mut out = String::with_capacity(s.len());
                        let mut chars = s.chars();
                        while let Some(c) = chars.next() {
                            if c == '\\' {
                                if let Some(n) = chars.next() {
                                    out.push(n);
                                }
                            } else {
                                out.push(c);
                            }
                        }
                        out
                    })
                    .collect();
                setaparam("ws", items);
                let mut describe_argv: Vec<String> = vec![
                    "-t".to_string(),
                    tag.clone(),
                    descr.clone(),
                    "ws".to_string(),
                    "-M".to_string(),
                    "r:|[_-]=* r:|=*".to_string(),
                ];
                describe_argv.extend(subopts.iter().cloned());
                let _ = dispatch_function_call("_describe", &describe_argv);
            } else if action.starts_with('(') && action.ends_with(')') {
                // sh:43  (literal list) → compadd direct
                let body = &action[1..action.len() - 1];
                let items: Vec<String> = body.split_whitespace().map(|s| s.to_string()).collect();
                setaparam("ws", items);
                loop {
                    let mut nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                    if next_label_byname(&nl) != 0 {
                        break;
                    }
                    nl.clear();
                    let expl = getaparam("expl").unwrap_or_default();
                    let mut compadd_argv: Vec<String> = subopts.clone();
                    compadd_argv.extend(expl);
                    compadd_argv.push("-a".to_string());
                    compadd_argv.push("-".to_string());
                    compadd_argv.push("ws".to_string());
                    let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
                }
            } else if action.starts_with('{') && action.ends_with('}') {
                // sh:51  {body} → eval body. Dispatch via execute_script
                //   hook (returns Ok(0) when no executor wired).
                let body = &action[1..action.len() - 1];
                loop {
                    let nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                    if next_label_byname(&nl) != 0 {
                        break;
                    }
                    let _ = crate::ported::exec::execute_script(body);
                }
            } else if action.starts_with(' ') {
                // sh:61 — `eval "action=( $action )"; "$action[@]"`.
                let parts: Vec<String> = crate::compsys::ported::eval_action_words(&action);
                loop {
                    let nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                    if next_label_byname(&nl) != 0 {
                        break;
                    }
                    if let Some(cmd) = parts.first() {
                        let rest: Vec<String> = parts[1..].to_vec();
                        let _ = dispatch_function_call(cmd, &rest);
                    }
                }
            } else {
                // sh:69 — `eval "action=( $action )"`, then cmd args with descs.
                let parts: Vec<String> = crate::compsys::ported::eval_action_words(&action);
                if let Some((cmd, rest)) = parts.split_first() {
                    loop {
                        let nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                        if next_label_byname(&nl) != 0 {
                            break;
                        }
                        let expl = getaparam("expl").unwrap_or_default();
                        let mut call_argv: Vec<String> = subopts.clone();
                        call_argv.extend(expl);
                        call_argv.extend(rest.iter().cloned());
                        let _ = if cmd == "compadd" {
                            bin_compadd("compadd", &call_argv, &make_ops(), 0)
                        } else {
                            dispatch_function_call(cmd, &call_argv).unwrap_or(1)
                        };
                    }
                }
            }
        }
        // sh:73  nm != $compstate[nmatches] → success
        let nm_now: i64 = get_compstate_str("nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if nm_now != nm_initial {
            // Restore + return success
            let _ = setsparam("curcontext", &saved_curcontext);
            return 0;
        }
    }

    // sh:77
    for d in &mesgs {
        let mut parts = d.splitn(2, ':');
        let tag = parts.next().unwrap_or("").to_string();
        let desc = parts.next().unwrap_or("").to_string();
        let _ = message_byname(&["-e".to_string(), tag, desc]);
    }

    // sh:81 restore + fail
    let _ = setsparam("curcontext", &saved_curcontext);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_for_empty_specs() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_alternative(&[]), 1);
    }

    #[test]
    fn returns_one_when_no_tag_requested() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_alternative(&["foo:desc:_files".to_string()]), 1);
    }
}
