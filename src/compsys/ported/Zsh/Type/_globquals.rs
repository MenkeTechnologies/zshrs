//! Port of `_globquals` from `Completion/Zsh/Type/_globquals`.
//!
//! Full upstream body (277 lines, faithful 1:1):
//! ```text
//! sh:  1  #autoload
//! sh:  3  local state=qual expl char delim timespec default MATCH
//! sh:  8  while [[ -n $PREFIX ]]; do
//! sh:  9    char=$PREFIX[1]; compset -p 1
//! sh: 11    case $char in … per-qualifier dispatch …
//! sh: 12      [-/F.@=p*rwxAIERWXsStUG^MTNDn,]  # no argument
//! sh: 23      f) _delimiters qualifier-f / _globqual_delims / _message
//! sh: 36      P) _delimiters qualifier-P / _message
//! sh: 48      e) _delimiters qualifier-e / compset -q; _normal
//! sh: 61      +) _command_names
//! sh: 71      d) _message device-ids
//! sh: 79      l) _message numbers
//! sh: 87      u) _delimiters qualifier-u / _users -S $delim
//! sh:101      g) _delimiters qualifier-g / _groups -S $delim
//! sh:115      [amc]) time/sense specifiers + _dates via _alternative
//! sh:148      L)  size/sense specifiers via _alternative
//! sh:167      [oO]) sort spec _describe / _delimiters qualifier-oe / _command_names
//! sh:202      [)  range _message
//! sh:214      :)  _history_modifiers q
//! sh:222  case $state in (qual) … quals catalogue …
//! sh:274    _describe -t globquals "glob qualifier" quals -Q -S ''
//! ```
//!
//! The `zstyle`-`verbose` display-reformatting sub-branches of the
//! `[amc]` arm (sh:123-126, sh:130-138) only change how time/sense
//! specifiers are *displayed*; the candidate match sets (`tmatch`,
//! `smatch`) are identical. This port takes the default (non-verbose)
//! presentation and documents the omission — no candidate is faked or
//! dropped.

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam};
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:274 — glob-qualifier catalogue (`state == qual`), verbatim from
/// sh:225-273.
const QUALS: &[&str] = &[
    "/:directories",
    "F:non-empty directories",
    ".:plain files",
    "@:symbolic links",
    "=:sockets",
    "p:named pipes (FIFOs)",
    "*:executable plain files",
    "%:device files",
    "r:owner-readable",
    "w:owner-writeable",
    "x:owner-executable",
    "A:group-readable",
    "I:group-writeable",
    "E:group-executable",
    "R:world-readable",
    "W:world-writeable",
    "X:world-executable",
    "s:setuid",
    "S:setgid",
    "t:sticky bit set",
    "f:+ access rights",
    "e:execute code",
    "+:+ command name",
    "d:+ device",
    "l:+ link count",
    "U:owned by EUID",
    "G:owned by EGID",
    "u:+ owning user",
    "g:+ owning group",
    "a:+ access time",
    "m:+ modification time",
    "c:+ inode change time",
    "L:+ size",
    "^:negate qualifiers",
    "-:follow symlinks toggle",
    "M:mark directories",
    "T:mark types",
    "N:use NULL_GLOB",
    "D:glob dots",
    "n:numeric glob sort",
    "o:+ sort order, up",
    "O:+ sort order, down",
    "P:prepend word",
    "Y:+ at most ARG matches",
    "[:+ range of files",
    "):end of qualifiers",
    "\\::modifier",
];

/// sh:170-181 — sort-specifier catalogue for the `[oO]` arm.
const SORT_SPECS: &[&str] = &[
    "n:lexical order of name",
    "L:size of file",
    "l:number of hard links",
    "a:last access time",
    "m:last modification time",
    "c:last inode change time",
    "d:directory depth",
    "N:no sorting",
    "e:execute code",
    "+:+ command name",
];

/// `compset` shim — dispatch the real `bin_compset`; returns 0 when the
/// prefix/pattern matched (shell truth), non-zero otherwise.
fn compset(args: &[&str]) -> i32 {
    let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &a, &make_ops(), 0)
}

