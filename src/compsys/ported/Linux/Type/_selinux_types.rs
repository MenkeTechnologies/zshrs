//! Port of `_selinux_types` from `Completion/Linux/Type/_selinux_types`.
//!
//! Full upstream body (20 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  # Pass -a attribute to filter types, e.g.:
//! sh: 4  #   -a domain    - for process types
//! sh: 5  #   -a file_type - for file types
//! sh: 6  #   -a port_type - for network ports
//! sh: 8  local -a setypes expl extra
//! sh:10  zparseopts -E -D -a extra a:
//! sh:12  if (( $#extra )); then
//! sh:13    setypes=( ${${${(f)"$(_call_program selinux-types seinfo $extra --flat -x)"}#[[:blank:]]}:1} )
//! sh:14  else
//! sh:15    setypes=( ${(f)"$(_call_program selinux-types seinfo --flat -t)"} )
//! sh:16  fi
//! sh:18  _description selinux-types expl "selinux type"
//! sh:19  compadd "$@" "$expl[@]" -a setypes
//! ```

use crate::compsys::ported::_call_program::_call_program;
use crate::compsys::ported::_description::_description;
use crate::ported::params::{getaparam, getsparam, setaparam};
use crate::ported::zle::complete::bin_compadd;
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:10 — `zparseopts -E -D -a extra a:` — pull every `-a VALUE` pair out
/// of argv into `extra` (kept as flat `-a`,`VALUE` pairs, since they are
/// re-spread verbatim onto the `seinfo` command line at sh:13); everything
/// else stays in `rest` (the `"$@"` passed through to `compadd`).
fn parse_a_opt(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut extra = Vec::new();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-a" && i + 1 < args.len() {
            extra.push(args[i].clone());
            extra.push(args[i + 1].clone());
            i += 2;
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    (extra, rest)
}

/// sh:13 — `#[[:blank:]]`: strip a single leading blank (space/tab)
/// character from a line, if present (glob char class, not `##`).
fn strip_leading_blank(line: &str) -> &str {
    let mut chars = line.chars();
    match chars.next() {
        Some(c) if c == ' ' || c == '\t' => chars.as_str(),
        _ => line,
    }
}

/// `_selinux_types` — offer SELinux type names reported by `seinfo`,
/// optionally filtered by attribute via repeated `-a ATTR` options.
pub fn _selinux_types(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_selinux_types");
    // sh:10
    let (extra, rest) = parse_a_opt(args);

    // sh:12-16
    let setypes: Vec<String> = if !extra.is_empty() {
        // sh:13  _call_program selinux-types seinfo $extra --flat -x
        let mut cmd_args: Vec<String> = vec!["selinux-types".to_string(), "seinfo".to_string()];
        cmd_args.extend(extra.iter().cloned());
        cmd_args.push("--flat".to_string());
        cmd_args.push("-x".to_string());
        let _ = _call_program(&cmd_args);
        let out = getsparam("REPLY").unwrap_or_default();
        // ${${(f)"..."}#[[:blank:]]}:1} — split into lines, strip one
        // leading blank per line, then drop the first (header) line.
        out.lines()
            .skip(1)
            .map(|l| strip_leading_blank(l).to_string())
            .collect()
    } else {
        // sh:15  _call_program selinux-types seinfo --flat -t
        let _ = _call_program(&[
            "selinux-types".to_string(),
            "seinfo".to_string(),
            "--flat".to_string(),
            "-t".to_string(),
        ]);
        let out = getsparam("REPLY").unwrap_or_default();
        out.lines().map(|s| s.to_string()).collect()
    };

    // sh:18  _description selinux-types expl "selinux type"
    let _ = _description(&[
        "selinux-types".to_string(),
        "expl".to_string(),
        "selinux type".to_string(),
    ]);

    // sh:19  compadd "$@" "$expl[@]" -a setypes
    setaparam("setypes", setypes);
    let mut cadd: Vec<String> = rest;
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-a".to_string());
    cadd.push("setypes".to_string());
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_a_opt_collects_repeated_pairs_leaving_rest() {
        let (extra, rest) = parse_a_opt(&[
            "-a".into(),
            "domain".into(),
            "-J".into(),
            "grp".into(),
            "-a".into(),
            "file_type".into(),
        ]);
        assert_eq!(
            extra,
            vec![
                "-a".to_string(),
                "domain".to_string(),
                "-a".to_string(),
                "file_type".to_string(),
            ]
        );
        assert_eq!(rest, vec!["-J".to_string(), "grp".to_string()]);
    }

    #[test]
    fn parse_a_opt_leaves_dangling_flag_without_value_in_rest() {
        let (extra, rest) = parse_a_opt(&["-a".into()]);
        assert!(extra.is_empty());
        assert_eq!(rest, vec!["-a".to_string()]);
    }

    #[test]
    fn strip_leading_blank_strips_single_space_or_tab_only() {
        assert_eq!(strip_leading_blank("  domain"), " domain");
        assert_eq!(strip_leading_blank("\tfile_type"), "file_type");
        assert_eq!(strip_leading_blank("port_type"), "port_type");
        assert_eq!(strip_leading_blank(""), "");
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_selinux_types(&[]), 1);
    }
}
