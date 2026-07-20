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

use super::_next_label::_next_label;
use super::_tags::_tags;
use crate::ported::modules::zutil::{bin_zformat, bin_zparseopts, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
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

/// sh:28 — bridge to real `bin_zparseopts -D -a gopt 1 2 V J` via
/// `-v <name>`.
fn run_gopt_message(args: &[String]) -> (Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
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
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src);
    (remaining, gopt)
}

/// `_message` — render a static message into the current completion
/// listing. Two modes:
///   * `-e <tag>? <description>` — emit per-spec messages via
///     `_next_label` loop (sh:5-25).
///   * default — pull message format from `messages` zstyle and emit
///     via `compadd -x` (sh:27-45).
pub fn _message(args: &[String]) -> i32 {
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
        if _tags(&[tag.clone()]) == 0 {
            loop {
                let nl_args = vec![tag.clone(), "expl".to_string(), descr.clone()];
                if _next_label(&nl_args) != 0 {
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
    if _tags(&["messages".to_string()]) != 0 {
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
    let mut compadd_argv: Vec<String> = gopt;
    compadd_argv.push("-x".to_string());
    compadd_argv.push(format_final);
    let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);

    // sh:44
    let _ = setsparam("_comp_mesg", "yes");

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
    fn dash_e_with_no_specs_returns_one() {
        // sh:24 — -e mode initial ret=1; never flipped to 0 when
        //   _next_label produces no matches.
        let r = with_incompfunc(|| {
            _message(&[
                "-e".to_string(),
                "unregistered_tag".to_string(),
                "descr".to_string(),
            ])
        });
        assert_eq!(r, 1);
    }

    #[test]
    fn sets_comp_mesg_in_dash_e_mode() {
        // sh:8 — `_comp_mesg=yes` is set unconditionally in -e mode.
        let _ = with_incompfunc(|| {
            let _ = setsparam("_comp_mesg", "");
            _message(&["-e".to_string(), "tag".to_string(), "descr".to_string()])
        });
        assert_eq!(getsparam("_comp_mesg").as_deref(), Some("yes"));
    }

    #[test]
    fn default_mode_requires_messages_tag() {
        // sh:30 — `_tags messages || return 1`. Without comptags
        //   pre-registered, _tags returns 1 → _message returns 1.
        let r = with_incompfunc(|| _message(&["my message".to_string()]));
        assert_eq!(r, 1);
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
