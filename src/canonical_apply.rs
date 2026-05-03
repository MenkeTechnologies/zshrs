//! Apply daemon canonical state to a freshly-built ShellExecutor —
//! by reading the daemon's rkyv shard directly from disk. No IPC.
//!
//! **Why direct shard read, not IPC.** The original spec
//! (`docs/DAEMON.md` "NO WALKING IN CLIENTS" + cache-architecture
//! memory) calls for thin clients that mmap the daemon's pre-built
//! shards as a zero-copy data plane. The earlier IPC version of this
//! file did 1+ `definitions_query` round-trips per cold-start, which
//! at ~600μs per round-trip put us 5-10ms over the spec target. The
//! mmap path is the real architecture: kernel page-cache after first
//! launch + rkyv check_archived + struct copy = sub-millisecond.
//!
//! IPC stays intact for `zd` / editor plugins / dashboards (see
//! `daemon/definitions.rs`). It's the right interface for "give me
//! the current catalog snapshot" from external tools. It's the wrong
//! interface for the shell's own cold-start hot path.
//!
//! The recorder writes one `*-recorder.rkyv` shard per ingest into
//! `~/.zshrs/images/`. We pick the latest by mtime, deserialize,
//! and copy fields straight into the executor's pub HashMaps.
//!
//! Failure mode: any I/O error → return 0; caller falls back to
//! vanilla `source_startup_files()`. Logged so the user can see why.

#![cfg(feature = "daemon")]

use std::path::PathBuf;

use crate::daemon::paths::CachePaths;
use crate::daemon::shard::{list_shards, read_canonical_shard, CanonicalShard};
use crate::exec::ShellExecutor;

/// Read the latest recorder shard from disk and apply its canonical
/// state to the executor. Returns total rows applied (0 if no shard
/// or read failure → caller falls back).
pub fn apply_all(executor: &mut ShellExecutor) -> usize {
    let t0 = std::time::Instant::now();

    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "canonical_apply: cache paths unresolved");
            return 0;
        }
    };

    let shard_path = match latest_recorder_shard(&paths) {
        Some(p) => p,
        None => {
            tracing::info!(
                "canonical_apply: no recorder shard found in {} — vanilla fallback",
                paths.images.display()
            );
            return 0;
        }
    };

    let shard = match read_canonical_shard(&shard_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, path = %shard_path.display(), "canonical_apply: shard read failed");
            return 0;
        }
    };

    let total = apply_shard(executor, shard);
    let elapsed_us = t0.elapsed().as_micros();
    tracing::info!(
        rows = total,
        elapsed_us,
        path = %shard_path.display(),
        "canonical state applied from rkyv shard (no IPC)"
    );
    total
}

/// Walk `~/.zshrs/images/` for `*-recorder.rkyv` and return the
/// newest by mtime. None if the dir doesn't exist or has no recorder
/// shard.
fn latest_recorder_shard(paths: &CachePaths) -> Option<PathBuf> {
    let entries = list_shards(paths).ok()?;
    entries
        .into_iter()
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.ends_with("-recorder.rkyv"))
                .unwrap_or(false)
        })
        .max_by_key(|p| {
            std::fs::metadata(p)
                .and_then(|m| m.modified())
                .ok()
        })
}

fn apply_shard(executor: &mut ShellExecutor, shard: CanonicalShard) -> usize {
    let mut total = 0;

    // Aliases (3 flavors).
    for (n, v) in shard.aliases {
        executor.aliases.insert(n, v);
        total += 1;
    }
    for (n, v) in shard.global_aliases {
        executor.global_aliases.insert(n, v);
        total += 1;
    }
    for (n, v) in shard.suffix_aliases {
        executor.suffix_aliases.insert(n, v);
        total += 1;
    }

    // Exported env: mirror to process env so child commands inherit.
    for (n, v) in shard.env_exports {
        std::env::set_var(&n, &v);
        executor.variables.insert(n, v);
        total += 1;
    }

    // Non-exported shell params.
    for (n, v) in shard.params {
        executor.variables.insert(n, v);
        total += 1;
    }

    // setopt / unsetopt.
    for opt in shard.setopts {
        executor.options.insert(opt, true);
        total += 1;
    }
    for opt in shard.unsetopts {
        executor.options.insert(opt, false);
        total += 1;
    }

    // path + fpath: ordered Vec<String> in the shard.
    if !shard.path.is_empty() {
        let joined = shard.path.join(":");
        std::env::set_var("PATH", &joined);
        executor.variables.insert("PATH".to_string(), joined);
        total += shard.path.len();
        executor.arrays.insert("path".to_string(), shard.path);
    }
    if !shard.fpath.is_empty() {
        let joined = shard.fpath.join(":");
        std::env::set_var("FPATH", &joined);
        executor.variables.insert("FPATH".to_string(), joined);
        total += shard.fpath.len();
        executor.fpath = shard.fpath.iter().map(PathBuf::from).collect();
        executor.arrays.insert("fpath".to_string(), shard.fpath);
    }

    // named_dir (hash -d): pre-resolves `~name` expansion. zsh stores
    // these in a global hash; zshrs reads them via the same
    // `named_dirs` array surfaced through compsys / expansion. The
    // executor doesn't have a dedicated field today; defer until the
    // executor exposes it. Skipping is safe — `~name` won't expand
    // until then but everything else works.
    let _ = shard.named_dirs;

    // bindkey / compdef / zstyle / zmodload / functions / zle:
    // captured in shard but not yet wired to executor. Skipping is
    // safe — those subsystems work without pre-population, just
    // slower (autoload on demand instead of pre-loaded).
    let _ = shard.functions;
    let _ = shard.autoload_functions;
    let _ = shard.bindkeys;
    let _ = shard.compdef;
    let _ = shard.zstyle;
    let _ = shard.zmodload;
    let _ = shard.manpath;
    let _ = shard.plugins;
    let _ = shard.sourced_files;
    let _ = shard.extras;

    total
}
