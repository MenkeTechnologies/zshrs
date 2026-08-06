//! Port of `_x_extension` from `Completion/X/Type/_x_extension`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  local expl
//! sh: 4
//! sh: 5  _tags extensions || return 1
//! sh: 6
//! sh: 7  (( $+_xe_cache )) || _xe_cache=( "${(@)${(@f)$(xdpyinfo)}[(r)number of extensions:*,-1][2,(r)default screen number:*][1,-2]//[      ]}" )
//! sh: 8
//! sh: 9  if [[ "$1" = -a ]]; then
//! sh:10    shift
//! sh:11
//! sh:12    _wanted extensions expl 'X extension' \
//! sh:13        compadd "$@" -M 'm:{a-z}={A-Z} r:|-=* r:|=*' - all "$_xe_cache[@]"
//! sh:14  else
//! sh:15    [[ "$1" = - ]] && shift
//! sh:16
//! sh:17    _wanted extensions expl 'X extension' \
//! sh:18        compadd "$@" -M 'm:{a-z}={A-Z} r:|-=* r:|=*' -a - _xe_cache
//! sh:19  fi
//! ```
//!
//! sh:7's parameter expansion, decomposed:
//!   1. `$(xdpyinfo)`             — raw stdout of `xdpyinfo` (no args).
//!   2. `${(f)...}`               — split into an array of lines.
//!   3. `[(r)"number of extensions:*",-1]` — slice from the first line
//!      matching that glob through the end of the array.
//!   4. `[2,(r)"default screen number:*"]` — within that slice, take
//!      elements 2..=(index of the first line matching that glob).
//!   5. `[1,-2]`                  — drop the trailing element (the
//!      "default screen number:" line itself), leaving just the
//!      (still-indented) extension name lines.
//!   6. `//[      ]`              — strip every literal space character
//!      from each element (removes the indentation; the class contains
//!      only the space character, repeated).
//!
//! Delegates to the already-ported `_tags` (sh:5) and `_wanted` (sh:12,
//! sh:17); `_wanted` itself fans out through `_all_labels` into the real
//! `bin_compadd` (`-a`/`-M` handled by the underlying builtin exactly as
//! in `_baudrates`/`_bsd_disks`). `xdpyinfo` is spawned raw via
//! `std::process::Command`, mirroring `_bsd_disks`'s bare-`$(cmd)`
//! convention (the upstream uses bare `$(xdpyinfo)`, not `_call_program`).

use crate::compsys::ported::_tags::tags_byname;
use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::params::{getaparam, setaparam};
use std::process::Command;

/// sh:7 inner — `${(@f)$(xdpyinfo)}[(r)number of extensions:*,-1]
/// [2,(r)default screen number:*][1,-2]//[      ]`. Returns the parsed
/// (still-unsorted, whitespace-stripped) extension name list, or an
/// empty vec if either anchor line is missing from `raw`.
fn parse_xdpyinfo_extensions(raw: &str) -> Vec<String> {
    let lines: Vec<&str> = raw.lines().collect();

    // sh:7  [(r)"number of extensions:*",-1] — first (leftmost) match.
    let idx1 = match lines
        .iter()
        .position(|l| l.starts_with("number of extensions:"))
    {
        Some(i) => i,
        None => return Vec::new(),
    };
    let b = &lines[idx1..];

    // sh:7  [2,(r)"default screen number:*"] — first match within `b`.
    let idx2 = match b
        .iter()
        .position(|l| l.starts_with("default screen number:"))
    {
        Some(i) => i,
        None => return Vec::new(),
    };
    // Need at least b[1] (1-based index 2) to exist as a valid range start.
    if idx2 < 1 {
        return Vec::new();
    }
    let c = &b[1..=idx2];

    // sh:7  [1,-2] — drop the trailing "default screen number:" line.
    if c.is_empty() {
        return Vec::new();
    }
    let c2 = &c[..c.len() - 1];

    // sh:7  //[      ] — strip every space character from each element.
    c2.iter()
        .map(|l| l.chars().filter(|&ch| ch != ' ').collect::<String>())
        .collect()
}

/// Spawn `cmd args...` and return its captured stdout (empty string on
/// spawn failure — zsh `$(...)` command substitution likewise degrades
/// to empty output rather than aborting the script).
fn run_capture(cmd: &str, args: &[&str]) -> String {
    match Command::new(cmd).args(args).output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).to_string(),
        Err(_) => String::new(),
    }
}

