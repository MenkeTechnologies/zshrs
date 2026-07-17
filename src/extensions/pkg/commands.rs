//! `zpm` subcommand implementations (global model). Ported in spirit from
//! strykelang's `pkg/commands.rs`, reduced to the global store: install a
//! plugin, record it in `installed.toml`, and load it — natives via the
//! `zmodload -R` plugin host, scripts via `source` + `$fpath`.

use std::path::Path;

use super::manifest::{PluginKind, PluginManifest};
use super::store::{InstalledIndex, InstalledPlugin, Store};
use super::{resolver, PkgError, PkgResult};

/// `zpm add <SOURCE>` — resolve, install into the store, record, and load.
pub fn add(spec: &str) -> PkgResult<()> {
    let store = Store::user_default()?;
    store.ensure_layout()?;

    let staged = resolver::resolve(spec, &store)?;
    let manifest = PluginManifest::load(&staged.dir)?;
    let name = manifest
        .as_ref()
        .map(|m| m.plugin.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| staged.name.clone());
    let version = manifest
        .as_ref()
        .map(|m| m.plugin.version.clone())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "0.0.0".into());
    let description = manifest
        .as_ref()
        .map(|m| m.plugin.description.clone())
        .unwrap_or_default();
    let kind = PluginKind::detect(&staged.dir, manifest.as_ref())?;

    // Native plugins may need a build step before the cdylib exists at the
    // tree root (where the store copy will find it).
    if let PluginKind::Native(spec) = &kind {
        prepare_native(&staged.dir, spec, &name)?;
    }

    // Copy the loadable subset into the content-addressed store.
    let store_path = store.install_dir(&name, &version, &staged.dir)?;
    let integrity = super::store_integrity(&store_path)?;

    // Build the index record from the store-relative load info.
    let mut entry = InstalledPlugin {
        name: name.clone(),
        version: version.clone(),
        source: staged.source.clone(),
        integrity,
        ..Default::default()
    };
    match &kind {
        PluginKind::Native(_) => {
            entry.kind = "native".into();
            entry.lib = find_cdylib(&store_path)
                .ok_or_else(|| PkgError::Resolve(format!("{}: no cdylib after build", name)))?;
        }
        PluginKind::Script(s) => {
            entry.kind = "script".into();
            entry.source_files = s.source.clone();
            entry.fpath = s.fpath.clone();
        }
    }

    let mut index = InstalledIndex::load_from(&store)?;
    index.upsert(entry.clone());
    index.save_to(&store)?;

    // Clean the git clone scratch — the store copy is authoritative.
    if staged.source.starts_with("github:") || staged.source.starts_with("git+") {
        let _ = std::fs::remove_dir_all(&staged.dir);
    }

    load_entry(&store, &entry)?;
    let desc = if description.is_empty() {
        String::new()
    } else {
        format!(" — {}", description)
    };
    println!("zpm: added {}@{} ({}){}", name, version, entry.kind, desc);
    Ok(())
}

/// `zpm remove <NAME>` — unload (best-effort), drop the store copy + index row.
pub fn remove(name: &str) -> PkgResult<()> {
    let store = Store::user_default()?;
    let mut index = InstalledIndex::load_from(&store)?;
    let Some(entry) = index.remove(name) else {
        return Err(PkgError::Other(format!("{} is not installed", name)));
    };
    if entry.kind == "native" {
        let _ = crate::plugin_host::unload(name);
    }
    let dir = store.package_dir(&entry.name, &entry.version);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| PkgError::Io(format!("remove {}: {}", dir.display(), e)))?;
    }
    index.save_to(&store)?;
    println!("zpm: removed {}", name);
    Ok(())
}

