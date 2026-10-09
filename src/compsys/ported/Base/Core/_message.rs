//! Port of `_message` from `Completion/Base/Core/_message`.
//!
//! Full upstream body (47 lines verbatim, master 8cc5eade5f):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local format raw
//! sh: 4  local -a gopt
//! sh: 5  local -A opth
//! sh: 6
//! sh: 7  zparseopts -A opth -D -F - \
//! sh: 8    {1+,2+,V+:,J+:}=gopt F: M: n o: q s: S: x: X: \
//! sh: 9    e r \
//! sh:10  || return
//! sh:11
//! sh:12  if (( $+opth[-e] )); then
//! sh:13    local expl ret=1 tag
//! sh:14
//! sh:15    _comp_mesg=yes
//! sh:16
//! sh:17    if (( $# > 1 )); then
//! sh:18      tag=$1
//! sh:19      shift
//! sh:20    else
//! sh:21      tag="$curtag"
//! sh:22    fi
//! sh:23    _tags "$tag" && while _next_label "$tag" expl "$1"; do
//! sh:24      compadd ${expl:/-X/-x}
//! sh:25      ret=0
//! sh:26    done
//! sh:27
//! sh:28    (( ! $compstate[nmatches] )) && [[ $compstate[insert] = *unambiguous* ]] &&
//! sh:29        compstate[insert]=
//! sh:30
//! sh:31    return ret
//! sh:32  fi
//! sh:33
//! sh:34  _tags messages || return 1
//! sh:35
//! sh:36  if (( $+opth[-r] )); then
//! sh:37    raw=yes format=$1
//! sh:38  else
//! sh:39    zstyle -s ":completion:${curcontext}:messages" format format ||
//! sh:40        zstyle -s ":completion:${curcontext}:descriptions" format format
//! sh:41  fi
//! sh:42
//! sh:43  if [[ -n "$format$raw" ]]; then
//! sh:44    [[ -z "$raw" ]] && zformat -Fq format "$format" "d:$1" "${(@)argv[2,-1]}"
//! sh:45    builtin compadd "$gopt[@]" -x "$format"
//! sh:46    _comp_mesg=yes
//! sh:47  fi
//! ```
//!
//! Calls real `bin_compadd`, `bin_zparseopts`, `bin_zformat`,
//! `lookupstyle`. Cross-fn calls (`_tags`, `_next_label`) go through
//! sibling ports. Reads/writes `$compstate[insert]`/`[nmatches]`
//! through `getsparam`/`setsparam` on `compstate[KEY]`.

use super::_next_label::_next_label_impl;
use super::_tags::_tags_impl;
use crate::compsys::ported::shared::zstyle_s;
use crate::ported::modules::zutil::{bin_zformat, bin_zparseopts};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::complete::{bin_compadd, bin_compadd_body};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}
/// sh:3-10 — `local -a gopt; local -A opth` and
/// `zparseopts -A opth -D -F - {1+,2+,V+:,J+:}=gopt F: M: n o: q s: S: x: X: e r || return`,
/// run through the real `bin_zparseopts` via `-v <name>`.
///
/// Returns the arguments left after the options, the `gopt` array (the
/// `-1`/`-2`/`-V grp`/`-J grp` group options handed on to compadd) and the
/// keys of `opth`; `Err(status)` when zparseopts failed (`|| return`).
fn parse_message_opts(args: &[String]) -> Result<(Vec<String>, Vec<String>, Vec<String>), i32> {
    use crate::ported::zsh_h::{PM_ARRAY, PM_HASHED};
    let src = "__compsys_argv";
    crate::compsys::ported::shared::declare_locals(&["format", "raw"], 0); // sh:3
    crate::compsys::ported::shared::declare_locals(&["gopt"], PM_ARRAY); // sh:4
    crate::compsys::ported::shared::declare_locals(&["opth"], PM_HASHED); // sh:5
    crate::compsys::ported::shared::set_bridge_argv(src, args);
    setaparam("gopt", Vec::new());
    // sh:7-9 — `{1+,2+,V+:,J+:}=gopt` is brace-expanded by the shell
    // before zparseopts sees it.
    let zpo: Vec<String> = [
        "-A", "opth", "-D", "-F", "-v", src, "-", "1+=gopt", "2+=gopt", "V+:=gopt", "J+:=gopt",
        "F:", "M:", "n", "o:", "q", "s:", "S:", "x:", "X:", "e", "r",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let rc = bin_zparseopts("zparseopts", &zpo, &make_ops(), 0);
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);
    if rc != 0 {
        return Err(rc); // sh:10 `|| return`
    }
    let gopt = getaparam("gopt").unwrap_or_default();
    let opth = crate::ported::params::gethkparam("opth").unwrap_or_default();
    Ok((remaining, gopt, opth))
}

