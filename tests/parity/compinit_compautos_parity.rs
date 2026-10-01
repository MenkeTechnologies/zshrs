//! `_compautos` after `compinit -C -d DUMP`.
//!
//! compinit sh:524 records an `#autoload` file in `_compautos` only when its
//! header carries options (`[[ "$_i_line" != \ # ]]`). With `-C` and an
//! existing dump zsh defines the table from the dump alone (sh:493-501).
//! zshrs also overlays the registrations of its bundled `~/.zshrs/functions`
//! tree onto the dump, and that overlay recorded every BARE `#autoload`
//! file: `$#_compautos` was 178 where zsh, sourcing the same dump, has 1. The
//! extra keys surfaced in every parameter listing (`unset <TAB>`, `${<TAB>`).
//!
//! The dump is written by zsh itself (a zshrs-written dump is not a valid
//! reference). Skips when zsh or its Completion tree is unavailable.

use crate::zpty_probe::{zsh_path, zshrs_bin};
use std::path::PathBuf;
use std::process::Command;

/// zsh's own Completion tree: every `Completion/*` and `Completion/*/*` dir.
fn zsh_completion_fpath() -> Option<String> {
    let root = std::env::var("ZTST_ZSH_SOURCE")
        .map(PathBuf::from)
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| PathBuf::from(h).join("forkedRepos/zsh")))
        .map(|p| p.join("Completion"))
        .filter(|p| p.is_dir())?;
    let mut dirs = vec![root.clone()];
    for top in std::fs::read_dir(&root).ok()? {
        let top = top.ok()?.path();
        if !top.is_dir() {
            continue;
        }
        dirs.push(top.clone());
        for sub in std::fs::read_dir(&top).ok()? {
            let sub = sub.ok()?.path();
            if sub.is_dir() {
                dirs.push(sub);
            }
        }
    }
    Some(dirs.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" "))
}

fn run(shell: &std::path::Path, script: &str) -> Option<String> {
    let o = Command::new(shell).args(["-f", "-c", script]).output().ok()?;
    Some(String::from_utf8_lossy(&o.stdout).into_owned())
}

#[test]
fn compinit_c_with_a_dump_records_only_autoload_files_with_options() {
    let Some(fpath) = zsh_completion_fpath() else {
        eprintln!("skip: no zsh Completion tree");
        return;
    };
    let zsh = PathBuf::from(zsh_path());
    let dump = std::env::temp_dir().join(format!("zshrs-compautos-dump-{}", std::process::id()));
    let write = format!("fpath=({fpath}); autoload -U compinit; compinit -u -d {}", dump.display());
    if run(&zsh, &write).is_none() || !dump.is_file() {
        eprintln!("skip: zsh did not write a dump");
        return;
    }
    let probe = format!(
        "fpath=({fpath}); autoload -U compinit; compinit -C -d {}; print -r -- $#_compautos ${{(ok)_compautos}}",
        dump.display()
    );
    let z = run(&zsh, &probe);
    let r = run(&zshrs_bin(), &probe);
    let _ = std::fs::remove_file(&dump);
    assert_eq!(z, r, "_compautos after compinit -C -d DUMP");
}
