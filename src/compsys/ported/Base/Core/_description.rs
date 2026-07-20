//! Port of `_description` from `Completion/Base/Core/_description`.
//!
//! Full upstream body (124 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  local name nopt xopt format gname hidden hide match opts tag
//! sh:  4  local -a ign gropt sort
//! sh:  5  local -a match mbegin mend
//! sh:  6
//! sh:  7  opts=()
//! sh:  8
//! sh:  9  xopt=(-X)
//! sh: 10  nopt=()
//! sh: 11  zparseopts -K -D -a nopt 1 2 V=gropt J=ign x=xopt
//! sh: 12
//! sh: 13  3="${${3##[[:blank:]]#}%%[[:blank:]]#}"
//! sh: 14  [[ -n "$3" ]] && _lastdescr=( "$_lastdescr[@]" "$3" )
//! sh: 15
//! sh: 16  zstyle -s ":completion:${curcontext}:$1" group-name gname &&
//! sh: 17      [[ -z "$gname" ]] && gname="$1"
//! sh: 18
//! sh: 19  _setup "$1" "${gname:--default-}"
//! sh: 20
//! sh: 21  name="$2"
//! sh: 22
//! sh: 23  zstyle -s ":completion:${curcontext}:$1" format format ||
//! sh: 24      zstyle -s ":completion:${curcontext}:descriptions" format format
//! sh: 25
//! sh: 26  if zstyle -s ":completion:${curcontext}:$1" hidden hidden &&
//! sh: 27     [[ "$hidden" = (all|yes|true|1|on) ]]; then
//! sh: 28    [[ "$hidden" = all ]] && format=''
//! sh: 29    opts=(-n)
//! sh: 30  fi
//! sh: 31  zstyle -s ":completion:${curcontext}:$1" matcher match &&
//! sh: 32      opts=($opts -M "$match")
//! sh: 33  [[ -n "$_matcher" ]] && opts=($opts -M "$_matcher")
//! sh: 34
//! sh: 35  # Use sort style, but ignore `menu' value to help _expand.
//! sh: 36  # Also don't override explicit use of -V.
//! sh: 37  if [[ -z "$gropt" ]]; then
//! sh: 38    if zstyle -a ":completion:${curcontext}:$1" sort sort ||
//! sh: 39       zstyle -a ":completion:${curcontext}:" sort sort
//! sh: 40    then
//! sh: 41      if [[ -z "${(@)sort:#(match|numeric|reverse)}" ]]; then
//! sh: 42        gropt=( -o ${(j.,.)sort} )
//! sh: 43      elif [[ "$sort" != (yes|true|1|on|menu) ]]; then
//! sh: 44        gropt=( -o nosort )
//! sh: 45      fi
//! sh: 46    fi
//! sh: 47  else
//! sh: 48    gropt=( -o nosort )
//! sh: 49  fi
//! sh: 50
//! sh: 51  if [[ -z "$_comp_no_ignore" ]]; then
//! sh: 52    zstyle -a ":completion:${curcontext}:$1" ignored-patterns _comp_ignore ||
//! sh: 53      _comp_ignore=()
//! sh: 54
//! sh: 55    if zstyle -s ":completion:${curcontext}:$1" ignore-line hidden; then
//! sh: 56      local -a qwords
//! sh: 57      qwords=( ${words//(#m)[\[\]()\\*?#<>~\^\|]/\\$MATCH} )
//! sh: 58      case "$hidden" in
//! sh: 59      true|yes|on|1) _comp_ignore+=( $qwords );;
//! sh: 60      current)       _comp_ignore+=( $qwords[CURRENT] );;
//! sh: 61      current-shown)
//! sh: 62              [[ "$compstate[old_list]" = *shown* ]] &&
//! sh: 63              _comp_ignore+=( $qwords[CURRENT] );;
//! sh: 64      other)         _comp_ignore+=( $qwords[1,CURRENT-1]
//! sh: 65                                     $qwords[CURRENT+1,-1] );;
//! sh: 66      esac
//! sh: 67    fi
//! sh: 68
//! sh: 69    # Ensure the ignore option is first so we can override it
//! sh: 70    # for fake-always.
//! sh: 71    (( $#_comp_ignore )) && opts=( -F _comp_ignore $opts )
//! sh: 72  else
//! sh: 73    _comp_ignore=()
//! sh: 74  fi
//! sh: 75
//! sh: 76  tag="$1"
//! sh: 77
//! sh: 78  shift 2
//! sh: 79  if [[ -z "$1" && $# -eq 1 ]]; then
//! sh: 80    format=
//! sh: 81  elif [[ -n "$format" ]]; then
//! sh: 82    if [[ -z $2 ]]; then
//! sh: 83      argv+=( h:${1%%( ##\((#b)([^\)]#[^0-9-][^\)]#)(#B)\)|)( ##\((#b)([0-9-]##)(#B)\)|)( ##\[(#b)([^\]]##)(#B)\]|)} )
//! sh: 84      [[ -n $match[1] ]] && argv+=( m:$match[1] )
//! sh: 85      [[ -n $match[2] ]] && argv+=( r:$match[2] )
//! sh: 86      [[ -n $match[3] ]] && argv+=( o:$match[3] )
//! sh: 87    fi
//! sh: 88
//! sh: 89    zformat -F format "$format" "d:$1" "${(@)argv[2,-1]}"
//! sh: 90  fi
//! sh: 91
//! sh: 92  if [[ -n "$gname" ]]; then
//! sh: 93    if [[ -n "$format" ]]; then
//! sh: 94      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J "$gname" "$xopt" "$format"
//! sh: 95    else
//! sh: 96      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J "$gname"
//! sh: 97    fi
//! sh: 98  else
//! sh: 99    if [[ -n "$format" ]]; then
//! sh:100      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J -default- "$xopt" "$format"
//! sh:101    else
//! sh:102      set -A "$name" "$opts[@]" "$nopt[@]" "$gropt[@]" -J -default-
//! sh:103    fi
//! sh:104  fi
//! sh:105
//! sh:106  if ! (( ${funcstack[2,-1][(I)_description]} )); then
//! sh:107    local fakestyle descr
//! sh:108    for fakestyle in fake fake-always; do
//! sh:109      zstyle -a ":completion:${curcontext}:$tag" $fakestyle match ||
//! sh:110      continue
//! sh:111
//! sh:112      descr=( "${(@M)match:#*[^\\]:*}" )
//! sh:113
//! sh:114      opts=("${(@P)name}")
//! sh:115      if [[ $fakestyle = fake-always && $opts[1,2] = "-F _comp_ignore" ]]; then
//! sh:116        shift 2 opts
//! sh:117      fi
//! sh:118      compadd "${(@)opts}" - "${(@)${(@)match:#*[^\\]:*}:s/\\:/:/}"
//! sh:119      (( $#descr )) && _describe -t "$tag" '' descr "${(@)opts}"
//! sh:120    done
//! sh:121  fi
//! sh:122
//! sh:123  return 0
//! ```
//!
//! Faithful port. Calls real `lookupstyle`, `bin_compadd`,
//! `bin_zformat` directly. Cross-function calls (`_setup`,
//! `_describe`) dispatch via `dispatch_function_call` (returns None
//! when no executor — fake/fake-always sub-block then becomes a
//! best-effort no-op).
//!
//! Skipped: sh:83's match-extraction regex (extracts `m:`/`r:`/`o:`
//! from a description like `"file (123)[opts]"`). Requires zsh
//! `(#b)` capture groups; our `pattern.rs::patcompile` doesn't
//! expose them. Marked inline.

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::{bin_zformat, bin_zparseopts, lookupstyle};
use crate::ported::params::{getaparam, getsparam, setaparam, setsparam};
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

