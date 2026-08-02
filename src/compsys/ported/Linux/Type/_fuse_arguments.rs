//! Port of `_fuse_arguments` from `Completion/Linux/Type/_fuse_arguments`.
//!
//! `_arguments` wrapper for FUSE-based mount helpers: injects the common
//! FUSE option specs (`-d -f -r -s -h/--help -V/--version`) plus an
//! `-o mount-option` spec whose values come from `_fuse_values`.
//!
//! Full upstream body (54 lines, abridged — head is `#autoload`):
//! ```text
//! sh: 3  local ret long rawret nm=${compstate[nmatches]} fsopt cvalsvar
//! sh: 4  typeset -a fargs opts
//! sh: 6  fargs=( '(-d -f)-d[…]' '-f[…]' '-r[…]' '-s[…]'
//! sh:11    '(-h --help)'{-h,--help}'[…]' '(-V --version)'{-V,--version}'[…]' )
//! sh:15  (( $# )) || fargs+='*:default: _default'
//! sh:17  long=$argv[(I)--]
//! sh:18  if (( long )); then argv[long]=($fargs --)   # splice before last --
//! sh:21  else set -- "$@" $fargs; fi                  # else append
//! sh:24  while [[ $1 == -(O*|F*|[CRWsw]) ]]; do       # eat _arguments passthru opts
//! sh:25    if [[ $1 == -F?* ]]; then cvalsvar=${1[3,-1]}
//! sh:27    elif [[ $1 == -F ]]; then cvalsvar=$2; shift
//! sh:30    else opts+=$1; [[ $1 == -R ]] && rawret=yes; fi
//! sh:34    shift
//! sh:35  done
//! sh:37  if [[ $cvalsvar != - ]]; then
//! sh:38    fsopt='*-o[specify mount options]:mount option:_fuse_values'
//! sh:39    [[ -n $cvalsvar ]] && fsopt+=" -A $cvalsvar"
//! sh:40    fsopt+=' mount\ option'
//! sh:41    set -- "$@" $fsopt
//! sh:42  fi
//! sh:44  _arguments -R $opts "$@"
//! sh:46  ret=$?
//! sh:48  if [[ $ret == 300 ]]; then
//! sh:49    compstate[restore]=
//! sh:50    [[ -z $rawret ]] && ret=$(( nm == $compstate[nmatches] ))
//! sh:51  fi
//! sh:53  return ret
//! ```

use crate::compsys::ported::_arguments::_arguments;
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// sh:6-12 — the fixed FUSE option specs, with the `{-h,--help}` /
/// `{-V,--version}` brace expansions already flattened into their two
/// resulting words each.
const FARGS: [&str; 8] = [
    "(-d -f)-d[enable debug output]",
    "-f[enable foreground operation]",
    "-r[mount filesystem read-only]",
    "-s[disable multi-threaded operation]",
    "(-h --help)-h[display help and exit]",
    "(-h --help)--help[display help and exit]",
    "(-V --version)-V[show version information and exit]",
    "(-V --version)--version[show version information and exit]",
];

/// sh:6-15 — build the `fargs` array. `no_positionals` mirrors
/// `(( $# ))`: when the caller passed no positional args, append the
/// catch-all `*:default: _default` spec.
fn build_fargs(no_positionals: bool) -> Vec<String> {
    let mut f: Vec<String> = FARGS.iter().map(|s| s.to_string()).collect();
    if no_positionals {
        f.push("*:default: _default".to_string()); // sh:15
    }
    f
}

/// sh:17-22 — splice `fargs` into `argv`. `long=$argv[(I)--]` finds the
/// LAST `--`; if present, `argv[long]=($fargs --)` replaces that one `--`
/// element with the fargs followed by `--`. Otherwise the fargs are
/// appended (`set -- "$@" $fargs`).
fn splice_fargs(argv: &[String], fargs: &[String]) -> Vec<String> {
    match argv.iter().rposition(|a| a == "--") {
        Some(idx) => {
            // sh:19 — argv[long]=($fargs --)
            let mut out = Vec::with_capacity(argv.len() + fargs.len());
            out.extend_from_slice(&argv[..idx]);
            out.extend_from_slice(fargs);
            out.push("--".to_string());
            out.extend_from_slice(&argv[idx + 1..]);
            out
        }
        None => {
            // sh:21 — set -- "$@" $fargs
            let mut out = argv.to_vec();
            out.extend_from_slice(fargs);
            out
        }
    }
}

