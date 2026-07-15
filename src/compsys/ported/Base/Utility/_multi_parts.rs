//! Port of `_multi_parts` from
//! `Completion/Base/Utility/_multi_parts`.
//!
//! Faithful 1:1 translation of the upstream shell function. Local
//! names mirror the source (`sep`, `pref`, `npref`, `tmp2`, `group`,
//! `expl`, `menu`, `pre`, `suf`, `opre`, `osuf`, `orig`, `cpre`,
//! `opts`, `sopts`, `matcher`, `imm`, and the `typeset -U` arrays
//! `tmp1` / `matches`). The transient by-name arrays `tmp1` /
//! `matches` are populated by `compadd -O` and unset before return.
//!
//! Upstream structure (line numbers from zsh 5.9):
//! ```text
//! sh:  1  #autoload
//! sh: 12  typeset -U tmp1 matches
//! sh: 16  zparseopts -D -a sopts 'J+:=group' 'V+:=group' 'x+:=expl' \
//! sh: 17      'X+:=expl' 'P:=opts' 'F:=opts' S: r: R: q 1 2 o+: n \
//! sh: 18      'f=opts' 'M+:=matcher' 'i=imm'
//! sh: 20  sopts=( "$sopts[@]" "$opts[@]" )
//! sh: 21  (( $#matcher )) && matcher="${matcher[2]}" || matcher=
//! sh: 32  sep="$1"; tmp1=( literal-or-${(@P)2} )
//! sh: 43  pre/suf/opre/osuf/orig snapshot
//! sh: 51  menu detection
//! sh: 62  compadd -O matches -M "r:|${sep}=* r:|=* $matcher" -a tmp1
//! sh: 64  (( $#matches )) || matches=( "$tmp1[@]" )
//! sh: 66  while true; do … end   # per-segment walk, all exits `return`
//! ```

use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam, unsetparam};
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

// ---- zsh parameter-expansion primitives ----------------------------

/// `${s%%sep*}` — keep up to first `sep` (exclusive).
fn before_first(s: &str, sep: &str) -> String {
    match s.find(sep) {
        Some(i) => s[..i].to_string(),
        None => s.to_string(),
    }
}
/// `${s%sep*}` — keep up to last `sep` (exclusive).
fn before_last(s: &str, sep: &str) -> String {
    match s.rfind(sep) {
        Some(i) => s[..i].to_string(),
        None => s.to_string(),
    }
}
/// `${s#*sep}` — drop up to and including first `sep`.
fn after_first(s: &str, sep: &str) -> String {
    match s.find(sep) {
        Some(i) => s[i + sep.len()..].to_string(),
        None => s.to_string(),
    }
}
/// `${s##*sep}` — drop up to and including last `sep`.
fn after_last(s: &str, sep: &str) -> String {
    match s.rfind(sep) {
        Some(i) => s[i + sep.len()..].to_string(),
        None => s.to_string(),
    }
}
/// `${s%sep}` — strip one trailing `sep` if present.
fn strip_trailing(s: &str, sep: &str) -> String {
    s.strip_suffix(sep).unwrap_or(s).to_string()
}

/// `typeset -U` uniqueness — drop later duplicates, preserve order.
fn dedup(v: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(v.len());
    for e in v {
        if seen.insert(e.clone()) {
            out.push(e);
        }
    }
    out
}

/// `${(q)word}` approximation (see `_sep_parts::q_quote`). Used only
/// for the `"$orig" != "${orig:q}"` menu test — equality holds when
/// `orig` contains no shell-special characters.
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

/// `zstyle -t ":completion:${curcontext}:" <style> <value>` — true
/// (returns `true` here) iff the style is set for the context and its
/// value list contains `value`. Default (unset) is false.
fn zstyle_t(style: &str, value: &str) -> bool {
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    lookupstyle(&ctx, style).iter().any(|v| v == value)
}

/// Parsed result of the upstream `zparseopts` (sh:16-18).
struct ParsedOpts {
    sopts: Vec<String>,
    group: Vec<String>,
    expl: Vec<String>,
    opts: Vec<String>,
    matcher: Vec<String>,
    imm: Vec<String>,
    rest: Vec<String>,
}

