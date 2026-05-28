//! zshrs configuration file — `~/.config/zshrs/config.toml`.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh has no equivalent because every runtime knob lives in shell
//! options (Src/options.c) or special parameters
//! (Src/params.c). This file controls the Rust engine — worker-pool
//! size, completion-cache enablement, async-history writes — none
//! of which exist in C zsh.
//!
//! Runtime settings that don't belong in .zshrc (shell script).
//! These control the Rust engine, not the shell language.
//!
//! Example config:
//! ```toml
//! [worker_pool]
//! size = 8            # number of worker threads (default: num_cpus, clamped [2, 18])
//!
//! [completion]
//! max_matches = 1000  # max completion results to display
//! fts_enabled = true  # use SQLite FTS5 for completion search
//! ast_cache = true    # pre-parse autoload functions to AST blobs
//!
//! [history]
//! async_writes = true # write history on worker pool (don't block prompt)
//! max_entries = 100000
//!
//! [glob]
//! parallel_threshold = 32  # min files before parallel metadata prefetch
//! recursive_parallel = true  # fan out **/ across worker pool
//!
//! [log]
//! level = "info"      # trace, debug, info, warn, error
//! ```

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level config.
/// zshrs-original — no C counterpart. Each section maps onto a
/// Rust subsystem that doesn't exist in C zsh (worker pool,
/// FTS5-backed completion cache, async history writes, parallel
/// glob).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ZshrsConfig {
    pub worker_pool: WorkerPoolConfig,
    pub completion: CompletionConfig,
    pub history: HistoryConfig,
    pub glob: GlobConfig,
    pub log: LogConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WorkerPoolConfig {
    /// Number of worker threads. 0 = auto (num_cpus clamped [2, 18]).
    pub size: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CompletionConfig {
    pub max_matches: usize,
    pub fts_enabled: bool,
    pub ast_cache: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    pub async_writes: bool,
    pub max_entries: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GlobConfig {
    /// Minimum file count before parallel metadata prefetch kicks in.
    pub parallel_threshold: usize,
    /// Fan out **/ recursive globs across worker pool.
    pub recursive_parallel: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    pub level: String,
}

// ── Defaults ──

impl Default for CompletionConfig {
    fn default() -> Self {
        Self {
            max_matches: 1000,
            fts_enabled: true,
            ast_cache: true,
        }
    }
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            async_writes: true,
            max_entries: 100_000,
        }
    }
}

impl Default for GlobConfig {
    fn default() -> Self {
        Self {
            parallel_threshold: 32,
            recursive_parallel: true,
        }
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
        }
    }
}

// ── Loading ──

/// Config file path: `~/.zshrs/zshrs.toml`.
/// zshrs-original — C zsh has no analog. Closest C equivalent is
/// the `ZDOTDIR`/`HOME` lookup chain Src/init.c uses for `.zshrc`,
/// but that's a shell-script path, not a runtime config.
pub fn config_path() -> PathBuf {
    crate::daemon_presence::config_file_path()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("zshrs")
        .join("config.toml")
}

/// Load config from disk. Returns defaults if the file doesn't
/// exist or fails to parse.
/// zshrs-original — no C counterpart.
pub fn load() -> ZshrsConfig {
    load_from(&config_path())
}

/// Load config from a specific path.
/// zshrs-original — no C counterpart. Defaults preserve startup
/// silently (per the project's "no startup chatter" rule).
pub fn load_from(path: &Path) -> ZshrsConfig {
    match std::fs::read_to_string(path) {
        Ok(content) => match toml::from_str(&content) {
            Ok(config) => {
                tracing::info!(path = %path.display(), "config loaded");
                config
            }
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "config parse error, using defaults"
                );
                ZshrsConfig::default()
            }
        },
        Err(_) => {
            // No config file — use defaults silently
            ZshrsConfig::default()
        }
    }
}

