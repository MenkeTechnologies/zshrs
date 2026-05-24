//! Port of `_canonical_paths` (zsh Completion/Unix/Type/_canonical_paths, 122 lines).
//!
//! Local shell reference: `compsys/functions/Base/Utility/_canonical_paths`
//! (or its `Unix/Type/` upstream equivalent at
//! `/opt/homebrew/share/zsh/functions/_canonical_paths`).
//!
//! Completes a list of given paths AND tries to offer completions that
//! point to the same file via alternate representations:
//!   - relative path when an absolute path is given (and vice versa)
//!   - parent chasing via `..` when the user types `..`-laden prefixes
//!   - paths obtained from symlink-aware canonicalisation
//!
//! Shell flags:
//!   `-A var`  — pull paths from the named array (we take direct `paths`)
//!   `-N`      — skip the leading canonicalisation pass
//!   `-MJV12onfX` — passthrough to compadd (we accept a CompaddPassthrough
//!                  but don't currently surface every flag at this layer)
//!
//! The previous Rust stub did `fs::canonicalize(path)` + prefix-match on
//! the input paths. That gave zero reverse-mapping (abs↔rel), no `..`
//! chasing, no symlinked-subdir recursion. Replaced.
//!
//! What the faithful port handles:
//!   - initial files list (canonicalised unless `-N`)
//!   - matches the user's PREFIX against `files` AND maps the canonical
//!     prefix back to the user-typed form (so `/u/l/b<TAB>` against a
//!     symlinked `/usr/local/bin` shows `/u/l/b/somefile` as `/u/l/b/...`
//!     rather than `/usr/local/bin/...`)
//!   - `..` chasing up to `canonical-paths-back-limit` (default 8) when
//!     PREFIX is a pure `.`/`..` chain
//!
//! What's intentionally simplified:
//!   - symlinked-subdir recursion `${expref?*(@)}` — needs a glob qualifier
//!     evaluator at the leaf; covered by the recursive walker but limited
//!     to one level of synthesis (the common case)
//!   - `-M matchspec` passthrough is stored on the opts but only honored
//!     at the compadd layer (so case-folding etc. routes through compadd
//!     as expected)

use std::path::{Path, PathBuf};

use crate::compcore::CompletionState;
use crate::completion::Completion;
use crate::zstyle::ZStyleStore;

pub struct CanonicalPathsOpts<'a> {
    pub tag: &'a str,
    pub description: &'a str,
    /// `-N` — input paths are already canonical; skip canonicalisation.
    pub skip_canonicalize: bool,
    /// `canonical-paths-back-limit` zstyle (default 8). Caps how deep
    /// `..` chasing goes.
    pub back_limit: usize,
}

impl<'a> Default for CanonicalPathsOpts<'a> {
    fn default() -> Self {
        Self {
            tag: "canonical-paths",
            description: "path",
            skip_canonicalize: false,
            back_limit: 8,
        }
    }
}

pub fn _canonical_paths(
    state: &mut CompletionState,
    opts: &CanonicalPathsOpts<'_>,
    styles: Option<&ZStyleStore>,
    curcontext: &str,
    paths: &[String],
) -> bool {
    // Effective back limit — zstyle override wins.
    let back_limit = styles
        .and_then(|s| {
            s.lookup_str(
                &format!(":completion:{}:{}", curcontext, opts.tag),
                "canonical-paths-back-limit",
            )
            .and_then(|v| v.parse().ok())
        })
        .unwrap_or(opts.back_limit);

    // Build initial `files` list.
    let mut files: Vec<PathBuf> = if opts.skip_canonicalize {
        paths.iter().map(PathBuf::from).collect()
    } else {
        paths
            .iter()
            .map(PathBuf::from)
            .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
            .collect()
    };

    let mut matches: Vec<String> = Vec::new();

    // Initial add: every file whose canonical form starts with the
    // canonical PREFIX gets emitted in the user-typed form.
    let prefix = state.params.prefix.clone();
    add_paths(&prefix, &files, &mut matches);

    // shell:91-94: if PREFIX is empty → also try with `/` as origpref.
    if prefix.is_empty() {
        add_paths("/", &files, &mut matches);
    }

    // shell:95-115: `..`-chasing. If PREFIX matches `..(/.(|.))*(|/)`
    // (pure dot-up chain, possibly trailing slash), walk up the parent
    // chain `back_limit` times, recomputing matches at each level.
    if is_dotup_chain(&prefix) {
        let mut base = prefix.clone();
        if !base.ends_with('/') {
            if !base.ends_with("..") {
                base.push('.');
            }
            base.push('/');
        }
        let mut remaining = back_limit;
        while remaining > 0 {
            // Stop if `$base..` resolves to same inode as `$base`
            // (filesystem root reached). We approximate via path-string
            // equality after canonicalize.
            let base_p = PathBuf::from(&base);
            let base_up = PathBuf::from(format!("{}..", base));
            let base_canon = std::fs::canonicalize(&base_p).ok();
            let up_canon = std::fs::canonicalize(&base_up).ok();
            if base_canon.is_some() && base_canon == up_canon {
                break;
            }
            base.push_str("../");
            // Recompute files relative to the new base.
            if !opts.skip_canonicalize {
                files = paths
                    .iter()
                    .map(PathBuf::from)
                    .map(|p| std::fs::canonicalize(&p).unwrap_or(p))
                    .collect();
            }
            add_paths(&base, &files, &mut matches);
            remaining -= 1;
        }
    }

    if matches.is_empty() {
        return false;
    }

    // Dedupe while preserving order.
    let mut seen = std::collections::HashSet::new();
    matches.retain(|m| seen.insert(m.clone()));

    state.begin_group(opts.tag, true);
    if !opts.description.is_empty() {
        state.add_explanation(opts.description.to_string(), Some(opts.tag));
    }
    for m in &matches {
        state.add_match(Completion::new(m.clone()), Some(opts.tag));
    }
    state.end_group();
    state.nmatches > 0
}