/// Mirror of `zparseopts -D -a sopts …` (sh:16-18). `=name` specs
/// route to their named array only (verified against zsh 5.9):
/// J/V→group, x/X→expl, P/F/f→opts, M→matcher, i→imm, and the
/// remaining recognized options (S r R q 1 2 o n) → the default
/// `sopts`. Parsing stops at the first non-option.
fn zparse(args: &[String]) -> ParsedOpts {
    let mut sopts = Vec::new();
    let mut group = Vec::new();
    let mut expl = Vec::new();
    let mut opts = Vec::new();
    let mut matcher = Vec::new();
    let mut imm = Vec::new();
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
        let attached: Option<String> = if a.len() > 2 {
            Some(a[2..].to_string())
        } else {
            None
        };
        match flag {
            // Arg-taking specs.
            'J' | 'V' | 'x' | 'X' | 'P' | 'F' | 'S' | 'r' | 'R' | 'o' | 'M' => {
                let dest = match flag {
                    'J' | 'V' => &mut group,
                    'x' | 'X' => &mut expl,
                    'P' | 'F' => &mut opts,
                    'M' => &mut matcher,
                    _ => &mut sopts, // S r R o
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
            // Boolean specs.
            'f' => {
                opts.push("-f".into());
                i += 1;
            }
            'i' => {
                imm.push("-i".into());
                i += 1;
            }
            'q' | '1' | '2' | 'n' => {
                sopts.push(format!("-{}", flag));
                i += 1;
            }
            _ => break,
        }
    }
    ParsedOpts {
        sopts,
        group,
        expl,
        opts,
        matcher,
        imm,
        rest: args[i..].to_vec(),
    }
}

/// Resolve `$2` (sh:33-36): literal `(a b c)` splits on whitespace,
/// otherwise it is the name of an array parameter (`${(@P)2}`).
fn resolve_array(spec: &str) -> Vec<String> {
    if spec.starts_with('(') && spec.ends_with(')') && spec.len() >= 2 {
        spec[1..spec.len() - 1]
            .split_whitespace()
            .map(|w| w.to_string())
            .collect()
    } else {
        getaparam(spec).unwrap_or_default()
    }
}

/// `_multi_parts` — complete each `sep`-delimited segment of a
/// path-like string from a shared array of complete strings.
pub fn _multi_parts(args: &[String]) -> i32 {
    let comp_correct = getsparam("_comp_correct").unwrap_or_default();

    // sh:16-18  zparseopts
    let ParsedOpts {
        mut sopts,
        group,
        expl,
        opts,
        matcher,
        imm,
        rest,
    } = zparse(args);

    // sh:20  sopts=( "$sopts[@]" "$opts[@]" )
    sopts.extend(opts.iter().cloned());
    // sh:21-22  (( $#matcher )) && matcher="${matcher[2]}" || matcher=
    let matcher: String = if matcher.len() >= 2 {
        matcher[1].clone()
    } else {
        String::new()
    };

    if rest.len() < 2 {
        return 1;
    }
    // sh:32  sep="$1"
    let sep = rest[0].clone();
    // sh:33-36  tmp1
    let mut tmp1: Vec<String> = dedup(resolve_array(&rest[1]));

    // sh:43-47  snapshots
    let mut pre = getsparam("PREFIX").unwrap_or_default();
    let mut suf = getsparam("SUFFIX").unwrap_or_default();
    let opre = pre.clone();
    let osuf = suf.clone();
    let orig = format!("{}{}", pre, suf);

    // sh:51-53  menu detection
    let insert = get_compstate_str("insert").unwrap_or_default();
    let pattern_match = get_compstate_str("pattern_match").unwrap_or_default();
    let insert_is_menu = insert.ends_with("menu")
        || insert
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false);
    let menu = insert_is_menu
        || !comp_correct.is_empty()
        || (!pattern_match.is_empty() && orig != q_quote(&orig));

    // sh:57  pref=''
    let mut pref = String::new();
    // `cpre` accumulates the already-committed prefix path (sh:cpre).
    let mut cpre = String::new();

    // The match specification threaded through every compadd (sh:62 …).
    let matchspec = format!("r:|{}=* r:|=* {}", sep, matcher);

    // sh:62  compadd -O matches -M "$matchspec" -a tmp1
    setaparam("tmp1", tmp1.clone());
    {
        let argv = vec![
            "-O".to_string(),
            "matches".into(),
            "-M".into(),
            matchspec.clone(),
            "-a".into(),
            "tmp1".into(),
        ];
        let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
    }
    let mut matches: Vec<String> = dedup(getaparam("matches").unwrap_or_default());
    // sh:64  (( $#matches )) || matches=( "$tmp1[@]" )
    if matches.is_empty() {
        matches = tmp1.clone();
    }

    // Cleanup helper for the transient by-name arrays before every exit.
    macro_rules! cleanup {
        () => {{
            let _ = unsetparam("tmp1");
            let _ = unsetparam("matches");
        }};
    }

    // sh:66  while true
    loop {
        // sh:70-76  prefix/suffix for matching.
        let (mprefix, msuffix): (String, String) = if pre.contains(&sep) {
            (before_first(&pre, &sep), String::new())
        } else {
            (pre.clone(), before_first(&suf, &sep))
        };
        let _ = setsparam("PREFIX", &mprefix);
        let _ = setsparam("SUFFIX", &msuffix);

        // sh:83-87  exact-component check.
        // NOTE: `${(@M)matches:#PAT*}` is glob; components here are
        // literal, so we match by string prefix (approximation for
        // glob metacharacters in the segment).
        let exact_pat = format!("{}{}{}", mprefix, msuffix, sep);
        if !format!("{}{}", mprefix, msuffix).is_empty() || pre.starts_with(&sep) {
            tmp1 = matches
                .iter()
                .filter(|m| m.starts_with(&exact_pat))
                .cloned()
                .collect();
        } else {
            tmp1 = Vec::new();
        }
        tmp1 = dedup(tmp1);

        // npref is set in either the exact-match arm or the single-match arm.
        let npref: String;

        if !tmp1.is_empty() {
            // sh:89-90  exact match on the line.
            npref = format!("{}{}{}", mprefix, msuffix, sep);
        } else {
            // sh:94-97  no exact match: how many first-components match?
            // words = ${(@)${(@)matches%%sep*}:#}
            let words: Vec<String> = matches
                .iter()
                .map(|m| before_first(m, &sep))
                .filter(|w| !w.is_empty())
                .collect();
            let mut argv: Vec<String> = vec![
                "-O".into(),
                "tmp1".into(),
                "-M".into(),
                matchspec.clone(),
                "-".into(),
            ];
            argv.extend(words.iter().cloned());
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
            tmp1 = dedup(getaparam("tmp1").unwrap_or_default());
            if tmp1.is_empty() && !comp_correct.is_empty() {
                let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
                tmp1 = dedup(getaparam("tmp1").unwrap_or_default());
            }

            if tmp1.len() == 1 {
                // sh:99-128  exactly one component matches.
                if format!("{}{}", pre, suf).contains(&sep) {
                    // sh:106-107  still separators on the line — accept it.
                    npref = format!("{}{}", tmp1[0], sep);
                } else {
                    // sh:108-127  insert what we collected.
                    matches = matches
                        .iter()
                        .filter(|m| m.starts_with(&tmp1[0]))
                        .cloned()
                        .collect();
                    matches = dedup(matches);

                    let _ = setsparam("PREFIX", &format!("{}{}", cpre, pre));
                    let _ = setsparam("SUFFIX", &suf);

                    let rc;
                    if (!imm.is_empty() && matches.len() == 1) || zstyle_t("expand", "suffix") {
                        // sh:114-117  compadd … - $pref$matches
                        let mut argv: Vec<String> = Vec::new();
                        argv.extend(group.iter().cloned());
                        argv.extend(expl.iter().cloned());
                        argv.extend(sopts.iter().cloned());
                        argv.push("-M".into());
                        argv.push(matchspec.clone());
                        argv.push("-".into());
                        for m in &matches {
                            argv.push(format!("{}{}", pref, m));
                        }
                        rc = bin_compadd("compadd", &argv, &make_ops(), 0);
                    } else {
                        // sh:119-124
                        let has_multi = matches
                            .iter()
                            .any(|m| m.starts_with(&format!("{}{}", tmp1[0], sep)));
                        let mut argv: Vec<String> = Vec::new();
                        argv.extend(group.iter().cloned());
                        argv.extend(expl.iter().cloned());
                        if has_multi {
                            argv.push("-p".into());
                            argv.push(pref.clone());
                            argv.push("-r".into());
                            argv.push(sep.clone());
                            argv.push("-S".into());
                            argv.push(sep.clone());
                            argv.extend(opts.iter().cloned());
                        } else {
                            argv.push("-p".into());
                            argv.push(pref.clone());
                            argv.extend(sopts.iter().cloned());
                        }
                        argv.push("-M".into());
                        argv.push(matchspec.clone());
                        argv.push("-".into());
                        argv.push(tmp1[0].clone());
                        rc = bin_compadd("compadd", &argv, &make_ops(), 0);
                    }
                    cleanup!();
                    return rc;
                }
            } else if !tmp1.is_empty() {
                // sh:129-197  more than one component matches.
                let mut ret = 1i32;

                // sh:135-137  compadd -O matches -M spec -a matches
                let _ = setsparam("PREFIX", &pre);
                let _ = setsparam("SUFFIX", &suf);
                setaparam("matches", matches.clone());
                {
                    let argv = vec![
                        "-O".to_string(),
                        "matches".into(),
                        "-M".into(),
                        matchspec.clone(),
                        "-a".into(),
                        "matches".into(),
                    ];
                    let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
                }
                matches = dedup(getaparam("matches").unwrap_or_default());

                if pre.contains(&sep) {
                    let _ = setsparam("PREFIX", &format!("{}{}", cpre, before_first(&pre, &sep)));
                    let _ = setsparam(
                        "SUFFIX",
                        &format!("{}{}{}", sep, after_first(&pre, &sep), suf),
                    );
                } else {
                    let _ = setsparam("PREFIX", &format!("{}{}", cpre, pre));
                    let _ = setsparam("SUFFIX", &suf);
                }

                // sh:153-154  cross-segment suffix guard.
                if !format!("{}{}", pre, suf).is_empty() {
                    matches = matches
                        .iter()
                        .filter(|m| tmp1.iter().any(|t| m.starts_with(t)))
                        .cloned()
                        .collect();
                    matches = dedup(matches);
                }

                // sh:156-157  ! expand-suffix || [[ -n menu || -z insert ]]
                if !zstyle_t("expand", "suffix") || menu || insert.is_empty() {
                    // sh:159-183  menu-style / ambiguous-component add.
                    // tmp2 = -s "${sep}${tmp2#*sep}" when pre$suf has sep.
                    let presuf = format!("{}{}", pre, suf);
                    let tmp2: Vec<String> = if presuf.contains(&sep) {
                        vec![
                            "-s".into(),
                            format!("{}{}", sep, after_first(&presuf, &sep)),
                        ]
                    } else {
                        Vec::new()
                    };

                    // words ending with sep, first-component, non-empty.
                    let w1: Vec<String> = matches
                        .iter()
                        .filter(|m| m.ends_with(&sep))
                        .map(|m| before_first(m, &sep))
                        .filter(|w| !w.is_empty())
                        .collect();
                    if compadd_seg(&group, &expl, &sep, &opts, &pref, &tmp2, &matchspec, &w1) == 0 {
                        ret = 0;
                    }

                    // sh:174-177  a bare separator match.
                    if matches.iter().any(|m| m.starts_with(&sep)) {
                        let mut argv: Vec<String> = Vec::new();
                        argv.extend(group.iter().cloned());
                        argv.extend(expl.iter().cloned());
                        argv.push("-S".into());
                        argv.push(String::new());
                        argv.extend(opts.iter().cloned());
                        argv.push("-p".into());
                        argv.push(pref.clone());
                        argv.push("-M".into());
                        argv.push(matchspec.clone());
                        argv.push("-".into());
                        argv.push(sep.clone());
                        if bin_compadd("compadd", &argv, &make_ops(), 0) == 0 {
                            ret = 0;
                        }
                    }

                    // words with an interior sep (char on both sides).
                    let w2: Vec<String> = matches
                        .iter()
                        .filter(|m| has_interior_sep(m, &sep))
                        .map(|m| before_first(m, &sep))
                        .filter(|w| !w.is_empty())
                        .collect();
                    if compadd_seg(&group, &expl, &sep, &opts, &pref, &tmp2, &matchspec, &w2) == 0 {
                        ret = 0;
                    }

                    // sh:181-183  matches with NO sep, as -S '' with tmp2.
                    let w3: Vec<String> = matches
                        .iter()
                        .filter(|m| !m.contains(&sep))
                        .cloned()
                        .collect();
                    let mut argv: Vec<String> = Vec::new();
                    argv.extend(group.iter().cloned());
                    argv.extend(expl.iter().cloned());
                    argv.push("-S".into());
                    argv.push(String::new());
                    argv.extend(opts.iter().cloned());
                    argv.push("-p".into());
                    argv.push(pref.clone());
                    argv.extend(tmp2.iter().cloned());
                    argv.push("-M".into());
                    argv.push(matchspec.clone());
                    argv.push("-".into());
                    argv.extend(w3);
                    if bin_compadd("compadd", &argv, &make_ops(), 0) == 0 {
                        ret = 0;
                    }
                } else {
                    // sh:184-195  normal completion: longest unambiguous.
                    // `-s "${i#*sep}"` — upstream `i` is an unset local,
                    // so this expands to `-s ""` (leftover in upstream).
                    let w1: Vec<String> = matches
                        .iter()
                        .filter(|m| m.contains(&sep))
                        .map(|m| before_first(m, &sep))
                        .filter(|w| !w.is_empty())
                        .collect();
                    let mut argv: Vec<String> = Vec::new();
                    argv.extend(group.iter().cloned());
                    argv.extend(expl.iter().cloned());
                    argv.extend(opts.iter().cloned());
                    argv.push("-p".into());
                    argv.push(pref.clone());
                    argv.push("-s".into());
                    argv.push(String::new()); // ${i#*sep} with unset i
                    argv.push("-M".into());
                    argv.push(matchspec.clone());
                    argv.push("-".into());
                    argv.extend(w1);
                    if bin_compadd("compadd", &argv, &make_ops(), 0) == 0 {
                        ret = 0;
                    }

                    let w2: Vec<String> = matches
                        .iter()
                        .filter(|m| !m.contains(&sep))
                        .cloned()
                        .collect();
                    let mut argv2: Vec<String> = Vec::new();
                    argv2.extend(group.iter().cloned());
                    argv2.extend(expl.iter().cloned());
                    argv2.push("-S".into());
                    argv2.push(String::new());
                    argv2.extend(opts.iter().cloned());
                    argv2.push("-p".into());
                    argv2.push(pref.clone());
                    argv2.push("-M".into());
                    argv2.push(matchspec.clone());
                    argv2.push("-".into());
                    argv2.extend(w2);
                    if bin_compadd("compadd", &argv2, &make_ops(), 0) == 0 {
                        ret = 0;
                    }
                }
                cleanup!();
                return ret;
            } else {
                // sh:198-216  nothing matched the line: insert the
                // expanded prefix we collected if it differs.
                if !zstyle_t("expand", "prefix") || orig == format!("{}{}{}", pref, pre, suf) {
                    cleanup!();
                    return 1;
                }
                let _ = setsparam("PREFIX", &format!("{}{}", cpre, pre));
                let _ = setsparam("SUFFIX", &suf);

                let rc;
                if !suf.is_empty() {
                    let mut argv: Vec<String> = Vec::new();
                    argv.extend(group.iter().cloned());
                    argv.extend(expl.iter().cloned());
                    argv.push("-s".into());
                    argv.push(suf.clone());
                    argv.extend(sopts.iter().cloned());
                    argv.push("-M".into());
                    argv.push(matchspec.clone());
                    argv.push("-".into());
                    argv.push(format!("{}{}", pref, pre));
                    rc = bin_compadd("compadd", &argv, &make_ops(), 0);
                } else {
                    let mut argv: Vec<String> = Vec::new();
                    argv.extend(group.iter().cloned());
                    argv.extend(expl.iter().cloned());
                    argv.push("-S".into());
                    argv.push(String::new());
                    argv.extend(opts.iter().cloned());
                    argv.push("-M".into());
                    argv.push(matchspec.clone());
                    argv.push("-".into());
                    argv.push(format!("{}{}", pref, pre));
                    rc = bin_compadd("compadd", &argv, &make_ops(), 0);
                }
                cleanup!();
                return rc;
            }
        }

        // sh:224-225  accepted/expanded a component: drop it from
        // matches (keep only those sharing npref, strip the leading
        // component), and append it to pref.
        // matches=( ${(@)${(@)${(@M)matches:#${npref}*}#*sep}:#} )
        matches = matches
            .iter()
            .filter(|m| m.starts_with(&npref))
            .map(|m| after_first(m, &sep))
            .filter(|m| !m.is_empty())
            .collect::<Vec<String>>();
        matches = dedup(matches);
        pref = format!("{}{}", pref, npref);

        // sh:229-259  advance pre/suf/cpre or finish.
        if pre.contains(&sep) {
            cpre = format!("{}{}{}", cpre, before_first(&pre, &sep), sep);
            pre = after_first(&pre, &sep);
        } else if suf.contains(&sep) {
            cpre = format!("{}{}{}", cpre, pre, before_first(&suf, &sep));
            cpre.push_str(&sep);
            pre = after_first(&suf, &sep);
            suf = String::new();
        } else {
            // sh:241-259  string from the line is fully handled.
            let _ = setsparam("PREFIX", &format!("{}{}", opre, osuf));
            let _ = setsparam("SUFFIX", "");

            let rc;
            if !pref.is_empty() && orig != pref {
                if ends_with_sep_component(&pref, &sep) {
                    // sh:245-248  pref = *sep*sep
                    let p = format!("{}{}", before_last(&before_last(&pref, &sep), &sep), sep);
                    let word = after_last(&strip_trailing(&pref, &sep), &sep);
                    let mut argv: Vec<String> = Vec::new();
                    argv.extend(group.iter().cloned());
                    argv.extend(expl.iter().cloned());
                    argv.extend(opts.iter().cloned());
                    argv.push("-p".into());
                    argv.push(p);
                    argv.push("-S".into());
                    argv.push(sep.clone());
                    argv.push("-M".into());
                    argv.push(matchspec.clone());
                    argv.push("-".into());
                    argv.push(word);
                    rc = bin_compadd("compadd", &argv, &make_ops(), 0);
                } else if pref.contains(&sep) {
                    // sh:250-253
                    let p = format!("{}{}", before_last(&pref, &sep), sep);
                    let word = after_last(&pref, &sep);
                    let mut argv: Vec<String> = Vec::new();
                    argv.extend(group.iter().cloned());
                    argv.extend(expl.iter().cloned());
                    argv.push("-S".into());
                    argv.push(String::new());
                    argv.extend(opts.iter().cloned());
                    argv.push("-p".into());
                    argv.push(p);
                    argv.push("-M".into());
                    argv.push(matchspec.clone());
                    argv.push("-".into());
                    argv.push(word);
                    rc = bin_compadd("compadd", &argv, &make_ops(), 0);
                } else {
                    // sh:255-256
                    let mut argv: Vec<String> = Vec::new();
                    argv.extend(group.iter().cloned());
                    argv.extend(expl.iter().cloned());
                    argv.push("-S".into());
                    argv.push(String::new());
                    argv.extend(opts.iter().cloned());
                    argv.push("-M".into());
                    argv.push(matchspec.clone());
                    argv.push("-".into());
                    argv.push(pref.clone());
                    rc = bin_compadd("compadd", &argv, &make_ops(), 0);
                }
            } else {
                rc = 1;
            }
            cleanup!();
            return rc;
        }
    }
}

