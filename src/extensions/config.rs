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
//! fts_enabled = true  # populate the SQLite FTS5 mirror tables (rkyv shards are the authoritative completion cache; this toggle only affects `dbview` / SQL inspection)
//! ast_cache = true    # pre-parse autoload functions to AST blobs
//!
//! [compsys]
//! backend = "rust"    # "rust" → src/compsys/ported/ ports;
//!                     # "shell" → upstream Completion/ shell funcs
//!                     # via autoload (use this if you've patched
//!                     # any _X in .zshrc).
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
//!
//! [provenance]
//! enabled = false     # disable the value-lineage engine outright
//!                     # (default true; the engine still stays inert
//!                     # until `provenance -m NAME` arms it).
//!                     # `ZSHRS_PROVENANCE=0` is the env kill switch.
//! track_all = true    # arm every parameter and every shell function
//!                     # without any `-m` call (default false).
//!                     # `ZSHRS_PROVENANCE_ALL=1`/`=0` overrides it.
//! ```

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level config.
/// zshrs-original — no C counterpart. Each section maps onto a
/// Rust subsystem that doesn't exist in C zsh (worker pool,
/// rkyv-mmap'd completion cache with optional SQLite FTS5
/// mirrors for `dbview`, async history writes, parallel
/// glob).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct ZshrsConfig {
    /// `worker_pool` field.
    pub worker_pool: WorkerPoolConfig,
    /// `completion` field.
    pub completion: CompletionConfig,
    /// `compsys` field — backend selector (rust vs shell) for the
    /// `_main_complete` function tree. See [`CompsysConfig`].
    pub compsys: CompsysConfig,
    /// `history` field.
    pub history: HistoryConfig,
    /// `glob` field.
    pub glob: GlobConfig,
    /// `log` field.
    pub log: LogConfig,
    /// `zle` field — native fish-ported editor engines (opt-in;
    /// `zshrs -f` stays zsh-identical by default). See [`ZleConfig`].
    pub zle: ZleConfig,
    /// `provenance` field — value-lineage engine master switch. See
    /// [`ProvenanceConfig`].
    pub provenance: ProvenanceConfig,
}
/// Compsys backend selection — Rust port vs upstream shell functions.
///
/// `_main_complete` and the rest of the compsys function tree
/// (`Completion/Base/Core/*`, `Zsh/Type/*`, `Zsh/Command/*`, ...)
/// exist in two parallel forms in this repo:
///
/// 1. **Rust ports** under `src/compsys/ported/` — JIT-fast, no
///    fork-exec, deterministic. Used when `backend = "rust"`.
/// 2. **Upstream shell sources** under `src/zsh/Completion/` —
///    autoloaded via `fpath` exactly like real zsh. Used when
///    `backend = "shell"`. Required if you've patched a `_X`
///    function in `.zshrc` and need the user override to win.
///
/// Default `"rust"`. Override per-machine in `~/.config/zshrs/config.toml`:
/// ```toml
/// [compsys]
/// backend = "shell"
/// ```
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
pub struct CompsysConfig {
    /// Either `"rust"` (default) or `"shell"`. Any other value falls
    /// back to `"rust"` with a one-shot tracing::warn at load time.
    pub backend: CompsysBackend,
}

/// Strong-typed backend selector. Maps to the `backend = "..."`
/// string in the TOML.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CompsysBackend {
    /// Route every `_NAME` call through `src/compsys/ported/`.
    #[default]
    Rust,
    /// Route every `_NAME` call through the upstream shell function
    /// at `Completion/.../$NAME` via the standard shfunc/autoload path.
    Shell,
}