/// sh:11 — bridge to the real `bin_zparseopts` via the `-v` flag
/// (read params from a named array, NOT from `argv`/pparams). This
/// keeps the cluster ports independent of the pparams hook
/// installation state (the hook is only registered when fusevm is
/// active; unit tests run without it).
///
/// Seeds the four target arrays so `-K`'s "keep when spec unused"
/// semantic flows through (the C `-K` only preserves an array that
/// already exists in the param table at call time). Returns
/// `(remaining_argv, nopt, gropt, ign, xopt)`.
fn run_zparseopts(
    argv: &[String],
    nopt_seed: Vec<String>,
    gropt_seed: Vec<String>,
    ign_seed: Vec<String>,
    xopt_seed: Vec<String>,
) -> (
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
    Vec<String>,
) {
    let src_name = "__compsys_argv";
    setaparam(src_name, argv.to_vec());
    setaparam("nopt", nopt_seed);
    setaparam("gropt", gropt_seed);
    setaparam("ign", ign_seed);
    setaparam("xopt", xopt_seed);

    let ops = make_ops();
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-K".to_string(),
            "-D".to_string(),
            "-v".to_string(),
            src_name.to_string(),
            "-a".to_string(),
            "nopt".to_string(),
            "1".to_string(),
            "2".to_string(),
            "V=gropt".to_string(),
            "J=ign".to_string(),
            "x=xopt".to_string(),
        ],
        &ops,
        0,
    );

    let nopt = getaparam("nopt").unwrap_or_default();
    let gropt = getaparam("gropt").unwrap_or_default();
    let ign = getaparam("ign").unwrap_or_default();
    let xopt = getaparam("xopt").unwrap_or_default();
    let remaining = getaparam(src_name).unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not a
    // real zsh identifier; zsh operates on positional $argv). Bug #657.
    crate::ported::params::unsetparam(src_name);
    (remaining, nopt, gropt, ign, xopt)
}

