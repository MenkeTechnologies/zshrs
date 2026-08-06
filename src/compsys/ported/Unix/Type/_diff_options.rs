//! Port of `_diff_options` from `Completion/Unix/Type/_diff_options`.
//!
//! Full upstream body (204 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 6  cmd="$1"; shift
//! sh: 8  _diff_palette() { … _values attribute … _alternative colors … }
//! sh:28  if _pick_variant -r variant -c $cmd gnu=GNU unix -v ||
//! sh:28    [[ $OSTYPE = (freebsd<12->|darwin<22->).* ]]; then
//! sh:30    # GNU diff: build $of/$ofwuc/$ouc/$oss/$ofwy/$ofwg/$ofwl
//! sh:       #   exclusion groups, then a large `_arguments -s $args …`.
//! sh:136  else
//! sh:      # BSD/Solaris diff: OS-specific `$of` + `$args`, smaller
//! sh:      #   `_arguments -s "$args[@]" …`.
//! sh:  fi
//! ```
//!
//! sh approx — the shell brace-expands `{-C+,--context=-}` and
//! interpolates `"($of $oss)"` BEFORE `_arguments` sees the specs; the
//! port reproduces the fully-expanded literal spec strings via the
//! `bx()` (brace) and `excl()` (exclusion-group) helpers. The
//! per-OS `;|` fall-through cases in the BSD/Solaris branch are matched
//! on `$OSTYPE` prefixes.

use crate::compsys::ported::_alternative::_alternative;
use crate::compsys::ported::_arguments::_arguments;
use crate::compsys::ported::_values::_values;
use crate::ported::exec::dispatch_function_call;
use crate::ported::params::getsparam;
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

/// Exclusion group `(v1 v2 …)` — the `"($of $oss)"` prefix.
fn excl(vars: &[&str]) -> String {
    format!("({})", vars.join(" "))
}

/// Brace expansion: `prefix{a,b}suffix` → `[prefix a suffix, prefix b suffix]`.
fn bx(prefix: &str, alts: &[&str], suffix: &str) -> Vec<String> {
    alts.iter()
        .map(|a| format!("{}{}{}", prefix, a, suffix))
        .collect()
}

/// sh:8-25 — nested `_diff_palette`: complete `--palette` color specs.
pub fn _diff_palette(_args: &[String]) -> i32 {
    let mut ret = 1;
    // sh:11-17
    // By NAME so `_values` gets its own `comp_wrapper` frame (c:1556) —
    // without one its `compstate[restore]=''` (`_values.rs:388`) leaks past
    // this function and cancels the caller's restore. `$state` (read at sh:18
    // just below) is a plain global write inside `_values`, unaffected by the
    // extra param scope.
    let vargs: Vec<String> = vec![
        "-s".to_string(),
        ":".to_string(),
        "attribute".to_string(),
        "ad[added text]:attribute [32]:->attrs".to_string(),
        "de[deleted text]:attribute [31]:->attrs".to_string(),
        "hd[header]:attribute [1]:->attrs".to_string(),
        "ln[line numbers]:attribute [36]:->attrs".to_string(),
        "rs[rest - other text]:attribute [0]:->attrs".to_string(),
    ];
    let r = crate::compsys::ported::shared::call_compfn("_values", &vargs, || _values(&vargs));
    if r == 0 {
        ret = 0;
    }
    // sh:18 — if the `->attrs` state fired, offer attribute/color values.
    if !getsparam("state").unwrap_or_default().is_empty() {
        let _ = bin_compset(
            "compset",
            &["-P".to_string(), "*;".to_string()],
            &make_ops(),
            0,
        );
        let matched_suf = bin_compset(
            "compset",
            &["-S".to_string(), "[;=]*".to_string()],
            &make_ops(),
            0,
        ) == 0;
        let mut alt: Vec<String> = vec!["-C".to_string(), "context".to_string()];
        if !matched_suf {
            // sh:20 — `_alternative -C context -O suf …` with suf set.
            alt.push("-O".to_string());
            alt.push("suf".to_string());
            crate::ported::params::setaparam(
                "suf",
                vec![
                    "-S:".to_string(),
                    "-r".to_string(),
                    ": ;\\\t\n\\=".to_string(),
                ],
            );
        }
        alt.push(
            "attributes:attributes:((0:reset 1:bold 3:italics 4:underline 5:blink))".to_string(),
        );
        alt.push(
            "colors:color:((30:default 31:red 32:green 33:yellow 34:blue 35:magenta 36:cyan 37:white))"
                .to_string(),
        );
        if _alternative(&alt) == 0 {
            ret = 0;
        }
    }
    ret
}

