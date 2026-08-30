//! Port of `_all_labels` from `Completion/Base/Core/_all_labels`.
//!
//! Full upstream body (43 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local __gopt __len __tmp __pre __suf __ret=1 __descr __spec __prev
//! sh: 4
//! sh: 5  if [[ "$1" = - ]]; then
//! sh: 6    __prev=-
//! sh: 7    shift
//! sh: 8  fi
//! sh: 9
//! sh:10  __gopt=()
//! sh:11  zparseopts -D -a __gopt 1 2 V J x
//! sh:12
//! sh:13  __tmp=${argv[(ib:4:)-]}
//! sh:14  __len=$#
//! sh:15  if [[ __tmp -lt __len ]]; then
//! sh:16    __pre=$(( __tmp-1 ))
//! sh:17    __suf=$__tmp
//! sh:18  elif [[ __tmp -eq $# ]]; then
//! sh:19    __pre=-2
//! sh:20    __suf=$(( __len+1 ))
//! sh:21  else
//! sh:22    __pre=4
//! sh:23    __suf=5
//! sh:24  fi
//! sh:25
//! sh:26  while comptags "-A$__prev" "$1" curtag __spec; do
//! sh:27    (( $#funcstack > _tags_level )) && _comp_tags="${_comp_tags% * }"
//! sh:28    _tags_level=$#funcstack
//! sh:29    _comp_tags="$_comp_tags $__spec "
//! sh:30    if [[ "$curtag" = *[^\\]:* ]]; then
//! sh:31      zformat -f __descr "${curtag#*:}" "d:$3"
//! sh:32      _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
//! sh:33      curtag="${curtag%:*}"
//! sh:34
//! sh:35      "$4" "${(P@)2}" "${(@)argv[5,-1]}" && __ret=0
//! sh:36    else
//! sh:37      _description "$__gopt[@]" "$curtag" "$2" "$3"
//! sh:38
//! sh:39      "${(@)argv[4,__pre]}" "${(P@)2}" "${(@)argv[__suf,-1]}" && __ret=0
//! sh:40    fi
//! sh:41  done
//! sh:42
//! sh:43  return __ret
//! ```
//!
//! Calls real `bin_comptags`/`bin_zformat`/`bin_zparseopts`; reads
//! real `FUNCSTACK`. The action at `$4` (or `${(@)argv[4,__pre]}`)
//! dispatches: when it's "compadd" we call `bin_compadd` directly;
//! otherwise we delegate to `crate::ported::exec::dispatch_function_call`
//! (returns None without an executor — in that case `__ret` stays 1
//! for that iteration, matching shell behavior when the action fn
//! returns non-zero).
//!
//! `_description` is reached BY NAME (`_description` →
//! [`crate::compsys::ported::shared::call_compfn`]), matching the sh body's
//! bare `_description …` command word: a user's own copy earlier on
//! `$fpath` wins, and the call gets its own `doshfunc` frame.

use super::_description::_description;
use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::parameter::FUNCSTACK;
use crate::ported::modules::zutil::{bin_zformat, bin_zparseopts};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::get_compstate_str as _get_compstate_str;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zle::computil::bin_comptags;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:11 — bridge to real `bin_zparseopts -D -a __gopt 1 2 V J x`
/// via `-v <name>` (named array source).
fn run_gopt(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("__gopt", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "__gopt".to_string(),
            "1".to_string(),
            "2".to_string(),
            "V".to_string(),
            "J".to_string(),
            "x".to_string(),
        ],
        &make_ops(),
        0,
    );
    let gopt = getaparam("__gopt").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, gopt)
}

/// `$#funcstack`.
fn funcstack_depth() -> usize {
    FUNCSTACK.lock().map(|s| s.len()).unwrap_or(0)
}

