//! Port of `_ps1234` from `Completion/Zsh/Type/_ps1234`.
//!
//! Full upstream body (175 lines verbatim):
//! ```text
//! sh:  1  #compdef -value-,PROMPT,-default- … (every prompt var)
//! sh:  3  local -a specs ccol
//! sh:  4  local expl grp cols bs suf pre changed=1 ret=1
//! sh:  5  local -A ansi
//! sh:  7  [[ -z $compstate[quote] ]] && bs='\'
//! sh: 11  while (( changed )); do    # strip complete %x specs, leave current
//! sh: 13    compset -P '%[DFK](\\|){[^}]#}' && changed=1   # %x{...}
//! sh: 14    compset -P '%[0-9-\\]#[^DFK(0-9-<>\\\[]' …      # normal formats
//! sh: 15    compset -P '%[0-9-\\]#(<…<|>…>|\[…\])' …        # truncations
//! sh: 16    compset -P '%[0-9-\\]#(\\|)\([0-9-]#[^0-9]?|[^%]' …  # ternary start
//! sh: 17    compset -P '[^%]##' …                          # sundry chars
//! sh: 19    [[ $PREFIX = %(-|)<->#[DFK]… ]] && compset -P '%[0-9\\-]#[DFK]' …
//! sh: 22  [[ $PREFIX = %(-|)<->[FK](#e) ]] && compset -P '*'
//! sh: 24  if compset -P '%[FK]'; then … ansi-colors / terminal-colors …
//! sh: 61  if   compset -P '…\([0-9-]#[^0-9]'  → _delimiters (ternary delim)
//! sh: 65  elif compset -P '%[0-9-\\]#[<>\]]'  → _message replacements
//! sh: 68  elif compset -P '…\([0-9-]#'        → _describe ternary test char
//! sh: 98  elif compset -P '%D(\\|){'          → _date_formats zsh
//! sh:101  elif [[ -prefix % ]] || ! zstyle -t … prefix-needed → _describe specs
//! sh:174  return ret
//! ```
//!
//! `$PROMPT` / `$PS1`-style prompt-escape completer. The `while`
//! loop strips every already-complete `%x` spec so the remaining
//! `%…` fragment can be completed contextually: `%F`/`%K` colour
//! names or terminal-colour numbers, ternary-conditional delimiters
//! and test characters, `%D{…}` strftime formats, or the general
//! `%x` format-specifier catalogue.

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::testforstyle;
use crate::ported::params::{getaparam, getsparam, setaparam, sethparam};
use crate::ported::zle::complete::{bin_compadd, bin_compset, cond_psfix};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `compset FLAG PAT` → true when the strip matched (shell `compset`
/// returns 0). Mutates the global PREFIX/SUFFIX completion state.
fn compset(argv: &[&str]) -> bool {
    let owned: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &owned, &make_ops(), 0) == 0
}
fn compset_p(pat: &str) -> bool {
    compset(&["-P", pat])
}
fn compset_s(pat: &str) -> bool {
    compset(&["-S", pat])
}

/// `[[ $PREFIX = pat ]]` — zsh extended-glob match of the current
/// `$PREFIX` against `pat`.
fn prefix_matches(pat: &str) -> bool {
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let mut tok = pat.to_string();
    crate::ported::glob::tokenize(&mut tok);
    match crate::ported::pattern::patcompile(&tok, 0, None) {
        Some(prog) => crate::ported::pattern::pattry(&prog, &prefix),
        None => false,
    }
}

/// Dispatch a compsys helper; `None` (no executor) collapses to a
/// non-zero (failure) status like a shell call that returned 1.
fn dfc(name: &str, args: &[&str]) -> i32 {
    let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
    dispatch_function_call(name, &owned).unwrap_or(1)
}