/// sh:24 — glob guard `-(O*|F*|[CRWsw])`: leading `-`, then either `O`/`F`
/// (with anything after) or exactly one of `C R W s w`.
fn matches_leading_opt(w: &str) -> bool {
    let b = w.as_bytes();
    if b.len() < 2 || b[0] != b'-' {
        return false;
    }
    match b[1] {
        b'O' | b'F' => true,
        b'C' | b'R' | b'W' | b's' | b'w' => b.len() == 2,
        _ => false,
    }
}

/// sh:24-42 — consume the leading `_arguments` passthrough options from
/// `argv`, then build the `-o` mount-option spec.
///
/// Returns `(opts, remaining, rawret)` where `opts` is the collected
/// passthrough flags (sh:31), `remaining` is the tail of argv with the
/// `fsopt` spec appended when applicable (sh:41), and `rawret` records
/// whether `-R` was seen (sh:32).
fn parse_leading(argv: Vec<String>) -> (Vec<String>, Vec<String>, bool) {
    let mut opts: Vec<String> = Vec::new();
    let mut rawret = false;
    let mut cvalsvar = String::new();

    // sh:24-35 — while [[ $1 == … ]]; do … shift; done
    let mut p = 0usize;
    while p < argv.len() {
        let w = &argv[p];
        if !matches_leading_opt(w) {
            break;
        }
        let b = w.as_bytes();
        if b[1] == b'F' && b.len() > 2 {
            // sh:25-26 — [[ $1 == -F?* ]] → cvalsvar=${1[3,-1]}
            cvalsvar = w[2..].to_string();
        } else if w == "-F" {
            // sh:27-29 — cvalsvar=$2; shift (consume the value word too)
            cvalsvar = argv.get(p + 1).cloned().unwrap_or_default();
            p += 1;
        } else {
            // sh:31-32 — opts+=$1; [[ $1 == -R ]] && rawret=yes
            opts.push(w.clone());
            if w == "-R" {
                rawret = true;
            }
        }
        p += 1; // sh:34 — trailing shift
    }

    let mut remaining: Vec<String> = argv[p..].to_vec();

    // sh:37-42 — build the mount-option spec unless suppressed by `-F -`.
    if cvalsvar != "-" {
        let mut fsopt = String::from("*-o[specify mount options]:mount option:_fuse_values"); // sh:38
        if !cvalsvar.is_empty() {
            fsopt.push_str(&format!(" -A {}", cvalsvar)); // sh:39
        }
        fsopt.push_str(" mount\\ option"); // sh:40 — literal backslash-space
        remaining.push(fsopt); // sh:41 — set -- "$@" $fsopt
    }

    (opts, remaining, rawret)
}

