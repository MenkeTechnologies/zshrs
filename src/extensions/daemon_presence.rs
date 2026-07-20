//! Daemon-presence detection + per-user config knob.
//!
//! **zshrs-original infrastructure — no C source counterpart.** C
//! zsh has no daemon process. Shell state lives in the running
//! interpreter and re-initializes on every launch. zshrs ships
//! `zshrs-daemon` as a separate binary that holds canonical
//! parameter/option/function state in shared memory; the shell
//! probes its socket at startup and decides whether to use the
//! cached state or fall back to vanilla full-init mode.
//!
//! Per the `zshrs ↔ zshrs-daemon = independent binaries` rule
//! (docs/DAEMON.md "Daemon lifecycle"), the shell does NOT spawn the
//! daemon. It probes once at startup and runs in one of three modes:
//!
//! | Mode             | Trigger                                       |
//! |------------------|-----------------------------------------------|
//! | DaemonPresent    | socket alive, daemon answered handshake       |
//! | DaemonAbsent     | no socket / connect refused; degraded vanilla |
//! | DaemonDisabled   | user set `[daemon] enabled = "off"` in config |
//!
//! Without the daemon: zshrs runs as a Rust-fast vanilla zsh — no
//! cache, no canonical state, every config re-evaluated per shell
//! launch ("rebuilding your house every morning"). With the daemon:
//! zshrs uses the cached canonical state for fast cold-start.
//!
//! The probe is single-shot at startup. Builtins / hooks check
//! `is_present()` before calling into the daemon client; on absent,
//! they noop or fall back to source-interp behavior.
//!
//! Config (`$ZSHRS_HOME/zshrs.toml` or `~/.zshrs/zshrs.toml`, all optional):
//!
//! ```toml
//! [daemon]
//! # "auto" (default) = probe at startup, use if alive
//! # "off"            = never probe; pure vanilla zsh mode
//! # "require"        = probe and warn if absent (no spawn either way)
//! enabled = "auto"
//!
//! [shell]
//! # "off"  (default) = always source .zshenv/.zprofile/.zshrc/.zlogin
//! # "auto"           = if daemon is present + has zshrs rows, skip
//! #                    every dotfile and apply canonical state
//! #                    from the daemon instead. ~10ms cold-start.
//! # "on"             = always skip dotfiles when the daemon is up;
//! #                    don't even check for zshrs rows. Strict mode.
//! skip_configs = "off"
//!
//! # Optional: zsh script sourced once after dotfiles / canonical_apply,
//! # before compsys + first prompt (omit key for default: none).
//! # Respects `-f` / `--no-rcs` (not sourced).
//! # startup_config = "/path/to/init.zsh"
//! ```
//!
//! Lives in `~/.zshrs/` alongside everything else (rkyv shards,
//! catalog.db, daemon.sock, zshrs-daemon.toml, log, …) — single
//! directory rule for all zshrs files. Survives OS cache eviction
//! (this is NOT cache-semantic state). `rm -rf ~/.zshrs/` is the
//! one-verb total reset.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

/// Daemon-presence probe result.
/// zshrs-original — no C counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Mode {
    /// Probe hasn't run yet.
    Unknown = 0,
    /// Socket connected; assume daemon is alive.
    Present = 1,
    /// Probe ran; daemon was not reachable. Shell runs in vanilla mode.
    Absent = 2,
    /// User opted out via `[daemon] enabled = "off"`. No probe attempted.
    Disabled = 3,
}

impl Mode {
    #[inline]
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Present,
            2 => Self::Absent,
            3 => Self::Disabled,
            _ => Self::Unknown,
        }
    }
}

static STATE: AtomicU8 = AtomicU8::new(Mode::Unknown as u8);

/// What the user said in `[daemon].enabled` (or the auto default).
/// zshrs-original — no C counterpart. C zsh has no daemon to enable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSetting {
    /// Probe at startup; use the daemon if alive (default).
    Auto,
    /// Skip the probe entirely; never talk to the daemon.
    Off,
    /// Probe at startup; if the daemon isn't alive, log a warning
    /// (still doesn't spawn — that's the user's responsibility).
    Require,
}

