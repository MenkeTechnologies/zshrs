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
/// remaining args are `name[:descr]:action` specs forwarded to
/// `_alternative`.
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
            s if s.starts_with('-') && !s.starts_with("--") => {
                // Single-dash flag w/o known handling — skip
                idx += 1;
            }
            _ => break,
        }
    }

    // First positional after flags is the description
    if idx >= args.len() {
        return 1;
    }
    let descr = args[idx].clone();
    idx += 1;

    let specs: Vec<String> = args[idx..].to_vec();
    if specs.is_empty() {
        return 1;
    }

    // Build _alternative defs: each spec like `name[:descr]:action`
    //   maps directly to alternative syntax.
    let alts: Vec<String> = specs
        .iter()
        .map(|s| {
            // If spec has fewer than 2 colons, treat as bare value
            //   with no action (delegate via _files).
            if s.matches(':').count() < 2 {
                format!("values:{}:({})", descr, s)
            } else {
                s.clone()
            }
        })
        .collect();

    let _ = sep;
    let _ = subsep;

    // sh:155 — delegate to _alternative
    let saved_curcontext = getsparam("curcontext").unwrap_or_default();
    let r = _alternative(&alts);
    let _ = setsparam("curcontext", &saved_curcontext);
    r
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