/// sh:_next_tags:39 / :66 — the `[[ "$_next_tags_not" = *\ ${__spec}\ * ]]`
/// glob test. Returns true when `spec` occurs as a whole, space-delimited
/// entry inside `not` (a space required on BOTH sides, exactly like the zsh
/// glob). This is how the Rust port replaces the `_next_tags` widget's
/// `unfunction _all_labels _next_label; <redefine>` trick (sh:10-82): a
/// shell-function body can't be injected from Rust, so the ports read
/// `$_next_tags_not` themselves. Empty `not`/`spec` → false, keeping the
/// default (unshadowed) path unchanged.
fn spec_in_not_list(spec: &str, not: &str) -> bool {
    !not.is_empty() && !spec.is_empty() && not.contains(&format!(" {} ", spec))
}

/// sh:39 — the two array-slice bounds of
/// `"${(@)argv[4,__pre]}" … "${(@)argv[__suf,-1]}"`, translated from
/// zsh's 1-based, possibly-negative subscripts to 0-based Rust slice
/// bounds over the post-`zparseopts` `argv`.
///
/// Returns `(pre_idx, suf_idx)`: the action chunk is `argv[3..pre_idx]`
/// and the trailing extras are `argv[suf_idx..]`.
///
/// zsh's negative subscript `-k` names the 1-based element `len-k+1`, so
/// the sh:19 branch's `__pre=-2` is `len-1`: `argv[4,-2]` DROPS the last
/// element. That last element is exactly the bare `-` separator the
/// caller wrote to mark where `$expl` should be spliced
/// (`Completion/Unix/Type/_user_at_host:30`,
/// `Completion/Unix/Type/_directories:5`, `Completion/Unix/Command/_rsync:11`).
/// Reading `__pre=-2` as `len` instead left that `-` in the action's own
/// argv, where it eventually reached `compadd` and ended its option
/// parsing early (`Src/Zle/complete.c:635-637`
/// `if (!(*argv)[1]) { argv++; break; }`), turning every following
/// option word into a literal match.
fn slice_bounds(pre: isize, suf: isize, argv_len: usize) -> (usize, usize) {
    let len = argv_len as isize;
    // 1-based inclusive end → 0-based exclusive end is the same number
    //   once a negative index has been resolved to `len + pre + 1`.
    let pre_idx = if pre < 0 { len + pre + 1 } else { pre }.clamp(0, len) as usize;
    // 1-based start → 0-based start is one less; a negative start
    //   resolves to `len + suf + 1` first. (sh:16-20 never produces a
    //   negative `__suf`; both bounds follow one rule so they can't
    //   drift apart.)
    let suf_idx = if suf < 0 { len + suf } else { suf - 1 }.clamp(0, len) as usize;
    (pre_idx, suf_idx)
}

/// sh:12 — `*[^\\]:*` test.
fn has_unescaped_colon(s: &str) -> bool {
    let b = s.as_bytes();
    for i in 0..b.len() {
        if b[i] == b':' && (i == 0 || b[i - 1] != b'\\') {
            return true;
        }
    }
    false
}

