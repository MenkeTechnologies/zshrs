//! Port of `_path_files` from
//! `Completion/Unix/Type/_path_files`.
//!
//! Full upstream body (895 lines, abridged):
//! ```text
//! sh:  1  #autoload
//! sh:  4  file-split-chars style → compset -P
//! sh: 22  _have_glob_qual → glob-qualifier dispatch (skipped in port)
//! sh: 60  flag-parse: -/ -f -g <pat> -W <dir> -P -S -F …
//! sh:200  split PREFIX on / into linepath + filename
//! sh:300  scan dir with glob filter
//! sh:500  per-style filtering (ignored-suffixes / list-suffixes / etc.)
//! sh:700  compadd with -W <dir> -P / -S
//! sh:895  return
//! ```
//!
//! Path completion entry point. This port covers the common case:
//!   * `-/` (dirs only), `-f` (files only — default), `-g <pat>` glob
//!   * `-W <dir>` walk-from
//!   * `-P <pre>` / `-S <suf>` literal prefix/suffix
//!   * `-X <desc>` group description
//! Caching, partial-path expansion, ignored-patterns, recursive
//! search, and special-dirs handling are TODOs (sh:200-700 of
//! the original).

use crate::ported::exec::dispatch_function_call;
use crate::ported::modules::zutil::lookupstyle;
use crate::ported::params::{getsparam, setaparam, setsparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zle::complete::{bin_compadd, bin_compset};
use crate::ported::zsh_h::{options, MAX_OPS};
use std::fs;
use std::path::Path;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

#[derive(Default)]
struct PathArgs {
    dirs_only: bool,
    files_only: bool,
    glob: Option<String>,
    walk_from: Option<String>,
    prefix_lit: Option<String>,
    suffix_lit: Option<String>,
    descr: Option<String>,
    pass_through: Vec<String>,
}

fn parse_args(args: &[String]) -> PathArgs {
    let mut p = PathArgs::default();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match a.as_str() {
            "-/" => {
                p.dirs_only = true;
                i += 1;
            }
            "-f" => {
                p.files_only = true;
                i += 1;
            }
            "-g" if i + 1 < args.len() => {
                p.glob = Some(args[i + 1].clone());
                i += 2;
            }
            s if s.starts_with("-g") && s.len() > 2 => {
                p.glob = Some(s[2..].to_string());
                i += 1;
            }
            "-W" if i + 1 < args.len() => {
                p.walk_from = Some(args[i + 1].clone());
                i += 2;
            }
            "-P" if i + 1 < args.len() => {
                p.prefix_lit = Some(args[i + 1].clone());
                i += 2;
            }
            "-S" if i + 1 < args.len() => {
                p.suffix_lit = Some(args[i + 1].clone());
                i += 2;
            }
            "-X" if i + 1 < args.len() => {
                p.descr = Some(args[i + 1].clone());
                i += 2;
            }
            _ => {
                p.pass_through.push(a.clone());
                i += 1;
            }
        }
    }
    p
}

/// sh:22-41 — `_have_glob_qual` dispatch. When the cursor sits
/// inside an unclosed `(...)` glob qualifier, hand off to
/// `_globquals` / `_globflags`.
fn handle_glob_qualifier() -> Option<i32> {
    let prefix = getsparam("PREFIX").unwrap_or_default();
    // Match a trailing `(` with an even (incl. 0) count of escapes
    //   in front. Approximation: detect bare `(` anywhere in PREFIX
    //   not preceded by `\`.
    let mut depth: i32 = 0;
    let mut last_open: Option<usize> = None;
    let bytes = prefix.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\\' {
            continue;
        }
        if b == b'(' && (i == 0 || bytes[i - 1] != b'\\') {
            depth += 1;
            last_open = Some(i);
        } else if b == b')' && (i == 0 || bytes[i - 1] != b'\\') {
            depth -= 1;
        }
    }
    if depth <= 0 {
        return None;
    }
    let open_at = last_open?;
    // Trim PREFIX up to the `(`
    let _ = bin_compset(
        "compset",
        &["-p".to_string(), open_at.to_string()],
        &make_ops(),
        0,
    );
    let _ = bin_compset(
        "compset",
        &["-S".to_string(), "[^\\)\\|\\~]#(|\\))".to_string()],
        &make_ops(),
        0,
    );
    // Check for `#` introducing glob flags
    if bin_compset(
        "compset",
        &["-P".to_string(), "\\#".to_string()],
        &make_ops(),
        0,
    ) == 0
    {
        return Some(dispatch_function_call("_globflags", &[]).unwrap_or(1));
    }
    Some(dispatch_function_call("_globquals", &[]).unwrap_or(1))
}

