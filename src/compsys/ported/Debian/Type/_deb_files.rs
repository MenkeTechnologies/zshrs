//! Port of `_deb_files` from `Completion/Debian/Type/_deb_files`.
//!
//! Full upstream body (19 lines verbatim, sh:1 usage comment omitted):
//! ```text
//! sh: 1  #autoload
//! sh: 2
//! sh: 3  # This function is used to generate file names for Debian package files (.deb
//! sh: 4  # and friends). In addition to the options supported by _files, it understands
//! sh: 5  # the following:
//! sh: 6  #
//! sh: 7  #   -c  Include .changes and .dsc files
//! sh: 8  #   -D  Only .dsc files (e.g. apt-get build-dep ./foo.dsc)
//! sh: 9
//! sh:10  local -a _expl _fopts _c _D _exts=( deb ddeb udeb )
//! sh:11
//! sh:12  zparseopts -a _fopts -D -E - \
//! sh:13    c=_c D=_D 1 2 F+: J+: M+: n P+: q r+: R+: S+: V+: W+: X+:
//! sh:14
//! sh:15  (( $#_c )) && _exts+=( changes dsc )
//! sh:16  (( $#_D )) && _exts+=( dsc )
//! sh:17
//! sh:18  _description files _expl 'Debian package'
//! sh:19  _files "${(@)_fopts}" "${(@)_expl}" -g "*.(${(j<|>)_exts})(-.)"
//! ```
//!
//! sh:12-13's `-D` and `-E` are top-level `zparseopts` builtin flags
//! (delete recognized options from the source array / don't stop at
//! the first unmatched arg) — distinct from the `D=_D` SPEC further
//! along, which recognizes a literal `-D` in `_deb_files`'s own argv
//! and stores it into local array `_D`. Bridged to the real
//! `bin_zparseopts` port via `-v <scratch-array>` exactly as
//! `_wanted.rs`/`_normal.rs` do, since zshrs has no positional-`$@`
//! substrate for this call.

use crate::compsys::ported::_description::description_byname;
use crate::compsys::ported::_files::_files;
use crate::ported::modules::zutil::bin_zparseopts;
use crate::ported::params::{getaparam, setaparam, unsetparam};
use crate::ported::zsh_h::{options, MAX_OPS};

fn make_ops() -> options {
    options {
        ind: [0u8; MAX_OPS],
        args: Vec::new(),
        argscount: 0,
        argsalloc: 0,
    }
}

/// sh:12-13 — bridge to real `bin_zparseopts -a _fopts -D -E -
/// c=_c D=_D 1 2 F+: J+: M+: n P+: q r+: R+: S+: V+: W+: X+:` via
/// `-v <name>` (named array source). Returns `(_fopts, _c, _D)`.
fn run_zparseopts_deb(args: &[String]) -> (Vec<String>, Vec<String>, Vec<String>) {
    let src = "__compsys_argv";
    setaparam(src, args.to_vec());
    setaparam("_fopts", Vec::new());
    setaparam("_c", Vec::new());
    setaparam("_D", Vec::new());
    let _ = bin_zparseopts(
        "zparseopts",
        &[
            "-v".to_string(),
            src.to_string(),
            "-a".to_string(),
            "_fopts".to_string(),
            "-D".to_string(),
            "-E".to_string(),
            "-".to_string(),
            "c=_c".to_string(),
            "D=_D".to_string(),
            "1".to_string(),
            "2".to_string(),
            "F+:".to_string(),
            "J+:".to_string(),
            "M+:".to_string(),
            "n".to_string(),
            "P+:".to_string(),
            "q".to_string(),
            "r+:".to_string(),
            "R+:".to_string(),
            "S+:".to_string(),
            "V+:".to_string(),
            "W+:".to_string(),
            "X+:".to_string(),
        ],
        &make_ops(),
        0,
    );
    let fopts = getaparam("_fopts").unwrap_or_default();
    let c = getaparam("_c").unwrap_or_default();
    let d = getaparam("_D").unwrap_or_default();
    // Tear down the `__compsys_argv` zparseopts-bridge scratch global (not
    // a real zsh identifier; zsh operates on positional $argv). See
    // _wanted.rs / Bug #657 for the same pattern.
    unsetparam(src);
    (fopts, c, d)
}

