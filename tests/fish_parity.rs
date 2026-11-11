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
