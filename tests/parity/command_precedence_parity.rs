//! Command-word precedence pins: `alias > reserved word > function >
//! builtin > external`.
//!
//! The critical case these pin is the CROSS-compile-unit one: a function
//! defined in one interactive line shadowing a builtin invoked on a LATER
//! line. The single-`-c` compiler resolves that at compile time via
//! `user_function_shadow` (the def is in the same unit), so it never
//! exercises the runtime dispatch path. The bug these guard against was
//! zshrs-original opcode builtins (`doctor`, `async`, `peach`, …) whose
//! `CallBuiltin` handlers ran the builtin without probing the function
//! table first — so `doctor() { … }` on one line then `doctor` on the
//! next silently ran the builtin, violating `function > builtin`. The
//! coreutils shadows (`cat`, `sort`) already probed; the extension
//! builtins and the `reg_passthru!` family did not.
//!
//! Faithful reproduction needs SEPARATE compile units, so these tests
//! drive the interactive shell (`--zsh -f -i`) over piped stdin — one
//! statement per line — exactly how a real session hits the runtime path.

#![allow(non_snake_case)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Run `script` line-by-line through an interactive shell over piped
/// stdin, returning captured stdout (prompts go to stderr, discarded).
/// A trailing `exit` is appended so the shell drains and exits cleanly.
fn run_interactive(prog: &str, args: &[&str], script: &str) -> String {
    let mut child = Command::new(prog)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn {prog}: {e}"));
    {
        let mut sin = child.stdin.take().expect("stdin");
        sin.write_all(script.as_bytes()).expect("write stdin");
        sin.write_all(b"\nexit\n").expect("write exit");
    } // drop closes stdin → shell sees EOF after `exit`
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn zshrs_i(script: &str) -> String {
    run_interactive(
        zshrs_bin().to_str().unwrap(),
        &["--zsh", "-f", "-i"],
        script,
    )
}
fn zsh_i(script: &str) -> String {
    run_interactive(zsh_path(), &["-f", "-i"], script)
}

// ── function > builtin: zshrs-original opcode builtins ──
//
// No zsh counterpart (these builtins don't exist in zsh), so pin the
// zshrs behavior directly: the same-named function must win.
#[test]
fn function_shadows_zshrs_original_builtins() {
    for name in [
        "doctor",
        "async",
        "await",
        "peach",
        "pmap",
        "pgrep",
        "barrier",
        "intercept",
        "profile",
        "dbview",
        "cdreplay",
        "zsleep",
    ] {
        let script = format!("{name}() {{ echo FN-{name} }}\n{name}");
        let out = zshrs_i(&script);
        assert!(
            out.contains(&format!("FN-{name}")),
            "function did not shadow zshrs-original builtin `{name}` \
             (function > builtin violated); stdout: {out:?}"
        );
    }
}

// ── function > builtin: real zsh builtins (parity) ──
//
// Non-recursive bodies (the wrapped builtin is a different command) so the
// call doesn't hit FUNCNEST. zshrs must match zsh exactly.
#[test]
fn function_shadows_real_zsh_builtins_parity() {
    if !zsh_available() {
        return;
    }
    // (builtin-name, function-body). Cover a coreutils shadow (`cat`), a
    // sortable one (`sort`), `reg_passthru!` zsh builtins (`zstyle`,
    // `bindkey`), and `echo` (via `print` to avoid self-recursion).
    // Assignment/keyword builtins (`export`, `typeset`, `local`,
    // `readonly`, `declare`) are parsed specially by zsh — they are not
    // ordinary function-shadowable commands, so they're out of scope here.
    for (name, body) in [
        ("cat", "echo FN"),
        ("sort", "echo FN"),
        ("zstyle", "echo FN"),
        ("bindkey", "echo FN"),
        ("echo", "print FN"),
    ] {
        let script = format!("{name}() {{ {body} }}\n{name}");
        let z = zsh_i(&script);
        let r = zshrs_i(&script);
        assert_eq!(
            z.trim_end(),
            r.trim_end(),
            "shadow parity mismatch for `{name}`:\n--- zsh ---\n{z:?}\n--- zshrs ---\n{r:?}"
        );
    }
}

// ── alias > function ──
//
// Function defined first, then an alias of the same name; the alias (a
// lexer-time substitution) must win when the name is later invoked.
#[test]
fn alias_beats_function_parity() {
    if !zsh_available() {
        return;
    }
    let script = "foo() { echo FN-foo }\nalias foo='echo ALIAS-foo'\nfoo";
    let z = zsh_i(script);
    let r = zshrs_i(script);
    assert_eq!(
        z.trim_end(),
        r.trim_end(),
        "alias>function: zsh={z:?} zshrs={r:?}"
    );
    assert!(
        r.contains("ALIAS-foo"),
        "alias did not win over function: {r:?}"
    );
}

// Note on the `reserved word > function` tier: reserved words (`if`,
// `while`, `for`, `case`, …) are recognized at parse time and cannot be
// shadowed at all — both zsh and zshrs reject `while() { … }` as a syntax
// error, so there is nothing to pin via a shadowing function. That tier is
// covered by construction (every loop/conditional test in the suite relies
// on reserved words winning over any same-named command lookup).

// ── `builtin NAME` forces the builtin (bypasses the shadowing function) ──
//
// The classic self-wrapping idiom: `cd() { builtin cd "$@"; … }`. Without
// the forced-builtin bypass this recurses infinitely. `builtin cd` must
// reach the real builtin, so the wrapper runs exactly once.
#[test]
fn builtin_prefix_bypasses_function_shadow() {
    let script = "cd() { builtin cd \"$@\"; echo WRAPPED }\ncd /tmp\npwd";
    let out = zshrs_i(script);
    assert!(
        out.contains("WRAPPED"),
        "wrapper did not run (builtin cd failed): {out:?}"
    );
    assert_eq!(
        out.matches("WRAPPED").count(),
        1,
        "wrapper ran more than once (forced builtin recursed): {out:?}"
    );
    assert!(out.contains("tmp"), "cd did not change directory: {out:?}");
}

// ── builtin runs when no function shadows it ──
//
// Bare `doctor` with no user function must reach the zshrs-original
// builtin (the fix must not make the builtin unreachable).
#[test]
fn bare_zshrs_builtin_still_reachable() {
    let out = zshrs_i("doctor");
    assert!(
        out.contains("zshrs doctor"),
        "bare zshrs-original builtin unreachable: {out:?}"
    );
}
