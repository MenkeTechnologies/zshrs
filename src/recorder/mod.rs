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

/// Suppress the per-event "Captured KIND NAME ..." stderr line. Set
/// by `zshrs-recorder --quiet`. The summary footer + tracing log still
/// fire — only the live-capture firehose is muted.
static QUIET: AtomicBool = AtomicBool::new(false);

/// Emit the end-of-run summary as a single JSON line to stdout
/// instead of the multi-line human text on stderr. Set by
/// `zshrs-recorder --json`.
static JSON_SUMMARY: AtomicBool = AtomicBool::new(false);

/// Optional path to write the bundle to as a JSON file (alongside
/// the daemon ship-out, or instead of it under --no-daemon). Set by
/// `zshrs-recorder -o PATH`. Lock contention is irrelevant — we set
/// it once at startup and read it once at flush time.
static OUTPUT_PATH: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

/// Optional override for the bundle's `shell_id`. Lets a test (or a
/// rebrand experiment) impersonate a different shell. None = default
/// "zshrs".
static SHELL_ID_OVERRIDE: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

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
    /// `zle -N WIDGET [FUNC]` — define a new ZLE widget. Distinct from
    /// `bindkey` (which binds a key sequence to a widget) and from
    /// `function` (which defines the underlying handler). Tracked
    /// because zinit-report lists widgets and a `zwhere` query for
    /// widgets is the natural counterpart.
    Zle,
    /// `_completion-name` file discovered in an fpath directory. zinit-
    /// report's "Completions:" section lists these per plugin; the
    /// recorder synthesises one event per `_*` file found whenever an
    /// fpath dir is added (set or appended). Distinct from `compdef`
    /// (which BINDS a completion function to a command — runtime call)
    /// and from `function` (the autoload registration that compinit
    /// will emit when it walks fpath). value field is the absolute
    /// path of the discovered file.
    Completion,
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
            DefKind::Zle => "zle",
            DefKind::Completion => "completion",
        }
    }
}

/// Structured parameter-attribute bitflags. Mirrors zsh's per-param
/// flag set (params.c PM_*) so the recorder records the full attribute
/// vector rather than encoding flags in the value string. Only the
/// `typeset` family populates this; other kinds set it to 0.
///
/// Wire-serialised as `u16` for compactness (zsh has ~12 user-visible
/// attribute flags; one byte would also fit but u16 leaves room for
/// PM_HASHELEM / PM_NAMEDDIR / PM_AUTOLOAD-style additions later
/// without a wire-format bump).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamAttrs(pub u16);

impl ParamAttrs {
    pub const NONE: Self = Self(0);
    pub const SCALAR: u16 = 1 << 0;
    pub const INTEGER: u16 = 1 << 1;
    pub const FLOAT: u16 = 1 << 2;
    pub const ASSOC: u16 = 1 << 3;
    pub const ARRAY: u16 = 1 << 4;
    pub const READONLY: u16 = 1 << 5;
    pub const EXPORT: u16 = 1 << 6;
    pub const GLOBAL: u16 = 1 << 7;
    pub const UNIQUE: u16 = 1 << 8;
    pub const TIED: u16 = 1 << 9;
    pub const HIDE: u16 = 1 << 10;
    pub const HIDE_VAL: u16 = 1 << 11;
    /// `+=` operation marker. Distinguishes `arr=(a b)` (replace) from
    /// `arr+=(c)` (extend). Replay needs this to drive the right
    /// codepath when reconstructing array state from the bundle.
    pub const APPEND: u16 = 1 << 12;

    pub fn set(&mut self, mask: u16) {
        self.0 |= mask;
    }
    pub fn has(self, mask: u16) -> bool {
        self.0 & mask != 0
    }