/// sh:35 / sh:39 — invoke the user-supplied action.
///   action_argv = the action command-line (e.g. `["compadd", "-X",
///   "..."]` or `["_files"]`).
///   prev_arr_vals = `${(P@)2}` — the current value of the
///   per-`_description` array named by the caller's $2.
///   extras = `${(@)argv[5,-1]}` or `${(@)argv[__suf,-1]}`.
///   `line` = the upstream line the command sits on (35 or 39), published
///   for the `scriptname:lineno:` prefix any diagnostic it raises carries.
fn dispatch_action(
    action_argv: &[String],
    prev_arr_vals: &[String],
    extras: &[String],
    line: u64,
) -> i32 {
    // sh:35 / sh:39 — the command line is the CONCATENATION of the action
    // words, `${(P@)2}` and the trailing arguments, and the COMMAND WORD is
    // whatever that concatenation starts with. It is not necessarily an
    // action word: called with no action at all (`_all_labels values expl
    // value`) sh:13-23 settles on `__pre=4`/`__suf=5`, `${(@)argv[4,4]}` is
    // empty on a 3-element `argv`, and the command word becomes `$expl[1]`.
    // zsh then reports `_all_labels:39: command not found: -J`.
    //
    // The port read `action_argv[0]` as the command and returned 1 outright
    // for an empty action, so the whole diagnostic was swallowed — silently,
    // which is the failure mode the sweep caught on both `_all_labels
    // no-action` and `_wanted no-action`.
    let mut full: Vec<String> = action_argv.to_vec();
    full.extend(prev_arr_vals.iter().cloned());
    full.extend(extras.iter().cloned());

    if full.is_empty() {
        // A command line that expands to no words at all runs nothing and
        // leaves status 0 (`zsh -fc 'x=(); "$x[@]"; echo $?'` → `0`).
        return 0;
    }
    let cmd = full.remove(0);

    // `lineno` is what `zerrmsg` prints after the function name
    // (`Src/utils.c:301-305`), and `FnScope` zeroed it on entry to this
    // body, so publish the upstream line before anything can diagnose.
    crate::compsys::ported::shared::set_sh_lineno(line);

    // The shell evaluates the resulting word list as a command — it could
    // be a builtin (compadd / compgen / etc.) or a shell function. We
    // route compadd to the real builtin in `src/ported/zle/complete`;
    // everything else goes through the exec-hook dispatch.
    if cmd == "compadd" {
        return bin_compadd("compadd", &full, &make_ops(), 0);
    }
    if let Some(rc) = dispatch_function_call(&cmd, &full) {
        return rc;
    }

    // Neither a shell function nor a registered port. zsh looks for a
    // builtin and then for an executable on `$PATH`; only when both miss
    // does it report the command as not found.
    if crate::ported::builtin::createbuiltintable().contains_key(cmd.as_str())
        || crate::ported::exec::findcmd(&cmd, 0, 0).is_some()
    {
        // The word names something real that this dispatch has no path to.
        // Keep the pre-existing "action did not succeed" answer rather than
        // inventing an execution route here.
        return 1;
    }

    // c:Src/exec.c:903 — `zerr("command not found: %s", arg0);`
    //
    // `zwarn`, NOT `zerr`: c:903 runs in the FORKED child of `execcmd`,
    // which `_exit(127)`s two lines later at c:908, so the parent shell's
    // `errflag` is never raised. `zerr` in this port runs in the live shell
    // and would set ERRFLAG_ERROR (`src/ported/utils.rs:236`), abandoning
    // the rest of the completion. The rendered text is identical — both
    // reach `zwarning` (`Src/utils.c:147-155`), which prints
    // `scriptname:lineno: msg`.
    crate::ported::utils::zwarn(&format!("command not found: {}", cmd)); // c:903
    127 // c:908 — `_exit((eno == EACCES || eno == ENOEXEC) ? 126 : 127)`
}

/// Reach `_all_labels` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_all_labels options expl option \`
/// (Completion/Base/Utility/_arguments sh:493) — so the normal function
/// lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_all_labels_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _all_labels(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_all_labels", args, || _all_labels_impl(args))
}

