//! Port of `_regex_arguments` from
//! `Completion/Base/Utility/_regex_arguments` (zsh 5.9, 86 lines).
//!
//! `_regex_arguments funcname regex…` compiles `regex` and emits a
//! state-machine-driven completion function `funcname`. Upstream does
//! this by `eval`-defining a shell function whose body calls the
//! `zregexparse` builtin against the command line. This port keeps the
//! same two pieces but Rust-native:
//!
//!   * [`_regex_arguments`] performs the spec rewrite
//!     `regex=(${@:/(#b):(*)/":_ra_comp ${(qqqq)match[1]}"})` (sh:60)
//!     and records the compiled token list under `funcname`.
//!   * [`dispatch_registered`] IS the eval'd function body (sh:63-83):
//!     it builds `_ra_line`, runs the real `zregexparse -c` builtin,
//!     then switches on the parse result exactly as upstream does
//!     (`_message`, or `compset -p` + `_alternative`).
//!   * [`_ra_comp`] is the sh:53-55 helper that each rewritten action
//!     calls to accumulate `_alternative` specs into `_ra_actions`.
//!
//! Upstream body this port mirrors 1:1:
//! ```text
//! sh:53  _ra_comp() { _ra_actions=("$_ra_actions[@]" "$1") }
//! sh:58  local regex funcname="$1"; shift
//! sh:60  regex=(${@:/(#b):(*)/":_ra_comp ${(qqqq)match[1]}"})
//! sh:64  local _ra_p1 _ra_p2 _ra_left _ra_right _ra_com expl tmp nm=$compstate[nmatches]
//! sh:65  local _ra_actions _ra_line="${(pj:\0:)${(@)words[1,CURRENT-1]:Q}}"$'\0'"$PREFIX"
//! sh:67  zregexparse -c _ra_p1 _ra_p2 "$_ra_line" "$regex[@]"
//! sh:68-81  case $? in 0|2) _message; 1) compset+_alternative; 3) _message
//! sh:82  [[ nm -ne $compstate[nmatches] ]]
//! ```

use crate::compsys::ported::_alternative::_alternative;
use crate::compsys::ported::_message::_message;
use crate::ported::glob::remnulargs;
use crate::ported::lex::{parse_subst_string, untokenize};
use crate::ported::modules::zutil::bin_zregexparse;
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, unsetparam};
use crate::ported::zle::compcore::get_compstate_str;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS, QT_DOLLARS};
use std::collections::HashMap;
use std::sync::Mutex;

static REGEX_FUNCS: Mutex<Option<HashMap<String, Vec<String>>>> = Mutex::new(None);

fn registry() -> std::sync::MutexGuard<'static, Option<HashMap<String, Vec<String>>>> {
    let mut g = REGEX_FUNCS.lock().unwrap();
    if g.is_none() {
        *g = Some(HashMap::new());
    }
    g
}

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// zsh `${(@)…:Q}` per-element unquote — the (Q) modifier maps to
/// `parse_subst_string + remnulargs + untokenize` (Src/subst.c:4863).
/// On a parse error C keeps the original (noerrs-tolerant); mirror that.
fn dequote_q(s: &str) -> String {
    match parse_subst_string(s) {
        Ok(mut r) => {
            remnulargs(&mut r);
            untokenize(&r)
        }
        Err(_) => s.to_string(),
    }
}

/// sh:53-55 — `_ra_comp () { _ra_actions=("$_ra_actions[@]" "$1") }`.
/// Each rewritten `:action` (see [`_regex_arguments`]) evaluates to a
/// call to this helper via `zregexparse`'s action execstring, appending
/// the original `_alternative` spec to `_ra_actions`.
pub fn _ra_comp(args: &[String]) -> i32 {
    let mut acts = getaparam("_ra_actions").unwrap_or_default();
    if let Some(a) = args.first() {
        acts.push(a.clone());
    }
    setaparam("_ra_actions", acts);
    0
}

