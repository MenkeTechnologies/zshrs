//! Port of `_values` from `Completion/Base/Utility/_values`.
//!
//! Full upstream body (160 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:  3-15  flag parse: -O -s -S -wC … specs to follow
//! sh: 20  for spec; do parse `name[:descr]:action` … into descs+actions
//! sh: 80  loop _next_label per requested tag emitting matches
//! sh:155  fall through to _alternative with defs
//! ```
//!
//! Heavy port; the full spec language (action types `(((`/`(`/`{`/
//! `_files`/etc.) is intentionally simplified to the action-dispatch
//! path used by `_alternative`. Most callers pass either a literal
//! `(val:desc val:desc …)` set or a `_files`-style dispatch.

use crate::compsys::ported::_alternative::_alternative;
use crate::ported::params::{getsparam, setsparam};

/// `_values` — generic value-completion entry. `-s SEP` enables
/// comma/sep-separated list, `-S =` enables `name=val` syntax,
/// remaining args are `name[desc]=:msg:action` (full form) or
/// `name[:descr]:action` (simple form) specs.
pub fn _values(args: &[String]) -> i32 {
    let mut sep: Option<String> = None;
    let mut subsep: Option<String> = None;
    let mut idx = 0usize;

    // sh:3-15  -s / -S / -O flag parse
    while idx < args.len() {
        let a = &args[idx];
        match a.as_str() {
            "-s" if idx + 1 < args.len() => {
                sep = Some(args[idx + 1].clone());
                idx += 2;
            }
            "-S" if idx + 1 < args.len() => {
                subsep = Some(args[idx + 1].clone());
                idx += 2;
            }
            "-O" if idx + 1 < args.len() => idx += 2,
            "-C" if idx + 1 < args.len() => idx += 2,
            "-w" => idx += 1,
            s if s.starts_with('-') && !s.starts_with("--") => idx += 1,
            _ => break,
        }
    }

    if idx >= args.len() {
        return 1;
    }
    let descr = args[idx].clone();
    idx += 1;

    let specs: Vec<String> = args[idx..].to_vec();
    if specs.is_empty() {
        return 1;
    }

    // Determine which value the user is currently typing (after the
    //   last separator) and which preceding values to dedupe out.
    let (active_val, prior_vals, completed_prefix) = if let Some(s) = sep.as_deref() {
        let prefix = crate::ported::params::getsparam("PREFIX").unwrap_or_default();
        let parts: Vec<&str> = prefix.split(s).collect();
        let active = parts.last().copied().unwrap_or("").to_string();
        let prior: Vec<String> = parts[..parts.len().saturating_sub(1)]
            .iter()
            .map(|p| p.to_string())
            .collect();
        let completed = if prior.is_empty() {
            String::new()
        } else {
            format!("{}{}", prior.join(s), s)
        };
        (active, prior, completed)
    } else {
        (
            crate::ported::params::getsparam("PREFIX").unwrap_or_default(),
            Vec::new(),
            String::new(),
        )
    };

    // Build the alternative-spec list. Each `_values` spec maps to
    //   one alternative entry, with name-only specs becoming a
    //   literal-list of self-described values.
    let mut alts: Vec<String> = Vec::new();
    let mut name_only: Vec<String> = Vec::new();
    for s in &specs {
        // Strip optional `name[desc]` brace prefix (sh:30-40)
        let (head, action_part) = if let Some(open) = s.find('[') {
            if let Some(close) = s[open..].find(']') {
                let after = &s[open + close + 1..];
                (s[..open].to_string(), after)
            } else {
                (s.clone(), "")
            }
        } else {
            (s.clone(), "")
        };
        // `name=:msg:action` form (subsep present)
        let bare_name = head.strip_suffix('=').unwrap_or(&head).to_string();
        if prior_vals.iter().any(|p| {
            // dedupe: strip `subsep VALUE` if -S given
            let key = if let Some(ss) = subsep.as_deref() {
                p.split(ss).next().unwrap_or(p)
            } else {
                p.as_str()
            };
            key == bare_name
        }) {
            continue;
        }
        if action_part.is_empty() {
            // name-only — accumulate into a literal list
            name_only.push(bare_name);
        } else {
            // full form: `:msg:action`
            let body = action_part.trim_start_matches(':');
            alts.push(format!("values:{}:{}", descr, body));
        }
    }
    if !name_only.is_empty() {
        alts.push(format!("values:{}:({})", descr, name_only.join(" ")));
    }
    if alts.is_empty() {
        return 1;
    }

    // Use compset to trim the leading prior-values prefix so the
    //   downstream completer matches against `active_val` only.
    if !completed_prefix.is_empty() {
        let _ = crate::ported::zle::complete::bin_compset(
            "compset",
            &["-P".to_string(), glob_escape(&completed_prefix)],
            &crate::ported::zsh_h::options {
                ind: [0u8; crate::ported::zsh_h::MAX_OPS],
                args: Vec::new(),
                argscount: 0,
                argsalloc: 0,
            },
            0,
        );
    }
    let _ = active_val;

    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let r = _alternative(&alts);
    let _ = setsparam("curcontext", &saved_curcontext);
    r
}

/// Backslash-escape regex/pattern metacharacters so `bin_compset
/// -P <prefix>` treats the string literally.
fn glob_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for c in s.chars() {
        if matches!(
            c,
            '(' | ')' | '|' | '*' | '?' | '[' | ']' | '#' | '~' | '^' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_returns_one() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_values(&[]), 1);
    }

    #[test]
    fn passes_through_to_alternative() {
        let _g = crate::test_util::global_state_lock();
        let r = _values(&[
            "options".to_string(),
            "values:option name:(alpha beta)".to_string(),
        ]);
        // _alternative returns 1 without tag setup
        assert_eq!(r, 1);
    }
}
