//! Port of `_describe` from `Completion/Base/Utility/_describe`.
//!
//! Faithful translation of the zsh shell function. Unlike the earlier
//! reimplementation (which hand-built display lines and invented a
//! `_describe_disp` array), this drives the C builtin `compdescribe`
//! exactly as the shell source does: `compdescribe -i`/`-I` initialises
//! the parsed state, then a `while compdescribe -g ...` loop pulls each
//! group's compadd-options/matches/displays out into named params which
//! are fed straight to `compadd`.
//!
//! Line numbers below are the CURRENT upstream file (identical in zsh 5.9.2
//! and master @ 599af4604f). They are load-bearing, not decoration: the
//! `compdescribe` / `compadd` call sites publish them through
//! [`crate::compsys::ported::shared::set_sh_lineno`] so a diagnostic reads
//! `_describe:compdescribe:129: no parsed state` exactly as C does. Re-read
//! the upstream file before changing one — the earlier numbers in this block
//! had drifted up to 6 lines behind the shipped `_describe`.
//!
//! ```text
//! sh:  1  #autoload
//! sh: 21  while getopts "oOt:12JVx" _opt; do …           (flag parse)
//! sh: 36  shift $(( OPTIND - 1 ))
//! sh: 39  [[ "$_type$_noprefix" = options && ! -prefix [-+]* ]] &&
//! sh: 40      zstyle -T … options prefix-needed && return 1
//! sh: 45  zstyle -T … verbose && _showd=yes
//! sh: 47  zstyle -s … list-separator _sep || _sep=--
//! sh: 48  zstyle -s … max-matches-width _mlen || _mlen=$((COLUMNS/2))
//! sh: 51  _descr="$1"; shift
//! sh: 54  if _showd && zstyle -T … list-grouped; then _oargv=("$@"); _grp=(-g)
//! sh: 62  [[ options ]] && zstyle -t … prefix-hidden && _hide=${(M)PREFIX##(--|[-+])}
//! sh: 66  _tags "$_type"
//! sh: 67  while _tags; do
//! sh: 68    while _next_label $_jvx12 "$_type" _expl "$_descr"; do
//! sh: 70      if (( $#_grp )); then … grouped -D/-O pre-pass … fi
//! sh: 111       compadd … -D $_strs -O $_mats - …            (pre-pass, with -O)
//! sh: 114       compadd … -D $_strs - …                      (pre-pass, no -O)
//! sh: 121     if _showd; compdescribe -I "$_hide" "$_mlen" "$_sep " _expl "$_grp[@]" "$@"
//! sh: 122        (the compdescribe -I call itself)
//! sh: 124     else       compdescribe -i "$_hide" "$_mlen" "$@"
//! sh: 127     compstate[list]="$csl"
//! sh: 129     while compdescribe -g csl2 _args _tmpm _tmpd; do
//! sh: 131       compstate[list]="$csl $csl2"
//! sh: 132       [[ -n "$csl2" ]] && compstate[list]="${compstate[list]:s/rows//}"
//! sh: 134       compadd "$_args[@]" -d _tmpd -a _tmpm && _ret=0
//! sh: 135     done
//! sh: 136   done
//! sh: 137   (( _ret )) || return 0
//! sh: 138 done
//! sh: 140 return 1
//! ```

use crate::compsys::ported::_next_label::_next_label;
use crate::compsys::ported::_tags::_tags;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getiparam, getsparam, setaparam, unsetparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zle::computil::bin_compdescribe;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// zsh boolean-style truthiness: first value ∈ {true,yes,on,1}.
fn style_is_true(vals: &[String]) -> bool {
    matches!(
        vals.first().map(String::as_str),
        Some("true") | Some("yes") | Some("on") | Some("1")
    )
}

/// `zstyle -T ctx style` — true when the style is unset, or set to a
/// true value (default-true).
fn zstyle_t_default_true(ctx: &str, style: &str) -> bool {
    let v = lookupstyle(ctx, style);
    if v.is_empty() {
        true
    } else {
        style_is_true(&v)
    }
}

/// `zstyle -t ctx style` — true only when the style is set to a true
/// value (default-false).
fn zstyle_t_default_false(ctx: &str, style: &str) -> bool {
    let v = lookupstyle(ctx, style);
    !v.is_empty() && style_is_true(&v)
}