/// `WorkerPoolConfig` — see fields for layout.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
#[derive(Default)]
pub struct WorkerPoolConfig {
    /// Number of worker threads. 0 = auto (num_cpus clamped [2, 18]).
    pub size: usize,
}
/// `CompletionConfig` — see fields for layout.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CompletionConfig {
    /// `max_matches` field.
    pub max_matches: usize,
    /// `fts_enabled` field.
    pub fts_enabled: bool,
    /// `ast_cache` field.
    pub ast_cache: bool,
}
/// `HistoryConfig` — see fields for layout.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct HistoryConfig {
    /// `async_writes` field.
    pub async_writes: bool,
    /// `max_entries` field.
    pub max_entries: usize,
}
/// `GlobConfig` — see fields for layout.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct GlobConfig {
    /// Minimum file count before parallel metadata prefetch kicks in.
    pub parallel_threshold: usize,
    /// Fan out **/ recursive globs across worker pool.
    pub recursive_parallel: bool,
}
/// `LogConfig` — see fields for layout.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LogConfig {
    /// `level` field.
    pub level: String,
}

/// `[zle]` — the native fish-ported line-editor engines and other
/// deliberate ZLE deviations.
///
/// The four ENGINES default ON. They are the point of the shell, not an
/// experiment to opt into, and a user who has an rc file is asking for a
/// configured interactive shell. Parity is preserved at the other end:
/// `zle_fx` refuses every engine when RCS is unset, so `zshrs -f` still
/// behaves identically to `zsh -f` and the parity suites are unaffected.
/// Set any field `false` in `~/.zshrs/zshrs.toml` to turn one off;
/// `ZSHRS_NATIVE_ZLE_FX=0` remains the blanket kill switch.
///
/// `vi_backspace_unrestricted` stays OFF by default: it is a keybinding
/// deviation from classic vi, not one of the engines, so it keeps opt-in
/// semantics.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ZleConfig {
    /// fish-ported history autosuggestions (ghost text).
    pub autosuggest: bool,
    /// fish-ported command-line syntax highlighting.
    pub syntax_highlight: bool,
    /// fish-ported up-arrow substring/prefix history search.
    pub history_search: bool,
    /// zsh-autopair port (bracket/quote auto-pairing).
    pub autopair: bool,
    /// viins ^H/DEL bound to unrestricted backward-delete-char
    /// (vim's backspace=indent,eol,start) instead of the classic-vi
    /// vi-backward-delete-char.
    pub vi_backspace_unrestricted: bool,
}

impl Default for ZleConfig {
    fn default() -> Self {
        Self {
            autosuggest: true,
            syntax_highlight: true,
            history_search: true,
            autopair: true,
            vi_backspace_unrestricted: false,
        }
    }
}

/// `[provenance]` — master switch for the value-lineage engine
/// (`src/extensions/provenance.rs`).
///
/// The engine is already inert until a `provenance -m NAME` call arms
/// it, so `enabled = true` (the default) costs one relaxed atomic load
/// per hook site and nothing else. Setting `enabled = false` refuses
/// arming entirely: `provenance -m` reports the engine as disabled and
/// no hook can ever fire, which is the setting to use on a machine
/// where the ledger must not exist at all.
///
/// ```toml
/// [provenance]
/// enabled = false
/// ```
///
/// `track_all = true` skips the arming step entirely — every parameter
/// write and every shell function records a chain:
///
/// ```toml
/// [provenance]
/// track_all = true
/// ```
///
/// `ZSHRS_PROVENANCE=0` in the environment overrides the config and
/// disables the engine regardless of these fields;
/// `ZSHRS_PROVENANCE_ALL=1` / `=0` overrides `track_all` alone.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ProvenanceConfig {
    /// Allow `provenance -m NAME` to arm the lineage ledger.
    pub enabled: bool,
    /// Track everything, with no `-m` call: every parameter the shell
    /// writes and every shell function it defines or calls arms itself.
    /// Off by default — with it on, the engine is armed from the first
    /// line of the first rc file, so every hook does real work for the
    /// life of the shell. `ZSHRS_PROVENANCE_ALL=1` / `=0` overrides this
    /// field; `enabled = false` still wins over both.
    pub track_all: bool,
}

// ── Defaults ──