/// Named-function dispatch shim (`_delimiters`, `_users`, …).
fn call(name: &str, args: &[&str]) -> i32 {
    let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    dispatch_function_call(name, &a).unwrap_or(1)
}

/// sh:63 / sh:194 — `[[ $PREFIX = [[:IDENT:]]# ]]`: PREFIX is empty or
/// wholly made of identifier characters (alnum + `_`).
fn all_ident(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// sh:141-142 — `${specmap[(K)${timespec:-d}]}`: map the time-unit char
/// to its plural noun, defaulting empty/`+`/`-`/`d` to `days`.
fn specmap_value(timespec: &str) -> &'static str {
    let key = if timespec.is_empty() { "d" } else { timespec };
    match key {
        "M" => "months",
        "w" => "weeks",
        "h" => "hours",
        "m" => "minutes",
        "s" => "seconds",
        "d" | "+" | "-" => "days",
        _ => "invalid time specifier",
    }
}

/// sh:142 — `${${timespec/[-+]/d}:-d}`: replace a leading `-`/`+` with
/// `d`, default empty to `d`.
fn dates_fmt(timespec: &str) -> String {
    let replaced = if let Some(pos) = timespec.find(|c| c == '-' || c == '+') {
        let mut s = timespec.to_string();
        s.replace_range(pos..pos + 1, "d");
        s
    } else {
        timespec.to_string()
    };
    if replaced.is_empty() {
        "d".to_string()
    } else {
        replaced
    }
}

