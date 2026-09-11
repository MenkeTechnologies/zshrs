//! zsh's man and info pages, shipped inside the binary.
//!
//! **zshrs-original infrastructure — no C source counterpart.** zsh's
//! `make install` drops `zsh.1`, `zshall.1`, `zshbuiltins.1` … under
//! `<prefix>/share/man/man1` and `zsh.info*` under `<prefix>/share/info`,
//! and `man`/`info` find them because that prefix is already on the
//! system's search path.
//!
//! zshrs is a drop-in binary that is not installed under a zsh prefix, so
//! `man zshall` and `info zsh` worked only on a host that happened to have
//! zsh installed — while the pages document the language zshrs itself
//! implements. `build.rs` packs `vendor/zsh-doc` into the binary and this
//! module writes it to `~/.zshrs/{man,info}` on first run, then puts those
//! directories on `MANPATH` / `INFOPATH`.
//!
//! Both variables are extended, never replaced: an empty trailing entry is
//! how `man` and `info` are told "and then your usual search path", so a
//! shell that had no `MANPATH` still finds every other page on the system.
//!
//! Errors are swallowed on purpose, as in [`crate::bundled_functions`]: a
//! read-only or full `$HOME` must not stop the shell from starting.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The packed tree: `u32 name_len | name | u32 body_len | body`, repeated,
/// little-endian, zstd-compressed. Names keep their `man1/` or `info/`
/// prefix. Written by `bundle_zsh_docs` in build.rs.
static BUNDLE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/zsh_docs.zst"));

include!(concat!(env!("OUT_DIR"), "/zsh_docs_id.rs"));

/// Written into the directory so a zshrs carrying different pages
/// refreshes them instead of leaving a stale set from an older build.
const STAMP: &str = ".zshrs-docs-version";

/// What [`STAMP`] holds: crate version plus the bundle's content hash.
/// The version alone is not enough — the pages can change within a
/// version, and then a version-only stamp never triggers a rewrite.
fn stamp_value() -> String {
    format!("{}-{}", env!("CARGO_PKG_VERSION"), DOCS_ID)
}

/// `~/.zshrs`.
fn base_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".zshrs"))
}

/// `~/.zshrs/man` — the `MANPATH` entry, holding `man1/`.
pub fn man_dir() -> Option<PathBuf> {
    Some(base_dir()?.join("man"))
}

/// `~/.zshrs/info` — the `INFOPATH` entry.
pub fn info_dir() -> Option<PathBuf> {
    Some(base_dir()?.join("info"))
}

/// `~/.zshrs/help` — the `HELPDIR` entry, `run-help`'s database.
pub fn help_dir() -> Option<PathBuf> {
    Some(base_dir()?.join("help"))
}

/// True when the tree is absent or holds a different bundle.
fn needs_write(base: &Path) -> bool {
    match std::fs::read_to_string(base.join(STAMP)) {
        Ok(s) => s.trim() != stamp_value(),
        Err(_) => true,
    }
}

/// Materialise the pages when missing or stale. Returns how many files
/// were written; `Some(0)` means the tree was already current.
///
/// The cost on the common path is one `read_to_string` of a short stamp.
pub fn ensure_installed() -> Option<usize> {
    let base = base_dir()?;
    if !needs_write(&base) {
        return Some(0);
    }
    let raw = zstd::decode_all(BUNDLE).ok()?;
    let mut n = 0usize;
    let mut i = 0usize;
    while i + 4 <= raw.len() {
        let nl = u32::from_le_bytes(raw[i..i + 4].try_into().ok()?) as usize;
        i += 4;
        if i + nl + 4 > raw.len() {
            break;
        }
        let name = String::from_utf8_lossy(&raw[i..i + nl]).into_owned();
        i += nl;
        let bl = u32::from_le_bytes(raw[i..i + 4].try_into().ok()?) as usize;
        i += 4;
        if i + bl > raw.len() {
            break;
        }
        let body = &raw[i..i + bl];
        i += bl;
        // The bundle is written by our own build.rs, but a relative name
        // is still the one field that could escape the directory.
        if name.is_empty() || name.starts_with('/') || name.contains("..") {
            continue;
        }
        let dest = base.join(&name);
        let Some(parent) = dest.parent() else { continue };
        if std::fs::create_dir_all(parent).is_err() {
            continue;
        }
        if std::fs::write(&dest, body).is_ok() {
            n += 1;
        }
    }
    if let Ok(mut f) = std::fs::File::create(base.join(STAMP)) {
        let _ = f.write_all(stamp_value().as_bytes());
    }
    tracing::info!(target: "bundled_docs", written = n, dir = %base.display(),
                   "materialised bundled zsh man/info pages");
    Some(n)
}

/// Prepend `dir` to a colon-separated search variable, keeping whatever
/// the user already had and leaving an empty trailing entry when the
/// variable was unset.
///
/// The empty entry matters: `man` and `info` read `a::b` / a trailing `:`
/// as "splice the built-in default search path in here". Setting a bare
/// `MANPATH=~/.zshrs/man` would instead REPLACE the system's man path and
/// hide every other page on the machine.
fn prepend_search_path(var: &str, dir: &Path) {
    let dir = dir.to_string_lossy().into_owned();
    let next = match std::env::var(var) {
        Ok(cur) if !cur.is_empty() => {
            if cur.split(':').any(|e| e == dir) {
                return;
            }
            format!("{dir}:{cur}")
        }
        // Trailing colon == "then the usual default path".
        _ => format!("{dir}:"),
    };
    unsafe { std::env::set_var(var, next) };
}

