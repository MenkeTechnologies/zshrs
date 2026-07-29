//! Port of `_x_color` from `Completion/X/Type/_x_color`.
//!
//! Full upstream body (37 lines, abridged — the head is a usage comment):
//! ```text
//! sh: 1  #autoload
//! sh:11  local expl
//! sh:13  if (( ! $+_cache_x_colors )); then
//! sh:14    typeset -ga _cache_x_colors; local file
//! sh:19    zstyle -s ":completion:${curcontext}:colors" path file
//! sh:20    if [[ -n "$file" ]]; then
//! sh:21      _cache_x_colors=( "${(@)${(@f)$(< $file)}[2,-1]##*\t\t}" )
//! sh:22    elif (( $+commands[showrgb] )); then
//! sh:23      _cache_x_colors=( "${(@)${(@)${(@f)$(_call_program colors showrgb)}[2,-1]##*\t\t}:#* *}" )
//! sh:24    else
//! sh:25      file=( /usr/{lib,{{X11R6,openwin},local{,/X11{,R6}}}/lib}/X11/rgb.txt(N) )
//! sh:27      (( $#file )) &&
//! sh:28          _cache_x_colors=( "${(@)${(@)${(@f)$(< $file[1])}[2,-1]##*\t\t}:#* *}" )
//! sh:29    fi
//! sh:33    (( $#_cache_x_colors )) || _cache_x_colors=(white black gray red blue green)
//! sh:34  fi
//! sh:36  _wanted colors expl 'color specification' compadd "$@" -M \
//! sh:37      'm:{a-z}={A-Z} m:-=\<SP> r:[^ A-Z0-9]||[ A-Z0-9]=* r:|=*' -a - _cache_x_colors
//! ```
//!
//! `_cache_x_colors` is a global array cache (`typeset -ga`), populated
//! exactly once per shell session: from a `colors_path`-style-configured
//! rgb.txt-format file (sh:19-21, no whitespace filter), else from
//! `showrgb`'s output (sh:22-23, whitespace-filtered), else by globbing
//! the well-known X11 `rgb.txt` install locations (sh:25-28,
//! whitespace-filtered), else a hardcoded 6-color fallback (sh:33).
//! `${line##*\t\t}` (sh:21/23/28) keeps only the text after the LAST
//! double-TAB in each line — rgb.txt format is `R G B<TAB><TAB>...name`.
//! `-a - _cache_x_colors` (sh:37) tells `compadd` to expand the NAMED
//! array parameter's values as the candidate matches (`CAF_ARRAYS`,
//! resolved by `compcore.rs`'s `bin_addmatches`), so the cache array is
//! populated via a real `setaparam` and only the literal argv tail is
//! forwarded — `bin_compadd` does the expansion itself.

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, getsparam, setaparam};

/// sh:25 — the brace-expanded (left-to-right) candidate `rgb.txt`
/// install paths: `/usr/{lib,{{X11R6,openwin},local{,/X11{,R6}}}/lib}/X11/rgb.txt`.
const RGB_TXT_CANDIDATES: &[&str] = &[
    "/usr/lib/X11/rgb.txt",
    "/usr/X11R6/lib/X11/rgb.txt",
    "/usr/openwin/lib/X11/rgb.txt",
    "/usr/local/lib/X11/rgb.txt",
    "/usr/local/X11/lib/X11/rgb.txt",
    "/usr/local/X11R6/lib/X11/rgb.txt",
];

/// sh:22 — `(( $+commands[showrgb] ))`: is `showrgb` a hashed command?
/// Mirrors the identical `has_command` helper in `_users_on.rs`.
fn has_command(name: &str) -> bool {
    getaparam("commands")
        .unwrap_or_default()
        .chunks(2)
        .any(|kv| kv.first().map(|k| k == name).unwrap_or(false))
}

/// sh:21/23/28 — `${line##*\t\t}`: strip the longest prefix ending in
/// two literal TAB characters (a zsh glob, greedy from the left), i.e.
/// keep only the text after the LAST `"\t\t"` occurrence. Unchanged if
/// no double-TAB is present (the glob simply doesn't match).
fn strip_double_tab_prefix(line: &str) -> String {
    match line.rfind("\t\t") {
        Some(idx) => line[idx + 2..].to_string(),
        None => line.to_string(),
    }
}

/// sh:21/23/28 — `${(@f)text}[2,-1]` (drop the header line) mapped
/// through [`strip_double_tab_prefix`], then, when `filter_spaces`,
/// `:#* *` (drop any element containing a literal space — multi-word
/// color names) as done for the `showrgb`/`rgb.txt` sources but NOT
/// for the style-configured `path` source (sh:21 lacks the filter).
fn parse_rgb_text(text: &str, filter_spaces: bool) -> Vec<String> {
    let stripped: Vec<String> = text
        .lines()
        .skip(1) // sh:[2,-1] — drop the header line.
        .map(strip_double_tab_prefix)
        .collect();
    if filter_spaces {
        stripped.into_iter().filter(|s| !s.contains(' ')).collect()
    } else {
        stripped
    }
}

/// sh:25-28 — first existing candidate `rgb.txt` path (`(N)` = nullglob:
/// no error, no elements, if none exist), in brace-expansion order.
fn find_rgb_txt() -> Option<&'static str> {
    RGB_TXT_CANDIDATES
        .iter()
        .copied()
        .find(|p| std::path::Path::new(p).exists())
}