    /// Parse a zsh `typeset -...` flag-letter sequence ("xrigU" etc.)
    /// into a ParamAttrs bitset. Includes letters from `typeset`,
    /// `integer`, `float`, `readonly`, `local`, `declare`, `export`.
    /// Letters not in the table are silently ignored.
    pub fn from_flag_chars(letters: &str) -> Self {
        let mut a = Self::NONE;
        for c in letters.chars() {
            match c {
                'i' => a.set(Self::INTEGER),
                'F' | 'E' => a.set(Self::FLOAT),
                'A' => a.set(Self::ASSOC),
                'a' => a.set(Self::ARRAY),
                'r' => a.set(Self::READONLY),
                'x' => a.set(Self::EXPORT),
                'g' => a.set(Self::GLOBAL),
                'U' => a.set(Self::UNIQUE),
                'T' => a.set(Self::TIED),
                'h' => a.set(Self::HIDE),
                'H' => a.set(Self::HIDE_VAL),
                _ => {}
            }
        }
        // If no concrete shape was set, mark as scalar (typeset
        // defaults to scalar string semantics).
        if a.0 & (Self::INTEGER | Self::FLOAT | Self::ASSOC | Self::ARRAY) == 0 {
            a.set(Self::SCALAR);
        }
        a
    }
}

/// One state-mutation event. Field set is the recorder's wire format
/// for the daemon `recorder_ingest` op; mirrors the SQL `definitions`
/// row in docs/RECORDER.md §Schema.
///
/// REPLAY-GRADE TYPE INFO. Every assign event carries enough structure
/// to round-trip the parameter exactly:
///
///   - `attrs`: `ParamAttrs` bitset — scalar/integer/float/assoc/array/
///     readonly/export/global/unique/tied. Populated on every emit_*
///     by inspecting the executor's current type for `name` (or the
///     declared type for typeset-family). Without this, replay can't
///     tell whether `EDITOR=vim` should restore as scalar or as a
///     previously-tied array.
///   - `value`: scalar form. Set for scalar / integer / float and as
///     a fallback for arrays/assocs (joined string).
///   - `value_array`: ORDERED element list for indexed-array events.
///     Empty for scalars/assocs. Lets replay reconstruct
///     `arr=(elem1 elem2 elem3)` exactly — the order is preserved
///     even when zsh's `typeset -U` would dedupe.
///   - `value_assoc`: ORDERED (key, value) pairs for assoc events.
///     Empty for scalars/arrays. Insertion order matters for replay
///     determinism (zsh assocs are insertion-ordered).
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
    #[serde(default, skip_serializing_if = "is_default_attrs")]
    pub attrs: ParamAttrs,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_array: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_assoc: Option<Vec<(String, String)>>,
    /// Recording-shell identity for the federated catalog (per
    /// docs/DAEMON_AS_SERVICE.md §"Third-party shell recorders" +
    /// `docs/SHELL_IDS.md`). Distinguishes records from different
    /// shells writing to the same daemon. None = inherit from the
    /// enclosing `RecorderBundle.shell_id` (defaults to "zshrs"
    /// at ingest time when neither is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_id: Option<String>,
}