/// `_diff_options` — option completion for GNU / BSD / Solaris `diff`.
pub fn _diff_options(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_diff_options");
    // sh:6 — first arg is the diff command name; the rest pass through.
    if args.is_empty() {
        return 1;
    }
    let cmd = args[0].clone();
    let passthru = &args[1..];

    let ostype = getsparam("OSTYPE").unwrap_or_default();

    // sh:27-28 — GNU vs BSD/Solaris variant selection.
    let gnu_pick = dispatch_function_call(
        "_pick_variant",
        &[
            "-r".to_string(),
            "variant".to_string(),
            "-c".to_string(),
            cmd.clone(),
            "gnu=GNU".to_string(),
            "unix".to_string(),
            "-v".to_string(),
        ],
    ) == Some(0)
        && getsparam("variant").as_deref() == Some("gnu");
    let osgnu =
        (ostype.starts_with("freebsd") || ostype.starts_with("darwin")) && ostype.contains('.');

    if gnu_pick || osgnu {
        gnu_arguments(&ostype, passthru)
    } else {
        bsd_arguments(&ostype, passthru)
    }
}

/// sh:30-188 — the GNU diff `_arguments` spec.
fn gnu_arguments(ostype: &str, passthru: &[String]) -> i32 {
    // sh:33-63 — output-format exclusion groups.
    let of = "-y --side-by-side -n --rcs -e -f --ed -q --brief -c -C --context -u -U --unified --old-group-format --new-group-format --changed-group-format --unchanged-group-format --line-format --old-line-format --new-line-format -D --ifdef";
    let ofwuc = "-y --side-by-side -n --rcs -e -f --ed -q --brief --old-group-format --new-group-format --changed-group-format --unchanged-group-format --line-format --old-line-format --new-line-format -D --ifdef";
    let ouc = "-L --label -p --show-c-function -F --show-function-line";
    let oss = "-W --width --left-column --suppress-common-lines";
    let ofwy = "-n --rcs -e -f --ed -q --brief -c -C --context -u -U --unified --old-group-format --new-group-format --changed-group-format --unchanged-group-format --line-format --old-line-format --new-line-format -D --ifdef";
    let ofwg = "-n --rcs -e -f --ed -q --brief -c -C --context -u -U --unified --line-format --old-line-format --new-line-format --unchanged-line-format -D --ifdef";
    let ofwl = "-n --rcs -e -f --ed -q --brief -c -C --context -u -U --unified --old-group-format --new-group-format --changed-group-format --unchanged-group-format";

    let mut args: Vec<String> = Vec::new();
    // sh:29 — `(( $#words > 2 )) && ign='!'`; used only on --version/--help.
    let ign = if crate::ported::params::getaparam("words")
        .map(|w| w.len() > 2)
        .unwrap_or(false)
    {
        "!"
    } else {
        ""
    };

    let variant_gnu = getsparam("variant").as_deref() == Some("gnu");
    if variant_gnu {
        // sh:65-77
        args.extend(bx(
            "(-H --speed-large-files)",
            &["-H", "--speed-large-files"],
            "[assume large files and many small changes]",
        ));
        args.extend(bx(
            "(-E --ignore-tab-expansion)",
            &["-E", "--ignore-tab-expansion"],
            "[ignore changes due to tab expansion]",
        ));
        args.extend(bx(
            "(-Z --ignore-trailing-space)",
            &["-Z", "--ignore-trailing-space"],
            "[ignore white space at line end]",
        ));
        args.push(format!(
            "{}--left-column[output only left column of common lines]",
            excl(&[ofwy, ouc])
        ));
        args.push(format!(
            "{}--old-group-format=[set old group format]:old group format",
            excl(&[ofwg, ouc, oss])
        ));
        args.push(format!(
            "{}--new-group-format=[set new group format]:new group format",
            excl(&[ofwg, ouc, oss])
        ));
        args.push(format!(
            "{}--unchanged-line-format=[set unchanged line format]:unchanged line format",
            excl(&[ofwl, ouc, oss])
        ));
        args.push(
            "(--to-file)--from-file=[compare specified file to all operands]:from file:_files"
                .to_string(),
        );
        args.push(
            "(--from-file)--to-file=[compare all operands to specified file]:to file:_files"
                .to_string(),
        );
        args.push("--palette=[specify colors to use]:color:_diff_palette".to_string());
        args.push(format!("{}(1 2)-v[display version information]", ign));
    } else {
        // sh:79-82
        args.push("!--speed-large-files".to_string());
        if ostype.starts_with("darwin") {
            // sh:81 — brace form `{-A+,--algorithm=}` expanded.
            args.extend(bx(
                "(-A --algorithm)",
                &["-A+", "--algorithm="],
                "[specify the algorithm to use]:algorithm:(myers patience stone)",
            ));
        }
    }

    // sh:85-187 — the shared GNU `_arguments -s $args …` spec.
    let mut spec: Vec<String> = vec!["-s".to_string()];
    spec.extend(args);
    let shared: Vec<String> = {
        let mut v: Vec<String> = Vec::new();
        v.push("--color=-[use colors in output]::when [auto]:(never always auto)".to_string());
        v.extend(bx(
            "(-i --ignore-case)",
            &["-i", "--ignore-case"],
            "[case insensitive]",
        ));
        v.push("--ignore-file-name-case[ignore case when comparing file names]".to_string());
        v.push("!(--ignore-file-name-case)--no-ignore-file-name-case".to_string());
        v.extend(bx(
            "(-b --ignore-space-change)",
            &["-b", "--ignore-space-change"],
            "[ignore changes in the amount of white space]",
        ));
        v.extend(bx(
            "(-w --ignore-all-space)",
            &["-w", "--ignore-all-space"],
            "[ignore all white space]",
        ));
        v.extend(bx(
            "(-B --ignore-blank-lines)",
            &["-B", "--ignore-blank-lines"],
            "[ignore lines that are all blank]",
        ));
        v.extend(bx(
            "(-I --ignore-matching-lines)",
            &["-I+", "--ignore-matching-lines="],
            "[ignore lines that match regex]:line exclusion regex:",
        ));
        v.push("--strip-trailing-cr[strip trailing carriage return on input]".to_string());
        v.extend(bx(
            "(-a --text)",
            &["-a", "--text"],
            "[treat all files as text]",
        ));
        v.extend(bx(
            &excl(&[of, oss]),
            &["-C+", "--context=-"],
            "[output a context diff]:number of lines of copied context",
        ));
        v.push(format!("{}-c[output a context diff]", excl(&[of, oss])));
        v.extend(bx(
            &excl(&[of, oss]),
            &["-U+", "--unified=-"],
            "[output a unified diff]:number of lines of unified context",
        ));
        v.push(format!("{}-u[output a unified diff]", excl(&[of, oss])));
        v.extend(bx(
            &format!("{}*", excl(&[ofwuc, oss])),
            &["-L+", "--label="],
            "[set label to use instead of file name and timestamp]:label",
        ));
        v.extend(bx(
            &excl(&[ofwuc, oss, "-p", "--show-c-function"]),
            &["-p", "--show-c-function"],
            "[show C function of each change]",
        ));
        v.extend(bx(
            &excl(&[ofwuc, oss, "-F", "--show-function-line"]),
            &["-F+", "--show-function-line="],
            "[show the most recent line matching regex]:regex",
        ));
        v.extend(bx(
            &excl(&[of, ouc, oss]),
            &["-q", "--brief"],
            "[output only whether files differ]",
        ));
        v.extend(bx(
            &excl(&[of, ouc, oss, "-e", "--ed"]),
            &["--ed", "-e"],
            "[output an ed script]",
        ));
        v.push(format!("!{}--normal", excl(&[of, ouc, oss])));
        v.extend(bx(
            &excl(&[of, ouc, oss]),
            &["-f", "--forward-ed"],
            "[output a reversed ed script]",
        ));
        v.extend(bx(
            &excl(&[of, ouc, oss]),
            &["-n", "--rcs"],
            "[output an RCS format diff]",
        ));
        v.extend(bx(
            &excl(&[of, oss]),
            &["-D+", "--ifdef="],
            "[output merged file with preprocessor directives]:preprocessor symbol",
        ));
        v.push(format!(
            "{}--changed-group-format=[set changed group format]:changed group format",
            excl(&[ofwg, ouc, oss])
        ));
        v.push(format!(
            "{}--unchanged-group-format=[set unchanged group format]:unchanged group format",
            excl(&[ofwg, ouc, oss])
        ));
        v.push(format!(
            "{}--line-format=[set line format]:line format",
            excl(&[ofwl, ouc, oss])
        ));
        v.push(format!(
            "{}--old-line-format=[set old line format]:old line format",
            excl(&[ofwl, ouc, oss])
        ));
        v.push(format!(
            "{}--new-line-format=[set new line format]:new line format",
            excl(&[ofwl, ouc, oss])
        ));
        v.extend(bx(
            "(-l --paginate)",
            &["-l", "--paginate"],
            "[long output format (paginate with pr(1))]",
        ));
        v.extend(bx(
            "(-t --expand-tabs)",
            &["-t", "--expand-tabs"],
            "[expand tabs to spaces]",
        ));
        v.extend(bx(
            "(-T --initial-tab)",
            &["-T", "--initial-tab"],
            "[prepend a tab]",
        ));
        v.push("--tabsize=[specify width of tab]:width [8]".to_string());
        v.extend(bx(
            "(-r --recursive)",
            &["-r", "--recursive"],
            "[recursively compare subdirectories]",
        ));
        v.push("--no-dereference[don't follow symbolic links]".to_string());
        v.extend(bx(
            "(-N --new-file)",
            &["-N", "--new-file"],
            "[treat absent files as empty]",
        ));
        v.extend(bx(
            "(-P --unidirectional-new-file)",
            &["-P", "--unidirectional-new-file"],
            "[treat absent first files as empty]",
        ));
        v.extend(bx(
            "(-s --report-identical-files)",
            &["-s", "--report-identical-files"],
            "[report when two files are the same]",
        ));
        v.extend(bx(
            "*",
            &["-x+", "--exclude="],
            "[exclude files matching pattern]:exclusion pattern",
        ));
        v.extend(bx(
            "*",
            &["-X+", "--exclude-from="],
            "[exclude files matching pattern in file]:exclude file:_files",
        ));
        v.extend(bx(
            "(-S --starting-file)",
            &["-S+", "--starting-file="],
            "[set first file in comparison]:start with file:_files",
        ));
        v.push("--horizon-lines=[set number of lines to keep in prefix and suffix]:number of horizon lines".to_string());
        v.extend(bx(
            "(-d --minimal)",
            &["-d", "--minimal"],
            "[try to find a smaller set of changes]",
        ));
        v.extend(bx(
            &excl(&[of, ouc]),
            &["-y", "--side-by-side"],
            "[output in two columns]",
        ));
        v.push(format!(
            "{}--suppress-common-lines[don't output common lines]",
            excl(&[ofwy, ouc])
        ));
        v.extend(bx(
            &excl(&[ofwy, ouc, "--width", "-W"]),
            &["--width=", "-W+"],
            "[set size of line]:number of characters per line",
        ));
        v.push(format!(
            "{}(1 2)--version[display version information]",
            ign
        ));
        v.push(format!("{}(1 2)--help[display usage information]", ign));
        v
    };
    spec.extend(shared);
    spec.extend(passthru.iter().cloned());
    // By NAME so `_arguments` runs under its own `comp_wrapper` frame
    // (c:1556); that frame is what bounds its `compstate[restore]=''`
    // (`_arguments.rs:1130`) to the action it dispatched.
    _arguments(&spec)
}

