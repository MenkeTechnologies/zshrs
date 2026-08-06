//! Port of `_sep_parts` from `Completion/Base/Utility/_sep_parts`.
//!
//! Faithful 1:1 translation of the upstream shell function. Local
//! variable names mirror the source (`str`, `arr`, `sep`, `testarr`,
//! `tmparr`, `prefix`, `suffixes`, `autosuffix`, `group`, `expl`,
//! `opts`, `matcher`, `nm`, `opre`, `osuf`). The transient by-name
//! arrays `testarr` / `tmparr` are populated by `compadd -O` and
//! unset before returning.
//!
//! Upstream body (verbatim structure, line numbers from zsh 5.9):
//! ```text
//! sh:  1  #autoload
//! sh: 20  local str arr sep test testarr tmparr prefix suffixes autosuffix
//! sh: 21  local matchflags opt group expl nm=$compstate[nmatches] opre osuf opts matcher
//! sh: 25  zparseopts -D -a opts 'J+:=group' 'V+:=group' P: F: S: r: R: q 1 2 o+: n \
//! sh: 26      'x+:=expl' 'X+:=expl' 'M+:=matcher'
//! sh: 30  opre="$PREFIX"; osuf="$SUFFIX"
//! sh: 32  str="$PREFIX$SUFFIX"; SUFFIX=""; prefix=""
//! sh: 38  while [[ $# -gt 1 ]]; do
//! sh: 40    arr="$1"; sep="$2"
//! sh: 44    [[ "$arr[1]" == '(' ]] && tmparr=( ${=arr[2,-2]} ); arr=tmparr
//! sh: 50    [[ "$str" != *${sep}* ]] && break
//! sh: 54    PREFIX="${str%%(|\\)${sep}*}"
//! sh: 55    builtin compadd -O testarr "$matcher[@]" -a "$arr"
//! sh: 56    [[ $#testarr -eq 0 && -n "$_comp_correct" ]] && compadd -O testarr …
//! sh: 62    (( $#testarr )) || return 1
//! sh: 63    [[ $#testarr -gt 1 ]] && break
//! sh: 68    prefix="${prefix}${testarr[1]}${sep}"; str="${str#*${sep}}"; shift 2
//! sh: 71  done
//! sh: 75  arr="$1"; [[ "$arr[1]" == '(' ]] && tmparr=( ${=arr[2,-2]} ); arr=tmparr
//! sh: 81  if [[ $# -le 1 || "$str" != *${2}* ]]; then
//! sh: 84    PREFIX="$str"; builtin compadd -O testarr "$matcher[@]" -a "$arr"; …
//! sh: 88  fi
//! sh: 90  [[ $#testarr -eq 0 || ${#testarr[1]} -eq 0 ]] && return 1
//! sh: 94  shift; suffixes=(""); autosuffix=()
//! sh: 98  while [[ $# -gt 0 && "$str" == *${1}* ]]; do
//! sh:101   str="${str#*${1}}"
//! sh:106   [[ $# -gt 2 ]] && PREFIX="${str%%${3}*}" || PREFIX="$str"
//! sh:115   arr="$2"; [[ "$arr[1]" == '(' ]] && tmparr=( ${=arr[2,-2]} ); arr=tmparr
//! sh:121   builtin compadd -O tmparr "$matcher[@]" -a "$arr"; …
//! sh:125   suffixes=("${(@)^suffixes[@]}${(q)1}${(@)^tmparr}"); shift 2
//! sh:128  done
//! sh:133  (( $# )) && autosuffix=(-qS "${(q)1}")
//! sh:137  PREFIX="$pre"; SUFFIX="$suf"
//! sh:139  for i in "$suffixes[@]"; do
//! sh:140    compadd -U "$group[@]" "$expl[@]" "$autosuffix[@]" "$opts[@]" \
//! sh:141            -i "$IPREFIX" -I "$ISUFFIX" -p "$prefix" -s "$i" -a testarr
//! sh:142  done
//! sh:146  [[ nm -ne compstate[nmatches] ]]
//! ```