/// `zpm list` — one line per installed plugin.
pub fn list() -> PkgResult<()> {
    let store = Store::user_default()?;
    let index = InstalledIndex::load_from(&store)?;
    if index.packages.is_empty() {
        println!("zpm: no plugins installed");
        return Ok(());
    }
    for p in &index.packages {
        println!("{:<24} {:<10} {:<7} {}", p.name, p.version, p.kind, p.source);
    }
    Ok(())
}

/// `zpm info <NAME>` — full record for one plugin.
pub fn info(name: &str) -> PkgResult<()> {
    let store = Store::user_default()?;
    let index = InstalledIndex::load_from(&store)?;
    let Some(p) = index.find(name) else {
        return Err(PkgError::Other(format!("{} is not installed", name)));
    };
    println!("name       {}", p.name);
    println!("version    {}", p.version);
    println!("kind       {}", p.kind);
    println!("source     {}", p.source);
    println!("store      {}", store.package_dir(&p.name, &p.version).display());
    if !p.integrity.is_empty() {
        println!("integrity  {}", p.integrity);
    }
    if !p.lib.is_empty() {
        println!("lib        {}", p.lib);
    }
    if !p.source_files.is_empty() {
        println!("files      {}", p.source_files.join(" "));
    }
    if !p.fpath.is_empty() {
        println!("fpath      {}", p.fpath.join(" "));
    }
    Ok(())
}

/// `zpm load [NAME]` — load one plugin, or every installed plugin when no name
/// is given. Zero network: reads only the store + index. This is what a
/// `.zshrc` calls at startup.
pub fn load(name: Option<&str>) -> PkgResult<()> {
    let store = Store::user_default()?;
    let index = InstalledIndex::load_from(&store)?;
    match name {
        Some(n) => {
            // 1. Already installed under this name → load from the store
            //    (fast: native = dlopen the mmap'd cdylib; no reinstall).
            if let Some(entry) = index.find(n) {
                return load_entry(&store, entry);
            }
            // 2. `n` is a SOURCE spec (owner/repo, github:…, path:…) — is a
            //    plugin from that source already installed? The index keys on
            //    the source label, since a repo basename usually differs from
            //    the plugin's `zpm.toml` name (`zshrs-forgit` → `forgit`).
            if let Some(label) = resolver::source_label(n) {
                if let Some(entry) = index.packages.iter().find(|p| p.source == label) {
                    return load_entry(&store, entry);
                }
                // 3. Not in the store yet → install-on-first-use, then load.
                //    This is what makes `zpm load owner/repo` in `.zshrc`
                //    self-install on the first startup and load fast after.
                //    (`add` records it in the store and loads it.)
                return add(n);
            }
            // A bare name that isn't installed and isn't a source.
            Err(PkgError::Other(format!("{} is not installed", n)))
        }
        None => {
            let mut errs = Vec::new();
            for p in &index.packages {
                if let Err(e) = load_entry(&store, p) {
                    errs.push(format!("{}: {}", p.name, e));
                }
            }
            if errs.is_empty() {
                Ok(())
            } else {
                Err(PkgError::Other(errs.join("; ")))
            }
        }
    }
}

/// `zpm update [NAME]` — re-resolve + reinstall from the recorded source.
pub fn update(name: Option<&str>) -> PkgResult<()> {
    let store = Store::user_default()?;
    let index = InstalledIndex::load_from(&store)?;
    let targets: Vec<String> = match name {
        Some(n) => vec![n.to_string()],
        None => index.packages.iter().map(|p| p.name.clone()).collect(),
    };
    for n in targets {
        let Some(p) = index.find(&n) else {
            return Err(PkgError::Other(format!("{} is not installed", n)));
        };
        let spec = source_to_spec(&p.source);
        add(&spec)?;
    }
    Ok(())
}

/// Convert a recorded provenance label back to a `zpm add` spec.
fn source_to_spec(source: &str) -> String {
    if let Some(rest) = source.strip_prefix("path+file://") {
        format!("path:{}", rest)
    } else {
        // `github:owner/repo` and `git+URL` are already valid `add` specs.
        source.to_string()
    }
}

