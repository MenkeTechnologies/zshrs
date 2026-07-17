//! Resolve a user-supplied source spec into a staged plugin directory.
//! Adapted from strykelang's `pkg/resolver.rs`, trimmed to the three source
//! forms zshrs plugins ship as (no semver/registry graph — global model).
//!
//! Source forms accepted by `znative add <SOURCE>`:
//! - `owner/repo` or `github:owner/repo` → `git clone https://github.com/owner/repo`
//! - `git+URL`, or any URL ending in `.git` → `git clone URL`
//! - `path:DIR`, an absolute path, or `./rel`, `../rel` → a local directory
//!
//! `@REF` may be appended to a git/github source to pin a branch/tag/commit
//! (`owner/repo@v1.2.0`). The resolver clones into `$ZSHRS_HOME/pkg/git/` and
//! returns the working tree; the caller copies the loadable subset into the
//! content-addressed store.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::store::Store;
use super::{PkgError, PkgResult};

/// A staged source ready to install into the store.
pub struct Staged {
    /// Working directory containing the plugin tree.
    pub dir: PathBuf,
    /// Inferred plugin name (repo/dir basename).
    pub name: String,
    /// Provenance label recorded in the index: `github:owner/repo`,
    /// `git+URL`, or `path+file://DIR`.
    pub source: String,
}

/// Resolve `spec` into a [`Staged`] tree. Clones (git/github) land under
/// `store.git_dir()`; local paths are used in place.
pub fn resolve(spec: &str, store: &Store) -> PkgResult<Staged> {
    let (base, git_ref) = split_ref(spec);

    // Local path forms.
    if let Some(p) = local_path(base) {
        let dir = p
            .canonicalize()
            .map_err(|e| PkgError::Resolve(format!("path {}: {}", p.display(), e)))?;
        if !dir.is_dir() {
            return Err(PkgError::Resolve(format!(
                "path {} is not a directory",
                dir.display()
            )));
        }
        let name = basename(&dir);
        let source = format!("path+file://{}", dir.display());
        return Ok(Staged { dir, name, source });
    }

    // Git / GitHub forms.
    let (url, label, name) = git_url(base)?;
    store
        .ensure_layout()
        .map_err(|e| PkgError::Resolve(e.to_string()))?;
    let dir = store.git_dir().join(&name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir)
            .map_err(|e| PkgError::Io(format!("clear {}: {}", dir.display(), e)))?;
    }
    git_clone(&url, &dir, git_ref)?;
    Ok(Staged {
        dir,
        name,
        source: label,
    })
}

/// The provenance label a `spec` WOULD receive, computed WITHOUT cloning or
/// network access. Used by `znative load <spec>` to check whether a source is
/// already installed (the index keys on this label, since a repo's basename
/// often differs from its `znative.toml` plugin name — e.g. `zshrs-forgit` →
/// `forgit`). Returns `None` for a bare plugin name (not a source form).
pub fn source_label(spec: &str) -> Option<String> {
    let (base, _ref) = split_ref(spec);
    if let Some(p) = local_path(base) {
        // Match the `path+file://<canonical>` the installer records.
        let dir = p.canonicalize().ok()?;
        return Some(format!("path+file://{}", dir.display()));
    }
    git_url(base).ok().map(|(_url, label, _name)| label)
}

/// Split a trailing `@REF` (branch/tag/commit) off a spec. Only splits on the
/// LAST `@` so `git@host:...` SSH URLs keep their `@`.
fn split_ref(spec: &str) -> (&str, Option<&str>) {
    // A leading path or scheme with an early `@` (SSH) should not be treated as
    // a ref; only accept `@` after the last `/`.
    if let Some(at) = spec.rfind('@') {
        let after_slash = spec.rfind('/').map(|s| at > s).unwrap_or(true);
        if after_slash && at + 1 < spec.len() {
            return (&spec[..at], Some(&spec[at + 1..]));
        }
    }
    (spec, None)
}

/// Recognize local-path forms; returns the path when `spec` is one.
fn local_path(spec: &str) -> Option<PathBuf> {
    if let Some(rest) = spec.strip_prefix("path:") {
        return Some(PathBuf::from(rest));
    }
    if spec.starts_with('/') || spec.starts_with("./") || spec.starts_with("../") || spec.starts_with('~')
    {
        let expanded = if let Some(rest) = spec.strip_prefix("~/") {
            if let Some(home) = std::env::var_os("HOME") {
                PathBuf::from(home).join(rest)
            } else {
                PathBuf::from(spec)
            }
        } else {
            PathBuf::from(spec)
        };
        return Some(expanded);
    }
    None
}

