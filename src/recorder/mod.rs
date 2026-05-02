//! Plugin-Framework-Agnostic State-Modification Recorder (PFA-SMR).
//!
//! Single-shot indexer. Spawns the user's shell init (or any script the
//! recorder bin was invoked with), captures every state mutation as it
//! flows through a state-mutating dispatcher, prints `Captured ...` to
//! stderr in real time, mirrors every line into the zshrs tracing log,
//! bundles the full set on shell exit, IPCs it once to `zshrs-daemon`,
//! prints summary stats, then exits. The daemon ingests the bundle and
//! rebuilds the rkyv + SQLite read caches from it.
//!
//! Per docs/RECORDER.md: only the recorder can capture state at 100%
//! fidelity; the daemon never walks user config. New plugin installs
//! require the user to re-run `zshrs-recorder`.
//!
//! Module gated by `#![cfg(feature = "recorder")]` so the default
//! `zshrs` binary contains zero recorder code.

#![cfg(feature = "recorder")]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};

/// Global on/off switch. `bins/zshrs-recorder.rs` calls `enable()` at
/// startup before the executor runs; `bins/zshrs.rs` never touches it
/// (this module doesn't exist in that build).
static ENABLED: AtomicBool = AtomicBool::new(false);

/// Skip the end-of-run daemon IPC. Used by hermetic tests
/// (`tests/recorder_harness.rs`) and one-off `--no-daemon` runs that
/// just want stderr capture without spawning a daemon.
static DAEMON_DISABLED: AtomicBool = AtomicBool::new(false);

/// Re-entrancy guard — set during emit so any builtin a recorder hook
/// itself triggers is not re-recorded.
static IN_RECORDER: AtomicBool = AtomicBool::new(false);

/// Monotonic per-record sequence number.
static ORDER_IDX: AtomicU64 = AtomicU64::new(0);

/// Recorder start time, used by the summary footer for `runs.started_at_ns`.
static START_NS: AtomicU64 = AtomicU64::new(0);

/// In-process buffer of every captured event. Flushed to the daemon
/// in one IPC call at end-of-run.
static BUFFER: Lazy<Mutex<Vec<RecordEvent>>> = Lazy::new(|| Mutex::new(Vec::with_capacity(4096)));

/// Mirror of the SQLite `definitions.kind` discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefKind {
    Alias,
    GAlias,
    SAlias,
    Function,
    Assign,
    Typeset,
    Export,
    PathMod,
    HashD,
    Zstyle,
    Bindkey,
    Compdef,
    Zmodload,
    Setopt,
    Unsetopt,
    Trap,
    Sched,
    Source,
    /// Removal events — RECORDER.md "Open question 4: Should we record
    /// `unalias` / `unset` / `disable` events?". Recorded so `zwhere -l`
    /// lineage can see "this name was defined at A:N, removed at B:M,
    /// redefined at C:K". Without these the override chain is invisible.
    Unalias,
    Unset,
}

impl DefKind {
    pub fn as_str(self) -> &'static str {
        match self {
            DefKind::Alias => "alias",
            DefKind::GAlias => "alias -g",
            DefKind::SAlias => "alias -s",
            DefKind::Function => "function",
            DefKind::Assign => "assign",
            DefKind::Typeset => "typeset",
            DefKind::Export => "export",
            DefKind::PathMod => "path_mod",
            DefKind::HashD => "hash -d",
            DefKind::Zstyle => "zstyle",
            DefKind::Bindkey => "bindkey",
            DefKind::Compdef => "compdef",
            DefKind::Zmodload => "zmodload",
            DefKind::Setopt => "setopt",
            DefKind::Unsetopt => "unsetopt",
            DefKind::Trap => "trap",
            DefKind::Sched => "sched",
            DefKind::Source => "source",
            DefKind::Unalias => "unalias",
            DefKind::Unset => "unset",
        }
    }
}

/// One state-mutation event. Field set is the recorder's wire format
/// for the daemon `recorder_ingest` op; mirrors the SQL `definitions`
/// row in docs/RECORDER.md §Schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordEvent {
    pub order_idx: u64,
    pub ts_ns: u64,
    pub kind: DefKind,
    pub name: String,
    pub value: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub fn_chain: Option<String>,
}

/// Bundle sent to the daemon at end-of-run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecorderBundle {
    pub started_at_ns: u64,
    pub finished_at_ns: u64,
    pub cmdline: String,
    pub zdotdir: Option<String>,
    pub home: Option<String>,
    pub events: Vec<RecordEvent>,
}

#[inline]
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
    START_NS.store(now_ns(), Ordering::Relaxed);
    tracing::info!("recorder: enabled");
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