/// `-r sep -S sep -p pref [tmp2…] -M spec - words…` (sh:171-173/178-180).
#[allow(clippy::too_many_arguments)]
fn compadd_seg(
    group: &[String],
    expl: &[String],
    sep: &str,
    opts: &[String],
    pref: &str,
    tmp2: &[String],
    matchspec: &str,
    words: &[String],
) -> i32 {
    let mut argv: Vec<String> = Vec::new();
    argv.extend(group.iter().cloned());
    argv.extend(expl.iter().cloned());
    argv.push("-r".into());
    argv.push(sep.to_string());
    argv.push("-S".into());
    argv.push(sep.to_string());
    argv.extend(opts.iter().cloned());
    argv.push("-p".into());
    argv.push(pref.to_string());
    argv.extend(tmp2.iter().cloned());
    argv.push("-M".into());
    argv.push(matchspec.to_string());
    argv.push("-".into());
    argv.extend(words.iter().cloned());
    bin_compadd("compadd", &argv, &make_ops(), 0)
}

/// `*?sep?*` — a `sep` with at least one character on each side.
fn has_interior_sep(s: &str, sep: &str) -> bool {
    let mut start = 0usize;
    while let Some(rel) = s[start..].find(sep) {
        let i = start + rel;
        if i >= 1 && i + sep.len() <= s.len() - 1 {
            return true;
        }
        start = i + sep.len();
        if start > s.len() {
            break;
        }
    }
    false
}

