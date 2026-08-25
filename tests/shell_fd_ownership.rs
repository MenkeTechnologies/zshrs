//! The descriptors the shell holds for itself are not the script's.
//!
//! zshrs keeps something zsh never keeps: long-lived READ-WRITE handles
//! to SQLite databases (history, plugins, compsys) and an append handle
//! to its log. They live above fd 9 because `crate::lowfd` puts them
//! there, but "above 9" is not out of reach — `print -u 11`,
//! `read -u 11` and `exec 3>&11` all name a descriptor directly.
//!
//! Before the fdtable registration landed, every one of those routes
//! reached the history database: `print -u 11` overwrote its SQLite
//! header and `read -u 11` returned the string `SQLite format 3` into a
//! shell variable. zsh 5.9.2 answers status 1 to all of them, because a
//! descriptor the shell owns is either absent or classified
//! (c:Src/exec.c:3884-3897, c:Src/exec.c:3830-3835).

use std::path::PathBuf;
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

/// Run `script` in a shell whose entire state lives in `home`.
fn run(home: &std::path::Path, script: &str) -> (String, String) {
    let out = Command::new(zshrs_bin())
        .args(["-f", "-c", script])
        .env("ZSHRS_HOME", home)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("spawn zshrs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Every route a script has to a descriptor is refused for the ones the
/// shell owns, and the history database is byte-identical afterwards.
#[test]
fn user_code_cannot_reach_the_shells_own_descriptors() {
    let home = tempfile::tempdir().expect("tempdir");
    // First run creates the log and the history database.
    run(home.path(), "print seed");
    let db = home.path().join("zshrs_history.db");
    let before = std::fs::read(&db).expect("history db must exist after a run");

    // Attack every descriptor the shell could plausibly hold, by every
    // route that names one.
    let (stdout, _) = run(
        home.path(),
        r#"for fd in 10 11 12 13 14; do
             print -u $fd "ATTACK-PRINT-$fd" 2>/dev/null && print "WROTE-print-$fd"
             read -u $fd line 2>/dev/null && print "LEAKED-read-$fd:$line"
             eval "exec 9>&$fd" 2>/dev/null && print "DUPED-$fd"
             eval "print ATTACK-REDIR-$fd >&$fd" 2>/dev/null && print "WROTE-redir-$fd"
           done
           print SWEEP-DONE"#,
    );
    assert!(
        stdout.contains("SWEEP-DONE"),
        "the probe script must run to completion; got `{stdout}`"
    );
    for marker in ["WROTE-print-", "LEAKED-read-", "DUPED-", "WROTE-redir-"] {
        assert!(
            !stdout.contains(marker),
            "a script reached a descriptor the shell owns ({marker}); got `{stdout}`"
        );
    }

    // The database must be untouched by the attack. The shell does append
    // the probe to its own history through its own API, so compare the
    // prefix that existed before rather than the whole file: a write
    // through a raw descriptor lands at offset 0 and would corrupt the
    // header, which is exactly what happened before this was fixed.
    let after = std::fs::read(&db).expect("history db must still exist");
    assert_eq!(
        &after[..16],
        &before[..16],
        "the SQLite header must be intact; a raw write through fd 11 lands here"
    );
    assert!(
        after.starts_with(b"SQLite format 3\0"),
        "history db no longer has a SQLite header"
    );
    // NOT asserted: the absence of the string "ATTACK-" in the file. The
    // shell records the probe COMMAND in history through its own API, so
    // the payload legitimately appears there as recorded command text.
    // What distinguishes a raw write is WHERE it lands — offset 0,
    // through a descriptor nobody handed the script — which the header
    // check above catches, and whether any route reported success, which
    // the marker checks above catch.
    assert!(
        after.len() >= before.len(),
        "the history database must not have been truncated"
    );
}

/// The script's OWN descriptors keep working — the guard above must not
/// have been bought by breaking ordinary redirection.
#[test]
fn the_scripts_own_descriptors_still_work() {
    let home = tempfile::tempdir().expect("tempdir");
    let (stdout, stderr) = run(
        home.path(),
        r#"exec 3>&1
           print -u 3 viafd3
           exec {v}>/dev/null
           print -u $v tovar
           exec 4>&$v
           print -u 1 one
           print -u 2 two
           print done"#,
    );
    assert!(
        stdout.contains("viafd3") && stdout.contains("one") && stdout.contains("done"),
        "script-owned descriptors must keep working; stdout=`{stdout}` stderr=`{stderr}`"
    );
    assert!(
        stderr.contains("two"),
        "fd 2 must still be stderr; stderr=`{stderr}`"
    );
}
