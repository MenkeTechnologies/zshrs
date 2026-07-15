//! Port of `_arg_compile` from
//! `Completion/Base/Utility/_arg_compile` (zsh 5.9, 199 lines).
//!
//! A simple compiler for `_arguments` descriptions. The first argument
//! is the NAME of an array parameter into which the compiled parse is
//! written (upstream uses `${(P)}` / `eval $safe[reply]'=(…)'`). The
//! remaining arguments form a series of `phrases`, each beginning with
//! one of the keywords `argument`, `option`, or `help`.
//!
//! Upstream flow this port mirrors 1:1:
//! ```text
//! sh: 92  local -h argspec dspec helpspec prelude xor
//! sh: 93  local -h -A amap dmap safe
//! sh: 95  [[ -n "$1" ]] || return 1
//! sh: 96  [[ ${(tP)${1}} = *-local ]] && { NAME CONFLICT; return 1 }
//! sh: 97  safe[reply]="$1"; shift
//! sh:104-110  consume the prelude (anything before the first phrase)
//! sh:114-193  consume argument/option/help phrases, building argspec/helpspec
//! sh:195  eval $safe[reply]'=( prelude argspec [-- helpspec] "$@" )'
//! sh:199  return 0
//! ```

use crate::ported::params::{paramtab, setaparam};
use crate::ported::modules::parameter::paramtypestr;
use std::collections::HashMap;
use std::collections::VecDeque;

/// Successive `${2:s/join/-/:s/close/-/…}` HOW rewrite (sh:148).
/// Each `:s/A/B/` replaces the FIRST occurrence of `A` with `B`, in
/// order: join→`-`, close→`-`, next→``, split→``, loose→`+`,
/// assign→`=`, none→``.
fn rewrite_follow(how: &str) -> String {
    let mut s = how.to_string();
    for (from, to) in [
        ("join", "-"),
        ("close", "-"),
        ("next", ""),
        ("split", ""),
        ("loose", "+"),
        ("assign", "="),
        ("none", ""),
    ] {
        s = s.replacen(from, to, 1);
    }
    s
}

/// sh:126 — POS is `<1->` (an integer ≥ 1) or `*`.
fn is_position(s: &str) -> bool {
    if s == "*" {
        return true;
    }
    // `<1->` matches the numeric VALUE ≥ 1 (so "0" does not match).
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && s.parse::<u64>().map_or(false, |n| n >= 1)
}

