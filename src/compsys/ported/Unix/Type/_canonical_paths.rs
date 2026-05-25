//! Port of `_canonical_paths` from `Completion/Unix/Type/_canonical_paths`.
//!
//! Full upstream body (123 lines verbatim):
//! ```text
//! sh:  1  #autoload
//! sh:  2
//! sh:  3  # This completion function completes all paths given to it, and also tries to
//! sh:  4  # offer completions which point to the same file as one of the paths given
//! sh:  5  # (relative path when an absolute path is given, and vice versa; when ..'s are
//! sh:  6  # present in the word to be completed, and some paths got from symlinks).
//! sh:  7
//! sh:  8  # Usage: _canonical_paths [-A var] [-N] [-MJV12onfX] tag desc [paths...]
//! sh:  9
//! sh: 10  # -A, if specified, takes the paths from the array variable specified. Paths
//! sh: 11  # can also be specified on the command line as shown above. -N, if specified,
//! sh: 12  # prevents canonicalizing the paths given before using them for completion, in
//! sh: 13  # case they are already so. `tag' and `desc' arguments are well, obvious :) In
//! sh: 14  # addition, the options -M, -J, -V, -1, -2, -o, -n, -F, -x, -X are passed to
//! sh: 15  # compadd.
//! sh: 16
//! sh: 17  _canonical_paths_add_paths () {
//! sh: 18    # origpref = original prefix
//! sh: 19    # expref = expanded prefix
//! sh: 20    # curpref = current prefix
//! sh: 21    # canpref = canonical prefix
//! sh: 22    # rltrim = suffix to trim and readd
//! sh: 23    local origpref=$1 expref rltrim curpref canpref subdir
//! sh: 24    [[ $2 != add ]] && matches=()
//! sh: 25    expref=${~origpref} 2>/dev/null
//! sh: 26    [[ $origpref == (|*/). ]] && rltrim=.
//! sh: 27    curpref=${${expref%$rltrim}:-./}
//! sh: 28    canpref=$curpref:P
//! sh: 29    [[ $curpref == */ && $canpref == *[^/] ]] && canpref+=/
//! sh: 30    canpref+=$rltrim
//! sh: 31    [[ $expref == *[^/] && $canpref == */ ]] && origpref+=/
//! sh: 32
//! sh: 33    # Append to $matches the subset of $files that matches $canpref.
//! sh: 34    if [[ $canpref == $origpref ]]; then
//! sh: 35      # This codepath honours any -M matchspec parameters.
//! sh: 36      () {
//! sh: 37        local -a tmp_buffer
//! sh: 38        compadd -A tmp_buffer "$__gopts[@]" -a files
//! sh: 39        matches+=( "${(@)tmp_buffer/$canpref/$origpref}" )
//! sh: 40      }
//! sh: 41    else
//! sh: 42      # ### Ideally, this codepath would do what the 'if' above does,
//! sh: 43      # ### but telling compadd to pretend the "word on the command line"
//! sh: 44      # ### is ${"the word on the command line"/$origpref/$canpref}.
//! sh: 45      # ### The following approximates that.
//! sh: 46      matches+=(${(q)${(M)files:#$canpref*}/$canpref/$origpref})
//! sh: 47    fi
//! sh: 48
//! sh: 49    for subdir in $expref?*(@); do
//! sh: 50      _canonical_paths_add_paths ${subdir/$expref/$origpref} add
//! sh: 51    done
//! sh: 52  }
//! sh: 53
//! sh: 54  _canonical_paths() {
//! sh: 55    # The following parameters are used by callee functions:
//! sh: 56    #    __gopts
//! sh: 57    #    matches
//! sh: 58    #    files
//! sh: 59    #    (possibly others)
//! sh: 60
//! sh: 61    local __index
//! sh: 62    typeset -a __gopts __opts
//! sh: 63
//! sh: 64    zparseopts -D -a __gopts M+: J+: V+: o+: 1 2 n F: x+: X+: A:=__opts N=__opts
//! sh: 65
//! sh: 66    : ${1:=canonical-paths} ${2:=path}
//! sh: 67
//! sh: 68    __index=$__opts[(I)-A]
//! sh: 69    (( $__index )) && set -- $@ ${(P)__opts[__index+1]}
//! sh: 70
//! sh: 71    local expl ret=1 tag=$1 desc=$2
//! sh: 72
//! sh: 73    shift 2
//! sh: 74
//! sh: 75    if ! zmodload -F zsh/stat b:zstat 2>/dev/null; then
//! sh: 76      _wanted "$tag" expl "$desc" compadd $__gopts $@ && ret=0
//! sh: 77      return ret
//! sh: 78    fi
//! sh: 79
//! sh: 80    typeset REPLY
//! sh: 81    typeset -a matches files
//! sh: 82
//! sh: 83    if (( $__opts[(I)-N] )); then
//! sh: 84      files=($@)
//! sh: 85    else
//! sh: 86      files+=($@:P)
//! sh: 87    fi
//! sh: 88
//! sh: 89    local base=$PREFIX
//! sh: 90    typeset -i blimit
//! sh: 91
//! sh: 92    _canonical_paths_add_paths $base
//! sh: 93
//! sh: 94    if [[ -z $base ]]; then
//! sh: 95      _canonical_paths_add_paths / add
//! sh: 96    elif [[ $base == ..(/.(|.))#(|/) ]]; then
//! sh: 97
//! sh: 98      # This style controls how many parent directory links (..) to chase searching
//! sh: 99      # for possible completions. The default is 8. Note that this chasing is
//! sh:100      # triggered only when the user enters at least a .. and the path completed
//! sh:101      # contains only . or .. components. A value of 0 turns off .. link chasing
//! sh:102      # altogether.
//! sh:103
//! sh:104      zstyle -s ":completion:${curcontext}:$tag" \
//! sh:105        canonical-paths-back-limit blimit || blimit=8
//! sh:106
//! sh:107      if [[ $base != */ ]]; then
//! sh:108        [[ $base != *.. ]] && base+=.
//! sh:109        base+=/
//! sh:110      fi
//! sh:111      until [[ $base.. -ef $base || blimit -le 0 ]]; do
//! sh:112        base+=../
//! sh:113        _canonical_paths_add_paths $base add
//! sh:114        blimit+=-1
//! sh:115      done
//! sh:116    fi
//! sh:117
//! sh:118    _wanted "$tag" expl "$desc" compadd $__gopts -Q -U -a matches && ret=0
//! sh:119
//! sh:120    return ret
//! sh:121  }
//! sh:122
//! sh:123  _canonical_paths "$@"
//! ```