/// What the user said in `[shell].skip_configs`.
/// Explicit discriminants pin the AtomicU8 round-trip in
/// `skip_configs_setting()`.
/// zshrs-original — no C counterpart. C zsh always sources every
/// startup file (Src/init.c `source_home_file()` chain); the
/// canonical-state path that lets us skip those is unique to zshrs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum SkipConfigs {
    /// Always source dotfiles (legacy / vanilla zsh behavior). Default.
    Off = 0,
    /// Skip dotfiles iff daemon is present AND has zshrs canonical
    /// rows. Falls back to dotfile sourcing otherwise. The recommended
    /// setting once the recorder has populated canonical state.
    Auto = 1,
    /// Always skip dotfiles when the daemon is up; don't bother
    /// checking for zshrs rows. Strict mode for users who know their
    /// daemon is fully populated.
    On = 2,
}

impl ConfigSetting {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "auto" | "" => Some(Self::Auto),
            "off" | "false" | "no" | "0" => Some(Self::Off),
            "require" | "on" | "true" | "yes" | "1" => Some(Self::Require),
            _ => None,
        }
    }
}

impl SkipConfigs {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "off" | "false" | "no" | "0" | "" => Some(Self::Off),
            "auto" => Some(Self::Auto),
            "on" | "true" | "yes" | "1" => Some(Self::On),
            _ => None,
        }
    }
}

/// Knobs from `~/.zshrs/zshrs.toml`. Missing file / section
/// / key returns the safe defaults (`daemon=auto`,
/// `skip_configs=off`, no `startup_config`). Unrecognized values
/// fall back with a log warning.
/// zshrs-original — no C counterpart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// `daemon` field.
    pub daemon: ConfigSetting,
    /// `skip_configs` field.
    pub skip_configs: SkipConfigs,
    /// Absolute or `~`-prefixed path to a zsh script sourced after
    /// normal startup when set. `None` if the key is absent or empty.
    pub startup_config: Option<PathBuf>,
    /// `[builtins].coreutils_shadows` — whether to use in-process
    /// coreutils shadow builtins (`cat`/`head`/`tail`/`sort`/`cut`/
    /// `tr`/`find`/`cp`/...) for the fork-elimination speedup, or
    /// fall through to the real `/bin/X` binaries.
    ///
    /// **Default `off`** — old scripts run against the canonical
    /// system coreutils out of the box, no edge-case divergence
    /// surprises. Opt IN to the speedup with the following stanza
    /// in `~/.zshrs/zshrs.toml`, or via `ZSHRS_COREUTILS_SHADOWS=1`
    /// env for one-off testing:
    ///
    /// ```toml
    /// [builtins]
    /// coreutils_shadows = "on"
    /// ```
    pub coreutils_shadows: bool,
}

/// Resolved `[shell].startup_config` from the last `probe()` (same
/// process). `None` when unset, omitted, or probe has not run.
static STARTUP_CONFIG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn resolve_startup_config_path(raw: &str) -> PathBuf {
    let s = raw.trim();
    if let Some(rest) = s.strip_prefix("~/") {
        return dirs::home_dir()
            .map(|h| h.join(rest))
            .unwrap_or_else(|| PathBuf::from(s));
    }
    if s == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(s));
    }
    PathBuf::from(s)
}
/// `read_config_full` — see implementation.
pub fn read_config_full() -> Config {
    let defaults = Config {
        daemon: ConfigSetting::Auto,
        skip_configs: SkipConfigs::Off,
        startup_config: None,
        // OFF by default — old scripts run against system /bin/X
        // out of the box. Opt-in to the in-process speedup via
        // [builtins].coreutils_shadows = on.
        coreutils_shadows: false,
    };
    let path = match config_file_path() {
        Some(p) => p,
        None => return defaults,
    };
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return defaults,
    };
    let parsed = match body.parse::<toml::Table>() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "zshrs.toml: parse failed; using defaults");
            return defaults;
        }
    };
    let daemon = parsed
        .get("daemon")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("enabled"))
        .and_then(|v| v.as_str())
        .map(|s| {
            ConfigSetting::parse(s).unwrap_or_else(|| {
                tracing::warn!(
                    value = s,
                    "zshrs.toml: [daemon].enabled invalid; using auto"
                );
                ConfigSetting::Auto
            })
        })
        .unwrap_or(ConfigSetting::Auto);
    let skip_configs = parsed
        .get("shell")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("skip_configs"))
        .and_then(|v| v.as_str())
        .map(|s| {
            SkipConfigs::parse(s).unwrap_or_else(|| {
                tracing::warn!(
                    value = s,
                    "zshrs.toml: [shell].skip_configs invalid; using off"
                );
                SkipConfigs::Off
            })
        })
        .unwrap_or(SkipConfigs::Off);
    let startup_config = parsed
        .get("shell")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("startup_config"))
        .and_then(|v| {
            if let Some(s) = v.as_str() {
                let t = s.trim();
                if t.is_empty() {
                    None
                } else {
                    Some(resolve_startup_config_path(t))
                }
            } else {
                tracing::warn!("zshrs.toml: [shell].startup_config must be a string; ignoring");
                None
            }
        });
    // `[builtins].coreutils_shadows` — accept booleans (`true`/`false`)
    // OR string aliases (`on`/`off`/`yes`/`no`). String-aliased
    // matches how `[daemon].enabled` accepts `auto`/`off`/`require`
    // so the file stays toml-string-uniform.
    let coreutils_shadows = parsed
        .get("builtins")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("coreutils_shadows"))
        .map(|v| match v {
            toml::Value::Boolean(b) => *b,
            toml::Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "off" | "false" | "no" | "0" => false,
                "on" | "true" | "yes" | "1" => true,
                other => {
                    tracing::warn!(
                        value = other,
                        "zshrs.toml: [builtins].coreutils_shadows invalid; using off"
                    );
                    false
                }
            },
            _ => {
                tracing::warn!(
                    "zshrs.toml: [builtins].coreutils_shadows must be bool or string; using off"
                );
                false
            }
        })
        .unwrap_or(false);
    Config {
        daemon,
        skip_configs,
        startup_config,
        coreutils_shadows,
    }
}

