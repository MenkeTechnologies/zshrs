//! Port of `_mime_types` from `Completion/Unix/Type/_mime_types`.
//!
//! Full upstream body (42 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh: 7  default_type_files=(~/.mime.types /etc/mime.types)
//! sh:10  if zstyle -a … mime-types type_files; then
//! sh:12    while (( (ind = ${type_files[(I)+]}) > 0 )); do
//! sh:12      type_files[$ind]=($default_type_files)   # splice defaults at `+`
//! sh:14  else type_files=($default_type_files)
//! sh:30  if [[ $PREFIX = (#b)([^/]##)/* ]]; then
//! sh:33    maintype=$match[1]; compset -p $(( ${#maintype} + 1 ))
//! sh:34    _wanted mime-subtypes … compadd -- $(sed … "s%^\(type=\|\)$maintype/\([^ \t]*\).*$%\2%p" …)
//! sh:37  else
//! sh:40    _wanted mime-types … compadd -S/ -- $(sed … "s/^type=//" "s%^\(${PREFIX:-[a-z]}[^=\"]*\)/.*$%\1%p" …)
//! sh:42  fi
//! ```
//!
//! sh:34/40 approx — the two `sed` extractions are reimplemented as
//! line-oriented parsing: strip an optional leading `type=`, then for the
//! main-type pass take the segment before the first `/` (filtered by the
//! `$PREFIX` prefix), and for the sub-type pass take the segment after
//! `maintype/` up to the first blank.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::modules::zutil::lookupstyle;
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

/// Expand a leading `~/` to `$HOME/`.
fn tilde_expand(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        let home = getsparam("HOME").unwrap_or_default();
        format!("{}/{}", home, rest)
    } else {
        p.to_string()
    }
}

/// Read every mime.types file, returning its lines with any leading `type=`
/// stripped (sh:40 `s/^type=//`).
fn read_type_lines(files: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for f in files {
        if let Ok(text) = std::fs::read_to_string(tilde_expand(f)) {
            for line in text.lines() {
                out.push(line.strip_prefix("type=").unwrap_or(line).to_string());
            }
        }
    }
    out
}

/// `_mime_types` — complete MIME type / subtype names from mime.types files.
pub fn _mime_types(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_mime_types");
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ctx = format!(":completion:{}:", curcontext);
    let default_type_files = vec!["~/.mime.types".to_string(), "/etc/mime.types".to_string()];

    // sh:10-14 — mime-types style with `+` → splice-in-defaults; else default.
    let styled = lookupstyle(&ctx, "mime-types");
    let type_files: Vec<String> = if styled.is_empty() {
        default_type_files.clone()
    } else {
        let mut v = Vec::new();
        for e in styled {
            if e == "+" {
                v.extend(default_type_files.clone());
            } else {
                v.push(e);
            }
        }
        v
    };

    let lines = read_type_lines(&type_files);
    let prefix = getsparam("PREFIX").unwrap_or_default();

    // sh:30 — [[ $PREFIX = (#b)([^/]##)/* ]] : a maintype/ already typed.
    if let Some(slash) = prefix.find('/') {
        if slash > 0 {
            // sh:33 — maintype and compset past it.
            let maintype = &prefix[..slash];
            let _ = bin_compset(
                "compset",
                &["-p".to_string(), (maintype.chars().count() + 1).to_string()],
                &make_ops(),
                0,
            );
            // sh:34 — subtypes of maintype (segment after `maintype/`, up to blank).
            let mut subs: Vec<String> = Vec::new();
            let needle = format!("{}/", maintype);
            for l in &lines {
                if let Some(rest) = l.strip_prefix(&needle) {
                    let sub = rest.split([' ', '\t']).next().unwrap_or("");
                    if !sub.is_empty() && !subs.iter().any(|s| s == sub) {
                        subs.push(sub.to_string());
                    }
                }
            }
            let mut w = vec![
                "mime-subtypes".to_string(),
                "expl".to_string(),
                "MIME subtype".to_string(),
                "compadd".to_string(),
                "--".to_string(),
            ];
            w.extend(subs);
            return _wanted(&w);
        }
    }

    // sh:38-40 — main types (segment before first `/`, filtered by $PREFIX,
    // or by a lowercase letter when $PREFIX is empty).
    let mut mains: Vec<String> = Vec::new();
    for l in &lines {
        let Some(slash) = l.find('/') else { continue };
        let main = &l[..slash];
        // sh:40 — the captured main segment must contain no `=` or `"`.
        if main.contains('=') || main.contains('"') {
            continue;
        }
        let keep = if prefix.is_empty() {
            main.chars().next().is_some_and(|c| c.is_ascii_lowercase())
        } else {
            main.starts_with(&prefix)
        };
        if keep && !mains.iter().any(|m| m == main) {
            mains.push(main.to_string());
        }
    }
    let mut w = vec![
        "mime-types".to_string(),
        "expl".to_string(),
        "MIME type".to_string(),
        "compadd".to_string(),
        "-S/".to_string(),
        "--".to_string(),
    ];
    w.extend(mains);
    _wanted(&w)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_without_registered_tags() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::params::setsparam("PREFIX", "");
        assert_eq!(_mime_types(&[]), 1);
    }
}