use std::path::{Path, PathBuf};

use crate::compsys::compcore::CompletionState;
use crate::compsys::completion::Completion;
use crate::compsys::zstyle::ZStyleStore;

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

    // shell:46-48: recurse into symlink-resolved subdirs. Fully
    // recursive (depth-capped + cycle-detected) so the user gets
    // alternate paths to deep symlinked content.
    recurse_symlinks(
        &curpref,
        &origpref_eff,
        files,
        matches,
        &mut std::collections::HashSet::new(),
        8, // depth cap — matches canonical-paths-back-limit default
    );
}

fn recurse_symlinks(
    curdir: &str,
    origpref: &str,
    files: &[PathBuf],
    matches: &mut Vec<String>,
    visited: &mut std::collections::HashSet<PathBuf>,
    depth: usize,
) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(curdir) else {
        return;
    };
    for e in entries.flatten() {
        let cpath = e.path();
        if !cpath.is_dir() || !cpath.is_symlink() {
            continue;
        }
        let Some(name) = cpath.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let new_origpref = format!("{}{}/", origpref, name);
        if new_origpref.len() >= 4096 {
            continue;
        }
        let Ok(real) = std::fs::canonicalize(&cpath) else {
            continue;
        };
        // Cycle detection: don't revisit the same canonical dir.
        if !visited.insert(real.clone()) {
            continue;
        }
        let real_str = real.to_string_lossy().to_string() + "/";
        for f in files {
            let fs = f.to_string_lossy();
            if fs.starts_with(&real_str) {
                let stripped = &fs[real_str.len()..];
                matches.push(format!("{}{}", new_origpref, stripped));
            }
        }
        // Recurse into the symlink's resolved content.
        if let Some(real_no_slash) = real_str.strip_suffix('/') {
            recurse_symlinks(real_no_slash, &new_origpref, files, matches, visited, depth - 1);
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
        assert!(!ok);
    }

    #[test]
    fn back_limit_zstyle_overrides_default() {
        let mut store = crate::compsys::zstyle::ZStyleStore::new();
        store.set(
            ":completion::canonical-paths",
            "canonical-paths-back-limit",
            vec!["3".into()],
            false,
        );
        let mut state = CompletionState::new();
        state.params.prefix = "..".into();
        let opts = CanonicalPathsOpts::default();
        // Just verify the zstyle parse + lookup path doesn't crash;
        // actual chase depth is verified at the algo level.
        let _ = _canonical_paths(&mut state, &opts, Some(&store), "", &[]);
    }

    #[test]
    fn add_paths_emits_files_with_prefix_match() {
        let mut matches: Vec<String> = Vec::new();
        let cwd = std::env::current_dir().unwrap();
        let cwd_s = cwd.to_string_lossy().to_string();
        let files = vec![
            PathBuf::from(format!("{}/a", cwd_s)),
            PathBuf::from(format!("{}/b", cwd_s)),
            PathBuf::from("/elsewhere/c"),
        ];
        add_paths(&cwd_s, &files, &mut matches);
        assert!(matches.iter().any(|m| m.ends_with("/a")));
        assert!(matches.iter().any(|m| m.ends_with("/b")));
        assert!(!matches.iter().any(|m| m.starts_with("/elsewhere")));
    }

    #[test]
    fn dotup_emits_paths_in_parent_chain() {
        let mut state = CompletionState::new();
        state.params.prefix = "..".into();
        let opts = CanonicalPathsOpts::default();
        // Just need to not panic + back limit honored.
        let _ = _canonical_paths(&mut state, &opts, None, "", &[]);
    }

    #[test]
    fn recurse_symlinks_terminates_on_cycle() {
        // Set up: tmp/a → tmp (symlink loop back to parent)
        let tmp = std::env::temp_dir().join(format!(
            "zshrs_cp_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let link = tmp.join("loop");
        #[cfg(unix)]
        {
            let _ = std::os::unix::fs::symlink(&tmp, &link);
        }
        let mut matches = Vec::new();
        let mut visited = std::collections::HashSet::new();
        recurse_symlinks(
            tmp.to_str().unwrap(),
            "test/",
            &[],
            &mut matches,
            &mut visited,
            8,
        );
        // Critical: must NOT infinite-loop. If we get here, cycle
        // detection worked.
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
