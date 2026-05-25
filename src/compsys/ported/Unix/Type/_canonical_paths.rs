//! Port of `_canonical_paths` from
//! `Completion/Unix/Type/_canonical_paths`.
//!
//! Full upstream body (123 lines, abridged):
//! ```text
//! sh: 1  #autoload
//! sh:30  while getopts "AMNa:f:p:n" opt; do …
//! sh:40-100 build pre-existing canonical path list + delegate to
//!         _path_files with `-W root` flag
//! sh:122 return ret
//! ```
//!
//! Common entry: `_canonical_paths -A var TAG ...` resolves each
//! caller-supplied path via `realpath()` (or equivalent) before
//! dispatching to `_path_files`.

use crate::compsys::ported::_wanted::_wanted;
use crate::ported::params::{getaparam, setaparam};
use std::path::Path;

/// `_canonical_paths` — complete file paths with symlinks resolved.
pub fn _canonical_paths(args: &[String]) -> i32 {
    let mut tag = "paths".to_string();
    let mut name = String::new();
    let mut paths: Vec<String> = Vec::new();
    let mut idx = 0usize;

    // sh:30  flag parse
    while idx < args.len() {
        let a = &args[idx];
        match a.as_str() {
            "-A" if idx + 1 < args.len() => {
                name = args[idx + 1].clone();
                idx += 2;
            }
            "-N" | "-M" | "-n" => idx += 1,
            "-a" | "-f" | "-p" if idx + 1 < args.len() => idx += 2,
            _ => break,
        }
    }
    if idx >= args.len() {
        return 1;
    }
    tag = args[idx].clone();
    idx += 1;

    // Remaining args are paths to canonicalize
    for p in &args[idx..] {
        if let Ok(real) = std::fs::canonicalize(Path::new(p)) {
            paths.push(real.to_string_lossy().into_owned());
        }
    }

    // If -A given, publish under that name
    if !name.is_empty() {
        setaparam(&name, paths.clone());
        let prev = getaparam(&name).unwrap_or_default();
        let _ = prev;
    }

    if paths.is_empty() {
        return 1;
    }

    setaparam("_canonical_paths_arr", paths);
    _wanted(&[
        tag,
        "expl".to_string(),
        "path".to_string(),
        "compadd".to_string(),
        "-a".to_string(),
        "_canonical_paths_arr".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_one_for_no_paths() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_canonical_paths(&["mytag".to_string()]), 1);
    }
}