#[inline]
pub fn set_daemon_disabled(v: bool) {
    DAEMON_DISABLED.store(v, Ordering::Relaxed);
}

#[inline]
fn daemon_disabled() -> bool {
    DAEMON_DISABLED.load(Ordering::Relaxed)
}

fn now_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// Per-call site context derived from the executor (current `$LINENO`,
/// current source file, current `$funcstack`).
#[derive(Debug, Clone, Default)]
pub struct RecordCtx {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub fn_chain: Option<String>,
}

fn loc_str(file: &Option<String>, line: Option<u32>) -> String {
    match (file.as_deref(), line) {
        (Some(f), Some(l)) => format!("{}:{}", f, l),
        (Some(f), None) => f.to_string(),
        _ => "<unknown>".to_string(),
    }
}

fn fn_chain_suffix(chain: &Option<String>) -> String {
    match chain.as_deref() {
        Some(c) if !c.is_empty() => format!(" ({})", c),
        _ => String::new(),
    }
}

/// Push a record. Real-time stderr line + tracing log line + push to
/// in-process buffer. Re-entrancy-guarded.
pub fn emit(
    kind: DefKind,
    name: impl Into<String>,
    value: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    fn_chain: Option<String>,
) {
    if !is_enabled() {
        return;
    }
    if IN_RECORDER.swap(true, Ordering::Acquire) {
        return;
    }
    let name = name.into();

    // Realtime "Captured ..." line, format per docs/RECORDER.md user spec.
    let value_part = match value.as_deref() {
        Some(v) => format!("={}", short_value(v)),
        None => String::new(),
    };
    let loc = loc_str(&file, line);
    let chain = fn_chain_suffix(&fn_chain);
    let kind_str = kind.as_str();
    eprintln!(
        "Captured {} {}{}, file: {}{}",
        kind_str, name, value_part, loc, chain
    );
    tracing::info!(
        kind = kind_str,
        %name,
        value = value.as_deref().unwrap_or(""),
        file = file.as_deref().unwrap_or(""),
        line = line.unwrap_or(0),
        fn_chain = fn_chain.as_deref().unwrap_or(""),
        "recorder: captured"
    );

    let ev = RecordEvent {
        order_idx: ORDER_IDX.fetch_add(1, Ordering::Relaxed),
        ts_ns: now_ns(),
        kind,
        name,
        value,
        file,
        line,
        fn_chain,
    };
    if let Ok(mut buf) = BUFFER.lock() {
        buf.push(ev);
    }
    IN_RECORDER.store(false, Ordering::Release);
}

/// Truncate values for the realtime stderr line. SQL/IPC store the full
/// value; only the human readout is trimmed.
fn short_value(s: &str) -> String {
    const MAX: usize = 120;
    let single = s.replace('\n', "\\n");
    if single.chars().count() <= MAX {
        format!("\"{}\"", single)
    } else {
        let mut clipped: String = single.chars().take(MAX).collect();
        clipped.push('…');
        format!("\"{}\"", clipped)
    }
}

