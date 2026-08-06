//! Port of `_x_arguments` from `Completion/X/Utility/_x_arguments`.
//!
//! `_arguments` wrapper for X11 clients: injects the common X toolkit
//! option specs (`-display`, `-geometry`) then delegates to `_arguments`,
//! honouring the `ret==300` restore/nm-comparison protocol.
//!
//! Full upstream body (36 lines, abridged — head is the `#compdef` line):
//! ```text
//! sh: 3  local ret long xargs opts rawret nm="$compstate[nmatches]"
//! sh: 5  xargs=( '-display:display:_x_display' '-geometry:geometry:_x_geometry' )
//! sh:10  (( $# )) || xargs=( "$xargs[@]" '*:default: _default' )
//! sh:12  long=$argv[(I)--]
//! sh:13  if (( long )); then argv[long]=( "$xargs[@]" -- )   # splice before last --
//! sh:15  else set -- "$@" "$xargs[@]"; fi                    # else append
//! sh:19  opts=()
//! sh:20  while [[ $1 = -(O*|[CRWsw]) ]]; do                  # eat _arguments passthru opts
//! sh:21    opts=($opts $1)
//! sh:22    [[ $1 = -R ]] && rawret=yes
//! sh:23    shift
//! sh:24  done
//! sh:26  _arguments -R "$opts[@]" "$@"
//! sh:28  ret=$?
//! sh:30  if [[ "$ret" = 300 ]]; then
//! sh:31    compstate[restore]=''
//! sh:32    [[ -z $rawret ]] && ret=$(( nm == $compstate[nmatches] ))
//! sh:33  fi
//! sh:35  return ret
//! ```

use crate::compsys::ported::_arguments::_arguments;
use crate::ported::zle::compcore::{get_compstate_str, set_compstate_str};

/// sh:5 — the fixed X toolkit option specs.
const XARGS: [&str; 2] = [
    "-display:display:_x_display",
    "-geometry:geometry:_x_geometry",
];

/// sh:5-10 — build the `xargs` array. `no_positionals` mirrors `(( $# ))`:
/// when the caller passed no positional args, append the catch-all
/// `*:default: _default` spec.
fn build_xargs(no_positionals: bool) -> Vec<String> {
    let mut x: Vec<String> = XARGS.iter().map(|s| s.to_string()).collect();
    if no_positionals {
        x.push("*:default: _default".to_string()); // sh:10
    }
    x
}

/// sh:12-16 — splice `xargs` into `argv`. `long=$argv[(I)--]` finds the LAST
/// `--`; if present, `argv[long]=( "$xargs[@]" -- )` replaces that one `--`
/// element with the xargs followed by `--`. Otherwise the xargs are appended
/// (`set -- "$@" "$xargs[@]"`).
fn splice_xargs(argv: &[String], xargs: &[String]) -> Vec<String> {
    match argv.iter().rposition(|a| a == "--") {
        Some(idx) => {
            // sh:14 — argv[long]=( "$xargs[@]" -- )
            let mut out = Vec::with_capacity(argv.len() + xargs.len());
            out.extend_from_slice(&argv[..idx]);
            out.extend_from_slice(xargs);
            out.push("--".to_string());
            out.extend_from_slice(&argv[idx + 1..]);
            out
        }
        None => {
            // sh:16 — set -- "$@" "$xargs[@]"
            let mut out = argv.to_vec();
            out.extend_from_slice(xargs);
            out
        }
    }
}

/// sh:20 — glob guard `-(O*|[CRWsw])`: leading `-`, then either `O`
/// (with anything after) or exactly one of `C R W s w`.
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

/// sh:19-24 — consume the leading `_arguments` passthrough options from
/// `argv`. Returns `(opts, remaining, rawret)` where `opts` is the collected
/// passthrough flags (sh:21), `remaining` is the tail of argv (the `"$@"`
/// passed to `_arguments`), and `rawret` records whether `-R` was seen (sh:22).
fn parse_leading(argv: &[String]) -> (Vec<String>, Vec<String>, bool) {
    let mut opts: Vec<String> = Vec::new();
    let mut rawret = false;

    let mut p = 0usize;
    // sh:20-24 — while [[ $1 = -(O*|[CRWsw]) ]]; do … shift; done
    while p < argv.len() {
        let w = &argv[p];
        if !matches_leading_opt(w) {
            break;
        }
        opts.push(w.clone()); // sh:21 — opts=($opts $1)
        if w == "-R" {
            rawret = true; // sh:22 — [[ $1 = -R ]] && rawret=yes
        }
        p += 1; // sh:23 — shift
    }

    let remaining: Vec<String> = argv[p..].to_vec();
    (opts, remaining, rawret)
}

