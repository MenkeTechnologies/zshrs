//! zsh-specific `{ try } always { cleanup }` block parity.
//!
//! The `always` clause runs regardless of how the try block exits
//! (normal, error, TRY_BLOCK_ERROR set). It's zsh's equivalent of
//! try/finally — no other shell has this.

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

    /// Try succeeds → always runs.
    #[test]
    fn always_runs_after_success() {
        assert_parity(
            r#"
{ echo try } always { echo finally }
"#,
        );
    }

    /// Try sets nonzero exit → always runs.
    #[test]
    fn always_runs_after_failure() {
        assert_parity(
            r#"
{ echo try; false } always { echo finally }
echo exit=$?
"#,
        );
    }

    /// Always runs after exit attempt? In a function-scope, return triggers cleanup.
    #[test]
    fn always_runs_after_explicit_false() {
        assert_parity(
            r#"
f() {
  { echo try-pre; return 99 } always { echo always-ran }
}
f
echo "after-call exit=$?"
"#,
        );
    }
}

mod try_block_error {
    use super::*;

    /// `TRY_BLOCK_ERROR` set inside always to 0 swallows the error.
    #[test]
    fn try_block_error_set_to_zero_swallows() {
        assert_parity(
            r#"
{ false } always { TRY_BLOCK_ERROR=0 }
echo "after exit=$?"
"#,
        );
    }

    /// Reading TRY_BLOCK_ERROR inside always reflects try-block exit.
    #[test]
    fn read_try_block_error_in_always() {
        assert_parity(
            r#"
{ false } always { echo tbe=$TRY_BLOCK_ERROR }
"#,
        );
    }
}

mod nesting {
    use super::*;

    /// Nested always blocks.
    #[test]
    fn nested_always_blocks() {
        assert_parity(
            r#"
{
  { echo inner-try } always { echo inner-always }
} always {
  echo outer-always
}
"#,
        );
    }
}

mod with_subshell {
    use super::*;

    /// Always block inside subshell.
    #[test]
    fn always_in_subshell() {
        assert_parity(
            r#"
( { echo try } always { echo finally } )
echo after
"#,
        );
    }
}

mod with_break_continue {
    use super::*;

    /// always runs even when try block triggers `break` from loop.
    #[test]
    fn always_runs_after_break_in_loop() {
        assert_parity(
            r#"
for i in 1 2 3; do
  { [[ $i == 2 ]] && break; echo "try $i" } always { echo "always $i" }
done
echo done
"#,
        );
    }

    /// always runs when try block triggers `continue`.
    #[test]
    fn always_runs_after_continue_in_loop() {
        assert_parity(
            r#"
for i in 1 2 3; do
  { [[ $i == 2 ]] && continue; echo "try $i" } always { echo "always $i" }
done
echo done
"#,
        );
    }
}

mod multiple_statements {
    use super::*;

    /// Multiple statements in try and always.
    #[test]
    fn multistatement_try_always() {
        assert_parity(
            r#"
{
  echo a
  echo b
  echo c
} always {
  echo cleanup-1
  echo cleanup-2
}
"#,
        );
    }
}

mod var_scope {
    use super::*;

    /// Vars set inside try persist into always (same scope as outer
    /// shell — `{ }` doesn't create a new scope).
    #[test]
    fn var_set_in_try_visible_in_always() {
        assert_parity(
            r#"
{ X=set-in-try } always { echo X=$X }
"#,
        );
    }

    /// Vars set in always persist outside the block.
    #[test]
    fn var_set_in_always_persists_after() {
        assert_parity(
            r#"
{ : } always { Y=from-always }
echo Y=$Y
"#,
        );
    }
}

mod with_error_subst {
    use super::*;

    /// Always runs even when command substitution fails.
    #[test]
    fn always_after_failed_cmd_subst() {
        assert_parity(
            r#"
{ X=$(false) } always { echo "always X=[$X]" }
"#,
        );
    }
}

mod always_only_runs_once {
    use super::*;

    #[test]
    fn always_runs_exactly_once_in_loop_iter() {
        assert_parity(
            r#"
n=0
for i in 1 2 3; do
  { : } always { (( n++ )) }
done
echo n=$n
"#,
        );
    }
}
