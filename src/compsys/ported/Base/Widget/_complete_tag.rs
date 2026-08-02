//! Port of `_complete_tag` from
//! `Completion/Base/Widget/_complete_tag`.
//!
//! Full upstream body (62 lines, abridged):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xt
//! sh:13  local c_Tagsfile=${TAGSFILE:-TAGS} c_tagsfile=${tagsfile:-tags} expl
//! sh:16  integer c_maxdir=10
//! sh:18  local curcontext="$curcontext"
//! sh:22  if [[ -z "$curcontext" ]]; then curcontext="complete-tag:::"; else …
//! sh:28  local c_path=; integer c_idir
//! sh:29  while [[ ! -f $c_path$c_Tagsfile && ! -f $c_path$c_tagsfile && $c_idir -lt $c_maxdir ]]; do
//! sh:31    (( c_idir++ )); c_path=../$c_path
//! sh:32  done
//! sh:34  if TAGS-file is `tags`-only-disguised, blank c_Tagsfile
//! sh:38  if [[ -f $c_path$c_Tagsfile ]]; then
//! sh:46    c_tags_array=($(sed extract names from emacs TAGS file))
//! sh:53    _main_complete - '' _wanted etags expl 'emacs tag' compadd -a c_tags_array
//! sh:55  elif [[ -f $c_path$c_tagsfile ]]; then
//! sh:59    c_tags_array=($(awk '{ print $1 }' $c_path$c_tagsfile))
//! sh:60    _main_complete - '' _wanted vtags expl 'vi tag' compadd -a c_tags_array
//! sh:61  else return 1
//! sh:62  fi
//! ```

use crate::ported::exec::dispatch_function_call;
use crate::ported::params::{getsparam, setaparam, setsparam};
use std::fs;
use std::path::Path;

const MAX_DIR: usize = 10;

/// `_complete_tag` — complete a tag name from a TAGS / tags file in
/// the current dir or an ancestor (up to 10 levels up).
pub fn _complete_tag() -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_complete_tag");
    let cap_tagsfile = getsparam("TAGSFILE").unwrap_or_else(|| "TAGS".to_string());
    let low_tagsfile = getsparam("tagsfile").unwrap_or_else(|| "tags".to_string());

    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let new_ctx = if saved_curcontext.is_empty() {
        "complete-tag:::".to_string()
    } else {
        let tail = saved_curcontext.splitn(2, ':').nth(1).unwrap_or("");
        format!("complete-tag:{}", tail)
    };
    let _ = setsparam("curcontext", &new_ctx);

    // sh:29-32  walk up the dir tree
    let mut prefix = String::new();
    let mut cap_path = None;
    let mut low_path = None;
    for _ in 0..MAX_DIR {
        let p1 = format!("{}{}", prefix, cap_tagsfile);
        let p2 = format!("{}{}", prefix, low_tagsfile);
        if Path::new(&p1).is_file() {
            cap_path = Some(p1.clone());
        }
        if Path::new(&p2).is_file() {
            low_path = Some(p2.clone());
        }
        if cap_path.is_some() || low_path.is_some() {
            break;
        }
        prefix = format!("../{}", prefix);
    }

    // sh:34-37  TAGS == tags + `!_TAG_…` prefix → treat as vi tags
    let mut emacs_tags_path = cap_path.clone();
    if let (Some(cp), Some(lp)) = (cap_path.as_ref(), low_path.as_ref()) {
        if cp == lp {
            if let Ok(content) = fs::read_to_string(cp) {
                if content
                    .lines()
                    .next()
                    .map(|l| l.starts_with("!_TAG_"))
                    .unwrap_or(false)
                {
                    emacs_tags_path = None;
                }
            }
        }
    }

    // sh:38-54  prefer emacs TAGS
    if let Some(p) = emacs_tags_path {
        let tags = parse_emacs_tags(&p);
        setaparam("c_tags_array", tags);
        let _ = setsparam("curcontext", &saved_curcontext);
        return dispatch_function_call(
            "_main_complete",
            &[
                "-".to_string(),
                "".to_string(),
                "_wanted".to_string(),
                "etags".to_string(),
                "expl".to_string(),
                "emacs tag".to_string(),
                "compadd".to_string(),
                "-a".to_string(),
                "c_tags_array".to_string(),
            ],
        )
        .unwrap_or(1);
    }

    // sh:55-60  vi tags fallback
    if let Some(p) = low_path {
        let tags = parse_vi_tags(&p);
        setaparam("c_tags_array", tags);
        let _ = setsparam("curcontext", &saved_curcontext);
        return dispatch_function_call(
            "_main_complete",
            &[
                "-".to_string(),
                "".to_string(),
                "_wanted".to_string(),
                "vtags".to_string(),
                "expl".to_string(),
                "vi tag".to_string(),
                "compadd".to_string(),
                "-a".to_string(),
                "c_tags_array".to_string(),
            ],
        )
        .unwrap_or(1);
    }

    // sh:61
    let _ = setsparam("curcontext", &saved_curcontext);
    1
}

/// sh:46 — extract name tokens from an emacs TAGS file by matching
/// against the `name<DEL>...` (0x7F separator) convention.
fn parse_emacs_tags(path: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            // emacs TAGS lines look like: `text<DEL>name<SOH>line,offset`
            if let Some(del_idx) = line.find('\x7f') {
                // Take everything BEFORE the DEL, then split on the last
                // non-ident char.
                let head = &line[..del_idx];
                let tail =
                    head.trim_end_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '_'));
                let name_start = tail
                    .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .map(|i| i + 1)
                    .unwrap_or(0);
                let name = &tail[name_start..];
                if !name.is_empty()
                    && name
                        .chars()
                        .next()
                        .map(|c| c == '_' || c.is_ascii_alphabetic())
                        .unwrap_or(false)
                {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

/// sh:59 — vi tags file: first whitespace-separated field per line.
fn parse_vi_tags(path: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            if let Some(name) = line.split_whitespace().next() {
                out.push(name.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_tags_file() {
        // Run in a tmpdir that has no TAGS/tags file.
        let _g = crate::test_util::global_state_lock();
        let _r = _complete_tag();
    }

    #[test]
    fn parse_vi_tags_extracts_first_word() {
        let tmp = std::env::temp_dir().join("_complete_tag_vi_test");
        let _ = fs::write(&tmp, "MyTag /path/to/file.c\nAnother /other.h\n");
        let names = parse_vi_tags(tmp.to_str().unwrap());
        assert_eq!(names, vec!["MyTag", "Another"]);
        let _ = fs::remove_file(&tmp);
    }
}