/// sh:189-241 — the BSD / Solaris `diff` `_arguments` spec.
fn bsd_arguments(ostype: &str, passthru: &[String]) -> i32 {
    let openbsd = ostype.starts_with("openbsd");
    let solaris = ostype.starts_with("solaris");

    // sh — `$of` accumulates across the `;|` fall-through cases.
    let mut of = String::from("-c -e -f");
    if openbsd || (solaris && ostype > "solaris2.9") {
        of.push_str(" -u -U");
    }
    if openbsd || solaris {
        of.push_str(" -n -C -D");
    }
    if solaris {
        of.push_str(" -h");
    }
    if openbsd {
        of.push_str(" -q");
    }

    let mut args: Vec<String> = Vec::new();
    if openbsd || (solaris && ostype > "solaris2.9") {
        args.push(format!("({})-u[output a unified diff]", of));
        args.push(format!(
            "({})-U+[output a unified diff]:lines of context",
            of
        ));
    }
    if openbsd || solaris {
        args.push(format!(
            "({})-C+[output a context diff]:lines of context",
            of
        ));
        args.push(format!(
            "({})-D+[output merged file with preprocessor directives]:preprocessor symbol",
            of
        ));
        args.push("-i[case insensitive]".to_string());
        args.push("-s[report on identical files]".to_string());
        args.push("-t[expand tabs in output lines]".to_string());
    }
    if solaris {
        args.push("-w[ignore all white space]".to_string());
        args.push(format!("({})-h[do a fast, half-hearted job]", of));
        args.push(format!("({})-n[output a reversed ed script]", of));
        args.push("-S+[set first file in comparison]:start with file:_files".to_string());
        args.push("-l[long output format (paginate with pr(1))]".to_string());
    } else if openbsd {
        args.push(format!("({})-n[output an rcsdiff(1)-compatible diff]", of));
        args.push(format!(
            "({})-q[only print a line when the files differ; does not produce a list of changes]",
            of
        ));
        args.push("-a[treat all files as ASCII text]".to_string());
        args.push("-d[try to produce the smallest diff possible]".to_string());
        args.push("-I[ignore changes whose lines match the extended regular expression]:extended regular expression pattern".to_string());
        args.push("*-L[print a label instead of the file name and time]:label".to_string());
        args.push("-p[show characters from the last line before the context]".to_string());
        args.push("-T[consistently align tabs]".to_string());
        args.push("-w[like -b, but totally ignore whitespace]".to_string());
        args.push("-N[treat absent files in either directory as if they were empty]".to_string());
        args.push(
            "-P[treat absent files in the second directory as if they were empty]".to_string(),
        );
        args.push("-S[start a directory diff from a file name]:file name:_files".to_string());
        args.push("*-X[exclude files and subdirectories whose basenames match lines in a file]:file name:_files".to_string());
        args.push(
            "*-x[exclude files and subdirectories whose basenames match a pattern]:pattern"
                .to_string(),
        );
    }

    // sh:236-241
    let mut spec: Vec<String> = vec!["-s".to_string()];
    spec.extend(args);
    spec.push(format!("({})-c[output a context diff]", of));
    spec.push(format!("({})-e[output an ed script]", of));
    spec.push(format!("({})-f[output a reversed ed script]", of));
    spec.push("-b[skip trailing white spaces]".to_string());
    spec.push("-r[recursively compare subdirectories]".to_string());
    spec.extend(passthru.iter().cloned());
    // By NAME — same `comp_wrapper` frame reasoning as `gnu_arguments` above.
    _arguments(&spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bx_expands_brace_alternatives() {
        assert_eq!(
            bx(
                "(-i --ignore-case)",
                &["-i", "--ignore-case"],
                "[case insensitive]"
            ),
            vec![
                "(-i --ignore-case)-i[case insensitive]".to_string(),
                "(-i --ignore-case)--ignore-case[case insensitive]".to_string(),
            ]
        );
    }

    #[test]
    fn excl_joins_exclusion_group() {
        assert_eq!(excl(&["-a -b", "-c"]), "(-a -b -c)");
    }

    #[test]
    fn empty_args_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_diff_options(&[]), 1);
    }
}