/// `_all_labels` — iterate every tag-spec registered for `$1`,
/// dispatching the supplied action per iteration. Returns 0 if ANY
/// iteration succeeded (`__ret=0`), 1 otherwise (`__ret=1` initial,
/// only flipped on action-success).
pub fn _all_labels_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_all_labels");
    // sh:3  local __gopt __len __tmp __pre __suf __ret=1 __descr __spec __prev
    crate::compsys::ported::shared::declare_locals(
        &[
            "__gopt", "__len", "__tmp", "__pre", "__suf", "__ret", "__descr", "__spec", "__prev",
        ],
        0,
    );
    // sh:3 locals
    let mut ret: i32 = 1;

    // sh:5-8  leading `-` flag → prev = "-"
    let (prev, argv): (&str, Vec<String>) = if !args.is_empty() && args[0] == "-" {
        ("-", args[1..].to_vec())
    } else {
        ("", args.to_vec())
    };

    // sh:10-11
    let (mut argv, gopt) = run_gopt(&argv);

    // sh:13-24  compute __pre / __suf based on whether a `-` separator
    //   appears at or after index 4 (1-based) in remaining argv.
    let argv_len = argv.len();
    // 1-based search: argv[(ib:4:)-]. Translate to 0-based: scan from
    //   index 3 onward. Returns 1-based index in shell, or len+1 when
    //   not found (zsh's array-not-found convention for (i) flag).
    let tmp_1based: usize = if argv_len >= 4 {
        argv[3..]
            .iter()
            .position(|s| s == "-")
            .map(|p| p + 4) // back to 1-based, offset by the 3-skip
            .unwrap_or(argv_len + 1)
    } else {
        argv_len + 1
    };

    // sh:15-24
    let (pre, suf): (isize, isize) = if (tmp_1based as isize) < (argv_len as isize) {
        // sh:16-17 — `-` found before end
        ((tmp_1based as isize) - 1, tmp_1based as isize)
    } else if tmp_1based == argv_len {
        // sh:19-20 — `-` is the last arg
        (-2, (argv_len as isize) + 1)
    } else {
        // sh:22-23 — no `-` found
        (4, 5)
    };

    // sh:26  while comptags -A$prev "$1" curtag __spec
    let arg1 = argv.first().cloned().unwrap_or_default();
    loop {
        let comptags_argv = vec![
            format!("-A{}", prev),
            arg1.clone(),
            "curtag".to_string(),
            "__spec".to_string(),
        ];
        // sh:26 — the line `comptags` is called FROM. `FnScope` zeroes
        // `lineno` on entry to a port body, so without this the builtin's
        // own diagnostics lost their line: zsh reports
        // `_all_labels:comptags:26: no tags registered`, the port reported
        // `_all_labels:comptags: no tags registered`. `zerrmsg` prints the
        // field only when `lineno` is non-zero (`Src/utils.c:301-305`).
        crate::compsys::ported::shared::set_sh_lineno(26);
        if bin_comptags("comptags", &comptags_argv, &make_ops(), 0) != 0 {
            break;
        }

        // sh:27-29 — funcstack housekeeping
        let cur_depth = funcstack_depth();
        let prev_level = getsparam("_tags_level")
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0);
        if cur_depth > prev_level {
            let comp_tags = getsparam("_comp_tags").unwrap_or_default();
            let trimmed = comp_tags.trim_end_matches(' ');
            let last_sp = trimmed.rfind(' ');
            let kept = match last_sp {
                Some(i) => &trimmed[..i],
                None => "",
            };
            let mut rebuilt = String::from(kept);
            if !rebuilt.is_empty() {
                rebuilt.push(' ');
            }
            let _ = setsparam("_comp_tags", &rebuilt);
        }
        let _ = setsparam("_tags_level", &cur_depth.to_string());

        let spec = getsparam("__spec").unwrap_or_default();
        // sh:_next_tags:39 — `[[ "$_next_tags_not" = *\ ${__spec}\ * ]] &&
        //   continue`. Skip any spec already shown in a previous cycle.
        //   This natively reproduces the tag-filtering `_all_labels` body
        //   that the `_next_tags` widget installs via
        //   `unfunction _all_labels; _all_labels() { … }` at sh:10-82 — a
        //   Rust port can't inject a shell-function body, so it consults
        //   `$_next_tags_not` directly instead. Empty `_next_tags_not`
        //   makes this a no-op, matching the unshadowed body exactly.
        if spec_in_not_list(&spec, &getsparam("_next_tags_not").unwrap_or_default()) {
            continue;
        }
        let mut comp_tags = getsparam("_comp_tags").unwrap_or_default();
        comp_tags.push_str(&format!(" {} ", spec));
        let _ = setsparam("_comp_tags", &comp_tags);

        // sh:30
        let curtag = getsparam("curtag").unwrap_or_default();
        let name = argv.get(1).cloned().unwrap_or_default();
        let descr_arg = argv.get(2).cloned().unwrap_or_default();

        let action_invoked = if has_unescaped_colon(&curtag) {
            // sh:31  zformat -f __descr "${curtag#*:}" "d:$3"
            let after = curtag.splitn(2, ':').nth(1).unwrap_or("").to_string();
            let _ = setsparam("__descr", "");
            let _ = bin_zformat(
                "zformat",
                &[
                    "-f".to_string(),
                    "__descr".to_string(),
                    after,
                    format!("d:{}", descr_arg),
                ],
                &make_ops(),
                0,
            );
            let descr = getsparam("__descr").unwrap_or_default();

            // sh:32  _description "$__gopt[@]" "${curtag%:*}" "$2" "$__descr"
            let lhs = curtag
                .rsplitn(2, ':')
                .nth(1)
                .map(|s| s.to_string())
                .unwrap_or_else(|| curtag.clone());
            let mut desc_argv = gopt.clone();
            desc_argv.push(lhs);
            desc_argv.push(name.clone());
            desc_argv.push(descr);
            let _ = _description(&desc_argv);

            // sh:35  "$4" "${(P@)2}" "${(@)argv[5,-1]}" && ret=0
            //   $4 = argv[3] (0-based); extras = argv[4..].
            let action: Vec<String> = if argv.len() > 3 {
                vec![argv[3].clone()]
            } else {
                Vec::new()
            };
            let extras: Vec<String> = if argv.len() > 4 {
                argv[4..].to_vec()
            } else {
                Vec::new()
            };
            let prev_arr = getaparam(&name).unwrap_or_default();
            dispatch_action(&action, &prev_arr, &extras, 35)
        } else {
            // sh:37  _description "$__gopt[@]" "$curtag" "$2" "$3"
            let mut desc_argv = gopt.clone();
            desc_argv.push(curtag);
            desc_argv.push(name.clone());
            desc_argv.push(descr_arg);
            let _ = _description(&desc_argv);

            // sh:39  "${(@)argv[4,__pre]}" "${(P@)2}" "${(@)argv[__suf,-1]}" && ret=0
            //   Translate 1-based [4,__pre] to 0-based [3..pre]
            //   (inclusive). Negative __pre (-2) means "len + __pre + 1"
            //   per zsh's negative-subscript rule: on a 13-element argv,
            //   `argv[4,-2]` is 1-based [4,12] — it DROPS the last
            //   element, which in the sh:19 branch is exactly the bare
            //   `-` separator that ended the action's own arguments.
            //   The port used `argv_len` here (an inclusive end of
            //   `len`, i.e. -1 not -2), so that `-` stayed in the
            //   action's argv: `_user_at_host`'s
            //   `_wanted users expl user _combination … -q "$@" -`
            //   handed `_combination`/`_users` a stray `-`, which ended
            //   compadd's option parsing early (`Src/Zle/complete.c:635`
            //   `if (!(*argv)[1]) { argv++; break; }`) and turned the
            //   REST of the option list into literal match words —
            //   `finger -<TAB>` offered `-  -J  -X  -k` instead of the
            //   132 users zsh offers.
            let (pre_idx, suf_idx) = slice_bounds(pre, suf, argv_len);
            let action_chunk: Vec<String> = if argv_len > 3 && pre_idx > 3 {
                argv[3..pre_idx].to_vec()
            } else {
                Vec::new()
            };
            let extras: Vec<String> = if suf_idx < argv_len {
                argv[suf_idx..].to_vec()
            } else {
                Vec::new()
            };
            let prev_arr = getaparam(&name).unwrap_or_default();
            dispatch_action(&action_chunk, &prev_arr, &extras, 39)
        };

        if action_invoked == 0 {
            ret = 0;
        }
    }

    // sh:43
    let _ = &mut argv;
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::zle::complete::INCOMPFUNC;
    use std::sync::atomic::Ordering;

    fn with_incompfunc<T, F: FnOnce() -> T>(f: F) -> T {
        let _g = crate::test_util::global_state_lock();
        let prev = INCOMPFUNC.load(Ordering::Relaxed);
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = f();
        INCOMPFUNC.store(prev, Ordering::Relaxed);
        r
    }

    #[test]
    fn no_specs_returns_one() {
        // sh:43 — initial ret=1; loop body never executes when
        //   comptags -A returns non-zero on its first call.
        let r = with_incompfunc(|| {
            _all_labels_impl(&[
                "unregistered_tag".to_string(),
                "name".to_string(),
                "descr".to_string(),
                "compadd".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }

    #[test]
    fn leading_dash_sets_prev_marker() {
        // sh:5-8 — `-` first arg → prev = "-"; subsequent comptags
        //   call gets `-A-` (preceding-level lookup).
        let r = with_incompfunc(|| {
            _all_labels_impl(&[
                "-".to_string(),
                "unregistered".to_string(),
                "name".to_string(),
                "d".to_string(),
                "compadd".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }

    #[test]
    fn dispatch_action_compadd_returns_compadd_status() {
        // sh:35 — when action is "compadd", route to bin_compadd.
        //   Without registered tags, bin_compadd returns 1 (no
        //   matches added); we just verify no panic + integer return.
        let _g = crate::test_util::global_state_lock();
        INCOMPFUNC.store(1, Ordering::Relaxed);
        let r = dispatch_action(
            &["compadd".to_string()],
            &["-J".to_string(), "-default-".to_string()],
            &["alpha".to_string(), "beta".to_string()],
            35,
        );
        let _ = r;
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }

    #[test]
    fn dispatch_action_no_words_at_all_returns_zero() {
        // sh:39 — when the action words, `${(P@)2}` and the trailing
        // arguments ALL expand to nothing, zsh runs no command and the
        // status is 0:
        //     % zsh -fc 'x=(); "$x[@]"; echo $?'
        //     0
        // (unreachable in practice: `_description` has already filled the
        // array named by `$2` with at least `-J -default-`).
        assert_eq!(dispatch_action(&[], &[], &[], 39), 0);
    }

    #[test]
    fn dispatch_action_reports_command_not_found_for_an_expl_flag() {
        // sh:39 — `_all_labels values expl value` has no action, so
        // `${(@)argv[4,__pre]}` is empty and the command word is
        // `$expl[1]`. zsh answers
        //     _all_labels:39: command not found: -J
        // with status 127 (c:Src/exec.c:903 + :908). The port used to
        // return 1 and print nothing.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            dispatch_action(&[], &["-J".to_string(), "-default-".to_string()], &[], 39),
            127
        );
    }

    #[test]
    fn dispatch_action_unknown_shell_fn_reports_command_not_found() {
        // sh:35/39 — an action naming nothing that exists is a
        // command-not-found in zsh too, exit 127 (c:Src/exec.c:908).
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            dispatch_action(&["nonexistent_fn_zpf".to_string()], &[], &[], 35),
            127
        );
    }

    #[test]
    fn slice_bounds_trailing_dash_branch_drops_the_separator() {
        // sh:13-20 for `_user_at_host:30`'s
        //   `_wanted users expl user _combination -s '[:@]' other-accounts \
        //      users-hosts users -S @ -q -`
        //   → argv (post-zparseopts) is 13 elements with the bare `-` LAST,
        //   so `__tmp == $#` → `__pre=-2`, `__suf=$#+1`.
        //   `argv[4,-2]` is 1-based [4,12]: the action keeps `-S @ -q` but
        //   NOT the trailing `-`, and `argv[14,-1]` is empty.
        let argv: Vec<&str> = vec![
            "users",
            "expl",
            "user",
            "_combination",
            "-s",
            "[:@]",
            "other-accounts",
            "users-hosts",
            "users",
            "-S",
            "@",
            "-q",
            "-",
        ];
        let (pre_idx, suf_idx) = slice_bounds(-2, argv.len() as isize + 1, argv.len());
        assert_eq!(
            &argv[3..pre_idx],
            &[
                "_combination",
                "-s",
                "[:@]",
                "other-accounts",
                "users-hosts",
                "users",
                "-S",
                "@",
                "-q"
            ],
            "the bare `-` separator must not survive into the action's argv"
        );
        assert_eq!(suf_idx, argv.len(), "nothing follows the separator");
    }

    #[test]
    fn slice_bounds_separator_in_the_middle_splits_around_it() {
        // sh:15-17 — `_users`' `_wanted users expl user compadd "$@" -k - userdirs`
        //   puts the bare `-` at index 10 of 11, i.e. `__tmp < $#` →
        //   `__pre=__tmp-1`, `__suf=__tmp`. The action is argv[4,9]
        //   (`compadd … -k`) and the extras are argv[10,-1] (`- userdirs`),
        //   with `$expl` spliced between them.
        let argv: Vec<&str> = vec![
            "users", "expl", "user", "compadd", "-J", "users", "-X", "fmt", "-k", "-", "userdirs",
        ];
        let tmp = 10isize; // ${argv[(ib:4:)-]}
        let (pre_idx, suf_idx) = slice_bounds(tmp - 1, tmp, argv.len());
        assert_eq!(
            &argv[3..pre_idx],
            &["compadd", "-J", "users", "-X", "fmt", "-k"]
        );
        assert_eq!(&argv[suf_idx..], &["-", "userdirs"]);
    }

    #[test]
    fn slice_bounds_no_separator_takes_only_the_command_word() {
        // sh:22-23 — no bare `-` at/after index 4: `__pre=4`, `__suf=5`, so
        //   the action is the single word argv[4] and everything from
        //   argv[5] on is appended after `$expl`.
        let argv: Vec<&str> = vec!["files", "expl", "file", "_files", "-/", "-W", "/tmp"];
        let (pre_idx, suf_idx) = slice_bounds(4, 5, argv.len());
        assert_eq!(&argv[3..pre_idx], &["_files"]);
        assert_eq!(&argv[suf_idx..], &["-/", "-W", "/tmp"]);
    }

    #[test]
    fn has_unescaped_colon_helper() {
        assert!(has_unescaped_colon("foo:bar"));
        assert!(!has_unescaped_colon("foo\\:bar"));
        assert!(!has_unescaped_colon("plain"));
    }

    #[test]
    fn spec_in_not_list_skips_listed_spec() {
        // sh:_next_tags:39 — a spec bracketed by spaces in $_next_tags_not
        //   is on the not-list, so the loop `continue`s past it (skips it).
        assert!(spec_in_not_list("files", " files directories "));
        assert!(spec_in_not_list("directories", " files directories "));
    }

    #[test]
    fn spec_in_not_list_default_and_boundary() {
        // Default path: empty $_next_tags_not never filters.
        assert!(!spec_in_not_list("files", ""));
        // Empty spec never filters (guards the `${__spec}` == "" edge).
        assert!(!spec_in_not_list("", " files "));
        // An unlisted spec is not skipped.
        assert!(!spec_in_not_list("options", " files directories "));
        // sh-glob parity: the trailing entry has no following space (the
        //   widget appends `" $tags"` with no trailing space at sh:95), so
        //   `*\ ${__spec}\ *` does NOT match it — mirrored here.
        assert!(!spec_in_not_list("dirs", " files dirs"));
    }

    #[test]
    fn next_tags_not_filter_with_empty_is_noop() {
        // sh:_next_tags:39 — empty `_next_tags_not` keeps the
        //   filter branch dead; unregistered tag still returns 1.
        let r = with_incompfunc(|| {
            setsparam("_next_tags_not", "").unwrap();
            _all_labels_impl(&[
                "unregistered_tag".to_string(),
                "name".to_string(),
                "descr".to_string(),
                "compadd".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }
}
