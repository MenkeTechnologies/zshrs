//! compaudit sh:18 — `[[ -n $commands[getent] ]] || getent() { … }`.
//!
//! The first statement of `Completion/compaudit`, which compinit runs on
//! every call but `-C` (compinit sh:456-458), `-u` included: `_i_fail=use`
//! only returns at compaudit sh:86. Two things follow from it in zsh:
//!
//!   * the `$commands[getent]` lookup enables `zsh/parameter`'s
//!     `p:commands` feature, so a later `local -A +h commands` keeps the
//!     special (`association-local-special`);
//!   * with no `getent` on `$PATH` (macOS), `getent` becomes a function.
//!
//! The Rust compaudit port skipped the line, and the `-u` path skipped
//! compaudit entirely: `p:commands` stayed off, `local -A +h commands` came
//! out `association-local`, and `getent` stayed undefined. Found by
//! compsys_spec_fuzz seed 9517 case0039 (`_command_names` sh:70's idiom).
//!
//! Skips when zsh or a stock function directory is unavailable.

use crate::zpty_probe::{zsh_path, zshrs_bin};
use std::path::Path;
use std::process::Command;

fn stock_fpath_exists() -> bool {
    std::fs::read_dir("/usr/share/zsh")
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .any(|e| e.path().join("functions").is_dir())
        })
        .unwrap_or(false)
}

fn run(shell: &Path, script: &str) -> String {
    let o = Command::new(shell)
        .args(["-f", "-c", script])
        .output()
        .expect("invoke shell");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn compinit_u_runs_the_compaudit_getent_shim() {
    if !Path::new(zsh_path()).exists() || !stock_fpath_exists() {
        eprintln!("skip: no zsh or no /usr/share/zsh/*/functions");
        return;
    }
    let probe = "fpath=(/usr/share/zsh/*/functions(N)); autoload -Uz compinit; compinit -u -D
zmodload -Fl zsh/parameter | grep -x '[+-]p:commands'
whence -w getent
f() { local -A +h commands; print -r -- ${(t)commands} }; f";
    let z = run(Path::new(zsh_path()), probe);
    let r = run(&zshrs_bin(), probe);
    assert!(z.contains("+p:commands"), "reference zsh did not run the shim:\n{z}");
    assert_eq!(z, r, "state compaudit sh:18 leaves after compinit -u");
}
