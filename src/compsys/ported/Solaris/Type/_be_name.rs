//! Port of `_be_name` from `Completion/Solaris/Type/_be_name`.
//!
//! Full upstream body (14 lines verbatim):
//! ```text
//! sh: 1  #autoload
//! sh: 3  local -a type be_names expl
//! sh: 5  zparseopts -D -E -a type t+:
//! sh: 7  be_names=( ${${(f)"$(_call_program boot-environs beadm list -H)"}%%;*} )
//! sh: 9  [[ -n $type[(r)all] ]] &&
//! sh:10    be_names+=( ${${${(f)"$(_call_program boot-environs beadm list -sH)"}#*;}%%;*} )
//! sh:12  _description boot-environs expl 'boot environment'
//! sh:13  compadd "$@" "$expl[@]" -a be_names
//! ```

use crate::compsys::ported::_call_program::call_program_capture;
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

/// sh:5 — `zparseopts -D -E -a type t+:`. Pulls every `-t VALUE` pair out
/// of `args`, appending `"-t"` then `VALUE` to the `type` array (mirroring
/// zsh's `-a` array-collection form) and leaving everything else in
/// `rest` (the `-D`-stripped `"$@"` later passed through to compadd).
fn zparse_type(args: &[String]) -> (Vec<String>, Vec<String>) {
    let mut ty = Vec::new();
    let mut rest = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "-t" {
            ty.push("-t".to_string());
            if i + 1 < args.len() {
                ty.push(args[i + 1].clone());
                i += 2;
            } else {
                i += 1;
            }
        } else {
            rest.push(args[i].clone());
            i += 1;
        }
    }
    (ty, rest)
}

/// sh:7 — `${line%%;*}`: keep only the text before the first `;`
/// (unchanged if there is no `;`).
fn first_field(line: &str) -> &str {
    match line.find(';') {
        Some(pos) => &line[..pos],
        None => line,
    }
}

/// sh:10 — `${${line#*;}%%;*}`: drop the text up to and including the
/// first `;`, then keep only the text before the next `;` — i.e. the
/// second `;`-delimited field (unchanged fallbacks when a `;` is
/// missing, matching zsh's no-match-leaves-unchanged `#`/`%%` behavior).
fn second_field(line: &str) -> &str {
    let rest = match line.find(';') {
        Some(pos) => &line[pos + 1..],
        None => line,
    };
    match rest.find(';') {
        Some(pos) => &rest[..pos],
        None => rest,
    }
}

/// `_be_name` — complete Solaris/illumos boot-environment names from
/// `beadm list`, optionally including snapshot names too via `-t all`.
pub fn _be_name(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_be_name");
    // sh:5
    let (ty, rest) = zparse_type(args);

    // sh:7  be_names=( ${${(f)"$(_call_program boot-environs beadm list -H)"}%%;*} )
    let _ = call_program_capture(&[
        "boot-environs".to_string(),
        "beadm".to_string(),
        "list".to_string(),
        "-H".to_string(),
    ]);
    let out = getsparam("REPLY").unwrap_or_default();
    let mut be_names: Vec<String> = out.lines().map(first_field).map(String::from).collect();

    // sh:9-10  [[ -n $type[(r)all] ]] && be_names+=( ... )
    if ty.iter().any(|s| s == "all") {
        let _ = call_program_capture(&[
            "boot-environs".to_string(),
            "beadm".to_string(),
            "list".to_string(),
            "-sH".to_string(),
        ]);
        let snap_out = getsparam("REPLY").unwrap_or_default();
        be_names.extend(snap_out.lines().map(second_field).map(String::from));
    }

    // sh:12  _description boot-environs expl 'boot environment'
    let _ = _description(&[
        "boot-environs".to_string(),
        "expl".to_string(),
        "boot environment".to_string(),
    ]);

    // sh:13  compadd "$@" "$expl[@]" -a be_names
    setaparam("be_names", be_names);
    let mut cadd: Vec<String> = rest;
    cadd.extend(getaparam("expl").unwrap_or_default());
    cadd.push("-a".to_string());
    cadd.push("be_names".to_string());
    bin_compadd("compadd", &cadd, &make_ops(), 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zparse_type_pulls_repeated_t_leaving_rest() {
        let (ty, rest) = zparse_type(&[
            "-t".into(),
            "all".into(),
            "-J".into(),
            "grp".into(),
            "-t".into(),
            "other".into(),
        ]);
        assert_eq!(
            ty,
            vec![
                "-t".to_string(),
                "all".to_string(),
                "-t".to_string(),
                "other".to_string()
            ]
        );
        assert_eq!(rest, vec!["-J".to_string(), "grp".to_string()]);
    }

    #[test]
    fn first_field_keeps_prefix_before_semicolon() {
        assert_eq!(first_field("myBE;NR;/;;native;123M;static;-"), "myBE");
        assert_eq!(first_field("no-semicolon-here"), "no-semicolon-here");
    }

    #[test]
    fn second_field_extracts_snapshot_column() {
        assert_eq!(second_field("myBE;myBE@snap1;;;;"), "myBE@snap1");
        assert_eq!(second_field("only-one-field"), "only-one-field");
    }

    #[test]
    fn returns_one_without_completion_context() {
        let _g = crate::test_util::global_state_lock();
        crate::ported::zle::complete::INCOMPFUNC.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(_be_name(&[]), 1);
    }
}