/// Per-builtin convenience wrappers — one per state-mutating dispatcher.
pub fn emit_alias(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Alias,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_galias(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::GAlias,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_salias(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::SAlias,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_function(name: &str, body: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Function,
        name,
        body.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_assign(name: &str, value: &str, ctx: RecordCtx) {
    emit(
        DefKind::Assign,
        name,
        Some(value.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_typeset(name: &str, flags_value: &str, ctx: RecordCtx) {
    emit(
        DefKind::Typeset,
        name,
        Some(flags_value.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_export(name: &str, value: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Export,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_path_mod(name: &str, op: &str, ctx: RecordCtx) {
    emit(
        DefKind::PathMod,
        name,
        Some(op.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_hash_d(name: &str, path: &str, ctx: RecordCtx) {
    emit(
        DefKind::HashD,
        name,
        Some(path.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_zstyle(pattern: &str, rest: &str, ctx: RecordCtx) {
    emit(
        DefKind::Zstyle,
        pattern,
        Some(rest.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_bindkey(seq: &str, widget: &str, ctx: RecordCtx) {
    emit(
        DefKind::Bindkey,
        seq,
        Some(widget.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_compdef(func: &str, cmds: &str, ctx: RecordCtx) {
    emit(
        DefKind::Compdef,
        func,
        Some(cmds.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_zmodload(module: &str, flags: &str, ctx: RecordCtx) {
    emit(
        DefKind::Zmodload,
        module,
        Some(flags.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_setopt(opt: &str, ctx: RecordCtx) {
    emit(
        DefKind::Setopt,
        opt,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_unsetopt(opt: &str, ctx: RecordCtx) {
    emit(
        DefKind::Unsetopt,
        opt,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_trap(sig: &str, handler: &str, ctx: RecordCtx) {
    emit(
        DefKind::Trap,
        sig,
        Some(handler.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_sched(when: &str, cmd: &str, ctx: RecordCtx) {
    emit(
        DefKind::Sched,
        when,
        Some(cmd.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_source(path: &str, ctx: RecordCtx) {
    emit(
        DefKind::Source,
        path,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_unalias(name: &str, ctx: RecordCtx) {
    emit(
        DefKind::Unalias,
        name,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_unset(name: &str, ctx: RecordCtx) {
    emit(
        DefKind::Unset,
        name,
        None,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}

/// End-of-run summary printed to stderr right before the IPC bundle
/// is sent. Counts per `kind` plus totals. Called from `atexit`, so
/// must avoid touching anything backed by thread-locals (tracing's
/// dispatch + once_cell::Lazy can both raise AccessError once Rust
/// starts tearing down TLS).
pub fn print_summary() {
    if !is_enabled() {
        return;
    }
    let buf = match BUFFER.try_lock() {
        Ok(b) => b,
        Err(_) => return,
    };
    let total = buf.len();
    let mut counts: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    for ev in buf.iter() {
        *counts.entry(ev.kind.as_str()).or_insert(0) += 1;
    }
    let started = START_NS.load(Ordering::Relaxed);
    let elapsed_ms = if started > 0 {
        (now_ns().saturating_sub(started)) / 1_000_000
    } else {
        0
    };
    eprintln!();
    eprintln!("--- zshrs-recorder summary ---");
    eprintln!("  total events: {}", total);
    for (k, v) in &counts {
        eprintln!("  {:<10} {}", k, v);
    }
    eprintln!("  elapsed:      {} ms", elapsed_ms);
}

/// Bundle every captured event, send a single IPC frame to
/// `zshrs-daemon`'s `recorder_ingest` op, and clear the buffer. Returns
/// `true` if the daemon accepted the bundle. Called from `atexit`, so
/// avoids tracing/TLS-touching helpers (use `eprintln!` only).
#[cfg(feature = "daemon")]
pub fn flush_to_daemon() -> bool {
    if !is_enabled() {
        return false;
    }
    if daemon_disabled() {
        return false;
    }
    let events = match BUFFER.try_lock() {
        Ok(mut b) => std::mem::take(&mut *b),
        Err(_) => return false,
    };
    if events.is_empty() {
        eprintln!("recorder: no events to flush");
        return false;
    }
    let bundle = RecorderBundle {
        started_at_ns: START_NS.load(Ordering::Relaxed),
        finished_at_ns: now_ns(),
        cmdline: std::env::args().collect::<Vec<_>>().join(" "),
        zdotdir: std::env::var("ZDOTDIR").ok(),
        home: std::env::var("HOME").ok(),
        events,
    };
    let event_count = bundle.events.len();
    let payload = match serde_json::to_value(&bundle) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("recorder: bundle serialize failed: {}", e);
            return false;
        }
    };
    let t0 = Instant::now();
    // `call_once` (not `_no_spawn`) so the recorder spawns the daemon if
    // it isn't already running — first-time-setup runs must succeed even
    // on a cold machine.
    match crate::daemon::client::call_once("recorder_ingest", payload) {
        Ok(_) => {
            let took_ms = t0.elapsed().as_millis();
            eprintln!(
                "recorder: bundled {} events to daemon in {} ms",
                event_count, took_ms
            );
            true
        }
        Err(e) => {
            eprintln!("recorder: daemon ingest failed: {}", e);
            false
        }
    }
}

#[cfg(not(feature = "daemon"))]
pub fn flush_to_daemon() -> bool {
    if !is_enabled() {
        return false;
    }
    eprintln!("recorder: daemon feature off — bundle not sent");
    false
}

extern "C" fn atexit_finalize() {
    // libc atexit runs AFTER the Rust runtime starts tearing down
    // thread-locals. `tracing::*` and `once_cell::Lazy` both touch TLS,
    // so we must catch the AccessError they raise during destruction —
    // an unwinding panic from an `extern "C"` function aborts the
    // process and swallows the very output the user is here to see.
    let _ = std::panic::catch_unwind(|| {
        print_summary();
    });
    let _ = std::panic::catch_unwind(|| {
        flush_to_daemon();
    });
}

/// Register `print_summary` + `flush_to_daemon` as a libc `atexit` hook
/// so they run even when the shell exits via `std::process::exit`.
pub fn install_atexit() {
    // SAFETY: `atexit_finalize` is a plain `extern "C"` function with the
    // libc-required signature.
    unsafe {
        libc::atexit(atexit_finalize);
    }
}