/// sh:200 partial-path expansion — `/u/l/b<TAB>` → `/usr/local/bin/`.
/// For each `dir/` segment of the prefix, if exactly one directory
/// matches the segment prefix, accept it and walk down.
fn expand_partial_path(prefix: &str) -> String {
    if !prefix.contains('/') {
        return prefix.to_string();
    }
    let parts: Vec<&str> = prefix.split('/').collect();
    let mut walked = String::new();
    if parts[0].is_empty() {
        walked.push('/');
    }
    for (i, seg) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            // Last segment — don't expand (that's what the completion
            //   itself will offer)
            walked.push_str(seg);
            break;
        }
        if seg.is_empty() {
            continue;
        }
        // Find unique dir under `walked` whose name starts with `seg`.
        let search_dir = if walked.is_empty() { "." } else { &walked };
        let matches: Vec<String> = std::fs::read_dir(std::path::Path::new(search_dir))
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| {
                        let n = e.file_name().to_string_lossy().into_owned();
                        if n.starts_with(seg) {
                            if let Ok(meta) = e.metadata() {
                                if meta.is_dir() {
                                    return Some(n);
                                }
                            }
                        }
                        None
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Only expand on exact-1 match
        let chosen = if matches.len() == 1 {
            matches[0].clone()
        } else {
            seg.to_string()
        };
        walked.push_str(&chosen);
        walked.push('/');
    }
    walked
}

