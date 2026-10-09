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

/// `$ZSH_VERSION` of `bin`, or `None` when it does not run.
fn version_of(bin: &str, args: &[&str]) -> Option<String> {
    std::process::Command::new(bin)
        .args(args)
        .args(["-f", "-c", "print -rn -- $ZSH_VERSION"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The release zsh zshrs reports as its own version, when one is installed.
///
/// [`zsh_path`] is the development tree, and zshrs is not that version: it
/// reports the release it targets (`$ZSH_VERSION`, `$ZSH_PATCHLEVEL`), pins the
/// builtins whose semantics moved on master (`zparseopts`, `zformat`) to that
/// release, and writes `.zwc` files whose header carries it — a shell refuses a
/// dump written by another version (`check_dump_file`, Src/parse.c). The tests
/// that pin one of those compare against this shell instead. Resolution:
///
/// 1. `$ZSHRS_RELEASE_ZSH` — an explicit binary (CI exports the cached build).
/// 2. `~/.cache/zshrs/zsh-5.9.2/bin/zsh` — `ZSHRS_ORACLE_PREFIX=… scripts/build_zsh_oracle.sh`
///    from the `zsh-5.9.2` tag.
/// 3. A system zsh.
///
/// Only a candidate that reports exactly zshrs's `$ZSH_VERSION` is returned, so
/// a machine without that release skips these tests rather than comparing
/// against a shell that cannot agree by construction.
pub fn release_zsh_path() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let zshrs = version_of(env!("CARGO_BIN_EXE_zshrs"), &["--zsh"])?;
        let mut candidates: Vec<String> = Vec::new();
        if let Ok(p) = std::env::var("ZSHRS_RELEASE_ZSH") {
            candidates.push(p);
        }
        if let Some(home) = std::env::var_os("HOME") {
            let built: PathBuf = [home.as_os_str(), ".cache/zshrs/zsh-5.9.2/bin/zsh".as_ref()]
                .iter()
                .collect();
            candidates.push(built.to_string_lossy().into_owned());
        }
        candidates.extend(
            ["/opt/homebrew/bin/zsh", "/usr/local/bin/zsh", "/bin/zsh", "/usr/bin/zsh"]
                .map(String::from),
        );
        candidates
            .into_iter()
            .filter(|p| Path::new(p).exists())
            .find(|p| version_of(p, &[]).as_deref() == Some(zshrs.as_str()))
    })
    .as_deref()
}