/// Read `$compstate[nmatches]` as an integer (0 when unset/unparsable).
fn nmatches() -> i64 {
    get_compstate_str("nmatches")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

/// `_fuse_arguments` — `_arguments` wrapper adding the standard FUSE
/// option specs and an `-o mount-option` completion.
pub fn _fuse_arguments(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_fuse_arguments");
    // sh:3 — nm=${compstate[nmatches]} captured up front.
    let nm = nmatches();

    // sh:6-15 / sh:17-22 — build fargs and splice into a mutable argv.
    let fargs = build_fargs(args.is_empty());
    let argv = splice_fargs(args, &fargs);

    // sh:24-42 — strip passthrough opts, assemble the mount-option spec.
    let (opts, remaining, rawret) = parse_leading(argv);

    // sh:44 — _arguments -R $opts "$@"
    let mut call: Vec<String> = Vec::with_capacity(1 + opts.len() + remaining.len());
    call.push("-R".to_string());
    call.extend(opts);
    call.extend(remaining);
    let mut ret = _arguments(&call);

    // sh:46-51
    if ret == 300 {
        set_compstate_str("restore", ""); // sh:49 — compstate[restore]=
        if !rawret {
            // sh:50 — ret=$(( nm == $compstate[nmatches] ))
            ret = if nm == nmatches() { 1 } else { 0 };
        }
    }

    ret // sh:53
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_opt_guard_matches_spec() {
        for ok in [
            "-C", "-R", "-W", "-s", "-w", "-O", "-Oxx", "-F", "-Fgrp", "-F-",
        ] {
            assert!(matches_leading_opt(ok), "{ok} should match");
        }
        for no in [
            "-Wx",
            "-Cx",
            "-d",
            "-r",
            "--",
            "-",
            "",
            "(-d -f)-d[x]",
            "spec",
        ] {
            assert!(!matches_leading_opt(no), "{no} should not match");
        }
    }

    #[test]
    fn build_fargs_appends_default_only_when_empty() {
        let with = build_fargs(true);
        assert_eq!(with.len(), 9);
        assert_eq!(with.last().unwrap(), "*:default: _default");
        // brace expansions flattened.
        assert!(with.contains(&"(-h --help)-h[display help and exit]".to_string()));
        assert!(with.contains(&"(-h --help)--help[display help and exit]".to_string()));
        assert!(with
            .contains(&"(-V --version)--version[show version information and exit]".to_string()));

        let without = build_fargs(false);
        assert_eq!(without.len(), 8);
        assert!(!without.iter().any(|s| s == "*:default: _default"));
    }

    #[test]
    fn splice_inserts_before_last_dashdash_else_appends() {
        let fargs = vec!["A".to_string(), "B".to_string()];

        // No `--` → appended at the end.
        let out = splice_fargs(&["x".to_string(), "y".to_string()], &fargs);
        assert_eq!(out, vec!["x", "y", "A", "B"]);

        // Last `--` replaced by fargs followed by `--`.
        let argv = vec![
            "-C".to_string(),
            "--".to_string(),
            "tail".to_string(),
            "--".to_string(),
            "z".to_string(),
        ];
        let out = splice_fargs(&argv, &fargs);
        assert_eq!(out, vec!["-C", "--", "tail", "A", "B", "--", "z"]);
    }

    #[test]
    fn parse_leading_collects_opts_and_rawret() {
        // -C and -R passthrough; -R sets rawret; empty cvalsvar → -o spec, no -A.
        let (opts, remaining, rawret) =
            parse_leading(vec!["-C".to_string(), "-R".to_string(), "spec".to_string()]);
        assert_eq!(opts, vec!["-C", "-R"]);
        assert!(rawret);
        assert_eq!(remaining.len(), 2);
        assert_eq!(remaining[0], "spec");
        assert_eq!(
            remaining[1],
            "*-o[specify mount options]:mount option:_fuse_values mount\\ option"
        );
    }

    #[test]
    fn parse_leading_f_consumes_value_and_adds_a() {
        // `-F grp` → cvalsvar=grp, both words consumed, ` -A grp` in fsopt.
        let (opts, remaining, rawret) = parse_leading(vec![
            "-F".to_string(),
            "grp".to_string(),
            "spec".to_string(),
        ]);
        assert!(opts.is_empty());
        assert!(!rawret);
        assert_eq!(remaining[0], "spec");
        assert_eq!(
            remaining[1],
            "*-o[specify mount options]:mount option:_fuse_values -A grp mount\\ option"
        );
    }

    #[test]
    fn parse_leading_glued_f_value() {
        // `-Fgrp` glued form → same cvalsvar=grp.
        let (_opts, remaining, _r) = parse_leading(vec!["-Fgrp".to_string(), "spec".to_string()]);
        assert_eq!(
            remaining[1],
            "*-o[specify mount options]:mount option:_fuse_values -A grp mount\\ option"
        );
    }

    #[test]
    fn parse_leading_dash_cvalsvar_suppresses_o_spec() {
        // `-F -` (glued as `-F-`) → cvalsvar="-", no -o spec appended.
        let (_opts, remaining, _r) = parse_leading(vec!["-F-".to_string(), "spec".to_string()]);
        assert_eq!(remaining, vec!["spec".to_string()]);
    }
}
