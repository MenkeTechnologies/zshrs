//! Port of `_xft_fonts` from `Completion/X/Type/_xft_fonts`.
//!
//! Full upstream body (43 lines verbatim):
//! ```text
//! sh: 1  #compdef fc-list fc-match
//! sh: 3  local -a expl suf
//! sh: 4  local font=${${PREFIX//-[0-9]##:/:}%:*}: ret=1
//! sh: 5  local attr
//! sh: 7  compset -S ':*' || suf=( -S: -r "-: \t\n\-" )
//! sh: 8  if compset -P '*:'; then
//! sh: 9    attr="${PREFIX%\=*}"
//! sh:10    if compset -P '*='; then
//! sh:11      case $attr in
//! sh:12      hintstyle)
//! sh:13        _wanted value expl 'value' compadd "$suf[@]" \
//! sh:14            hint{none,slight,medium,full} && ret=0
//! sh:16      *)
//! sh:17        _wanted value expl 'value' compadd "$suf[@]" \
//! sh:18            ${${(f)"$(_call_program font-attrs
//! sh:19            fc-list $font $attr 2>/dev/null)"//,/$'\n'}##*=} && ret=0
//! sh:21      esac
//! sh:22    else
//! sh:23      _tags elements {weight,slant}-constants
//! sh:24      while _tags; do
//! sh:25        _requested elements expl element compadd -qS= hintstyle hinting autohint \
//! sh:26            size ${${(u)${(M)${(f)"$(_call_program elements
//! sh:27            fc-list -v $font 2>/dev/null)"}:#	[a-z]*}%%:*}#?} && ret=0
//! sh:28        _requested weight-constants expl 'weight constant' compadd "$suf[@]" \
//! sh:29            thin bold regular medium semibold heavy roman && ret=0
//! sh:30        _requested slant-constants expl 'slant constant' compadd "$suf[@]" \
//! sh:31            roman italic oblique && ret=0
//! sh:33        (( ret )) || break
//! sh:34      done
//! sh:35    fi
//! sh:36  elif compset -P '*[^\\]-'; then
//! sh:37    _message -e size 'point size' && ret=0
//! sh:38  else
//! sh:39    _wanted fonts expl font compadd "$suf[@]" \
//! sh:40        ${(us:,:)$(_call_program fonts fc-list -f '%\{family\},' 2>/dev/null)} && ret=0
//! sh:41  fi
//! sh:43  return ret
//! ```
//!
//! `compset -P/-S` dispatch to the real `bin_compset` (mutating the real
//! `PREFIX`/`SUFFIX`/`IPREFIX` params, exactly like the shell builtin).
//! `_call_program` output is read back from `$REPLY` (sh:18-19/27/40),
//! then run through the same `(f)`/`:#`/`%%`/`#`/`(u)`/`(s:,:)` string
//! transforms the upstream expansions perform.

use crate::compsys::ported::_call_program::call_program_capture;
use crate::compsys::ported::_message::_message;
use crate::compsys::ported::_requested::_requested;
use crate::compsys::ported::_tags::_tags;
use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::getsparam;
use crate::ported::zle::complete::bin_compset;
use crate::ported::zsh_h::{options, MAX_OPS};
use regex::Regex;
use std::sync::OnceLock;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

fn compset(argv: &[&str]) -> i32 {
    let v: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    bin_compset("compset", &v, &make_ops(), 0)
}

fn get(name: &str) -> String {
    getsparam(name).unwrap_or_default()
}

/// sh:4 — matches `-[0-9]##:` (a `-`, one-or-more digits, `:`), the
/// point-size marker zsh's own completion inserts between family name
/// and attribute list (e.g. `DejaVu Sans-12:`).
fn size_marker() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"-[0-9]+:").unwrap())
}

/// sh:4 — `${${PREFIX//-[0-9]##:/:}%:*}:` — collapse any `-<digits>:`
/// size marker back to a bare `:`, drop the last `:`-delimited field
/// (the piece still being typed), then re-append the literal `:`.
fn compute_font(prefix: &str) -> String {
    let collapsed = size_marker().replace_all(prefix, ":").into_owned();
    // `%:*` removes the shortest suffix matching `:*`, i.e. everything
    // from the LAST `:` onward.
    let trimmed = match collapsed.rfind(':') {
        Some(idx) => collapsed[..idx].to_string(),
        None => collapsed,
    };
    format!("{}:", trimmed)
}

/// sh:9 — `${PREFIX%\=*}` — strip the shortest `=...` suffix, i.e.
/// everything from the LAST `=` onward.
fn compute_attr(prefix: &str) -> String {
    match prefix.rfind('=') {
        Some(idx) => prefix[..idx].to_string(),
        None => prefix.to_string(),
    }
}

/// sh:18-19 — `${${(f)"$(...)"}//,/$'\n'}##*=}`: split the captured
/// text into lines, turn every `,` into an embedded newline, then for
/// each line keep only the text after the LAST `=`.
fn parse_font_attr_text(text: &str) -> Vec<String> {
    text.trim_end_matches('\n')
        .lines()
        .map(|line| {
            let repl = line.replace(',', "\n");
            match repl.rfind('=') {
                Some(idx) => repl[idx + 1..].to_string(),
                None => repl,
            }
        })
        .collect()
}