/// `_arg_compile` — compile arg-specs into the caller-named array.
pub fn _arg_compile(args: &[String]) -> i32 {
    // sh:95 — `[[ -n "$1" ]] || return 1`.
    let reply = match args.first() {
        Some(r) if !r.is_empty() => r.clone(),
        _ => return 1,
    };

    // sh:96 — `[[ ${(tP)${1}} = *-local ]] && { NAME CONFLICT; return 1 }`.
    // `${(tP)${1}}` is the TYPE string of the param NAMED by $1; a
    // `*-local` type means the caller already declared it local and we
    // must not clobber it.
    {
        let tab = paramtab().read().unwrap();
        if let Some(pm) = tab.get(&reply) {
            if paramtypestr(pm).ends_with("-local") {
                eprintln!("NAME CONFLICT: {}", reply);
                return 1;
            }
        }
    }

    // sh:97 — `safe[reply]="$1"; shift`.
    let mut rest: VecDeque<String> = args[1..].iter().cloned().collect();

    // sh:92-93 locals.
    let mut argspec: Vec<String> = Vec::new();
    let mut helpspec: Vec<String> = Vec::new(); // sh:101
    let mut prelude: Vec<String> = Vec::new(); // sh:102

    // sh:104-110 — consume and save anything before the argument phrases.
    while let Some(head) = rest.front() {
        match head.as_str() {
            "argument" | "help" | "option" => break, // sh:107
            _ => {
                // sh:108 — prelude+=($1); shift
                prelude.push(rest.pop_front().unwrap());
            }
        }
    }

    // sh:114-193 — consume all phrases and build argspec/helpspec.
    while let Some(head) = rest.front().cloned() {
        // sh:116-117 — amap=(); dspec=()
        let mut amap: HashMap<String, String> = HashMap::new();
        let mut dspec: Vec<String> = Vec::new();

        match head.as_str() {
            // sh:121 — argument [POS] [means MSG] [action ACT]
            "argument" => {
                rest.pop_front(); // sh:122 shift
                while let Some(k) = rest.front().cloned() {
                    if is_position(&k) {
                        // sh:126 — amap[position]="$1"; shift
                        amap.insert("position".to_string(), k);
                        rest.pop_front();
                    } else if k == "means" || k == "action" {
                        // sh:127 — amap[$1]="$2"; shift 2
                        rest.pop_front();
                        let v = rest.pop_front().unwrap_or_default();
                        amap.insert(k, v);
                    } else if k == "argument" || k == "option" || k == "help" {
                        break; // sh:128
                    } else {
                        // sh:129
                        eprintln!("SYNTAX ERROR at {}", vecdeque_join(&rest));
                        return 1;
                    }
                }
                // sh:132-135
                if !amap.is_empty() {
                    argspec.push(format!(
                        "{}:{}:{}",
                        amap.get("position").cloned().unwrap_or_default(),
                        amap.get("means").cloned().unwrap_or_default(),
                        amap.get("action").cloned().unwrap_or_default(),
                    ));
                }
            }

            // sh:139 — option OPT [follow HOW] [explain STR] {unless XOR}
            //          {[through PAT] [means MSG] [action ACT]}
            "option" => {
                // sh:140 — amap[option]="$2"; shift 2
                rest.pop_front(); // "option"
                let opt = rest.pop_front().unwrap_or_default();
                amap.insert("option".to_string(), opt);
                let mut dmap: HashMap<String, String> = HashMap::new(); // sh:141
                let mut xor: Vec<String> = Vec::new(); // sh:142
                'opt_outer: while let Some(k) = rest.front().cloned() {
                    // sh:145 — (( ${+amap[$1]} || ${+dmap[through]} )) && break
                    if amap.contains_key(&k) || dmap.contains_key("through") {
                        break;
                    }
                    match k.as_str() {
                        "follow" => {
                            // sh:147-149
                            rest.pop_front();
                            let v = rest.pop_front().unwrap_or_default();
                            amap.insert("follow".to_string(), rewrite_follow(&v));
                        }
                        "explain" => {
                            // sh:150 — amap[explain]="[$2]"; shift 2
                            rest.pop_front();
                            let v = rest.pop_front().unwrap_or_default();
                            amap.insert("explain".to_string(), format!("[{}]", v));
                        }
                        "unless" => {
                            // sh:151 — xor+=("${(@)=2}"); shift 2
                            rest.pop_front();
                            let v = rest.pop_front().unwrap_or_default();
                            xor.extend(v.split_whitespace().map(|s| s.to_string()));
                        }
                        "through" | "means" | "action" => {
                            // sh:152-161 — inner loop collecting dmap entries
                            while let Some(k2) = rest.front().cloned() {
                                // sh:155 — (( ${+dmap[$1]} )) && break 2
                                // A repeated through/means/action key ends the
                                // WHOLE option phrase and, crucially, SKIPS the
                                // sh:165 dspec append for this iteration (the C
                                // `break 2` jumps past it).
                                if dmap.contains_key(&k2) {
                                    break 'opt_outer;
                                }
                                match k2.as_str() {
                                    "through" | "means" | "action" => {
                                        // sh:157 — dmap[$1]=":${2}"; shift 2
                                        rest.pop_front();
                                        let v = rest.pop_front().unwrap_or_default();
                                        dmap.insert(k2, format!(":{}", v));
                                    }
                                    "argument" | "option" | "help" | "follow" | "explain"
                                    | "unless" => {
                                        break; // sh:158 (break 1 — inner only)
                                    }
                                    _ => {
                                        // sh:159
                                        eprintln!("SYNTAX ERROR at {}", vecdeque_join(&rest));
                                        return 1;
                                    }
                                }
                            }
                        }
                        "argument" | "option" | "help" => break, // sh:162
                        _ => {
                            // sh:163
                            eprintln!("SYNTAX ERROR at {}", vecdeque_join(&rest));
                            return 1;
                        }
                    }
                    // sh:165-168 — if (( $#dmap )) dspec+=(through means|: action|:)
                    if !dmap.is_empty() {
                        dspec.push(format!(
                            "{}{}{}",
                            dmap.get("through").cloned().unwrap_or_default(),
                            // sh:167 — ${dmap[means]:-:} / ${dmap[action]:-:}
                            non_empty_or_colon(dmap.get("means")),
                            non_empty_or_colon(dmap.get("action")),
                        ));
                    }
                }
                // sh:170-173
                if !amap.is_empty() {
                    // sh:172 — "${xor:+($xor)}${amap[option]}${amap[follow]}
                    //           ${amap[explain]}${dspec}"
                    let xor_prefix = if xor.is_empty() {
                        String::new()
                    } else {
                        format!("({})", xor.join(" "))
                    };
                    argspec.push(format!(
                        "{}{}{}{}{}",
                        xor_prefix,
                        amap.get("option").cloned().unwrap_or_default(),
                        amap.get("follow").cloned().unwrap_or_default(),
                        amap.get("explain").cloned().unwrap_or_default(),
                        // `${dspec}` in double quotes joins the array with a space.
                        dspec.join(" "),
                    ));
                }
            }

            // sh:176 — help PAT [means MSG] action ACT
            "help" => {
                // sh:177 — amap[pattern]="$2"; shift 2
                rest.pop_front(); // "help"
                let pat = rest.pop_front().unwrap_or_default();
                amap.insert("pattern".to_string(), pat);
                while let Some(k) = rest.front().cloned() {
                    // sh:180 — (( ${+amap[$1]} )) && break
                    if amap.contains_key(&k) {
                        break;
                    }
                    match k.as_str() {
                        "means" | "action" => {
                            // sh:182 — amap[$1]="$2"; shift 2
                            rest.pop_front();
                            let v = rest.pop_front().unwrap_or_default();
                            amap.insert(k, v);
                        }
                        "argument" | "option" | "help" => break, // sh:183
                        _ => {
                            // sh:184
                            eprintln!("SYNTAX ERROR at {}", vecdeque_join(&rest));
                            return 1;
                        }
                    }
                }
                // sh:187-190
                if !amap.is_empty() {
                    helpspec.push(format!(
                        "{}:{}:{}",
                        amap.get("pattern").cloned().unwrap_or_default(),
                        amap.get("means").cloned().unwrap_or_default(),
                        amap.get("action").cloned().unwrap_or_default(),
                    ));
                }
            }

            // sh:191 — (*) break
            _ => break,
        }
    }

    // sh:195 — eval $safe[reply]'=( "${prelude[@]}" "${argspec[@]}"
    //           ${helpspec:+"-- ${helpspec[@]}"} "$@" )'
    let mut result: Vec<String> = Vec::new();
    result.extend(prelude);
    result.extend(argspec);
    if !helpspec.is_empty() {
        // `"-- ${helpspec[@]}"` glues the `-- ` prefix to the FIRST
        // helpspec element (zsh joins adjacent literal text to the
        // boundary element of an `[@]` expansion inside double quotes).
        let mut first = String::from("-- ");
        first.push_str(&helpspec[0]);
        result.push(first);
        result.extend(helpspec[1..].iter().cloned());
    }
    // Remaining `"$@"` (anything after the last phrase) passes through.
    result.extend(rest.into_iter());

    setaparam(&reply, result);

    0 // sh:199
}

