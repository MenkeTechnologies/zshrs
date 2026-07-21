//! Port of `_x_keysym` from `Completion/X/Type/_x_keysym`.
//!
//! Full upstream body (23 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local expl
//! sh: 5  _tags keysyms || return 1
//! sh: 7  if (( ! $+_keysym_cache )); then
//! sh: 8    local file
//! sh:10    file=( /usr/{include,{{X11R6,openwin},local{,/X11{,R6}}}/include}/X11/keysymdef.h(N) )
//! sh:12    if (( $#file )); then
//! sh:13      _keysym_cache=( "${(@)${(@)${(M@)${(@f)$(< $file[1])}:#\#define[ 	]##XK_*}#\#define[ 	]##XK_}%%[ 	]*}" )
//! sh:14    else
//! sh:15      _keysym_cache=( BackSpace Tab Linefeed Clear Return Pause Escape
//! sh:16                     Delete Left Right Up Down Space Home Begin End
//! sh:17                     F{1,2,3,4,5,6,7,8,9,10,11,12} )
//! sh:18    fi
//! sh:19  fi
//! sh:21  _wanted keysyms expl 'key symbol' \
//! sh:22      compadd "$@" -M 'm:{a-z}={A-Z} r:|-=* r:|=*' -a - _keysym_cache
//! ```
//!
//! sh:10 — brace-expands to 6 candidate `keysymdef.h` locations (in
//! this exact order): `/usr/include`, `/usr/X11R6/include`,
//! `/usr/openwin/include`, `/usr/local/include`,
//! `/usr/local/X11/include`, `/usr/local/X11R6/include`, each
//! joined with `/X11/keysymdef.h`. `(N)` is the null-glob qualifier
//! (no error, empty result if none exist); `$file[1]` picks the
//! first that exists.
//!
//! sh:13 — reads the header (`$(< $file[1])`), splits into lines
//! (`${(@f)...}`), keeps only lines matching the glob
//! `#define[ \t]##XK_*` (`M` flag = keep matched elements), strips
//! the `#define[ \t]##XK_` prefix (shortest match) off each kept
//! line, then truncates at the first following space/tab
//! (`%%[ \t]*` — a single whitespace char followed by anything,
//! longest suffix match starting at the first whitespace).

use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, setaparam};

/// sh:10 — the 6 `keysymdef.h` candidate paths, in brace-expansion
/// order. Pure/testable; filesystem existence check happens in
/// [`find_keysymdef_file`].
fn keysymdef_candidates() -> Vec<String> {
    [
        "include",
        "X11R6/include",
        "openwin/include",
        "local/include",
        "local/X11/include",
        "local/X11R6/include",
    ]
    .iter()
    .map(|mid| format!("/usr/{mid}/X11/keysymdef.h"))
    .collect()
}

/// sh:10, sh:12 — `file=( ...(N) )`; `(( $#file ))` then `$file[1]`:
/// first candidate path that actually exists, or `None` if none do
/// (mirrors the null-glob-then-empty-array case).
fn find_keysymdef_file() -> Option<String> {
    keysymdef_candidates()
        .into_iter()
        .find(|p| std::path::Path::new(p).exists())
}

/// sh:13 — for each header line, keep it only if it matches
/// `#define[ \t]##XK_*`, then strip the `#define[ \t]##XK_` prefix
/// and truncate at the first following space/tab.
fn parse_keysymdef(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            let after_define = line.strip_prefix("#define")?;
            let ws_len = after_define
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .count();
            if ws_len == 0 {
                return None; // sh:13 `[ \t]##` requires >=1 whitespace
            }
            let after_xk = after_define[ws_len..].strip_prefix("XK_")?;
            let end = after_xk
                .find(|c: char| c == ' ' || c == '\t')
                .unwrap_or(after_xk.len());
            Some(after_xk[..end].to_string())
        })
        .collect()
}

/// sh:15-17 — fallback keysym list when no `keysymdef.h` was found.
fn default_keysyms() -> Vec<String> {
    let mut v: Vec<String> = [
        "BackSpace", "Tab", "Linefeed", "Clear", "Return", "Pause", "Escape", "Delete", "Left",
        "Right", "Up", "Down", "Space", "Home", "Begin", "End",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for i in 1..=12 {
        v.push(format!("F{i}")); // sh:17  F{1,2,...,12}
    }
    v
}

/// sh:7-19 — build the `_keysym_cache` contents (only called when the
/// cache param is not yet set).
fn build_keysym_cache() -> Vec<String> {
    match find_keysymdef_file() {
        Some(path) => {
            // sh:13  $(< $file[1])
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            parse_keysymdef(&content)
        }
        None => default_keysyms(), // sh:14-18
    }
}

/// `_x_keysym` — complete an X keysym name (e.g. from `keysymdef.h`,
/// cached in the global `_keysym_cache` array).
pub fn _x_keysym(args: &[String]) -> i32 {
    // sh:5  _tags keysyms || return 1
    if _tags(&["keysyms".to_string()]) != 0 {
        return 1;
    }

    // sh:7-19  if (( ! $+_keysym_cache )); then ... fi
    if getaparam("_keysym_cache").is_none() {
        setaparam("_keysym_cache", build_keysym_cache());
    }

    // sh:21-22  _wanted keysyms expl 'key symbol' \
    //     compadd "$@" -M 'm:{a-z}={A-Z} r:|-=* r:|=*' -a - _keysym_cache
    let mut w: Vec<String> = vec![
        "keysyms".to_string(),
        "expl".to_string(),
        "key symbol".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-M".to_string());
    w.push("m:{a-z}={A-Z} r:|-=* r:|=*".to_string());
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("_keysym_cache".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysymdef_candidates_are_six_paths_in_brace_order() {
        assert_eq!(
            keysymdef_candidates(),
            vec![
                "/usr/include/X11/keysymdef.h".to_string(),
                "/usr/X11R6/include/X11/keysymdef.h".to_string(),
                "/usr/openwin/include/X11/keysymdef.h".to_string(),
                "/usr/local/include/X11/keysymdef.h".to_string(),
                "/usr/local/X11/include/X11/keysymdef.h".to_string(),
                "/usr/local/X11R6/include/X11/keysymdef.h".to_string(),
            ]
        );
    }

    #[test]
    fn parse_keysymdef_extracts_name_and_stops_at_first_prefix() {
        let raw = "\
/* comment line, not a #define */
#define XK_BackSpace                     0xff08  /* Back space */
#define XK_Tab\t\t\t\t0xff09
#define NotAKeysym                       1
#define XK_A                             0x0041
";
        assert_eq!(
            parse_keysymdef(raw),
            vec!["BackSpace".to_string(), "Tab".to_string(), "A".to_string()]
        );
    }

    #[test]
    fn parse_keysymdef_requires_whitespace_between_define_and_xk() {
        // sh:13 `#define[ \t]##XK_*` needs >=1 whitespace char between
        // `#define` and `XK_`; a directly-glued token doesn't match.
        assert_eq!(parse_keysymdef("#defineXK_Foo 1\n"), Vec::<String>::new());
    }

    #[test]
    fn parse_keysymdef_empty_input_yields_empty() {
        assert_eq!(parse_keysymdef(""), Vec::<String>::new());
    }

    #[test]
    fn default_keysyms_has_16_named_plus_f1_through_f12() {
        let d = default_keysyms();
        assert_eq!(d.len(), 28);
        assert_eq!(d[0], "BackSpace");
        assert_eq!(d[15], "End");
        assert_eq!(d[16], "F1");
        assert_eq!(d[27], "F12");
    }

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_x_keysym(&[]), 1);
    }
}
