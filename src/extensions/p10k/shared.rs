//! Helpers shared by every p10k segment module: the `_p9k_color1/2`
//! scheme colours, the `_p9k_declare` typed reads, `(g::)` decoding,
//! `_p9k_get_icon` resolution and the `_p9k_prompt_segment` constructor.
//!
//! `~/forkedRepos/powerlevel10k/internal/p10k.zsh` is THE SPEC; lines
//! are cited as `// p10k:NNN`.

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::extensions::p10k::icons;
use crate::extensions::p10k::render::Segment;
use crate::ported::params::getsparam;
use crate::ported::utils::getkeystring;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// p10k:8390-8396 — `[[ $_POWERLEVEL9K_COLOR_SCHEME == light ]] &&
/// _p9k_color1=7 || _p9k_color1=0`.
pub(crate) fn color1() -> &'static str {
    if p9k_global("COLOR_SCHEME", "dark") == "light" {
        "7"
    } else {
        "0"
    }
}

/// p10k:8392/8395 — `_p9k_color2`: the inverse of color1.
pub(crate) fn color2() -> &'static str {
    if p9k_global("COLOR_SCHEME", "dark") == "light" {
        "0"
    } else {
        "7"
    }
}

/// Read a parameter, falling back to the process environment (covers
/// early-startup renders before exports land in the paramtab).
pub(crate) fn env_or_param(name: &str) -> String {
    if let Some(v) = getsparam(name) {
        return v;
    }
    std::env::var(name).unwrap_or_default()
}

/// `_p9k_declare -b` read semantics (p10k:141-151): ONLY the literal
/// string `true` is truthy; unset uses the declared default.
pub(crate) fn global_bool(name: &str, default: bool) -> bool {
    match getsparam(&format!("POWERLEVEL9K_{name}")) {
        Some(v) => v == "true",
        None => default,
    }
}

/// `_p9k_declare -i` read: unset/empty/unparseable → default.
pub(crate) fn global_int(name: &str, default: i64) -> i64 {
    getsparam(&format!("POWERLEVEL9K_{name}"))
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(default)
}

/// `_p9k_declare -F` read: unset/empty/unparseable → default.
pub(crate) fn global_float(name: &str, default: f64) -> f64 {
    getsparam(&format!("POWERLEVEL9K_{name}"))
        .and_then(|v| v.trim().parse::<f64>().ok())
        .unwrap_or(default)
}

/// Escape `%` for prompt-expansion contexts — p10k's ubiquitous
/// `${x//\%/%%}`.
pub(crate) fn esc_pct(s: &str) -> String {
    s.replace('%', "%%")
}

/// zsh `${(g::)x}` echo-style escape decoding, as p10k applies to
/// user-supplied icons/templates/mode strings (p10k:524 and every
/// `_p9k_declare -e`).
pub(crate) fn decode_g(s: &str) -> String {
    getkeystring(s).0
}

/// Port of `_p9k_get_icon $1 $2` (p10k:511-530) —
/// probe the user-param chain for `<KEY>`; on
/// a hit apply `(g::)` decoding (p10k:524) plus the backspace-wrap
/// quirk (p10k:525); on a whole-chain miss the mode icon table answers.
pub(crate) fn seg_icon(segment: &str, state: Option<&str>, key: &str) -> String {
    let probed = p9k_param(segment, state, key, "\u{1}");
    if probed == "\u{1}" {
        return icons::icon(key).to_string();
    }
    let decoded = decode_g(&probed);
    // p10k:525 — [[ $ret != $'\b'? ]] || ret="%{$ret%}"
    let mut ch = decoded.chars();
    if ch.next() == Some('\u{8}') && ch.next().is_some() && ch.next().is_none() {
        return format!("%{{{decoded}%}}");
    }
    decoded
}