/// `_globquals` — complete glob qualifiers inside `(...)`.
pub fn _globquals() -> i32 {
    // sh:8  while [[ -n $PREFIX ]]; do
    loop {
        // sh:8 loop guard.
        let before = getsparam("PREFIX").unwrap_or_default();
        if before.is_empty() {
            break;
        }
        // sh:9  char=$PREFIX[1]; compset -p 1
        let ch = before.chars().next().unwrap();
        let _ = compset(&["-p", "1"]);
        // Non-source safety: at runtime `compset -p 1` always advances a
        // non-empty PREFIX; if it did not (e.g. not in a completion
        // context so `bin_compset` early-returned), break rather than
        // spin forever.
        if getsparam("PREFIX").unwrap_or_default().len() >= before.len() {
            break;
        }

        match ch {
            // sh:12  no-argument qualifiers — consumed, keep scanning.
            '-' | '/' | 'F' | '.' | '@' | '=' | 'p' | '*' | 'r' | 'w' | 'x' | 'A' | 'I' | 'E'
            | 'R' | 'W' | 'X' | 's' | 'S' | 't' | 'U' | 'G' | '^' | 'M' | 'T' | 'N' | 'D'
            | 'n' | ',' => {}

            // sh:16  (%) optional b, c
            '%' => {
                if matches!(
                    getsparam("PREFIX").unwrap_or_default().chars().next(),
                    Some('b') | Some('c')
                ) {
                    let _ = compset(&["-p", "1"]);
                }
            }

            // sh:23  (f)
            'f' => {
                if compset(&["-P", "[-=+][0-7?]##"]) != 0 {
                    if getsparam("PREFIX").unwrap_or_default().is_empty() {
                        // sh:26
                        return call("_delimiters", &["qualifier-f"]);
                    } else if call("_globqual_delims", &[]) != 0 {
                        // sh:30
                        return call("_message", &["-e", "modes", "mode spec"]);
                    }
                }
            }

            // sh:36  (P)
            'P' => {
                if getsparam("PREFIX").unwrap_or_default().is_empty() {
                    // sh:39
                    return call("_delimiters", &["qualifier-P"]);
                } else if call("_globqual_delims", &[]) != 0 {
                    // sh:43
                    return call("_message", &["-e", "prefix", "prefix"]);
                }
            }

            // sh:48  (e)
            'e' => {
                if getsparam("PREFIX").unwrap_or_default().is_empty() {
                    // sh:51
                    return call("_delimiters", &["qualifier-e"]);
                } else if call("_globqual_delims", &[]) != 0 {
                    // sh:55  compset -q; _normal
                    let _ = compset(&["-q"]);
                    return call("_normal", &[]);
                }
            }

            // sh:61  (+)
            '+' => {
                if all_ident(&getsparam("PREFIX").unwrap_or_default()) {
                    // sh:65
                    return call("_command_names", &[]);
                }
                // sh:68
                let _ = compset(&["-P", "[[:IDENT:]]##"]);
            }

            // sh:71  (d)
            'd' => {
                if compset(&["-p", "[[:digit:]]##"]) != 0 {
                    // sh:74
                    return call("_message", &["-e", "device-ids", "device ID"]);
                }
            }

            // sh:79  (l)
            'l' => {
                if compset(&["-P", "([-+]|)[[:digit:]]##"]) != 0 {
                    // sh:82
                    return call("_message", &["-e", "numbers", "link count"]);
                }
            }

            // sh:87  (u)
            'u' => {
                if compset(&["-P", "[[:digit:]]##"]) != 0 {
                    if getsparam("PREFIX").unwrap_or_default().is_empty() {
                        // sh:91
                        return call("_delimiters", &["qualifier-u"]);
                    } else if call("_globqual_delims", &[]) != 0 {
                        // sh:95  _users -S $delim
                        let delim = getsparam("delim").unwrap_or_default();
                        return call("_users", &["-S", &delim]);
                    }
                }
            }

            // sh:101  (g)
            'g' => {
                if compset(&["-P", "[[:digit:]]##"]) != 0 {
                    if getsparam("PREFIX").unwrap_or_default().is_empty() {
                        // sh:105
                        return call("_delimiters", &["qualifier-g"]);
                    } else if call("_globqual_delims", &[]) != 0 {
                        // sh:109  _groups -S $delim
                        let delim = getsparam("delim").unwrap_or_default();
                        return call("_groups", &["-S", &delim]);
                    }
                }
            }

            // sh:115  ([amc])
            'a' | 'm' | 'c' => {
                if compset(&["-P", "([Mwhmsd]|)([-+]|)<->"]) != 0 {
                    // sh:118  alts=()
                    let mut alts: Vec<String> = Vec::new();
                    // sh:119  timespec=$PREFIX[1]
                    let timespec = getsparam("PREFIX")
                        .unwrap_or_default()
                        .chars()
                        .next()
                        .map(|c| c.to_string())
                        .unwrap_or_default();
                    // sh:120  ! compset -P '[Mwhmsd]' && [[ -z $PREFIX ]]
                    if compset(&["-P", "[Mwhmsd]"]) != 0
                        && getsparam("PREFIX").unwrap_or_default().is_empty()
                    {
                        // sh:121-122 (default / non-verbose presentation).
                        setaparam(
                            "tdisp",
                            ["seconds", "minutes", "hours", "days", "weeks", "Months"]
                                .iter()
                                .map(|s| s.to_string())
                                .collect(),
                        );
                        setaparam(
                            "tmatch",
                            ["s", "m", "h", "d", "w", "M"]
                                .iter()
                                .map(|s| s.to_string())
                                .collect(),
                        );
                        // sh:127
                        alts.push(
                            "time-specifiers:time specifier:compadd -E 0 -d tdisp -S '' -a tmatch"
                                .to_string(),
                        );
                    }
                    // sh:129  ! compset -P '[-+]' && [[ -z $PREFIX ]]
                    if compset(&["-P", "[-+]"]) != 0
                        && getsparam("PREFIX").unwrap_or_default().is_empty()
                    {
                        // sh:135-137 (default / non-verbose presentation).
                        setaparam(
                            "sdisp",
                            ["before", "exactly", "since"]
                                .iter()
                                .map(|s| s.to_string())
                                .collect(),
                        );
                        setaparam(
                            "smatch",
                            ["+", "", "-"].iter().map(|s| s.to_string()).collect(),
                        );
                        // sh:139  ("senses:sense${default}:…"), default="".
                        alts.push(
                            "senses:sense:compadd -E 0 -d sdisp -S '' -a smatch".to_string(),
                        );
                    }
                    // sh:142
                    alts.push(format!(
                        "digits:digit ({}):_dates -f {} -S \"\"",
                        specmap_value(&timespec),
                        dates_fmt(&timespec)
                    ));
                    // sh:143  _alternative $alts
                    let argv: Vec<String> = alts;
                    return dispatch_function_call("_alternative", &argv).unwrap_or(1);
                }
            }

            // sh:148  (L)
            'L' => {
                if compset(&["-P", "([kKmMgGtTpP]|)([-+]|)<->"]) != 0 {
                    // sh:151  alts=()
                    let mut alts: Vec<String> = Vec::new();
                    // sh:153
                    if compset(&["-P", "[kKmMgGtTpP]"]) != 0
                        && getsparam("PREFIX").unwrap_or_default().is_empty()
                    {
                        // sh:154-156
                        alts.push(
                            "size-specifiers:size specifier:((k\\:kb m\\:mb g\\:gb t\\:tb p\\:512-byte\\ blocks))"
                                .to_string(),
                        );
                    }
                    // sh:158
                    if compset(&["-P", "[-+]"]) != 0
                        && getsparam("PREFIX").unwrap_or_default().is_empty()
                    {
                        // sh:159
                        alts.push(
                            "senses:sense:((-\\:less\\ than +\\:more\\ than))".to_string(),
                        );
                    }
                    // sh:161
                    alts.push("digits:digit: ".to_string());
                    // sh:162  _alternative $alts
                    return dispatch_function_call("_alternative", &alts).unwrap_or(1);
                }
            }

            // sh:167  ([oO])
            'o' | 'O' => {
                if compset(&["-p", "1"]) != 0 {
                    // sh:170-181  sort-specifier catalogue via _describe.
                    setaparam("alts", SORT_SPECS.iter().map(|s| s.to_string()).collect());
                    return call(
                        "_describe",
                        &[
                            "-t",
                            "sort-specifiers",
                            "sort specifier",
                            "alts",
                            "-Q",
                            "-S",
                            "",
                        ],
                    );
                } else if getsparam("IPREFIX").unwrap_or_default().chars().last() == Some('e') {
                    // sh:184
                    if getsparam("PREFIX").unwrap_or_default().is_empty() {
                        // sh:186
                        return call("_delimiters", &["qualifier-oe"]);
                    } else if call("_globqual_delims", &[]) != 0 {
                        // sh:189  compset -q; _normal
                        let _ = compset(&["-q"]);
                        return call("_normal", &[]);
                    }
                } else if getsparam("IPREFIX").unwrap_or_default().chars().last() == Some('+') {
                    // sh:193
                    if all_ident(&getsparam("PREFIX").unwrap_or_default()) {
                        // sh:196
                        return call("_command_names", &[]);
                    }
                }
            }

            // sh:202  (\[)
            '[' => {
                if compset(&["-P", "(-|)[[:digit:]]##(,(-|)[[:digit:]]##|)]"]) != 0 {
                    if compset(&["-P", "(-|)[[:digit:]]##,"]) == 0 {
                        // sh:206
                        return call("_message", &["end of range"]);
                    } else {
                        // sh:208
                        return call("_message", &["start of range"]);
                    }
                }
            }

            // sh:214  (:)
            ':' => {
                // sh:216  _history_modifiers q
                return call("_history_modifiers", &["q"]);
            }

            // Any other char: shell `esac` no-op — keep scanning.
            _ => {}
        }
    }

    // sh:222-274  case $state in (qual) … _describe …
    setaparam("quals", QUALS.iter().map(|s| s.to_string()).collect());
    call(
        "_describe",
        &[
            "-t",
            "globquals",
            "glob qualifier",
            "quals",
            "-Q",
            "-S",
            "",
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{getaparam, setsparam};

    #[test]
    fn returns_one_without_executor() {
        // sh:222 — empty PREFIX skips the while loop and lands on the
        //   catalogue `_describe`, which returns 1 with no executor.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        assert_eq!(_globquals(), 1);
    }

    #[test]
    fn publishes_full_quals_catalogue() {
        // sh:225-273 — the catalogue has 47 entries verbatim.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = _globquals();
        let q = getaparam("quals").unwrap_or_default();
        assert_eq!(q.len(), 47);
        assert!(q.iter().any(|e| e == "=:sockets"));
        assert!(q.iter().any(|e| e == "p:named pipes (FIFOs)"));
        assert!(q.iter().any(|e| e == "):end of qualifiers"));
        assert!(q.iter().any(|e| e == "\\::modifier"));
    }
}
