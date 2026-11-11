//! `zshrs --fish` against the reference fish.
//!
//! Every `tests/data/fish_parity/*.fish` script is run by both shells with startup
//! files off (`fish --no-config`, `zshrs --fish -f`) in an empty scratch directory
//! under an empty `$HOME`; stdout and the exit status must match. Scripts are
//! measured on fish first and left out when fish itself rejects them. Skipped where
//! `fish` is not installed.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fish() -> Option<PathBuf> {
    ["/opt/homebrew/bin/fish", "/usr/local/bin/fish", "/usr/bin/fish", "/bin/fish"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.exists())
}

struct Run {
    stdout: String,
    status: Option<i32>,
}

fn run(bin: &Path, args: &[&str], script: &Path, home: &Path, cwd: &Path) -> Run {
    let out = Command::new(bin)
        .args(args)
        .arg(script)
        .env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .expect("spawn shell");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        status: out.status.code(),
    }
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("zshrs-fish-parity-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn fish_scripts_match_fish() {
    let Some(reference) = fish() else { return };
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/fish_parity");
    let mut scripts: Vec<PathBuf> = std::fs::read_dir(&corpus)
        .expect("corpus directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fish"))
        .collect();
    scripts.sort();
    assert!(!scripts.is_empty(), "no fish scripts in {}", corpus.display());

    let home = scratch("home");
    let cwd = scratch("cwd");
    let mut bad = Vec::new();
    for script in &scripts {
        let want = run(&reference, &["--no-config"], script, &home, &cwd);
        let got = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"], script, &home, &cwd);
        if want.stdout != got.stdout || want.status != got.status {
            bad.push(format!(
                "{}\n  fish : {:?} {:?}\n  zshrs: {:?} {:?}",
                script.file_name().unwrap().to_string_lossy(),
                want.status,
                want.stdout,
                got.status,
                got.stdout
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&cwd);
    assert!(bad.is_empty(), "fish divergences:\n{}", bad.join("\n"));
}

/// Startup files: `conf.d/*.fish` in name order, then `config.fish`, for a `-c` string.
#[test]
fn startup_files_are_read_in_fish_order() {
    let home = scratch("rc");
    let dir = home.join(".config/fish");
    std::fs::create_dir_all(dir.join("conf.d")).unwrap();
    std::fs::write(dir.join("conf.d/b.fish"), "set -g ORDER $ORDER b\n").unwrap();
    std::fs::write(dir.join("conf.d/a.fish"), "set -g ORDER $ORDER a\n").unwrap();
    std::fs::write(
        dir.join("config.fish"),
        "set -g ORDER $ORDER config\nfunction from_config\n    echo $ORDER\nend\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-c", "from_config"])
        .env("HOME", &home)
        .env_remove("XDG_CONFIG_HOME")
        .stdin(Stdio::null())
        .output()
        .expect("spawn zshrs");
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a b config\n");
}

/// A syntax error anywhere stops the script before its first command, status 127.
#[test]
fn a_syntax_error_runs_nothing() {
    let home = scratch("syntax");
    let out = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-f", "-c", "echo before\nif true\necho x"])
        .env("HOME", &home)
        .stdin(Stdio::null())
        .output()
        .expect("spawn zshrs");
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(out.status.code(), Some(127));
}

/// Script text on stdin is translated like a file, a block spanning several lines included.
#[test]
fn stdin_input_is_translated() {
    use std::io::Write;
    let home = scratch("stdin");
    let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-f"])
        .env("HOME", &home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"set x 1 2\nif test (count $x) -eq 2\n    echo two\nend\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&home);
    assert_eq!(String::from_utf8_lossy(&out.stdout), "two\n");
}

/// Every function and completion script fish itself ships must parse and translate
/// (the files are read out of the installed fish with `status get-file`). A construct the
/// translator rejects here is a gap in the grammar, not in the script.
#[test]
fn fish_own_scripts_translate() {
    let Some(reference) = fish() else { return };
    let listing = Command::new(&reference)
        .args(["--no-config", "-c", "status list-files"])
        .output()
        .expect("spawn fish");
    let names: Vec<String> = String::from_utf8_lossy(&listing.stdout)
        .lines()
        .filter(|l| (l.starts_with("functions/") || l.starts_with("completions/")) && l.ends_with(".fish"))
        .map(str::to_string)
        .collect();
    if names.is_empty() {
        return; // a fish built without embedded files
    }
    let mut bad = Vec::new();
    for name in &names {
        let out = Command::new(&reference)
            .args(["--no-config", "-c", &format!("status get-file {name}")])
            .output()
            .expect("spawn fish");
        let src = String::from_utf8_lossy(&out.stdout);
        if let Err(e) = zsh::fish::translate(&src) {
            bad.push(format!("{name}: {e}"));
        }
    }
    assert!(bad.is_empty(), "{} of {} fish scripts do not translate:\n{}", bad.len(), names.len(), bad.join("\n"));
}

/// `complete` definitions answer `complete -C` as fish answers it: options with
/// descriptions, an option that takes the next word, `--opt=` arguments,
/// conditions that read `commandline`, `--wraps`, a definition autoloaded from
/// `~/.config/fish/completions/<cmd>.fish`, and fish's own scripts (string, set_color)
/// read out of the installed fish. The scratch directory is empty, so file
/// completions contribute nothing on either side.
#[test]
fn completions_match_fish() {
    let Some(reference) = fish() else { return };
    let home = scratch("complete");
    let cwd = scratch("complete-cwd");
    let comp_dir = home.join(".config/fish/completions");
    std::fs::create_dir_all(&comp_dir).unwrap();
    std::fs::write(comp_dir.join("zqauto.fish"), "complete -c zqauto -f -l from-file -d 'Autoloaded'\n").unwrap();
    let script = home.join("complete.fish");
    std::fs::write(
        &script,
        r#"function zq; end
function zqauto; end
function zw; end
complete -c zq -f -s v -l verbose -d 'Be loud'
complete -c zq -f -l color -xa 'auto never always'
complete -c zq -f -n 'test (count (commandline -xpc)) -eq 1' -a 'first second' -d 'Subcommand'
complete -c zq -f -n 'contains -- first (commandline -xpc)' -s q -l quiet -d 'Only after first'
complete -c zw -w zq
complete -C 'zq --'
complete -C 'zq --color '
complete -C 'zq --color=a'
complete -C 'zq '
complete -C 'zq first -'
complete -C 'zq -v'
complete -C 'zw f'
complete -C 'string re'
complete -C 'set_color br'
"#,
    )
    .unwrap();
    let want = run(&reference, &["--no-config"], &script, &home, &cwd);
    let got = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish", "-f"], &script, &home, &cwd);
    // with configuration on, `<cmd>.fish` autoloads from the user's completions dir
    let auto = home.join("auto.fish");
    std::fs::write(&auto, "function zqauto; end\ncomplete -C 'zqauto --'\ncomplete -C 'zqauto --fr'\n").unwrap();
    let want_auto = run(&reference, &[], &auto, &home, &cwd);
    let got_auto = run(Path::new(env!("CARGO_BIN_EXE_zshrs")), &["--fish"], &auto, &home, &cwd);
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&cwd);
    assert!(!want.stdout.is_empty(), "fish produced nothing");
    assert_eq!(got.stdout, want.stdout);
    assert_eq!(want_auto.stdout, "--from-file\tAutoloaded\n--from-file\tAutoloaded\n");
    assert_eq!(got_auto.stdout, want_auto.stdout);
}

/// The fish preamble runs at startup, ahead of the first prompt line: the first
/// line no longer carries it through history expansion (its `!` produced
/// "no such word in event"), and the prompt is fish's from the start.
#[test]
fn preamble_is_installed_before_the_first_line() {
    use std::io::Write;
    let home = scratch("preamble");
    let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
        .args(["--fish", "-f", "-i"])
        .env("HOME", &home)
        .env("ZSHRS_HISTORY", "0")
        .env("ZSHRS_NATIVE_ZLE_FX", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn zshrs");
    child.stdin.take().unwrap().write_all(b"echo first\n").unwrap();
    let out = child.wait_with_output().unwrap();
    let _ = std::fs::remove_dir_all(&home);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(String::from_utf8_lossy(&out.stdout).contains("first"), "stdout: {:?}", out.stdout);
    assert!(!err.contains("event") && !err.contains("setopt"), "stderr: {err}");
}

/// `return` / `exit` inside a redirected group in `$( … )` leave its redirect
/// scope open; the substitution must still see end of file (it hung, holding a
/// copy of its own capture pipe). zsh: `[] 1`, `[] 3`.
#[test]
fn command_substitution_ends_when_the_body_returns_from_a_redirected_group() {
    for (body, want) in [
        ("{ return 1 } >/dev/null 2>&1", "[] 1\n"),
        ("{ exit 3 } >/dev/null; print x", "[] 3\n"),
    ] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_zshrs"))
            .args(["-f", "-c", &format!("x=$( {body} ); echo \"[$x] $?\"")])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .spawn()
            .expect("spawn zshrs");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while child.try_wait().unwrap().is_none() {
            if std::time::Instant::now() > deadline {
                let _ = child.kill();
                panic!("`{body}` in $( ) never returned");
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let out = child.wait_with_output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), want, "{body}");
    }
}
