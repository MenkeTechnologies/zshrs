//! The reference zsh every parity module compares zshrs against.
//!
//! The reference is the zsh development tree the port follows (the C source
//! its `c:` citations point at, `ZSH_VERSION=5.9.0.3-test`), not a released
//! zsh: where the two disagree, zshrs follows the tree. Resolution order:
//!
//! 1. `$ZSHRS_ORACLE_ZSH` — an explicit binary.
//! 2. `~/.cache/zshrs/zsh-oracle/bin/zsh` — the build `scripts/build_zsh_oracle.sh`
//!    installs from that tree.
//! 3. The system zsh (`/opt/homebrew/bin/zsh`, `/usr/local/bin/zsh`,
//!    `/bin/zsh`, `/usr/bin/zsh`) — a released zsh, so rows where the tree
//!    and the release differ compare against the wrong reference. Kept so a
//!    machine without the build (CI) still runs the suite.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Path of the reference zsh (see the module docs for the order).
pub fn zsh_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        if let Ok(p) = std::env::var("ZSHRS_ORACLE_ZSH") {
            if Path::new(&p).exists() {
                return p;
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            let built: PathBuf = [home.as_os_str(), ".cache/zshrs/zsh-oracle/bin/zsh".as_ref()]
                .iter()
                .collect();
            if built.exists() {
                return built.to_string_lossy().into_owned();
            }
        }
        ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh", "/usr/bin/zsh"]
            .into_iter()
            .find(|p| Path::new(p).exists())
            .unwrap_or("zsh")
            .to_string()
    })
}

/// Whether the reference zsh and zshrs report the same `$ZSH_VERSION`.
///
/// A compiled function file (`.zwc`) carries the version of the shell that wrote
/// it, and a shell refuses a dump written by another version (`check_dump_file`,
/// Src/parse.c). zshrs reports the release it targets and the reference is the
/// development tree, so a dump written by one is not read by the other even
/// though zshrs and a same-version zsh read each other's. Tests that move a
/// `.zwc` between the two shells have nothing to measure unless the versions
/// agree, and skip.
pub fn same_zsh_version() -> bool {
    static SAME: OnceLock<bool> = OnceLock::new();
    *SAME.get_or_init(|| {
        let version = |bin: &str, args: &[&str]| {
            std::process::Command::new(bin)
                .args(args)
                .args(["-f", "-c", "print -rn -- $ZSH_VERSION"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        let reference = version(zsh_path(), &[]);
        let zshrs = version(env!("CARGO_BIN_EXE_zshrs"), &["--zsh"]);
        reference.is_some() && reference == zshrs
    })
}