/// sh:10,15,16 — default extension table plus `-c`/`-D` extras.
/// Factored out standalone so it's testable without global param
/// state: `has_c`/`has_d` are `(( $#_c ))` / `(( $#_D ))`.
fn compute_exts(has_c: bool, has_d: bool) -> Vec<String> {
    let mut exts: Vec<String> = vec!["deb".to_string(), "ddeb".to_string(), "udeb".to_string()];
    if has_c {
        // sh:15 — both pushed unconditionally; NOT deduped against -D's dsc.
        exts.push("changes".to_string());
        exts.push("dsc".to_string());
    }
    if has_d {
        // sh:16
        exts.push("dsc".to_string());
    }
    exts
}

/// `_deb_files` — complete Debian package files (.deb/.ddeb/.udeb, plus
/// .changes/.dsc under `-c`/`-D`).
pub fn _deb_files(args: &[String]) -> i32 {
    let _fn_scope = crate::compsys::ported::shared::FnScope::enter("_deb_files");
    // sh:12-13
    let (fopts, c, d) = run_zparseopts_deb(args);

    // sh:15-16
    let exts = compute_exts(!c.is_empty(), !d.is_empty());

    // sh:18  _description files _expl 'Debian package'
    let _ = description_byname(&[
        "files".to_string(),
        "_expl".to_string(),
        "Debian package".to_string(),
    ]);

    // sh:19  _files "${(@)_fopts}" "${(@)_expl}" -g "*.(${(j<|>)_exts})(-.)"
    let mut a: Vec<String> = fopts;
    a.extend(getaparam("_expl").unwrap_or_default());
    a.push("-g".to_string());
    a.push(format!("*.({})(-.)", exts.join("|")));
    crate::compsys::ported::shared::call_compfn("_files", &a, || _files(&a))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_exts_default() {
        // sh:10 — no -c/-D: just the base three.
        assert_eq!(compute_exts(false, false), vec!["deb", "ddeb", "udeb"]);
    }

    #[test]
    fn compute_exts_c_adds_changes_and_dsc() {
        // sh:15
        assert_eq!(
            compute_exts(true, false),
            vec!["deb", "ddeb", "udeb", "changes", "dsc"]
        );
    }

    #[test]
    fn compute_exts_d_adds_dsc_only() {
        // sh:16
        assert_eq!(
            compute_exts(false, true),
            vec!["deb", "ddeb", "udeb", "dsc"]
        );
    }

    #[test]
    fn compute_exts_c_and_d_both_push_dsc_undeduped() {
        // sh:15-16 — -c pushes dsc, -D pushes dsc again; upstream does
        // not dedupe, so the glob would contain `dsc` twice.
        assert_eq!(
            compute_exts(true, true),
            vec!["deb", "ddeb", "udeb", "changes", "dsc", "dsc"]
        );
    }

    #[test]
    fn zparseopts_routes_c_flag_into_c_array_not_fopts() {
        // sh:12-13 — `c=_c` binds matches to _c, NOT the default _fopts.
        let _g = crate::test_util::global_state_lock();
        let (fopts, c, d) = run_zparseopts_deb(&["-c".to_string()]);
        assert_eq!(fopts, Vec::<String>::new());
        assert_eq!(c, vec!["-c"]);
        assert_eq!(d, Vec::<String>::new());
    }

    #[test]
    fn zparseopts_routes_d_flag_into_d_array_not_fopts() {
        let _g = crate::test_util::global_state_lock();
        let (fopts, c, d) = run_zparseopts_deb(&["-D".to_string()]);
        assert_eq!(fopts, Vec::<String>::new());
        assert_eq!(c, Vec::<String>::new());
        assert_eq!(d, vec!["-D"]);
    }

    #[test]
    fn zparseopts_passthrough_flags_land_in_fopts() {
        // sh:12-13 — `_files`-passthrough specs (J+: takes an arg) land
        // in the default array, not consumed as -c/-D.
        let _g = crate::test_util::global_state_lock();
        let (fopts, c, d) =
            run_zparseopts_deb(&["-J".to_string(), "grp".to_string(), "-n".to_string()]);
        assert_eq!(fopts, vec!["-J", "grp", "-n"]);
        assert_eq!(c, Vec::<String>::new());
        assert_eq!(d, Vec::<String>::new());
    }

    #[test]
    fn returns_one_without_executor() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(_deb_files(&[]), 1);
    }
}