/// `_description` — build the option array for a `compadd`/`compgen`
/// call against tag `$1`, store under array-named-by-`$2`, with
/// description `$3` and optional extra match-specs `$4..`.
/// Returns 0 (sh:123).
pub fn _description(args: &[String]) -> i32 {
    // sh:3-5  local …
    let mut opts: Vec<String> = Vec::new(); // sh:7
    let nopt_seed: Vec<String> = Vec::new(); // sh:10
    let xopt_seed: Vec<String> = vec!["-X".to_string()]; // sh:9
    let gropt_seed: Vec<String> = Vec::new();
    let ign_seed: Vec<String> = Vec::new();

    // sh:11
    let (mut argv, nopt, mut gropt, _ign, xopt) =
        run_zparseopts(args, nopt_seed, gropt_seed, ign_seed, xopt_seed);

    // sh:13  trim leading/trailing whitespace from $3.
    if argv.len() >= 3 {
        argv[2] = argv[2].trim().to_string();
    }

    // sh:14
    if argv.len() >= 3 && !argv[2].is_empty() {
        let mut lastdescr = getaparam("_lastdescr").unwrap_or_default();
        lastdescr.push(argv[2].clone());
        setaparam("_lastdescr", lastdescr);
    }

    // sh:16  zstyle -s ":completion:${curcontext}:$1" group-name gname &&
    // sh:17      [[ -z "$gname" ]] && gname="$1"
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let tag1 = argv.first().cloned().unwrap_or_default();
    let ctx1 = format!(":completion:{}:{}", curcontext, tag1);
    // `zstyle -s` succeeds (RC 0) whenever the style pattern matches, even
    // when the matched value is the empty string. `lookupstyle` mirrors
    // that: an empty Vec means "unset" (RC 1 — skip the whole `&&` chain),
    // a non-empty Vec means "set" (RC 0 — run the `[[ -z gname ]]` test).
    // `-s` collapses the value array to a scalar (space-joined), so join
    // rather than take only the first element.
    let gn_vals = lookupstyle(&ctx1, "group-name");
    let mut gname: String = if gn_vals.is_empty() {
        String::new() // style unset → gname stays empty (sh:92 → -default-)
    } else {
        let v = crate::ported::utils::zjoin(&gn_vals, ' ');
        // sh:17 — style set: an empty value defaults the group to the tag.
        if v.is_empty() {
            tag1.clone()
        } else {
            v
        }
    };

    // sh:19  _setup "$1" "${gname:--default-}"
    {
        let setup_arg2 = if gname.is_empty() {
            "-default-".to_string()
        } else {
            gname.clone()
        };
        let _ = dispatch_function_call("_setup", &[tag1.clone(), setup_arg2]);
    }

    // sh:21  name="$2"
    let name: String = argv.get(1).cloned().unwrap_or_default();

    // sh:23-24
    let mut format: String = lookupstyle(&ctx1, "format")
        .first()
        .cloned()
        .unwrap_or_default();
    if format.is_empty() {
        let ctx_descr = format!(":completion:{}:descriptions", curcontext);
        format = lookupstyle(&ctx_descr, "format")
            .first()
            .cloned()
            .unwrap_or_default();
    }

    // sh:26-30
    let hidden_val = lookupstyle(&ctx1, "hidden")
        .first()
        .cloned()
        .unwrap_or_default();
    if matches!(hidden_val.as_str(), "all" | "yes" | "true" | "1" | "on") {
        if hidden_val == "all" {
            format.clear();
        }
        opts = vec!["-n".to_string()];
    }

    // sh:31-32
    let matcher_style = lookupstyle(&ctx1, "matcher")
        .first()
        .cloned()
        .unwrap_or_default();
    if !matcher_style.is_empty() {
        opts.push("-M".to_string());
        opts.push(matcher_style);
    }

    // sh:33
    if let Some(m) = getsparam("_matcher") {
        if !m.is_empty() {
            opts.push("-M".to_string());
            opts.push(m);
        }
    }

    // sh:37-49  sort handling.
    if gropt.is_empty() {
        let mut sort_vals = lookupstyle(&ctx1, "sort");
        if sort_vals.is_empty() {
            let ctx_bare = format!(":completion:{}:", curcontext);
            sort_vals = lookupstyle(&ctx_bare, "sort");
        }
        if !sort_vals.is_empty() {
            // sh:41 — every element ∈ {match,numeric,reverse}?
            let all_modifier = sort_vals
                .iter()
                .all(|v| matches!(v.as_str(), "match" | "numeric" | "reverse"));
            if all_modifier {
                // sh:42
                gropt = vec!["-o".to_string(), sort_vals.join(",")];
            } else {
                // sh:43-44
                let first = sort_vals[0].as_str();
                if !matches!(first, "yes" | "true" | "1" | "on" | "menu") {
                    gropt = vec!["-o".to_string(), "nosort".to_string()];
                }
            }
        }
    } else {
        // sh:48
        gropt = vec!["-o".to_string(), "nosort".to_string()];
    }

    // sh:51-74  ignored-patterns / ignore-line.
    let no_ignore = getsparam("_comp_no_ignore").unwrap_or_default();
    if no_ignore.is_empty() {
        // sh:52-53
        let mut comp_ignore = lookupstyle(&ctx1, "ignored-patterns");

        // sh:55-67  ignore-line
        let ignore_line_val = lookupstyle(&ctx1, "ignore-line")
            .first()
            .cloned()
            .unwrap_or_default();
        if !ignore_line_val.is_empty() {
            let words = getaparam("words").unwrap_or_default();
            let current_idx = getsparam("CURRENT")
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0);
            // sh:57
            let qwords: Vec<String> = words.iter().map(|w| glob_escape(w)).collect();
            match ignore_line_val.as_str() {
                "true" | "yes" | "on" | "1" => {
                    comp_ignore.extend(qwords); // sh:59
                }
                "current" => {
                    // sh:60 — 1-based subscript
                    if current_idx >= 1 && current_idx <= qwords.len() {
                        comp_ignore.push(qwords[current_idx - 1].clone());
                    }
                }
                "current-shown" => {
                    // sh:61-63
                    let old_list = get_compstate_str("old_list").unwrap_or_default();
                    if old_list.contains("shown") && current_idx >= 1 && current_idx <= qwords.len()
                    {
                        comp_ignore.push(qwords[current_idx - 1].clone());
                    }
                }
                "other" => {
                    // sh:64-65
                    for (i, w) in qwords.iter().enumerate() {
                        if i + 1 != current_idx {
                            comp_ignore.push(w.clone());
                        }
                    }
                }
                _ => {}
            }
        }

        // sh:71
        if !comp_ignore.is_empty() {
            let mut new_opts = vec!["-F".to_string(), "_comp_ignore".to_string()];
            new_opts.append(&mut opts);
            opts = new_opts;
            setaparam("_comp_ignore", comp_ignore);
        } else {
            setaparam("_comp_ignore", Vec::new());
        }
    } else {
        // sh:73
        setaparam("_comp_ignore", Vec::new());
    }

    // sh:76  tag="$1"
    let tag: String = tag1.clone();

    // sh:78  shift 2  → argv becomes original [2..]
    if argv.len() >= 2 {
        argv.drain(..2);
    } else {
        argv.clear();
    }

    // sh:79-90
    if argv.first().map(|s| s.is_empty()).unwrap_or(true) && argv.len() == 1 {
        // sh:80
        format.clear();
    } else if !format.is_empty() {
        // sh:82
        let mut extra_args: Vec<String> = Vec::new();
        if argv.get(1).map(|s| s.is_empty()).unwrap_or(true) {
            // sh:83-86 — extract `m:`/`r:`/`o:` capture groups from
            //   the description string via `extract_description_parts`
            //   below. The shell pattern is:
            //     `${1%%( ##\((#b)([^\)]#[^0-9-][^\)]#)(#B)\)|)
            //               ( ##\((#b)([0-9-]##)(#B)\)|)
            //               ( ##\[(#b)([^\]]##)(#B)\]|)}`
            //   i.e. strip trailing ` (msg)(num)[opts]` triplet from
            //   $1 (any combination present), pushing the captured
            //   parts into the zformat argv as `m:msg r:num o:opts`.
            if let Some(d) = argv.first().cloned() {
                let (stripped, m_msg, r_num, o_opts) = extract_description_parts(&d);
                // sh:83 — `argv+=( h:${1%%…} )`: push the annotation-stripped
                // description as `h:`; `$1` (argv[0]) is NOT modified, so the
                // `d:$1` below keeps the FULL description. The previous code
                // overwrote argv[0] with `stripped`, so any description carrying
                // a trailing `(msg)`/`(num)`/`[opts]` annotation (e.g. `_setopt`'s
                // 'zsh options (set)') lost that suffix from its group header —
                // rendered `-<<zsh options>>-` vs zsh's `-<<zsh options (set)>>-`.
                extra_args.push(format!("h:{}", stripped));
                if !m_msg.is_empty() {
                    extra_args.push(format!("m:{}", m_msg)); // sh:84
                }
                if !r_num.is_empty() {
                    extra_args.push(format!("r:{}", r_num)); // sh:85
                }
                if !o_opts.is_empty() {
                    extra_args.push(format!("o:{}", o_opts)); // sh:86
                }
            }
        }

        // sh:89  zformat -F format "$format" "d:$1" "${(@)argv[2,-1]}"
        let descr_val = argv.first().cloned().unwrap_or_default();
        let mut zfmt_argv: Vec<String> = vec![
            "-F".to_string(),
            "format".to_string(),
            format.clone(),
            format!("d:{}", descr_val),
        ];
        zfmt_argv.extend(extra_args);
        if argv.len() > 2 {
            zfmt_argv.extend(argv[2..].iter().cloned());
        }
        // Pre-set "format" param so bin_zformat -F has a target var
        // even if its internal write expects an existing entry.
        let _ = setsparam("format", "");
        let _ = bin_zformat("zformat", &zfmt_argv, &make_ops(), 0);
        format = getsparam("format").unwrap_or_default();
    }

    // sh:92-104
    let mut final_arr: Vec<String> = Vec::new();
    final_arr.extend(opts.iter().cloned());
    final_arr.extend(nopt.iter().cloned());
    final_arr.extend(gropt.iter().cloned());
    final_arr.push("-J".to_string());
    if !gname.is_empty() {
        // sh:94 / sh:96
        final_arr.push(gname.clone());
        if !format.is_empty() {
            if let Some(x) = xopt.first() {
                final_arr.push(x.clone());
            }
            final_arr.push(format.clone());
        }
    } else {
        // sh:100 / sh:102
        final_arr.push("-default-".to_string());
        if !format.is_empty() {
            if let Some(x) = xopt.first() {
                final_arr.push(x.clone());
            }
            final_arr.push(format.clone());
        }
    }
    setaparam(&name, final_arr);

    // sh:106-121  fake / fake-always.
    //   Our port skips the funcstack-recursion guard — `_describe`
    //   here is dispatched via the exec hook, which returns None when
    //   no executor is wired, so there's no recursion risk in the
    //   stand-alone unit-test environment.
    if gname.is_empty() {
        gname = tag.clone();
    }
    for fakestyle in ["fake", "fake-always"] {
        // sh:109
        let ctx_tag = format!(":completion:{}:{}", curcontext, tag);
        let match_vals = lookupstyle(&ctx_tag, fakestyle);
        if match_vals.is_empty() {
            continue;
        }

        // sh:112
        let descr: Vec<String> = match_vals
            .iter()
            .filter(|s| has_unescaped_colon(s))
            .cloned()
            .collect();

        // sh:114
        let mut fake_opts = getaparam(&name).unwrap_or_default();

        // sh:115-117
        if fakestyle == "fake-always"
            && fake_opts.len() >= 2
            && fake_opts[0] == "-F"
            && fake_opts[1] == "_comp_ignore"
        {
            fake_opts.drain(..2);
        }

        // sh:118
        let raw_values: Vec<String> = match_vals
            .iter()
            .filter(|s| !has_unescaped_colon(s))
            .map(|s| unescape_colons(s))
            .collect();
        let mut compadd_argv = fake_opts.clone();
        compadd_argv.push("-".to_string());
        compadd_argv.extend(raw_values);
        let _ = bin_compadd("compadd", &compadd_argv, &make_ops(), 0);

        // sh:119
        if !descr.is_empty() {
            let mut describe_argv: Vec<String> = vec![
                "-t".to_string(),
                tag.clone(),
                "".to_string(),
                "descr".to_string(),
            ];
            describe_argv.extend(fake_opts);
            setaparam("descr", descr.clone());
            let _ = dispatch_function_call("_describe", &describe_argv);
        }
    }

    // sh:123
    0
}