/// Snapshot of `[builtins].coreutils_shadows` cached after the first
/// `coreutils_shadows_enabled()` lookup. `OnceLock` lazy so the toml
/// read is one-shot, then every subsequent dispatch through
/// `reg_overridable!` becomes a single atomic load.
static CORE_SHADOWS_CACHE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

/// Hot-path query for whether to dispatch the in-process coreutils
/// shadow (`cat`/`cp`/`find`/`sort`/etc) or fall through to the
/// system binary. Read by the `reg_overridable!` macro in
/// `fusevm_bridge.rs` on every shadowed-builtin invocation. O(1)
/// after first call.
pub fn coreutils_shadows_enabled() -> bool {
    *CORE_SHADOWS_CACHE.get_or_init(|| {
        // `--zsh` strict-parity mode NEVER shadows externals — `head`
        // / `cat` / `sort` must be the system binaries with the
        // system binaries' exact flag handling and diagnostics
        // (`head -0` is an error on BSD head, the shadow accepted
        // it). Unconditional: parity floor outranks the toml knob
        // and the env override.
        if crate::IS_ZSH_MODE.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        // Env override wins for one-off testing:
        //   ZSHRS_COREUTILS_SHADOWS=1 — force ON
        //   ZSHRS_COREUTILS_SHADOWS=0 — force OFF
        // Absent → fall through to toml / default-off.
        if let Ok(v) = std::env::var("ZSHRS_COREUTILS_SHADOWS") {
            let v = v.trim().to_ascii_lowercase();
            if matches!(v.as_str(), "1" | "on" | "true" | "yes") {
                return true;
            }
            if matches!(v.as_str(), "0" | "off" | "false" | "no" | "") {
                return false;
            }
        }
        read_config_full().coreutils_shadows
    })
}

/// Path from `[shell].startup_config` captured during `probe()`.
#[inline]
pub fn startup_config_path() -> Option<PathBuf> {
    STARTUP_CONFIG_PATH.lock().ok().and_then(|g| g.clone())
}

/// Back-compat wrapper kept for callers that only need the daemon knob.
pub fn read_config() -> ConfigSetting {
    read_config_full().daemon
}

/// `[log] level` from `~/.zshrs/zshrs.toml` for the SHELL side
/// (zsh::log + zshrs-recorder). Same precedence model the daemon uses
/// for its own side: `$ZSHRS_LOG` env wins, then this directive, then
/// `"info"`. Caller hands the returned string straight to
/// `EnvFilter::try_new`. Errors / missing file / missing key all
/// resolve to "info" silently — a malformed directive produces a
/// useful "bad directive" message at the EnvFilter parse layer
/// instead.
pub fn read_log_directive() -> String {
    const DEFAULT: &str = "info";
    let path = match config_file_path() {
        Some(p) => p,
        None => return DEFAULT.into(),
    };
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return DEFAULT.into(),
    };
    let parsed = match body.parse::<toml::Table>() {
        Ok(t) => t,
        Err(_) => return DEFAULT.into(),
    };
    parsed
        .get("log")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("level"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| DEFAULT.into())
}

