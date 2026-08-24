//! `~` (tilde) and `=cmd` expansion parity tests.
//!
//! ZSHRS-WIDE BUG CLUSTER: most tilde-extension forms diverge from zsh.
//! Plain `~` → `$HOME` works; same with `~/sub` and tilde in assignments.
//! But `~+`, `~-`, `~0`, `~user`, `~` inside double quotes (zsh
//! extension), `=cmd` (EQUALS option) all fail. 11 of 20 tests fail;
//! all marked #[ignore] with FIXME.

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

mod tilde_home {
    use super::*;

    #[test]
    fn tilde_expands_to_home() {
        assert_parity(r#"[[ "$(echo ~)" == "$HOME" ]]; echo $?"#);
    }

    #[test]
    fn tilde_slash_expands() {
        assert_parity(r#"[[ "$(echo ~/sub)" == "$HOME/sub" ]]; echo $?"#);
    }

    #[test]
    fn tilde_inside_double_quote_zsh_expands() {
        assert_parity(r#"echo "~""#);
    }

    #[test]
    fn tilde_inside_single_quote_literal() {
        assert_parity(r#"echo '~'"#);
    }

    #[test]
    fn tilde_not_at_word_start_literal() {
        assert_parity(r#"echo abc~def"#);
    }
}

mod tilde_named_user {
    use super::*;

    #[test]
    fn tilde_root_user_expands() {
        if !zsh_available() {
            return;
        }
        let probe = run_zsh(r#"echo ~root"#);
        if probe.stdout.trim().starts_with('~') {
            return;
        }
        assert_parity(r#"echo ~root"#);
    }

    #[test]
    fn tilde_unknown_user_stays_literal() {
        assert_parity(r#"echo ~nonexistent_user_xyz_zzz_42"#);
    }
}

mod tilde_plus_minus {
    use super::*;

    #[test]
    fn tilde_plus_is_pwd() {
        assert_parity(r#"cd /tmp; [[ "$(echo ~+)" == "$(pwd)" ]]; echo $?"#);
    }

    #[test]
    fn tilde_minus_is_oldpwd() {
        assert_parity(
            r#"cd /; cd /tmp; [[ "$(echo ~-)" == "/" ]] || [[ "$(echo ~-)" == "$OLDPWD" ]]; echo $?"#,
        );
    }

    #[test]
    fn tilde_zero_is_top_of_dirstack() {
        assert_parity(r#"cd /tmp; [[ "$(echo ~0)" == "$(pwd)" ]]; echo $?"#);
    }
}

mod tilde_in_assignments {
    use super::*;

    #[test]
    fn tilde_in_assignment_value_expands() {
        assert_parity(r#"X=~; [[ "$X" == "$HOME" ]]; echo $?"#);
    }

    #[test]
    fn tilde_after_colon_in_assignment() {
        assert_parity(r#"X=/usr/bin:~/bin; echo "$X""#);
    }

    #[test]
    fn tilde_in_arg_position_expands() {
        assert_parity(r#"echo ~"#);
    }
}

mod equals_command_lookup {
    use super::*;

    #[test]
    fn equals_resolves_to_full_path() {
        assert_parity(r#"setopt EQUALS; echo =ls"#);
    }

    #[test]
    fn equals_unknown_cmd_stays_or_errors() {
        assert_parity(r#"setopt EQUALS; echo =nonexistent_xyz_42_zzz 2>/dev/null"#);
    }

    #[test]
    fn equals_without_option_stays_literal() {
        assert_parity(r#"unsetopt EQUALS 2>/dev/null; echo =ls"#);
    }
}

mod tilde_in_complex {
    use super::*;

    #[test]
    fn tilde_in_default_expansion() {
        assert_parity(r#"echo "${UNSET:-~/x}""#);
    }

    /// Multiple tildes in one word.
    #[test]
    fn multiple_tildes_in_one_word() {
        assert_parity(r#"echo ~/a:~/b"#);
    }
}

mod no_expand {
    use super::*;

    #[test]
    fn backslash_tilde_literal() {
        assert_parity(r#"echo \~"#);
    }

    /// Tilde in middle of path — literal.
    #[test]
    fn tilde_mid_path_literal() {
        assert_parity(r#"echo /home/~/x"#);
    }
}

/// Named-directory names allow the full IUSER class (alnum + `_` + `-`
/// + `.`), both in `hash -d NAME=path` registration (c:builtin.c:4285
/// itype_end IUSER) and in `~NAME` tilde expansion (c:subst.c filesub,
/// utils.c:4173-4191). The port previously allowed only alnum + `_`, so
/// `hash -d t-t=/foo; print ~t-t` errored then left `~t-t` unexpanded.
mod named_dir_iuser_chars {
    use super::*;

    #[test]
    fn hyphen_in_name() {
        assert_parity(r#"hash -d t-t=/foo; print ~t-t/bar"#);
    }

    #[test]
    fn multiple_hyphens() {
        assert_parity(r#"hash -d a-b-c=/x; print ~a-b-c"#);
    }

    #[test]
    fn dot_in_name() {
        assert_parity(r#"hash -d a.b=/y; print ~a.b/z"#);
    }

    /// Invalid char (`/`) still rejected (regression guard).
    #[test]
    fn slash_still_invalid() {
        assert_parity(r#"hash -d "a/b"=/x 2>/dev/null; echo rc=$?"#);
    }

    /// Plain alnum name still works (regression guard).
    #[test]
    fn plain_name_unaffected() {
        assert_parity(r#"hash -d good=/g; print ~good"#);
    }
}

/// A leading `~` in a MATCH PATTERN — `[[ … == ~/* ]]`, `${~var}` holding
/// `~/*`, and `case` arms. c:Src/cond.c:299-307 `singsub`s the RHS before
/// `patcompile`, and c:Src/loop.c:610 does the same for `case`, so the
/// tilde is a home directory on both.
///
/// powerlevel10k's `_POWERLEVEL9K_DIR_CLASSES` walk (internal/p10k.zsh:2029)
/// is the real-world consumer: `[[ $_p9k__cwd == ${~a} ]]` against `~` and
/// `~/*`. When those missed, every path under `$HOME` fell through to the
/// `*` class and the prompt drew the wrong directory icon. BUGS.md #1088.
mod tilde_in_match_pattern {
    use super::*;

    #[test]
    fn cond_rhs_literal_tilde_slash_star() {
        assert_parity(r#"cwd=$HOME/a/b; [[ $cwd == ~/* ]] && print sub || print no"#);
    }

    #[test]
    fn cond_rhs_bare_tilde_matches_home() {
        assert_parity(r#"[[ $HOME == ~ ]] && print home || print no"#);
    }

    #[test]
    fn cond_rhs_named_user_tilde() {
        assert_parity(r#"cwd=$HOME/a; [[ $cwd == ~$USERNAME/* ]] && print sub || print no"#);
    }

    /// The `${~var}` spelling p10k actually uses — GLOB_SUBST promotes the
    /// value to a pattern, and prefork's `filesub` still expands its tilde.
    #[test]
    fn cond_rhs_globsubst_var_holding_tilde_pattern() {
        assert_parity(r#"cwd=$HOME/a/b; a='~/*'; [[ $cwd == ${~a} ]] && print sub || print no"#);
    }

    /// The p10k class walk in miniature: first match wins, so `~` must not
    /// swallow a subfolder and `~/*` must beat the catch-all `*`.
    #[test]
    fn dir_classes_walk_picks_home_subfolder() {
        assert_parity(
            r#"cwd=$HOME/a/b
               for a in '/etc|/etc/*' '~' '~/*' '*'; do
                 [[ $cwd == ${~a} ]] && { print -r -- "match=$a"; break }
               done"#,
        );
    }

    #[test]
    fn dir_classes_walk_picks_home_exactly() {
        assert_parity(
            r#"cwd=$HOME
               for a in '/etc|/etc/*' '~' '~/*' '*'; do
                 [[ $cwd == ${~a} ]] && { print -r -- "match=$a"; break }
               done"#,
        );
    }

    #[test]
    fn case_arm_tilde_slash_star() {
        assert_parity(
            r#"cwd=$HOME/a; case $cwd in (~) print home;; (~/*) print sub;; (*) print def;; esac"#,
        );
    }

    /// Regression guard: a QUOTED leading tilde stays literal.
    #[test]
    fn quoted_tilde_stays_literal() {
        assert_parity(r#"[[ '~/a' == '~/'* ]] && print lit || print no"#);
    }

    /// Regression guard: without the `~` flag the substituted value is
    /// literal text, so no tilde expansion and no glob promotion.
    #[test]
    fn plain_var_pattern_does_not_tilde_expand() {
        assert_parity(r#"cwd=$HOME/a; p='~/*'; [[ $cwd == $p ]] && print match || print no"#);
    }

    /// Regression guard: a NON-leading `~` is EXTENDED_GLOB's "except"
    /// operator and must survive the filesub round trip untouched.
    #[test]
    fn extended_glob_except_operator_unaffected() {
        assert_parity(
            r#"setopt extendedglob
               [[ ac == (a*)~ab ]] && print c1 || print n1
               [[ ab == (a*)~ab ]] && print c2 || print n2"#,
        );
    }
}