/// VISUAL_IDENTIFIER_EXPANSION hook (p10k:720/951).
pub(crate) fn apply_visual_identifier(segment: &str, state: Option<&str>, icon: String) -> Option<String> {
    let exp = p9k_param(
        segment,
        state,
        "VISUAL_IDENTIFIER_EXPANSION",
        "${P9K_VISUAL_IDENTIFIER}",
    );
    let resolved = if exp == "${P9K_VISUAL_IDENTIFIER}" {
        icon
    } else if !exp.contains('$') {
        exp
    } else {
        icon // `${...}` templates are evaluated at assembly (render.rs → expansion.rs)
    };
    if resolved.is_empty() {
        None
    } else {
        Some(resolved)
    }
}

/// CONTENT_EXPANSION (p10k:724/955) is evaluated once, at assembly, by
/// render.rs through expansion.rs (the shell expander with `P9K_CONTENT`
/// set); evaluating it here as well would apply the template twice.
pub(crate) fn apply_content_expansion(_segment: &str, _state: Option<&str>, content: String) -> String {
    content
}

/// Common constructor mirroring `_p9k_prompt_segment name bg fg icon
/// expand cond content`
/// (p10k:1101 + the color/icon/expansion hooks of
/// _p9k_left_prompt_segment).
pub(crate) fn make_segment(
    name: &str,
    state: Option<&str>,
    default_bg: &str,
    default_fg: &str,
    icon_key: &str,
    content: String,
) -> Segment {
    let bg = p9k_param(name, state, "BACKGROUND", default_bg);
    let fg = p9k_param(name, state, "FOREGROUND", default_fg);
    let icon_glyph = if icon_key.is_empty() {
        String::new()
    } else {
        seg_icon(name, state, icon_key)
    };
    let icon = apply_visual_identifier(name, state, icon_glyph);
    let content = apply_content_expansion(name, state, content);
    Segment {
        name: name.to_string(),
        state: state.map(|s| s.to_string()),
        content,
        icon,
        fg,
        bg,
    }
}


/// Does `name` match the zsh glob `pattern`? (tokenized, same engine as
/// `[[ $name == $~pattern ]]`.)
pub(crate) fn glob_name_matches(pattern: &str, name: &str) -> bool {
    let mut t = pattern.to_string();
    crate::ported::glob::tokenize(&mut t);
    crate::ported::pattern::patcompile(&t, 0, None)
        .is_some_and(|prog| crate::ported::pattern::pattry(&prog, name))
}

// ---------------------------------------------------------------------
// TTL cache for subprocess / filesystem probes (one store, two entry
// points). Replaces the four per-module `cached_ttl` copies.
// ---------------------------------------------------------------------

/// One cached probe result. `None` values are cached too, so a broken
/// tool cannot fork on every prompt.
struct TtlEntry {
    expiry: Instant,
    val: Option<String>,
    /// A background refresh is already running for this key.
    refreshing: bool,
}

static TTL_STORE: OnceLock<Mutex<HashMap<String, TtlEntry>>> = OnceLock::new();

fn ttl_store() -> &'static Mutex<HashMap<String, TtlEntry>> {
    TTL_STORE.get_or_init(Default::default)
}

fn ttl_put(key: &str, ttl: Duration, val: Option<String>) {
    let expiry = Instant::now()
        .checked_add(ttl)
        .unwrap_or_else(|| Instant::now() + Duration::from_secs(3600));
    if let Ok(mut g) = ttl_store().lock() {
        g.insert(key.to_string(), TtlEntry { expiry, val, refreshing: false });
    }
}

/// `(fresh, value)` for a key present in the store.
fn ttl_peek(key: &str) -> Option<(bool, Option<String>)> {
    let g = ttl_store().lock().ok()?;
    let e = g.get(key)?;
    Some((Instant::now() < e.expiry, e.val.clone()))
}

/// Blocking TTL cache. `run` returns `(ttl, value)` — the producer picks
/// the lifetime, so a failure can retry sooner than a success.
pub(crate) fn ttl_cached(
    key: &str,
    run: impl FnOnce() -> (Duration, Option<String>),
) -> Option<String> {
    if let Some((true, val)) = ttl_peek(key) {
        return val;
    }
    let (ttl, val) = run();
    ttl_put(key, ttl, val.clone());
    val
}