/// Cached `[shell].skip_configs` value. Set by `probe()`; read by the
/// shell-init path before sourcing dotfiles.
static SKIP_CONFIGS: AtomicU8 = AtomicU8::new(0);

/// Have we confirmed the daemon has zshrs canonical rows? Set during
/// the same probe pass so the `skip_configs` decision is one atomic
/// load on the hot path.
static SHOULD_SKIP_CONFIGS: AtomicU8 = AtomicU8::new(0);

/// Resolve `$ZSHRS_HOME/zshrs.toml` or `~/.zshrs/zshrs.toml`.
/// Single-directory rule: every zshrs file lives under one root.
/// Returns None if neither $ZSHRS_HOME nor $HOME is set, which is
/// rare enough to treat as "no config file".
pub fn config_file_path() -> Option<std::path::PathBuf> {
    let root = if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        std::path::PathBuf::from(custom)
    } else {
        std::path::PathBuf::from(std::env::var_os("HOME")?).join(".zshrs")
    };
    Some(root.join("zshrs.toml"))
}

/// Cheap probe — connect-only, no handshake — does the daemon
/// socket answer?
/// zshrs-original — no C counterpart. Sets the global state so
/// subsequent `is_present()` checks are O(1) atomic loads.
///
/// Honors `[daemon].enabled`:
///   - `Off` → state = Disabled, no probe
///   - `Auto` / `Require` → probe; state = Present or Absent
///
/// `Require` additionally logs a warning if the daemon isn't alive —
/// signal to the user that they configured the shell to expect a
/// daemon but didn't actually start one.
pub fn probe() -> Mode {
    let cfg = read_config_full();
    if let Ok(mut slot) = STARTUP_CONFIG_PATH.lock() {
        *slot = cfg.startup_config.clone();
    }
    SKIP_CONFIGS.store(cfg.skip_configs as u8, Ordering::Relaxed);

    match cfg.daemon {
        ConfigSetting::Off => {
            tracing::info!("daemon: disabled in config ([daemon] enabled = \"off\")");
            STATE.store(Mode::Disabled as u8, Ordering::Relaxed);
            // Disabled daemon → no skip; dotfiles always source.
            SHOULD_SKIP_CONFIGS.store(0, Ordering::Relaxed);
            return Mode::Disabled;
        }
        ConfigSetting::Auto | ConfigSetting::Require => {}
    }

    let alive = probe_socket();
    let mode = if alive { Mode::Present } else { Mode::Absent };
    STATE.store(mode as u8, Ordering::Relaxed);

    if alive {
        tracing::info!("daemon: present (socket reachable)");
    } else {
        match cfg.daemon {
            ConfigSetting::Require => {
                tracing::warn!(
                    "daemon: absent — config requires it but socket is not reachable. \
                     Start it via `zshrs-daemon`, `systemctl --user start zshrs-daemon`, \
                     `launchctl load ~/Library/LaunchAgents/com.menketechnologies.zshrs-daemon.plist`, \
                     or `brew services start zshrs`. Falling back to vanilla mode."
                );
            }
            _ => {
                tracing::info!(
                    "daemon: absent (socket not reachable) — running in vanilla zsh mode"
                );
            }
        }
    }

    // Resolve [shell].skip_configs against daemon presence + zshrs-row
    // availability. Three settings collapse to a yes/no decision:
    //   Off  → never skip
    //   On   → skip iff daemon Present (don't even check rows)
    //   Auto → skip iff daemon Present AND has zshrs canonical rows
    let should_skip = match cfg.skip_configs {
        SkipConfigs::Off => false,
        SkipConfigs::On => mode == Mode::Present,
        SkipConfigs::Auto => mode == Mode::Present && daemon_has_zshrs_rows(),
    };
    SHOULD_SKIP_CONFIGS.store(if should_skip { 1 } else { 0 }, Ordering::Relaxed);
    if should_skip {
        tracing::info!(
            "shell: skip_configs active — bypassing /etc/zshenv + ~/.{{zshenv,zprofile,zshrc,zlogin}} and \
             applying canonical state from daemon"
        );
    } else if cfg.skip_configs != SkipConfigs::Off {
        tracing::info!(
            mode = ?cfg.skip_configs,
            daemon = ?mode,
            "shell: skip_configs configured but conditions not met — sourcing dotfiles normally"
        );
    }
    mode
}