/// Read `$compstate[nmatches]` as an integer (0 when unset/unparsable).
fn nmatches() -> i64 {
    get_compstate_str("nmatches")
        .and_then(|s| s.trim().parse::<i64>().ok())
        .unwrap_or(0)
}

/// `_x_arguments` — `_arguments` wrapper adding the standard X toolkit
/// `-display` / `-geometry` option specs.
pub fn _x_arguments(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_arguments");
    // sh:3 — nm=$compstate[nmatches] captured up front.
    let nm = nmatches();

    // sh:5-10 / sh:12-16 — build xargs and splice into a mutable argv.
    let xargs = build_xargs(args.is_empty());
    let argv = splice_xargs(args, &xargs);

    // sh:19-24 — strip passthrough opts.
    let (opts, remaining, rawret) = parse_leading(&argv);

    // sh:26 — _arguments -R "$opts[@]" "$@"
    let mut call: Vec<String> = Vec::with_capacity(1 + opts.len() + remaining.len());
    call.push("-R".to_string());
    call.extend(opts);
    call.extend(remaining);
    // By NAME, not a direct Rust call: `_arguments` is a shell function in zsh
    // and so runs inside its own `comp_wrapper` frame (`Src/Zle/complete.c:1556`),
    // whose c:1642 epilogue restores the CALLER's `compstate[restore]`. That is
    // what stops `_arguments`' own `compstate[restore]=''` (`_arguments.rs:1130`)
    // from cancelling the restore owed to whoever called `_x_arguments`. The
    // sh:31 opt-out below is this function's own and is unaffected; the 300
    // status survives `doshfunc` (LASTVAL is an i32, unmasked).
    let mut ret = _arguments(&call);

    // sh:28 — ret=$?
    // sh:30-33
    if ret == 300 {
        set_compstate_str("restore", ""); // sh:31 — compstate[restore]=''
        if !rawret {
            // sh:32 — ret=$(( nm == $compstate[nmatches] ))
            ret = if nm == nmatches() { 1 } else { 0 };
        }
    }

    ret // sh:35 — return ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_opt_guard_matches_spec() {
        for ok in ["-C", "-R", "-W", "-s", "-w", "-O", "-Oxx"] {
            assert!(matches_leading_opt(ok), "{ok} should match");
        }
        for no in [
            "-Wx", "-Cx", "-d", "-r", "--", "-", "", "-F", "-Fgrp", "spec",
        ] {
            assert!(!matches_leading_opt(no), "{no} should not match");
        }
    }

    #[test]
    fn build_xargs_appends_default_only_when_empty() {
        let with = build_xargs(true);
        assert_eq!(with.len(), 3);
        assert_eq!(with.last().unwrap(), "*:default: _default");
        assert!(with.contains(&"-display:display:_x_display".to_string()));
        assert!(with.contains(&"-geometry:geometry:_x_geometry".to_string()));

        let without = build_xargs(false);
        assert_eq!(without.len(), 2);
        assert!(!without.iter().any(|s| s == "*:default: _default"));
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
        // -C and -R passthrough; -R sets rawret; specs left in remaining.
        let (opts, remaining, rawret) = parse_leading(&[
            "-C".to_string(),
            "-R".to_string(),
            "-display:display:_x_display".to_string(),
        ]);
        assert_eq!(opts, vec!["-C", "-R"]);
        assert!(rawret);
        assert_eq!(remaining, vec!["-display:display:_x_display".to_string()]);
    }

    #[test]
    fn parse_leading_stops_at_first_nonopt() {
        // First non-matching word ends the loop; nothing after is consumed.
        let (opts, remaining, rawret) =
            parse_leading(&["-W".to_string(), "spec".to_string(), "-C".to_string()]);
        assert_eq!(opts, vec!["-W"]);
        assert!(!rawret);
        assert_eq!(remaining, vec!["spec".to_string(), "-C".to_string()]);
    }
}
