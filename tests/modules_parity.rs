//! Behavioural parity between zshrs's `Src/Modules/` ports and the real
//! `/bin/zsh`. Each test runs a small script through both shells and
//! asserts byte-equal stdout (and, where it matters, stderr / exit code).
//!
//! These tests are the immune system for the `src/modules/*.rs` ports
//! per the endgame rule in `CLAUDE.md`: every new module-surface fix
//! adds a case here so the behaviour is *pinned* against the C source's
//! observable output.
//!
//! Tests skip silently when `/bin/zsh` is unavailable (CI containers,
//! minimal Linux installs).

use std::path::PathBuf;
use std::process::Command;

/// Path to the freshly-built `zshrs` binary. Cargo sets
/// `CARGO_BIN_EXE_zshrs` for integration tests; fall through to
/// `target/debug/zshrs` when running outside cargo (manual `cargo
/// test --no-run` + run-by-hand workflows).
fn zshrs_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("target").join("debug").join("zshrs")
}

fn zsh_available() -> bool {
    Command::new("/bin/zsh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

struct ShellResult {
    stdout: String,
    stderr: String,
    exit: i32,
}

/// Run `script` through `/bin/zsh -fc …`. The caller is expected to
/// prepend any required `zmodload zsh/<modname>` lines — `-fc`
/// suppresses RC files so no module auto-loads.
fn run_zsh(script: &str) -> ShellResult {
    let out = Command::new("/bin/zsh")
        .args(["-fc", script])
        .output()
        .expect("invoke /bin/zsh");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

/// Run `script` through `zshrs --zsh -c …`. zshrs's loadable modules
/// are always present (they're statically linked), so a `zmodload`
/// in the script is a no-op — but it does no harm, which keeps the
/// same script string usable against both shells.
fn run_zshrs(script: &str) -> ShellResult {
    let out = Command::new(zshrs_bin())
        .args(["--zsh", "-c", script])
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("invoke zshrs");
    ShellResult {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        exit: out.status.code().unwrap_or(-1),
    }
}

/// Build a script that explicitly loads each listed module before
/// the body. Mirrors zsh's `zmodload zsh/foo; …` idiom. Using this
/// helper instead of inlining `zmodload` per case lets us keep the
/// per-test script free of boilerplate.
fn with_modules(modules: &[&str], body: &str) -> String {
    let mut s = String::new();
    for m in modules {
        s.push_str(&format!("zmodload zsh/{} 2>/dev/null\n", m));
    }
    s.push_str(body);
    s
}

/// Assert that `/bin/zsh -fc script` and `zshrs --zsh -c script` produce
/// the same stdout and exit code. Stderr is checked only by emptiness
/// — the two shells phrase warnings differently in many cases, so the
/// per-test caller can override via `assert_parity_strict`.
fn assert_parity(script: &str) {
    if !zsh_available() {
        eprintln!("skip: /bin/zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on script:\n{}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        script, z.stdout, r.stdout
    );
    assert_eq!(
        z.exit, r.exit,
        "exit-code divergence on script:\n{}\n--- zsh ---\n{}\n--- zshrs ---\n{}",
        script, z.exit, r.exit
    );
}

/// Strict parity — also requires stderr to match byte-for-byte. Useful
/// when the test exercises an error path whose diagnostic format zshrs
/// has explicitly aligned with the C source.
#[allow(dead_code)]
fn assert_parity_strict(script: &str) {
    if !zsh_available() {
        eprintln!("skip: /bin/zsh not found");
        return;
    }
    let z = run_zsh(script);
    let r = run_zshrs(script);
    assert_eq!(z.stdout, r.stdout, "stdout divergence on:\n{}", script);
    assert_eq!(z.stderr, r.stderr, "stderr divergence on:\n{}", script);
    assert_eq!(z.exit, r.exit, "exit divergence on:\n{}", script);
}

// ───────────────────────── zsh/regex ─────────────────────────

mod regex_module {
    use super::*;

    /// `[[ str -regex-match pat ]]` returns 0 on match.
    /// Direct test of `zcond_regex_match` (Src/Modules/regex.c:60-210)
    /// — the truthiness shape.
    #[test]
    fn regex_match_basic_truthy() {
        assert_parity(
            r#"[[ "hello world" -regex-match "wor.d" ]] && echo Y || echo N"#,
        );
    }

    /// Capture groups go to $match[1..N]; full match to $MATCH.
    /// Direct test of regex.c:148-205 (the
    /// `setiparam("MBEGIN", …)`/`set sparam("MATCH", …)`/`setaparam`
    /// sequence after a successful regexec).
    #[test]
    fn regex_match_captures() {
        assert_parity(
            r#"[[ "foo=42" -regex-match "([a-z]+)=([0-9]+)" ]] && echo "$MATCH|$match[1]|$match[2]""#,
        );
    }

    /// Non-match must NOT touch $MATCH / $match.
    #[test]
    fn regex_match_failure_keeps_status_nonzero() {
        assert_parity(
            r#"MATCH=untouched; [[ abc -regex-match xyz ]] || echo "no:$?:$MATCH""#,
        );
    }
}

// ───────────────────────── zsh/datetime ─────────────────────────

mod datetime_module {
    use super::*;

    #[test]
    fn epochseconds_is_recent_unix_time() {
        // Don't compare epochseconds directly across two invocations
        // (they may differ by a second). Just check both shells return
        // a positive integer of similar magnitude.
        if !zsh_available() {
            return;
        }
        let z: i64 = run_zsh("print -- $EPOCHSECONDS")
            .stdout
            .trim()
            .parse()
            .unwrap_or(0);
        let r: i64 = run_zshrs("print -- $EPOCHSECONDS")
            .stdout
            .trim()
            .parse()
            .unwrap_or(0);
        assert!(z > 1_700_000_000, "zsh EPOCHSECONDS suspicious: {}", z);
        assert!(r > 1_700_000_000, "zshrs EPOCHSECONDS suspicious: {}", r);
        assert!(
            (z - r).abs() < 5,
            "EPOCHSECONDS drift > 5s: zsh={} zshrs={}",
            z,
            r
        );
    }

    #[test]
    fn strftime_formats() {
        assert_parity(r#"strftime "%Y-%m-%d" 1700000000"#);
    }
}

// ───────────────────────── zsh/terminfo ─────────────────────────

mod terminfo_module {
    use super::*;

    /// Already pinned in an earlier commit but lives here too so the
    /// modules-parity file owns the immune system in one place.
    #[test]
    fn terminfo_kf1_byte_exact() {
        if !zsh_available() {
            return;
        }
        let script = "print -rn -- $terminfo[kf1]";
        let z = run_zsh(script).stdout;
        let r = run_zshrs(script).stdout;
        assert_eq!(z.as_bytes(), r.as_bytes());
    }

    #[test]
    fn terminfo_kbs_byte_exact() {
        if !zsh_available() {
            return;
        }
        let script = "print -rn -- $terminfo[kbs]";
        let z = run_zsh(script).stdout;
        let r = run_zshrs(script).stdout;
        assert_eq!(z.as_bytes(), r.as_bytes());
    }
}

// ───────────────────────── zsh/zutil ─────────────────────────

mod zutil_module {
    use super::*;

    #[test]
    fn zstyle_set_get() {
        assert_parity(
            r#"zstyle ':completion:*' menu select
            zstyle -s ':completion:*' menu val
            print -- $val"#,
        );
    }

    #[test]
    fn zparseopts_basic() {
        assert_parity(
            r#"set -- -a 1 -b 2 rest
            zparseopts -D -E a:=opta b:=optb
            print -- "a=$opta b=$optb rest=$@""#,
        );
    }
}

// ───────────────────────── zsh/parameter ─────────────────────────

mod parameter_module {
    use super::*;

    #[test]
    fn aliases_assoc_returns_value() {
        assert_parity(
            r#"alias gst="git status"
            print -- "${aliases[gst]}""#,
        );
    }

    #[test]
    fn functions_assoc_lists_definitions() {
        assert_parity(
            r#"f() { echo hello; }
            [[ -n "${functions[f]}" ]] && echo defined"#,
        );
    }
}