/// `(( $+terminfo[colors] ))` + `$terminfo[colors]` — number of
/// colours the terminal supports. Approximation: reads the terminfo
/// `colors` numeric capability directly (the `$terminfo` module hash
/// getter); `None` when the cap is absent/unset, standing in for a
/// false `$+terminfo[colors]` test.
fn terminfo_colors() -> Option<i64> {
    let pm = crate::ported::modules::terminfo::getterminfo(std::ptr::null_mut(), "colors")?;
    if (pm.node.flags as u32 & crate::ported::zsh_h::PM_UNSET) != 0 {
        return None;
    }
    Some(pm.u_val)
}

/// `_ps1234` — complete a `%X` prompt-escape spec.
pub fn _ps1234() -> i32 {
    // sh: 3-5  locals (`expl`, `grp`, `cols` are scalars in the
    //   source; `_description` fills `expl` as an array).
    let mut specs: Vec<String>;
    let mut ccol: Vec<String>;
    let mut expl: Vec<String>;
    let mut grp: String;
    let mut cols: i64;
    let mut bs = String::new();
    let mut suf: Vec<String> = Vec::new();
    let mut pre: Vec<String> = Vec::new();
    let mut changed = 1;
    let mut ret = 1;
    let ansi: Vec<String>;

    // sh: 7  [[ -z $compstate[quote] ]] && bs='\'
    if getsparam("compstate[quote]").unwrap_or_default().is_empty() {
        bs = "\\".to_string();
    }

    // sh: 9-21  strip already-complete prompt specs, leaving only the
    //   current, incomplete one.
    while changed != 0 {
        changed = 0;
        // sh:13  formats with arg: %x{...}
        if compset_p(r"%[DFK](\\|){[^}]#}") {
            changed = 1;
        }
        // sh:14  normal formats
        if compset_p(r"%[0-9-\\]#[^DFK(0-9-<>\\\[]") {
            changed = 1;
        }
        // sh:15  truncations
        if compset_p(r"%[0-9-\\]#(<[^<]#<|>[^>]#>|\[[^\]]#\])") {
            changed = 1;
        }
        // sh:16  start of ternary
        if compset_p(r"%[0-9-\\]#(\\|)\([0-9-]#[^0-9]?|[^%]") {
            changed = 1;
        }
        // sh:17  sundry other characters
        if compset_p(r"[^%]##") {
            changed = 1;
        }
        // sh:18-20  %D/%F/%K without a following { ... }
        if prefix_matches(r"%(-|)<->#[DFK](\\[^{]|[^{\\])*") && compset_p(r"%[0-9\\-]#[DFK]")
        {
            changed = 1;
        }
    }
    // sh:22  F/K with number
    if prefix_matches(r"%(-|)<->[FK](#e)") {
        compset_p(r"*");
    }

    // sh:24-59  %F / %K colour completion
    if compset_p(r"%[FK]") {
        // sh:26  compset -P '(\\|){' || pre=( -p '{' )
        if !compset_p(r"(\\|){") {
            pre = vec!["-p".to_string(), "{".to_string()];
        }
        // sh:27  compset -S '(\\|)}*' || suf=( -S "$bs}" )
        if !compset_s(r"(\\|)}*") {
            suf = vec!["-S".to_string(), format!("{}}}", bs)];
        }
        // sh:28-38  ansi=( black 30 red 31 … default 39 )
        let ansi_pairs: [(&str, &str); 9] = [
            ("black", "30"),
            ("red", "31"),
            ("green", "32"),
            ("yellow", "33"),
            ("blue", "34"),
            ("magenta", "35"),
            ("cyan", "36"),
            ("white", "37"),
            ("default", "39"),
        ];
        ansi = ansi_pairs
            .iter()
            .flat_map(|(k, v)| [k.to_string(), v.to_string()])
            .collect();
        sethparam("ansi", ansi.clone());

        // sh:40  _description -V ansi-colors expl 'ansi color'
        let _ = dfc("_description", &["-V", "ansi-colors", "expl", "ansi color"]);
        expl = getaparam("expl").unwrap_or_default();
        // sh:41  grp="$expl[expl[(i)-J]+1]" — element of expl following "-J"
        grp = expl
            .iter()
            .position(|x| x == "-J")
            .and_then(|i| expl.get(i + 1))
            .cloned()
            .unwrap_or_default();
        // sh:42  print -v ccol -f "($grp)=%s=%s" ${(kv)ansi}
        //   → one "($grp)=key=value" entry per assoc pair.
        ccol = ansi_pairs
            .iter()
            .map(|(k, v)| format!("({})={}={}", grp, k, v))
            .collect();
        // sh:43  _comp_colors+=( $ccol )
        let mut comp_colors = getaparam("_comp_colors").unwrap_or_default();
        comp_colors.extend(ccol.clone());
        setaparam("_comp_colors", comp_colors);

        // sh:44  compadd "$expl[@]" $suf $pre -k ansi && ret=0
        let mut argv = expl.clone();
        argv.extend(suf.clone());
        argv.extend(pre.clone());
        argv.push("-k".to_string());
        argv.push("ansi".to_string());
        if bin_compadd("compadd", &argv, &make_ops(), 0) == 0 {
            ret = 0;
        }

        // sh:45  if (( $#suf )) && compset -P "(<->|%v)"; then
        if !suf.is_empty() && compset_p(r"(<->|%v)") {
            // sh:46  _wanted ansi-colors expl 'closing brace' compadd -S '' \}
            if dfc(
                "_wanted",
                &["ansi-colors", "expl", "closing brace", "compadd", "-S", "", "}"],
            ) == 0
            {
                ret = 0;
            }
        } else if let Some(term_cols) = terminfo_colors() {
            // sh:47-55  terminal-colour numbers
            // sh:48  (( cols = $terminfo[colors] - 1 ))
            cols = term_cols - 1;
            // sh:49  (( cols = cols > 255 ? 255 : cols ))
            cols = if cols > 255 { 255 } else { cols };
            // sh:50  _description -V terminal-colors expl 'terminal color'
            let _ = dfc(
                "_description",
                &["-V", "terminal-colors", "expl", "terminal color"],
            );
            expl = getaparam("expl").unwrap_or_default();
            // sh:51  grp="$expl[expl[(i)-J]+1]"
            grp = expl
                .iter()
                .position(|x| x == "-J")
                .and_then(|i| expl.get(i + 1))
                .cloned()
                .unwrap_or_default();
            // sh:52  compadd "$expl[@]" $suf $pre {0..$cols}
            let mut argv = expl.clone();
            argv.extend(suf.clone());
            argv.extend(pre.clone());
            for c in 0..=cols {
                argv.push(c.to_string());
            }
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
            // sh:53-55  for c in {0..$cols}; do
            //     _comp_colors+=( "($grp)=${c}=${${${(%):-%F{$c\}}#?\[}%m}" )
            //   The value is the SGR body of the prompt-expanded %F{$c}
            //   escape (strip leading "ESC[", trailing "m"). Without a
            //   live prompt engine here we reproduce the standard 256-
            //   colour %F{n} form "38;5;n" (approximation).
            let mut comp_colors = getaparam("_comp_colors").unwrap_or_default();
            for c in 0..=cols {
                comp_colors.push(format!("({})={}=38;5;{}", grp, c, c));
            }
            setaparam("_comp_colors", comp_colors);
        } else {
            // sh:57  _message -e terminal-colors "number"
            let _ = dfc("_message", &["-e", "terminal-colors", "number"]);
        }
    }

    // sh:61-172  ternary / truncation / date / general spec catalogue
    if compset_p(r"%[0-9-\\]#(\\|)\([0-9-]#[^0-9]") {
        // sh:62-64  ternary conditional: first delimiter
        compset_s(r"*");
        if dfc("_delimiters", &[]) == 0 {
            ret = 0;
        }
    } else if compset_p(r"%[0-9-\\]#[<>\]]") {
        // sh:66-67  truncation
        let _ = dfc("_message", &["-e", "replacements", "replacement string"]);
    } else if compset_p(r"%[0-9-\\]#(\\|)\([0-9-]#") {
        // sh:68-97  ternary conditional: condition character
        // sh:70  compset -S '[.:+/-%]*' || suf=( -S . )
        if !compset_s(r"[.:+/-%]*") {
            suf = vec!["-S".to_string(), ".".to_string()];
        }
        // sh:71  compset -S '*'
        compset_s(r"*");
        // sh:72-94
        specs = vec![
            "!:running with privileges".to_string(),
            "#:effective uid".to_string(),
            "?:exit status".to_string(),
            "_:at least n shell constructs started".to_string(),
            "C:at least n path elements".to_string(),
            "/:at least n path elements".to_string(),
            ".:at least n path elements".to_string(),
            "c:at least n path elements".to_string(),
            "~:at least n path elements".to_string(),
            "D:month".to_string(),
            "d:day of month".to_string(),
            "g:effective gid".to_string(),
            "j:number of jobs".to_string(),
            "L:SHLVL".to_string(),
            "l:number of characters already printed".to_string(),
            "S:SECONDS parameter at least n".to_string(),
            "T:current hour".to_string(),
            "t:current minute".to_string(),
            "v:psvar has at least n elements".to_string(),
            "V:element n of psvar is set and non-empty".to_string(),
            "w:day of week (Sunday = 0)".to_string(),
        ];
        // sh:95-96  [[ $IPREFIX != *- ]] && _describe … specs $suf && ret=0
        if !getsparam("IPREFIX").unwrap_or_default().ends_with('-') {
            setaparam("specs", specs);
            let mut argv = vec![
                "-t".to_string(),
                "ternary-prompt-expressions".to_string(),
                "ternary prompt format test character".to_string(),
                "specs".to_string(),
            ];
            argv.extend(suf.clone());
            let owned: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
            if dfc("_describe", &owned) == 0 {
                ret = 0;
            }
        }
        // sh:97  _message -e numbers number
        let _ = dfc("_message", &["-e", "numbers", "number"]);
    } else if compset_p(r"%D(\\|){") {
        // sh:98-100  %D{...} strftime format
        compset_s(r"(\\|)}*");
        if dfc("_date_formats", &["zsh"]) == 0 {
            ret = 0;
        }
    } else if cond_psfix(&["%".to_string()], 0) != 0
        || testforstyle(
            &format!(
                ":completion:{}:prompt-format-specifiers",
                getsparam("curcontext").unwrap_or_default()
            ),
            "prefix-needed",
        ) != 0
    {
        // sh:104-120  base format-specifier catalogue
        specs = vec![
            "m:hostname up to first .".to_string(),
            "_:status of parser".to_string(),
            "^:reversed status of parser".to_string(),
            "d:current working directory".to_string(),
            "/:current working directory".to_string(),
            "~:current working directory, with ~ replacement".to_string(),
            "N:name of current script or shell function".to_string(),
            "x:name of file containing code being executed".to_string(),
            "c:deprecated".to_string(),
            ".:deprecated".to_string(),
            "C:deprecated".to_string(),
            "F:start using fg color".to_string(),
            "K:start using bg color".to_string(),
            "G:counts as extra character inside %{...%}".to_string(),
            "(:ternary expression %(x.true-string.false-string)".to_string(),
        ];
        // sh:121  compset -P '%' || pre=( -p '%' )
        if !compset_p(r"%") {
            pre = vec!["-p".to_string(), "%".to_string()];
        }
        // sh:122  if ! compset -P '(-|)<->'; then
        if !compset_p(r"(-|)<->") {
            // sh:123-128  SPROMPT-only specs
            if getsparam("service")
                .unwrap_or_default()
                .starts_with("-value-,SPROMPT,")
            {
                specs.push("r:suggested correction".to_string());
                specs.push("R:corrected string".to_string());
            }
            // sh:129-167
            specs.extend(
                [
                    "%:A %",
                    "):A )",
                    "l:current line (tty) with /dev/tty stripped",
                    "M:full hostname",
                    "n:username",
                    "y:current line (tty)",
                    "#:a # when root, % otherwise",
                    "?:return status of last command",
                    "h:current history event number",
                    "!:current history event number",
                    "i:current line number",
                    "I:current source line number",
                    "j:number of jobs",
                    "L:$SHLVL",
                    "D:date in yy-mm-dd format",
                    "T:current time of day, 24-hour format",
                    "t:current time of day, 12-hour am/pm format",
                    "@:current time of day, 12-hour am/pm format",
                    "*:current time of day, 24-hour format with seconds",
                    "w:the date in day-dd format",
                    "W:the date in mm/dd/yy format",
                    "D{:format string like strftime",
                    "B:start bold",
                    "b:stop bold",
                    "E:clear to end of line",
                    "U:start underline",
                    "u:stop underline",
                    "S:start standout",
                    "s:stop standout",
                    "f:reset fg color",
                    "k:reset bg color",
                    "{:start literal escape sequence",
                    "}:stop literal escape sequence",
                    "v:value from $psvar array",
                    "<:truncation from left %len<string<",
                    ">:truncation from right %len>string>",
                    "[:truncation from who knows where",
                ]
                .into_iter()
                .map(String::from),
            );
        }
        // sh:169-170  _describe -t prompt-format-specifiers … specs -S '' $pre
        setaparam("specs", specs);
        let mut argv = vec![
            "-t".to_string(),
            "prompt-format-specifiers".to_string(),
            "prompt format specifier".to_string(),
            "specs".to_string(),
            "-S".to_string(),
            "".to_string(),
        ];
        let pre_empty = pre.is_empty();
        argv.extend(pre);
        let owned: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();
        if dfc("_describe", &owned) == 0 {
            ret = 0;
        }
        // sh:171  (( ! $#pre )) && _message -e prompt-format-specifiers number
        if pre_empty {
            let _ = dfc("_message", &["-e", "prompt-format-specifiers", "number"]);
        }
    }

    // sh:174  return ret
    ret
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::{getaparam, setsparam};

    #[test]
    fn returns_one_without_executor() {
        // No executor → every `compset`/`_describe` fails, the final
        //   `elif` (prefix-needed unset) fires and publishes the spec
        //   catalogue; `ret` stays 1.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("service", "");
        let _ = setsparam("IPREFIX", "");
        assert_eq!(_ps1234(), 1);
    }

    #[test]
    fn publishes_format_specifier_catalogue() {
        // sh:104-167 — the base + general specs land in the `specs`
        //   shell array consumed by `_describe -t prompt-format-specifiers`.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("service", "");
        let _ = setsparam("IPREFIX", "");
        let _ = _ps1234();
        let specs = getaparam("specs").unwrap_or_default();
        // 15 base (sh:104-120) + 37 general (sh:129-167).
        assert!(specs.len() >= 30, "specs.len() = {}", specs.len());
        assert!(specs.iter().any(|s| s.starts_with("m:")));
        assert!(specs.iter().any(|s| s == "v:value from $psvar array"));
    }

    #[test]
    fn sprompt_service_adds_correction_specs() {
        // sh:123-128 — SPROMPT context gains r/R correction specs.
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "");
        let _ = setsparam("IPREFIX", "");
        let _ = setsparam("service", "-value-,SPROMPT,-default-");
        let _ = _ps1234();
        let specs = getaparam("specs").unwrap_or_default();
        assert!(specs.iter().any(|s| s == "r:suggested correction"));
        assert!(specs.iter().any(|s| s == "R:corrected string"));
    }
}