/// `_x_extension` — complete an X server extension name, sourced from
/// `xdpyinfo`'s "number of extensions:" block (cached in `$_xe_cache`
/// for the lifetime of the shell). `-a` prepends a literal `all`
/// alternative alongside the cached names; otherwise the cache array is
/// completed by name via `compadd -a`.
pub fn _x_extension(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_extension");
    // sh:5  _tags extensions || return 1
    if tags_byname(&["extensions".to_string()]) != 0 {
        return 1;
    }

    // sh:7  (( $+_xe_cache )) || _xe_cache=( ... )
    if getaparam("_xe_cache").is_none() {
        let raw = run_capture("xdpyinfo", &[]);
        setaparam("_xe_cache", parse_xdpyinfo_extensions(&raw));
    }

    let mut argv = args.to_vec();

    // sh:9  if [[ "$1" = -a ]]; then
    if !argv.is_empty() && argv[0] == "-a" {
        // sh:10  shift
        argv.remove(0);

        // sh:12-13  _wanted extensions expl 'X extension' \
        //   compadd "$@" -M '...' - all "$_xe_cache[@]"
        let xe_cache = getaparam("_xe_cache").unwrap_or_default();
        let mut wanted_argv: Vec<String> = vec![
            "extensions".to_string(),
            "expl".to_string(),
            "X extension".to_string(),
            "compadd".to_string(),
        ];
        wanted_argv.extend(argv.iter().cloned());
        wanted_argv.push("-M".to_string());
        wanted_argv.push("m:{a-z}={A-Z} r:|-=* r:|=*".to_string());
        wanted_argv.push("-".to_string());
        wanted_argv.push("all".to_string());
        wanted_argv.extend(xe_cache);
        wanted_byname(&wanted_argv)
    } else {
        // sh:15  [[ "$1" = - ]] && shift
        if !argv.is_empty() && argv[0] == "-" {
            argv.remove(0);
        }

        // sh:17-18  _wanted extensions expl 'X extension' \
        //   compadd "$@" -M '...' -a - _xe_cache
        let mut wanted_argv: Vec<String> = vec![
            "extensions".to_string(),
            "expl".to_string(),
            "X extension".to_string(),
            "compadd".to_string(),
        ];
        wanted_argv.extend(argv.iter().cloned());
        wanted_argv.push("-M".to_string());
        wanted_argv.push("m:{a-z}={A-Z} r:|-=* r:|=*".to_string());
        wanted_argv.push("-a".to_string());
        wanted_argv.push("-".to_string());
        wanted_argv.push("_xe_cache".to_string());
        wanted_byname(&wanted_argv)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_XDPYINFO: &str = "\
name of display:    :0
version number:    11.0
vendor string:    The X.Org Foundation
number of extensions:    5
    BIG-REQUESTS
    Composite
    DAMAGE
    DOUBLE-BUFFER
    GLX
default screen number:  0
number of screens:    1
";

    #[test]
    fn parse_xdpyinfo_extensions_strips_indent_and_trims_range() {
        assert_eq!(
            parse_xdpyinfo_extensions(SAMPLE_XDPYINFO),
            vec![
                "BIG-REQUESTS".to_string(),
                "Composite".to_string(),
                "DAMAGE".to_string(),
                "DOUBLE-BUFFER".to_string(),
                "GLX".to_string(),
            ]
        );
    }

    #[test]
    fn parse_xdpyinfo_extensions_empty_without_number_of_extensions_anchor() {
        assert_eq!(
            parse_xdpyinfo_extensions("name of display:    :0\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn parse_xdpyinfo_extensions_empty_without_default_screen_anchor() {
        let raw = "number of extensions:    2\n    BIG-REQUESTS\n    Composite\n";
        assert_eq!(parse_xdpyinfo_extensions(raw), Vec::<String>::new());
    }

    #[test]
    fn parse_xdpyinfo_extensions_empty_input() {
        assert_eq!(parse_xdpyinfo_extensions(""), Vec::<String>::new());
    }

    #[test]
    fn parse_xdpyinfo_extensions_zero_extensions_between_anchors() {
        // "number of extensions:" immediately followed by "default screen
        // number:" (idx2 == 0 relative to `b`) — no room for a valid
        // [2,idx2] range, so sh:7 yields an empty array.
        let raw = "number of extensions:    0\ndefault screen number:  0\n";
        assert_eq!(parse_xdpyinfo_extensions(raw), Vec::<String>::new());
    }

    #[test]
    fn returns_one_without_completion_context() {
        // sh:5 — `_tags extensions || return 1` fires when no completion
        // context / registered tags are set up (matches _baudrates'/
        // _x_borderwidth's no-context convention).
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_x_extension(&[]), 1);
    }
}