/// The search-path directories to publish, once materialised. `HELPDIR`
/// is not among them: it is a plain scalar naming ONE directory, not a
/// colon list, and it is a shell parameter rather than an environment
/// entry, so it is set outright by [`seed_helpdir_param`].
fn published_dirs() -> Vec<(&'static str, PathBuf)> {
    let mut out = Vec::new();
    if let Some(d) = man_dir() {
        if d.is_dir() {
            out.push(("MANPATH", d));
        }
    }
    if let Some(d) = info_dir() {
        if d.is_dir() {
            out.push(("INFOPATH", d));
        }
    }
    out
}

/// `HELPDIR`, if the user has not chosen one.
///
/// zsh does not export `HELPDIR`; `run-help` falls back to a default
/// baked in at build time by whichever zsh compiled it — the vendored copy
/// carries `/opt/homebrew/Cellar/zsh/5.9.2/share/zsh/help`, a path that
/// does not exist on a host without that exact install. Pointing it at the
/// bundled tree is what makes `run-help` work with no zsh on the machine.
///
/// A user-set value always wins: this only fills an empty slot.
fn helpdir_value() -> Option<String> {
    let d = help_dir()?;
    d.is_dir().then(|| d.to_string_lossy().into_owned())
}

/// Point `$HELPDIR` at the bundled help tree as a PLAIN SHELL PARAMETER.
///
/// `run-help` is a shell function — `local HELPDIR="${HELPDIR:-<compiled
/// default>}"` at `vendor/zsh/functions/run-help:13` — so it reads the
/// shell's own parameter table and never the process environment. Putting
/// `HELPDIR` in the environment therefore bought the feature nothing and
/// leaked the name into EVERY child process: with `$HOME` set, a child of
/// zshrs saw `HELPDIR`, `MANPATH` and `INFOPATH` where a child of zsh saw
/// only `MANPATH`, and `env rm <TAB>` (which lists exported names via
/// `_parameters -g "*export*"`) offered a name zsh does not have.
/// `MANPATH` / `INFOPATH` stay exported because `man` and `info` are
/// separate PROCESSES and read them from their environment.
///
/// Must be called AFTER the environment-import loop has built paramtab:
/// c:Src/params.c:893 `createparamtable` imports `environ` exactly as it
/// was at process entry, so a user's exported `HELPDIR` is already in the
/// table by then and the `getsparam` guard below leaves it alone —
/// value and `PM_EXPORTED` both intact, since it came in exported.
pub fn seed_helpdir_param() {
    if crate::ported::params::getsparam("HELPDIR").is_some_and(|v| !v.is_empty()) {
        return; // a user-set value always wins; this only fills an empty slot
    }
    if let Some(v) = helpdir_value() {
        crate::ported::params::setsparam("HELPDIR", &v);
    }
}

/// Materialise the pages and put them on `MANPATH` / `INFOPATH` in the OS
/// environment, so any child process (`man`, `info`) inherits them.
///
/// Called from `ShellExecutor::new`. This alone does NOT make the shell's
/// own `$MANPATH` / `$INFOPATH` show the new value when the variable was
/// already in the inherited environment — paramtab is built from the
/// process-entry `environ` snapshot taken in `main`, which a later
/// `setenv` cannot reach. The binary entry calls [`publish_into`] on that
/// snapshot for the shell-visible half.
///
/// `HELPDIR` is not published here, or anywhere else in the environment:
/// it is a shell parameter only, seeded by [`seed_helpdir_param`] once
/// paramtab exists.
pub fn install_and_publish() {
    let _ = ensure_installed();
    for (var, dir) in published_dirs() {
        prepend_search_path(var, &dir);
    }
}

/// Materialise the pages and publish them into a process-entry `environ`
/// snapshot before it is frozen.
///
/// c:Src/params.c:893 — `createparamtable` imports `environ` exactly as it
/// was at process entry, and zshrs preserves that by snapshotting `envp`
/// in `main`. An `$INFOPATH` inherited from the parent therefore keeps its
/// old value in the shell no matter what the shell `setenv`s afterwards;
/// `$MANPATH` only appeared to work because it is a PM_TIED colonarray
/// re-derived from the live environment later. Editing the snapshot fixes
/// both, and keeps the process environment in step for child commands.
pub fn publish_into(env: &mut Vec<(String, String)>) {
    let _ = ensure_installed();
    for (var, dir) in published_dirs() {
        let dir = dir.to_string_lossy().into_owned();
        match env.iter_mut().find(|(k, _)| k == var) {
            Some((_, v)) if !v.is_empty() => {
                if !v.split(':').any(|e| e == dir) {
                    *v = format!("{dir}:{v}");
                }
            }
            Some((_, v)) => *v = format!("{dir}:"),
            None => env.push((var.to_string(), format!("{dir}:"))),
        }
        // Keep the live environment in step so `man` / `info` launched as
        // children see the same path the shell reports.
        let val = env
            .iter()
            .find(|(k, _)| k == var)
            .map(|(_, v)| v.clone())
            .unwrap_or_default();
        unsafe { std::env::set_var(var, val) };
    }
    // `HELPDIR` is deliberately absent: it is a shell parameter, not an
    // environment entry — see [`seed_helpdir_param`].
}