use crate::ported::params::{getaparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::get_compstate_str;
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

/// Parsed result of the upstream `zparseopts` call (sh:25-26).
struct ParsedOpts {
    /// `-a opts`: P F S r R q 1 2 o n (option + value, verbatim).
    opts: Vec<String>,
    /// `J+:=group` / `V+:=group`.
    group: Vec<String>,
    /// `x+:=expl` / `X+:=expl`.
    expl: Vec<String>,
    /// `M+:=matcher`.
    matcher: Vec<String>,
    /// Remaining positional arguments (arrays + separators).
    rest: Vec<String>,
}

/// Mirror of the `zparseopts -D` call at sh:25-26. Recognized options
/// are consumed from the front; parsing stops at the first
/// non-option (no `-E`), which is where the array/separator
/// positional arguments begin. Options with `=name` specs go ONLY to
/// that named array, matching real zparseopts semantics (verified
/// against zsh 5.9).
fn zparse(args: &[String]) -> ParsedOpts {
    let mut opts = Vec::new();
    let mut group = Vec::new();
    let mut expl = Vec::new();
    let mut matcher = Vec::new();
    let mut i = 0usize;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            i += 1;
            break;
        }
        if !(a.starts_with('-') && a.len() >= 2) {
            break;
        }
        let flag = a.as_bytes()[1] as char;
        // Attached value form `-Jval`, else the next word.
        let attached: Option<String> = if a.len() > 2 {
            Some(a[2..].to_string())
        } else {
            None
        };
        match flag {
            // spec `J+:` / `V+:` → group ; `x+:` / `X+:` → expl ;
            // `M+:` → matcher ; `P:` `F:` `S:` `r:` `R:` `o+:` → opts.
            'J' | 'V' | 'x' | 'X' | 'M' | 'P' | 'F' | 'S' | 'r' | 'R' | 'o' => {
                let dest = match flag {
                    'J' | 'V' => &mut group,
                    'x' | 'X' => &mut expl,
                    'M' => &mut matcher,
                    _ => &mut opts,
                };
                dest.push(format!("-{}", flag));
                if let Some(v) = attached {
                    dest.push(v);
                    i += 1;
                } else if i + 1 < args.len() {
                    dest.push(args[i + 1].clone());
                    i += 2;
                } else {
                    i += 1;
                }
            }
            // Boolean flags `q 1 2 n` → opts.
            'q' | '1' | '2' | 'n' => {
                opts.push(format!("-{}", flag));
                i += 1;
            }
            // Unrecognized leading dash: stop (zparseopts leaves it).
            _ => break,
        }
    }
    ParsedOpts {
        opts,
        group,
        expl,
        matcher,
        rest: args[i..].to_vec(),
    }
}

/// Number of elements currently stored in the named shell array.
fn arrlen(name: &str) -> usize {
    getaparam(name).map(|v| v.len()).unwrap_or(0)
}

/// `${(q)word}` — backslash-quote shell-special characters.
/// Approximation of zsh `(q)`: alphanumerics and the common
/// separator characters (`@ % + = : , . / - _`) pass through
/// unquoted; anything else is backslash-escaped. Single-character
/// separators (the usual `_sep_parts` case) are covered exactly.
fn q_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric()
            || matches!(c, '@' | '%' | '+' | '=' | ':' | ',' | '.' | '/' | '-' | '_')
        {
            out.push(c);
        } else {
            out.push('\\');
            out.push(c);
        }
    }
    out
}

/// Resolve `$1` into the shell-array NAME to hand to `compadd -a`.
/// A literal `(foo bar)` is split on whitespace into `tmparr` (via
/// `${=arr[2,-2]}`, sh:44/75/115) and the name `"tmparr"` returned;
/// otherwise the argument is already an array name and returned as-is.
fn resolve_array_name(arr: &str) -> String {
    if arr.starts_with('(') && arr.ends_with(')') && arr.len() >= 2 {
        let inner = &arr[1..arr.len() - 1];
        let elems: Vec<String> = inner.split_whitespace().map(|w| w.to_string()).collect();
        setaparam("tmparr", elems);
        "tmparr".to_string()
    } else {
        arr.to_string()
    }
}

