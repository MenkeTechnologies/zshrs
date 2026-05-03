//! Daemon-presence detection + per-user config knob.
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
//! Config (`~/.cache/zshrs/zshrs.toml`, all optional):
//!
//!     [daemon]
//!     # "auto" (default) = probe at startup, use if alive
//!     # "off"            = never probe; pure vanilla zsh mode
//!     # "require"        = probe and warn if absent (no spawn either way)
//!     enabled = "auto"
//!
//! Lives in `~/.cache/zshrs/` alongside everything else (rkyv shards,
//! catalog.db, daemon.sock, daemon.toml, log, …) — single directory
//! rule for all zshrs files. Survives normal cache eviction by virtue
//! of being the user's own config; if the user `rm -rf ~/.cache/zshrs/`
//! they're explicitly resetting both cache + config together, which
//! is the intent.

use std::sync::atomic::{AtomicU8, Ordering};

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

/// Read the user's `[daemon].enabled` setting from
/// `~/.config/zshrs/zshrs.toml`. Missing file / missing section /
/// missing key = `Auto`. Unrecognized value also = `Auto` (with a
/// log warning so the typo is visible).
pub fn read_config() -> ConfigSetting {
    let path = match config_file_path() {
        Some(p) => p,
        None => return ConfigSetting::Auto,
    };
    let body = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return ConfigSetting::Auto,
    };
    let parsed = match body.parse::<toml::Table>() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "zshrs.toml: parse failed; using daemon=auto");
            return ConfigSetting::Auto;
        }
    };
    let enabled = parsed
        .get("daemon")
        .and_then(|v| v.as_table())
        .and_then(|t| t.get("enabled"))
        .and_then(|v| v.as_str())
        .unwrap_or("auto");
    match ConfigSetting::parse(enabled) {
        Some(c) => c,
        None => {
            tracing::warn!(
                value = enabled,
                "zshrs.toml: [daemon].enabled is not auto/off/require; falling back to auto"
            );
            ConfigSetting::Auto
        }
    }
}

/// Resolve `~/.cache/zshrs/zshrs.toml` (respecting `$XDG_CACHE_HOME`).
/// Single-directory rule: every zshrs file lives under
/// `~/.cache/zshrs/`. Returns None if neither $XDG_CACHE_HOME nor
/// $HOME is set, which is rare enough to treat as "no config file".
fn config_file_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache"))
        })?;
    Some(base.join("zshrs").join("zshrs.toml"))
}

/// Cheap probe — connect-only, no handshake — does the daemon socket
/// answer? Sets the global state so subsequent `is_present()` checks
/// are O(1) atomic loads.
///
/// Honors `[daemon].enabled`:
///   - `Off` → state = Disabled, no probe
///   - `Auto` / `Require` → probe; state = Present or Absent
///
/// `Require` additionally logs a warning if the daemon isn't alive —
/// signal to the user that they configured the shell to expect a
/// daemon but didn't actually start one.
pub fn probe() -> Mode {
    let setting = read_config();
    match setting {
        ConfigSetting::Off => {
            tracing::info!("daemon: disabled in config ([daemon] enabled = \"off\")");
            STATE.store(Mode::Disabled as u8, Ordering::Relaxed);
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
        match setting {
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
    mode
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
#[inline]
pub fn current() -> Mode {
    Mode::from_u8(STATE.load(Ordering::Relaxed))
}

/// Convenience: did the probe see a live daemon? `false` for any
/// other state (Unknown / Absent / Disabled).
#[inline]
pub fn is_present() -> bool {
    current() == Mode::Present
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_setting_parses_common_aliases() {
        assert_eq!(ConfigSetting::parse("auto"), Some(ConfigSetting::Auto));
        assert_eq!(ConfigSetting::parse(""), Some(ConfigSetting::Auto));
        assert_eq!(ConfigSetting::parse("off"), Some(ConfigSetting::Off));
        assert_eq!(ConfigSetting::parse("false"), Some(ConfigSetting::Off));
        assert_eq!(ConfigSetting::parse("no"), Some(ConfigSetting::Off));
        assert_eq!(ConfigSetting::parse("0"), Some(ConfigSetting::Off));
        assert_eq!(ConfigSetting::parse("require"), Some(ConfigSetting::Require));
        assert_eq!(ConfigSetting::parse("on"), Some(ConfigSetting::Require));
        assert_eq!(ConfigSetting::parse("true"), Some(ConfigSetting::Require));
        assert_eq!(ConfigSetting::parse("garbage"), None);
    }

    #[test]
    fn mode_round_trips_through_atomic() {
        for m in [Mode::Unknown, Mode::Present, Mode::Absent, Mode::Disabled] {
            assert_eq!(Mode::from_u8(m as u8), m);
        }
    }
}
