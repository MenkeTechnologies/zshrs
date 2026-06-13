//! NO_CLOBBER option + `>|` / `>!` force-clobber redirection parity.

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
fn run_zsh_in(d: &Path, s: &str) -> R {
    let o = Command::new(zsh_path())
        .args(["-fc", s])
        .current_dir(d)
        .output()
        .expect("zsh");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn run_zshrs_in(d: &Path, s: &str) -> R {
    let o = Command::new(zshrs_bin())
        .args(["--zsh", "-f", "-c", s])
        .current_dir(d)
        .env_remove("ZSHRS_CACHE")
        .output()
        .expect("zshrs");
    R {
        stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
        exit: o.status.code().unwrap_or(-1),
    }
}
fn assert_parity_in(d: &Path, s: &str) {
    if !zsh_available() {
        return;
    }
    // Snapshot the dir's file contents BEFORE running so zsh + zshrs
    // each start with the same state. Without this, redirect-append
    // tests (`echo more >> file`) saw the first shell's append
    // persist into the second shell's run, causing false divergence.
    fn snapshot(d: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(d) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Ok(data) = std::fs::read(&path) {
                        out.push((path, data));
                    }
                }
            }
        }
        out
    }
    fn restore(d: &Path, snap: &[(std::path::PathBuf, Vec<u8>)]) {
        // Remove any new files that weren't in the snapshot.
        if let Ok(entries) = std::fs::read_dir(d) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && !snap.iter().any(|(p, _)| p == &path) {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        for (path, data) in snap {
            let _ = std::fs::write(path, data);
        }
    }
    let snap = snapshot(d);
    let z = run_zsh_in(d, s);
    restore(d, &snap);
    let r = run_zshrs_in(d, s);
    assert_eq!(
        z.stdout, r.stdout,
        "stdout divergence on:\n{s}\n--- zsh ---\n{:?}\n--- zshrs ---\n{:?}",
        z.stdout, r.stdout
    );
    assert_eq!(z.exit, r.exit);
}
fn tdir() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

mod default_no_clobber_off {
    use super::*;

    /// Default — no clobber option, `>` overwrites silently.
    #[test]
    fn default_redirect_overwrites_existing() {
        let d = tdir();
        std::fs::write(d.path().join("out.txt"), "old\n").unwrap();
        assert_parity_in(d.path(), "echo new > out.txt; cat out.txt");
    }
}

mod noclobber_on {
    use super::*;

    /// With NO_CLOBBER set, `>` on existing file errors out.
    #[test]
    fn no_clobber_blocks_overwrite() {
        let d = tdir();
        std::fs::write(d.path().join("exists.txt"), "old\n").unwrap();
        assert_parity_in(
            d.path(),
            "setopt no_clobber; echo new > exists.txt 2>/dev/null; echo exit=$?; cat exists.txt",
        );
    }

    /// NO_CLOBBER does NOT block `>` on missing file (still creates).
    #[test]
    fn no_clobber_allows_new_file() {
        let d = tdir();
        assert_parity_in(
            d.path(),
            "setopt no_clobber; echo new > brandnew.txt; cat brandnew.txt",
        );
    }

    /// `>|` force-clobber overrides NO_CLOBBER.
    #[test]
    fn pipe_bar_force_clobber_works() {
        let d = tdir();
        std::fs::write(d.path().join("exists.txt"), "old\n").unwrap();
        assert_parity_in(
            d.path(),
            "setopt no_clobber; echo new >| exists.txt; cat exists.txt",
        );
    }

    /// `>!` zsh alias for `>|`.
    #[test]
    fn bang_force_clobber_works() {
        let d = tdir();
        std::fs::write(d.path().join("exists.txt"), "old\n").unwrap();
        assert_parity_in(
            d.path(),
            "setopt no_clobber; echo new >! exists.txt; cat exists.txt",
        );
    }

    /// `>>` append still works under NO_CLOBBER (file exists).
    #[test]
    fn noclobber_append_existing_works() {
        let d = tdir();
        std::fs::write(d.path().join("exists.txt"), "old\n").unwrap();
        assert_parity_in(
            d.path(),
            "setopt no_clobber; echo more >> exists.txt; cat exists.txt",
        );
    }
}

mod noclobber_off {
    use super::*;

    /// `setopt clobber` (default) → standard overwrite.
    #[test]
    fn clobber_set_overwrites() {
        let d = tdir();
        std::fs::write(d.path().join("out.txt"), "old\n").unwrap();
        assert_parity_in(d.path(), "setopt clobber; echo new > out.txt; cat out.txt");
    }

    /// Toggling no_clobber off mid-script restores `>` behavior.
    #[test]
    fn unset_no_clobber_restores_overwrite() {
        let d = tdir();
        std::fs::write(d.path().join("out.txt"), "old\n").unwrap();
        assert_parity_in(
            d.path(),
            "setopt no_clobber; unsetopt no_clobber; echo new > out.txt; cat out.txt",
        );
    }
}

mod no_clobber_alias {
    use super::*;

    /// `setopt NOCLOBBER` (no underscore) is also accepted.
    #[test]
    fn noclobber_no_underscore_works() {
        let d = tdir();
        std::fs::write(d.path().join("exists.txt"), "old\n").unwrap();
        assert_parity_in(
            d.path(),
            "setopt NOCLOBBER; echo new > exists.txt 2>/dev/null; echo exit=$?",
        );
    }
}
