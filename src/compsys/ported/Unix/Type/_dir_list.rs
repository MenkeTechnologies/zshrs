//! Port of `_dir_list` from `Completion/Unix/Type/_dir_list`.
//!
//! Full upstream body (29 lines verbatim):
//! ```text
//! sh: 1  #compdef -value-,TERMINFO_DIRS,-default- -P -value-,*PATH,-default-
//! sh: 3  # options:
//! sh: 4  #  -s <sep> to specify the separator (default is a colon)
//! sh: 5  #  -S       to say that the separator should be added as a suffix
//! sh:10  local sep=: dosuf suf
//! sh:12  while [[ "$1" = -(s*|S) ]]; do
//! sh:13    case "$1" in
//! sh:14    -s)  sep="$2"; shift 2;;
//! sh:15    -s*) sep="${1[3,-1]}"; shift;;
//! sh:16    -S)  dosuf=yes; shift;;
//! sh:17    esac
//! sh:18  done
//! sh:20  compset -P "*${sep}"
//! sh:21  compset -S "${sep}*" || suf="$sep"
//! sh:23  if [[ -n "$dosuf" ]]; then
//! sh:24    suf=(-S "$suf")
//! sh:25  else
//! sh:26    suf=()
//! sh:27  fi
//! sh:29  _directories "$suf[@]" -r "${sep}"' /\t\t\-' "$@"
//! ```
//!
//! Inline flag-parse (no zparseopts: the `-s<v>` short-form needs
//! arg-attached value support which zparseopts doesn't handle
//! cleanly). Calls real `bin_compset` then delegates to ported
//! `_directories`.

use crate::compsys::ported::_directories::_directories;
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

/// `_dir_list` — complete one entry of a separator-delimited path
/// list (e.g. `$PATH`). `-s <sep>` sets the separator (default `:`);
/// `-S` makes the separator a suffix.
pub fn _dir_list(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_dir_list");
    let mut sep = ":".to_string();
    let mut dosuf = false;
    let mut argv: Vec<String> = args.to_vec();

    // sh:12-18  inline flag parse
    while let Some(a) = argv.first() {
        if a == "-s" && argv.len() >= 2 {
            sep = argv[1].clone();
            argv.drain(..2);
        } else if let Some(rest) = a.strip_prefix("-s") {
            if !rest.is_empty() {
                sep = rest.to_string();
                argv.remove(0);
            } else {
                break;
            }
        } else if a == "-S" {
            dosuf = true;
            argv.remove(0);
        } else {
            break;
        }
    }

    // sh:20
    let _ = bin_compset(
        "compset",
        &["-P".to_string(), format!("*{}", sep)],
        &make_ops(),
        0,
    );
    // sh:21
    let mut suf_value = String::new();
    if bin_compset(
        "compset",
        &["-S".to_string(), format!("{}*", sep)],
        &make_ops(),
        0,
    ) != 0
    {
        suf_value = sep.clone();
    }

    // sh:23-27
    let suf: Vec<String> = if dosuf {
        vec!["-S".to_string(), suf_value]
    } else {
        Vec::new()
    };

    // sh:29
    let mut dir_args: Vec<String> = suf;
    dir_args.push("-r".to_string());
    dir_args.push(format!("{} /\t\t-", sep));
    dir_args.extend(argv);
    _directories(&dir_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_separator_colon() {
        // No flag → sep defaults to `:`; _directories returns 1
        //   without registered tags.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_dir_list(&[]), 1);
    }

    #[test]
    fn s_flag_attached_value() {
        // sh:14 — `-s,` means separator is `,`.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_dir_list(&["-s,".to_string()]), 1);
    }
}