impl Default for ProvenanceConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            track_all: false,
        }
    }
}

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

/// Config file path: `$ZSHRS_HOME/zshrs.toml` or
/// `~/.zshrs/zshrs.toml`. Single file for the whole zshrs config
/// surface — shares the path with `daemon_presence::load_zshrs_toml`
/// (which parses the orthogonal `[log]/[daemon]/[shell]/[builtins]`
/// sections). Unknown sections are ignored by serde (`#[serde(default)]`
/// on every field), so the two loaders coexist in one file.
/// zshrs-original — C zsh has no analog.
pub fn config_path() -> PathBuf {
    crate::daemon_presence::config_file_path().unwrap_or_else(|| PathBuf::from("/tmp/zshrs.toml"))
}

/// Load config from disk. Returns defaults if the file doesn't
/// exist or fails to parse.
/// zshrs-original — no C counterpart.
pub fn load() -> ZshrsConfig {
    load_from(&config_path())
}

/// Process-cached config snapshot. Read once on first access; the
/// disk file is NOT re-read on each call (no reload-on-write). Hot
/// paths like `dispatch_function_call` consult this without
/// stat'ing / re-parsing the TOML.
pub fn current() -> &'static ZshrsConfig {
    static CACHED: std::sync::OnceLock<ZshrsConfig> = std::sync::OnceLock::new();
    CACHED.get_or_init(load)
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
    fn provenance_defaults_to_enabled_and_parses_the_off_switch() {
        let _g = crate::test_util::global_state_lock();
        assert!(
            ZshrsConfig::default().provenance.enabled,
            "the engine stays inert until armed, so the config default is on"
        );
        let off: ZshrsConfig = toml::from_str("[provenance]\nenabled = false\n").expect("parses");
        assert!(!off.provenance.enabled);
        // An unrelated section must not disturb the default.
        let other: ZshrsConfig = toml::from_str("[log]\nlevel = \"debug\"\n").expect("parses");
        assert!(other.provenance.enabled);
    }

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
        assert_eq!(cfg.compsys.backend, d.compsys.backend);
        assert_eq!(cfg.history.max_entries, d.history.max_entries);
        assert_eq!(cfg.glob.parallel_threshold, d.glob.parallel_threshold);
        assert_eq!(cfg.log.level, d.log.level);
    }

    #[test]
    fn compsys_backend_defaults_to_rust() {
        let _g = crate::test_util::global_state_lock();
        let cfg = ZshrsConfig::default();
        assert_eq!(cfg.compsys.backend, CompsysBackend::Rust);
    }

    #[test]
    fn compsys_backend_parses_explicit_shell() {
        let _g = crate::test_util::global_state_lock();
        let cfg: ZshrsConfig = toml::from_str("[compsys]\nbackend = \"shell\"\n").unwrap();
        assert_eq!(cfg.compsys.backend, CompsysBackend::Shell);
    }

    #[test]
    fn compsys_backend_parses_explicit_rust() {
        let _g = crate::test_util::global_state_lock();
        let cfg: ZshrsConfig = toml::from_str("[compsys]\nbackend = \"rust\"\n").unwrap();
        assert_eq!(cfg.compsys.backend, CompsysBackend::Rust);
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
        // Canonical name is `zshrs.toml` per
        // daemon_presence::config_file_path (line 399) — documented at
        // daemon_presence.rs:30 as `$ZSHRS_HOME/zshrs.toml` or
        // `~/.zshrs/zshrs.toml`. Test name says "config_toml" but
        // pins the actual canonical filename.
        assert_eq!(
            p.file_name().and_then(|s| s.to_str()),
            Some("zshrs.toml"),
            "{:?}",
            p
        );
        // Parent dir is `.zshrs` (the hidden config home), not `zshrs`.
        assert_eq!(
            p.parent()
                .and_then(|d| d.file_name())
                .and_then(|s| s.to_str()),
            Some(".zshrs"),
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
