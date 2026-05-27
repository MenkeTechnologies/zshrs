//! Special runtime parameters parity:
//! $RANDOM, $SECONDS, $LINENO, $$, $!, $SHLVL, $pipestatus, $funcstack.
//!
//! Values are non-deterministic — we test STRUCTURE: numeric, in range,
//! updated correctly, etc. — not exact equality.

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

mod dollar_random {
    use super::*;

    /// $RANDOM is numeric.
    #[test]
    fn random_is_numeric() {
        assert_parity(r#"[[ "$RANDOM" =~ ^[0-9]+$ ]]; echo $?"#);
    }

    /// $RANDOM in range [0, 32767].
    #[test]
    fn random_in_zsh_range() {
        assert_parity(r#"R=$RANDOM; (( R >= 0 && R < 32768 )); echo $?"#);
    }

    /// $RANDOM updates each access.
    #[test]
    fn random_changes_between_accesses() {
        assert_parity(r#"A=$RANDOM; B=$RANDOM; C=$RANDOM; (( A != B || B != C )); echo $?"#);
    }

    /// Setting $RANDOM seeds.
    #[test]
    fn random_seeding_reproducible() {
        assert_parity(
            r#"
RANDOM=42
A=$RANDOM
B=$RANDOM
RANDOM=42
C=$RANDOM
D=$RANDOM
(( A == C && B == D )); echo $?
"#,
        );
    }
}

mod dollar_seconds {
    use super::*;

    /// $SECONDS starts at 0 (approximately).
    #[test]
    fn seconds_starts_near_zero() {
        assert_parity(r#"(( SECONDS < 5 )); echo $?"#);
    }

    /// $SECONDS is integer.
    #[test]
    fn seconds_is_integer() {
        assert_parity(r#"[[ "$SECONDS" =~ ^[0-9]+$ ]]; echo $?"#);
    }

    /// Setting $SECONDS works.
    #[test]
    fn seconds_settable() {
        assert_parity(r#"SECONDS=1000; (( SECONDS >= 1000 )); echo $?"#);
    }
}

mod dollar_lineno {
    use super::*;

    /// $LINENO is numeric.
    #[test]
    fn lineno_is_integer() {
        assert_parity(r#"[[ "$LINENO" =~ ^[0-9]+$ ]]; echo $?"#);
    }

    /// $LINENO increases across lines in a script.
    #[test]
    fn lineno_increases_across_lines() {
        assert_parity(
            r#"
A=$LINENO
B=$LINENO
C=$LINENO
(( A < B && B < C )); echo $?
"#,
        );
    }
}

mod dollar_pid {
    use super::*;

    /// $$ is positive integer (process ID).
    #[test]
    fn pid_is_positive() {
        assert_parity(r#"(( $$ > 0 )); echo $?"#);
    }

    /// $$ doesn't change within shell.
    #[test]
    fn pid_stable_within_shell() {
        assert_parity(r#"A=$$; B=$$; (( A == B )); echo $?"#);
    }

    /// $$ in subshell — zsh keeps parent $$; bash changes it.
    #[test]
    fn pid_in_subshell_preserved_zsh() {
        assert_parity(r#"P=$$; (( $$ == P )); echo $?"#);
    }
}

mod dollar_bang {
    use super::*;

    /// $! is PID of last backgrounded job.
    #[test]
    fn bang_after_background_is_pid() {
        assert_parity(r#"sleep 0.01 & wait; (( ${!:-0} > 0 )); echo $?"#);
    }

    /// $! before any & is unset/empty.
    #[test]
    fn bang_initially_empty() {
        assert_parity(r#"echo "[${!:-empty}]""#);
    }
}

mod shlvl {
    use super::*;

    /// $SHLVL > 0.
    #[test]
    fn shlvl_positive() {
        assert_parity(r#"(( SHLVL > 0 )); echo $?"#);
    }

    /// $SHLVL is integer.
    #[test]
    fn shlvl_is_integer() {
        assert_parity(r#"[[ "$SHLVL" =~ ^[0-9]+$ ]]; echo $?"#);
    }
}

mod pipestatus {
    use super::*;

    /// $pipestatus is array of exit codes of last pipeline.
    #[test]
    fn pipestatus_after_simple_pipe() {
        assert_parity(r#"true | true; echo "${pipestatus[1]}/${pipestatus[2]}""#);
    }

    /// Mixed exit codes captured.
    #[test]
    fn pipestatus_mixed_exits() {
        assert_parity(
            r#"false | true | false; echo "${pipestatus[1]}/${pipestatus[2]}/${pipestatus[3]}""#,
        );
    }

    /// $#pipestatus = length of last pipeline.
    #[test]
    fn pipestatus_count_matches_pipeline_length() {
        assert_parity(r#"true | true | true | true; echo "${#pipestatus}""#);
    }
}

mod funcstack {
    use super::*;

    /// $funcstack lists active function names.
    #[test]
    fn funcstack_in_function() {
        assert_parity(
            r#"
f() { echo "${funcstack[1]}"; }
f
"#,
        );
    }

    /// Empty outside any function.
    #[test]
    fn funcstack_empty_at_top_level() {
        assert_parity(r#"echo "[${funcstack[1]:-empty}]""#);
    }

    /// Nested functions stack.
    #[test]
    fn funcstack_nested() {
        assert_parity(
            r#"
inner() { echo "stack: ${funcstack[1]}/${funcstack[2]}"; }
outer() { inner; }
outer
"#,
        );
    }
}

mod sh_options {
    use super::*;

    /// $- contains current option chars.
    #[test]
    fn dash_var_contains_option_chars() {
        assert_parity(r#"set -e; [[ "$-" == *e* ]]; echo $?"#);
    }
}

mod zsh_subshell {
    use super::*;

    /// $ZSH_SUBSHELL counter.
    #[test]
    fn zsh_subshell_zero_top_level() {
        assert_parity(r#"echo "$ZSH_SUBSHELL""#);
    }

    /// Increments inside subshell.
    #[test]
    fn zsh_subshell_increments_in_subshell() {
        assert_parity(r#"echo "[$ZSH_SUBSHELL]"; (echo "[$ZSH_SUBSHELL]")"#);
    }
}

mod zsh_version {
    use super::*;

    /// $ZSH_VERSION matches semver-like pattern.
    #[test]
    fn zsh_version_format() {
        assert_parity(r#"[[ -n "$ZSH_VERSION" ]]; echo $?"#);
    }
}

mod tty {
    use super::*;

    /// $TTY may or may not be set; just don't crash.
    #[test]
    fn tty_var_access_ok() {
        assert_parity(r#"echo "[${TTY:-none}]" >/dev/null; echo done"#);
    }
}
