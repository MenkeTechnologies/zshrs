//! Port of `_brace_parameter` from
//! `Completion/Zsh/Context/_brace_parameter`.
//!
//! Full upstream body (214 lines), branch map:
//! ```text
//! sh:  1  #compdef -brace-parameter-
//! sh:  3  local char delim found_percent found_m exp ; local -a flags ; integer q_last n_q
//! sh:  7  if [[ $PREFIX = *'${('[^\)]# ]]  → parameter-flag char parser
//! sh:  9    compset -p 3                    # strip the `${(`
//! sh: 12    while [[ -n $PREFIX ]]; do char=$PREFIX[1]; compset -p 1; … done
//! sh:106    build $flags from found_percent / q_last / n_q / found_m + full catalog
//! sh:190    _describe -t flags "parameter flag" flags -Q -S '' ; return
//! sh:192  elif compset -P '*:([\|\*\^]|\^\^)'  → _arrays ; return
//! sh:195  elif compset -P '*:'                 → operator catalog + _history_modifiers p
//! sh:214  _parameters -e
//! ```
//!
//! Local names mirror the source: `char`, `delim`, `found_percent`,
//! `found_m`, `q_last`, `n_q`, `flags`. Sub-argument completion for
//! the `g I j s Z _ l r` flags delegates to `_globqual_delims` /
//! `_delimiters` / `_message` / `_describe` exactly as the source
//! does. The scratch `flags` by-name array is unset after each use.

use crate::compsys::ported::_arrays::_arrays;
use crate::compsys::ported::_delimiters::_delimiters;
use crate::compsys::ported::_describe::_describe;
use crate::compsys::ported::_globqual_delims::_globqual_delims;
use crate::compsys::ported::_history_modifiers::_history_modifiers;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_parameters::_parameters;
use crate::ported::params::{getsparam, setaparam, setsparam, unsetparam};
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

fn s(v: &str) -> String {
    v.to_string()
}

/// `$PREFIX[1]` — first char as a string (empty when PREFIX empty).
fn first_char() -> String {
    getsparam("PREFIX")
        .unwrap_or_default()
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default()
}

/// `_describe -t format 'format option' flags -Q -S ''` with the
/// `flags` scratch array pre-populated; unsets it afterwards.
fn describe_format(specs: &[&str]) -> i32 {
    setaparam("flags", specs.iter().map(|x| s(x)).collect());
    let r = _describe(&[
        s("-t"),
        s("format"),
        s("format option"),
        s("flags"),
        s("-Q"),
        s("-S"),
        s(""),
    ]);
    unsetparam("flags");
    r
}

