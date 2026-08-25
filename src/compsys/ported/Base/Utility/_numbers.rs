//! Port of `_numbers` from `Completion/Base/Utility/_numbers`.
//!
//! Full upstream body (90 lines; sh:1-38 is the usage comment block):
//! ```text
//! sh:40  local MATCH MBEGIN MEND
//! sh:41  local desc tag range suffixes suffix suffixfmt pad pat='<->' partial=''
//! sh:42  local -a expl formats
//! sh:43  local -a default max min keep tags units
//! sh:47  zparseopts -K -D -A opts M+:=keep q+=keep s+:=keep S+:=keep J+: V+: 1 2 o+: n F: x+: X+: \
//! sh:48    t:=tags u:=units l:=min m:=max d:=default f=type e=type N=type
//! sh:50  desc="${1:-number}" tag="${tags[2]:-numbers}"
//! sh:51  (( $# )) && shift
//! sh:53  [[ -n ${(M)type:#-f} ]] && pat='(<->.[0-9]#|[0-9]#.<->|<->)' partial='(|.)'
//! sh:54  [[ -n ${(M)type:#-N} || $min[2] = -* || $max[2] = -* ]] && \
//! sh:55      pat="(|-)$pat" partial="(|-)$partial"
//! sh:57  if (( $#argv )) && compset -P "$pat"; then
//! sh:58    zstyle -s ":completion:${curcontext}:units" list-separator sep || sep=--
//! sh:59    _description -V units expl unit
//! sh:60    pad=${#${(O)${${argv%%:*}//?/.}}[1]} # length of longest suffix
//! sh:61    disp=( ${${argv#:}/(#m)[^:]##/${(pr<$pad>< >)MATCH}} ) # pad suffixes
//! sh:62    disp=( ${disp/:/ $sep } )
//! sh:63    compadd -M 'r:|/=* r:|=*' -d disp "$keep[@]" "$expl[@]" - ${${argv#:}%%:*}
//! sh:64    return
//! sh:65  elif [[ -prefix $~pat || $PREFIX = $~partial ]]; then
//! sh:66    formats=( "h:$desc" )
//! sh:67    (( $#units )) && formats+=( m:${units[2]} ) desc+=" ($units[2])"
//! sh:68    (( $#min )) && range="$min[2]-"
//! sh:69    (( $#max )) && range="${range:--}$max[2]"
//! sh:70    [[ -n $range ]] && formats+=( r:$range ) desc+=" ($range)"
//! sh:71    (( $#default )) && formats+=( o:${default[2]} ) desc+=" [$default[2]]"
//! sh:73    zstyle -s ":completion:${curcontext}:unit-suffixes" format suffixfmt || \
//! sh:74        suffixfmt='%(d.%U.)%x%(d.%u.)%(r..|)'
//! sh:75    for ((i=0;i<$#;i++)); do
//! sh:76      zformat -f suffix "$suffixfmt" "x:${${${argv[i+1]#:}%%:*}//\%/%%}" \
//! sh:77          "X:${${${argv[i+1]#:}#*:}//\%/%%}" "d:${#${argv[i+1]}[1]#:}" \
//! sh:78          i:i r:$(( $# - i - 1))
//! sh:79      suffixes+=$suffix
//! sh:80    done
//! sh:81    [[ -n $suffixes ]] && formats+=( x:$suffixes )
//! sh:83    _comp_mesg=yes
//! sh:84    _description -x $tag expl "$desc" $formats
//! sh:85    [[ $compstate[insert] = *unambiguous* ]] && compstate[insert]=
//! sh:86    compadd "$expl[@]"
//! sh:87    return 0
//! sh:88  fi
//! sh:90  return 1
//! ```
//!
//! Number completion with optional unit suffixes (e.g. `5s` / `200MB`).
//!
//! Two upstream locals are deliberately absent from sh:41-45 and are
//! therefore GLOBAL scratch in zsh too: `disp` (sh:61-63) and `sep`
//! (sh:58). The port keeps them as parameters under the same names so
//! `compadd -d disp` reads the array the shell would have read.
//!
//! The `sh:47` line differs between trees: `Completion/Base/Utility/_numbers`
//! in the checked-out zsh writes `q:=keep`, the shipped 5.9.2 copy that the
//! reference shell actually runs writes `q+=keep`. The port follows the
//! shipped form. Every other line, and every line NUMBER, is identical in
//! both trees.

