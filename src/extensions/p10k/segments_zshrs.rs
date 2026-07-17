//! p10k ZSHRS-NATIVE segments — zshrs_daemon / zshrs_workers /
//! zshrs_jit / zshrs_cache / zshrs_history / stryke.
//!
//! Beyond the p10k spec: `~/forkedRepos/powerlevel10k/internal/p10k.zsh`
//! has no counterpart for these — they expose zshrs's OWN runtime in
//! the prompt (world-first shell-introspection segment family). The
//! module CONVENTIONS still mirror the ported segment modules
//! (segments_sys.rs / segments_extra.rs): make_segment + the
//! VISUAL_IDENTIFIER / CONTENT_EXPANSION hooks, `Some(vec![])` =
//! handled-but-hidden, TTL caches for anything that touches disk or a
//! database, tracing-only logging. All styling is overridable via
//! `POWERLEVEL9K_ZSHRS_*` / `POWERLEVEL9K_STRYKE_*` params through
//! `config::p9k_param` — no code change needed to restyle.
//!
//! Data sources (all in-process — NO subprocess ever runs here):
//! - zshrs_daemon: `crate::daemon_presence::current()` — O(1) atomic
//!   load of the startup socket probe (daemon_presence.rs:654-668;
//!   probe itself at :423-489).
//! - zshrs_workers: `ShellExecutor.worker_pool` (vm_helper.rs:510) —
//!   `WorkerPool::size()` / `queue_depth()` (worker.rs:215-224).
//! - zshrs_jit: `fusevm::JitCompiler::jit_cache_dir()` (fusevm-0.14.5
//!   jit.rs:7055) resolves the on-disk native-code cache; the `*.fjit`
//!   blob scan runs behind a TTL. No tier/compile-counter stats API is
//!   exported by fusevm (VM keeps its JIT state private) — the segment
//!   shows the DISK-cache state only; counters are a fusevm follow-up.
//! - zshrs_cache: rkyv shard files under `$ZSHRS_HOME` (default
//!   `~/.zshrs`) — `autoloads.rkyv` (autoload_cache.rs:424-433),
//!   `scripts.rkyv` (script_cache.rs:3), plus the daemon's
//!   `images/*.rkyv` (daemon/paths.rs:141-145). Stat-only, TTL-cached.
//! - zshrs_history: sqlite store at `$ZSHRS_HOME/zshrs_history.db`
//!   (history.rs:114-116, root at :137-145); entry count via
//!   `HistoryEngine::count()` (history.rs:492-495) through
//!   `with_session_engine` (history.rs:575-583), long TTL.
//! - stryke: ALWAYS HIDDEN. The `@`-prefix stryke hook exists
//!   (lib.rs:393-408) but `STRYKE_HANDLER` is a private `OnceLock`
//!   with no is-registered probe, and `try_stryke_dispatch` EXECUTES
//!   the handler — not callable from prompt render. Hidden rather
//!   than faked until lib.rs grows a read-only registration query.
//!
//! Shared-helper duplication: color1/decode_g/apply_visual_identifier/
//! apply_content_expansion/cached_ttl mirror segments_sys.rs (private
//! there; this module may not edit other files). Hoisting them into a
//! shared submodule is a follow-up for the orchestrator.

use crate::extensions::p10k::config::{p9k_global, p9k_param};
use crate::extensions::p10k::render::Segment;
use crate::ported::utils::getkeystring;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------