fn is_default_attrs(a: &ParamAttrs) -> bool {
    a.0 == 0
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
    /// Federated-catalog shell identity. Falls back to "zshrs" if not
    /// supplied. Per-event `shell_id` overrides this top-level value
    /// for individual records (lets one bundle carry mixed sources).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell_id: Option<String>,
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

#[inline]
pub fn set_quiet(v: bool) {
    QUIET.store(v, Ordering::Relaxed);
}

#[inline]
fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

#[inline]
pub fn set_json_summary(v: bool) {
    JSON_SUMMARY.store(v, Ordering::Relaxed);
}

#[inline]
fn json_summary_enabled() -> bool {
    JSON_SUMMARY.load(Ordering::Relaxed)
}

pub fn set_output_path(p: Option<String>) {
    if let Ok(mut g) = OUTPUT_PATH.lock() {
        *g = p;
    }
}

fn output_path() -> Option<String> {
    OUTPUT_PATH.lock().ok().and_then(|g| g.clone())
}

pub fn set_shell_id_override(s: Option<String>) {
    if let Ok(mut g) = SHELL_ID_OVERRIDE.lock() {
        *g = s;
    }
}

fn shell_id_override() -> Option<String> {
    SHELL_ID_OVERRIDE.lock().ok().and_then(|g| g.clone())
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
    emit_with_attrs(kind, name, value, file, line, fn_chain, ParamAttrs::NONE)
}

/// Push a record with explicit ParamAttrs. Used by typeset-family
/// dispatchers so the structured attrs ride alongside the record.
pub fn emit_with_attrs(
    kind: DefKind,
    name: impl Into<String>,
    value: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    fn_chain: Option<String>,
    attrs: ParamAttrs,
) {
    emit_full(kind, name, value, file, line, fn_chain, attrs, None, None)
}

/// Full emit with replay-grade structured payload. Array and assoc
/// hooks call this directly so element ordering / key-value pairs
/// survive the wire trip to the daemon for exact reconstruction.
#[allow(clippy::too_many_arguments)]
pub fn emit_full(
    kind: DefKind,
    name: impl Into<String>,
    value: Option<String>,
    file: Option<String>,
    line: Option<u32>,
    fn_chain: Option<String>,
    attrs: ParamAttrs,
    value_array: Option<Vec<String>>,
    value_assoc: Option<Vec<(String, String)>>,
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
    let attrs_part = if attrs.0 == 0 {
        String::new()
    } else {
        format!(" [{}]", attrs_to_str(attrs))
    };
    if !quiet() {
        eprintln!(
            "Captured {} {}{}{}, file: {}{}",
            kind_str, name, attrs_part, value_part, loc, chain
        );
    }
    tracing::info!(
        kind = kind_str,
        %name,
        value = value.as_deref().unwrap_or(""),
        attrs = attrs.0,
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
        attrs,
        value_array,
        value_assoc,
        // Per-event shell_id stays None — the bundle's top-level
        // shell_id at flush time tells the daemon which shell these
        // events came from. Shaving N bytes per event matters at
        // recorder scale (zpwr ingest = ~20k events).
        shell_id: None,
    };
    if let Ok(mut buf) = BUFFER.lock() {
        buf.push(ev);
    }
    IN_RECORDER.store(false, Ordering::Release);
}

/// Format ParamAttrs as a comma-joined human label for the realtime
/// stderr line (`[scalar,export,readonly]` etc.). Wire format stays
/// the raw u16.
fn attrs_to_str(a: ParamAttrs) -> String {
    let mut parts: Vec<&'static str> = Vec::new();
    if a.has(ParamAttrs::INTEGER) {
        parts.push("integer");
    }
    if a.has(ParamAttrs::FLOAT) {
        parts.push("float");
    }
    if a.has(ParamAttrs::ASSOC) {
        parts.push("assoc");
    }
    if a.has(ParamAttrs::ARRAY) {
        parts.push("array");
    }
    if a.has(ParamAttrs::SCALAR) && parts.is_empty() {
        parts.push("scalar");
    }
    if a.has(ParamAttrs::READONLY) {
        parts.push("readonly");
    }
    if a.has(ParamAttrs::EXPORT) {
        parts.push("export");
    }
    if a.has(ParamAttrs::GLOBAL) {
        parts.push("global");
    }
    if a.has(ParamAttrs::UNIQUE) {
        parts.push("unique");
    }
    if a.has(ParamAttrs::TIED) {
        parts.push("tied");
    }
    if a.has(ParamAttrs::HIDE) {
        parts.push("hide");
    }
    if a.has(ParamAttrs::HIDE_VAL) {
        parts.push("hideval");
    }
    if a.has(ParamAttrs::APPEND) {
        parts.push("append");
    }
    parts.join(",")
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

/// Scalar-assign with structured attrs. Call sites that have already
/// inspected the executor for the parameter's declared type
/// (`var_attrs.kind`, `readonly`, `export`) pass a populated
/// `ParamAttrs` here so the recorded event tells replay exactly how
/// to declare the variable. Without populated attrs, replay would
/// have to guess scalar vs array vs integer from the value string —
/// guesses break when `EDITOR=vim` was previously declared `typeset
/// -gx EDITOR`, since plain replay loses the export/global bits.
pub fn emit_assign_typed(name: &str, value: &str, attrs: ParamAttrs, ctx: RecordCtx) {
    emit_with_attrs(
        DefKind::Assign,
        name,
        Some(value.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
    );
}

/// Indexed-array assign with the ordered element list preserved.
/// `is_append` controls a SET vs APPEND attribute bit so replay knows
/// whether to start the array fresh or extend an existing one.
pub fn emit_array_assign(
    name: &str,
    elements: Vec<String>,
    mut attrs: ParamAttrs,
    is_append: bool,
    ctx: RecordCtx,
) {
    attrs.set(ParamAttrs::ARRAY);
    if is_append {
        attrs.set(ParamAttrs::APPEND);
    }
    let joined = elements.join(" ");
    emit_full(
        DefKind::Assign,
        name,
        Some(joined),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
        Some(elements),
        None,
    );
}

/// Assoc-array assign with insertion-ordered (key, value) pairs.
/// `is_append` distinguishes `h=(...)` (replace) from `h+=(...)`
/// / `h[k]=v` element-add semantics.
pub fn emit_assoc_assign(
    name: &str,
    pairs: Vec<(String, String)>,
    mut attrs: ParamAttrs,
    is_append: bool,
    ctx: RecordCtx,
) {
    attrs.set(ParamAttrs::ASSOC);
    if is_append {
        attrs.set(ParamAttrs::APPEND);
    }
    // Joined key/value pairs as the scalar fallback for clients that
    // only read `value`.
    let joined = pairs
        .iter()
        .flat_map(|(k, v)| [k.as_str(), v.as_str()])
        .collect::<Vec<_>>()
        .join(" ");
    emit_full(
        DefKind::Assign,
        name,
        Some(joined),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
        None,
        Some(pairs),
    );
}
/// Backwards-compat: legacy emit_typeset used by sites that haven't
/// migrated to structured attrs yet. Parses the leading flags out of
/// `flags_value` (everything starting with `-`) and routes to the
/// attrs-aware emitter. New call sites should use `emit_typeset_attrs`.
pub fn emit_typeset(name: &str, flags_value: &str, ctx: RecordCtx) {
    let mut letters = String::new();
    let mut value_part = String::new();
    let mut iter = flags_value.split_whitespace();
    while let Some(tok) = iter.next() {
        if let Some(rest) = tok.strip_prefix('-') {
            letters.push_str(rest);
        } else if let Some(rest) = tok.strip_prefix('+') {
            // +F means "unset float attribute"; for the recorder we
            // don't currently distinguish set/unset of attrs — record
            // the letters as-is.
            letters.push_str(rest);
        } else {
            if !value_part.is_empty() {
                value_part.push(' ');
            }
            value_part.push_str(tok);
        }
    }
    let attrs = ParamAttrs::from_flag_chars(&letters);
    let value_opt = if value_part.is_empty() {
        None
    } else {
        Some(value_part)
    };
    emit_with_attrs(
        DefKind::Typeset,
        name,
        value_opt,
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
    );
}

/// Typed typeset emitter — call sites that already have attrs in hand
/// (e.g. `builtin_integer` knows it's emitting an integer) skip the
/// flag-string round-trip.
pub fn emit_typeset_attrs(name: &str, value: Option<&str>, attrs: ParamAttrs, ctx: RecordCtx) {
    emit_with_attrs(
        DefKind::Typeset,
        name,
        value.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
        attrs,
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
pub fn emit_zle(widget: &str, func: Option<&str>, ctx: RecordCtx) {
    emit(
        DefKind::Zle,
        widget,
        func.map(str::to_string),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}
pub fn emit_completion(name: &str, abs_path: &str, ctx: RecordCtx) {
    emit(
        DefKind::Completion,
        name,
        Some(abs_path.to_string()),
        ctx.file,
        ctx.line,
        ctx.fn_chain,
    );
}

/// Walk `dir` and emit one `Completion` event per `_*` file found —
/// matches what zinit-report surfaces in its "Completions:" section.
/// Called from the path/fpath array hook whenever an fpath dir is
/// added (set or appended). Filesystem I/O — bounded to the directory
/// the user just registered, so the cost is paid once per fpath edit
/// (typically tens of files per plugin, hundreds for big completion
/// trees like zsh-more-completions).
///
/// Filename rule (matches compinit/compaudit at zsh/Src/Zle/comp1.c):
/// every entry starting with `_` and not a directory is a candidate.
/// Symlinks are dereferenced. Hidden subdirs and `.zwc` files are
/// skipped. Errors (unreadable dir, EACCES) are tracing-warn'd and
/// otherwise silent — this is a best-effort surfacing layer.
pub fn discover_completions_in_fpath_dir(dir: &str, ctx: &RecordCtx) {
    if !is_enabled() {
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!(?e, dir, "recorder: completion discovery skipped");
            return;
        }
    };
    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if !name.starts_with('_') {
            continue;
        }
        if name.ends_with(".zwc") {
            continue;
        }
        let path = entry.path();
        // file_type may need to follow a symlink to know if it's a dir.
        let is_file = match entry.file_type() {
            Ok(ft) if ft.is_symlink() => std::fs::metadata(&path)
                .map(|m| m.is_file())
                .unwrap_or(false),
            Ok(ft) => ft.is_file(),
            Err(_) => false,
        };
        if !is_file {
            continue;
        }
        let abs = path.to_string_lossy().to_string();
        emit_completion(&name, &abs, ctx.clone());
    }
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
    if json_summary_enabled() {
        // Single JSON line to stdout — pipes cleanly into jq / scripts.
        let mut counts_pairs = Vec::with_capacity(counts.len());
        for (k, v) in &counts {
            counts_pairs.push(format!("\"{}\":{}", k, v));
        }
        println!(
            "{{\"total_events\":{},\"elapsed_ms\":{},\"counts\":{{{}}}}}",
            total,
            elapsed_ms,
            counts_pairs.join(",")
        );
    } else {
        eprintln!();
        eprintln!("--- zshrs-recorder summary ---");
        eprintln!("  total events: {}", total);
        for (k, v) in &counts {
            eprintln!("  {:<10} {}", k, v);
        }
        eprintln!("  elapsed:      {} ms", elapsed_ms);
    }
}

/// Bundle every captured event, send a single IPC frame to
/// `zshrs-daemon`'s `recorder_ingest` op, and clear the buffer. Returns
/// `true` if the daemon accepted the bundle. Called from `atexit`, so
/// avoids tracing/TLS-touching helpers (use `eprintln!` only).
///
/// `--no-daemon` skips the IPC step but `-o PATH` still writes the
/// bundle to disk so post-mortem inspection works without a daemon.
#[cfg(feature = "daemon")]
pub fn flush_to_daemon() -> bool {
    if !is_enabled() {
        return false;
    }
    let events = match BUFFER.try_lock() {
        Ok(mut b) => std::mem::take(&mut *b),
        Err(_) => return false,
    };
    let events_empty = events.is_empty();
    let bundle = RecorderBundle {
        started_at_ns: START_NS.load(Ordering::Relaxed),
        finished_at_ns: now_ns(),
        cmdline: std::env::args().collect::<Vec<_>>().join(" "),
        zdotdir: std::env::var("ZDOTDIR").ok(),
        home: std::env::var("HOME").ok(),
        events,
        // `--shell-id ID` overrides this so a recorder run can
        // impersonate a non-zshrs source (federation testing,
        // rebrand experiments). Default = "zshrs" since this code
        // path is exclusive to the AOP-instrumented zshrs-recorder.
        shell_id: Some(shell_id_override().unwrap_or_else(|| "zshrs".to_string())),
    };

    // `-o PATH` writes the bundle to a JSON file alongside (or instead
    // of, under --no-daemon) shipping it to the daemon. Useful for
    // post-mortem inspection without spinning a daemon up. Empty
    // bundles still write — caller asked for the file, give them the
    // file (even if it just confirms "recorder ran, captured nothing"
    // — that itself is a useful diagnostic when a parse error caused
    // zero events).
    if let Some(path) = output_path() {
        match serde_json::to_string(&bundle) {
            Ok(s) => {
                if let Err(e) = std::fs::write(&path, s.as_bytes()) {
                    eprintln!("recorder: write {path}: {e}");
                } else {
                    eprintln!("recorder: bundle written to {path}");
                }
            }
            Err(e) => eprintln!("recorder: bundle serialize for output failed: {e}"),
        }
    }

    if events_empty {
        eprintln!("recorder: no events to flush");
        return false;
    }

    if daemon_disabled() {
        return false;
    }
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