/// Cheap probe: does the daemon have a recorder shard on disk for
/// shell_id "zshrs"? A `*-recorder.rkyv` file in `~/.zshrs/images/`
/// means the recorder ran at least once and we have canonical state to
/// apply. **No IPC** — this is a directory listing + filename match,
/// because the shell cold-start path can afford zero IPC roundtrips
/// (the architecture's whole speed thesis).
///
/// Returns false on any I/O error → caller falls through to vanilla
/// `source_startup_files()`.
/// Is the recorded environment STALE relative to the user's rc files?
///
/// "Configs are ignored after the recorder" is the speed thesis, but that
/// makes an edited `.zshrc` invisible until the user re-runs the recorder.
/// This is the cheap staleness oracle: compare the newest rc-file mtime
/// against the newest `*-recorder.rkyv` shard mtime. Returns the path of the
/// offending rc file when any rc is newer than the recording (i.e. the user
/// edited config and hasn't re-recorded), else `None`.
///
/// NO IPC and NO re-sourcing — a directory listing + a handful of `stat`s,
/// so it stays within the cold-start budget. The caller decides the channel;
/// per the no-startup-chatter rule this is logged (never printed to the tty)
/// and surfaced in `--doctor`.
#[cfg(feature = "daemon")]
pub fn recording_staleness() -> Option<String> {
    use std::time::SystemTime;
    let paths = crate::daemon::paths::CachePaths::resolve().ok()?;
    // Newest recorder shard mtime.
    let mut newest_shard: Option<SystemTime> = None;
    for entry in std::fs::read_dir(&paths.images).ok()?.flatten() {
        if entry
            .file_name()
            .to_str()
            .map(|s| s.ends_with("-recorder.rkyv"))
            .unwrap_or(false)
        {
            if let Ok(m) = entry.metadata().and_then(|md| md.modified()) {
                if newest_shard.map(|n| m > n).unwrap_or(true) {
                    newest_shard = Some(m);
                }
            }
        }
    }
    let shard_mtime = newest_shard?; // no recording yet → not "stale", just absent
                                     // Any rc file newer than the recording?
    let zdotdir = std::env::var("ZDOTDIR")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let rc_set = [
        "/etc/zshenv".to_string(),
        format!("{}/.zshenv", zdotdir),
        "/etc/zprofile".to_string(),
        format!("{}/.zprofile", zdotdir),
        "/etc/zshrc".to_string(),
        format!("{}/.zshrc", zdotdir),
        "/etc/zlogin".to_string(),
        format!("{}/.zlogin", zdotdir),
    ];
    let mut newest_rc: Option<(SystemTime, String)> = None;
    for path in &rc_set {
        if let Ok(m) = std::fs::metadata(path).and_then(|md| md.modified()) {
            if newest_rc.as_ref().map(|(n, _)| m > *n).unwrap_or(true) {
                newest_rc = Some((m, path.clone()));
            }
        }
    }
    match newest_rc {
        Some((rc_mtime, rc_path)) if rc_mtime > shard_mtime => Some(rc_path),
        _ => None,
    }
}

/// Non-daemon builds have no recorder shard — nothing to be stale against.
#[cfg(not(feature = "daemon"))]
pub fn recording_staleness() -> Option<String> {
    None
}

