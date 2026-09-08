//! `setopt`/`unsetopt` deep parity:
//! -m pattern, listing, no_underscore alias, set / [[ -o opt ]], INTERACTIVE_COMMENTS, etc.

use std::path::{Path, PathBuf};
use std::process::Command;

fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}
fn zsh_path() -> &'static str {
    if Path::new("/opt/homebrew/bin/zsh").exists() {
        "/opt/homebrew/bin/zsh"
    } else if Path::new("/usr/local/bin/zsh").exists() {
        "/usr/local/bin/zsh"
    } else {
        "/bin/zsh"
    }
}
fn zsh_available() -> bool {
    Command::new(zsh_path())
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
struct R {
    stdout: String,
    exit: i32,
}
fn run_zsh(s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs(s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity(s: &str) {
    if !zsh_available() {
        return;
    }
    let z = run_zsh(s);
    let r = run_zshrs(s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}

mod basic {
    use super::*;

    /// `setopt extendedglob` then `[[ -o extendedglob ]]`.
    #[test]
    fn setopt_then_test_o() {
        assert_parity(r#"setopt extendedglob; [[ -o extendedglob ]]; echo $?"#);
    }

    /// `[[ -o opt ]]` on an invalid option name returns $?=3 (per
    /// zsh's `[[ -o invalid ]]` semantics — distinct from unset's $?=1).
    #[test]
    fn test_o_not_set_default() {
        assert_parity(r#"[[ -o nonexistent_option_xyz ]] 2>/dev/null; echo $?"#);
    }

    /// `unsetopt extendedglob` toggles back.
    #[test]
    fn unsetopt_clears() {
        assert_parity(
            r#"setopt extendedglob; unsetopt extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod underscore_case_alias {
    use super::*;

    /// `setopt EXTENDED_GLOB` (underscored uppercase) same as `extendedglob`.
    #[test]
    fn underscored_uppercase_works() {
        assert_parity(r#"setopt EXTENDED_GLOB; [[ -o extendedglob ]]; echo $?"#);
    }

    /// CamelCase `setopt ExtendedGlob` should also work.
    #[test]
    fn camel_case_works() {
        assert_parity(r#"setopt ExtendedGlob; [[ -o extendedglob ]]; echo $?"#);
    }

    /// `no_extended_glob` is the negation form.
    #[test]
    fn no_prefix_negates() {
        assert_parity(
            r#"setopt extendedglob; setopt no_extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod no_alias_options {
    use super::*;

    /// Some opts have built-in `NO_` alias for inverse meaning.
    /// `setopt nounset` = `setopt no_unset` etc.
    #[test]
    fn nounset_via_short_form() {
        assert_parity(r#"setopt nounset; [[ -o nounset ]]; echo $?"#);
    }
}

mod setopt_m_pattern {
    use super::*;

    /// `setopt -m 'extended*'` matches multiple options.
    #[test]
    fn setopt_m_pattern_enables_multi() {
        assert_parity(r#"setopt -m 'extended*'; [[ -o extendedglob ]]; echo $?"#);
    }
}

mod listing {
    use super::*;

    /// `setopt` (no arg) lists active options.
    #[test]
    fn setopt_no_arg_lists() {
        assert_parity(r#"setopt extendedglob; setopt 2>&1 | grep -c extendedglob"#);
    }
}

mod set_dash_o {
    use super::*;

    /// `set -o name` enables.
    #[test]
    fn set_dash_o_enables() {
        assert_parity(r#"set -o extendedglob; [[ -o extendedglob ]]; echo $?"#);
    }

    /// `set +o name` disables.
    #[test]
    fn set_plus_o_disables() {
        assert_parity(
            r#"set -o extendedglob; set +o extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod set_dash_short_flags {
    use super::*;

    /// `set -e` = errexit.
    #[test]
    fn set_dash_e_is_errexit() {
        assert_parity(r#"set -e; [[ -o errexit ]]; echo $?"#);
    }

    /// `set -u` = nounset.
    #[test]
    fn set_dash_u_is_nounset() {
        assert_parity(r#"set -u; [[ -o nounset ]]; echo $?"#);
    }

    /// `set -x` = xtrace.
    #[test]
    fn set_dash_x_is_xtrace() {
        assert_parity(r#"set -x 2>/dev/null; [[ -o xtrace ]]; echo $?"#);
    }
}

mod common_options_behavior {
    use super::*;

    /// `setopt err_exit` then false → script aborts (in subshell).
    #[test]
    fn errexit_aborts_on_false() {
        assert_parity(r#"(set -e; false; echo "should not print"); echo exit=$?"#);
    }

    /// `setopt no_unset` (nounset) → reading unset var errors.
    #[test]
    fn nounset_errors_on_unset_var() {
        assert_parity(r#"(setopt nounset; echo "[$NONEXISTENT_XYZ]") 2>/dev/null; echo exit=$?"#);
    }

    /// `setopt pipefail` propagates first nonzero pipe exit.
    #[test]
    fn pipefail_propagates_first_failure() {
        assert_parity(r#"setopt pipefail; false | true; echo $?"#);
    }
}

mod interactive_comments {
    use super::*;

    /// Default zsh non-interactive: comments work in scripts.
    #[test]
    fn comments_in_script() {
        assert_parity(
            r#"
# this is a comment
echo hi
# trailing
"#,
        );
    }

    /// `setopt interactive_comments` enables `#` in interactive shells —
    /// non-interactive: always allowed.
    #[test]
    fn setopt_interactive_comments_takes() {
        assert_parity(r#"setopt interactive_comments; echo "before" # inline; echo "after""#);
    }
}

mod glob_options {
    use super::*;

    /// `setopt null_glob` → unmatched globs become empty.
    #[test]
    fn null_glob_unmatched_empty() {
        assert_parity(
            r#"setopt null_glob; for f in /tmp/nonexistent_xyz_*.txt; do echo "[$f]"; done; echo done"#,
        );
    }

    /// `setopt no_match` (default) → unmatched globs error.
    #[test]
    fn nomatch_errors_on_unmatched_glob() {
        assert_parity(r#"echo /tmp/nonexistent_xyz_*.txt 2>/dev/null; echo exit=$?"#);
    }

    /// `unsetopt nomatch` allows unmatched globs (literal).
    #[test]
    fn unsetopt_nomatch_allows_literal() {
        assert_parity(r#"unsetopt nomatch; echo /tmp/nonexistent_xyz_*.txt"#);
    }
}

mod option_precedence {
    use super::*;

    /// Last setopt wins.
    #[test]
    fn last_setopt_wins() {
        assert_parity(
            r#"setopt extendedglob; unsetopt extendedglob; setopt extendedglob; [[ -o extendedglob ]]; echo $?"#,
        );
    }
}

mod option_persist_in_subshell {
    use super::*;

    /// Options set in outer visible in subshell.
    #[test]
    fn options_inherited_by_subshell() {
        assert_parity(
            r#"setopt extendedglob; (echo "[[ -o extendedglob ]] is: $([[ -o extendedglob ]]; echo $?)")"#,
        );
    }

    /// Options set in subshell don't leak out.
    #[test]
    fn subshell_setopt_no_leak() {
        assert_parity(r#"(setopt extendedglob); [[ -o extendedglob ]]; echo $?"#);
    }
}

/// SH_GLOB disables `(` as a pattern grouping character
/// (c:Src/pattern.c:500-510 `zpc_special[ZPC_INPAR] = Marker`), and the
/// consequences split by WHERE the pattern's tokenization was decided.
///
/// A `[[ … ]]` / `case` pattern is tokenized once, by the parser, before
/// `setopt shglob` runs. Its `(` is therefore a grouping token that SH_GLOB
/// then disables, which strands the `)`: `patcompbranch` stops on it and
/// `patcompswitch`'s c:Src/pattern.c:913-917 termination test rejects the
/// whole pattern. That is what real zsh prints when compsys runs
/// `_arguments` (whose line 14 is `while [[ "$1" = -([AMO]*|[0CRSWnsw]) ]]`)
/// under an option state carrying SH_GLOB:
///     _arguments:15: bad pattern: -([AMO]*|[0CRSWnsw])
///
/// A `${…}` pattern operand is re-lexed at RUN time
/// (c:Src/subst.c:3382-3393 `parse_subst_string(s)` / `shtokenize(s)`), and a
/// value spliced in by `${~spec}` / GLOB_SUBST is tokenized at run time too
/// (c:Src/subst.c:822/830 `if (glbsub) shtokenize(dest)`); both read SH_GLOB
/// as it stands NOW, and `zshtokenize` declines to tokenize `(`, `|` and `)`
/// under ZSHTOK_SHGLOB (c:Src/glob.c:3575-3580, :3617-3620). Those patterns
/// therefore hold three ordinary characters and never fail to compile.
///
/// The two halves have to move together: rejecting the stranded `)` without
/// the run-time-tokenization half turns every `${x#…(…|…)…}` under SH_GLOB
/// into a spurious "bad pattern".
mod shglob_paren_grouping {
    use super::*;

    /// stdout + stderr + exit, because the whole point of these cases is a
    /// diagnostic on stderr and a status the caller can see.
    fn assert_full_parity(s: &str) {
        if !zsh_available() {
            return;
        }
        let z = Command::new(zsh_path())
            .args(["-fc", s])
            .output()
            .expect("zsh");
        let r = Command::new(zshrs_bin())
            .args(["--zsh", "-f", "-c", s])
            .env_remove("ZSHRS_CACHE")
            .output()
            .expect("zshrs");
        assert_eq!(
            String::from_utf8_lossy(&z.stdout),
            String::from_utf8_lossy(&r.stdout),
            "stdout divergence on:\n{s}"
        );
        assert_eq!(
            String::from_utf8_lossy(&z.stderr),
            String::from_utf8_lossy(&r.stderr),
            "stderr divergence on:\n{s}"
        );
        assert_eq!(
            z.status.code().unwrap_or(-1),
            r.status.code().unwrap_or(-1),
            "exit divergence on:\n{s}"
        );
    }

    /// The `_arguments` line 14 pattern, verbatim. zsh:
    /// `zsh:1: bad pattern: -([AMO]*|[0CRSWnsw])`.
    #[test]
    fn cond_stranded_close_paren_is_a_bad_pattern() {
        assert_full_parity(
            r#"setopt shglob; [[ "-A" = -([AMO]*|[0CRSWnsw]) ]] && echo M || echo N"#,
        );
    }

    /// c:Src/pattern.c:1292-1298 — with `(` disabled and no group open, a `)`
    /// reached while `patcomppiece` is scanning a LITERAL run is swallowed as
    /// an ordinary character, so `-(a|b)` compiles to `-(a` | `b)` and matches
    /// the literal text `-(a`. Only a `)` handed back to `patcompbranch` by a
    /// metacharacter (the `*` in `-(a|b*)`) strands.
    #[test]
    fn cond_close_paren_after_literal_run_stays_ordinary() {
        assert_full_parity(r#"setopt shglob; [[ "-(a" = -(a|b) ]] && echo M || echo N"#);
    }

    /// Same split for a `case` arm (c:Src/loop.c:663-667 `zerr("bad pattern")`),
    /// which is the other consumer of parser-decided tokenization.
    #[test]
    fn case_arm_stranded_close_paren_is_a_bad_pattern() {
        assert_full_parity(r#"setopt shglob; case "-A" in -(a|b*)) echo C1;; *) echo C2;; esac"#);
    }

    #[test]
    fn case_arm_close_paren_after_literal_run_stays_ordinary() {
        assert_full_parity(r#"setopt shglob; case "-A" in -(a|b)) echo C1;; *) echo C2;; esac"#);
    }

    /// A `${x#pat}` operand is re-lexed at run time, so all three characters
    /// are ordinary and the whole `-(a|b*)` matches as literal text.
    #[test]
    fn brace_param_pattern_parens_are_literal() {
        assert_full_parity(r#"setopt shglob; s='-(a|b*)x'; print -r -- ${s#-(a|b*)}"#);
    }

    /// Same for the replace and array-filter operands — these are the shapes
    /// that a naive rejection breaks.
    #[test]
    fn brace_param_replace_and_filter_do_not_error() {
        assert_full_parity(r#"setopt shglob; s='-a'; print -r -- ${s//-(a|b*)/Y}"#);
        assert_full_parity(r#"setopt shglob; a=('-a' zz); print -r -- ${a:#-(a|b*)}"#);
    }

    /// `${~spec}` and GLOB_SUBST splice a VALUE, tokenized by `shtokenize`
    /// under the run-time option state — no grouping, no diagnostic.
    #[test]
    fn tilde_globsubst_value_parens_are_literal() {
        assert_full_parity(
            r#"setopt shglob; p='-([AMO]*|[0CRSWnsw])'; [[ "-s" = $~p ]] && echo M || echo N"#,
        );
        assert_full_parity(
            r#"setopt shglob globsubst; p='-(a|b*)'; [[ "-a" = $p ]] && echo M || echo N"#,
        );
        assert_full_parity(
            r#"emulate ksh -c 'p="-([AMO]*|[0CRSWnsw])"; [[ "-s" = $~p ]] && echo M || echo N'"#,
        );
    }

    /// Without SH_GLOB nothing changes: `(` still groups, so the same pattern
    /// matches and no termination test can fire.
    #[test]
    fn without_shglob_the_group_still_matches() {
        assert_full_parity(r#"[[ "-a" = -(a|b) ]] && echo M || echo N"#);
    }

    /// An UNBALANCED top-level `)` — `(` is a live grouping character here, so
    /// the `)` closes nothing and is ordinary text (the `_rm` option-filter
    /// idiom). Pinned so the termination test cannot start rejecting it.
    #[test]
    fn unbalanced_close_paren_stays_ordinary_under_normal_glob() {
        assert_full_parity(r#"a=('x)--y' z); print -r -- ${a:#*)--*}"#);
    }
}