/// sh:167 — `${dmap[key]:-:}` — the value if present and non-empty,
/// else a bare colon.
fn non_empty_or_colon(v: Option<&String>) -> String {
    match v {
        Some(s) if !s.is_empty() => s.clone(),
        _ => ":".to_string(),
    }
}

/// Render the remaining args for a `SYNTAX ERROR at "$@"` diagnostic.
fn vecdeque_join(rest: &VecDeque<String>) -> String {
    rest.iter().cloned().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(spec: &[&str]) -> Vec<String> {
        let args: Vec<String> = spec.iter().map(|s| s.to_string()).collect();
        let rc = _arg_compile(&args);
        assert_eq!(rc, 0, "compile should succeed");
        crate::ported::params::getaparam(spec[0]).unwrap_or_default()
    }

    #[test]
    fn empty_name_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_arg_compile(&[]), 1);
        assert_eq!(_arg_compile(&["".to_string()]), 1);
    }

    #[test]
    fn prelude_passes_through() {
        let _g = crate::test_util::global_state_lock();
        let out = compile(&["args", "-s", "-v"]);
        assert_eq!(out, vec!["-s", "-v"]);
    }

    #[test]
    fn argument_phrase_builds_pos_means_action() {
        let _g = crate::test_util::global_state_lock();
        // argument 1 means "file" action _files
        let out = compile(&[
            "args", "argument", "1", "means", "file", "action", "_files",
        ]);
        assert_eq!(out, vec!["1:file:_files"]);
    }

    #[test]
    fn option_phrase_with_follow_and_explain() {
        let _g = crate::test_util::global_state_lock();
        // option -d follow close means "debug level"
        // follow close => "-"; dspec entry = through("")+means(":debug level")
        //   +action(":") => ":debug level:"; argspec = "-d" + "-" + ":debug level:"
        let out = compile(&[
            "args", "option", "-d", "follow", "close", "means", "debug level",
        ]);
        assert_eq!(out, vec!["-d-:debug level:"]);
    }

    #[test]
    fn option_with_explain_and_unless_xor() {
        let _g = crate::test_util::global_state_lock();
        // option -a explain foo unless -b
        // -> "(-b)-a[foo]"
        let out = compile(&[
            "args", "option", "-a", "explain", "foo", "unless", "-b",
        ]);
        assert_eq!(out, vec!["(-b)-a[foo]"]);
    }

    #[test]
    fn help_phrase_and_trailing_marker() {
        let _g = crate::test_util::global_state_lock();
        // help '*=name*' means "function name" action '->funcs'
        let out = compile(&[
            "args",
            "help",
            "*=name*",
            "means",
            "function name",
            "action",
            "->funcs",
        ]);
        // helpspec => "*=name*:function name:->funcs", prefixed by "-- "
        assert_eq!(out, vec!["-- *=name*:function name:->funcs"]);
    }

    #[test]
    fn mixed_argument_then_help_phrase() {
        let _g = crate::test_util::global_state_lock();
        // argument means "profile file" action _files
        // help *=dirs* action _dir_list
        let out = compile(&[
            "args",
            "argument",
            "means",
            "profile file",
            "action",
            "_files",
            "help",
            "*=dirs*",
            "action",
            "_dir_list",
        ]);
        assert_eq!(
            out,
            vec![":profile file:_files", "-- *=dirs*::_dir_list"]
        );
    }

    #[test]
    fn syntax_error_returns_one() {
        let _g = crate::test_util::global_state_lock();
        // A bare non-keyword token inside a phrase is a syntax error
        // (sh:129/159/163/184) — here `bogus` after the argument phrase's
        // means/action slots.
        let rc = _arg_compile(&[
            "args".to_string(),
            "argument".to_string(),
            "means".to_string(),
            "m".to_string(),
            "bogus".to_string(),
        ]);
        assert_eq!(rc, 1);
    }
}