/// Stale-while-revalidate TTL cache: an expired entry is returned at
/// once and refreshed on a background thread, so only the very first
/// lookup of a key ever blocks a render. `run` must not touch shell
/// state — resolve everything it needs before the call.
pub(crate) fn ttl_cached_bg(
    key: &str,
    run: impl FnOnce() -> (Duration, Option<String>) + Send + 'static,
) -> Option<String> {
    match ttl_peek(key) {
        Some((true, val)) => val,
        Some((false, stale)) => {
            let claimed = ttl_store()
                .lock()
                .ok()
                .and_then(|mut g| {
                    let e = g.get_mut(key)?;
                    if e.refreshing {
                        return None;
                    }
                    e.refreshing = true;
                    Some(())
                })
                .is_some();
            if claimed {
                let owned = key.to_string();
                let spawned = std::thread::Builder::new()
                    .name("p10k-refresh".into())
                    .spawn({
                        let owned = owned.clone();
                        move || {
                            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
                            match r {
                                Ok((ttl, val)) => ttl_put(&owned, ttl, val),
                                Err(_) => {
                                    if let Ok(mut g) = ttl_store().lock() {
                                        if let Some(e) = g.get_mut(&owned) {
                                            e.refreshing = false;
                                        }
                                    }
                                }
                            }
                        }
                    });
                if spawned.is_err() {
                    if let Ok(mut g) = ttl_store().lock() {
                        if let Some(e) = g.get_mut(&owned) {
                            e.refreshing = false;
                        }
                    }
                }
            }
            stale
        }
        None => {
            let (ttl, val) = run();
            ttl_put(key, ttl, val.clone());
            val
        }
    }
}

/// `ttl_cached` for a fixed lifetime.
pub(crate) fn ttl_cached_fixed(
    key: &str,
    ttl: Duration,
    run: impl FnOnce() -> Option<String>,
) -> Option<String> {
    ttl_cached(key, || (ttl, run()))
}

#[cfg(test)]
mod ttl_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn blocking_cache_runs_producer_once_within_ttl() {
        let runs = AtomicUsize::new(0);
        let probe = || {
            runs.fetch_add(1, Ordering::SeqCst);
            (Duration::from_secs(60), Some("v".to_string()))
        };
        assert_eq!(ttl_cached("test.blocking_once", probe).as_deref(), Some("v"));
        assert_eq!(ttl_cached("test.blocking_once", probe).as_deref(), Some("v"));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn failure_is_cached_for_its_own_ttl() {
        let runs = AtomicUsize::new(0);
        let probe = || {
            runs.fetch_add(1, Ordering::SeqCst);
            (Duration::from_secs(60), None)
        };
        assert_eq!(ttl_cached("test.cached_none", probe), None);
        assert_eq!(ttl_cached("test.cached_none", probe), None);
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn expired_entry_is_served_stale_while_refresh_runs_off_thread() {
        let key = "test.swr_stale";
        // Seed: expires immediately.
        assert_eq!(
            ttl_cached_bg(key, || (Duration::ZERO, Some("old".into()))).as_deref(),
            Some("old")
        );
        // Expired: the slow refresh must NOT block the caller.
        let t0 = Instant::now();
        let got = ttl_cached_bg(key, || {
            std::thread::sleep(Duration::from_millis(300));
            (Duration::from_secs(60), Some("new".into()))
        });
        assert_eq!(got.as_deref(), Some("old"));
        assert!(t0.elapsed() < Duration::from_millis(150), "caller blocked on refresh");
        // The refresh lands.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some((true, Some(v))) = ttl_peek(key) {
                assert_eq!(v, "new");
                break;
            }
            assert!(Instant::now() < deadline, "refresh never landed");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn concurrent_expired_lookups_start_one_refresh() {
        let key = "test.swr_single_flight";
        ttl_cached_bg(key, || (Duration::ZERO, Some("old".into())));
        let runs = std::sync::Arc::new(AtomicUsize::new(0));
        for _ in 0..8 {
            let runs = runs.clone();
            ttl_cached_bg(key, move || {
                runs.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(200));
                (Duration::from_secs(60), Some("new".into()))
            });
        }
        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }
}