/// Segment dispatch per the p10k module contract. `None` = not handled
/// by this module; `Some(vec![])` = handled but hidden this prompt.
pub fn build_segment(name: &str) -> Option<Vec<Segment>> {
    match name {
        "zshrs_daemon" => Some(zshrs_daemon_segments()),
        "zshrs_workers" => Some(zshrs_workers_segments()),
        "zshrs_jit" => Some(zshrs_jit_segments()),
        "zshrs_cache" => Some(zshrs_cache_segments()),
        "zshrs_history" => Some(zshrs_history_segments()),
        "stryke" => Some(stryke_segments()),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Shared helpers (mirror segments_sys.rs — see module doc)
// ---------------------------------------------------------------------

/// p10k:8390-8396 — `[[ $_POWERLEVEL9K_COLOR_SCHEME == light ]] &&
/// _p9k_color1=7 || _p9k_color1=0`.
fn color1() -> &'static str {
    if p9k_global("COLOR_SCHEME", "dark") == "light" {
        "7"
    } else {
        "0"
    }
}

/// zsh `${(g::)x}` echo-style escape decoding, as p10k applies to
/// user-supplied icons/templates (p10k:524 and every `_p9k_declare -e`).
fn decode_g(s: &str) -> String {
    getkeystring(s).0
}

/// Icon resolution for zshrs-native keys: probe the user-param chain
/// for `<KEY>` (same chain `_p9k_get_icon` walks, p10k:511-530); on a
/// whole-chain miss fall back to `default_glyph`. Differs from
/// segments_sys::seg_icon only in the fallback — these keys have no
/// entry in the ported icons.rs mode table (icons.rs:188-194 answers
/// "" for unknown keys and would debug-log every prompt), so the
/// sensible default lives here instead.
fn seg_icon_or(segment: &str, state: Option<&str>, key: &str, default_glyph: &str) -> String {
    let probed = p9k_param(segment, state, key, "\u{1}");
    if probed == "\u{1}" {
        return default_glyph.to_string();
    }
    let decoded = decode_g(&probed);
    // p10k:525 — [[ $ret != $'\b'? ]] || ret="%{$ret%}"
    let mut ch = decoded.chars();
    if ch.next() == Some('\u{8}') && ch.next().is_some() && ch.next().is_none() {
        return format!("%{{{decoded}%}}");
    }
    decoded
}

/// VISUAL_IDENTIFIER_EXPANSION hook (p10k:720/951) — identical to
/// segments_sys::apply_visual_identifier.
fn apply_visual_identifier(segment: &str, state: Option<&str>, icon: String) -> Option<String> {
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
        tracing::debug!(
            target: "p10k",
            segment,
            ?state,
            %exp,
            "dynamic VISUAL_IDENTIFIER_EXPANSION unported — using resolved icon"
        );
        icon
    };
    if resolved.is_empty() {
        None
    } else {
        Some(resolved)
    }
}

/// CONTENT_EXPANSION hook (p10k:724/955) — identical to
/// segments_sys::apply_content_expansion.
fn apply_content_expansion(segment: &str, state: Option<&str>, content: String) -> String {
    let exp = p9k_param(segment, state, "CONTENT_EXPANSION", "${P9K_CONTENT}");
    if exp == "${P9K_CONTENT}" {
        return content;
    }
    if !exp.contains('$') {
        return exp;
    }
    if exp.contains("${P9K_CONTENT}") {
        let out = exp.replace("${P9K_CONTENT}", &content);
        if out.contains('$') {
            tracing::debug!(
                target: "p10k",
                segment, ?state, %exp,
                "CONTENT_EXPANSION has unevaluated expansions beyond ${{P9K_CONTENT}}"
            );
        }
        return out;
    }
    tracing::debug!(
        target: "p10k",
        segment, ?state, %exp,
        "dynamic CONTENT_EXPANSION unported — using segment content"
    );
    content
}