/// `_path_files` — file/directory completion.
pub fn _path_files(args: &[String]) -> i32 {
    // sh:22-41 — glob-qualifier dispatch before any path resolution.
    if let Some(rc) = handle_glob_qualifier() {
        return rc;
    }

    let p = parse_args(args);

    let prefix_raw = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
    // sh:200 — partial-path expansion (`/u/l/b` → `/usr/local/bin`).
    let prefix = expand_partial_path(&prefix_raw);
    if prefix != prefix_raw {
        let _ = setsparam("PREFIX", &prefix);
    }
    let combined = format!("{}{}", prefix, suffix);

    // Split into directory part + filename part
    let (dir_part, name_part) = match combined.rfind('/') {
        Some(i) => (combined[..=i].to_string(), combined[i + 1..].to_string()),
        None => (String::new(), combined.clone()),
    };

    // Determine scan dir: -W root + dir_part, else just dir_part
    let scan_root = match p.walk_from.as_deref() {
        Some(w) => {
            if dir_part.starts_with('/') {
                // Absolute prefix overrides -W
                dir_part.clone()
            } else if w.ends_with('/') {
                format!("{}{}", w, dir_part)
            } else {
                format!("{}/{}", w, dir_part)
            }
        }
        None => {
            if dir_part.is_empty() {
                ".".to_string()
            } else {
                dir_part.clone()
            }
        }
    };

    // Compile glob (default `*` — matches anything when no -g)
    let glob_pat = p.glob.clone().unwrap_or_else(|| "*".to_string());
    let glob_prog = patcompile(&{ let mut __pat_tok = (&glob_pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, 0, None);

    // sh:500 — ignored-patterns + ignored-suffixes filtering.
    let curcontext = getsparam("curcontext").unwrap_or_default();
    let ignored_patterns: Vec<Box<dyn Fn(&str) -> bool>> =
        lookupstyle(&format!(":completion:{}:", curcontext), "ignored-patterns")
            .into_iter()
            .filter_map(|pat| {
                patcompile(&{ let mut __pat_tok = (&pat).to_string(); crate::ported::glob::tokenize(&mut __pat_tok); __pat_tok }, 0, None).map(|prog| {
                    Box::new(move |name: &str| pattry(&prog, name)) as Box<dyn Fn(&str) -> bool>
                })
            })
            .collect();
    let ignored_suffixes: Vec<String> =
        lookupstyle(&format!(":completion:{}:", curcontext), "ignored-suffixes");
    let special_dirs_style = lookupstyle(&format!(":completion:{}:", curcontext), "special-dirs")
        .first()
        .cloned()
        .unwrap_or_default();
    let special_dirs_on = matches!(
        special_dirs_style.as_str(),
        "yes" | "true" | "1" | "on" | ".." | "true .."
    );

    let entries = match fs::read_dir(Path::new(&scan_root)) {
        Ok(e) => e,
        Err(_) => return 1,
    };

    let mut matches: Vec<String> = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        // Skip dotfiles unless the prefix explicitly asks for them.
        //   `.` and `..` are only emitted when `special-dirs` is set.
        if name.starts_with('.') && !name_part.starts_with('.') {
            if special_dirs_on && (name == "." || name == "..") {
                // fall through
            } else {
                continue;
            }
        }
        // Filename prefix-match
        if !name.starts_with(&name_part) {
            continue;
        }
        // sh:500 — ignored-patterns + ignored-suffixes
        if ignored_patterns.iter().any(|f| f(&name)) {
            continue;
        }
        if ignored_suffixes
            .iter()
            .any(|suf| name.ends_with(suf as &str))
        {
            continue;
        }
        // dirs_only / files_only filter
        let meta = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if p.dirs_only && !meta.is_dir() {
            continue;
        }
        if p.files_only && !meta.is_file() {
            continue;
        }
        // Glob pattern filter (only applied to file basename)
        if let Some(prog) = glob_prog.as_ref() {
            if !pattry(prog, &name) {
                continue;
            }
        }
        matches.push(name);
    }
    if matches.is_empty() {
        return 1;
    }
    matches.sort();

    // Suffix slash for directories to enable continued navigation
    let with_slash: Vec<String> = matches
        .iter()
        .map(|m| {
            let full = format!("{}/{}", scan_root, m);
            match fs::metadata(&full) {
                Ok(meta) if meta.is_dir() => format!("{}/", m),
                _ => m.clone(),
            }
        })
        .collect();
    setaparam("_path_files_arr", with_slash);

    // Build compadd argv
    let mut compadd_argv: Vec<String> = p.pass_through.clone();
    if let Some(d) = p.descr {
        compadd_argv.push("-X".to_string());
        compadd_argv.push(d);
    }
    // Use -W only when the caller supplied one (lets compadd
    //   surface absolute paths correctly when we resolved dir_part).
    if let Some(_w) = p.walk_from.as_deref() {
        compadd_argv.push("-W".to_string());
        compadd_argv.push(scan_root.clone());
    } else if !dir_part.is_empty() {
        compadd_argv.push("-W".to_string());
        compadd_argv.push(dir_part.clone());
    }
    if let Some(pre) = p.prefix_lit {
        compadd_argv.push("-P".to_string());
        compadd_argv.push(pre);
    }
    if let Some(suf) = p.suffix_lit {
        compadd_argv.push("-S".to_string());
        compadd_argv.push(suf);
    }
    compadd_argv.push("-a".to_string());
    compadd_argv.push("_path_files_arr".to_string());
    bin_compadd("compadd", &compadd_argv, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ported::params::setsparam;

    #[test]
    fn parses_dirs_only_flag() {
        let p = parse_args(&["-/".to_string()]);
        assert!(p.dirs_only);
        assert!(!p.files_only);
    }

    #[test]
    fn parses_glob_flag() {
        let p = parse_args(&["-g".to_string(), "*.rs".to_string()]);
        assert_eq!(p.glob.as_deref(), Some("*.rs"));
    }

    #[test]
    fn parses_attached_glob_form() {
        let p = parse_args(&["-g*.txt".to_string()]);
        assert_eq!(p.glob.as_deref(), Some("*.txt"));
    }

    #[test]
    fn empty_dir_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("PREFIX", "/nonexistent/path/here_");
        let _ = setsparam("SUFFIX", "");
        assert_eq!(_path_files(&[]), 1);
    }
}