/// Reach `_message` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_message kind` (Completion/Unix/Command/_ctags sh:44) — so
/// the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_message_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _message(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_message", args, || _message_impl(args))
}

/// `_message` — render a static message into the current completion
/// listing. Two modes:
///   * `-e <tag>? <description>` — emit per-spec messages via
///     `_next_label` loop (sh:5-25).
///   * default — pull message format from `messages` zstyle and emit
///     via `compadd -x` (sh:27-45).
pub fn _message_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_message");
    // sh:3-10
    let (mut argv, gopt, opth) = match parse_message_opts(args) {
        Ok(parsed) => parsed,
        Err(rc) => return rc,
    };
    // sh:12  if (( $+opth[-e] )); then
    if opth.iter().any(|k| k == "-e") {
        // sh:13 `local expl ret=1 tag`. `_next_label` below writes `expl`
        // through its NAME, so without the shell binding the array landed in
        // the caller's scope and outlived the call: `_xt_session_id` (sh:3,
        // a bare `_message -e ids 'session ID'`) left `expl=(-J -default-)`
        // behind where zsh leaves the caller's `expl` untouched.
        crate::compsys::ported::shared::declare_locals(&["expl", "ret", "tag"], 0);
        let mut ret: i32 = 1;
        // sh:15
        let _ = setsparam("_comp_mesg", "yes");

        // sh:17-22 — `-e` was taken out by zparseopts -D, so `$1` is the
        // tag when two words remain and the description otherwise.
        let (tag, descr): (String, String) = if argv.len() > 1 {
            (argv[0].clone(), argv[1].clone()) // sh:18-19 tag=$1; shift
        } else {
            (
                getsparam("curtag").unwrap_or_default(), // sh:21
                argv.first().cloned().unwrap_or_default(),
            )
        };

        // sh:23  _tags "$tag" && while _next_label "$tag" expl "$1"
        //
        // `comptags` is indexed by `locallevel`, and in zsh `_message` is a
        // real shell function, so its `_tags` registers ONE level below the
        // caller's and is discarded on return. The Rust port calls the
        // sibling `_tags` directly, which skips doshfunc's inc_locallevel —
        // so this registration REPLACED the caller's. Concretely: an
        // `_arguments` spec with an empty-action positional (`'*:key
        // sequence: '`, `'*:in-string: '`) runs `_message -e` inside the
        // `while _tags` loop; the clobber dropped the pending `options` tag
        // set, so the loop re-offered the argument tag and option
        // completion never happened — `bindkey -`, `fd -`, `rustup -` and
        // every other such spec silently completed nothing. Same guard as
        // _requested.rs. The `_next_label` loop must run INSIDE the nested
        // level, where the tags were registered.
        //
        // These two calls deliberately name the raw bodies `_tags_impl` /
        // `_next_label_impl` rather than the dispatching `_tags` /
        // `_next_label` (unlike the `_description` calls in `_next_label` /
        // `_all_labels` / `_requested`). Dispatching replaces this hand-rolled
        // depth with `doshfunc`'s own `inc_locallevel`
        // (`src/ported/exec.rs:6131`) — arguably the faithful arrangement,
        // since `_tags` and `_next_label` are real shell functions in zsh —
        // but it also drops the level for everything AFTER the loop, and it
        // flips `dash_e_registers_its_own_tag_level`,
        // `default_mode_registers_the_messages_tag` and eleven downstream
        // `_x_*` `routes_to_message_*` tests. Which answer matches zsh needs
        // a live completion-context run of `_message` in the reference
        // shell; that was not obtained, so this is left as-is rather than
        // landed on a guess.
        crate::ported::utils::inc_locallevel();
        // sh:23 — in zsh `_tags` is a shell function, so its `FUNCSTACK`
        // frame sits between `_message` and whatever `_tags` calls. The raw
        // `_impl` call skips `doshfunc`, so that frame was absent, and every
        // consumer that reads the funcstack BY INDEX from inside a `_tags`
        // callee saw the window shifted one frame too deep. `_help_sort_tags`
        // (`Base/Widget/_complete_help` sh:80) is exactly such a consumer:
        // `${funcstack[3,(i)_($~_help_scan_funcstack)]}` expects [1]
        // `_help_sort_tags`, [2] `_tags`, [3] the innermost real caller — so
        // the missing frame dropped `_message` itself off the front of every
        // reported call chain. Measured on `git log --grep=<C-x h>` with zsh
        // 5.9.2 as reference:
        //
        //   zsh    option--grep-1  (_message _arguments _git-log _git _git)
        //   zshrs  option--grep-1  (          _arguments _git-log _git _git)
        //
        // and identically on `git tag -l `, `git stash branch `,
        // `git ls-tree --format=`, `git repack --window=`, `git --namespace=`.
        // Supplied on its own rather than by switching to the dispatching
        // `_tags`, so the hand-managed `locallevel` pairing above stays
        // intact — same arrangement as the `_next_label` frame below. The
        // frame is scoped to the `_tags` CALL, so it is gone again before the
        // `_next_label` loop and `_tags_level` (`_next_label` sh:10) is
        // unaffected.
        let tags_rc = {
            let _tags_frame = crate::compsys::ported::shared::PortFuncstackFrame::push("_tags");
            _tags_impl(&[tag.clone()])
        };
        if tags_rc == 0 {
            loop {
                let nl_args = vec![tag.clone(), "expl".to_string(), descr.clone()];
                // sh:23 — in zsh `_next_label` is a shell function, so this
                // call runs one `FUNCSTACK` frame deeper than `_message`. The
                // raw `_impl` call below skips `doshfunc`, and `_next_label`
                // sh:9 gates its `_comp_tags` strip on
                // `(( $#funcstack > _tags_level ))`: without the frame the
                // guard never fires and a tag zsh drops survives. Measured on
                // `_arguments : '1::optional delimiter:(\:)' '*:spec'` —
                // zsh `_tags_level=9 _comp_tags=' argument-rest '`, zshrs
                // `_tags_level=8 _comp_tags=' argument-1  argument-rest '`.
                // The frame is supplied on its own, NOT via `doshfunc`, so the
                // hand-managed `locallevel` pairing above stays intact.
                let nl_rc = {
                    let _nl_frame =
                        crate::compsys::ported::shared::PortFuncstackFrame::push("_next_label");
                    _next_label_impl(&nl_args)
                };
                if nl_rc != 0 {
                    break;
                }
                // sh:24  compadd ${expl:/-X/-x}
                //   `${expl:/-X/-x}` — replace first occurrence of
                //   `-X` with `-x` in the array. compadd then emits
                //   the message-as-explanation.
                let expl = getaparam("expl").unwrap_or_default();
                let compadd_argv: Vec<String> = expl
                    .iter()
                    .map(|s| {
                        if s == "-X" {
                            "-x".to_string()
                        } else {
                            s.clone()
                        }
                    })
                    .collect();
                let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);
                ret = 0;
            }
        }
        crate::ported::utils::dec_locallevel();

        // sh:28-29  if no matches AND compstate[insert] contains
        //   "unambiguous", clear compstate[insert].
        let nmatches: i64 = crate::ported::params::getsparam("compstate[nmatches]")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if nmatches == 0 {
            let insert = crate::ported::params::getsparam("compstate[insert]").unwrap_or_default();
            if insert.contains("unambiguous") {
                let _ = crate::ported::params::setsparam("compstate[insert]", "");
            }
        }

        // sh:31
        return ret;
    }

    // sh:34  _tags messages || return 1
    //
    // Same locallevel guard as the `-e` branch above: this registration
    // must NOT replace the caller's tag sets. Everything below runs at the
    // nested level, so each return path drops it again. Left direct for the
    // same reason as the `-e` branch — see the comment there.
    crate::ported::utils::inc_locallevel();
    // sh:34 — `_tags` is a shell function in zsh, so it owns a `FUNCSTACK`
    // frame; supply it here for the same reason as the `-e` branch above (a
    // funcstack-index consumer such as `_help_sort_tags` otherwise loses the
    // innermost caller off the front of the chain it reports).
    let tags_rc = {
        let _tags_frame = crate::compsys::ported::shared::PortFuncstackFrame::push("_tags");
        _tags_impl(&["messages".to_string()])
    };
    if tags_rc != 0 {
        crate::ported::utils::dec_locallevel();
        return 1;
    }

    // sh:36-41  format determination
    let (raw, format_seed): (bool, String) = if opth.iter().any(|k| k == "-r") {
        // sh:37 — `raw=yes format=$1`: the raw format is the first word
        // left after zparseopts -D.
        (true, argv.first().cloned().unwrap_or_default())
    } else {
        // sh:39-40  zstyle -s ":…:messages" format format ||
        //               zstyle -s ":…:descriptions" format format
        //
        // Same shape as `_description` sh:23-24: the `||` runs on the STATUS
        // (`zutil.c:648` tests `vals[0]`, a pointer), so `messages format ''`
        // is SET and stops the chain — a global `:descriptions` format must
        // NOT leak into messages that were deliberately silenced. And
        // `zutil.c:649` joins the whole value array, so a format written
        // unquoted keeps all of its words.
        let curcontext = getsparam("curcontext").unwrap_or_default();
        let ctx_msg = format!(":completion:{}:messages", curcontext);
        let f = match zstyle_s(&ctx_msg, "format") {
            Some(f) => f,
            None => {
                let ctx_desc = format!(":completion:{}:descriptions", curcontext);
                zstyle_s(&ctx_desc, "format").unwrap_or_default()
            }
        };
        (false, f)
    };

    // sh:43  if [[ -n "$format$raw" ]]
    let combined = format!("{}{}", format_seed, if raw { "y" } else { "" });
    if combined.is_empty() {
        crate::ported::utils::dec_locallevel();
        return 0;
    }

    // sh:44  in cooked mode, `zformat -Fq` into the `format` param.
    let format_final: String = if raw {
        format_seed
    } else {
        let descr = argv.first().cloned().unwrap_or_default();
        let mut zf_argv: Vec<String> = vec![
            "format".to_string(),
            format_seed.clone(),
            format!("d:{}", descr),
        ];
        if argv.len() > 1 {
            zf_argv.extend(argv[1..].iter().cloned());
        }
        let _ = setsparam("format", "");
        let mut zf_ops = make_ops();
        zf_ops.ind[b'F' as usize] = 1; // `-F` and `-q` are parsed flags (zutil.c:2151);
        zf_ops.ind[b'q' as usize] = 1; // `-q` doubles `%` in the specs (74fa234140)
        let _ = bin_zformat("zformat", &zf_argv, &zf_ops, 0);
        getsparam("format").unwrap_or_default()
    };

    // sh:45  builtin compadd "$gopt[@]" -x "$format"
    //   `builtin` bypasses the `compadd()` shell function
    //   `_approximate` / `_correct` install (and `_complete_help`'s
    //   `compadd() { return 1 }` at sh:_complete_help:13) so the
    //   message is emitted unconditionally.
    let mut compadd_argv: Vec<String> = gopt;
    compadd_argv.push("-x".to_string());
    compadd_argv.push(format_final);
    let _ = bin_compadd_body("compadd", &compadd_argv, &make_ops(), 0);

    // sh:46
    let _ = setsparam("_comp_mesg", "yes");

    crate::ported::utils::dec_locallevel();
    0
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
    fn dash_e_registers_its_own_tag_level() {
        // sh:23 — `_tags "$tag"` REGISTERS the tag at `_message`'s own
        // function-nesting level (comptags is indexed by locallevel), so it
        // succeeds even for a tag the caller never offered, and the
        // `_next_label` loop then adds the message: ret=0.
        //
        // Checked against the reference shell — a completer whose whole body
        // is `_message -e titles 'title'; print rc=$?` prints `rc=0` under
        // `zsh -f` with compinit loaded. This test previously asserted 1,
        // which was the signature of the missing inc_locallevel: `_tags`
        // clobbered the CALLER's registration and reported failure.
        let r = with_incompfunc(|| {
            _message_impl(&[
                "-e".to_string(),
                "unregistered_tag".to_string(),
                "descr".to_string(),
            ])
        });
        assert_eq!(r, 0);
    }

    /// sh:23 — `_next_label` is a shell function, so `_message -e` calls it
    /// one `FUNCSTACK` frame deeper than itself. `_next_label` sh:9 gates its
    /// `_comp_tags` strip on `(( $#funcstack > _tags_level ))`, so the frame
    /// decides whether a tag the caller published is dropped.
    ///
    /// Without it, `_arguments : '1::optional delimiter:(\:)' '*:spec'` — an
    /// empty rest action, which sh:413-417 routes here — left
    /// `_comp_tags=' argument-1  argument-rest '` where zsh leaves
    /// `' argument-rest '`, and `_tags_level` read 8 against zsh's 9.
    #[test]
    fn dash_e_runs_next_label_one_funcstack_frame_deeper() {
        fn depth() -> usize {
            crate::ported::modules::parameter::FUNCSTACK
                .lock()
                .map(|s| s.len())
                .unwrap_or(0)
        }

        // `with_incompfunc` takes `global_state_lock()` itself, and that guard
        // is a plain non-reentrant `Mutex` (test_util.rs:24-37): taking it here
        // too would deadlock this thread against itself. Everything therefore
        // runs inside the closure, as in every sibling test.
        let (before, after_call, inner, after_drop) = with_incompfunc(|| {
            let before = depth();
            let _ = _message_impl(&[
                "-e".to_string(),
                "some_tag".to_string(),
                "descr".to_string(),
            ]);
            let after_call = depth();

            // The guard itself: one FS_FUNC frame while it is alive.
            let inner = {
                let _f = crate::compsys::ported::shared::PortFuncstackFrame::push("_next_label");
                depth()
            };
            (before, after_call, inner, depth())
        });

        assert_eq!(
            after_call, before,
            "the frame must be popped again (c:6218-6219)"
        );
        assert_eq!(inner, before + 1, "c:6005-6016 — one FS_FUNC frame pushed");
        assert_eq!(after_drop, before, "c:6218-6219 — popped on drop");
    }

    #[test]
    fn sets_comp_mesg_in_dash_e_mode() {
        // sh:15 — `_comp_mesg=yes` is set unconditionally in -e mode.
        let _ = with_incompfunc(|| {
            let _ = setsparam("_comp_mesg", "");
            _message_impl(&["-e".to_string(), "tag".to_string(), "descr".to_string()])
        });
        assert_eq!(getsparam("_comp_mesg").as_deref(), Some("yes"));
    }

    #[test]
    fn default_mode_registers_the_messages_tag() {
        // sh:34 — `_tags messages` registers `messages` at _message's own
        // nesting level and succeeds, so the body runs to completion.
        // `zsh -f` + compinit: a completer body of `_message -r 'raw text';
        // print rc=$?` prints `rc=0`. (Asserted 1 before the missing
        // inc_locallevel around this `_tags` call was added.)
        let r = with_incompfunc(|| _message_impl(&["my message".to_string()]));
        assert_eq!(r, 0);
    }

    #[test]
    fn parses_group_options_into_gopt_and_the_rest_into_opth() {
        // sh:7-10 — `-1`/`-2` and `-V`/`-J` with their group name go to
        // gopt in order; opth keys every option seen (oracle:
        // `opth=(-1 -e -V -X)`); zparseopts -D leaves the description.
        let _g = crate::test_util::global_state_lock();
        let (rem, gopt, mut opth) = parse_message_opts(&[
            "-V".to_string(),
            "grp".to_string(),
            "-1".to_string(),
            "-e".to_string(),
            "-X".to_string(),
            "ignored".to_string(),
            "the message".to_string(),
        ])
        .expect("zparseopts accepts the spec");
        opth.sort();
        assert_eq!(gopt, vec!["-V", "grp", "-1"]);
        assert_eq!(opth, vec!["-1", "-V", "-X", "-e"]);
        assert_eq!(rem, vec!["the message"]);
    }

    #[test]
    fn an_unknown_option_fails_the_way_zparseopts_f_does() {
        // sh:7 `-F` + sh:10 `|| return`: an option outside the spec is an
        // error and its status is the function's.
        let _g = crate::test_util::global_state_lock();
        assert!(parse_message_opts(&["-Z".to_string(), "m".to_string()]).is_err());
    }
}