/// Common constructor mirroring segments_sys::make_segment, with a
/// literal default glyph instead of an icons.rs table key (see
/// seg_icon_or).
fn make_segment(
    name: &str,
    state: Option<&str>,
    default_bg: &str,
    default_fg: &str,
    icon_key: &str,
    default_glyph: &str,
    content: String,
) -> Segment {
    let bg = p9k_param(name, state, "BACKGROUND", default_bg);
    let fg = p9k_param(name, state, "FOREGROUND", default_fg);
    let icon_glyph = seg_icon_or(name, state, icon_key, default_glyph);
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

// ---------------------------------------------------------------------
// TTL cache (mirror segments_sys.rs)
// ---------------------------------------------------------------------

/// Disk / database probes run at most once per TTL (same shape as the
/// segments_sys replacement for p10k's `_p9k_worker_invoke` loop).
/// `None` results (scan/query failure) are cached too, so a broken
/// source can't re-probe on every prompt. Bounded by the number of
/// `&'static str` keys.
static TTL_CACHE: OnceLock<Mutex<HashMap<&'static str, (Instant, Option<String>)>>> =
    OnceLock::new();

fn cached_ttl(
    key: &'static str,
    ttl: Duration,
    run: impl FnOnce() -> Option<String>,
) -> Option<String> {
    let m = TTL_CACHE.get_or_init(Default::default);
    if let Ok(guard) = m.lock() {
        if let Some((at, val)) = guard.get(key) {
            if at.elapsed() < ttl {
                return val.clone();
            }
        }
    }
    let val = run();
    if let Ok(mut guard) = m.lock() {
        guard.insert(key, (Instant::now(), val.clone()));
    }
    val
}

// ---------------------------------------------------------------------
// Pure formatters (unit-tested below)
// ---------------------------------------------------------------------

/// Port of `_p9k_human_readable_bytes` (p10k:327-343) — identical to
/// segments_sys::human_readable_bytes; used for the JIT disk-cache
/// size.
fn human_readable_bytes(bytes: f64) -> String {
    // p10k:322 — __p9k_byte_suffix=('B' 'K' 'M' 'G' 'T' 'P' 'E' 'Z' 'Y')
    const SUFFIX: [&str; 9] = ["B", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let mut n = bytes;
    let mut suf = SUFFIX[0];
    for s in SUFFIX {
        suf = s;
        if n < 1024.0 {
            break;
        }
        n /= 1024.0;
    }
    let mut ret = if n >= 100.0 {
        format!("{n:.0}.")
    } else if n >= 10.0 {
        format!("{n:.1}")
    } else {
        format!("{n:.2}")
    };
    while ret.ends_with('0') {
        ret.pop();
    }
    if ret.ends_with('.') {
        ret.pop();
    }
    format!("{ret}{suf}")
}

/// Compact integer for prompt real estate: full digits below 10k,
/// then `478k` / `1.2M`. History stores run to hundreds of thousands
/// of rows — raw digits would dominate the prompt line.
fn compact_count(n: i64) -> String {
    if n < 10_000 {
        n.to_string()
    } else if n < 1_000_000 {
        format!("{}k", n / 1_000)
    } else {
        let m = n as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{m:.0}M")
        } else {
            format!("{m:.1}M")
        }
    }
}

/// zshrs_jit content: an empty (but enabled) disk cache shows the
/// static "jit" tag; a populated one shows blob count + total size.
fn jit_content(count: usize, bytes: u64) -> String {
    if count == 0 {
        "jit".to_string()
    } else {
        format!("{count} {}", human_readable_bytes(bytes as f64))
    }
}

/// `$ZSHRS_HOME` else `~/.zshrs` — the single directory every zshrs
/// artifact lives under. Mirrors autoload_cache.rs:424-433 /
/// history.rs:137-145 / daemon CachePaths::resolve
/// (daemon/paths.rs:190-200); duplicated because those roots are
/// module-private and this module may not edit other files.
fn zshrs_root() -> PathBuf {
    if let Some(custom) = std::env::var_os("ZSHRS_HOME") {
        PathBuf::from(custom)
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join(".zshrs")
    }
}

/// True in the modes where zshrs-introspection segments are
/// meaningless: `--zsh` strict parity (caches/daemon off — see
/// vm_helper.rs:496-500) and the executor's zsh/bash/posix compat
/// flags (vm_helper.rs:500-508). Outside a VM execution context
/// (`try_with_executor` → None) only the global flag answers.
fn introspection_irrelevant() -> bool {
    if crate::IS_ZSH_MODE.load(Ordering::Relaxed) {
        return true;
    }
    crate::fusevm_bridge::try_with_executor(|exec| {
        exec.zsh_compat || exec.bash_compat || exec.posix_mode
    })
    .unwrap_or(false)
}

// ---------------------------------------------------------------------
// zshrs_daemon
// ---------------------------------------------------------------------

/// Daemon presence: glyph + state from the startup socket probe.
/// States: PRESENT (socket answered) / ABSENT (vanilla mode).
/// Hidden when the probe never ran (Unknown), when the user opted out
/// (`[daemon] enabled = "off"` → Disabled, daemon_presence.rs:74-77),
/// and in parity/compat modes where the daemon never runs.
///
/// Defaults: bg `color1`, fg `green` (PRESENT) / `8` dim (ABSENT),
/// icon `\u{F0AE}` (server). Override via
/// POWERLEVEL9K_ZSHRS_DAEMON_{PRESENT,ABSENT}_{FOREGROUND,BACKGROUND}
/// and POWERLEVEL9K_ZSHRS_DAEMON_ICON.
fn zshrs_daemon_segments() -> Vec<Segment> {
    if introspection_irrelevant() {
        return vec![];
    }
    use crate::daemon_presence::Mode;
    // daemon_presence.rs:654-660 — O(1) atomic load of the probe result.
    let (state, fg, content) = match crate::daemon_presence::current() {
        Mode::Present => ("PRESENT", "green", "up"),
        Mode::Absent => ("ABSENT", "8", "down"),
        // Probe never ran, or user disabled the daemon — irrelevant.
        Mode::Unknown | Mode::Disabled => return vec![],
    };
    vec![make_segment(
        "zshrs_daemon",
        Some(state),
        color1(),
        fg,
        "ZSHRS_DAEMON_ICON",
        "\u{F0AE}",
        content.to_string(),
    )]
}

// ---------------------------------------------------------------------
// zshrs_workers
// ---------------------------------------------------------------------

/// Worker-pool size (+ queued backlog when nonzero): `18` or `18+3`.
/// `WorkerPool` exposes size (worker.rs:215-217) and an approximate
/// queue depth (worker.rs:222-224, incremented on submit / decremented
/// on task START) — a true "busy right now" counter does not exist, so
/// the backlog is shown instead of being invented. Hidden outside a VM
/// execution context and in POSIX mode ("no worker pool",
/// vm_helper.rs:507).
///
/// Defaults: bg `color1`, fg `cyan`, icon `\u{F085}` (cogs). Override
/// via POWERLEVEL9K_ZSHRS_WORKERS_*.
fn zshrs_workers_segments() -> Vec<Segment> {
    if introspection_irrelevant() {
        return vec![];
    }
    // vm_helper.rs:510 — worker_pool lives on the executor; reachable
    // only inside a VM execution context (fusevm_bridge.rs:501-512).
    let Some((size, queued)) = crate::fusevm_bridge::try_with_executor(|exec| {
        (exec.worker_pool.size(), exec.worker_pool.queue_depth())
    }) else {
        return vec![];
    };
    let content = if queued > 0 {
        format!("{size}+{queued}")
    } else {
        size.to_string()
    };
    vec![make_segment(
        "zshrs_workers",
        None,
        color1(),
        "cyan",
        "ZSHRS_WORKERS_ICON",
        "\u{F085}",
        content,
    )]
}

// ---------------------------------------------------------------------
// zshrs_jit
// ---------------------------------------------------------------------

/// fusevm JIT disk-cache state: `.fjit` native-blob count + total
/// size, e.g. `42 1.3M`; a fresh (empty) cache shows the static `jit`
/// tag. Hidden when disk caching is disabled
/// (`FUSEVM_JIT_CACHE_DIR=off` → `jit_cache_dir()` None, fusevm-0.14.5
/// jit.rs:7055) and in parity/compat modes. Live tier / block / trace
/// compile counters are NOT exported by fusevm's public API (verified
/// against fusevm-0.14.5 lib.rs exports — VM JIT state is private);
/// only the on-disk state is shown. 15s TTL on the directory scan.
///
/// Defaults: bg `color1`, fg `magenta`, icon `\u{F0E7}` (bolt).
/// Override via POWERLEVEL9K_ZSHRS_JIT_*.
fn zshrs_jit_segments() -> Vec<Segment> {
    if introspection_irrelevant() {
        return vec![];
    }
    let Some(joined) = cached_ttl("zshrs_jit.scan", Duration::from_secs(15), || {
        // fusevm-0.14.5 jit.rs:7055 — resolution order: programmatic
        // override, $FUSEVM_JIT_CACHE_DIR (off-sentinel disables),
        // default ~/.cache/fusevm-jit. None = disk caching disabled.
        // JitCompiler::new() is an empty-Vec constructor (jit.rs) —
        // the method is effectively static.
        let dir = fusevm::JitCompiler::new().jit_cache_dir()?;
        // Same *.fjit filter as jit_cache_size_bytes (jit.rs:7062);
        // one pass yields count AND bytes. Missing dir = enabled but
        // never populated — real state, count 0.
        let (mut count, mut bytes) = (0usize, 0u64);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                if entry.file_name().to_string_lossy().ends_with(".fjit") {
                    count += 1;
                    bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
        Some(format!("{count}\u{1f}{bytes}"))
    }) else {
        return vec![];
    };
    let mut f = joined.split('\u{1f}');
    let count: usize = f.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let bytes: u64 = f.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    vec![make_segment(
        "zshrs_jit",
        None,
        color1(),
        "magenta",
        "ZSHRS_JIT_ICON",
        "\u{F0E7}",
        jit_content(count, bytes),
    )]
}

// ---------------------------------------------------------------------
// zshrs_cache
// ---------------------------------------------------------------------

/// rkyv cache-shard census under `$ZSHRS_HOME`: top-level `*.rkyv`
/// (autoloads.rkyv — autoload_cache.rs:3/424-433; scripts.rkyv —
/// script_cache.rs:3) plus the daemon's sharded `images/*.rkyv`
/// (daemon/paths.rs:141-145). Shows shard count + total bytes (humanized). Hidden when the
/// root is missing or holds zero shards, and in parity/compat modes
/// (caches off — vm_helper.rs:496-500). Stat-only scan behind a 30s
/// TTL.
///
/// Defaults: bg `color1`, fg `blue`, icon `\u{F1C0}` (database).
/// Override via POWERLEVEL9K_ZSHRS_CACHE_*.
fn zshrs_cache_segments() -> Vec<Segment> {
    if introspection_irrelevant() {
        return vec![];
    }
    let Some(joined) = cached_ttl("zshrs_cache.scan", Duration::from_secs(30), || {
        let root = zshrs_root();
        // One pass per dir yields shard count AND total bytes — same
        // shape as the zshrs_jit `.fjit` scan above.
        let rkyv_in = |dir: &std::path::Path| -> (usize, u64) {
            std::fs::read_dir(dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| e.file_name().to_string_lossy().ends_with(".rkyv"))
                        .fold((0usize, 0u64), |(c, b), e| {
                            (c + 1, b + e.metadata().map(|m| m.len()).unwrap_or(0))
                        })
                })
                .unwrap_or((0, 0))
        };
        let (c1, b1) = rkyv_in(&root);
        let (c2, b2) = rkyv_in(&root.join("images"));
        let (count, bytes) = (c1 + c2, b1 + b2);
        if count == 0 {
            return None; // no cache dir / no shards — hidden
        }
        Some(format!("{count}\u{1f}{bytes}"))
    }) else {
        return vec![];
    };
    let mut f = joined.split('\u{1f}');
    let count: usize = f.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let bytes: u64 = f.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    vec![make_segment(
        "zshrs_cache",
        None,
        color1(),
        "blue",
        "ZSHRS_CACHE_ICON",
        "\u{F1C0}",
        format!("{count} {}", human_readable_bytes(bytes as f64)),
    )]
}

