//! Startup parameters a shell must synthesise when the environment does not
//! supply them.
//!
//! A shell launched from cron, a launchd/systemd unit, a container entrypoint
//! or plain `env -i` inherits almost nothing. C zsh covers that by seeding two
//! values during startup:
//!
//!   * `$TMPPREFIX` — `Src/params.c:892`
//!     `setsparam("TMPPREFIX", ztrdup_metafy(DEFAULT_TMPPREFIX));`
//!     with `DEFAULT_TMPPREFIX` = `"/tmp/zsh"` (`configure.ac:3030`).
//!   * `$HOME` — `Src/init.c:1237-1250`, which reads the password database:
//!     `home = ztrdup_metafy(pswd->pw_dir)` when `getpwuid(cached_uid)`
//!     succeeds, else `home = ztrdup("/")`. `Src/params.c:938-943` then clears
//!     `PM_UNSET` and `addenv`s it, so children see it too.
//!
//! Both seeds run BEFORE `createparamtable`'s environment-import loop
//! (`Src/params.c:893-924`), so an inherited value always overrides the
//! default — including an explicitly empty `HOME=`.
//!
//! These are regression tests for a real gap: zshrs left both parameters unset
//! under a scrubbed environment, which made every `~` expansion, rc-file path
//! and cache path downstream of `$HOME` resolve to the empty string (`~/x`
//! expanded to `/x`).

use std::process::Command;

fn zshrs_bin() -> String {
    env!("CARGO_BIN_EXE_zshrs").to_string()
}

/// Run `zshrs -f -c <code>` with a scrubbed environment plus `extra`, and
/// return stdout with the trailing newline removed.
fn scrubbed(extra: &[(&str, &str)], code: &str) -> String {
    let mut cmd = Command::new(zshrs_bin());
    cmd.env_clear()
        .env("TERM", "dumb")
        .env("PATH", "/usr/bin:/bin")
        .arg("-f")
        .arg("-c")
        .arg(code);
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to spawn zshrs");
    assert!(
        out.status.success(),
        "zshrs -f -c {code:?} exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .trim_end_matches('\n')
        .to_string()
}

/// The home directory `Src/init.c:1239-1248` would pick: `getpwuid(getuid())`'s
/// `pw_dir`, or `"/"` when the lookup fails or yields no directory. Computed
/// the same way the shell does rather than hardcoded, so the test is valid on
/// any host (including CI containers whose passwd entry differs).
fn passwd_home() -> String {
    let pswd = unsafe { libc::getpwuid(libc::getuid()) };
    let pw_dir = if pswd.is_null() {
        std::ptr::null()
    } else {
        unsafe { (*pswd).pw_dir }
    };
    if pw_dir.is_null() {
        return "/".to_string();
    }
    unsafe { std::ffi::CStr::from_ptr(pw_dir) }
        .to_string_lossy()
        .into_owned()
}

/// c:Src/params.c:892 — `$TMPPREFIX` defaults to `DEFAULT_TMPPREFIX` when the
/// environment has none. Anything reading it for a temp-file path (the
/// `gettempname` family) depends on it being set, not on each call site
/// carrying its own fallback.
#[test]
fn tmpprefix_defaults_when_absent_from_environment() {
    assert_eq!(
        scrubbed(&[], r#"print -r -- "${TMPPREFIX-UNSET}""#),
        "/tmp/zsh"
    );
}

/// c:Src/params.c:892 vs the import loop at c:893-924 — the seed runs first, so
/// an exported `$TMPPREFIX` must win over the default. This is the direction
/// that breaks if the default is seeded after the import instead of before.
#[test]
fn tmpprefix_from_environment_beats_the_default() {
    assert_eq!(
        scrubbed(
            &[("TMPPREFIX", "/var/tmp/custom-prefix")],
            r#"print -r -- "$TMPPREFIX""#
        ),
        "/var/tmp/custom-prefix"
    );
}

/// c:Src/init.c:1239-1248 — with no `$HOME` in the environment the shell
/// synthesises one from the password database (or `/` when that fails).
#[test]
fn home_synthesised_from_password_database_when_absent() {
    assert_eq!(
        scrubbed(&[], r#"print -r -- "${HOME-UNSET}""#),
        passwd_home()
    );
}

/// c:Src/params.c:893-924 — the import calls `homesetfn`, overwriting the
/// value setupvals synthesised. A `$HOME` that was already in the environment
/// must therefore survive untouched.
#[test]
fn home_from_environment_beats_the_password_database() {
    let out = scrubbed(
        &[("HOME", "/tmp/zshrs-home-parity")],
        r#"print -r -- "$HOME""#,
    );
    assert_eq!(out, "/tmp/zshrs-home-parity");
    assert_ne!(
        out,
        passwd_home(),
        "test is only meaningful when the override differs from the passwd home"
    );
}

/// An explicitly empty `HOME=` was still IN the environment, so C's import loop
/// assigns it and the password-database fallback never runs. Verified against
/// the reference binary: `env -i TERM=dumb PATH=/usr/bin:/bin zsh -f -c
/// 'print -r -- "[${HOME-UNSET}]"'` prints `[]` when `HOME=` is exported.
/// Guards against a fallback keyed on emptiness rather than on presence.
#[test]
fn empty_home_in_environment_stays_empty() {
    assert_eq!(
        scrubbed(&[("HOME", "")], r#"print -r -- "[${HOME-UNSET}]""#),
        "[]"
    );
}

/// The reason the `$HOME` gap mattered: `~` is expanded from it. With no
/// synthesised home, `~/lib` expanded to `/lib` — a path outside the user's
/// tree that silently resolves.
#[test]
fn tilde_expands_to_the_synthesised_home() {
    let home = passwd_home();
    assert_eq!(scrubbed(&[], "print -r -- ~"), home);
    assert_eq!(
        scrubbed(&[], "print -r -- ~/lib"),
        format!("{}/lib", home.trim_end_matches('/'))
    );
}

/// c:Src/params.c:942-943 — `if (!(pm->node.flags & PM_EXPORTED)) addenv(pm,
/// home);`. The synthesised value is exported, so a child process launched from
/// the scrubbed shell still gets a `$HOME`. Without the `addenv` the parameter
/// would look right inside the shell and vanish for everything it runs.
#[test]
fn synthesised_home_is_exported_to_children() {
    let out = scrubbed(&[], "/usr/bin/env");
    let home_line = out
        .lines()
        .find(|l| l.starts_with("HOME="))
        .unwrap_or_else(|| panic!("no HOME in the child environment; got:\n{out}"));
    assert_eq!(home_line, format!("HOME={}", passwd_home()));
}
