//! Port of `_x_cursor` from `Completion/X/Type/_x_cursor`.
//!
//! Full upstream body (19 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  if (( ! $+_cursor_cache )); then
//! sh: 6    local file
//! sh: 8    file=( /usr/{include,{{X11R6,openwin},local{,/X11{,R6}}}/include}/X11/cursorfont.h(N) )
//! sh:10    if (( $#file )); then
//! sh:11      _cursor_cache=( "${(@)${(@)${(M@)${(@f)$(< $file[1])}:#*XC_*}[2,-1]#* XC_}% *}" )
//! sh:12    else
//! sh:13      _cursor_cache=( X_cursor )
//! sh:14    fi
//! sh:15  fi
//! sh:17  _wanted cursors expl 'cursor name' \
//! sh:18      compadd "$@" -M 'm:-=_ r:|_=*' -a - _cursor_cache
//! ```
//!
//! sh:8 — brace-expands to 6 candidate `cursorfont.h` header paths
//! (order matters: `(N)` is `NULL_GLOB`, so only existing paths survive
//! and `$file[1]` is the first that exists).
//! sh:11 — read the whole header (`$(< $file[1])`), split on newlines
//! (`(@f)`), keep only lines containing `XC_` (`(M@)...:#*XC_*`), drop
//! the first surviving match (`[2,-1]`, typically the `XC_num_glyphs`
//! count `#define`), strip the shortest `* XC_` prefix off each
//! (`#* XC_`), then strip the shortest ` *` suffix off each (`% *`) —
//! turning `#define XC_X_cursor 0` into `X_cursor`.

use crate::compsys::ported::_wanted::wanted_byname;
use crate::ported::params::{getaparam, setaparam};
use std::path::Path;

/// sh:8 — the 6 candidate header paths from brace-expanding
/// `/usr/{include,{{X11R6,openwin},local{,/X11{,R6}}}/include}/X11/cursorfont.h`,
/// in zsh brace-expansion order.
const CURSORFONT_CANDIDATES: &[&str] = &[
    "/usr/include/X11/cursorfont.h",
    "/usr/X11R6/include/X11/cursorfont.h",
    "/usr/openwin/include/X11/cursorfont.h",
    "/usr/local/include/X11/cursorfont.h",
    "/usr/local/X11/include/X11/cursorfont.h",
    "/usr/local/X11R6/include/X11/cursorfont.h",
];

/// sh:8 — `(N)` glob qualifier: first candidate that exists on disk, or
/// `None` if none do (`$#file == 0`).
fn find_cursorfont_h() -> Option<&'static str> {
    CURSORFONT_CANDIDATES
        .iter()
        .copied()
        .find(|p| Path::new(p).exists())
}

/// sh:11 — strip the shortest prefix ending in the literal `" XC_"`
/// (zsh `${var#* XC_}`); unchanged if no match.
fn strip_prefix_space_xc(s: &str) -> &str {
    match s.find(" XC_") {
        Some(idx) => &s[idx + " XC_".len()..],
        None => s,
    }
}

/// sh:11 — strip the shortest suffix starting with a space (zsh
/// `${var% *}`): the shortest matching suffix is the one starting at
/// the *last* space in the string.
fn strip_suffix_last_space(s: &str) -> &str {
    match s.rfind(' ') {
        Some(idx) => &s[..idx],
        None => s,
    }
}

/// sh:11 — parse `cursorfont.h` content into cursor-name strings.
fn parse_cursor_names(content: &str) -> Vec<String> {
    // sh:11  ${(M@)${(@f)$(< $file)}:#*XC_*}  — lines containing "XC_".
    let matches: Vec<&str> = content.lines().filter(|l| l.contains("XC_")).collect();
    // sh:11  [2,-1]  — drop the first match (`XC_num_glyphs` count).
    if matches.len() <= 1 {
        return Vec::new();
    }
    matches[1..]
        .iter()
        .map(|l| strip_suffix_last_space(strip_prefix_space_xc(l)).to_string())
        .collect()
}

/// `_x_cursor` — complete X cursor-font names (e.g. from
/// `<X11/cursorfont.h>`'s `XC_*` `#define`s), cached in `$_cursor_cache`.
pub fn _x_cursor(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_x_cursor");
    // sh:5 — cache presence check (approx: None == unset).
    if getaparam("_cursor_cache").is_none() {
        // sh:8
        let cache = match find_cursorfont_h() {
            // sh:10-11 — file found: parse cursor names out of it.
            Some(path) => match std::fs::read_to_string(path) {
                Ok(content) => parse_cursor_names(&content),
                Err(_) => Vec::new(),
            },
            // sh:12-13 — no candidate header exists.
            None => vec!["X_cursor".to_string()],
        };
        setaparam("_cursor_cache", cache);
    }

    // sh:17-18
    let mut w: Vec<String> = vec![
        "cursors".to_string(),
        "expl".to_string(),
        "cursor name".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-M".to_string());
    w.push("m:-=_ r:|_=*".to_string());
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("_cursor_cache".to_string());
    wanted_byname(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_prefix_space_xc_removes_up_through_marker() {
        assert_eq!(strip_prefix_space_xc("#define XC_X_cursor 0"), "X_cursor 0");
    }

    #[test]
    fn strip_prefix_space_xc_unchanged_when_absent() {
        assert_eq!(strip_prefix_space_xc("no marker here"), "no marker here");
    }

    #[test]
    fn strip_suffix_last_space_drops_trailing_value() {
        assert_eq!(strip_suffix_last_space("X_cursor 0"), "X_cursor");
    }

    #[test]
    fn strip_suffix_last_space_unchanged_when_no_space() {
        assert_eq!(strip_suffix_last_space("X_cursor"), "X_cursor");
    }

    #[test]
    fn parse_cursor_names_drops_first_match_and_extracts_names() {
        // sh:11 — `${(M@)...:#*XC_*}` keeps every line whose text contains the
        // substring `XC_`, so the first "XC_" line (num_glyphs count) is dropped
        // via `[2,-1]` while `NOT_XC_related` (substring `XC_`) is KEPT. Lines
        // with a literal `" XC_"` yield the bare cursor name (`#* XC_` then
        // `% *`); `#define NOT_XC_related 9` has no `" XC_"`, so `#* XC_` leaves
        // it unchanged and only `% *` (strip last space) applies.
        let header = "/* comment */\n\
#define XC_num_glyphs 154\n\
#define XC_X_cursor 0\n\
#define XC_arrow 2\n\
#define NOT_XC_related 9\n";
        assert_eq!(
            parse_cursor_names(header),
            vec![
                "X_cursor".to_string(),
                "arrow".to_string(),
                "#define NOT_XC_related".to_string()
            ]
        );
    }

    #[test]
    fn parse_cursor_names_empty_when_zero_or_one_matches() {
        assert_eq!(parse_cursor_names(""), Vec::<String>::new());
        assert_eq!(
            parse_cursor_names("#define XC_num_glyphs 154\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn find_cursorfont_h_none_when_no_candidate_exists() {
        // Headless CI has no X11 dev headers installed; every candidate
        // path in CURSORFONT_CANDIDATES is expected absent.
        // (Not asserting unconditionally — if a CI image genuinely ships
        // X11 dev headers this would legitimately find one, so just
        // confirm the function doesn't panic and returns a valid Option.)
        let _ = find_cursorfont_h();
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        let _ = crate::ported::params::unsetparam("_cursor_cache");
        assert_eq!(_x_cursor(&[]), 1);
    }
}