/// `_sep_parts` — complete each part of a separator-delimited string
/// from the paired (array, separator) arguments.
pub fn _sep_parts(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_sep_parts");
    // sh:21  nm=$compstate[nmatches]
    let nm: i64 = get_compstate_str("nmatches")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    // sh:25-26  zparseopts
    let parsed = zparse(args);
    let ParsedOpts {
        opts,
        group,
        expl,
        matcher,
        rest,
    } = parsed;
    let comp_correct = getsparam("_comp_correct").unwrap_or_default();

    // sh:30-32
    let _opre = getsparam("PREFIX").unwrap_or_default();
    let _osuf = getsparam("SUFFIX").unwrap_or_default();
    let mut str = format!(
        "{}{}",
        getsparam("PREFIX").unwrap_or_default(),
        getsparam("SUFFIX").unwrap_or_default()
    );
    let _ = setsparam("SUFFIX", "");
    let mut prefix = String::new();

    // Positional cursor: `$1`==rest[pos], `$#`==rest.len()-pos.
    let mut pos = 0usize;
    let nargs = |pos: usize| rest.len().saturating_sub(pos);

    // sh:38-71  walk (arr, sep) pairs to find the longest unambiguous prefix.
    while nargs(pos) > 1 {
        // sh:40
        let arr = resolve_array_name(&rest[pos]);
        let sep = rest[pos + 1].clone();

        // sh:50
        if !str.contains(&sep) {
            break;
        }

        // sh:54  PREFIX="${str%%(|\\)${sep}*}"
        let seg = {
            let cut = str.find(&sep).unwrap();
            let head = &str[..cut];
            head.strip_suffix('\\').unwrap_or(head).to_string()
        };
        let _ = setsparam("PREFIX", &seg);

        // sh:55  builtin compadd -O testarr "$matcher[@]" -a "$arr"
        //   `builtin` bypasses the `compadd()` shell function
        //   `_approximate` installs, so this pass matches EXACTLY;
        //   sh:56-57 then retries through the shadow if it found
        //   nothing.
        let mut argv: Vec<String> = vec!["-O".into(), "testarr".into()];
        argv.extend(matcher.iter().cloned());
        argv.push("-a".into());
        argv.push(arr.clone());
        let _ = bin_compadd_body("compadd", &argv, &make_ops(), 0);
        // sh:56-57  correction retry (identical call, shadowed)
        if arrlen("testarr") == 0 && !comp_correct.is_empty() {
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
        }

        // sh:62-63
        let testarr = getaparam("testarr").unwrap_or_default();
        if testarr.is_empty() {
            let _ = unsetparam("testarr");
            let _ = unsetparam("tmparr");
            return 1;
        }
        if testarr.len() > 1 {
            break;
        }

        // sh:68  single match → fold into prefix, skip it in str.
        prefix.push_str(&testarr[0]);
        prefix.push_str(&sep);
        str = str.splitn(2, &sep).nth(1).unwrap_or("").to_string();
        pos += 2; // shift 2
    }

    // sh:75  arr="$1" (empty when positionals are exhausted, per upstream)
    let arr = if pos < rest.len() {
        resolve_array_name(&rest[pos])
    } else {
        String::new()
    };

    // sh:81-88  no more separators → build the matches into testarr.
    let build = nargs(pos) <= 1 || {
        let sep2 = &rest[pos + 1];
        !str.contains(sep2 as &str)
    };
    if build {
        let _ = setsparam("PREFIX", &str);
        // sh:85  builtin compadd -O testarr "$matcher[@]" -a "$arr"
        //   (shadow bypassed — see sh:55 above)
        let mut argv: Vec<String> = vec!["-O".into(), "testarr".into()];
        argv.extend(matcher.iter().cloned());
        argv.push("-a".into());
        argv.push(arr.clone());
        let _ = bin_compadd_body("compadd", &argv, &make_ops(), 0);
        // sh:86-87  correction retry (shadowed)
        if arrlen("testarr") == 0 && !comp_correct.is_empty() {
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
        }
    }

    // sh:90
    let testarr = getaparam("testarr").unwrap_or_default();
    if testarr.is_empty() || testarr[0].is_empty() {
        let _ = unsetparam("testarr");
        let _ = unsetparam("tmparr");
        return 1;
    }

    // sh:94-95  shift; suffixes=(""); autosuffix=()
    pos += 1;
    let mut suffixes: Vec<String> = vec![String::new()];
    let mut autosuffix: Vec<String> = Vec::new();

    // sh:98-128  incrementally build the suffix cross-products.
    while nargs(pos) > 0 && str.contains(&rest[pos]) {
        let sep = rest[pos].clone();
        // sh:101  str="${str#*${1}}"
        str = str.splitn(2, &sep).nth(1).unwrap_or("").to_string();

        // sh:106
        if nargs(pos) > 2 {
            let sep3 = &rest[pos + 2];
            let seg = str.splitn(2, sep3 as &str).next().unwrap_or("").to_string();
            let _ = setsparam("PREFIX", &seg);
        } else {
            let _ = setsparam("PREFIX", &str);
        }

        // sh:115  arr="$2" (empty when `$2` is missing, per upstream)
        let arr2 = if pos + 1 < rest.len() {
            resolve_array_name(&rest[pos + 1])
        } else {
            String::new()
        };

        // sh:121  builtin compadd -O tmparr "$matcher[@]" -a "$arr"
        //   (shadow bypassed — see sh:55 above)
        let mut argv: Vec<String> = vec!["-O".into(), "tmparr".into()];
        argv.extend(matcher.iter().cloned());
        argv.push("-a".into());
        argv.push(arr2.clone());
        let _ = bin_compadd_body("compadd", &argv, &make_ops(), 0);
        // sh:122-123  correction retry. NOTE: upstream uses `- "$arr"`
        // (bare, not `-a`) here — an upstream inconsistency; the name
        // is added literally. Reproduced faithfully.
        if arrlen("tmparr") == 0 && !comp_correct.is_empty() {
            let mut argv2: Vec<String> = vec!["-O".into(), "tmparr".into()];
            argv2.extend(matcher.iter().cloned());
            argv2.push("-".into());
            argv2.push(arr2.clone());
            let _ = bin_compadd("compadd", &argv2, &make_ops(), 0);
        }

        // sh:125  suffixes=("${(@)^suffixes[@]}${(q)1}${(@)^tmparr}")
        // rc-style `^` double distribution → cartesian product.
        let tmparr = getaparam("tmparr").unwrap_or_default();
        let qsep = q_quote(&sep);
        let mut next: Vec<String> = Vec::with_capacity(suffixes.len() * tmparr.len().max(1));
        for s in &suffixes {
            for t in &tmparr {
                next.push(format!("{}{}{}", s, qsep, t));
            }
        }
        suffixes = next;
        pos += 2; // shift 2
    }

    // sh:133  (( $# )) && autosuffix=(-qS "${(q)1}")
    if nargs(pos) > 0 {
        autosuffix.push("-qS".into());
        autosuffix.push(q_quote(&rest[pos]));
    }

    // sh:137  PREFIX="$pre"; SUFFIX="$suf"
    // NOTE: upstream references `$pre`/`$suf`, which are never assigned
    // and are not in the `local` list — they expand to empty (unless a
    // caller happens to have globals of those names). Read them by
    // name to preserve that exact (quirky) behavior.
    let _ = setsparam("PREFIX", &getsparam("pre").unwrap_or_default());
    let _ = setsparam("SUFFIX", &getsparam("suf").unwrap_or_default());

    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    let isuffix = getsparam("ISUFFIX").unwrap_or_default();

    // sh:139-142  add the matches for each suffix.
    for i in &suffixes {
        let mut argv: Vec<String> = vec!["-U".into()];
        argv.extend(group.iter().cloned());
        argv.extend(expl.iter().cloned());
        argv.extend(autosuffix.iter().cloned());
        argv.extend(opts.iter().cloned());
        argv.push("-i".into());
        argv.push(iprefix.clone());
        argv.push("-I".into());
        argv.push(isuffix.clone());
        argv.push("-p".into());
        argv.push(prefix.clone());
        argv.push("-s".into());
        argv.push(i.clone());
        argv.push("-a".into());
        argv.push("testarr".into());
        let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
    }

    // Unset the transient by-name arrays.
    let _ = unsetparam("testarr");
    let _ = unsetparam("tmparr");

    // sh:146  [[ nm -ne compstate[nmatches] ]]
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
    use crate::ported::params::setsparam;

    #[test]
    fn returns_compadd_status() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("SUFFIX", "");
        let _r = _sep_parts(&[
            "(foo bar)".to_string(),
            "@".to_string(),
            "(host1 host2)".to_string(),
        ]);
        // Transient arrays must be cleaned up afterwards.
        assert!(getaparam("testarr").map(|v| v.is_empty()).unwrap_or(true));
        assert!(getaparam("tmparr").map(|v| v.is_empty()).unwrap_or(true));
    }

    #[test]
    fn zparse_routes_specs_to_named_arrays() {
        let p = zparse(&[
            "-J".into(),
            "grp".into(),
            "-M".into(),
            "mspec".into(),
            "-S".into(),
            "sfx".into(),
            "-q".into(),
            "(foo bar)".into(),
            "@".into(),
            "hosts".into(),
        ]);
        assert_eq!(p.group, vec!["-J", "grp"]);
        assert_eq!(p.matcher, vec!["-M", "mspec"]);
        assert_eq!(p.opts, vec!["-S", "sfx", "-q"]);
        assert!(p.expl.is_empty());
        assert_eq!(p.rest, vec!["(foo bar)", "@", "hosts"]);
    }
}
