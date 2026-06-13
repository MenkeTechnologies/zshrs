//! IFS / word-splitting parity tests.
//!
//! NOTE: zsh by default does NOT word-split parameter expansions
//! (unlike bash). It splits only via $= flag, the (s/x/) flag, or
//! when SH_WORD_SPLIT option is set.

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

mod default_no_split {
    use super::*;

    /// zsh default: $var doesn't word-split, stays one word.
    #[test]
    fn unquoted_var_doesnt_split_by_default() {
        assert_parity(r#"X="a b c"; f() { echo $#; }; f $X"#);
    }

    /// "$var" never splits.
    #[test]
    fn quoted_var_stays_one_arg() {
        assert_parity(r#"X="a b c"; f() { echo $#; }; f "$X""#);
    }
}

mod equals_split {
    use super::*;

    /// `$=var` forces splitting.
    #[test]
    fn equals_prefix_forces_split() {
        assert_parity(r#"X="a b c"; f() { echo $#; }; f $=X"#);
    }

    /// `$=var` uses $IFS for splitting.
    #[test]
    fn equals_split_uses_ifs() {
        assert_parity(r#"X="a:b:c"; IFS=:; f() { echo $#; }; f $=X"#);
    }
}

mod sh_word_split {
    use super::*;

    /// `setopt SH_WORD_SPLIT` makes $var split like POSIX shells.
    #[test]
    fn shwordsplit_enables_unquoted_split() {
        assert_parity(r#"setopt SH_WORD_SPLIT; X="a b c"; f() { echo $#; }; f $X"#);
    }

    /// Even with SH_WORD_SPLIT, "$var" stays one arg.
    #[test]
    fn shwordsplit_doesnt_affect_quoted_var() {
        assert_parity(r#"setopt SH_WORD_SPLIT; X="a b c"; f() { echo $#; }; f "$X""#);
    }
}

mod custom_ifs {
    use super::*;

    /// IFS=: with `$=X` splits on colon.
    #[test]
    fn ifs_colon_splits_on_colon() {
        assert_parity(r#"IFS=:; X="a:b:c"; f() { echo $#; }; f $=X"#);
    }

    /// IFS empty disables splitting entirely.
    #[test]
    fn ifs_empty_disables_split() {
        assert_parity(r#"IFS=; X="a b c"; f() { echo $#; }; f $=X"#);
    }
}

mod for_loop_iteration {
    use super::*;

    /// `for x in $var` with default IFS — zsh: one iter (no split).
    #[test]
    fn for_in_unquoted_var_no_split_zsh_default() {
        assert_parity(r#"X="a b c"; n=0; for x in $X; do n=$((n+1)); done; echo $n"#);
    }

    /// With SH_WORD_SPLIT, three iters.
    #[test]
    fn for_in_unquoted_var_splits_with_shwordsplit() {
        assert_parity(
            r#"setopt SH_WORD_SPLIT; X="a b c"; n=0; for x in $X; do n=$((n+1)); done; echo $n"#,
        );
    }

    /// for-in with $=X forces split → three iters.
    #[test]
    fn for_in_equals_forces_split() {
        assert_parity(r#"X="a b c"; n=0; for x in $=X; do n=$((n+1)); done; echo $n"#);
    }

    /// For-in with literal list — clear three iters.
    #[test]
    fn for_in_literal_list_three_iters() {
        assert_parity(r#"n=0; for x in a b c; do n=$((n+1)); done; echo $n"#);
    }
}

mod cmdsubst_split {
    use super::*;

    /// Unquoted $(...) — zsh DOES split (per zsh docs, $(...) always splits).
    #[test]
    fn cmdsubst_splits_in_zsh() {
        assert_parity(r#"f() { echo $#; }; f $(echo a b c)"#);
    }

    /// "$( )" never splits.
    #[test]
    fn quoted_cmdsubst_no_split() {
        assert_parity(r#"f() { echo $#; }; f "$(echo a b c)""#);
    }

    /// $(...) with IFS=: and colon-separated output.
    #[test]
    fn cmdsubst_with_ifs_colon() {
        assert_parity(r#"IFS=:; f() { echo $#; }; f $(echo a:b:c)"#);
    }
}

mod ifs_in_read {
    use super::*;

    /// `read` uses IFS to split input.
    #[test]
    fn read_splits_on_ifs_colon() {
        assert_parity(r#"IFS=: read X Y Z <<< 'one:two:three'; echo "[$X][$Y][$Z]""#);
    }

    /// `IFS=` read keeps whole line in first var.
    #[test]
    fn read_with_empty_ifs_no_split() {
        assert_parity(r#"IFS= read X Y <<< 'one two three'; echo "[$X][$Y]""#);
    }
}

mod ifs_multi_char {
    use super::*;

    /// Multi-char IFS — each char in IFS is a splitter.
    #[test]
    fn ifs_multi_char_each_splits() {
        assert_parity(r#"IFS=':|'; f() { echo $#; }; f $=$"$(echo 'a:b|c:d')""#);
    }
}