/// `_brace_parameter` — complete inside `${…}`.
pub fn _brace_parameter() -> i32 {
    let prefix = getsparam("PREFIX").unwrap_or_default();

    // sh:7  if [[ $PREFIX = *'${('[^\)]# ]]  — inside an unterminated
    //   parameter-flag group `${(`.
    let in_flag_group = match prefix.rfind("${(") {
        Some(idx) => !prefix[idx + 3..].contains(')'),
        None => false,
    };

    if in_flag_group {
        // sh:9  compset -p 3  — strip the `${(`.
        let _ = bin_compset("compset", &[s("-p"), s("3")], &make_ops(), 0);

        // sh:3 integers / flags-seen state
        let mut q_last: i64 = 0;
        let mut n_q: i64 = 0;
        let mut found_percent = false;
        let mut found_m = false;

        // sh:12  while [[ -n $PREFIX ]]; do
        loop {
            let pfx = getsparam("PREFIX").unwrap_or_default();
            if pfx.is_empty() {
                break;
            }
            // sh:13  char=$PREFIX[1]
            let char = pfx.chars().next().unwrap();
            // sh:14  compset -p 1
            let _ = bin_compset("compset", &[s("-p"), s("1")], &make_ops(), 0);

            // sh:15-20  q run tracking
            if char == 'q' {
                q_last += 1;
                n_q += 1;
                continue;
            } else {
                q_last = 0;
            }

            // sh:22  case $char
            match char {
                // sh:23-25
                '%' => {
                    found_percent = true;
                }
                // sh:27-29
                'm' => {
                    found_m = true;
                }
                // sh:31  [gIjsZ_] — single delimited argument
                'g' | 'I' | 'j' | 's' | 'Z' | '_' => {
                    // sh:33
                    if getsparam("PREFIX").unwrap_or_default().is_empty() {
                        // sh:34  _delimiters qualifier-$char ; return
                        return _delimiters(&[format!("qualifier-{}", char)]);
                    } else if _globqual_delims() != 0 {
                        // sh:36  still completing argument
                        match char {
                            // sh:40-42
                            'g' => {
                                let _ = bin_compset("compset", &[s("-P"), s("*")], &make_ops(), 0);
                                return describe_format(&[
                                    "o:octal escapes",
                                    "c:expand ^X etc.",
                                    "e:expand \\M-t etc.",
                                ]);
                            }
                            // sh:45
                            'I' => {
                                return _message(&[s("integer expression")]);
                            }
                            // sh:49
                            'j' | 's' => {
                                return _message(&[s("separator")]);
                            }
                            // sh:53-60
                            'Z' => {
                                let _ = bin_compset("compset", &[s("-P"), s("*")], &make_ops(), 0);
                                return describe_format(&[
                                    "c:parse comments as strings (else as ordinary words)",
                                    "C:strip comments (else treat as ordinary words)",
                                    "n:treat newlines as whitespace",
                                ]);
                            }
                            // sh:63
                            '_' => {
                                return _message(&[s("no useful values")]);
                            }
                            _ => unreachable!(),
                        }
                    }
                    // sh:36 `_globqual_delims` matched (returned 0) → the
                    //   argument is complete; fall through and continue the
                    //   while loop onto the next flag char.
                }
                // sh:71  [lr] — one compulsory argument, two optional
                'l' | 'r' => {
                    // sh:73
                    if getsparam("PREFIX").unwrap_or_default().is_empty() {
                        // sh:74  _delimiters qualifier-$char ; return
                        return _delimiters(&[format!("qualifier-{}", char)]);
                    } else {
                        // sh:77  delim=$PREFIX[1]
                        let _ = setsparam("delim", &first_char());
                        // sh:78  if ! _globqual_delims
                        if _globqual_delims() != 0 {
                            // sh:80  _message "padding width" ; return
                            return _message(&[s("padding width")]);
                        }
                        // `_globqual_delims` has rewritten $delim to the
                        //   closing delimiter (dynamic scope in the shell).
                        // sh:88  if [[ $delim = $PREFIX[1] ]]
                        if getsparam("delim").unwrap_or_default() == first_char() {
                            // sh:90  second argument
                            if _globqual_delims() != 0 {
                                // sh:92  _message "repeated padding" ; return
                                return _message(&[s("repeated padding")]);
                            }
                            // sh:94  if [[ $delim = $PREFIX[1] ]]
                            if getsparam("delim").unwrap_or_default() == first_char() {
                                // sh:95  if ! _globqual_delims
                                if _globqual_delims() != 0 {
                                    // sh:97  _message "one-off padding" ; return
                                    return _message(&[s("one-off padding")]);
                                }
                            }
                        }
                    }
                }
                // No other char consumes an argument; continue.
                _ => {}
            }
        }

        // sh:106-141  base `%`/`q`/`Q`/`m` flags, sensitive to the
        //   flags already seen.
        let mut flags: Vec<String> = Vec::new();
        // sh:106-110
        if !found_percent {
            flags.push(s("%:expand prompt sequences"));
        } else {
            flags.push(s("%:expand prompts respecting options"));
        }
        // sh:111-133  case $q_last
        match q_last {
            0 => {
                // sh:113  if (( n_q == 0 ))
                if n_q == 0 {
                    flags.push(s("q:quote with backslashes"));
                }
            }
            1 => {
                flags.push(s("q:quote with single quotes"));
                flags.push(s("-:quote minimally for readability"));
                flags.push(s("+:quote like q-, plus $'...' for unprintable characters"));
            }
            2 => {
                flags.push(s("q:quote with double quotes"));
            }
            3 => {
                flags.push(s("q:quote with $'...'"));
            }
            _ => {}
        }
        // sh:134-136
        if n_q == 0 {
            flags.push(s("Q:remove one level of quoting"));
        }
        // sh:137-141
        if !found_m {
            flags.push(s("m:count multibyte width in padding calculation"));
        } else {
            flags.push(s(
                "m:count number of character code points in padding calculation",
            ));
        }

        // sh:142-189  full flag catalog
        for spec in [
            "#:interpret numeric expression as character code",
            "@:prevent double-quoted joining of arrays",
            "*:enable extended globs for pattern",
            "A:assign as an array parameter",
            "a:sort in array index order (with O to reverse)",
            "b:backslash quote pattern characters only",
            "c:count characters in an array (with ${(c)#...})",
            "C:capitalize words",
            "D:perform directory name abbreviation",
            "e:perform single-word shell expansions",
            "f:split the result on newlines",
            "F:join arrays with newlines",
            "g:process echo array sequences (needs options)",
            "i:sort case-insensitively",
            "k:substitute keys of associative arrays",
            "L:lower case all letters",
            "n:sort positive decimal integers numerically",
            "-:sort decimal integers numerically",
            "o:sort in ascending order (lexically if no other sort option)",
            "O:sort in descending order (lexically if no other sort option)",
            "P:use parameter value as name of parameter for redirected lookup",
            "t:substitute type of parameter",
            "u:substitute first occurrence of each unique word",
            "U:upper case all letters",
            "v:substitute values of associative arrays (with (k))",
            "V:visibility enhancements for special characters",
            "w:count words in array or string (with ${(w)#...})",
            "W:count words including empty words (with ${(W)#...})",
            "X:report parsing errors and eXit substitution",
            "z:split words as if zsh command line",
            "0:split words on null bytes",
            "p:handle print escapes or variables in parameter flag arguments",
            "~:treat strings in parameter flag arguments as patterns",
            "j:join arrays with specified string",
            "l:left-pad resulting words",
            "r:right-pad resulting words",
            "s:split words on specified string",
            "Z:split words as if zsh command line (with options)",
            // sh:181 `"_:extended flags, for future expansion"` is
            //   commented out in the source; kept out here too.
            "S:match non-greedy in /, // or search substrings in % and # expressions",
            "I:search <argument>th match in #, %, / expressions",
            "B:include index of beginning of match in #, % expressions",
            "E:include index of one past end of match in #, % expressions",
            "M:include matched portion in #, % expressions",
            "N:include length of match in #, % expressions",
            "R:include rest (unmatched portion) in #, % expressions",
        ] {
            flags.push(s(spec));
        }

        // sh:190  _describe -t flags "parameter flag" flags -Q -S ''
        setaparam("flags", flags);
        let r = _describe(&[
            s("-t"),
            s("flags"),
            s("parameter flag"),
            s("flags"),
            s("-Q"),
            s("-S"),
            s(""),
        ]);
        unsetparam("flags");
        // sh:191  return
        return r;
    }

    // sh:192  elif compset -P '*:([\|\*\^]|\^\^)'  → array names
    if bin_compset(
        "compset",
        &[s("-P"), s("*:([\\|\\*\\^]|\\^\\^)")],
        &make_ops(),
        0,
    ) == 0
    {
        // sh:193  _arrays ; return
        return _arrays(&[]);
    }

    // sh:195  elif compset -P '*:'  → operators + history modifiers
    if bin_compset("compset", &[s("-P"), s("*:")], &make_ops(), 0) == 0 {
        // sh:196-208  operator catalog
        setaparam(
            "flags",
            vec![
                s("-:substitute alternate value if parameter is null"),
                s("+:substitute alternate value if parameter is non-null"),
                s("=:substitute and assign alternate value if parameter is null"),
                s("\\:=:unconditionally assign value to parameter"),
                s("?:print error if parameter is null"),
                s("#:filter value matching pattern"),
                s("/:replace whole word matching pattern"),
                s("|:set difference"),
                s("*:set intersection"),
                s("^:zip arrays"),
                s("^^:zip arrays reusing values from shorter array"),
            ],
        );
        // sh:209  _describe -t flags "operator" flags -Q -S ''
        let _ = _describe(&[
            s("-t"),
            s("flags"),
            s("operator"),
            s("flags"),
            s("-Q"),
            s("-S"),
            s(""),
        ]);
        unsetparam("flags");
        // sh:210  _history_modifiers p ; return
        return _history_modifiers(&[s("p")]);
    }

    // sh:214  _parameters -e
    _parameters(&[s("-e")])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_name_falls_to_parameters() {
        // sh:214 — a bare `${myvar` (no `${(`, no `:`) drops to
        //   `_parameters -e`, which returns 1 without an executor.
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "myvar");
        assert_eq!(_brace_parameter(), 1);
    }

    #[test]
    fn in_flag_group_detection() {
        // sh:7 — `${(` with no closing `)` in the tail is the flag group.
        let mk = |p: &str| -> bool {
            match p.rfind("${(") {
                Some(idx) => !p[idx + 3..].contains(')'),
                None => false,
            }
        };
        assert!(mk("${("));
        assert!(mk("${(f"));
        assert!(mk("x${(oL"));
        assert!(!mk("${(f)")); // closed
        assert!(!mk("${(f)x")); // closed
        assert!(!mk("myvar")); // no flag group
    }

    #[test]
    fn first_char_reads_prefix_head() {
        let _g = crate::test_util::global_state_lock();
        let _ = crate::ported::params::setsparam("PREFIX", "abc");
        assert_eq!(first_char(), "a");
        let _ = crate::ported::params::setsparam("PREFIX", "");
        assert_eq!(first_char(), "");
    }
}
