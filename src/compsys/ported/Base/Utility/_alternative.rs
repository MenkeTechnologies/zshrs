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
//! sh:76    [[ nm -ne compstate[nmatches] ]] && return 0
//! sh:77  done
//! sh:79  for descr in "$mesgs[@]"; do
//! sh:80    _message -e "${descr%%:*}" "${descr#*:}"
//! sh:81  done
//! sh:83  return 1
//! ```
//!
//! Dispatches a list of `tag:description:action` specs, building
//! per-tag completion via `_description` + `_next_label` + the
//! action. Five action forms: empty (message-only), `((…))`
//! describe-style, `(…)` literal list, `{…}` eval-body, bare `…`
//! command, and `cmd args…` with desc passthrough.

use crate::compsys::ported::_description::_description;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
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
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_alternative_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _alternative(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_alternative", args, || _alternative_impl(args))
}

/// `_alternative` — try each `tag:descr:action` spec until one
/// produces matches. Returns 0 on first success, 1 if none match.
pub fn _alternative_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_alternative");
    // sh:3  local tags def expl descr action mesgs nm="$compstate[nmatches]" subopts
    // sh:4  local opt ws curcontext="$curcontext"
    //
    // Of that list this port publishes two names THROUGH the param table —
    // `expl` (handed to `_description` at sh:28, then named again at sh:48 /
    // sh:54 / sh:62 / sh:70) and `ws` (the array-literal target of sh:39 and
    // sh:46's `eval ws\=\( … \)`, read back by name at sh:41 / sh:49);
    // the rest stay Rust locals, and `curcontext` is saved/restored by hand
    // below because sh:9/sh:11 reassign it mid-body.
    //
    // Without the declaration those two writes landed in the GLOBAL param
    // table and OUTLIVED the call, so the caller got `_alternative`'s `expl`
    // back instead of its own: the stock-utility sweep read
    // `expl[2] = '-J' '-default-'` after `_alternative t1:first:(a1 a2)`
    // where zsh reads `expl[0] =`. `local` in zsh SAVES and RESTORES; it
    // neither destroys the outer binding nor leaks the inner value. Same
    // `LocalScope` pattern `_files`/`_command_names` already use for `expl`.
    let _locals = crate::compsys::ported::shared::LocalScope::declare(
        &["expl", "ws"],
        crate::ported::zsh_h::PM_ARRAY,
    );
    // sh:5
    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let mut subopts: Vec<String> = Vec::new();
    let mut curcontext = saved_curcontext.clone();
    let mut idx = 0usize;

    // sh:7-13  getopts O:/C:
    //
    // The loop is written out here instead of calling the `getopts`
    // builtin, so the builtin's PARAMETER side effects have to be
    // reproduced by hand or they are simply missing. `$OPTARG` is the
    // one that escapes: `getopts` stores the consumed option-argument in
    // the C global `zoptarg` (`Src/builtin.c:5763-5776`, `zoptarg = p`),
    // which backs the *special* parameter `OPTARG` — and sh:3-4 declares
    // `local tags def expl descr action mesgs nm subopts` / `local opt ws
    // curcontext` WITHOUT `OPTARG`, so the store lands on the global and
    // outlives the call. (`OPTIND` does not escape: `doshfunc` saves and
    // resets `zoptind`/`optcind` per function entry and restores them on
    // exit, `Src/exec.c:5904-5909` / `:6060-6063` — so every callee sees
    // `OPTIND=1` regardless. Only `zoptarg` is left un-saved.)
    //
    // Losing it is observable, not cosmetic: `_command_names` calls
    // `_alternative -O args …` (the user's
    // `~/.zpwr/autoload/comp_utils/_command_names` sh:55), so under zsh
    // every completer below that frame sees `OPTARG=args`, while zshrs
    // left it empty. `_parameters` lists parameters *with their values as
    // descriptions*, `compdescribe -I … -g` groups matches by identical
    // description (`cd_group`, `Src/Zle/computil.c:142-181`), and an
    // empty `OPTARG` joined the empty-value group — which re-chunked the
    // whole grouped listing to 6 columns where zsh uses 7. The column
    // count is what the match TOTAL is made of, because every gap in a
    // column is padded with `-E<n>` dummy matches (`CRT_DUMMY`,
    // `Src/Zle/computil.c:754-768`): one lost column cost 91 matches, so
    // `-<TAB><TAB>` offered "all 2080 possibilities" against zsh's 2171.
    while idx < args.len() {
        let a = &args[idx];
        if a == "-O" && idx + 1 < args.len() {
            // Read the named array `${(@P)OPTARG}`
            let _ = setsparam("OPTARG", &args[idx + 1]); // c:builtin.c:5776
            subopts = getaparam(&args[idx + 1]).unwrap_or_default();
            idx += 2;
        } else if a == "-C" && idx + 1 < args.len() {
            let _ = setsparam("OPTARG", &args[idx + 1]); // c:builtin.c:5776
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
    let _ = _tags(&tag_names);

    let nm_initial: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // sh:22  while _tags …
    loop {
        if _tags(&[]) != 0 {
            break;
        }

        // sh:23
        for def in &defs {
            let mut parts = def.splitn(3, ':');
            let tag = parts.next().unwrap_or("").to_string();
            let descr = parts.next().unwrap_or("").to_string();
            let action = parts.next().unwrap_or("").to_string();

            // sh:24
            if _requested(&[tag.clone()]) != 0 {
                continue;
            }

            // sh:28
            let _ = _description(&[tag.clone(), "expl".to_string(), descr.clone()]);

            // sh:30 action dispatch
            if action.trim().is_empty() {
                // sh:30  Empty action → defer to messages collection.
                mesgs.push(format!("{}:{}", tag, descr));
            } else if action.starts_with("((") && action.ends_with("))") {
                // sh:35  ((value:desc value:desc …)) → describe-style.
                // sh:39 does `eval ws\=\( "${action[3,-3]}" \)` — a real
                // shell array-literal eval. It both SPLITS on unescaped
                // whitespace and strips the escapes, so `a\:all` becomes
                // one element `a:all` and `add-profile\:add\ PKCS11\ …`
                // stays ONE element with its spaces intact.
                // `split_whitespace()` + a manual unescape got the first
                // half right and the second half wrong: it split at the
                // `\ ` too, so every multi-word description generated by
                // `_regex_words` (sh:45 escapes `[: ()]`) was truncated at
                // its first space — `p11-kit <TAB>` listed `add-profile
                // -- add` instead of `-- add PKCS11 profile to the token`.
                let body = &action[2..action.len() - 2];
                let items: Vec<String> = crate::compsys::ported::eval_action_words(body);
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
                // sh:41 — the line `_describe` is called FROM. `FnScope`
                // zeroes `lineno` for a port body (shared.rs), so without
                // this the frame `_describe` pushes records `_alternative:0`
                // where zsh's `$functrace` reads `_alternative:41`.
                crate::compsys::ported::shared::set_sh_lineno(41);
                let _ = dispatch_function_call("_describe", &describe_argv);
            } else if action.starts_with('(') && action.ends_with(')') {
                // sh:43  (literal list) → compadd direct
                let body = &action[1..action.len() - 1];
                // sh:46 `eval ws\=\( "${action[2,-2]}" \)` — the SAME
                // array-literal eval as the `((…))` arm 17 lines above, so it
                // STRIPS escapes as well as splitting on unescaped whitespace.
                // `split_whitespace()` only split. `_bpf_filters` sh:66 passes
                // the action `(not \()`, whose second word is a
                // backslash-escaped `(`; unstripped, the match became the
                // TWO-character `\(` and compadd then quoted both of them, so
                // `tcpdump <TAB>` listed `\\\(` where zsh lists `\(`.
                let items: Vec<String> = crate::compsys::ported::eval_action_words(body);
                setaparam("ws", items);
                loop {
                    let mut nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                    if _next_label(&nl) != 0 {
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
                    if _next_label(&nl) != 0 {
                        break;
                    }
                    let _ = crate::ported::exec::execute_script(body);
                }
            } else if action.starts_with(' ') {
                // sh:61 — `eval "action=( $action )"; "$action[@]"`.
                let parts: Vec<String> = crate::compsys::ported::eval_action_words(&action);
                loop {
                    let nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                    if _next_label(&nl) != 0 {
                        break;
                    }
                    if let Some(cmd) = parts.first() {
                        let rest: Vec<String> = parts[1..].to_vec();
                        // sh:63 — `"$action[@]"`, the line the callee's frame
                        // records as its caller line (`$functrace` reads
                        // `_alternative:63`).
                        // A bare `dispatch_function_call` resolves SHELL FUNCTIONS and native
                        // ports only (`src/vm_helper.rs:4706` ends in
                        // `functions_compiled.get(name).cloned()?`), so a builtin, a
                        // `$PATH` executable and a NONEXISTENT name all came back
                        // `None` and were flattened to "non-zero" with NO diagnostic.
                        // zsh prints `_alternative:NN: command not found: …` for the
                        // last of those. `dispatch_action_command` publishes the line,
                        // routes `compadd` to the builtin, then falls back to the
                        // builtin table and `findcmd` before reporting.
                        // `compadd` is a BUILTIN, so `dispatch_function_call`
                        // finds no shell function and the action adds NOTHING.
                        // The sh:69 arm 40 lines below already handles this;
                        // this arm did not, and `_values` has it on BOTH of
                        // its arms — so the omission is here, not the pattern.
                        //
                        // Live cost: `_rsync:16` is
                        // `rsync='rsync:rsync: compadd -S "" rsync://'`, so
                        // `rsync <TAB>` offered 229 matches where zsh offers
                        // 230 — the missing one is the literal `rsync://`.
                        let _ = crate::compsys::ported::shared::dispatch_action_command(cmd, &rest, 63);
                    }
                }
            } else {
                // sh:69 — `eval "action=( $action )"`, then cmd args with descs.
                let parts: Vec<String> = crate::compsys::ported::eval_action_words(&action);
                if let Some((cmd, rest)) = parts.split_first() {
                    loop {
                        let nl = vec![tag.clone(), "expl".to_string(), descr.clone()];
                        if _next_label(&nl) != 0 {
                            break;
                        }
                        let expl = getaparam("expl").unwrap_or_default();
                        let mut call_argv: Vec<String> = subopts.clone();
                        call_argv.extend(expl);
                        call_argv.extend(rest.iter().cloned());
                        // sh:71 — `"$action[1]" "$subopts[@]" "$expl[@]"
                        // "${(@)action[2,-1]}"`, the line the callee's frame
                        // records as its caller line.
                        // Same silent-dispatch gap as the sh:63 arm above.
                        let _ = crate::compsys::ported::shared::dispatch_action_command(cmd, &call_argv, 71);
                    }
                }
            }
        }
        // sh:76  nm != $compstate[nmatches] → success
        let nm_now: i64 = get_compstate_str("nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if nm_now != nm_initial {
            // Restore + return success
            let _ = setsparam("curcontext", &saved_curcontext);
            return 0;
        }
    }

    // sh:79
    for d in &mesgs {
        let mut parts = d.splitn(2, ':');
        let tag = parts.next().unwrap_or("").to_string();
        let desc = parts.next().unwrap_or("").to_string();
        let _ = _message(&["-e".to_string(), tag, desc]);
    }

    // sh:83 restore + fail
    let _ = setsparam("curcontext", &saved_curcontext);
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_for_empty_specs() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_alternative_impl(&[]), 1);
    }

    #[test]
    fn returns_one_when_no_tag_requested() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_alternative_impl(&["foo:desc:_files".to_string()]), 1);
    }
}