/// `zstyle -s ctx style name [sep]` — Src/Modules/zutil.c:643-658:
///   `if ((vals = lookupstyle(args[1], args[2])) && vals[0]) {`
///   `    ret = sepjoin(vals, (args[4] ? args[4] : " "), 0); val = 0; }`
///   `else { ret = ztrdup(""); val = 1; }`
/// ALL values are joined with `sep` (default a single space) — returning
/// only the first value silently truncated multi-word styles. `None` is
/// the `val = 1` (style unset) arm; a style set to one empty string is
/// still a hit in C (`vals[0]` is a valid pointer) → `Some("")`.
/// No `_describe` call site passes the optional `sep`, so it is fixed at
/// the C default `" "`.
fn zstyle_s(ctx: &str, style: &str) -> Option<String> {
    let vals = lookupstyle(ctx, style);
    if vals.is_empty() {
        None
    } else {
        Some(crate::ported::utils::sepjoin(&vals, Some(" ")))
    }
}

/// Extract the value ("match") half of a `value:description` entry and
/// unescape it — the Rust equivalent of the zsh expansion
/// `${(@)${(@M)${(@P)ARR}##([^:\\]|\\?)##}//\\(#b)(?)/$match[1]}`:
/// take the leading run of unescaped chars up to the first unescaped
/// `:`, treating `\X` as the single char `X`.
fn extract_match_parts(arr: &[String]) -> Vec<String> {
    arr.iter()
        .map(|e| {
            let b = e.as_bytes();
            let mut out = Vec::<u8>::with_capacity(b.len());
            let mut i = 0usize;
            while i < b.len() {
                if b[i] == b'\\' && i + 1 < b.len() {
                    out.push(b[i + 1]); // \X -> X
                    i += 2;
                } else if b[i] == b':' {
                    break;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            String::from_utf8(out).unwrap_or_default()
        })
        .collect()
}

/// Resolve one grouped-pre-pass argument to array values: either a
/// literal `(a b c)` list or the contents of the named array param.
/// Literal splitting is a simplified approximation of zsh word-
/// splitting (whitespace only, no quote handling) — grouped `_describe`
/// callers pass array *names* in practice, not inline literals.
fn resolve_array_arg(arg: &str) -> Vec<String> {
    if arg.starts_with('(') && arg.ends_with(')') && arg.len() >= 2 {
        arg[1..arg.len() - 1]
            .split_whitespace()
            .map(str::to_string)
            .collect()
    } else {
        getaparam(arg).unwrap_or_default()
    }
}

/// Reach `_describe` as a BARE COMMAND WORD, the way every upstream caller
/// writes it — `_describe \` (Completion/Unix/Command/_7zip sh:132) — so the
/// normal function lookup runs.
///
/// This is the DEFAULT entry point for the port, and the one a sibling port
/// should call. It goes through
/// [`crate::compsys::ported::shared::call_compfn`], which supplies both of
/// the things a bare Rust call to the body would skip: `$fpath` / shfunc
/// arbitration (the user's own copy of the function wins instead of being
/// inert) and the `doshfunc` frame (a `FUNCSTACK` entry, and the callee's
/// `declare_locals` landing in its OWN param scope rather than the caller's).
///
/// [`_describe_impl`] is the raw body, reserved for the two callers that must not
/// re-enter dispatch: this wrapper's own fallback (it runs only when neither
/// a shell function nor a registered port claims the name — i.e. unit tests
/// with no executor installed), and the `compsys::router` arm, which has to
/// target the body or dispatch would re-enter this wrapper forever.
pub fn _describe(args: &[String]) -> i32 {
    crate::compsys::ported::shared::call_compfn("_describe", args, || _describe_impl(args))
}

/// `_describe` — add options or values with descriptions as matches.
/// Flags: `-o` options-mode, `-O` options-mode with no prefix test,
/// `-t TAG`, `-1/-2/-J/-V/-x` forwarded to `_next_label`.
///
/// Signature preserved for callers (`_arguments`, `_alternative`).
pub fn _describe_impl(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_describe");
    // sh:12-17 — `local _opt _expl _tmpm _tmpd _mlen _noprefix`,
    // `local _type=values _descr _ret=1 _showd _nm _hide _args _grp
    // _sep`, `local csl=… csl2`, `local _oargv _argv _new _strs _mats
    // _opts _i _try=0`, `local OPTIND OPTARG`, `local -a _jvx12`.
    {
        use crate::compsys::ported::shared::{declare_locals, PM_ARRAY};
        declare_locals(
            &[
                "_opt",
                "_expl",
                "_tmpm",
                "_tmpd",
                "_mlen",
                "_noprefix",
                "_type",
                "_descr",
                "_ret",
                "_showd",
                "_nm",
                "_hide",
                "_args",
                "_grp",
                "_sep",
                "csl",
                "csl2",
                "_oargv",
                "_argv",
                "_new",
                "_strs",
                "_mats",
                "_opts",
                "_i",
                "_try",
                "OPTIND",
                "OPTARG",
            ],
            0,
        );
        declare_locals(&["_jvx12"], PM_ARRAY);
    }
    // sh:5-16 — locals.
    let mut _type = "values".to_string();
    let mut _noprefix = false;
    let mut _ret: i32 = 1;
    let mut _hide = String::new();
    let mut _jvx12: Vec<String> = Vec::new();

    // sh:19-30 — getopts "oOt:12JVx".  Single-token parse (getopts flag
    // clustering like `-12` is not reproduced — `_describe` callers pass
    // each flag separately in practice).
    let mut idx = 0usize;
    while idx < args.len() {
        match args[idx].as_str() {
            "-o" => {
                _type = "options".to_string(); // sh:22
                idx += 1;
            }
            "-O" => {
                _type = "options".to_string(); // sh:24
                _noprefix = true; // sh:25
                idx += 1;
            }
            "-t" if idx + 1 < args.len() => {
                _type = args[idx + 1].clone(); // sh:27
                idx += 2;
            }
            "-1" | "-2" | "-J" | "-V" | "-x" => {
                _jvx12.push(args[idx].clone()); // sh:28-29
                idx += 1;
            }
            _ => break,
        }
    }

    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:38-39 — options + prefix not [-+]* + prefix-needed (default-true)
    // ⇒ bail.  `! -prefix [-+]*` is true when PREFIX does not start with
    // `-` or `+`.
    if _type == "options" && !_noprefix {
        let prefix = getsparam("PREFIX").unwrap_or_default();
        let is_dashplus = prefix.starts_with('-') || prefix.starts_with('+');
        if !is_dashplus
            && zstyle_t_default_true(
                &format!(":completion:{}:options", curcontext),
                "prefix-needed",
            )
        {
            return 1;
        }
    }

    let style_ctx = format!(":completion:{}:{}", curcontext, _type);

    // sh:42 — verbose (default-true) ⇒ show descriptions.
    let _showd = zstyle_t_default_true(&style_ctx, "verbose");

    // sh:46 — list-separator, default "--".
    let _sep = zstyle_s(&style_ctx, "list-separator").unwrap_or_else(|| "--".to_string());

    // sh:47-48 — max-matches-width, default COLUMNS/2.
    let _mlen: String = zstyle_s(&style_ctx, "max-matches-width").unwrap_or_else(|| {
        let cols = getiparam("COLUMNS");
        (if cols > 0 { cols / 2 } else { 0 }).to_string()
    });

    // sh:51 — _descr="$1"; shift.
    if idx >= args.len() {
        return 1;
    }
    let _descr = args[idx].clone();
    idx += 1;

    // The remaining args (value-array names, optional match arrays,
    // `--`-separated per-set opts) — the shell's `"$@"`.
    let positional: Vec<String> = args[idx..].to_vec();

    // sh:53-58 — list-grouped (default-true) under verbose ⇒ pre-pass.
    let grouped = _showd && zstyle_t_default_true(&style_ctx, "list-grouped");
    let _oargv: Vec<String> = if grouped {
        positional.clone()
    } else {
        Vec::new()
    };

    // sh:61-63 — options + prefix-hidden (default-false) ⇒ strip the
    // leading `--`/`-`/`+` of PREFIX into _hide (= ${(M)PREFIX##(--|[-+])}).
    if _type == "options"
        && zstyle_t_default_false(
            &format!(":completion:{}:options", curcontext),
            "prefix-hidden",
        )
    {
        let prefix = getsparam("PREFIX").unwrap_or_default();
        _hide = if prefix.starts_with("--") {
            "--".to_string()
        } else if prefix.starts_with('-') {
            "-".to_string()
        } else if prefix.starts_with('+') {
            "+".to_string()
        } else {
            String::new()
        };
    }

    // sh:64 — request the tag.
    let _ = _tags(&[_type.clone()]);

    let csl = get_compstate_str("list").unwrap_or_default();
    let mut _try = 0i32;

    // sh:65 — while _tags; do
    while _tags(&[]) == 0 {
        // sh:66 — while _next_label $_jvx12 "$_type" _expl "$_descr"; do
        loop {
            let mut nl_args = _jvx12.clone();
            nl_args.push(_type.clone());
            nl_args.push("_expl".to_string());
            nl_args.push(_descr.clone());
            if _next_label(&nl_args) != 0 {
                break;
            }

            // Transient `_a_<try><i>` params created by the grouped
            // pre-pass — unset at the end of this iteration.
            let mut a_names: Vec<String> = Vec::new();

            // The argv handed to compdescribe: `$@`.  In grouped mode it
            // is rebuilt from _oargv each iteration; otherwise it is the
            // untouched positional list.
            let cd_argv: Vec<String> = if grouped {
                // sh:68-116 — grouped -D/-O pre-pass.  Walk _oargv,
                // stashing each value array (and optional match array)
                // into a fresh `_a_<try><i>` param, doing a dry-run
                // `compadd -D _strs [-O _mats]` to filter to the matches
                // that actually apply, then rebuild argv from the stash.
                _try += 1; // sh:73
                let _expl_vals = getaparam("_expl").unwrap_or_default();
                let mut _argv: Vec<String> = Vec::with_capacity(_oargv.len());
                let mut _i = 1i32; // 1-based, mirrors the shell
                let mut p = 0usize;
                while p < _oargv.len() {
                    // sh:76-84 — value array → _a_<try><i>.
                    let _strs = format!("_a_{}{}", _try, _i);
                    let vals = resolve_array_arg(&_oargv[p]);
                    setaparam(&_strs, vals.clone());
                    a_names.push(_strs.clone());
                    _argv.push(_strs.clone());
                    p += 1;
                    _i += 1;

                    // sh:86-97 — optional match array (only if next arg
                    // is non-empty and not an option).
                    let (mats_name, mats_vals): (Option<String>, Vec<String>) = if p >= _oargv.len()
                        || _oargv[p].is_empty()
                        || _oargv[p].starts_with('-')
                    {
                        (None, Vec::new())
                    } else {
                        let mn = format!("_a_{}{}", _try, _i);
                        let mv = resolve_array_arg(&_oargv[p]);
                        setaparam(&mn, mv.clone());
                        a_names.push(mn.clone());
                        _argv.push(mn.clone());
                        p += 1;
                        _i += 1;
                        (Some(mn), mv)
                    };

                    // sh:99-104 — gather per-set opts up to `--`.
                    let mut _opts: Vec<String> = Vec::new();
                    while p < _oargv.len() && _oargv[p] != "--" {
                        _opts.push(_oargv[p].clone());
                        _argv.push(_oargv[p].clone());
                        p += 1;
                        _i += 1;
                    }
                    if p < _oargv.len() && _oargv[p] == "--" {
                        _argv.push("--".to_string()); // sh:106 leave `--` in place
                        p += 1;
                        _i += 1;
                    }

                    // sh:110-116 — dry-run compadd that filters _strs
                    // (and records into _mats) against the current word.
                    let mut cadd: Vec<String> = _opts;
                    cadd.push("-2".to_string());
                    cadd.push("-o".to_string());
                    cadd.push("nosort".to_string());
                    cadd.extend(_expl_vals.clone());
                    cadd.push("-D".to_string());
                    cadd.push(_strs.clone());
                    if let Some(mn) = &mats_name {
                        cadd.push("-O".to_string());
                        cadd.push(mn.clone());
                    }
                    cadd.push("-".to_string());
                    let words = if mats_name.is_some() {
                        extract_match_parts(&mats_vals)
                    } else {
                        extract_match_parts(&vals)
                    };
                    cadd.extend(words);
                    // The two upstream branches are merged into one call here,
                    // so publish whichever branch's line this invocation is.
                    crate::compsys::ported::shared::set_sh_lineno(if mats_name.is_some() {
                        111
                    } else {
                        114
                    });
                    bin_compadd("compadd", &cadd, &make_ops(), 0);
                }
                _argv // sh:118 set - "$_argv[@]"
            } else {
                positional.clone()
            };

            // sh:121-125 — init parsed state.
            if _showd {
                // compdescribe -I "$_hide" "$_mlen" "$_sep " _expl "$_grp[@]" "$@"
                let mut cd: Vec<String> = vec![
                    "-I".to_string(),
                    _hide.clone(),
                    _mlen.clone(),
                    format!("{} ", _sep),
                    "_expl".to_string(),
                ];
                if grouped {
                    cd.push("-g".to_string());
                }
                cd.extend(cd_argv.clone());
                crate::compsys::ported::shared::set_sh_lineno(122);
                bin_compdescribe("compdescribe", &cd, &make_ops(), 0);
            } else {
                // compdescribe -i "$_hide" "$_mlen" "$@"
                let mut cd: Vec<String> = vec!["-i".to_string(), _hide.clone(), _mlen.clone()];
                cd.extend(cd_argv.clone());
                crate::compsys::ported::shared::set_sh_lineno(124);
                let rc_i = bin_compdescribe("compdescribe", &cd, &make_ops(), 0);
                tracing::debug!(target: "compsys_args", rc_i, ?cd_argv, "compdescribe -i");
            }

            // sh:127 — compstate[list]="$csl".
            set_compstate_str("list", &csl);

            // sh:129-135 — pull each group out and add it.
            loop {
                let g_argv = [
                    "-g".to_string(),
                    "csl2".to_string(),
                    "_args".to_string(),
                    "_tmpm".to_string(),
                    "_tmpd".to_string(),
                ];
                crate::compsys::ported::shared::set_sh_lineno(129);
                if bin_compdescribe("compdescribe", &g_argv, &make_ops(), 0) != 0 {
                    break;
                }

                // sh:131-132 — compstate[list]="$csl $csl2"; drop "rows".
                let csl2 = getsparam("csl2").unwrap_or_default();
                let mut list_val = format!("{} {}", csl, csl2);
                if !csl2.is_empty() {
                    list_val = list_val.replacen("rows", "", 1);
                }
                set_compstate_str("list", &list_val);

                // sh:134 — compadd "$_args[@]" -d _tmpd -a _tmpm && _ret=0.
                let mut cadd: Vec<String> = getaparam("_args").unwrap_or_default();
                cadd.push("-d".to_string());
                cadd.push("_tmpd".to_string());
                cadd.push("-a".to_string());
                cadd.push("_tmpm".to_string());
                crate::compsys::ported::shared::set_sh_lineno(134);
                let rc_add = bin_compadd("compadd", &cadd, &make_ops(), 0);
                tracing::debug!(
                    target: "compsys_args",
                    rc_add,
                    tmpm = getaparam("_tmpm").unwrap_or_default().len(),
                    tmpd = getaparam("_tmpd").unwrap_or_default().len(),
                    args = ?getaparam("_args").unwrap_or_default(),
                    "describe group compadd"
                );
                if rc_add == 0 {
                    _ret = 0;
                }
            }

            // Clean up transient grouped-pre-pass params.
            for n in &a_names {
                unsetparam(n);
            }
        }

        // sh:137 — (( _ret )) || return 0.
        if _ret == 0 {
            return 0;
        }
    }

    // sh:140 — return 1.
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_for_empty_args() {
        let _g = crate::test_util::global_state_lock();
        // No `_descr` arg ⇒ sh:51 early return 1.
        assert_eq!(_describe_impl(&[]), 1);
    }

    #[test]
    fn returns_one_with_no_tag_setup() {
        let _g = crate::test_util::global_state_lock();
        // `_tags` reports no requested tag ⇒ the outer while never runs
        // and _ret stays 1 (sh:65/sh:134).
        assert_eq!(
            _describe_impl(&[
                "-t".to_string(),
                "mytag".to_string(),
                "description".to_string(),
                "myarr".to_string(),
            ]),
            1
        );
    }

    #[test]
    fn extract_match_parts_splits_on_unescaped_colon_and_unescapes() {
        // value:description ⇒ value; `\:` is an escaped colon inside the
        // value; `\\` collapses to `\`.
        assert_eq!(
            extract_match_parts(&[
                "foo:the foo".to_string(),
                "a\\:b:desc".to_string(),
                "plain".to_string(),
            ]),
            vec!["foo".to_string(), "a:b".to_string(), "plain".to_string()]
        );
    }

    #[test]
    fn style_truthiness_matches_zstyle_semantics() {
        // -T (default true): unset ⇒ true; explicit false-ish ⇒ false.
        assert!(style_is_true(&["yes".to_string()]));
        assert!(style_is_true(&["1".to_string()]));
        assert!(!style_is_true(&["no".to_string()]));
        assert!(!style_is_true(&[]));
    }

    #[test]
    fn resolve_array_arg_parses_inline_literal() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            resolve_array_arg("(a b c)"),
            vec!["a".to_string(), "b".to_string(), "c".to_string()]
        );
    }
}