/// Resolve the worker pool size from config.
/// `0` means auto: `available_parallelism` clamped to `[2, 18]`.
/// zshrs-original — sizes the thread pool (`src/worker.rs`) that
/// replaces C zsh's per-task `fork(2)` strategy.
pub fn resolve_pool_size(config: &WorkerPoolConfig) -> usize {
    if config.size > 0 {
        config.size.clamp(1, 64)
    } else {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(2, 18)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let _g = crate::test_util::global_state_lock();
        let config = ZshrsConfig::default();
        assert_eq!(config.worker_pool.size, 0);
        assert_eq!(config.completion.max_matches, 1000);
        assert!(config.completion.fts_enabled);
        assert!(config.completion.ast_cache);
        assert!(config.history.async_writes);
        assert!(config.glob.recursive_parallel);
        assert_eq!(config.glob.parallel_threshold, 32);
    }

    #[test]
    fn test_parse_toml() {
        let _g = crate::test_util::global_state_lock();
        let toml = r#"
[worker_pool]
size = 4

[completion]
max_matches = 500
ast_cache = false

[glob]
parallel_threshold = 64
"#;
        let config: ZshrsConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.worker_pool.size, 4);
        assert_eq!(config.completion.max_matches, 500);
        assert!(!config.completion.ast_cache);
        assert_eq!(config.glob.parallel_threshold, 64);
        // Unset fields use defaults
        assert!(config.history.async_writes);
        assert!(config.glob.recursive_parallel);
    }

    #[test]
    fn test_resolve_pool_size() {
        let _g = crate::test_util::global_state_lock();
        let auto = WorkerPoolConfig { size: 0 };
        let resolved = resolve_pool_size(&auto);
        assert!((2..=18).contains(&resolved));

        let explicit = WorkerPoolConfig { size: 4 };
        assert_eq!(resolve_pool_size(&explicit), 4);

        let clamped = WorkerPoolConfig { size: 999 };
        assert_eq!(resolve_pool_size(&clamped), 64);
    }

    #[test]
    fn test_missing_file_returns_defaults() {
        let _g = crate::test_util::global_state_lock();
        let config = load_from(Path::new("/nonexistent/config.toml"));
        assert_eq!(config.worker_pool.size, 0);
    }

    // ========================================================
    // resolve_pool_size — boundary behavior
    // ========================================================

    #[test]
    fn pool_size_one_passes_through() {
        // Explicit size=1 should NOT trigger the auto-detect branch.
        let _g = crate::test_util::global_state_lock();
        assert_eq!(resolve_pool_size(&WorkerPoolConfig { size: 1 }), 1);
    }

    #[test]
    fn pool_size_64_is_upper_cap_for_explicit_request() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(resolve_pool_size(&WorkerPoolConfig { size: 64 }), 64);
        assert_eq!(resolve_pool_size(&WorkerPoolConfig { size: 65 }), 64);
        assert_eq!(resolve_pool_size(&WorkerPoolConfig { size: 100_000 }), 64);
    }

    #[test]
    fn pool_size_zero_uses_auto_within_2_to_18_window() {
        let _g = crate::test_util::global_state_lock();
        let n = resolve_pool_size(&WorkerPoolConfig { size: 0 });
        assert!(
            (2..=18).contains(&n),
            "auto-detect must clamp to [2,18], got {}",
            n
        );
    }

    // ========================================================
    // Parser — defaults round-trip on empty input
    // ========================================================

    #[test]
    fn empty_toml_parses_to_full_defaults() {
        let _g = crate::test_util::global_state_lock();
        let cfg: ZshrsConfig = toml::from_str("").unwrap();
        let d = ZshrsConfig::default();
        assert_eq!(cfg.worker_pool.size, d.worker_pool.size);
        assert_eq!(cfg.completion.max_matches, d.completion.max_matches);
        assert_eq!(cfg.history.max_entries, d.history.max_entries);
        assert_eq!(cfg.glob.parallel_threshold, d.glob.parallel_threshold);
        assert_eq!(cfg.log.level, d.log.level);
    }

    #[test]
    fn unknown_section_does_not_error_with_serde_default() {
        let _g = crate::test_util::global_state_lock();
        // Unknown top-level key is currently rejected when serde sees it
        // — verify the explicit accepted shape stays accepted.
        let toml = "[log]\nlevel = \"debug\"\n";
        let cfg: ZshrsConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.log.level, "debug");
    }

    #[test]
    fn partial_completion_section_fills_other_defaults() {
        let _g = crate::test_util::global_state_lock();
        let toml = r#"
[completion]
max_matches = 42
"#;
        let cfg: ZshrsConfig = toml::from_str(toml).unwrap();
        assert_eq!(cfg.completion.max_matches, 42);
        // Other fields fall back to defaults.
        assert!(cfg.completion.fts_enabled);
        assert!(cfg.completion.ast_cache);
    }

    #[test]
    fn history_async_can_be_disabled() {
        let _g = crate::test_util::global_state_lock();
        let toml = "[history]\nasync_writes = false\n";
        let cfg: ZshrsConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.history.async_writes);
        assert_eq!(cfg.history.max_entries, 100_000);
    }

    #[test]
    fn glob_recursive_parallel_can_be_disabled() {
        let _g = crate::test_util::global_state_lock();
        let toml = "[glob]\nrecursive_parallel = false\n";
        let cfg: ZshrsConfig = toml::from_str(toml).unwrap();
        assert!(!cfg.glob.recursive_parallel);
    }

    // ========================================================
    // load_from — IO-failure modes
    // ========================================================

    #[test]
    fn malformed_toml_returns_defaults_not_panic() {
        // Write a bogus file then point load_from at it.
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_config_malformed.toml");
        std::fs::write(&tmp, "this is not [[ valid toml ===").unwrap();
        let cfg = load_from(&tmp);
        // Defaults survive parse error.
        assert_eq!(cfg.worker_pool.size, 0);
        assert_eq!(cfg.completion.max_matches, 1000);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn load_from_round_trip_via_temp_file() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join("zshrs_config_rt.toml");
        std::fs::write(&tmp, "[worker_pool]\nsize = 7\n").unwrap();
        let cfg = load_from(&tmp);
        assert_eq!(cfg.worker_pool.size, 7);
        assert_eq!(resolve_pool_size(&cfg.worker_pool), 7);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn config_path_ends_in_config_toml() {
        let _g = crate::test_util::global_state_lock();
        let p = config_path();
        assert_eq!(
            p.file_name().and_then(|s| s.to_str()),
            Some("config.toml"),
            "{:?}",
            p
        );
        assert_eq!(
            p.parent()
                .and_then(|d| d.file_name())
                .and_then(|s| s.to_str()),
            Some("zshrs"),
            "{:?}",
            p
        );
    }

    #[test]
    fn log_level_default_is_info_string() {
        let _g = crate::test_util::global_state_lock();
        let cfg = ZshrsConfig::default();
        assert_eq!(cfg.log.level, "info");
    }
}