// ---------------------------------------------------------------------
// zshrs_history
// ---------------------------------------------------------------------

/// History-store size: entry count from the sqlite index at
/// `$ZSHRS_HOME/zshrs_history.db` (history.rs:114-116), compacted
/// (`478k`). `HistoryEngine::count()` (history.rs:492-495) runs
/// through the session engine (history.rs:575-583 — thread-local,
/// already open on the interactive thread) behind a 60s TTL. The db
/// file is stat-gated first so a session that never wrote history
/// doesn't create `~/.zshrs` from prompt render. Hidden when the db is
/// absent, the query fails, or in parity/compat modes.
///
/// Defaults: bg `color1`, fg `yellow`, icon `\u{F1DA}` (history clock).
/// Override via POWERLEVEL9K_ZSHRS_HISTORY_*.
fn zshrs_history_segments() -> Vec<Segment> {
    if introspection_irrelevant() {
        return vec![];
    }
    let Some(count_s) = cached_ttl("zshrs_history.count", Duration::from_secs(60), || {
        let db = zshrs_root().join("zshrs_history.db");
        if !db.exists() {
            return None;
        }
        crate::history::with_session_engine(|e| e.count().ok())
            .flatten()
            .map(compact_count)
    }) else {
        return vec![];
    };
    vec![make_segment(
        "zshrs_history",
        None,
        color1(),
        "yellow",
        "ZSHRS_HISTORY_ICON",
        "\u{F1DA}",
        count_s,
    )]
}