/// Map a non-local spec to `(clone_url, provenance_label, name)`.
fn git_url(spec: &str) -> PkgResult<(String, String, String)> {
    if let Some(rest) = spec.strip_prefix("git+") {
        let name = repo_basename(rest);
        return Ok((rest.to_string(), format!("git+{}", rest), name));
    }
    if let Some(rest) = spec.strip_prefix("github:") {
        let url = format!("https://github.com/{}", rest.trim_end_matches(".git"));
        let name = repo_basename(&url);
        return Ok((url, format!("github:{}", rest.trim_end_matches(".git")), name));
    }
    if spec.ends_with(".git") || spec.contains("://") {
        let name = repo_basename(spec);
        return Ok((spec.to_string(), format!("git+{}", spec), name));
    }
    // `owner/repo` shorthand → GitHub.
    if spec.split('/').count() == 2 && !spec.contains(' ') {
        let owner_repo = spec.trim_end_matches(".git");
        let url = format!("https://github.com/{}", owner_repo);
        let name = repo_basename(&url);
        return Ok((url, format!("github:{}", owner_repo), name));
    }
    Err(PkgError::Resolve(format!(
        "unrecognized source '{}': expected owner/repo, github:owner/repo, \
         git+URL, or a local path",
        spec
    )))
}

/// `git clone --depth 1 [--branch REF] URL DIR` — shallow for speed.
fn git_clone(url: &str, dir: &Path, git_ref: Option<&str>) -> PkgResult<()> {
    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(r) = git_ref {
        cmd.arg("--branch").arg(r);
    }
    cmd.arg(url).arg(dir);
    let out = cmd
        .output()
        .map_err(|e| PkgError::Resolve(format!("git clone: {} (is git installed?)", e)))?;
    if !out.status.success() {
        // Retry without --branch: a REF that's a commit sha can't be used with
        // `--branch` on a shallow clone. Fall back to a full clone + checkout.
        if git_ref.is_some() {
            return git_clone_checkout(url, dir, git_ref.unwrap());
        }
        return Err(PkgError::Resolve(format!(
            "git clone {} failed: {}",
            url,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// Full clone + `git checkout REF` — the fallback when a shallow `--branch`
/// clone can't reach an arbitrary commit.
fn git_clone_checkout(url: &str, dir: &Path, git_ref: &str) -> PkgResult<()> {
    if dir.exists() {
        let _ = std::fs::remove_dir_all(dir);
    }
    let out = Command::new("git")
        .arg("clone")
        .arg(url)
        .arg(dir)
        .output()
        .map_err(|e| PkgError::Resolve(format!("git clone: {}", e)))?;
    if !out.status.success() {
        return Err(PkgError::Resolve(format!(
            "git clone {} failed: {}",
            url,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    let out = Command::new("git")
        .current_dir(dir)
        .arg("checkout")
        .arg(git_ref)
        .output()
        .map_err(|e| PkgError::Resolve(format!("git checkout: {}", e)))?;
    if !out.status.success() {
        return Err(PkgError::Resolve(format!(
            "git checkout {} failed: {}",
            git_ref,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

/// Basename of a directory path, sans trailing separators.
fn basename(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "plugin".into())
}

/// Repo name from a clone URL: strip `.git`, take the last path segment.
fn repo_basename(url: &str) -> String {
    url.trim_end_matches('/')
        .trim_end_matches(".git")
        .rsplit(['/', ':'])
        .next()
        .unwrap_or("plugin")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_ref_only_after_last_slash() {
        assert_eq!(split_ref("o/r@v1"), ("o/r", Some("v1")));
        assert_eq!(split_ref("o/r"), ("o/r", None));
        // SSH URL @ must not split.
        assert_eq!(split_ref("git@github.com:o/r.git"), ("git@github.com:o/r.git", None));
    }

    #[test]
    fn git_url_forms() {
        let (u, l, n) = git_url("owner/repo").unwrap();
        assert_eq!(u, "https://github.com/owner/repo");
        assert_eq!(l, "github:owner/repo");
        assert_eq!(n, "repo");
        let (u, _, n) = git_url("github:a/b").unwrap();
        assert_eq!(u, "https://github.com/a/b");
        assert_eq!(n, "b");
        let (u, l, _) = git_url("git+https://x.com/y.git").unwrap();
        assert_eq!(u, "https://x.com/y.git");
        assert_eq!(l, "git+https://x.com/y.git");
        assert!(git_url("not a source").is_err());
    }

    #[test]
    fn local_path_forms() {
        assert!(local_path("path:/tmp/x").is_some());
        assert!(local_path("/abs").is_some());
        assert!(local_path("./rel").is_some());
        assert!(local_path("owner/repo").is_none());
    }
}