use std::collections::HashMap;

use crate::compsys::ported::_description::_description;
use crate::ported::glob::{matchpat, shtokenize};
use crate::ported::modules::zutil::{bin_zparseopts, lookupstyle, zformat_substring};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam, unsetparam};
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};
use crate::ported::zle::complete::{bin_compadd, bin_compset, cond_psfix, CVT_PREPAT};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `${spec%%:*}` — the text before the FIRST colon (whole string when
/// there is none). An element written `:s:seconds` (leading colon = "this
/// is the default unit") therefore yields the empty string, which is what
/// makes sh:60's longest-suffix scan ignore it.
fn before_first_colon(s: &str) -> &str {
    match s.find(':') {
        Some(i) => &s[..i],
        None => s,
    }
}

/// `${spec#*:}` — the text after the FIRST colon, or the whole string
/// unchanged when there is no colon (C-shell `#` removes the shortest
/// matching prefix, and no match means no removal).
fn after_first_colon(s: &str) -> &str {
    match s.find(':') {
        Some(i) => &s[i + 1..],
        None => s,
    }
}

/// `zstyle -s CONTEXT STYLE var || var=DEFAULT` — `zstyle -s` succeeds
/// only when the style is set, joining its values with a space; on failure
/// the caller's `||` assignment wins.
fn style_or(context: &str, style: &str, default: &str) -> String {
    let vals = lookupstyle(context, style);
    if vals.is_empty() {
        default.to_string()
    } else {
        vals.join(" ")
    }
}

/// `_numbers` — numeric input completion with optional unit
/// suffixes.
pub fn _numbers(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_numbers");
    // sh:47-48  zparseopts. zshrs ports have no `$argv`, so the house
    // bridge passes the positional list through a scratch array named by
    // `-v` and lets `-D` strip the parsed options out of it. `-a opts_flat`
    // stands in for upstream's `-A opts`: the assoc is never read back
    // (nothing below sh:48 mentions `$opts`), and both spellings park the
    // options that carry no `=name` target out of the way.
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    for name in &[
        "opts_flat",
        "keep",
        "tags",
        "units",
        "min",
        "max",
        "default",
        "type",
    ] {
        // sh:42-43 — `local -a expl formats` / `local -a default max min
        // keep tags units` start every collector EMPTY, and `-K` (keep)
        // means zparseopts would otherwise append to whatever a previous
        // call left behind.
        setaparam(name, Vec::new());
    }
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-K".to_string(),
            "-D".to_string(),
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "opts_flat".to_string(),
            "M+:=keep".to_string(),
            "q+=keep".to_string(),
            "s+:=keep".to_string(),
            "S+:=keep".to_string(),
            "J+:".to_string(),
            "V+:".to_string(),
            "1".to_string(),
            "2".to_string(),
            "o+:".to_string(),
            "n".to_string(),
            "F:".to_string(),
            "x+:".to_string(),
            "X+:".to_string(),
            "t:=tags".to_string(),
            "u:=units".to_string(),
            "l:=min".to_string(),
            "m:=max".to_string(),
            "d:=default".to_string(),
            "f=type".to_string(),
            "e=type".to_string(),
            "N=type".to_string(),
        ],
        &make_ops(),
        0,
    );
    let positional = getaparam(src).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    unsetparam(src);
    let keep = getaparam("keep").unwrap_or_default();
    let tags = getaparam("tags").unwrap_or_default();
    let units = getaparam("units").unwrap_or_default();
    let min = getaparam("min").unwrap_or_default();
    let max = getaparam("max").unwrap_or_default();
    let default = getaparam("default").unwrap_or_default();
    let typ_arr = getaparam("type").unwrap_or_default();

    // sh:50  desc="${1:-number}" tag="${tags[2]:-numbers}"
    let mut desc = positional
        .first()
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "number".to_string());
    let tag = tags
        .get(1)
        .cloned()
        .unwrap_or_else(|| "numbers".to_string());
    // sh:51  (( $# )) && shift — everything after the description is a
    // unit-suffix spec, and `$#`/`$argv` from here on mean THAT list.
    let argv: Vec<String> = if positional.is_empty() {
        Vec::new()
    } else {
        positional[1..].to_vec()
    };

    // sh:53-55  build pat/partial
    let f_flag = typ_arr.iter().any(|t| t == "-f"); // sh:53 ${(M)type:#-f}
    let n_flag = typ_arr.iter().any(|t| t == "-N") // sh:54 ${(M)type:#-N}
        || min.get(1).map(|v| v.starts_with('-')).unwrap_or(false)
        || max.get(1).map(|v| v.starts_with('-')).unwrap_or(false);
    let mut pat = if f_flag {
        "(<->.[0-9]#|[0-9]#.<->|<->)".to_string() // sh:53
    } else {
        "<->".to_string() // sh:41
    };
    let mut partial = if f_flag {
        "(|.)".to_string() // sh:53
    } else {
        String::new() // sh:41
    };
    if n_flag {
        pat = format!("(|-){}", pat); // sh:55
        partial = format!("(|-){}", partial); // sh:55
    }

    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:57  if (( $#argv )) && compset -P "$pat"; then
    if !argv.is_empty()
        && bin_compset("compset", &["-P".to_string(), pat.clone()], &make_ops(), 0) == 0
    {
        // sh:58
        let sep = style_or(
            &format!(":completion:{}:units", curcontext),
            "list-separator",
            "--",
        );
        // sh:59  _description -V units expl unit
        let _ = _description(&[
            "-V".to_string(),
            "units".to_string(),
            "expl".to_string(),
            "unit".to_string(),
        ]);
        let expl = getaparam("expl").unwrap_or_default();
        // sh:60  pad = width of the longest suffix NAME. `${argv%%:*}`
        // maps each spec to its name, `//?/.` turns it into a run of
        // dots, `(O)` sorts descending and `[1]` takes the first — for
        // equal-character strings that is simply the longest.
        let pad = argv
            .iter()
            .map(|e| before_first_colon(e).chars().count())
            .max()
            .unwrap_or(0);
        // sh:61  strip the leading `:` default-unit marker, then right-pad
        //        the leading `[^:]##` run to `pad`.
        // sh:62  turn the first remaining `:` into ` $sep `.
        let disp: Vec<String> = argv
            .iter()
            .map(|e| {
                let e = e.strip_prefix(':').unwrap_or(e); // sh:61 ${argv#:}
                let name = before_first_colon(e);
                let padded = format!(
                    "{}{}",
                    name,
                    " ".repeat(pad.saturating_sub(name.chars().count()))
                );
                let rest = &e[name.len()..];
                match rest.strip_prefix(':') {
                    // sh:62
                    Some(tail) => format!("{} {} {}", padded, sep, tail),
                    None => padded,
                }
            })
            .collect();
        setaparam("disp", disp);
        // sh:63  compadd -M 'r:|/=* r:|=*' -d disp "$keep[@]" "$expl[@]" - ${${argv#:}%%:*}
        let mut cargs: Vec<String> = vec![
            "-M".to_string(),
            "r:|/=* r:|=*".to_string(),
            "-d".to_string(),
            "disp".to_string(),
        ];
        cargs.extend(keep);
        cargs.extend(expl);
        cargs.push("-".to_string());
        cargs.extend(
            argv.iter()
                .map(|e| before_first_colon(e.strip_prefix(':').unwrap_or(e)).to_string()),
        );
        // sh:64  return — the bare `return` propagates compadd's status.
        return bin_compadd("compadd", &cargs, &make_ops(), 0);
    }

    // sh:65  elif [[ -prefix $~pat || $PREFIX = $~partial ]]; then
    //
    // `$~` sets `globsubst` (c:Src/subst.c:2596 `case '~': globsubst = 2`),
    // whose only effect is c:Src/subst.c:4419-4420 `shtokenize(y)` — the
    // value's metachars become pattern-active. The condition operand then
    // reaches `cond_psfix` through `cond_str(a, 0, 1)` (c:Src/cond.c:525),
    // whose `raw` argument keeps those tokens. Hand `cond_psfix` the same
    // tokenized text.
    let prefix_matches = {
        let mut tokenized = pat.clone();
        shtokenize(&mut tokenized);
        cond_psfix(&[tokenized], CVT_PREPAT) != 0
    };
    let prefix = getsparam("PREFIX").unwrap_or_default();
    if !prefix_matches && !matchpat(&partial, &prefix, true, true) {
        // sh:88-90 — neither disjunct held, so `_numbers` contributed
        // nothing and the caller (`_arguments`' action, say) must be told
        // so. Returning 0 here published an EMPTY description group: with
        // the `descriptions` format style set, `gtimeout -<TAB>` grew a
        // `duration` header under the option list that zsh never shows.
        return 1;
    }

    // sh:66-71  formats + description suffixes. `formats` takes the BARE
    // `$desc`; the ` (units)` / ` (range)` / ` [default]` text is appended
    // to `desc` AFTERWARDS, so only the sh:84 `_description` sees it.
    let mut formats: Vec<String> = vec![format!("h:{}", desc)]; // sh:66
    if let Some(u) = units.get(1) {
        // sh:67 (( $#units ))
        formats.push(format!("m:{}", u));
        desc.push_str(&format!(" ({})", u));
    }
    let mut range = String::new();
    if let Some(m) = min.get(1) {
        range = format!("{}-", m); // sh:68
    }
    if let Some(m) = max.get(1) {
        // sh:69 — `${range:--}` is `$range`, or `-` when it is empty.
        if range.is_empty() {
            range.push('-');
        }
        range.push_str(m);
    }
    if !range.is_empty() {
        // sh:70
        formats.push(format!("r:{}", range));
        desc.push_str(&format!(" ({})", range));
    }
    if let Some(d) = default.get(1) {
        // sh:71
        formats.push(format!("o:{}", d));
        desc.push_str(&format!(" [{}]", d));
    }

    // sh:73-74
    let suffixfmt = style_or(
        &format!(":completion:{}:unit-suffixes", curcontext),
        "format",
        "%(d.%U.)%x%(d.%u.)%(r..|)",
    );
    // sh:75-80 — one `zformat -f` per unit suffix, concatenated. The
    // builtin's `-f` arm is c:Src/Modules/zutil.c:971-994: seed the spec
    // table with `%`→`%` and `)`→`)` (c:976-977), add each `c:value` pair
    // (c:987), then `zformat_substring` (c:990). Calling that directly is
    // the same computation without the `setsparam`/re-read round trip the
    // shell needs.
    let mut suffixes = String::new();
    let n = argv.len();
    for i in 0..n {
        let elem = &argv[i];
        let stripped = elem.strip_prefix(':').unwrap_or(elem); // ${argv[i+1]#:}
        let mut specs: HashMap<char, String> = HashMap::new();
        specs.insert('%', "%".to_string()); // c:zutil.c:976
        specs.insert(')', ")".to_string()); // c:zutil.c:977
        specs.insert('x', before_first_colon(stripped).replace('%', "%%")); // sh:76
        specs.insert('X', after_first_colon(stripped).replace('%', "%%")); // sh:77
                                                                           // sh:77 `d:${#${argv[i+1]}[1]#:}` — the length of the element's
                                                                           // FIRST CHARACTER after stripping a leading `:` from it: 0 for a
                                                                           // `:name:desc` default-unit spec, 1 otherwise. sh:74's default
                                                                           // format underlines the `%(d.…)` == 0 case.
        let d = match elem.chars().next() {
            None | Some(':') => 0,
            Some(_) => 1,
        };
        specs.insert('d', d.to_string());
        // sh:78 `i:i` — upstream passes the LETTER, not `$i`.
        specs.insert('i', "i".to_string());
        specs.insert('r', (n - i - 1).to_string()); // sh:78
        suffixes.push_str(&zformat_substring(&suffixfmt, &specs, false)); // sh:79
    }
    if !suffixes.is_empty() {
        // sh:81
        formats.push(format!("x:{}", suffixes));
    }

    // sh:83 — `_comp_mesg=yes` is a SCALAR; `_main_complete` tests it with
    // `[[ -n "$_comp_mesg" ]]` (Base/Core/_main_complete sh:220/350/353).
    let _ = setsparam("_comp_mesg", "yes");
    // sh:84  _description -x $tag expl "$desc" $formats
    let mut desc_argv: Vec<String> = vec!["-x".to_string(), tag, "expl".to_string(), desc];
    desc_argv.extend(formats);
    let _ = _description(&desc_argv);
    // sh:85
    let insert = get_compstate_str("insert").unwrap_or_default();
    if insert.contains("unambiguous") {
        set_compstate_str("insert", "");
    }
    // sh:86
    let expl = getaparam("expl").unwrap_or_default();
    let _ = bin_compadd("compadd", &expl, &make_ops(), 0);
    0 // sh:87
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_zero_for_simple_case() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _r = _numbers(&[]);
    }

    /// sh:60-62 — the display column is the suffix name padded to the
    /// widest name, then ` <sep> `, then the description. The leading `:`
    /// marker is stripped and does NOT count toward the width.
    #[test]
    fn unit_suffix_display_pads_to_longest_name() {
        let argv = [":s:seconds", "ms:milliseconds", "min:minutes"];
        let pad = argv
            .iter()
            .map(|e| before_first_colon(e).chars().count())
            .max()
            .unwrap();
        // `:s:seconds` contributes 0, so the widest name is `min`.
        assert_eq!(pad, 3);
        let rendered: Vec<String> = argv
            .iter()
            .map(|e| {
                let e = e.strip_prefix(':').unwrap_or(e);
                let name = before_first_colon(e);
                let padded = format!(
                    "{}{}",
                    name,
                    " ".repeat(pad.saturating_sub(name.chars().count()))
                );
                match e[name.len()..].strip_prefix(':') {
                    Some(tail) => format!("{} {} {}", padded, "--", tail),
                    None => padded,
                }
            })
            .collect();
        assert_eq!(
            rendered,
            vec![
                "s   -- seconds".to_string(),
                "ms  -- milliseconds".to_string(),
                "min -- minutes".to_string(),
            ]
        );
    }

    /// sh:76-79 with the sh:74 default format: the DEFAULT unit (the spec
    /// written with a leading `:`) is the one that gets `%U`/`%u`, because
    /// `%(d.…)` fires when `d` is 0, and every suffix but the last is
    /// followed by `|`.
    #[test]
    fn unit_suffix_format_underlines_default_and_bars_all_but_last() {
        let argv = [":s:seconds", "m:minutes", "h:hours"];
        let fmt = "%(d.%U.)%x%(d.%u.)%(r..|)";
        let n = argv.len();
        let mut out = String::new();
        for i in 0..n {
            let elem = argv[i];
            let stripped = elem.strip_prefix(':').unwrap_or(elem);
            let mut specs: HashMap<char, String> = HashMap::new();
            specs.insert('%', "%".to_string());
            specs.insert(')', ")".to_string());
            specs.insert('x', before_first_colon(stripped).to_string());
            specs.insert('X', after_first_colon(stripped).to_string());
            specs.insert(
                'd',
                match elem.chars().next() {
                    None | Some(':') => 0,
                    Some(_) => 1,
                }
                .to_string(),
            );
            specs.insert('i', "i".to_string());
            specs.insert('r', (n - i - 1).to_string());
            out.push_str(&zformat_substring(fmt, &specs, false));
        }
        assert_eq!(out, "%Us%u|m|h");
    }

    /// sh:77 `${…#*:}` leaves a colon-less spec untouched — the shortest
    /// prefix match fails, so nothing is removed.
    #[test]
    fn after_first_colon_keeps_a_colonless_spec() {
        assert_eq!(after_first_colon("bytes"), "bytes");
        assert_eq!(after_first_colon("s:seconds"), "seconds");
        assert_eq!(after_first_colon("a:b:c"), "b:c");
        assert_eq!(before_first_colon("bytes"), "bytes");
        assert_eq!(before_first_colon(":s:seconds"), "");
    }
}