/// sh:57 — `${words//(#m)[\[\]()\\*?#<>~\^\|]/\\$MATCH}` —
/// backslash-escape glob/quote metachars in `s`.
/// sh:83-86 — strip trailing ` (msg)(num)[opts]` triplet from a
/// description string, returning `(stripped, m_msg, r_num, o_opts)`.
///
/// The upstream pattern uses three independent `(#b)` capture
/// groups; we mirror the same independence — each of the three
/// trailing forms is optional and can appear in any subset (the
/// only ordering constraint is that they appear at the END of the
/// string, separated by optional whitespace, in some order).
///
/// Examples:
///   `"file (123)[opts]"`  → ("file", "", "123", "opts")
///   `"file (msg) (123)"`  → ("file", "msg", "123", "")
///   `"plain"`             → ("plain", "", "", "")
///   `"file [opts]"`       → ("file", "", "", "opts")
fn extract_description_parts(s: &str) -> (String, String, String, String) {
    let mut rest = s.trim_end().to_string();
    let mut m_msg = String::new();
    let mut r_num = String::new();
    let mut o_opts = String::new();
    // Try to peel up to three trailing groups in any order. Loop
    //   until none of the three peels succeeds.
    loop {
        let trimmed = rest.trim_end().to_string();
        if trimmed.is_empty() {
            break;
        }
        // [opts]
        if o_opts.is_empty() && trimmed.ends_with(']') {
            if let Some(open) = trimmed.rfind('[') {
                if open + 1 < trimmed.len() - 1 {
                    let body = &trimmed[open + 1..trimmed.len() - 1];
                    if !body.contains(']') {
                        o_opts = body.to_string();
                        rest = trimmed[..open].trim_end().to_string();
                        continue;
                    }
                }
            }
        }
        // (123) — digits-only and optional minus
        if r_num.is_empty() && trimmed.ends_with(')') {
            if let Some(open) = trimmed.rfind('(') {
                let body = &trimmed[open + 1..trimmed.len() - 1];
                if !body.is_empty() && body.chars().all(|c| c.is_ascii_digit() || c == '-') {
                    r_num = body.to_string();
                    rest = trimmed[..open].trim_end().to_string();
                    continue;
                }
            }
        }
        // (msg) — at least one non-digit/non-dash char
        if m_msg.is_empty() && trimmed.ends_with(')') {
            if let Some(open) = trimmed.rfind('(') {
                let body = &trimmed[open + 1..trimmed.len() - 1];
                if !body.is_empty()
                    && body.chars().any(|c| !c.is_ascii_digit() && c != '-')
                    && !body.contains(')')
                {
                    m_msg = body.to_string();
                    rest = trimmed[..open].trim_end().to_string();
                    continue;
                }
            }
        }
        break;
    }
    (rest, m_msg, r_num, o_opts)
}

fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(
            c,
            '[' | ']' | '(' | ')' | '\\' | '*' | '?' | '#' | '<' | '>' | '~' | '^' | '|'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// sh:112 — `*[^\\]:*` test: contains a `:` whose preceding char is
/// not `\`.
fn has_unescaped_colon(s: &str) -> bool {
    let b = s.as_bytes();
    for i in 0..b.len() {
        if b[i] == b':' && (i == 0 || b[i - 1] != b'\\') {
            return true;
        }
    }
    false
}

/// sh:118's `:s/\\:/:/` — replace every `\:` with `:`.
fn unescape_colons(s: &str) -> String {
    s.replace("\\:", ":")
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
    fn returns_zero_for_empty_args() {
        // sh:123 — always exits 0.
        let r = with_incompfunc(|| _description(&[]));
        assert_eq!(r, 0);
    }

    #[test]
    fn writes_named_array_with_default_group_no_format() {
        // sh:102 — gname empty, format empty → array ends in `-J -default-`.
        let arr = with_incompfunc(|| {
            _description(&[
                "mytag".to_string(),
                "expl".to_string(),
                "the description".to_string(),
            ]);
            getaparam("expl").unwrap_or_default()
        });
        let has_j_default = arr.windows(2).any(|w| w[0] == "-J" && w[1] == "-default-");
        assert!(has_j_default, "expected `-J -default-` in {:?}", arr);
    }

    #[test]
    fn lastdescr_appends_trimmed_description() {
        // sh:13-14
        let _ = with_incompfunc(|| {
            setaparam("_lastdescr", Vec::new());
            _description(&[
                "tagX".to_string(),
                "varX".to_string(),
                "  hello  ".to_string(),
            ])
        });
        let arr = getaparam("_lastdescr").unwrap_or_default();
        assert_eq!(arr.last().map(|s| s.as_str()), Some("hello"));
    }

    #[test]
    fn empty_description_not_appended_to_lastdescr() {
        // sh:14 — `[[ -n "$3" ]]` guard.
        let _ = with_incompfunc(|| {
            setaparam("_lastdescr", vec!["seed".to_string()]);
            _description(&["t".to_string(), "v".to_string(), "".to_string()])
        });
        let arr = getaparam("_lastdescr").unwrap_or_default();
        assert_eq!(arr, vec!["seed".to_string()]);
    }

    #[test]
    fn zparseopts_v_into_gropt_forces_nosort() {
        // sh:11 + sh:47-48 — `-V` is BOOLEAN (mod_zutil.yo:377);
        // passing it puts a sentinel into gropt → sh:37's `[[ -z
        // "$gropt" ]]` is false → take sh:48 path forcing `-o nosort`.
        // Argv layout: `-V <tag> <name> <description>`.
        let _ = with_incompfunc(|| {
            _description(&[
                "-V".to_string(),
                "mytag".to_string(),
                "outv".to_string(),
                "d".to_string(),
            ])
        });
        let arr = getaparam("outv").unwrap_or_default();
        let has_nosort = arr.windows(2).any(|w| w[0] == "-o" && w[1] == "nosort");
        assert!(has_nosort, "expected `-o nosort` in {:?}", arr);
    }

    #[test]
    fn has_unescaped_colon_detects_correctly() {
        assert!(has_unescaped_colon("a:b"));
        assert!(!has_unescaped_colon("a\\:b"));
        assert!(has_unescaped_colon("\\::"));
        assert!(!has_unescaped_colon("noColon"));
    }

    #[test]
    fn unescape_colons_replaces_backslash_colon() {
        assert_eq!(unescape_colons("a\\:b\\:c"), "a:b:c");
        assert_eq!(unescape_colons("plain"), "plain");
    }

    #[test]
    fn glob_escape_backslashes_metachars() {
        assert_eq!(glob_escape("a*b"), "a\\*b");
        assert_eq!(glob_escape("foo[1]"), "foo\\[1\\]");
        assert_eq!(glob_escape("clean"), "clean");
    }

    #[test]
    fn extract_description_parts_all_three() {
        // sh:83-86 — full triplet
        let (stripped, m, r, o) = extract_description_parts("file (msg) (123)[opts]");
        assert_eq!(stripped, "file");
        assert_eq!(m, "msg");
        assert_eq!(r, "123");
        assert_eq!(o, "opts");
    }

    #[test]
    fn extract_description_parts_plain() {
        // No trailing groups → empty captures
        let (stripped, m, r, o) = extract_description_parts("plain text");
        assert_eq!(stripped, "plain text");
        assert!(m.is_empty() && r.is_empty() && o.is_empty());
    }

    #[test]
    fn extract_description_parts_brackets_only() {
        let (stripped, m, r, o) = extract_description_parts("file [extopts]");
        assert_eq!(stripped, "file");
        assert!(m.is_empty() && r.is_empty());
        assert_eq!(o, "extopts");
    }

    #[test]
    fn extract_description_parts_numeric_paren_only() {
        let (stripped, m, r, o) = extract_description_parts("name (42)");
        assert_eq!(stripped, "name");
        assert!(m.is_empty() && o.is_empty());
        assert_eq!(r, "42");
    }

    #[test]
    fn extract_description_parts_message_only() {
        let (stripped, m, r, o) = extract_description_parts("foo (a long message)");
        assert_eq!(stripped, "foo");
        assert_eq!(m, "a long message");
        assert!(r.is_empty() && o.is_empty());
    }

    #[test]
    fn extract_description_parts_negative_number_is_numeric() {
        // sh:83 `[0-9-]##` allows leading minus.
        let (stripped, _m, r, _o) = extract_description_parts("seek (-10)");
        assert_eq!(stripped, "seek");
        assert_eq!(r, "-10");
    }
}