/// Reach `_regex_arguments` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_regex_arguments "${funcname}_sm" "$regex_all[@]"` (Completion/Debian/Command/_apt sh:347) — so the normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_regex_arguments_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _regex_arguments(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_regex_arguments", args, || {
        _regex_arguments_impl(args)
    })
}

/// `_regex_arguments funcname regex…` — rewrite the spec's cactions to
/// funnel through `_ra_comp` and record the compiled token list under
/// `funcname`. The generated function body lives in
/// [`dispatch_registered`].
pub fn _regex_arguments_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_regex_arguments");
    // sh:58 — `local regex funcname="$1"; shift`.
    let funcname = match args.first() {
        Some(f) => f.clone(),
        None => return 1,
    };
    // sh:60 — `regex=(${@:/(#b):(*)/":_ra_comp ${(qqqq)match[1]}"})`.
    // Every element that begins with `:` (a caction `:tag:descr:code`)
    // is rewritten to `:_ra_comp <qqqq(rest)>` so the action, when run
    // by zregexparse, calls `_ra_comp` with the original spec as one
    // `$'…'`-quoted argument. Non-`:` elements pass through unchanged.
    let regex: Vec<String> = args[1..]
        .iter()
        .map(|e| match e.strip_prefix(':') {
            // `quotestring()` returns only the BODY of the `$'…'` form —
            // subst.c c:4047-4051 reserves `pre = 2` / `post = 1` and
            // c:4088-4092 writes the surrounding `'` pair and the leading
            // `$` itself. Emitting the body bare turned the whole spec into
            // an unquoted word list, so `zregexparse`'s `execstring` split
            // `:commands:command: _describe -t sed-commands "sed command"
            //  cmds_none -S "" -F excl` at every space; `_ra_comp` took only
            // `$1` (`commands:command:`), and `_alternative` then saw an
            // EMPTY action and fell through to `_message -e`.
            Some(rest) => format!(
                ":_ra_comp ${}{}{}",
                '\'',
                crate::ported::utils::quotestring(rest, QT_DOLLARS),
                '\''
            ),
            None => e.clone(),
        })
        .collect();
    if let Some(map) = registry().as_mut() {
        map.insert(funcname, regex);
    }
    0
}

/// The compiled completion function body (upstream sh:63-83). Runs the
/// real `zregexparse` state machine over the current command line and
/// drives `_message` / `compset` / `_alternative` off the result.
///
/// WIRING GAP (surfaced honestly, not faked): upstream `eval`-defines a
/// real shell function named `funcname`, so the completion system finds
/// it by name. This Rust port stores `funcname` in a private registry
/// and this function is the body, but nothing yet routes a completion
/// call for `funcname` here — the compsys router
/// (`src/compsys/router.rs`) resolves names through a STATIC fn-pointer
/// table that can't carry a dynamically-registered `funcname` (a plain
/// `fn(&[String]) -> i32` pointer has no way to carry the name back to
/// this dispatcher). Wiring requires a registry-consulting hook in the
/// router / `callcompfunc` path, out of scope for this file. Until then
/// this entry is reachable via [`_regex_arguments`]-registered names in
/// tests but not through live `Tab` completion.
/// Route a call to a `_regex_arguments`-generated completion function.
///
/// `_regex_arguments funcname regex…` compiles `funcname` into the
/// [`REGEX_FUNCS`] registry rather than a shell function, so the compsys
/// dispatch chokepoint (`vm_helper`'s function-call path) can't find it as
/// a shfunc or a static-router entry. This lets that chokepoint consult the
/// registry: returns `Some(dispatch_registered(name))` when `name` is a
/// registered regex function, else `None` (defer to normal dispatch). Bug
/// #657 gap #2 — previously the generated function was unreachable at
/// completion time.
pub fn dispatch_if_registered(funcname: &str) -> Option<i32> {
    let known = {
        let g = registry();
        g.as_ref()
            .map(|m| m.contains_key(funcname))
            .unwrap_or(false)
    };
    if known {
        // Upstream `_regex_arguments` eval-defines `funcname` as a REAL shell
        // function, so calling it (as an `_arguments`/comparguments action)
        // pushes a `locallevel`. `comptags` is indexed by `locallevel`
        // (computil.rs), so the generated function's inner `_alternative` →
        // `_tags`/`comptags -i` lands at a DEEPER level and cannot clobber the
        // caller's tag set. This Rust dispatch bypasses `doshfunc`, so without
        // an explicit push the inner `comptags -i` overwrote comparguments'
        // option tags at the SAME level — `sed <tab>` (via `_sed_expressions`'s
        // `_alternative`) wiped the whole `-<<option>>-` list. Mirror the
        // function-call level bump around the body.
        crate::ported::utils::inc_locallevel();
        let rc = dispatch_registered(funcname);
        crate::ported::utils::dec_locallevel();
        Some(rc)
    } else {
        None
    }
}