/// Does a recorder shard exist at all? Distinguishes "no recording" (this
/// shell sources rc files normally) from "recording present and fresh" — so
/// `--doctor` doesn't claim "up to date" when nothing was ever recorded.
#[cfg(feature = "daemon")]
pub fn recording_present() -> bool {
    crate::daemon::paths::CachePaths::resolve()
        .ok()
        .and_then(|p| std::fs::read_dir(&p.images).ok())
        .map(|it| {
            it.flatten().any(|e| {
                e.file_name()
                    .to_str()
                    .map(|s| s.ends_with("-recorder.rkyv"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

#[cfg(not(feature = "daemon"))]
pub fn recording_present() -> bool {
    false
}

#[cfg(feature = "daemon")]
fn daemon_has_zshrs_rows() -> bool {
    let paths = match crate::daemon::paths::CachePaths::resolve() {
        Ok(p) => p,
        Err(_) => return false,
    };
    let entries = match std::fs::read_dir(&paths.images) {
        Ok(it) => it,
        Err(_) => return false,
    };
    for entry in entries.flatten() {
        if let Some(s) = entry.file_name().to_str() {
            if s.ends_with("-recorder.rkyv") {
                return true;
            }
        }
    }
    false
}

#[cfg(not(feature = "daemon"))]
fn daemon_has_zshrs_rows() -> bool {
    false
}

/// Should the shell-init path skip every `/etc/zsh*` + `~/.zsh*`
/// dotfile and apply canonical state from the daemon instead? O(1)
/// atomic load — set by `probe()` at startup.
/// zshrs-original — no C counterpart. C zsh's Src/init.c always
/// sources every startup file unconditionally.
#[inline]
pub fn should_skip_configs() -> bool {
    SHOULD_SKIP_CONFIGS.load(Ordering::Relaxed) != 0
}

/// Read the cached `[shell].skip_configs` setting (verbatim from
/// config — independent of whether conditions to skip were met).
/// Mostly useful for diagnostics / `zshrs --doctor`.
pub fn skip_configs_setting() -> SkipConfigs {
    match SKIP_CONFIGS.load(Ordering::Relaxed) {
        1 => SkipConfigs::Auto,
        2 => SkipConfigs::On,
        _ => SkipConfigs::Off,
    }
}

/// Run the probe via the daemon-client crate's cheap is-alive helper.
/// Falls back to a manual socket check if the daemon feature is off
/// (which is the workspace's stub-mode build path).
#[cfg(feature = "daemon")]
fn probe_socket() -> bool {
    match crate::daemon::paths::CachePaths::resolve() {
        Ok(paths) => crate::daemon::client::Client::is_daemon_alive(&paths),
        Err(_) => false,
    }
}

#[cfg(not(feature = "daemon"))]
fn probe_socket() -> bool {
    false
}

/// O(1) read of the cached probe result. Returns `Unknown` until
/// `probe()` has run.
/// zshrs-original — no C counterpart.
#[inline]
pub fn current() -> Mode {
    Mode::from_u8(STATE.load(Ordering::Relaxed))
}

/// Did the probe see a live daemon? `false` for any other state
/// (`Unknown` / `Absent` / `Disabled`).
/// zshrs-original — no C counterpart.
#[inline]
pub fn is_present() -> bool {
    current() == Mode::Present
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn config_setting_parses_common_aliases() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(ConfigSetting::parse("auto"), Some(ConfigSetting::Auto));
        assert_eq!(ConfigSetting::parse(""), Some(ConfigSetting::Auto));
        assert_eq!(ConfigSetting::parse("off"), Some(ConfigSetting::Off));
        assert_eq!(ConfigSetting::parse("false"), Some(ConfigSetting::Off));
        assert_eq!(ConfigSetting::parse("no"), Some(ConfigSetting::Off));
        assert_eq!(ConfigSetting::parse("0"), Some(ConfigSetting::Off));
        assert_eq!(
            ConfigSetting::parse("require"),
            Some(ConfigSetting::Require)
        );
        assert_eq!(ConfigSetting::parse("on"), Some(ConfigSetting::Require));
        assert_eq!(ConfigSetting::parse("true"), Some(ConfigSetting::Require));
        assert_eq!(ConfigSetting::parse("garbage"), None);
    }

    #[test]
    fn mode_round_trips_through_atomic() {
        let _g = crate::test_util::global_state_lock();
        for m in [Mode::Unknown, Mode::Present, Mode::Absent, Mode::Disabled] {
            assert_eq!(Mode::from_u8(m as u8), m);
        }
    }

    #[test]
    fn resolve_startup_config_path_absolute_trims() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(
            super::resolve_startup_config_path("  /tmp/x.zsh  "),
            PathBuf::from("/tmp/x.zsh")
        );
    }

    #[test]
    fn resolve_startup_config_path_tilde() {
        let _g = crate::test_util::global_state_lock();
        let home = dirs::home_dir().expect("HOME");
        assert_eq!(
            super::resolve_startup_config_path("~/init.zsh"),
            home.join("init.zsh")
        );
    }

    #[test]
    fn read_config_full_startup_config_from_zshrs_home() {
        let _g = crate::test_util::global_state_lock();
        use std::sync::Mutex;
        static ENV_LOCK: Mutex<()> = Mutex::new(());
        let _lock = ENV_LOCK.lock().expect("env test lock");
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("zshrs.toml"),
            "[shell]\nstartup_config = \"/tmp/zshrs-startup-test.zsh\"\n",
        )
        .expect("write zshrs.toml");
        unsafe {
            std::env::set_var("ZSHRS_HOME", dir.path());
        }
        let cfg = read_config_full();
        unsafe {
            std::env::remove_var("ZSHRS_HOME");
        }
        assert_eq!(
            cfg.startup_config,
            Some(PathBuf::from("/tmp/zshrs-startup-test.zsh"))
        );
    }
}
