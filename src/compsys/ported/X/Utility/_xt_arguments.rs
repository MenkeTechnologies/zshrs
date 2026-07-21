//! Port of `_xt_arguments` from `Completion/X/Utility/_xt_arguments`.
//!
//! `_arguments` wrapper injecting the standard X Toolkit (Xt) command-line
//! option specs (`-display`, `-geometry`, `-fg`, `-bg`, `-xrm`, …) into the
//! caller's spec list, then delegating to `_arguments`.
//!
//! Full upstream body (72 lines, abridged — head is a usage comment on the
//! XrmOptionDescRec → spec mapping):
//! ```text
//! sh:23  local ret long xargs opts rawret nm="$compstate[nmatches]"
//! sh:25  xargs=( -+{rv,synchronous} -{reverse,iconic}
//! sh:28    '-background:background color:_x_color' … '*-xrm:resource:_x_resource'
//! sh:45    '-xtsessionID:session ID:_xt_session_id' )
//! sh:48  long=$argv[(I)--]
//! sh:49  if (( long )); then argv[long]=( "$xargs[@]" -- )   # splice before last --
//! sh:52  else set -- "$@" "$xargs[@]"; fi                    # else append
//! sh:55  opts=()
//! sh:56  while [[ $1 = -(O*|[CRWsw]) ]]; do                  # eat _arguments passthru opts
//! sh:57    opts=($opts $1)
//! sh:58    [[ $1 = -R ]] && rawret=yes
//! sh:59    shift
//! sh:60  done
//! sh:62  _arguments -R "$opts[@]" "$@"
//! sh:64  ret=$?
//! sh:66  if [[ "$ret" = 300 ]]; then
//! sh:67    compstate[restore]=''
//! sh:68    [[ -z $rawret ]] && ret=$(( nm == $compstate[nmatches] ))
//! sh:69  fi
//! sh:71  return ret
//! ```

use crate::compsys::ported::_arguments::_arguments;
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// sh:25-46 — the fixed Xt option specs. The two leading brace expansions
/// are flattened into their resulting words:
///   `-+{rv,synchronous}` → `-+rv`, `-+synchronous`
///   `-{reverse,iconic}`  → `-reverse`, `-iconic`
const XARGS: [&str; 22] = [
    "-+rv",
    "-+synchronous",
    "-reverse",
    "-iconic",
    "-background:background color:_x_color",
    "-bd:border color:_x_color",
    "-bg:background color:_x_color",
    "-bordercolor:border color:_x_color",
    "-borderwidth:border width:_x_borderwidth",
    "-bw:border width:_x_borderwidth",
    "-display:display:_x_display",
    "-fg:foreground color:_x_color",
    "-font:font:_x_font",
    "-fn:font:_x_font",
    "-foreground:foreground color:_x_color",
    "-geometry:geometry:_x_geometry",
    "-name:name:_x_name",
    "-selectionTimeout:selection timeout (milliseconds):_x_selection_timeout",
    "-title:title:_x_title",
    "-xnllanguage:locale:_x_locale",
    "*-xrm:resource:_x_resource",
    "-xtsessionID:session ID:_xt_session_id",
];

/// sh:48-53 — splice `xargs` into `argv`. `long=$argv[(I)--]` finds the LAST
/// `--`; if present, `argv[long]=("$xargs[@]" --)` replaces that one `--`
/// element with the xargs followed by `--`. Otherwise the xargs are appended
/// (`set -- "$@" "$xargs[@]"`).
fn splice_xargs(argv: &[String], xargs: &[String]) -> Vec<String> {
    match argv.iter().rposition(|a| a == "--") {
        Some(idx) => {
            // sh:49 — argv[long]=( "$xargs[@]" -- )
            let mut out = Vec::with_capacity(argv.len() + xargs.len());
            out.extend_from_slice(&argv[..idx]);
            out.extend_from_slice(xargs);
            out.push("--".to_string());
            out.extend_from_slice(&argv[idx + 1..]);
            out
        }
        None => {
            // sh:52 — set -- "$@" "$xargs[@]"
            let mut out = argv.to_vec();
            out.extend_from_slice(xargs);
            out
        }
    }
}

/// sh:56 — glob guard `-(O*|[CRWsw])`: leading `-`, then either `O`
/// (followed by anything, including nothing) or exactly one of `C R W s w`.
fn matches_leading_opt(w: &str) -> bool {
    let b = w.as_bytes();
    if b.len() < 2 || b[0] != b'-' {
        return false;
    }
    match b[1] {
        b'O' => true,
        b'C' | b'R' | b'W' | b's' | b'w' => b.len() == 2,
        _ => false,
    }
}

