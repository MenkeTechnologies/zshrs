//! Port of `_message` from `Completion/Base/Core/_message`.
//!
//! Full upstream body (39 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local format raw gopt
//! sh: 4
//! sh: 5  if [[ "$1" = -e ]]; then
//! sh: 6    local expl ret=1 tag
//! sh: 7
//! sh: 8    _comp_mesg=yes
//! sh: 9
//! sh:10    if (( $# > 2 )); then
//! sh:11      tag="$2"
//! sh:12      shift
//! sh:13    else
//! sh:14      tag="$curtag"
//! sh:15    fi
//! sh:16    _tags "$tag" && while _next_label "$tag" expl "$2"; do
//! sh:17      compadd ${expl:/-X/-x}
//! sh:18      ret=0
//! sh:19    done
//! sh:20
//! sh:21    (( ! $compstate[nmatches] )) && [[ $compstate[insert] = *unambiguous* ]] &&
//! sh:22        compstate[insert]=
//! sh:23
//! sh:24    return ret
//! sh:25  fi
//! sh:26
//! sh:27  gopt=()
//! sh:28  zparseopts -D -a gopt 1 2 V J
//! sh:29
//! sh:30  _tags messages || return 1
//! sh:31
//! sh:32  if [[ "$1" = -r ]]; then
//! sh:33    raw=yes
//! sh:34    shift
//! sh:35    format="$1"
//! sh:36  else
//! sh:37    zstyle -s ":completion:${curcontext}:messages" format format ||
//! sh:38        zstyle -s ":completion:${curcontext}:descriptions" format format
//! sh:39  fi
//! sh:40
//! sh:41  if [[ -n "$format$raw" ]]; then
//! sh:42    [[ -z "$raw" ]] && zformat -F format "$format" "d:$1" "${(@)argv[2,-1]}"
//! sh:43    builtin compadd "$gopt[@]" -x "$format"
//! sh:44    _comp_mesg=yes
//! sh:45  fi
//! ```
//!
//! Calls real `bin_compadd`, `bin_zparseopts`, `bin_zformat`,
//! `lookupstyle`. Cross-fn calls (`_tags`, `_next_label`) go through
//! sibling ports. Reads/writes `$compstate[insert]`/`[nmatches]`
//! through `get_compstate_str`/`set_compstate_str`.