// ---------------------------------------------------------------------
// stryke
// ---------------------------------------------------------------------

/// Embedded stryke language state — ALWAYS HIDDEN today (module doc):
/// the fat-binary registration hook exists (lib.rs:393-408
/// `set_stryke_handler` / `STRYKE_HANDLER`), but the OnceLock is
/// private with no read-only "is a handler registered?" accessor, and
/// `try_stryke_dispatch` would EXECUTE the handler — running foreign
/// code from prompt render is not an option. No version constant is
/// linked into this crate either. Hidden rather than faked; the
/// segment name is claimed so the wiring (POWERLEVEL9K_STRYKE_*
/// params) is ready the moment lib.rs exposes a probe.
fn stryke_segments() -> Vec<Segment> {
    tracing::debug!(
        target: "p10k",
        "stryke segment: no registration probe in lib.rs (STRYKE_HANDLER private) — hidden"
    );
    vec![]
}

// ---------------------------------------------------------------------
// Tests (pure formatters only)
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_count_bands() {
        assert_eq!(compact_count(0), "0");
        assert_eq!(compact_count(9_999), "9999");
        assert_eq!(compact_count(10_000), "10k");
        assert_eq!(compact_count(478_123), "478k");
        assert_eq!(compact_count(999_999), "999k");
        assert_eq!(compact_count(1_234_567), "1.2M");
        assert_eq!(compact_count(12_345_678), "12M");
    }

    /// p10k:327-343 semantics (mirrors segments_sys): trailing zeros
    /// and dots stripped, suffix ladder in 1024 steps.
    #[test]
    fn human_readable_bytes_forms() {
        assert_eq!(human_readable_bytes(0.0), "0B");
        assert_eq!(human_readable_bytes(1024.0), "1K");
        assert_eq!(human_readable_bytes(1_363_148.8), "1.3M");
        assert_eq!(human_readable_bytes(5.0 * 1024.0 * 1024.0), "5M");
    }

    #[test]
    fn jit_content_empty_vs_populated() {
        assert_eq!(jit_content(0, 0), "jit");
        assert_eq!(jit_content(42, 1_363_149), "42 1.3M");
        assert_eq!(jit_content(1, 1024), "1 1K");
    }
}