/// sh:55-60 — consume the leading `_arguments` passthrough options from
/// `argv`. Returns `(opts, remaining, rawret)` where `opts` are the collected
/// passthrough flags, `remaining` is the tail of argv, and `rawret` records
/// whether `-R` was seen (sh:58).
fn parse_leading(argv: &[String]) -> (Vec<String>, Vec<String>, bool) {
    let mut opts: Vec<String> = Vec::new();
    let mut rawret = false;
    let mut p = 0usize;
    // sh:56-60 — while [[ $1 = -(O*|[CRWsw]) ]]; do … shift; done
    while p < argv.len() {
        let w = &argv[p];
        if !matches_leading_opt(w) {
            break;
        }
        opts.push(w.clone()); // sh:57
        if w == "-R" {
            rawret = true; // sh:58
        }
        p += 1; // sh:59
    }
    (opts, argv[p..].to_vec(), rawret)
}

/// Read `$compstate[nmatches]` as an integer (0 when unset/unparsable).
fn nmatches() -> i64 {
    get_compstate_str("nmatches")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

/// `_xt_arguments` — `_arguments` wrapper adding the standard X Toolkit
/// command-line option specs.
pub fn _xt_arguments(args: &[String]) -> i32 {
    // sh:23 — nm=$compstate[nmatches] captured up front.
    let nm = nmatches();

    // sh:25-46 — the fixed Xt specs.
    let xargs: Vec<String> = XARGS.iter().map(|s| s.to_string()).collect();

    // sh:48-53 — splice xargs into a mutable argv.
    let argv = splice_xargs(args, &xargs);

    // sh:55-60 — strip passthrough opts.
    let (opts, remaining, rawret) = parse_leading(&argv);

    // sh:62 — _arguments -R "$opts[@]" "$@"
    let mut call: Vec<String> = Vec::with_capacity(1 + opts.len() + remaining.len());
    call.push("-R".to_string());
    call.extend(opts);
    call.extend(remaining);
    let mut ret = _arguments(&call);

    // sh:64-69
    if ret == 300 {
        set_compstate_str("restore", ""); // sh:67 — compstate[restore]=''
        if !rawret {
            // sh:68 — ret=$(( nm == $compstate[nmatches] ))
            ret = if nm == nmatches() { 1 } else { 0 };
        }
    }

    ret // sh:71
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_opt_guard_matches_spec() {
        for ok in ["-C", "-R", "-W", "-s", "-w", "-O", "-Oxx"] {
            assert!(matches_leading_opt(ok), "{ok} should match");
        }
        // -F is NOT in this guard (unlike _fuse_arguments); neither are 2+ char
        // C/R/W/s/w forms, nor bare specs.
        for no in [
            "-F", "-Fgrp", "-Wx", "-Cx", "-d", "-r", "--", "-", "", "-xrm", "spec",
        ] {
            assert!(!matches_leading_opt(no), "{no} should not match");
        }
    }

    #[test]
    fn splice_inserts_before_last_dashdash_else_appends() {
        let xargs = vec!["A".to_string(), "B".to_string()];

        // No `--` → appended at the end.
        let out = splice_xargs(&["x".to_string(), "y".to_string()], &xargs);
        assert_eq!(out, vec!["x", "y", "A", "B"]);

        // Last `--` replaced by xargs followed by `--`.
        let argv = vec![
            "-C".to_string(),
            "--".to_string(),
            "tail".to_string(),
            "--".to_string(),
            "z".to_string(),
        ];
        let out = splice_xargs(&argv, &xargs);
        assert_eq!(out, vec!["-C", "--", "tail", "A", "B", "--", "z"]);
    }

    #[test]
    fn parse_leading_collects_opts_and_rawret() {
        // -C and -R passthrough; -R sets rawret; first non-guard word stops it.
        let (opts, remaining, rawret) = parse_leading(&[
            "-C".to_string(),
            "-R".to_string(),
            "-xrm:resource:_x_resource".to_string(),
        ]);
        assert_eq!(opts, vec!["-C", "-R"]);
        assert!(rawret);
        assert_eq!(remaining, vec!["-xrm:resource:_x_resource".to_string()]);
    }

    #[test]
    fn parse_leading_no_rawret_without_dash_r() {
        let (opts, remaining, rawret) =
            parse_leading(&["-Ostuff".to_string(), "spec".to_string()]);
        assert_eq!(opts, vec!["-Ostuff"]);
        assert!(!rawret);
        assert_eq!(remaining, vec!["spec".to_string()]);
    }

    #[test]
    fn xargs_contains_flattened_brace_words() {
        let x: Vec<String> = XARGS.iter().map(|s| s.to_string()).collect();
        for w in ["-+rv", "-+synchronous", "-reverse", "-iconic"] {
            assert!(x.contains(&w.to_string()), "{w} missing from XARGS");
        }
        assert!(x.contains(&"*-xrm:resource:_x_resource".to_string()));
    }
}