use super::_next_label::_next_label_impl;
use super::_tags::_tags_impl;
use crate::ported::modules::zutil::{bin_zformat, bin_zparseopts, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
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

/// sh:28 — bridge to real `bin_zparseopts -D -a gopt 1 2 V J` via
/// `-v <name>`.
fn run_gopt_message(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    crate::compsys::ported::shared::set_bridge_argv(src, args);
    setaparam("gopt", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "gopt".to_string(),
            "1".to_string(),
            "2".to_string(),
            "V".to_string(),
            "J".to_string(),
        ],
        &make_ops(),
        0,
    );
    let gopt = getaparam("gopt").unwrap_or_default();
    let remaining = getaparam(src).unwrap_or_default();
    // Tear down `__compsys_argv` — the zparseopts-bridge scratch array, not a
    // real zsh identifier (zsh operates on positional $argv). It is declared
    // FUNCTION-LOCAL by `shared::set_bridge_argv`; this unset is what clears it
    // when the port runs outside any function scope. Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, gopt)
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
    // sh:5  -e mode
    if args.first().map(|s| s == "-e").unwrap_or(false) {
        // sh:6
        let mut ret: i32 = 1;
        // sh:8
        let _ = setsparam("_comp_mesg", "yes");

        // sh:10-15 — `$2` becomes the tag when >2 args, else
        //   inherit from $curtag. `$#` counts `-e` as $1, so with
        //   `args` including "-e" at index 0 the predicate is
        //   `args.len() > 2` (matching zsh `(( $# > 2 ))`).
        let (tag, descr): (String, String) = if args.len() > 2 {
            // shift drops original $1, so new $2 is original $3
            (
                args.get(1).cloned().unwrap_or_default(),
                args.get(2).cloned().unwrap_or_default(),
            )
        } else {
            (
                getsparam("curtag").unwrap_or_default(),
                args.get(1).cloned().unwrap_or_default(),
            )
        };

        // sh:16  _tags "$tag" && while _next_label "$tag" expl "$2"
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
        let tags_rc = _tags_impl(&[tag.clone()]);
        if tags_rc == 0 {
            loop {
                let nl_args = vec![tag.clone(), "expl".to_string(), descr.clone()];
                if _next_label_impl(&nl_args) != 0 {
                    break;
                }
                // sh:17  compadd ${expl:/-X/-x}
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

        // sh:21-22  if no matches AND compstate[insert] contains
        //   "unambiguous", clear compstate[insert].
        let nmatches: i64 = get_compstate_str("nmatches")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        if nmatches == 0 {
            let insert = get_compstate_str("insert").unwrap_or_default();
            if insert.contains("unambiguous") {
                set_compstate_str("insert", "");
            }
        }

        // sh:24
        return ret;
    }

    // sh:27-28
    let (mut argv, gopt) = run_gopt_message(args);

    // sh:30  _tags messages || return 1
    //
    // Same locallevel guard as the `-e` branch above: this registration
    // must NOT replace the caller's tag sets. Everything below runs at the
    // nested level, so each return path drops it again. Left direct for the
    // same reason as the `-e` branch — see the comment there.
    crate::ported::utils::inc_locallevel();
    if _tags_impl(&["messages".to_string()]) != 0 {
        crate::ported::utils::dec_locallevel();
        return 1;
    }

    // sh:32-39  format determination
    let (raw, format_seed): (bool, String) = if argv.first().map(|s| s == "-r").unwrap_or(false) {
        // sh:32-35 — raw mode: $2 is the literal format
        argv.remove(0); // drop "-r"
        let f = if argv.is_empty() {
            String::new()
        } else {
            argv.remove(0)
        };
        (true, f)
    } else {
        // sh:37-38
        let curcontext = getsparam("curcontext").unwrap_or_default();
        let ctx_msg = format!(":completion:{}:messages", curcontext);
        let mut f = lookupstyle(&ctx_msg, "format")
            .first()
            .cloned()
            .unwrap_or_default();
        if f.is_empty() {
            let ctx_desc = format!(":completion:{}:descriptions", curcontext);
            f = lookupstyle(&ctx_desc, "format")
                .first()
                .cloned()
                .unwrap_or_default();
        }
        (false, f)
    };

    // sh:41  if [[ -n "$format$raw" ]]
    let combined = format!("{}{}", format_seed, if raw { "y" } else { "" });
    if combined.is_empty() {
        crate::ported::utils::dec_locallevel();
        return 0;
    }

    // sh:42  in cooked mode, run zformat -F into `format` param.
    let format_final: String = if raw {
        format_seed
    } else {
        let descr = argv.first().cloned().unwrap_or_default();
        let mut zf_argv: Vec<String> = vec![
            "-F".to_string(),
            "format".to_string(),
            format_seed.clone(),
            format!("d:{}", descr),
        ];
        if argv.len() > 1 {
            zf_argv.extend(argv[1..].iter().cloned());
        }
        let _ = setsparam("format", "");
        let _ = bin_zformat("zformat", &zf_argv, &make_ops(), 0);
        getsparam("format").unwrap_or_default()
    };

    // sh:43  builtin compadd "$gopt[@]" -x "$format"
    //   `builtin` bypasses the `compadd()` shell function
    //   `_approximate` / `_correct` install (and `_complete_help`'s
    //   `compadd() { return 1 }` at sh:_complete_help:13) so the
    //   message is emitted unconditionally.
    let mut compadd_argv: Vec<String> = gopt;
    compadd_argv.push("-x".to_string());
    compadd_argv.push(format_final);
    let _ = bin_compadd_body("compadd", &compadd_argv, &make_ops(), 0);

    // sh:44
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
        // sh:16 — `_tags "$tag"` REGISTERS the tag at `_message`'s own
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

    #[test]
    fn sets_comp_mesg_in_dash_e_mode() {
        // sh:8 — `_comp_mesg=yes` is set unconditionally in -e mode.
        let _ = with_incompfunc(|| {
            let _ = setsparam("_comp_mesg", "");
            _message_impl(&["-e".to_string(), "tag".to_string(), "descr".to_string()])
        });
        assert_eq!(getsparam("_comp_mesg").as_deref(), Some("yes"));
    }

    #[test]
    fn default_mode_registers_the_messages_tag() {
        // sh:30 — `_tags messages` registers `messages` at _message's own
        // nesting level and succeeds, so the body runs to completion.
        // `zsh -f` + compinit: a completer body of `_message -r 'raw text';
        // print rc=$?` prints `rc=0`. (Asserted 1 before the missing
        // inc_locallevel around this `_tags` call was added.)
        let r = with_incompfunc(|| _message_impl(&["my message".to_string()]));
        assert_eq!(r, 0);
    }

    #[test]
    fn parses_gopt_via_zparseopts() {
        // sh:28 — `1 2 V J` (no x flag here, unlike other cluster fns).
        let _g = crate::test_util::global_state_lock();
        let (rem, gopt) = run_gopt_message(&[
            "-V".to_string(),
            "-1".to_string(),
            "the message".to_string(),
        ]);
        assert_eq!(gopt, vec!["-V", "-1"]);
        assert_eq!(rem, vec!["the message"]);
    }
}