/// sh:13-34 — populate the `_cache_x_colors` global-array cache the
/// first time this function runs in the shell session.
fn populate_cache_if_unset() {
    // sh:13  (( ! $+_cache_x_colors ))
    if getaparam("_cache_x_colors").is_some() {
        return;
    }

    let curcontext = getsparam("curcontext").unwrap_or_default();

    // sh:19  zstyle -s ":completion:${curcontext}:colors" path file
    let style_ctx = format!(":completion:{}:colors", curcontext);
    let file = crate::ported::modules::zutil::lookupstyle(&style_ctx, "path")
        .into_iter()
        .next()
        .unwrap_or_default();

    let mut cache: Vec<String> = if !file.is_empty() {
        // sh:20-21 — no `:#* *` whitespace filter on this source.
        let raw = std::fs::read_to_string(&file).unwrap_or_default();
        parse_rgb_text(&raw, false)
    } else if has_command("showrgb") {
        // sh:22-23
        let _ = _call_program(&["colors".to_string(), "showrgb".to_string()]);
        let raw = getsparam("REPLY").unwrap_or_default();
        parse_rgb_text(&raw, true)
    } else if let Some(path) = find_rgb_txt() {
        // sh:25-28
        let raw = std::fs::read_to_string(path).unwrap_or_default();
        parse_rgb_text(&raw, true)
    } else {
        Vec::new()
    };

    // sh:33 — stupid default value.
    if cache.is_empty() {
        cache = ["white", "black", "gray", "red", "blue", "green"]
            .iter()
            .map(|s| s.to_string())
            .collect();
    }

    setaparam("_cache_x_colors", cache);
}

/// `_x_color` — complete an X11 color name from a cached list sourced
/// (in priority order) from a style-configured file, `showrgb`'s
/// output, the system `rgb.txt`, or a 6-color hardcoded fallback.
pub fn _x_color(args: &[String]) -> i32 {
    // sh:13-34
    populate_cache_if_unset();

    // sh:36-37  _wanted colors expl 'color specification' compadd "$@" \
    //             -M 'm:{a-z}={A-Z} m:-=\<SP> r:[^ A-Z0-9]||[ A-Z0-9]=* r:|=*' \
    //             -a - _cache_x_colors
    let mut w: Vec<String> = vec![
        "colors".to_string(),
        "expl".to_string(),
        "color specification".to_string(),
        "compadd".to_string(),
    ];
    w.extend(args.iter().cloned());
    w.push("-M".to_string());
    w.push("m:{a-z}={A-Z} m:-=\\  r:[^ A-Z0-9]||[ A-Z0-9]=* r:|=*".to_string());
    w.push("-a".to_string());
    w.push("-".to_string());
    w.push("_cache_x_colors".to_string());
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_double_tab_prefix_keeps_text_after_last_double_tab() {
        // sh:21/23/28 — rgb.txt format: "R G B\t\tname". The glob
        // `*\t\t` is greedy, so with a run of 3 tabs the match ends at
        // the LAST two of the run, leaving no tab before the name.
        assert_eq!(strip_double_tab_prefix("255 250 250\t\t\tsnow"), "snow");
        assert_eq!(strip_double_tab_prefix("255 250 250\t\tsnow"), "snow");
    }

    #[test]
    fn strip_double_tab_prefix_unchanged_without_double_tab() {
        // Glob `*\t\t` doesn't match -> `##` removes nothing.
        assert_eq!(strip_double_tab_prefix("no tabs here"), "no tabs here");
    }

    #[test]
    fn parse_rgb_text_drops_header_and_strips_fields() {
        let text = "! header comment\n255 250 250\t\tsnow\n0 0 0\t\tblack\n";
        assert_eq!(
            parse_rgb_text(text, false),
            vec!["snow".to_string(), "black".to_string()]
        );
    }

    #[test]
    fn parse_rgb_text_filter_spaces_drops_multiword_names() {
        // sh:23/28 — `:#* *` drops entries containing a literal space.
        let text = "! header\n1 2 3\t\tsnow\n4 5 6\t\tdark blue\n";
        assert_eq!(parse_rgb_text(text, true), vec!["snow".to_string()]);
        // sh:21 (no filter) keeps the multi-word entry.
        assert_eq!(
            parse_rgb_text(text, false),
            vec!["snow".to_string(), "dark blue".to_string()]
        );
    }

    #[test]
    fn has_command_false_when_commands_array_empty() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setaparam("commands", Vec::new());
        assert!(!has_command("showrgb"));
    }

    #[test]
    fn has_command_true_when_present_in_commands_array() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setaparam(
            "commands",
            vec!["showrgb".to_string(), "/usr/bin/showrgb".to_string()],
        );
        assert!(has_command("showrgb"));
    }

    #[test]
    fn populate_cache_falls_back_to_stupid_default_without_any_source() {
        // sh:33 — no style path, no `showrgb`, no rgb.txt found on this
        // (headless-CI) box -> the hardcoded 6-color default.
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("_cache_x_colors");
        crate::ported::params::setaparam("commands", Vec::new());
        crate::ported::params::setsparam("curcontext", ":completion::::");
        populate_cache_if_unset();
        assert_eq!(
            getaparam("_cache_x_colors"),
            Some(vec![
                "white".to_string(),
                "black".to_string(),
                "gray".to_string(),
                "red".to_string(),
                "blue".to_string(),
                "green".to_string(),
            ])
        );
    }

    #[test]
    fn populate_cache_is_a_noop_once_already_set() {
        // sh:13 — `$+_cache_x_colors` existence check short-circuits.
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setaparam("_cache_x_colors", vec!["custom".to_string()]);
        populate_cache_if_unset();
        assert_eq!(
            getaparam("_cache_x_colors"),
            Some(vec!["custom".to_string()])
        );
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::unsetparam("_cache_x_colors");
        crate::ported::params::setaparam("commands", Vec::new());
        crate::ported::params::setsparam("curcontext", ":completion::::");
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_x_color(&[]), 1);
    }
}
