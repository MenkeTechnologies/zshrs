//! Doctor report data — shared by `zshrs --doctor` and the `doctor` builtin.
//!
//! **zshrs-original — no C source counterpart.** [`gather`] collects every
//! datum once as one JSON object; the binary renders it as text or dumps it
//! with `--json`, and `doctor --json` prints it from inside a running shell.

use crate::compsys::cache::CompsysCache;
use std::path::PathBuf;

/// Count the entries of every existing directory in a `:`-separated list.
/// Returns `(total, valid, missing_dirs, entry_count)`.
pub fn scan_dir_list(list: &str) -> (usize, usize, Vec<String>, usize) {
    let dirs: Vec<&str> = list.split(':').filter(|s| !s.is_empty()).collect();
    let mut valid = 0usize;
    let mut entries = 0usize;
    let mut missing = Vec::new();
    for dir in &dirs {
        if std::path::Path::new(dir).is_dir() {
            valid += 1;
            if let Ok(rd) = std::fs::read_dir(dir) {
                entries += rd.count();
            }
        } else {
            missing.push(dir.to_string());
        }
    }
    (dirs.len(), valid, missing, entries)
}

pub fn file_len(p: &std::path::Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}


pub fn is_script_cached(plugin_db_path: &std::path::Path, script_path: &str) -> bool {
    if !plugin_db_path.exists() {
        return false;
    }
    let cache = match crate::plugin_cache::PluginCache::open(plugin_db_path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    if let Some((mt_s, mt_ns)) = crate::plugin_cache::file_mtime(std::path::Path::new(script_path)) {
        cache.check(script_path, mt_s, mt_ns).is_some()
    } else {
        false
    }
}

/// Collect the report. `startup_ms` is `(executor_init_ms, compinit_drain_ms)` when the
/// caller timed a fresh executor (the binary does); a running shell cannot, so it passes
/// `None` and `startup_benchmark` is `null`.
pub fn gather(startup_ms: Option<(u64, u64)>) -> serde_json::Value {
    use serde_json::json;

    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "?".to_string());
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let environment = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "pid": std::process::id(),
        "cwd": cwd,
        "shell": std::env::var("SHELL").unwrap_or_else(|_| "?".to_string()),
        "cpus": cpus,
        "pool_size": cpus.clamp(2, 18),
    });

    let (path_total, path_valid, path_missing, path_cmds) =
        scan_dir_list(&std::env::var("PATH").unwrap_or_default());
    let (fpath_total, fpath_valid, fpath_missing, fpath_files) =
        scan_dir_list(&std::env::var("FPATH").unwrap_or_default());

    // Per docs/DESIGN_GOALS.md:13 and docs/DAEMON.md:226, the only shell cache
    // layer is rkyv-mmapped bytecode under `~/.zshrs/images/` with the
    // top-level `~/.zshrs/index.rkyv`. Hot lookups never hit SQLite.
    let zshrs_dir = dirs::home_dir()
        .map(|h| h.join(".zshrs"))
        .unwrap_or_else(|| PathBuf::from("/tmp/.zshrs"));
    let index_rkyv = zshrs_dir.join("index.rkyv");
    let images_dir = zshrs_dir.join("images");
    let mut shards: Vec<(String, u64)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&images_dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("rkyv") {
                let name = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                shards.push((name, entry.metadata().map(|m| m.len()).unwrap_or(0)));
            }
        }
    }
    shards.sort();
    let scripts = crate::script_cache::stats().map(|(count, bytes)| {
        json!({
            "entries": count,
            "bytes": bytes,
            "path": crate::script_cache::default_cache_path().display().to_string(),
        })
    });
    let autoload_count = crate::autoload_cache::entry_count();
    let caches = json!({
        "index": {
            "path": index_rkyv.display().to_string(),
            "present": index_rkyv.exists(),
            "bytes": file_len(&index_rkyv),
        },
        "images": {
            "path": images_dir.display().to_string(),
            "present": images_dir.is_dir(),
            "total_bytes": shards.iter().map(|(_, s)| *s).sum::<u64>(),
            "shards": shards.iter().map(|(n, s)| json!({"name": n, "bytes": s})).collect::<Vec<_>>(),
        },
        "scripts": scripts,
        "autoloads": (autoload_count > 0).then(|| json!({
            "functions": autoload_count,
            "path": crate::autoload_cache::default_cache_path().display().to_string(),
        })),
    });

    // The recorder shard is what startup replays, so ITS counts are the
    // shell's real state; the SQLite files are daemon-maintained copies.
    let recorded = crate::ext_builtins::recorded_state_counts().map(|(shard, counts)| {
        json!({
            "shard": shard,
            "counts": counts.iter().map(|(l, n)| json!({"label": l, "count": n})).collect::<Vec<_>>(),
        })
    });

    let compsys_path = crate::compsys::cache::default_cache_path();
    let mut sqlite = json!({
        "compsys": {
            "path": compsys_path.display().to_string(),
            "present": compsys_path.exists(),
            "bytes": file_len(&compsys_path),
            "completions": if compsys_path.exists() {
                CompsysCache::open(&compsys_path)
                    .ok()
                    .map(|c| crate::compsys::cache_entry_count(&c))
                    .unwrap_or(0)
            } else { 0 },
        },
    });
    let plugin_path = crate::plugin_cache::default_cache_path();
    #[cfg(feature = "daemon")]
    {
        let catalog = zshrs_daemon::paths::CachePaths::resolve()
            .ok()
            .map(|p| p.catalog_db);
        sqlite["plugins"] = json!(catalog.and_then(|p| zshrs_daemon::catalog::mirrored_plugin_count(&p)));
    }

    // History is a durable command record, not a cache.
    let hist_path = crate::history::HistoryEngine::db_path();
    let history = json!({
        "path": hist_path.display().to_string(),
        "present": hist_path.exists(),
        "bytes": file_len(&hist_path),
        "entries": if hist_path.exists() {
            crate::history::HistoryEngine::new().ok().and_then(|e| e.count().ok()).unwrap_or(0)
        } else { 0 },
    });

    let log_path = crate::log::log_path();
    let log = json!({
        "path": log_path.display().to_string(),
        "present": log_path.exists(),
        "bytes": file_len(&log_path),
        "lines": std::fs::read_to_string(&log_path).map(|s| s.lines().count()).unwrap_or(0),
    });

    let zdotdir = std::env::var("ZDOTDIR")
        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string()));
    let startup: Vec<serde_json::Value> = [
        crate::global_rc::global_rc_path("/etc/zshenv"),
        format!("{}/.zshenv", zdotdir),
        crate::global_rc::global_rc_path("/etc/zprofile"),
        format!("{}/.zprofile", zdotdir),
        crate::global_rc::global_rc_path("/etc/zshrc"),
        format!("{}/.zshrc", zdotdir),
        crate::global_rc::global_rc_path("/etc/zlogin"),
        format!("{}/.zlogin", zdotdir),
    ]
    .iter()
    .map(|path| {
        let p = std::path::Path::new(path);
        json!({
            "path": path,
            "present": p.exists(),
            "bytes": file_len(p),
            "cached": p.exists() && is_script_cached(&plugin_path, path),
        })
    })
    .collect();
    let recording = if crate::daemon_presence::recording_present() {
        json!({"present": true, "stale_rc": crate::daemon_presence::recording_staleness()})
    } else {
        json!({"present": false, "stale_rc": null})
    };

    let profiling = json!({
        "chrome_tracing": crate::log::profiling_enabled(),
        "flamegraph": crate::log::flamegraph_enabled(),
        "prometheus": crate::log::prometheus_enabled(),
        "zshrs_log": std::env::var("ZSHRS_LOG").ok(),
    });

    let startup_benchmark = startup_ms.map(|(init_ms, drain_ms)| {
        json!({
            "executor_init_ms": init_ms,
            "compinit_drain_ms": drain_ms,
            "total_ms": init_ms + drain_ms,
        })
    });

    let mut issues: Vec<String> = Vec::new();
    if !path_missing.is_empty() {
        issues.push(format!("{} PATH entries missing", path_missing.len()));
    }
    if !fpath_missing.is_empty() {
        issues.push(format!("{} FPATH entries missing", fpath_missing.len()));
    }
    if !hist_path.exists() {
        issues.push("no history database".to_string());
    }
    if !compsys_path.exists() {
        issues.push("no completion cache".to_string());
    }
    if startup_ms.is_some_and(|(init, drain)| init + drain > 100) {
        issues.push("startup > 100ms".to_string());
    }

    json!({
        "environment": environment,
        "path": {
            "total": path_total, "valid": path_valid,
            "missing": path_missing, "commands": path_cmds,
        },
        "fpath": {
            "total": fpath_total, "valid": fpath_valid,
            "missing": fpath_missing, "function_files": fpath_files,
        },
        "caches": caches,
        "recorded_state": recorded,
        "sqlite": sqlite,
        "history": history,
        "log": log,
        "startup_files": startup,
        "recording": recording,
        "profiling": profiling,
        "startup_benchmark": startup_benchmark,
        "issues": issues,
    })
}