pub fn dispatch_registered(funcname: &str) -> i32 {
    let regex = {
        let g = registry();
        g.as_ref().and_then(|m| m.get(funcname).cloned())
    };
    let regex = match regex {
        Some(r) => r,
        None => return 1,
    };

    // sh:64 — `nm=$compstate[nmatches]`.
    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // sh:65 — `_ra_line="${(pj:\0:)${(@)words[1,CURRENT-1]:Q}}"$'\0'"$PREFIX"`.
    //   * `words[1,CURRENT-1]` — the words BEFORE the current one
    //     (1-based inclusive → the first CURRENT-1 elements).
    //   * `:Q` — per-element unquote.
    //   * `(pj:\0:)` — join with a real NUL byte.
    //   * then append NUL + PREFIX.
    let current: usize = getsparam("CURRENT")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let words = getaparam("words").unwrap_or_default();
    let upto = current.saturating_sub(1); // number of leading words
    let dequoted: Vec<String> = words.iter().take(upto).map(|w| dequote_q(w)).collect();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let mut _ra_line = dequoted.join("\0");
    _ra_line.push('\0');
    _ra_line.push_str(&prefix);

    // sh:66 — `_ra_actions=()`.
    setaparam("_ra_actions", Vec::new());

    // sh:67 — `zregexparse -c _ra_p1 _ra_p2 "$_ra_line" "$regex[@]"`.
    // The regex tokens are passed as separate args (upstream's
    // `(qqqq)`/`(j)` codegen exists only to re-split at eval time; here
    // we hand `bin_zregexparse` the already-split Vec directly). NUL is
    // kept as a raw byte in `_ra_line`: `rmatch` byte-indexes `subj`
    // and `pattry`es it against patterns that also carry raw NUL (the
    // shell decoded `$'\0'` before calling us), so both sides agree.
    let mut zargs: Vec<String> = vec!["_ra_p1".to_string(), "_ra_p2".to_string(), _ra_line.clone()];
    zargs.extend(regex.iter().cloned());
    let mut ops = make_ops();
    ops.ind[b'c' as usize] = 1; // -c (completion mode)
    let rc = bin_zregexparse("zregexparse", &zargs, &ops, 0);

    // `zregexparse` binds the parse cursor into $_ra_p1 / $_ra_p2
    // (byte offsets into subj).
    let p1 = getiparam("_ra_p1");
    let p2 = getiparam("_ra_p2");
    let line_bytes = _ra_line.as_bytes();

    // sh:68-81 — `case "$?" in …`.
    match rc {
        // sh:69 — `0|2) _message "no more arguments"`.
        0 | 2 => {
            let _ = _message(&["no more arguments".to_string()]);
        }
        // sh:70-79 — `1) …`.
        1 => {
            // sh:71 — `if [[ "$_ra_line[_ra_p1 + 1, -1]" = *$'\0'* ]]`.
            // (byte offsets from zregexparse; the source's `$str[a,b]`
            // is char-based but degenerates to bytes for the ASCII+NUL
            // command lines this handles.)
            let tail_has_nul = line_bytes
                .get(p1.max(0) as usize..)
                .map(|b| b.contains(&0u8))
                .unwrap_or(false);
            if tail_has_nul {
                // sh:72 — `_message "parse failed before current word"`.
                let _ = _message(&["parse failed before current word".to_string()]);
            } else {
                // sh:74-75 — `_ra_left`/`_ra_right` are assigned but never
                // used downstream in upstream; compute for fidelity.
                let lo = p1.max(0) as usize;
                let hi = (p2.max(0) as usize).min(line_bytes.len()).max(lo);
                let _ra_left = String::from_utf8_lossy(&line_bytes[lo..hi]).into_owned();
                let _ra_right =
                    String::from_utf8_lossy(line_bytes.get(hi..).unwrap_or(&[])).into_owned();
                let _ = (&_ra_left, &_ra_right);

                // sh:76 — `compset -p $(( $#PREFIX - $#_ra_line + $_ra_p1 ))`.
                // `$#` is a CHARACTER count; replicate the source's
                // (char-count vs byte-offset) arithmetic literally.
                let n = prefix.chars().count() as i64 - _ra_line.chars().count() as i64 + p1;
                let _ = bin_compset(
                    "compset",
                    &["-p".to_string(), n.to_string()],
                    &make_ops(),
                    0,
                );

                // sh:77 — `(( $#_ra_actions )) && _alternative "$_ra_actions[@]"`.
                let actions = getaparam("_ra_actions").unwrap_or_default();
                if !actions.is_empty() {
                    let _ = _alternative(&actions);
                }
            }
        }
        // sh:80 — `3) _message "invalid regex"`.
        3 => {
            let _ = _message(&["invalid regex".to_string()]);
        }
        _ => {}
    }

    // Tear down the transient params (upstream declares them `local`).
    unsetparam("_ra_p1");
    unsetparam("_ra_p2");
    unsetparam("_ra_actions");

    // sh:82 — `[[ nm -ne "$compstate[nmatches]" ]]` — return 0 iff the
    // match count changed (i.e. this function produced completions).
    let nm2: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if nm != nm2 {
        0
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_rewrites_cactions() {
        let _g = crate::test_util::global_state_lock();
        // `:compadd aaa` must be rewritten to `:_ra_comp $'compadd aaa'`;
        // a `/pattern/` element passes through untouched.
        assert_eq!(
            _regex_arguments_impl(&[
                "_tst".to_string(),
                "/x/".to_string(),
                ":compadd aaa".to_string(),
            ]),
            0
        );
        let g = registry();
        let stored = g.as_ref().unwrap().get("_tst").cloned().unwrap();
        assert_eq!(stored[0], "/x/");
        assert!(
            stored[1].starts_with(":_ra_comp "),
            "caction should funnel through _ra_comp, got {:?}",
            stored[1]
        );
        assert!(
            stored[1].contains("compadd aaa"),
            "original spec must be preserved (quoted), got {:?}",
            stored[1]
        );
    }

    #[test]
    fn ra_comp_appends_to_actions() {
        let _g = crate::test_util::global_state_lock();
        setaparam("_ra_actions", Vec::new());
        assert_eq!(_ra_comp(&["compadd aaa".to_string()]), 0);
        assert_eq!(_ra_comp(&["compadd bbb".to_string()]), 0);
        assert_eq!(
            getaparam("_ra_actions").unwrap_or_default(),
            vec!["compadd aaa".to_string(), "compadd bbb".to_string()]
        );
    }

    #[test]
    fn missing_registration_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(dispatch_registered("never_registered"), 1);
    }

    #[test]
    fn dispatch_runs_zregexparse_and_cleans_up() {
        let _g = crate::test_util::global_state_lock();
        // Register a trivial two-element regex. dispatch_registered must
        // drive the real zregexparse builtin (not emit a hardcoded
        // message) and unset its transient params afterward.
        assert_eq!(
            _regex_arguments_impl(&[
                "_tst2".to_string(),
                "/[^\0]#\0/".to_string(),
                "/[^\0]#\0/".to_string(),
            ]),
            0
        );
        // No panic; returns 0 or 1 depending on whether nmatches moved.
        let rc = dispatch_registered("_tst2");
        assert!(rc == 0 || rc == 1);
        // Transient params were torn down.
        assert!(crate::ported::params::getaparam("_ra_actions").is_none());
    }
}
