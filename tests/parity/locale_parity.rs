//! Runtime locale-assignment parity.
//!
//! zsh installs a GSU setfn on `$LANG`, `$LC_ALL` and each `$LC_*`
//! (`Src/params.c:257-262` `lang_gsu`/`lc_all_gsu`/`lc_blah_gsu`, registered
//! at `Src/params.c:332-341`), so assigning one of them at RUNTIME calls
//! `setlocale()` and every later libc-driven decision — `mbrtowc`,
//! `iswprint`, `strcoll` — moves with it.
//!
//! The observable consequence tested here is `${(q)}`: `quotestring`
//! (`Src/utils.c:6422-6442`) passes a character through verbatim when
//! `WC_ISPRINT(cc)` holds and escapes it as `$'\NNN'` otherwise, and whether a
//! UTF-8 sequence decodes as ONE character or three Latin-1 bytes is a
//! property of the C library locale. A shell that never re-runs `setlocale`
//! keeps quoting `日本語` as `346$'\227'245…` after `export LC_ALL=<utf8>`.

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

/// A UTF-8 locale this host actually has, or None.
///
/// `locale -a` is POSIX and present on both macOS and every Linux image that
/// ships locales at all; a host with none (a bare container) makes the whole
/// question unmeasurable, so the tests below skip rather than guess.
fn utf8_locale() -> Option<String> {
    let out = Command::new("locale").arg("-a").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let mut names: Vec<String> = text
        .lines()
        .map(str::trim)
        .filter(|l| {
            let low = l.to_ascii_lowercase();
            low.ends_with("utf-8") || low.ends_with("utf8")
        })
        .map(str::to_string)
        .collect();
    // Prefer en_US so the collation table is the same one the developer sees.
    names.sort_by_key(|n| !n.starts_with("en_US"));
    names.into_iter().next()
}

fn run(bin: &str, args: &[&str], script: &str) -> Vec<u8> {
    let mut cmd = Command::new(bin);
    cmd.args(args).arg(script);
    // Both shells start in a SINGLE-BYTE locale, so the only thing that can
    // move them to UTF-8 is the assignment inside the script.
    cmd.env("LC_ALL", "C").env("LANG", "C");
    cmd.env_remove("ZSHRS_CACHE");
    cmd.output().expect("shell run").stdout
}

fn assert_locale_parity(script: &str) {
    if !zsh_available() {
        return;
    }
    let want = run(zsh_path(), &["-f", "-c"], script);
    let got = run(
        zshrs_bin().to_str().expect("bin path"),
        &["--zsh", "-f", "-c"],
        script,
    );
    assert_eq!(
        String::from_utf8_lossy(&want),
        String::from_utf8_lossy(&got),
        "script: {script}",
    );
}

#[test]
fn lc_all_assignment_reaches_the_c_library_locale() {
    let Some(loc) = utf8_locale() else { return };
    // `q1` warms whatever one-shot locale read the implementation caches
    // BEFORE the assignment, which is the case that regressed: a cold shell
    // reads the environment lazily and happens to be right.
    assert_locale_parity(&format!(
        "a=日本語; q1=${{(q)a}}; export LC_ALL={loc}; print -rn -- ${{(q)a}}"
    ));
}

#[test]
fn lang_assignment_reaches_the_c_library_locale() {
    let Some(loc) = utf8_locale() else { return };
    // `LC_ALL` must go first: `langsetfn` -> `setlang` returns early while a
    // non-empty `$LC_ALL` is set (Src/params.c:4797).
    assert_locale_parity(&format!(
        "a=日本語; q1=${{(q)a}}; unset LC_ALL; export LANG={loc}; print -rn -- ${{(q)a}}"
    ));
}

#[test]
fn lc_ctype_assignment_reaches_the_c_library_locale() {
    let Some(loc) = utf8_locale() else { return };
    assert_locale_parity(&format!(
        "a=日本語; q1=${{(q)a}}; unset LC_ALL; export LC_CTYPE={loc}; print -rn -- ${{(q)a}}"
    ));
}
