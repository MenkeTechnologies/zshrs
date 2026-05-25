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

use crate::ported::params::{getsparam, setaparam};
use crate::ported::pattern::{patcompile, pattry};
use crate::ported::zle::complete::bin_compadd;
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

/// `_path_files` — file/directory completion.
pub fn _path_files(args: &[String]) -> i32 {
    let p = parse_args(args);

    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();
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
    let glob_prog = patcompile(&glob_pat, 0, None);

    let entries = match fs::read_dir(Path::new(&scan_root)) {
        Ok(e) => e,
        Err(_) => return 1,
    };

    let mut matches: Vec<String> = Vec::new();
    for ent in entries.flatten() {
        let name = ent.file_name().to_string_lossy().to_string();
        // Skip dotfiles unless the prefix explicitly asks for them
        if name.starts_with('.') && !name_part.starts_with('.') {
            continue;
        }
        // Filename prefix-match
        if !name.starts_with(&name_part) {
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