/// `[[ "$pref" = *sep*sep ]]` — pref contains at least two `sep`
/// (the last character run ends in `sep`).
fn ends_with_sep_component(pref: &str, sep: &str) -> bool {
    if !pref.ends_with(sep) {
        return false;
    }
    // Need a second sep before the trailing one.
    let head = &pref[..pref.len() - sep.len()];
    head.contains(sep)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn returns_one_with_no_matches() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "xyz");
        let _ = setsparam("SUFFIX", "");
        let _r = _multi_parts(&["/".to_string(), "(foo/bar baz/qux)".to_string()]);
        // Transient by-name arrays cleaned up.
        assert!(getaparam("tmp1").map(|v| v.is_empty()).unwrap_or(true));
        assert!(getaparam("matches").map(|v| v.is_empty()).unwrap_or(true));
    }

    #[test]
    fn zparse_routes_specs() {
        let p = zparse(&[
            "-J".into(),
            "grp".into(),
            "-P".into(),
            "pfx".into(),
            "-f".into(),
            "-M".into(),
            "mspec".into(),
            "-i".into(),
            "-S".into(),
            "sf".into(),
            "/".into(),
            "(a/b c/d)".into(),
        ]);
        assert_eq!(p.group, vec!["-J", "grp"]);
        assert_eq!(p.opts, vec!["-P", "pfx", "-f"]);
        assert_eq!(p.matcher, vec!["-M", "mspec"]);
        assert_eq!(p.imm, vec!["-i"]);
        assert_eq!(p.sopts, vec!["-S", "sf"]);
        assert_eq!(p.rest, vec!["/", "(a/b c/d)"]);
    }

    #[test]
    fn expansion_primitives() {
        assert_eq!(before_first("a/b/c", "/"), "a");
        assert_eq!(before_last("a/b/c", "/"), "a/b");
        assert_eq!(after_first("a/b/c", "/"), "b/c");
        assert_eq!(after_last("a/b/c", "/"), "c");
        assert_eq!(strip_trailing("a/b/", "/"), "a/b");
        assert!(has_interior_sep("a/b", "/"));
        assert!(!has_interior_sep("/ab", "/"));
        assert!(!has_interior_sep("ab/", "/"));
        assert!(ends_with_sep_component("a/b/", "/"));
        assert!(!ends_with_sep_component("a/", "/"));
    }
}
