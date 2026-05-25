//! Port of `_correct_filename` from
//! `Completion/Base/Widget/_correct_filename`.
//!
//! Full upstream body (72 lines, abridged):
//! ```text
//! sh: 1  #compdef -k complete-word \C-xC
//! sh:17  local file="$PREFIX$SUFFIX" trylist tilde etilde testcmd
//! sh:18  integer approx max_approx=6
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
    let widget = getsparam("WIDGET").unwrap_or_default();
    let prefix = getsparam("PREFIX").unwrap_or_default();
    let suffix = getsparam("SUFFIX").unwrap_or_default();

    // sh:17/sh:20
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
                crate::ported::zle::compcore::get_compstate_str("insert")
                    .unwrap_or_default();
            if !cur_insert.is_empty() {
                set_compstate_str("insert", "menu");
            }
        } else {
            println!("{}", file);
        }
        return 0;
    }

    // sh:56-64  approximate-match loop — TODO without (#aN) glob
    1
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