/// Implements shell:17-49 `_canonical_paths_add_paths`. Walks the
/// canonical → original prefix map and pushes matching files into the
/// receiver.
fn add_paths(origpref: &str, files: &[PathBuf], matches: &mut Vec<String>) {
    // shell:24-31: rltrim handling + curpref / canpref computation.
    let (curpref, rltrim) = if origpref.is_empty() {
        ("./".to_string(), "".to_string())
    } else if origpref == "." || origpref.ends_with("/.") {
        (origpref[..origpref.len() - 1].to_string(), ".".to_string())
    } else {
        (origpref.to_string(), "".to_string())
    };
    let canpref = match std::fs::canonicalize(&curpref) {
        Ok(p) => {
            let mut s = p.to_string_lossy().to_string();
            if curpref.ends_with('/') && !s.ends_with('/') {
                s.push('/');
            }
            s + &rltrim
        }
        Err(_) => return,
    };
    let origpref_eff = if !origpref.ends_with('/')
        && !origpref.is_empty()
        && canpref.ends_with('/')
    {
        format!("{}/", origpref)
    } else {
        origpref.to_string()
    };

    for f in files {
        let fstr = f.to_string_lossy();
        if fstr.starts_with(canpref.as_str()) {
            // shell:35-40 fast path: when canpref == origpref no
            // rewriting needed; otherwise replace canpref with
            // origpref in the file.
            let mapped = if canpref == origpref_eff {
                fstr.to_string()
            } else {
                let stripped = &fstr[canpref.len()..];
                format!("{}{}", origpref_eff, stripped)
            };
            matches.push(mapped);
        }
    }

    // shell:46-48: recurse into immediate symlink-resolved subdirs.
    // We approximate this by trying every immediate child of the
    // expanded curpref; cheap because hash-based dedupe catches dups.
    if let Ok(entries) = std::fs::read_dir(&curpref) {
        for e in entries.flatten() {
            let cpath = e.path();
            if cpath.is_dir() && cpath.is_symlink() {
                let leaf = cpath.file_name().and_then(|s| s.to_str());
                if let Some(name) = leaf {
                    let new_origpref = format!("{}{}/", origpref_eff, name);
                    // Bounded recursion: depth limited by string growth.
                    if new_origpref.len() < 4096 {
                        // Inline single-step: scan files matching new_origpref.
                        if let Ok(rp) = std::fs::canonicalize(&cpath) {
                            let cs = rp.to_string_lossy().to_string() + "/";
                            for f in files {
                                let fs = f.to_string_lossy();
                                if fs.starts_with(&cs) {
                                    let stripped = &fs[cs.len()..];
                                    matches.push(format!("{}{}", new_origpref, stripped));
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// True when `s` is a pure `..(/.(|.))*(|/)`-shaped chain — i.e.
/// `..`, `../`, `../..`, `../../`, `../.`, etc. Shell pattern at
/// _canonical_paths:95.
fn is_dotup_chain(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let stripped = s.strip_suffix('/').unwrap_or(s);
    if stripped.is_empty() {
        return false;
    }
    stripped
        .split('/')
        .all(|seg| seg == ".." || seg == "." || seg.is_empty())
        && stripped.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_canonicalize_uses_paths_verbatim() {
        // Use the test process's actual cwd which is symlink-free.
        // Filesystem roots like /tmp and /etc on macOS go through
        // /private/* symlinks which exposes the canonicalization
        // mismatch.
        let cwd = std::env::current_dir().unwrap();
        let cwd_s = cwd.to_string_lossy().to_string();
        let mut state = CompletionState::new();
        state.params.prefix = cwd_s.clone();
        let opts = CanonicalPathsOpts {
            skip_canonicalize: true,
            ..Default::default()
        };
        let foo = format!("{}/foo", cwd_s);
        let bar = format!("{}/bar", cwd_s);
        let off = "/var/log".to_string();
        let ok = _canonical_paths(
            &mut state,
            &opts,
            None,
            "",
            &[foo.clone(), bar.clone(), off],
        );
        assert!(ok);
        let names: Vec<&str> = state.groups[0]
            .matches
            .iter()
            .map(|c| c.str_.as_str())
            .collect();
        assert!(names.iter().any(|n| n == &foo.as_str()), "got {names:?}");
        assert!(names.iter().any(|n| n == &bar.as_str()));
        assert!(!names.iter().any(|n| n == &"/var/log"));
    }

    #[test]
    fn dotup_chain_detection() {
        assert!(is_dotup_chain(".."));
        assert!(is_dotup_chain("../"));
        assert!(is_dotup_chain("../../"));
        assert!(is_dotup_chain("../."));
        assert!(!is_dotup_chain("/etc"));
        assert!(!is_dotup_chain("foo/.."));
        assert!(!is_dotup_chain(""));
    }

    #[test]
    fn nonexistent_prefix_returns_false() {
        let mut state = CompletionState::new();
        state.params.prefix = "/no/such/path".into();
        let opts = CanonicalPathsOpts {
            skip_canonicalize: true,
            ..Default::default()
        };
        let ok = _canonical_paths(
            &mut state,
            &opts,
            None,
            "",
            &["/etc/hosts".into()],
        );
        // /etc/hosts doesn't start with /no/such/path → no matches.
        assert!(!ok);
    }
}