/// Load one installed plugin: native via the `zmodload -R` host, script by
/// evaluating `fpath=(...); source ...` on the live shell.
fn load_entry(store: &Store, p: &InstalledPlugin) -> PkgResult<()> {
    let dir = store.package_dir(&p.name, &p.version);
    match p.kind.as_str() {
        "native" => {
            let lib = dir.join(&p.lib);
            crate::plugin_host::load(&lib.to_string_lossy())
                .map(|_| ())
                .map_err(PkgError::Resolve)
        }
        "script" => {
            let mut code = String::new();
            if !p.fpath.is_empty() {
                let dirs: Vec<String> = p
                    .fpath
                    .iter()
                    .map(|f| shquote(&dir.join(f).to_string_lossy()))
                    .collect();
                code.push_str(&format!("fpath=({} $fpath)\n", dirs.join(" ")));
            }
            for f in &p.source_files {
                let path = dir.join(f);
                code.push_str(&format!("source {}\n", shquote(&path.to_string_lossy())));
            }
            if code.is_empty() {
                return Ok(());
            }
            crate::ported::exec::execute_script(&code)
                .map(|_| ())
                .map_err(PkgError::Other)
        }
        other => Err(PkgError::Other(format!(
            "{}: unknown plugin kind '{}'",
            p.name, other
        ))),
    }
}

/// Build a native plugin's cdylib into the tree root so the store copy carries
/// it (the store copy skips `target/`). If a `lib*.{dylib,so}` already sits at
/// the root, use it as-is. Runs `cargo build --release` when a `Cargo.toml`
/// exists and building isn't disabled.
fn prepare_native(dir: &Path, spec: &super::manifest::NativeSpec, name: &str) -> PkgResult<()> {
    if find_cdylib(dir).is_some() {
        return Ok(()); // prebuilt cdylib already at the root.
    }
    let has_cargo = dir.join("Cargo.toml").is_file();
    let want_build = spec.build.unwrap_or(has_cargo);
    if !want_build {
        return Err(PkgError::Resolve(format!(
            "{}: native plugin has no prebuilt cdylib and build is disabled",
            name
        )));
    }
    if !has_cargo {
        return Err(PkgError::Resolve(format!(
            "{}: native plugin has neither a cdylib nor a Cargo.toml to build",
            name
        )));
    }
    let out = std::process::Command::new("cargo")
        .current_dir(dir)
        .arg("build")
        .arg("--release")
        .output()
        .map_err(|e| PkgError::Resolve(format!("cargo build: {} (is cargo installed?)", e)))?;
    if !out.status.success() {
        return Err(PkgError::Resolve(format!(
            "{}: cargo build failed:\n{}",
            name,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    // Copy the built cdylib from target/release to the tree root.
    let rel = dir.join("target").join("release");
    let built = find_cdylib(&rel).ok_or_else(|| {
        PkgError::Resolve(format!(
            "{}: cargo build produced no cdylib in {} (need crate-type=[\"cdylib\"])",
            name,
            rel.display()
        ))
    })?;
    std::fs::copy(rel.join(&built), dir.join(&built))
        .map_err(|e| PkgError::Io(format!("stage cdylib: {}", e)))?;
    Ok(())
}

/// Find a `lib*.{dylib,so}` (or `*.dll`) filename directly inside `dir`.
fn find_cdylib(dir: &Path) -> Option<String> {
    let suffix = std::env::consts::DLL_SUFFIX; // .dylib / .so / .dll
    let rd = std::fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let n = entry.file_name().to_string_lossy().into_owned();
        if n.ends_with(suffix) && n.starts_with(std::env::consts::DLL_PREFIX) {
            return Some(n);
        }
    }
    None
}

/// Minimal single-quote shell quoting for eval'd paths (paths rarely contain
/// `'`; escape it the POSIX way just in case).
fn shquote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
