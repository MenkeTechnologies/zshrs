//! A file that defines aliases is cached, and a replay that diverges from the
//! recorded run on how many aliases it defines finishes the file from source.
//!
//! The commands after an `alias` are lexed with its expansion in force, so a
//! cached program is only right while the alias tables are where the recording
//! left them. Each recorded event carries its position in the alias tables'
//! history (`script_cache::GuardedEvent`).

use std::path::Path;
use std::process::Command;

fn run(home: &Path, args: &[&str], env: &[(&str, &str)]) -> (String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_zshrs"));
    cmd.args(args).env("ZSHRS_HOME", home).env("HOME", home);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("spawn zshrs");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("zshrs-alias-guard-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn a_file_that_defines_aliases_is_stored_and_replays_the_same() {
    let dir = scratch("stored");
    let file = dir.join("defs.zsh");
    std::fs::write(&file, "alias zqa='echo expanded'\nzqa one\nalias zqb='echo second'\nzqb two\n").unwrap();
    let src = format!("source {}", file.display());

    let (first, _) = run(&dir, &["-f", "-c", &src], &[]);
    assert_eq!(first, "expanded one\nsecond two\n");
    // the entry exists and the replay produces the same output
    let (listing, _) = run(&dir, &["-f", "-c", "dbview scripts"], &[]);
    assert!(listing.contains("defs.zsh"), "a file that defines aliases is stored:\n{listing}");
    let (second, _) = run(&dir, &["-f", "-c", &src], &[]);
    assert_eq!(second, first);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_conditional_alias_that_flips_is_relexed_not_replayed_stale() {
    let dir = scratch("flip");
    let file = dir.join("cond.zsh");
    // `zqc` exists only when FLIP is set; the next line uses it.
    std::fs::write(
        &file,
        "if [[ -n $FLIP ]]; then alias zqc='echo aliased'; fi\nzqc hi\necho done\n",
    )
    .unwrap();
    let src = format!("source {}", file.display());

    // Record with the alias defined: `zqc hi` was lexed as `echo aliased hi`.
    let (on, _) = run(&dir, &["-f", "-c", &src], &[("FLIP", "1")]);
    assert_eq!(on, "aliased hi\ndone\n");
    // Replay with it undefined: the line must be lexed again — `zqc` is not a
    // command — not run as the recorded `echo aliased hi`.
    let (off, err) = run(&dir, &["-f", "-c", &src], &[]);
    assert_eq!(off, "done\n", "stale expansion replayed: {off:?}");
    assert!(err.contains("zqc"), "the command is reported missing: {err:?}");

    // And the other way round, against a recording made with it undefined.
    let dir2 = scratch("flip2");
    let (off_first, _) = run(&dir2, &["-f", "-c", &src], &[]);
    assert_eq!(off_first, "done\n");
    let (on_second, _) = run(&dir2, &["-f", "-c", &src], &[("FLIP", "1")]);
    assert_eq!(on_second, "aliased hi\ndone\n", "alias defined on replay: {on_second:?}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn a_script_run_as_a_file_gets_the_same_guard() {
    let dir = scratch("toplevel");
    let file = dir.join("top.zsh");
    std::fs::write(
        &file,
        "if [[ -n $FLIP ]]; then alias zqd='echo top'; fi\nzqd x\necho end\n",
    )
    .unwrap();
    let f = file.to_str().unwrap();
    let (on, _) = run(&dir, &["-f", f], &[("FLIP", "1")]);
    assert_eq!(on, "top x\nend\n");
    let (off, _) = run(&dir, &["-f", f], &[]);
    assert_eq!(off, "end\n", "stale expansion replayed: {off:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_script_run_by_relative_path_that_changes_directory_is_still_stored() {
    let dir = scratch("relcd");
    std::fs::write(dir.join("walk.sh"), "cd /\necho walked\n").unwrap();

    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["-f", "walk.sh"])
        .current_dir(&dir)
        .env("ZSHRS_HOME", &dir)
        .env("HOME", &dir)
        .output()
        .expect("spawn zshrs");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "walked\n");

    let (listing, _) = run(&dir, &["-f", "-c", "dbview scripts"], &[]);
    assert!(listing.contains("walk.sh"), "a script that cd's is stored:\n{listing}");
    let _ = std::fs::remove_dir_all(&dir);
}