/// sh:18-19 — `_call_program font-attrs fc-list $font $attr`.
fn font_attr_values(font: &str, attr: &str) -> Vec<String> {
    let _ = call_program_capture(&[
        "font-attrs".to_string(),
        "fc-list".to_string(),
        font.to_string(),
        attr.to_string(),
    ]);
    parse_font_attr_text(&get("REPLY"))
}

/// sh:27 — `${${(u)${(M)${(f)"$(...)"}:#	[a-z]*}%%:*}#?}`: keep only
/// lines that start with a literal tab followed by a lowercase letter,
/// strip the suffix from the first `:` onward, dedupe (first
/// occurrence wins), then drop the leading tab character.
fn parse_elements_text(text: &str) -> Vec<String> {
    let filtered: Vec<&str> = text
        .trim_end_matches('\n')
        .lines()
        .filter(|line| {
            let mut c = line.chars();
            c.next() == Some('\t') && c.next().map(|ch| ch.is_ascii_lowercase()).unwrap_or(false)
        })
        .collect();
    let stripped: Vec<String> = filtered
        .into_iter()
        .map(|line| match line.find(':') {
            Some(idx) => line[..idx].to_string(),
            None => line.to_string(),
        })
        .collect();
    let mut seen = std::collections::HashSet::new();
    stripped
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .map(|s| s.chars().skip(1).collect())
        .collect()
}

/// sh:27 — `_call_program elements fc-list -v $font`.
fn compute_elements(font: &str) -> Vec<String> {
    let _ = call_program_capture(&[
        "elements".to_string(),
        "fc-list".to_string(),
        "-v".to_string(),
        font.to_string(),
    ]);
    parse_elements_text(&get("REPLY"))
}

