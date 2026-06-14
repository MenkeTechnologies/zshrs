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
    (remaining, gopt)
}

/// `$#funcstack`.
fn funcstack_depth() -> usize {
    FUNCSTACK.lock().map(|s| s.len()).unwrap_or(0)
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
fn dispatch_action(action_argv: &[String], prev_arr_vals: &[String], extras: &[String]) -> i32 {
    if action_argv.is_empty() {
        return 1;
    }
    let cmd = &action_argv[0];
    let mut full: Vec<String> = action_argv[1..].to_vec();
    full.extend(prev_arr_vals.iter().cloned());
    full.extend(extras.iter().cloned());

    // The shell evaluates `"$4" args...` as a command — could be a
    // builtin (compadd / compgen / etc.) or a shell function. We
    // route compadd to the real builtin in `src/ported/zle/complete`;
    // everything else goes through the exec-hook dispatch.
    if cmd == "compadd" {
        bin_compadd("compadd", &full, &make_ops(), 0)
    } else {
        dispatch_function_call(cmd, &full).unwrap_or(1)
    }
}

/// `_all_labels` — iterate every tag-spec registered for `$1`,
/// dispatching the supplied action per iteration. Returns 0 if ANY
/// iteration succeeded (`__ret=0`), 1 otherwise (`__ret=1` initial,
/// only flipped on action-success).
pub fn _all_labels(args: &[String]) -> i32 {
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
        //   continue`. When the `_next_tags` shadow is active, skip any
        //   spec already on the not-list. Empty `_next_tags_not` makes
        //   this a no-op, matching the unshadowed body.
        let not = getsparam("_next_tags_not").unwrap_or_default();
        if !not.is_empty() && !spec.is_empty() {
            let needle = format!(" {} ", spec);
            if not.contains(&needle) {
                continue;
            }
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
            dispatch_action(&action, &prev_arr, &extras)
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
            //   per zsh's negative-subscript rule; pre=-2 → len-1
            //   (last element index in 1-based terms). For our [3..]
            //   slice that's the full action chunk through the end.
            let pre_idx: usize = if pre < 0 {
                // pre=-2 → len-1 in 1-based terms; in 0-based slice
                //   bound (exclusive end), that's `argv_len` (full
                //   tail).
                argv_len
            } else {
                // 1-based inclusive → 0-based exclusive: pre+0 (zsh
                //   inclusive end at 1-based pre = 0-based index
                //   pre-1; exclusive end is pre).
                (pre as usize).min(argv_len)
            };
            let suf_idx: usize = if suf < 0 {
                argv_len
            } else {
                ((suf as usize).saturating_sub(1)).min(argv_len)
            };
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
            dispatch_action(&action_chunk, &prev_arr, &extras)
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
            _all_labels(&[
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
            _all_labels(&[
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
        );
        let _ = r;
        INCOMPFUNC.store(0, Ordering::Relaxed);
    }

    #[test]
    fn dispatch_action_empty_command_returns_one() {
        // Guard rail: empty action argv returns 1 (no command to
        //   invoke).
        assert_eq!(dispatch_action(&[], &[], &[]), 1);
    }

    #[test]
    fn dispatch_action_unknown_shell_fn_returns_one() {
        // sh:35/39 — when the action names an unregistered shell fn,
        //   dispatch_function_call returns None → we return 1
        //   (matching shell behavior when the function call fails).
        assert_eq!(
            dispatch_action(&["nonexistent_fn".to_string()], &[], &[]),
            1
        );
    }

    #[test]
    fn has_unescaped_colon_helper() {
        assert!(has_unescaped_colon("foo:bar"));
        assert!(!has_unescaped_colon("foo\\:bar"));
        assert!(!has_unescaped_colon("plain"));
    }

    #[test]
    fn next_tags_not_filter_with_empty_is_noop() {
        // sh:_next_tags:39 — empty `_next_tags_not` keeps the
        //   filter branch dead; unregistered tag still returns 1.
        let r = with_incompfunc(|| {
            setsparam("_next_tags_not", "").unwrap();
            _all_labels(&[
                "unregistered_tag".to_string(),
                "name".to_string(),
                "descr".to_string(),
                "compadd".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }
}
