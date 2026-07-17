//! A plugin's optional `znative.toml`, declaring how the plugin is loaded.
//!
//! Ported/adapted from strykelang's `pkg/manifest.rs` (`[package]`+`[ffi]`),
//! trimmed to the two zshrs plugin kinds. When a plugin repo ships no
//! `znative.toml`, [`PluginKind::detect`] infers the kind from the tree, so
//! ordinary oh-my-zsh/zinit `*.plugin.zsh` repos install with no metadata.
//!
//! Schema:
//! ```toml
//! [plugin]
//! name = "git-fuzzy"
//! version = "0.1.0"
//! description = "fzf-driven git UI"
//!
//! # Native (Rust cdylib) plugin — dlopened via `zmodload -R`:
//! [native]
//! lib = "git_fuzzy"          # produces lib<lib>.{dylib,so}
//!
//! # …OR script (zsh) plugin — sourced + added to fpath:
//! [script]
//! source = ["git-fuzzy.plugin.zsh"]   # files to `source`, in order
//! fpath  = ["functions"]              # dirs to prepend to $fpath
//! ```

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::{PkgError, PkgResult};

/// Manifest filename, at the root of a plugin's tree.
pub const MANIFEST_FILE: &str = "znative.toml";

/// Parsed `znative.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginManifest {
    /// `[plugin]` metadata.
    #[serde(default)]
    pub plugin: PluginMeta,
    /// `[native]` — present for Rust cdylib plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativeSpec>,
    /// `[script]` — present for zsh script plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<ScriptSpec>,
}

/// `[plugin]` table.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginMeta {
    /// `name` — defaults to the source basename when absent.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// `version` — defaults to `"0.0.0"` when absent.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// One-line description (shown by `znative list`/`info`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// `[native]` — a Rust cdylib plugin using the `zshrs-plugin` SDK.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NativeSpec {
    /// Library file stem — produces `lib<lib>.{dylib,so}`. When empty the
    /// installer infers it from the built artifact or the package name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lib: String,
    /// When true, run `cargo build --release` in the staged tree before
    /// looking for the cdylib. Defaults to true when a `Cargo.toml` exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<bool>,
}

/// `[script]` — a zsh script plugin.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ScriptSpec {
    /// Files to `source`, in order, relative to the plugin root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source: Vec<String>,
    /// Directories to prepend to `$fpath`, relative to the plugin root.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fpath: Vec<String>,
}

/// Resolved plugin kind — either an explicit `znative.toml` or an inferred layout.
#[derive(Debug, Clone)]
pub enum PluginKind {
    /// Rust cdylib: the `[native]` spec + the built/shipped lib stem.
    Native(NativeSpec),
    /// zsh script: files to source + fpath dirs.
    Script(ScriptSpec),
}

impl PluginManifest {
    /// Parse a `znative.toml` string.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> PkgResult<PluginManifest> {
        toml::from_str::<PluginManifest>(s)
            .map_err(|e| PkgError::Manifest(format!("znative.toml: {}", e.message())))
    }

    /// Load a plugin's `znative.toml` if present at `dir/znative.toml`.
    pub fn load(dir: &Path) -> PkgResult<Option<PluginManifest>> {
        let path = dir.join(MANIFEST_FILE);
        if !path.is_file() {
            return Ok(None);
        }
        let s = std::fs::read_to_string(&path)
            .map_err(|e| PkgError::Io(format!("read {}: {}", path.display(), e)))?;
        Ok(Some(PluginManifest::from_str(&s)?))
    }
}

impl PluginKind {
    /// Determine the plugin kind for a staged tree. Prefers an explicit
    /// `znative.toml` (`[native]` beats `[script]` when both present), then falls
    /// back to layout detection:
    ///
    /// 1. A prebuilt `lib*.{dylib,so}` anywhere, or a `Cargo.toml` whose
    ///    `[lib] crate-type` mentions `cdylib` → [`PluginKind::Native`].
    /// 2. Any `*.plugin.zsh` (source it; add its dir to fpath), or a
    ///    `functions/` dir, or `*.zsh` files → [`PluginKind::Script`].
    ///
    /// Returns [`PkgError::Unknown`] when nothing matches.
    pub fn detect(dir: &Path, manifest: Option<&PluginManifest>) -> PkgResult<PluginKind> {
        if let Some(m) = manifest {
            if let Some(n) = &m.native {
                return Ok(PluginKind::Native(n.clone()));
            }
            if let Some(s) = &m.script {
                return Ok(PluginKind::Script(s.clone()));
            }
        }
        // Layout detection — native first (a repo can carry both a build tree
        // and helper .zsh, but if it declares a cdylib it's a native plugin).
        if has_cdylib(dir) || cargo_is_cdylib(dir) {
            return Ok(PluginKind::Native(NativeSpec::default()));
        }
        let mut source: Vec<String> = Vec::new();
        let mut fpath: Vec<String> = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let name = entry?.file_name().to_string_lossy().into_owned();
            if name.ends_with(".plugin.zsh") {
                source.push(name);
            }
        }
        if dir.join("functions").is_dir() {
            fpath.push("functions".into());
        }
        // Fallback: a lone `<basename>.zsh` or any `.zsh`.
        if source.is_empty() {
            if let Some(base) = dir.file_name().and_then(|s| s.to_str()) {
                let cand = format!("{}.zsh", base);
                if dir.join(&cand).is_file() {
                    source.push(cand);
                }
            }
        }
        if source.is_empty() && fpath.is_empty() {
            for entry in std::fs::read_dir(dir)? {
                let name = entry?.file_name().to_string_lossy().into_owned();
                if name.ends_with(".zsh") {
                    source.push(name);
                }
            }
        }
        if source.is_empty() && fpath.is_empty() {
            return Err(PkgError::Unknown(
                "could not determine plugin kind: no znative.toml, no *.plugin.zsh, \
                 no functions/, and no Rust cdylib/Cargo.toml"
                    .into(),
            ));
        }
        source.sort();
        Ok(PluginKind::Script(ScriptSpec { source, fpath }))
    }
}

/// True if a `lib*.{dylib,so}` exists at the tree root (a prebuilt cdylib).
fn has_cdylib(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    for entry in rd.flatten() {
        let n = entry.file_name();
        let n = n.to_string_lossy();
        if n.starts_with("lib") && (n.ends_with(".dylib") || n.ends_with(".so")) {
            return true;
        }
    }
    false
}

/// True if `Cargo.toml` declares a `cdylib` crate-type (so `cargo build`
/// produces a dlopen-able library).
fn cargo_is_cdylib(dir: &Path) -> bool {
    let cargo = dir.join("Cargo.toml");
    let Ok(s) = std::fs::read_to_string(&cargo) else {
        return false;
    };
    s.contains("cdylib")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_native_manifest() {
        let m = PluginManifest::from_str(
            "[plugin]\nname='x'\nversion='0.1.0'\n[native]\nlib='foo'\n",
        )
        .unwrap();
        assert_eq!(m.plugin.name, "x");
        assert_eq!(m.native.unwrap().lib, "foo");
    }

    #[test]
    fn parses_script_manifest() {
        let m = PluginManifest::from_str(
            "[plugin]\nname='y'\n[script]\nsource=['y.plugin.zsh']\nfpath=['fns']\n",
        )
        .unwrap();
        let s = m.script.unwrap();
        assert_eq!(s.source, vec!["y.plugin.zsh"]);
        assert_eq!(s.fpath, vec!["fns"]);
    }
}
