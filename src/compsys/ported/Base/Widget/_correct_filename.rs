//! Port of `_correct_filename` from
//! `Completion/Base/Widget/_correct_filename`.
//!
//! Full upstream body (72 lines, abridged):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xC
//! sh:19  local file="$PREFIX$SUFFIX" trylist tilde etilde testcmd
//! sh:20  integer approx max_approx=6
//! sh:20  if [[ -z $WIDGET ]]; then file=$1; local IPREFIX
//! sh:23  else (( ${NUMERIC:-1} > 1 )) && max_approx=$NUMERIC
//! sh:25  if [[ $file = \~*/* ]]; then tilde-expand
//! sh:31  if [[ $CURRENT -eq 1 && $file != /* ]]; then testcmd=1
//! sh:33  elif [[ $file = \=* ]]; then …testcmd=1
//! sh:40  if -e file (or whence file) → emit + return
//! sh:50  for approx 1..max_approx do
//! sh:57    trylist via `(#a$approx)` glob (or whence -wm)
//! sh:64  done
//! sh:72  return 1
//! ```
//!
//! The `(#a$approx)` glob qualifier (approximate match within
//! `$approx` errors) is part of zsh's extended-glob; mimicking it
//! exactly requires a Levenshtein-aware globber. The port covers
//! the exact-match emit + bails on approximate fall-through.

use crate::ported::params::{getiparam, getsparam, setsparam};
use crate::ported::zle::compcore::set_compstate_str;
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};
use std::path::Path;

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// `_correct_filename` — try to correct the misspelled filename
/// under the cursor (or print correction to stdout when called as
/// a non-widget). Approximate-match fall-through left as a TODO.
pub fn _correct_filename(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_correct_filename");
    let widget = getsparam("WIDGET").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();

    // sh:19/sh:20
    let (mut file, in_widget): (String, bool) = if widget.is_empty() {
        (args.first().cloned().unwrap_or_default(), false)
    } else {
        let numeric = getiparam("NUMERIC");
        let _max_approx = if numeric > 1 { numeric } else { 6 };
        (format!("{}{}", prefix, suffix), true)
    };

    // sh:25-29  tilde expand (~/path)
    if file.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            file = format!("{}{}", home, &file[1..]);
        }
    }

    // sh:31-39  testcmd detection
    let current = getiparam("CURRENT");
    let mut testcmd = false;
    let iprefix = getsparam("IPREFIX").unwrap_or_default();
    if current == 1 && !file.starts_with('/') {
        testcmd = true;
    } else if file.starts_with('=') {
        if in_widget {
            let _ = setsparam("PREFIX", &prefix[1..]);
        }
        let _ = setsparam("IPREFIX", &format!("{}={}", iprefix, ""));
        file = file[1..].to_string();
        testcmd = true;
    }

    // sh:40-49  exact match short-circuit
    let exists = if testcmd {
        which(&file).is_some()
    } else {
        Path::new(&file).exists()
    };
    if exists {
        if in_widget {
            let argv: Vec<String> = vec![
                "-QUf".to_string(),
                "-i".to_string(),
                iprefix,
                "-I".to_string(),
                getsparam("ISUFFIX").unwrap_or_default(),
                file.clone(),
            ];
            let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
            let cur_insert =
                crate::ported::zle::compcore::get_compstate_str("insert").unwrap_or_default();
            if !cur_insert.is_empty() {
                set_compstate_str("insert", "menu");
            }
        } else {
            println!("{}", file);
        }
        return 0;
    }

    // sh:56-64  approximate-match loop. `(#a$N)` is zsh's
    //   Levenshtein-≤N glob qualifier; we mirror by computing
    //   edit-distance against every candidate in the dir (file
    //   mode) or every executable basename (cmd mode), accepting
    //   the closest one within `max_approx` errors.
    let max_approx: usize = if in_widget {
        let numeric = getiparam("NUMERIC");
        if numeric > 1 {
            numeric as usize
        } else {
            6
        }
    } else {
        6
    };
    let candidates = if testcmd {
        path_basenames()
    } else {
        directory_basenames(&file)
    };
    let target = std::path::Path::new(&file)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.clone());
    let mut best: Option<(usize, String)> = None;
    for c in &candidates {
        let d = crate::compsys::ported::shared::edit_distance(&target, c);
        if d == 0 || d > max_approx {
            continue;
        }
        if best.as_ref().map(|(bd, _)| d < *bd).unwrap_or(true) {
            best = Some((d, c.clone()));
        }
    }
    let corrected = match best {
        Some((_, name)) => name,
        None => return 1,
    };
    let corrected_full = if testcmd {
        which(&corrected).unwrap_or(corrected)
    } else if let Some(slash) = file.rfind('/') {
        format!("{}/{}", &file[..slash], corrected)
    } else {
        corrected
    };
    if in_widget {
        let argv: Vec<String> = vec![
            "-QUf".to_string(),
            "-i".to_string(),
            iprefix.clone(),
            "-I".to_string(),
            getsparam("ISUFFIX").unwrap_or_default(),
            corrected_full,
        ];
        let _ = bin_compadd("compadd", &argv, &make_ops(), 0);
        let cur_insert =
            crate::ported::zle::compcore::get_compstate_str("insert").unwrap_or_default();
        if !cur_insert.is_empty() {
            set_compstate_str("insert", "menu");
        }
    } else {
        println!("{}", corrected_full);
    }
    0
}

/// Enumerate every basename in `file`'s parent directory (file
/// approximate-match mode).
fn directory_basenames(file: &str) -> Vec<String> {
    let dir = match file.rfind('/') {
        Some(i) => file[..=i].to_string(),
        None => ".".to_string(),
    };
    let scan = if dir.is_empty() { ".".to_string() } else { dir };
    std::fs::read_dir(Path::new(&scan))
        .map(|e| {
            e.flatten()
                .map(|ent| ent.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Enumerate every executable basename across `$PATH` (cmd mode).
fn path_basenames() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Ok(path) = std::env::var("PATH") {
        for dir in path.split(':') {
            if dir.is_empty() {
                continue;
            }
            if let Ok(entries) = std::fs::read_dir(dir) {
                for ent in entries.flatten() {
                    if let Ok(meta) = ent.metadata() {
                        if meta.is_file() {
                            out.push(ent.file_name().to_string_lossy().into_owned());
                        }
                    }
                }
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

/// `whence` substitute — search $PATH for `file`.
fn which(file: &str) -> Option<String> {
    if file.contains('/') {
        if Path::new(file).is_file() {
            return Some(file.to_string());
        }
        return None;
    }
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = format!("{}/{}", dir, file);
        if Path::new(&candidate).is_file() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_existing_path_returns_one() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("WIDGET", "");
        let r = _correct_filename(&["/definitely/not/here/xyz".to_string()]);
        assert_eq!(r, 1);
    }

    #[test]
    fn existing_path_prints_and_returns_zero() {
        let _g = crate::test_util::global_state_lock();
        let _ = setsparam("WIDGET", "");
        let r = _correct_filename(&["/".to_string()]);
        assert_eq!(r, 0);
    }
}