/// sh:40 — `${(us:,:)$(...)}`: split the captured text on `,`, then
/// dedupe (first occurrence wins).
fn parse_fonts_text(text: &str) -> Vec<String> {
    let trimmed = text.trim_end_matches('\n');
    let mut seen = std::collections::HashSet::new();
    trimmed
        .split(',')
        .map(|s| s.to_string())
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

/// sh:40 — `_call_program fonts fc-list -f '%\{family\},'`.
fn compute_fonts() -> Vec<String> {
    let _ = call_program_capture(&[
        "fonts".to_string(),
        "fc-list".to_string(),
        "-f".to_string(),
        "%\\{family\\},".to_string(),
    ]);
    parse_fonts_text(&get("REPLY"))
}

/// `_xft_fonts` — complete Xft font names / attributes / point sizes
/// for `fc-list`/`fc-match` (the `family[-size][:attr[=value]]*`
/// grammar).
pub fn _xft_fonts(_args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_xft_fonts");
    // sh:4
    let font = compute_font(&get("PREFIX"));
    let mut ret = 1;

    // sh:7
    let suf: Vec<String> = if compset(&["-S", ":*"]) != 0 {
        vec![
            "-S:".to_string(),
            "-r".to_string(),
            "-: \t\n\\-".to_string(),
        ]
    } else {
        Vec::new()
    };

    // sh:8
    if compset(&["-P", "*:"]) == 0 {
        // sh:9
        let attr = compute_attr(&get("PREFIX"));
        // sh:10
        if compset(&["-P", "*="]) == 0 {
            // sh:11-22
            match attr.as_str() {
                "hintstyle" => {
                    // sh:12-15
                    let mut w: Vec<String> = vec![
                        "value".to_string(),
                        "expl".to_string(),
                        "value".to_string(),
                        "compadd".to_string(),
                    ];
                    w.extend(suf.clone());
                    w.extend(
                        ["hintnone", "hintslight", "hintmedium", "hintfull"]
                            .iter()
                            .map(|s| s.to_string()),
                    );
                    if _wanted(&w) == 0 {
                        ret = 0;
                    }
                }
                _ => {
                    // sh:16-21
                    let mut w: Vec<String> = vec![
                        "value".to_string(),
                        "expl".to_string(),
                        "value".to_string(),
                        "compadd".to_string(),
                    ];
                    w.extend(suf.clone());
                    w.extend(font_attr_values(&font, &attr));
                    if _wanted(&w) == 0 {
                        ret = 0;
                    }
                }
            }
        } else {
            // sh:23 — register the tag set.
            let _ = _tags(&[
                "elements".to_string(),
                "weight-constants".to_string(),
                "slant-constants".to_string(),
            ]);
            // sh:25-34
            while _tags(&[]) == 0 {
                // sh:25-26
                let mut r1: Vec<String> = vec![
                    "elements".to_string(),
                    "expl".to_string(),
                    "element".to_string(),
                    "compadd".to_string(),
                    "-qS=".to_string(),
                    "hintstyle".to_string(),
                    "hinting".to_string(),
                    "autohint".to_string(),
                    "size".to_string(),
                ];
                r1.extend(compute_elements(&font));
                if _requested(&r1) == 0 {
                    ret = 0;
                }
                // sh:28-29
                let mut r2: Vec<String> = vec![
                    "weight-constants".to_string(),
                    "expl".to_string(),
                    "weight constant".to_string(),
                    "compadd".to_string(),
                ];
                r2.extend(suf.clone());
                r2.extend(
                    [
                        "thin", "bold", "regular", "medium", "semibold", "heavy", "roman",
                    ]
                    .iter()
                    .map(|s| s.to_string()),
                );
                if _requested(&r2) == 0 {
                    ret = 0;
                }
                // sh:30-31
                let mut r3: Vec<String> = vec![
                    "slant-constants".to_string(),
                    "expl".to_string(),
                    "slant constant".to_string(),
                    "compadd".to_string(),
                ];
                r3.extend(suf.clone());
                r3.extend(["roman", "italic", "oblique"].iter().map(|s| s.to_string()));
                if _requested(&r3) == 0 {
                    ret = 0;
                }
                // sh:33
                if ret == 0 {
                    break;
                }
            }
        }
    } else if compset(&["-P", r"*[^\\]-"]) == 0 {
        // sh:36-37
        if _message(&[
            "-e".to_string(),
            "size".to_string(),
            "point size".to_string(),
        ]) == 0
        {
            ret = 0;
        }
    } else {
        // sh:38-40
        let mut w: Vec<String> = vec![
            "fonts".to_string(),
            "expl".to_string(),
            "font".to_string(),
            "compadd".to_string(),
        ];
        w.extend(suf.clone());
        w.extend(compute_fonts());
        if _wanted(&w) == 0 {
            ret = 0;
        }
    }

    // sh:43
    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_font_collapses_size_marker_and_drops_last_field() {
        // sh:4 — a `-<digits>:` size marker collapses to a bare `:`;
        // the last `:`-field (still being typed) is then dropped and a
        // literal `:` re-appended.
        assert_eq!(
            compute_font("DejaVu Sans-12:style=Bold:hint"),
            "DejaVu Sans:style=Bold:"
        );
        assert_eq!(compute_font("DejaVu Sans"), "DejaVu Sans:");
        assert_eq!(compute_font(""), ":");
    }

    #[test]
    fn compute_attr_strips_last_equals_field() {
        // sh:9 — `${PREFIX%\=*}` keeps everything before the LAST `=`.
        assert_eq!(compute_attr("style=Bold"), "style");
        assert_eq!(compute_attr("hintstyle"), "hintstyle");
        assert_eq!(compute_attr("a=b=c"), "a=b");
    }

    #[test]
    fn parse_font_attr_text_splits_commas_and_keeps_tail_after_last_equals() {
        // sh:18-19
        let got = parse_font_attr_text("style=Regular,Bold\nweight=80\n");
        assert_eq!(got, vec!["Regular\nBold".to_string(), "80".to_string()]);
    }

    #[test]
    fn parse_font_attr_text_line_without_equals_is_kept_whole() {
        let got = parse_font_attr_text("noequalshere\n");
        assert_eq!(got, vec!["noequalshere".to_string()]);
    }

    #[test]
    fn parse_elements_text_filters_tab_lowercase_lines_and_dedupes() {
        // sh:27 — only tab+lowercase lines survive; "Hash: 42" (capital
        // H) is excluded; the two "family" lines dedupe to one.
        let text = "Pattern has 2 elts (size 4):\n\tfamily: \"DejaVu Sans\"(s)\n\tfamily: \"DejaVu\"(s)\n\tstyle: \"Book\"(s)\n\tHash: 42\n";
        let got = parse_elements_text(text);
        assert_eq!(got, vec!["family".to_string(), "style".to_string()]);
    }

    #[test]
    fn parse_elements_text_empty_input_is_empty() {
        assert_eq!(parse_elements_text(""), Vec::<String>::new());
    }

    #[test]
    fn parse_fonts_text_splits_and_dedupes() {
        // sh:40 — `${(us:,:)...}`: split on `,`, dedupe first-occurrence.
        let got = parse_fonts_text("DejaVu Sans,Arial,DejaVu Sans,");
        assert_eq!(
            got,
            vec![
                "DejaVu Sans".to_string(),
                "Arial".to_string(),
                "".to_string(),
            ]
        );
    }

    #[test]
    fn returns_one_without_completion_context() {
        // With INCOMPFUNC == 0, `bin_compset` (the real completion
        // builtin) refuses to run, so both `compset -P` checks fail and
        // the else branch's `_wanted`/`compadd` calls fail in turn —
        // `ret` stays at its initial value of 1.
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        let _ = crate::ported::params::setsparam("PREFIX", "");
        let _ = crate::ported::params::setsparam("SUFFIX", "");
        let _ = crate::ported::params::setsparam("IPREFIX", "");
        assert_eq!(_xft_fonts(&[]), 1);
    }
}
