//! Provenance engine — value lineage across bytecode execution.
//!
//! **zshrs-original — no C zsh counterpart.** C zsh has no notion of
//! where a parameter's bytes came from; `typeset -p` shows the current
//! value and nothing about the chain of expansions, substitutions and
//! assignments that produced it. This module is the ledger that answers
//! "where did this value come from?" for a running shell.
//!
//! Ported from stryke's `strykelang/provenance.rs` (the `mark` /
//! `provenance` / `unmark` builtin trio). Three things change in the
//! shell port:
//!
//! 1. **Two key spaces instead of one.** stryke keys purely on the
//!    `Arc<HeapObject>` pointer of a value, because every stryke value
//!    stays an `Arc` from creation to use. A shell value does not: the
//!    VM/host boundary (`ShellHost::exec`, `::cmd_subst`, `::glob`)
//!    passes `String`, and the parameter table
//!    (`ported::params::assignsparam`) stores `String`, so the `Arc`
//!    identity dies at the first assignment. stryke documents that hole
//!    for its own string results ("the VM's scalar-return path re-Arcs
//!    the string"); in a shell it is the common case, not the corner
//!    case. So this port keys on three things:
//!      * `Ptr`     — `Arc` identity of an in-flight `fusevm::Value`
//!                    (`Str(Arc<String>)` / `Array(Arc<Vec<Value>>)`),
//!                    the direct stryke mechanism, valid inside a chunk.
//!      * `Name`    — a tracked parameter name; survives every
//!                    assignment/expansion round trip.
//!      * `Content` — the exact bytes of a value that crossed a
//!                    `String`-typed host boundary, so a command
//!                    substitution's output can still be recognised when
//!                    it lands in `assignsparam` a few ops later.
//! 2. **Origins are created by shell events, not by a `mark()` call on a
//!    heap object.** `$(...)`, `<(...)`, glob expansion and an explicitly
//!    tracked parameter are the origin kinds.
//! 3. **The engine is off unless armed.** `PROV_ACTIVE` is an
//!    `AtomicBool` flipped on by `provenance -m NAME` (stryke flips it in
//!    `mark`). Every hook is a single relaxed load when nobody armed it,
//!    and the whole engine can be disabled outright in
//!    `~/.zshrs/zshrs.toml` (`[provenance] enabled = false`) or with
//!    `ZSHRS_PROVENANCE=0`, in which case arming is refused and the
//!    hooks can never fire.
//!
//! Staleness handling is stryke's v1.1 design, ported as-is: a `Ptr`
//! entry stores a `Weak` alongside the raw address, and a lookup that
//! cannot upgrade the `Weak` (or upgrades it to a *different* address)
//! reaps the entry instead of reporting a lineage that belongs to a
//! long-dead allocation the allocator has since recycled.
//!
//! `Content` entries are speculative — recorded before anyone knows
//! whether the value will ever reach a tracked name — so they are
//! bounded by a FIFO of [`CONTENT_CAP`] entries. `Name` and `Ptr`
//! entries are not speculative and are only created for values that are
//! already tracked.
//!
//! Independent of the PFA-SMR recorder (`src/recorder/`): no daemon, no
//! catalog, no bundle emission. The recorder answers "what state did
//! this shell define?"; this answers "how was this value built?".

use chrono::{Local, NaiveDate, SecondsFormat, TimeZone};
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use fusevm::Value;

/// Maximum number of speculative content-keyed entries kept alive.
/// Each command substitution, process substitution and glob expansion
/// records one while the engine is armed; the FIFO drops the oldest
/// past this bound so a long-running armed shell cannot grow the ledger
/// without limit.
pub const CONTENT_CAP: usize = 8192;

/// Maximum ops kept on one chain. A tracked parameter that the shell
/// itself reads on every prompt (`PATH`, `PS1`) would otherwise grow an
/// unbounded chain; past the cap the ops are counted, not stored.
pub const MAX_OPS: usize = 256;

/// Ceiling on names [`track_all`] may arm by itself. A shell that runs
/// long enough touches an unbounded number of parameters and functions;
/// past the ceiling new names are counted and ignored, so a
/// track-everything session cannot grow the ledger without limit.
pub const MAX_AUTO_NAMES: usize = 4096;

/// Parameters [`track_all`] never arms: the shell rewrites these on its
/// own, once or more per command, so their chains would record the
/// shell's own bookkeeping and nothing the user did. Positional
/// parameters (`1`, `2`, …) are skipped by the same rule, tested
/// numerically rather than listed.
const VOLATILE: &[&str] = &[
    "_",
    "?",
    "!",
    "$",
    "#",
    "COLUMNS",
    "EPOCHREALTIME",
    "EPOCHSECONDS",
    "HISTCMD",
    "LINENO",
    "LINES",
    "RANDOM",
    "SECONDS",
    "pipestatus",
    "status",
];

/// Longest value prefix stored as a content key. Content keying hashes
/// the whole string, but the *summary* strings kept in the ledger are
/// truncated to this so the ledger stays small next to the values it
/// describes.
const SUMMARY_MAX: usize = 64;

/// Flipped to `true` the first time a name is tracked or a value is
/// marked. Every hook checks it via [`active`] — one inlined relaxed
/// load on the universal path where nobody armed the engine.
static PROV_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Track-everything mode: every parameter write and every shell function
/// arms itself. Set from `[provenance] track_all` at startup, or by
/// `provenance -a` at runtime.
static TRACK_ALL: AtomicBool = AtomicBool::new(false);

/// Last `$LINENO` seen by [`note_line`]. Updated from the
/// `BUILTIN_SET_LINENO` bytecode handler while armed, so lineage
/// records carry a source line without any hook having to take the
/// parameter-table lock (which the param write hooks run inside).
static CURRENT_LINE: AtomicUsize = AtomicUsize::new(0);

/// Where and when a tap fired. Every origin and every op carries one, so
/// a chain reads as a timeline: which file and line produced the bytes,
/// and at what wall-clock instant.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Site {
    /// `$LINENO` as last mirrored by [`note_line`], or 0 when unknown.
    pub line: usize,
    /// File the line belongs to: the file a shell function was defined
    /// in while one is on the stack, otherwise the script being run or
    /// sourced. `None` for `-c` input and interactive lines, which have
    /// no file.
    pub file: Option<String>,
    /// Shell function the op ran inside, when there is one. Without it
    /// a line number taken inside a function — which zsh counts from the
    /// function's own first line — has nothing to anchor it.
    pub func: Option<String>,
    /// Wall clock, milliseconds since the Unix epoch.
    pub time_ms: i64,
}

impl Site {
    /// Capture the current site. Called at hook entry, before the ledger
    /// lock is taken, so the `scriptfilename` read never nests inside it.
    fn now() -> Self {
        // Inside a function `$LINENO` counts from the function's own
        // first line, so the line is only meaningful next to the file
        // the function was defined in, offset by where that definition
        // starts — `funcstack`'s `f->prev->flineno + f->lineno`
        // (`Src/Modules/parameter.c:747`), the same sum `funcfiletrace`
        // reports. Outside one, `$LINENO` already indexes the script.
        let frame = current_function();
        let (func, file, line) = match frame {
            Some((name, file, flineno)) => (
                Some(name),
                file,
                current_line() + flineno.max(0) as usize,
            ),
            None => (
                None,
                crate::ported::utils::scriptfilename_get(),
                current_line(),
            ),
        };
        Self {
            line,
            // `zsh -c` stamps the literal "zsh" into `scriptfilename`
            // (`Src/init.c:479`), which is a shell name, not a file. A
            // chain says `line N` there rather than naming a file that
            // does not exist.
            file: file.filter(|f| f != "zsh"),
            func,
            time_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        }
    }

    /// Same source position — the comparison [`ProvNode::push_op`] uses
    /// to collapse an immediate repeat, which must ignore the clock or
    /// no two ops would ever compare equal.
    fn same_position(&self, other: &Self) -> bool {
        self.line == other.line && self.file == other.file && self.func == other.func
    }

    /// `file:line` when the line belongs to a file, `line N` otherwise,
    /// with the enclosing function appended when the op ran inside one.
    pub fn location(&self) -> String {
        let base = match &self.file {
            Some(f) => format!("{}:{}", f, self.line),
            None => format!("line {}", self.line),
        };
        match &self.func {
            Some(fun) => format!("{} ({})", base, fun),
            None => base,
        }
    }

    /// Local wall clock. `with_date` spells out the day as well as the
    /// time — used on the origin line, and on any op that happened on a
    /// different day than the origin.
    pub fn clock(&self, with_date: bool) -> String {
        let fmt = if with_date {
            "%Y-%m-%d %H:%M:%S%.3f"
        } else {
            "%H:%M:%S%.3f"
        };
        match Local.timestamp_millis_opt(self.time_ms).single() {
            Some(dt) => dt.format(fmt).to_string(),
            None => String::new(),
        }
    }

    /// RFC 3339 local timestamp with millisecond precision, for `-j`.
    pub fn rfc3339(&self) -> String {
        match Local.timestamp_millis_opt(self.time_ms).single() {
            Some(dt) => dt.to_rfc3339_opts(SecondsFormat::Millis, false),
            None => String::new(),
        }
    }

    /// Calendar day, for deciding whether an op line needs the date.
    fn day(&self) -> Option<NaiveDate> {
        Local
            .timestamp_millis_opt(self.time_ms)
            .single()
            .map(|dt| dt.date_naive())
    }
}

/// One operation in a value's lineage chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvOp {
    /// Operation name — `cmdsubst`, `glob`, `expand`, `assign`, `exec`,
    /// `function`, `origin`, `unset`.
    pub op: String,
    /// Short summaries of the operands, in argument order.
    pub args: Vec<String>,
    /// Where and when the op ran.
    pub site: Site,
}

/// Lineage record for a single value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvNode {
    /// How the value entered the shell — e.g. `cmdsubst "date +%s"`,
    /// `glob "*.rs"`, `param FOO`.
    pub origin: String,
    /// Where and when the value entered the shell.
    pub origin_site: Site,
    /// Append-only chain of operations that touched the value after the
    /// origin. Latest op last.
    pub ops: Vec<ProvOp>,
    /// Tracked parameter this chain belongs to, when it has one. Ops
    /// recorded against a derived value also extend the owner's node, so
    /// `provenance NAME` shows what happened to the value after it left
    /// the parameter.
    pub owner: Option<String>,
    /// Ops that were not stored because the chain hit [`MAX_OPS`].
    pub dropped_ops: usize,
}

impl ProvNode {
    /// New origin node with an empty op chain.
    fn origin(origin: impl Into<String>, site: Site) -> Self {
        Self {
            origin: origin.into(),
            origin_site: site,
            ops: Vec::new(),
            owner: None,
            dropped_ops: 0,
        }
    }

    /// Append an op, collapsing an immediate repeat (same op, operands
    /// and line — a value read several times while evaluating one
    /// statement) and counting instead of storing past [`MAX_OPS`].
    fn push_op(&mut self, op: ProvOp) {
        if self
            .ops
            .last()
            .is_some_and(|l| l.op == op.op && l.args == op.args && l.site.same_position(&op.site))
        {
            return;
        }
        if self.ops.len() >= MAX_OPS {
            self.dropped_ops += 1;
            return;
        }
        self.ops.push(op);
    }
}

/// Weak half of a `Ptr` entry. `fusevm::Value`'s heap variants carry
/// different payload types, so the weak reference is per-variant rather
/// than the single `Weak<HeapObject>` stryke gets from its uniform heap.
enum ValueWeak {
    /// Weak half of a `Value::Str(Arc<String>)`.
    Str(Weak<String>),
    /// Weak half of a `Value::Array(Arc<Vec<Value>>)`.
    Array(Weak<Vec<Value>>),
}

impl ValueWeak {
    /// True when the weak reference still resolves to the same address
    /// the entry was keyed on. False means the original allocation is
    /// gone — the address may since have been recycled by an unrelated
    /// value, which is exactly the false positive this check exists to
    /// prevent.
    fn still_at(&self, ptr: usize) -> bool {
        match self {
            ValueWeak::Str(w) => w.upgrade().is_some_and(|a| Arc::as_ptr(&a) as usize == ptr),
            ValueWeak::Array(w) => w.upgrade().is_some_and(|a| Arc::as_ptr(&a) as usize == ptr),
        }
    }
}

/// A `Ptr`-keyed ledger row: the lineage plus the weak reference used to
/// detect address reuse.
struct PtrEntry {
    /// Weak reference to the value this row describes.
    weak: ValueWeak,
    /// Lineage of the value.
    node: ProvNode,
}

/// The ledger. One mutex over all three key spaces — every hook takes it
/// only after [`active`] returned true, so an unarmed shell never
/// contends on it.
#[derive(Default)]
struct Ledger {
    /// In-flight VM values, keyed by `Arc` address.
    ptr: HashMap<usize, PtrEntry>,
    /// Tracked parameters, keyed by name.
    name: HashMap<String, ProvNode>,
    /// Names armed via `provenance -m`, including names that have no
    /// lineage yet (they gain one on the next assignment).
    tracked: HashSet<String>,
    /// Tracked shell functions, keyed by name. Separate from `name` so a
    /// function and a parameter may share a name without sharing a
    /// chain — `path` and `path()` are different things.
    func: HashMap<String, ProvNode>,
    /// Functions armed via `provenance -m -f`, or by [`track_all`].
    tracked_funcs: HashSet<String>,
    /// Names [`track_all`] declined to arm because [`MAX_AUTO_NAMES`]
    /// was already reached.
    auto_dropped: usize,
    /// Values that crossed a `String`-typed host boundary, keyed by a
    /// hash of their exact bytes.
    content: HashMap<u64, ProvNode>,
    /// Insertion order of `content` keys, for the FIFO bound.
    content_order: VecDeque<u64>,
}

fn ledger() -> &'static Mutex<Ledger> {
    static LEDGER: OnceLock<Mutex<Ledger>> = OnceLock::new();
    LEDGER.get_or_init(|| Mutex::new(Ledger::default()))
}

/// Take the ledger lock, recovering from a poisoned mutex. A panic in a
/// hook must not turn every later provenance call into a silent no-op,
/// and no invariant spans the lock — each row is self-contained.
fn lock() -> MutexGuard<'static, Ledger> {
    match ledger().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── Gates ───────────────────────────────────────────────────────────

/// Whether the engine may be armed at all: `[provenance] enabled` in
/// `~/.zshrs/zshrs.toml` (default `true`), with `ZSHRS_PROVENANCE=0` as
/// an environment kill switch that wins over the config. Read once per
/// process — the config snapshot itself is already process-cached.
pub fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        if std::env::var("ZSHRS_PROVENANCE").is_ok_and(|v| v == "0") {
            return false;
        }
        crate::config::current().provenance.enabled
    })
}

/// Whether `[provenance] track_all` (or `ZSHRS_PROVENANCE_ALL`) asked
/// for track-everything mode. `ZSHRS_PROVENANCE_ALL` wins over the file
/// in both directions: `=1` turns it on, `=0` off.
fn track_all_configured() -> bool {
    static CONFIGURED: OnceLock<bool> = OnceLock::new();
    *CONFIGURED.get_or_init(|| match std::env::var("ZSHRS_PROVENANCE_ALL") {
        Ok(v) if v == "1" => true,
        Ok(v) if v == "0" => false,
        _ => crate::config::current().provenance.track_all,
    })
}

/// Arm track-everything mode when the config asks for it. Called once at
/// shell startup — before that the engine is inert whatever the file
/// says, because nothing has read it.
pub fn init_from_config() {
    if enabled() && track_all_configured() {
        set_track_all(true);
    }
}

/// Whether every parameter write and function arms itself.
#[inline]
pub fn track_all() -> bool {
    TRACK_ALL.load(Ordering::Relaxed)
}

/// Turn track-everything mode on or off at runtime (`provenance -a`).
/// Returns false when the engine is disabled by config/env, which is the
/// one thing `-a` cannot override.
pub fn set_track_all(on: bool) -> bool {
    if on && !enabled() {
        return false;
    }
    TRACK_ALL.store(on, Ordering::Relaxed);
    if on {
        PROV_ACTIVE.store(true, Ordering::Relaxed);
    } else {
        let l = lock();
        let empty = l.tracked.is_empty()
            && l.name.is_empty()
            && l.tracked_funcs.is_empty()
            && l.func.is_empty();
        drop(l);
        if empty {
            PROV_ACTIVE.store(false, Ordering::Relaxed);
        }
    }
    true
}

/// Whether [`track_all`] may arm `name` by itself: not one of the
/// parameters the shell rewrites on its own, and not a positional.
fn auto_armable(name: &str) -> bool {
    !name.is_empty()
        && !VOLATILE.contains(&name)
        && !name.chars().all(|c| c.is_ascii_digit())
}

/// The hot gate. `false` until something is tracked, so every hook site
/// costs one relaxed load in the universal case.
#[inline]
pub fn active() -> bool {
    PROV_ACTIVE.load(Ordering::Relaxed)
}

/// Record the current source line. Called from the `BUILTIN_SET_LINENO`
/// bytecode handler while armed; deliberately does not read `$LINENO`
/// from the parameter table, because the param-write hooks run with that
/// table locked.
#[inline]
pub fn note_line(line: usize) {
    CURRENT_LINE.store(line, Ordering::Relaxed);
}

/// Line most recently reported by [`note_line`], or 0 when unknown.
pub fn current_line() -> usize {
    CURRENT_LINE.load(Ordering::Relaxed)
}

/// The shell function whose body is executing right now: its name, the
/// file it was defined in, and the file line that definition starts on
/// (`funcsourcetrace`'s pair — `Src/exec.c:1613` fills the frame's
/// filename from `scriptfilename` at call time).
///
/// Only the topmost frame counts. A `source` or `eval` frame above a
/// function makes `$LINENO` count in *that* text instead, so the
/// function's definition line is the wrong thing to add to it.
///
/// Takes the stack with `try_lock`: a hook can fire from anywhere the VM
/// runs, and a lineage row is worth less than a deadlock against the
/// frame push/pop that a blocking lock would risk.
fn current_function() -> Option<(String, Option<String>, i64)> {
    let stack = crate::ported::modules::parameter::FUNCSTACK.try_lock().ok()?;
    let frame = stack.last()?;
    (frame.tp == crate::ported::zsh_h::FS_FUNC)
        .then(|| (frame.name.clone(), frame.filename.clone(), frame.flineno))
}

// ── Summaries ───────────────────────────────────────────────────────

/// Short, single-line description of a string value for a ledger row.
/// The full value stays reachable through the shell itself; the ledger
/// only needs a human-readable handle.
pub fn summarize_str(s: &str) -> String {
    let mut out = String::with_capacity(SUMMARY_MAX + 12);
    out.push('"');
    for ch in s.chars().take(SUMMARY_MAX) {
        match ch {
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    if s.chars().count() > SUMMARY_MAX {
        out.push('…');
    }
    out.push('"');
    out
}

/// Short description of a `fusevm::Value` for a ledger row.
pub fn summarize_value(v: &Value) -> String {
    match v {
        Value::Str(s) => summarize_str(s),
        Value::Array(a) => format!("ARRAY len={}", a.len()),
        Value::Hash(h) => format!("ASSOC entries={}", h.len()),
        Value::Int(n) => n.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Status(c) => format!("STATUS {}", c),
        Value::Undef => "unset".to_string(),
        other => format!("{:?}", other),
    }
}

fn content_key(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// `Arc` address of a value's heap payload, or `None` for immediates
/// (ints, floats, bools, statuses, undef) which have no stable identity
/// to key on.
fn value_ptr(v: &Value) -> Option<usize> {
    match v {
        Value::Str(s) => Some(Arc::as_ptr(s) as usize),
        Value::Array(a) => Some(Arc::as_ptr(a) as usize),
        _ => None,
    }
}

fn value_weak(v: &Value) -> Option<ValueWeak> {
    match v {
        Value::Str(s) => Some(ValueWeak::Str(Arc::downgrade(s))),
        Value::Array(a) => Some(ValueWeak::Array(Arc::downgrade(a))),
        _ => None,
    }
}

// ── Ledger primitives ───────────────────────────────────────────────

impl Ledger {
    /// Look up a `Ptr` row, reaping it when the weak check says the
    /// address was recycled.
    fn ptr_node(&mut self, ptr: usize) -> Option<ProvNode> {
        let live = match self.ptr.get(&ptr) {
            Some(e) => e.weak.still_at(ptr),
            None => return None,
        };
        if !live {
            self.ptr.remove(&ptr);
            return None;
        }
        self.ptr.get(&ptr).map(|e| e.node.clone())
    }

    fn put_ptr(&mut self, v: &Value, node: ProvNode) {
        let (Some(ptr), Some(weak)) = (value_ptr(v), value_weak(v)) else {
            return;
        };
        self.ptr.insert(ptr, PtrEntry { weak, node });
    }

    fn put_content(&mut self, s: &str, node: ProvNode) {
        // Empty strings are the most common value in a shell and carry
        // no useful identity — keying on them would attach a lineage to
        // every unset expansion in the script.
        if s.is_empty() {
            return;
        }
        let key = content_key(s);
        if self.content.insert(key, node).is_none() {
            self.content_order.push_back(key);
            while self.content_order.len() > CONTENT_CAP {
                if let Some(old) = self.content_order.pop_front() {
                    self.content.remove(&old);
                }
            }
        }
    }

    fn content_node(&self, s: &str) -> Option<ProvNode> {
        if s.is_empty() {
            return None;
        }
        self.content.get(&content_key(s)).cloned()
    }

    /// Append `op` to the owning parameter's chain, when the node has an
    /// owner that is still tracked. This is what makes
    /// `provenance NAME` show what happened to the value *after* it left
    /// the parameter.
    fn extend_owner(&mut self, node: &ProvNode, op: &ProvOp) {
        let Some(owner) = node.owner.as_deref() else {
            return;
        };
        if let Some(target) = self.name.get_mut(owner) {
            target.push_op(op.clone());
        }
    }
}

// ── Tracking control (the `provenance` builtin's surface) ───────────

/// Arm tracking for parameter `name`. A parameter that already holds a
/// value gets it as the chain's origin, so lineage starts at "what it
/// holds now"; an unset one takes its origin from its first assignment.
/// Returns false when the engine is disabled by config/env.
pub fn track_name(name: &str, current_value: Option<&str>) -> bool {
    if !enabled() {
        return false;
    }
    let site = Site::now();
    let mut l = lock();
    l.tracked.insert(name.to_string());
    // A parameter that already holds a value gets that value as its
    // origin. An unset one gets no node at all: its first assignment
    // supplies the origin, so `X=$(date)` reads as a cmdsubst origin
    // rather than a placeholder the assignment has to argue with.
    if let Some(v) = current_value {
        let mut node = ProvNode::origin(format!("param {} = {}", name, summarize_str(v)), site);
        node.owner = Some(name.to_string());
        l.name.insert(name.to_string(), node);
    }
    drop(l);
    PROV_ACTIVE.store(true, Ordering::Relaxed);
    true
}

/// Drop tracking and lineage for `name`. Idempotent; returns true when
/// something was actually removed.
pub fn untrack_name(name: &str) -> bool {
    let mut l = lock();
    let had = l.tracked.remove(name) | l.name.remove(name).is_some();
    let empty = l.tracked.is_empty()
        && l.name.is_empty()
        && l.tracked_funcs.is_empty()
        && l.func.is_empty();
    drop(l);
    // Track-everything mode keeps the engine armed: the next write to
    // any parameter arms it again, so disarming here would only cost the
    // rows between now and then.
    if empty && !track_all() {
        PROV_ACTIVE.store(false, Ordering::Relaxed);
    }
    had
}

/// Drop every ledger entry and disarm the engine, track-everything mode
/// included — `provenance -c` is the full stop.
pub fn clear() {
    let mut l = lock();
    *l = Ledger::default();
    drop(l);
    TRACK_ALL.store(false, Ordering::Relaxed);
    PROV_ACTIVE.store(false, Ordering::Relaxed);
}

/// Names currently armed, sorted.
pub fn tracked_names() -> Vec<String> {
    let l = lock();
    let mut v: Vec<String> = l.tracked.iter().cloned().collect();
    v.sort();
    v
}

/// Lineage of a tracked parameter, or `None` when it is not tracked.
pub fn lookup_name(name: &str) -> Option<ProvNode> {
    lock().name.get(name).cloned()
}

/// Lineage attached to an in-flight VM value, or `None`. Stale rows are
/// reaped on lookup.
pub fn lookup_value(v: &Value) -> Option<ProvNode> {
    let ptr = value_ptr(v)?;
    lock().ptr_node(ptr)
}

/// Lineage attached to a raw string that crossed a host boundary.
pub fn lookup_content(s: &str) -> Option<ProvNode> {
    lock().content_node(s)
}

// ── Hook points ─────────────────────────────────────────────────────
//
// Every function below is called from a bytecode/host site and must be
// guarded by `if provenance::active()` at the call site, so an unarmed
// shell pays one relaxed load and nothing else.

/// A command substitution produced `out`. Records a speculative origin
/// so a later assignment of the same bytes can inherit it.
pub fn on_cmd_subst(source: &str, out: &str) {
    let site = Site::now();
    let origin = if source.is_empty() {
        "cmdsubst".to_string()
    } else {
        format!("cmdsubst {}", summarize_str(source))
    };
    lock().put_content(out, ProvNode::origin(origin, site));
}

/// A process substitution produced the path `out` for sub-chunk source
/// `source`.
pub fn on_process_subst(source: &str, out: &str) {
    let site = Site::now();
    let origin = if source.is_empty() {
        "procsubst".to_string()
    } else {
        format!("procsubst {}", summarize_str(source))
    };
    lock().put_content(out, ProvNode::origin(origin, site));
}

/// A glob expanded to `results`. Skips high-fanout expansions: a
/// thousand-file glob would evict the whole content FIFO for matches
/// nobody is tracking.
pub fn on_glob(pattern: &str, results: &[String]) {
    if results.is_empty() || results.len() > 32 {
        return;
    }
    let site = Site::now();
    let mut l = lock();
    for r in results {
        l.put_content(
            r,
            ProvNode::origin(format!("glob {}", summarize_str(pattern)), site.clone()),
        );
    }
}

/// A heredoc / herestring body became the next command's stdin.
pub fn on_heredoc(kind: &str, body: &str) {
    let site = Site::now();
    lock().put_content(body, ProvNode::origin(kind.to_string(), site));
}

/// Parameter `name` was read by an `ExpandParam` bytecode op, producing
/// `value`. When the parameter is tracked, the value carries the
/// parameter's chain forward — by `Arc` identity for the rest of the
/// chunk, and by content for the host boundaries that only see `String`.
pub fn on_param_read(name: &str, value: &Value) {
    let site = Site::now();
    let mut l = lock();
    let Some(mut node) = l.name.get(name).cloned() else {
        return;
    };
    let op = ProvOp {
        op: "expand".to_string(),
        args: vec![format!("${}", name), summarize_value(value)],
        site,
    };
    if let Some(target) = l.name.get_mut(name) {
        target.push_op(op.clone());
    }
    node.push_op(op);
    node.owner = Some(name.to_string());
    l.put_ptr(value, node.clone());
    if let Value::Str(s) = value {
        l.put_content(s, node);
    }
}

/// Two word segments were concatenated by a bytecode concat op. When
/// either operand carries a lineage — by `Arc` identity for a value the
/// VM is still holding, or by content for one that crossed a host
/// boundary — the produced value inherits the richer chain. This is the
/// link that keeps derived values traceable: `G=${F}.bak` does not hold
/// F's bytes, but it was built from them.
pub fn on_concat(lhs: &Value, rhs: &Value, result: &Value) {
    let site = Site::now();
    let mut l = lock();
    let mut chosen: Option<ProvNode> = None;
    for operand in [lhs, rhs] {
        let found = value_ptr(operand)
            .and_then(|p| l.ptr_node(p))
            .or_else(|| match operand {
                Value::Str(s) => l.content_node(s),
                _ => None,
            });
        if let Some(node) = found {
            // Prefer the longer chain, matching stryke's `record_op`
            // parent selection.
            if chosen.as_ref().is_none_or(|c| node.ops.len() > c.ops.len()) {
                chosen = Some(node);
            }
        }
    }
    let Some(mut node) = chosen else {
        return;
    };
    // A segment concat where one side is empty adds no bytes — the word
    // assembler emits one per leading/trailing segment. Propagate the
    // lineage, but do not record an op that changed nothing.
    let is_noop = matches!(lhs, Value::Str(s) if s.is_empty())
        || matches!(rhs, Value::Str(s) if s.is_empty());
    if !is_noop {
        let op = ProvOp {
            op: "concat".to_string(),
            args: vec![summarize_value(lhs), summarize_value(rhs)],
            site,
        };
        l.extend_owner(&node, &op);
        node.push_op(op);
    }
    l.put_ptr(result, node.clone());
    if let Value::Str(s) = result {
        l.put_content(s, node);
    }
}

/// Parameter `name` was assigned. `kind` names the write funnel
/// (`assign`, `assign[]`, `append`, `array`, `assoc`) and `value` is the
/// summary of what was stored. When the assigned bytes already carry a
/// lineage — a command substitution's output, a glob match, another
/// tracked parameter's value — that chain becomes this parameter's
/// origin on the first write, and is spliced into the chain (as an
/// `origin` op followed by its ops) on later ones. Reassignment never
/// discards the parameter's history: the chain is the whole life of the
/// parameter, one write op per assignment.
pub fn on_param_write(name: &str, kind: &str, value: &str) {
    let site = Site::now();
    let mut l = lock();
    if !l.tracked.contains(name) {
        // Track-everything mode arms a parameter the first time the
        // shell writes it, which is also the first moment its chain can
        // say anything.
        if !track_all() || !auto_armable(name) {
            return;
        }
        if l.tracked.len() + l.tracked_funcs.len() >= MAX_AUTO_NAMES {
            l.auto_dropped += 1;
            return;
        }
        l.tracked.insert(name.to_string());
    }
    // A value whose bytes are this parameter's own current value carries
    // this same chain; splicing it back in would duplicate the chain
    // onto itself.
    let inherited = l
        .content_node(value)
        .filter(|n| n.owner.as_deref() != Some(name));
    let mut node = match l.name.remove(name) {
        // The parameter already has a chain: the assignment extends it.
        // A value that arrived with its own lineage contributes that
        // lineage — recorded as an `origin` op, then its ops — instead
        // of replacing the parameter's history.
        Some(mut existing) => {
            if let Some(inh) = inherited {
                existing.push_op(ProvOp {
                    op: "origin".to_string(),
                    args: vec![inh.origin],
                    site: inh.origin_site,
                });
                for op in inh.ops {
                    existing.push_op(op);
                }
            }
            existing
        }
        // First write to a parameter armed while unset: the value's own
        // lineage is the origin, or the assignment itself is.
        None => inherited.unwrap_or_else(|| {
            ProvNode::origin(format!("{} {}", kind, summarize_str(value)), site.clone())
        }),
    };
    node.owner = Some(name.to_string());
    node.push_op(ProvOp {
        op: kind.to_string(),
        args: vec![name.to_string(), summarize_str(value)],
        site,
    });
    l.name.insert(name.to_string(), node.clone());
    l.put_content(value, node);
}

/// Parameter `name` was unset. Keeps the chain (that is the interesting
/// part) and records the unset as its final op.
pub fn on_param_unset(name: &str) {
    let site = Site::now();
    let mut l = lock();
    if let Some(node) = l.name.get_mut(name) {
        node.push_op(ProvOp {
            op: "unset".to_string(),
            args: vec![name.to_string()],
            site,
        });
    }
}

/// A command is about to run with `args` (argv[0] first). Any argument
/// carrying a lineage gets an `exec` op appended, and the op also
/// extends the owning parameter's chain so `provenance NAME` shows where
/// the value was consumed.
pub fn on_exec(kind: &str, args: &[String]) {
    if args.is_empty() {
        return;
    }
    let site = Site::now();
    let cmd = args[0].clone();
    let mut l = lock();
    for (i, a) in args.iter().enumerate().skip(1) {
        let Some(mut node) = l.content_node(a) else {
            continue;
        };
        let op = ProvOp {
            op: kind.to_string(),
            args: vec![cmd.clone(), format!("argv[{}]", i)],
            site: site.clone(),
        };
        l.extend_owner(&node, &op);
        node.push_op(op);
        l.put_content(a, node);
    }
}

// ── Shell functions ─────────────────────────────────────────────────
//
// A function has a lineage of its own: where it was defined, every
// redefinition, every call, and the `unfunction` that ended it. The
// value taps above answer "where did these bytes come from?"; these
// answer the same question about the code that produced them.

/// Site of a function's definition, as `shfunctab` recorded it —
/// `Src/exec.c:5383-5388` stores the defining file and the line the
/// definition starts on. Falls back to the current site when the
/// function came from somewhere with neither (an `eval`, a `-c` line).
fn def_site(file: Option<&str>, line: i64) -> Site {
    let mut site = Site::now();
    if file.is_some() || line > 0 {
        site.file = file.map(str::to_string).filter(|f| f != "zsh");
        site.line = line.max(0) as usize;
        site.func = None;
    }
    site
}

/// Arm tracking for shell function `name`, seeding its origin from where
/// the function is defined right now. Returns false when the engine is
/// disabled by config/env.
pub fn track_func(name: &str, body: Option<&str>, file: Option<&str>, line: i64) -> bool {
    if !enabled() {
        return false;
    }
    let mut l = lock();
    l.tracked_funcs.insert(name.to_string());
    // Same rule as `track_name`, which this used to break: a function that
    // ALREADY EXISTS gets its definition site as the origin, and one that does
    // not exist yet gets no node at all, so its first definition supplies the
    // origin.
    //
    // Seeding a node unconditionally meant arming a function before defining it
    // stamped an origin of `function NAME (line 0)` — a definition site that had
    // never happened — and then `on_func_define` found a node already there and
    // logged the REAL first definition as a `redefine` against it. Arming early
    // is the normal way to watch a function get defined, so that is the common
    // path, not an edge case.
    //
    // `(None, 0)` is `shfunc_def_site`'s documented "no such function"; a defined
    // one always carries a 1-based line.
    if file.is_some() || line > 0 {
        let site = def_site(file, line);
        // Seed the origin with the body the function has RIGHT NOW.
        // Arming an already-defined function is the common case
        // (`provenance -m greet` on something sourced earlier), and it
        // never passes through `on_func_define`, so this is the only
        // chance to record what the body was before any redefinition.
        let summary = body_summary(body);
        l.func.entry(name.to_string()).or_insert_with(|| {
            ProvNode::origin(
                match &summary {
                    Some(b) => format!("function {} {{ {} }}", name, b),
                    None => format!("function {}", name),
                },
                site,
            )
        });
    }
    drop(l);
    PROV_ACTIVE.store(true, Ordering::Relaxed);
    true
}

/// Drop tracking and lineage for function `name`.
pub fn untrack_func(name: &str) -> bool {
    let mut l = lock();
    let had = l.tracked_funcs.remove(name) | l.func.remove(name).is_some();
    let empty = l.tracked.is_empty()
        && l.name.is_empty()
        && l.tracked_funcs.is_empty()
        && l.func.is_empty();
    drop(l);
    if empty && !track_all() {
        PROV_ACTIVE.store(false, Ordering::Relaxed);
    }
    had
}

/// Lineage of a tracked function, or `None` when it is not tracked.
pub fn lookup_func(name: &str) -> Option<ProvNode> {
    lock().func.get(name).cloned()
}

/// Tracked function names, sorted.
pub fn tracked_func_names() -> Vec<String> {
    let l = lock();
    let mut v: Vec<String> = l.tracked_funcs.iter().cloned().collect();
    v.sort();
    v
}

/// Names [`track_all`] declined to arm because [`MAX_AUTO_NAMES`] was
/// already reached.
pub fn auto_dropped() -> usize {
    lock().auto_dropped
}

/// Arm `name` in the function namespace when track-everything mode is
/// on and the ledger has room. Caller holds the lock.
fn auto_arm_func(l: &mut Ledger, name: &str) -> bool {
    if l.tracked_funcs.contains(name) {
        return true;
    }
    if !track_all() || !auto_armable(name) {
        return false;
    }
    if l.tracked.len() + l.tracked_funcs.len() >= MAX_AUTO_NAMES {
        l.auto_dropped += 1;
        return false;
    }
    l.tracked_funcs.insert(name.to_string());
    true
}

/// Collapse a function body to one line that fits the chain's argument
/// column. Bodies are multi-line and indented; the chain wants "what is
/// this, and what did it become", not a listing, so runs of whitespace
/// (newlines included) fold to a single space and the tail is cut with
/// `…`. Returns None when there is no body text to show, so a caller
/// with nothing to record renders exactly as it did before.
fn body_summary(body: Option<&str>) -> Option<String> {
    let collapsed = body?.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    // `render` lays the argument column out as `{:<40}`; leave room for
    // the function name and the ellipsis.
    const MAX_BODY_WIDTH: usize = 32;
    if collapsed.chars().count() > MAX_BODY_WIDTH {
        let keep: String = collapsed.chars().take(MAX_BODY_WIDTH).collect();
        Some(format!("{}…", keep))
    } else {
        Some(collapsed)
    }
}

/// A function was defined, at `file`:`line`. The first definition is the
/// origin; a later one is a `redefine` op, so a chain shows every body
/// the name ever had and where each came from.
///
/// `body` is the function's source text. It is recorded on both the
/// origin and every `redefine`, because a redefinition op that names
/// only the function cannot answer what the body was changed TO — which
/// is the only reason to look at a redefine op at all.
pub fn on_func_define(name: &str, body: Option<&str>, file: Option<&str>, line: i64) {
    let site = def_site(file, line);
    let summary = body_summary(body);
    let mut l = lock();
    if !auto_arm_func(&mut l, name) {
        return;
    }
    match l.func.get_mut(name) {
        Some(node) => node.push_op(ProvOp {
            op: "redefine".to_string(),
            args: vec![match &summary {
                Some(b) => format!("{} {{ {} }}", name, b),
                None => name.to_string(),
            }],
            site,
        }),
        None => {
            l.func.insert(
                name.to_string(),
                ProvNode::origin(
                    match &summary {
                        Some(b) => format!("function {} {{ {} }}", name, b),
                        None => format!("function {}", name),
                    },
                    site,
                ),
            );
        }
    }
}

/// Render a call's positionals the way the chain displays them:
/// `greet(alpha beta)`. An argument that is empty or carries whitespace
/// is single-quoted so `f 'a b'` and `f a b` stay distinguishable; the
/// whole list is capped to the width of `render`'s argument column so a
/// call with a hundred arguments cannot smear the table.
fn call_signature(name: &str, args: &[String]) -> String {
    // `render` lays the argument column out as `{:<40}`; leave room for
    // the name, the parentheses and the ellipsis.
    const MAX_ARGS_WIDTH: usize = 32;
    let mut rendered = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            rendered.push(' ');
        }
        if a.is_empty() || a.chars().any(char::is_whitespace) {
            rendered.push('\'');
            rendered.push_str(a);
            rendered.push('\'');
        } else {
            rendered.push_str(a);
        }
        if rendered.chars().count() > MAX_ARGS_WIDTH {
            let keep: String = rendered.chars().take(MAX_ARGS_WIDTH).collect();
            rendered = format!("{}…", keep);
            break;
        }
    }
    format!("{}({})", name, rendered)
}

/// A function is about to run. The op records the *call* site, which is
/// where the caller stands, not where the function was defined, and the
/// ARGUMENTS the call was made with — without them two calls to the same
/// function are indistinguishable on the chain, which defeats the point
/// of asking where a value came from.
///
/// `args` is the positional list only: `doshfunc`'s `doshargs[0]` is the
/// function name (`Src/exec.c:5986` sets `argzero` from it) and the
/// positionals are `doshargs[1..]` (c:5978-5998).
pub fn on_func_call(
    name: &str,
    args: &[String],
    body: Option<&str>,
    file: Option<&str>,
    line: i64,
) {
    let site = Site::now();
    // Arming a function that already exists (`provenance -m greet` on a
    // function defined earlier) never saw `on_func_define`, so the chain
    // is created HERE. Seed its origin with the body the function
    // currently has; without this the common arm-after-definition case
    // shows an origin with no body at all.
    let summary = body_summary(body);
    let mut l = lock();
    if !auto_arm_func(&mut l, name) {
        return;
    }
    let node = l.func.entry(name.to_string()).or_insert_with(|| {
        ProvNode::origin(
            match &summary {
                Some(b) => format!("function {} {{ {} }}", name, b),
                None => format!("function {}", name),
            },
            def_site(file, line),
        )
    });
    node.push_op(ProvOp {
        op: "call".to_string(),
        args: vec![call_signature(name, args)],
        site,
    });
}

/// A function was removed (`unfunction`, `unset -f`). Kept on the chain
/// as its final op, the same way an unset parameter is.
pub fn on_func_unset(name: &str) {
    let site = Site::now();
    let mut l = lock();
    if let Some(node) = l.func.get_mut(name) {
        node.push_op(ProvOp {
            op: "unfunction".to_string(),
            args: vec![name.to_string()],
            site,
        });
    }
}

// ── Rendering ───────────────────────────────────────────────────────

/// Human-readable lineage report, as printed by `provenance NAME`.
pub fn render(label: &str, node: &ProvNode) -> String {
    let mut out = format!("{}\n", label);
    out.push_str(&format!(
        "  origin: {} ({}, {})\n",
        node.origin,
        node.origin_site.location(),
        node.origin_site.clock(true)
    ));
    if node.ops.is_empty() {
        out.push_str("  ops: (none)\n");
        return out;
    }
    // An op that ran on a different day than the origin spells its date
    // out; the rest carry the time alone, which is what a chain built
    // inside one session needs.
    let origin_day = node.origin_site.day();
    out.push_str("  ops:\n");
    for (i, op) in node.ops.iter().enumerate() {
        out.push_str(&format!(
            "    {:>2}. {:<10} {:<40} {:<24} {}\n",
            i + 1,
            op.op,
            op.args.join(" "),
            op.site.location(),
            op.site.clock(op.site.day() != origin_day)
        ));
    }
    if node.dropped_ops > 0 {
        out.push_str(&format!(
            "    … {} more ops (chain capped at {})\n",
            node.dropped_ops, MAX_OPS
        ));
    }
    out
}

/// A JSON string, or `null` when there is no file to name.
fn json_str_or_null(s: Option<&str>) -> String {
    match s {
        Some(v) => format!("{:?}", v),
        None => "null".to_string(),
    }
}

/// JSON lineage report, as printed by `provenance -j NAME`. Hand-rolled
/// so the engine pulls in no serializer of its own; the shapes here are
/// flat strings and integers.
pub fn render_json(label: &str, node: &ProvNode) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"name\":{:?},\"origin\":{:?},\"origin_line\":{},\"origin_file\":{},\"origin_function\":{},\"origin_time\":{:?},\"ops\":[",
        label,
        node.origin,
        node.origin_site.line,
        json_str_or_null(node.origin_site.file.as_deref()),
        json_str_or_null(node.origin_site.func.as_deref()),
        node.origin_site.rfc3339(),
    ));
    for (i, op) in node.ops.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"op\":{:?},\"args\":[", op.op));
        for (j, a) in op.args.iter().enumerate() {
            if j > 0 {
                out.push(',');
            }
            out.push_str(&format!("{:?}", a));
        }
        out.push_str(&format!(
            "],\"line\":{},\"file\":{},\"function\":{},\"time\":{:?}}}",
            op.site.line,
            json_str_or_null(op.site.file.as_deref()),
            json_str_or_null(op.site.func.as_deref()),
            op.site.rfc3339(),
        ));
    }
    out.push_str(&format!("],\"dropped_ops\":{}}}", node.dropped_ops));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every test drives the same process-global ledger, so they take
    /// the shared state lock and start from a clean ledger.
    fn setup() -> std::sync::MutexGuard<'static, ()> {
        let g = crate::test_util::global_state_lock();
        clear();
        PROV_ACTIVE.store(true, Ordering::Relaxed);
        g
    }

    #[test]
    fn tracked_name_seeds_an_origin_from_the_current_value() {
        let _g = setup();
        note_line(7);
        assert!(track_name("FOO", Some("bar")));
        let node = lookup_name("FOO").expect("tracked name has a node");
        assert_eq!(node.origin_site.line, 7);
        assert!(node.origin.contains("param FOO"), "origin = {}", node.origin);
        assert!(node.ops.is_empty(), "no ops at the origin");
        assert_eq!(tracked_names(), vec!["FOO".to_string()]);
    }

    #[test]
    fn assignment_inherits_the_command_substitutions_lineage() {
        let _g = setup();
        note_line(3);
        track_name("OUT", None);
        on_cmd_subst("date +%s", "1750000000");
        on_param_write("OUT", "assign", "1750000000");
        let node = lookup_name("OUT").expect("assignment created a node");
        assert!(
            node.origin.starts_with("cmdsubst"),
            "origin must come from the substitution, got {}",
            node.origin
        );
        assert_eq!(node.ops.len(), 1);
        assert_eq!(node.ops[0].op, "assign");
    }

    #[test]
    fn expansion_then_exec_extends_the_owning_parameters_chain() {
        let _g = setup();
        note_line(1);
        track_name("F", Some("report.txt"));
        on_param_write("F", "assign", "report.txt");
        // `$F` read by an ExpandParam op, then handed to `wc -l $F`.
        let v = Value::str("report.txt");
        on_param_read("F", &v);
        on_exec("exec", &["wc".into(), "-l".into(), "report.txt".into()]);
        let node = lookup_name("F").expect("F is tracked");
        let ops: Vec<&str> = node.ops.iter().map(|o| o.op.as_str()).collect();
        assert!(
            ops.contains(&"exec"),
            "consumption must land on the owner's chain, got {:?}",
            ops
        );
        let exec_op = node.ops.iter().find(|o| o.op == "exec").unwrap();
        assert_eq!(exec_op.args[0], "wc");
        assert_eq!(exec_op.args[1], "argv[2]");
    }

    #[test]
    fn concat_carries_the_lineage_of_whichever_operand_has_one() {
        let _g = setup();
        note_line(4);
        track_name("F", Some("alpha"));
        on_param_write("F", "assign", "alpha");
        let read = Value::str("alpha");
        on_param_read("F", &read);
        let joined = Value::str("alpha.bak");
        on_concat(&read, &Value::str(".bak"), &joined);
        let node = lookup_value(&joined).expect("result inherited the chain");
        assert_eq!(node.owner.as_deref(), Some("F"));
        assert_eq!(node.ops.last().map(|o| o.op.as_str()), Some("concat"));
        // The owner's own chain records the concat too, so
        // `provenance F` shows what was built out of it.
        let owner = lookup_name("F").unwrap();
        assert!(owner.ops.iter().any(|o| o.op == "concat"), "{owner:?}");
    }

    #[test]
    fn an_empty_segment_concat_propagates_without_recording_an_op() {
        let _g = setup();
        track_name("F", Some("alpha"));
        on_param_write("F", "assign", "alpha");
        let read = Value::str("alpha");
        on_param_read("F", &read);
        let before = lookup_name("F").unwrap().ops.len();
        // The word assembler emits `"" + $F` for a leading segment.
        let same = Value::str("alpha");
        on_concat(&Value::str(""), &read, &same);
        assert_eq!(
            lookup_name("F").unwrap().ops.len(),
            before,
            "a concat that adds no bytes must not appear in the chain"
        );
        assert!(
            lookup_value(&same).is_some(),
            "the lineage still has to reach the produced value"
        );
    }

    #[test]
    fn concat_of_two_untracked_values_records_nothing() {
        let _g = setup();
        let out = Value::str("ab");
        on_concat(&Value::str("a"), &Value::str("b"), &out);
        assert!(lookup_value(&out).is_none());
    }

    #[test]
    fn untracked_names_never_gain_a_lineage() {
        let _g = setup();
        on_cmd_subst("ls", "a\nb");
        on_param_write("UNTRACKED", "assign", "a\nb");
        assert!(
            lookup_name("UNTRACKED").is_none(),
            "writes to untracked names must not create rows"
        );
    }

    #[test]
    fn value_lineage_survives_arc_clones_and_dies_with_the_value() {
        let _g = setup();
        track_name("V", Some("x"));
        let v = Value::str("payload");
        on_param_read("V", &v);
        let clone = v.clone();
        assert!(
            lookup_value(&clone).is_some(),
            "an Arc clone is the same value"
        );
        let ptr = value_ptr(&v).unwrap();
        drop(v);
        drop(clone);
        // The row is stale now: the Weak cannot upgrade, so a later
        // allocation reusing the address must not inherit the lineage.
        let mut l = lock();
        assert!(l.ptr_node(ptr).is_none(), "dropped value must reap its row");
        assert!(!l.ptr.contains_key(&ptr), "reaped row must be removed");
    }

    #[test]
    fn content_entries_are_bounded_by_the_fifo_cap() {
        let _g = setup();
        for i in 0..(CONTENT_CAP + 100) {
            on_cmd_subst("gen", &format!("value-{}", i));
        }
        let l = lock();
        assert_eq!(l.content.len(), CONTENT_CAP);
        assert_eq!(l.content_order.len(), CONTENT_CAP);
        drop(l);
        assert!(
            lookup_content("value-0").is_none(),
            "oldest speculative entry must have been evicted"
        );
        assert!(
            lookup_content(&format!("value-{}", CONTENT_CAP + 99)).is_some(),
            "newest speculative entry must survive"
        );
    }

    #[test]
    fn high_fanout_globs_are_not_recorded() {
        let _g = setup();
        let many: Vec<String> = (0..64).map(|i| format!("f{}", i)).collect();
        on_glob("*", &many);
        assert!(
            lookup_content("f0").is_none(),
            "a 64-match glob must not evict the ledger"
        );
        on_glob("*.rs", &["lib.rs".to_string()]);
        assert!(lookup_content("lib.rs").is_some(), "small globs record");
    }

    #[test]
    fn untrack_and_clear_disarm_the_engine() {
        let _g = setup();
        track_name("A", Some("1"));
        track_name("B", Some("2"));
        assert!(untrack_name("A"));
        assert!(active(), "still armed while B is tracked");
        assert!(untrack_name("B"));
        assert!(!active(), "last untrack disarms the hot gate");
        assert!(!untrack_name("B"), "second untrack is a no-op");
        track_name("C", None);
        clear();
        assert!(!active());
        assert!(tracked_names().is_empty());
    }

    #[test]
    fn reassignment_extends_the_chain_instead_of_replacing_it() {
        let _g = setup();
        note_line(1);
        track_name("X", None);
        note_line(2);
        on_param_write("X", "assign", "23");
        note_line(3);
        on_param_write("X", "assign", "1");
        note_line(4);
        on_param_write("X", "assign", "55");
        let node = lookup_name("X").expect("X has a chain");
        assert_eq!(node.origin_site.line, 2, "origin is the first write");
        let seen: Vec<(&str, &str, usize)> = node
            .ops
            .iter()
            .map(|o| (o.op.as_str(), o.args[1].as_str(), o.site.line))
            .collect();
        assert_eq!(
            seen,
            vec![
                ("assign", "\"23\"", 2),
                ("assign", "\"1\"", 3),
                ("assign", "\"55\"", 4),
            ],
            "every assignment stays on the chain: {:?}",
            node.ops
        );
    }

    #[test]
    fn a_later_substitution_is_spliced_in_not_swapped_for_the_chain() {
        let _g = setup();
        note_line(1);
        track_name("X", Some("seed"));
        note_line(2);
        on_param_write("X", "assign", "23");
        note_line(3);
        on_cmd_subst("date +%s", "1750000000");
        on_param_write("X", "assign", "1750000000");
        let node = lookup_name("X").expect("X has a chain");
        assert!(
            node.origin.contains("param X = "),
            "the armed value stays the origin, got {}",
            node.origin
        );
        let ops: Vec<&str> = node.ops.iter().map(|o| o.op.as_str()).collect();
        assert_eq!(ops, vec!["assign", "origin", "assign"], "{:?}", node.ops);
        assert!(
            node.ops[1].args[0].starts_with("cmdsubst"),
            "the substitution is recorded where it happened: {:?}",
            node.ops[1]
        );
    }

    #[test]
    fn rewriting_a_parameter_with_its_own_value_does_not_duplicate_the_chain() {
        let _g = setup();
        track_name("X", None);
        on_param_write("X", "assign", "same");
        on_param_write("X", "assign", "same");
        let node = lookup_name("X").unwrap();
        assert_eq!(node.ops.len(), 1, "immediate repeat collapses: {:?}", node.ops);
        note_line(9);
        on_param_write("X", "assign", "same");
        let node = lookup_name("X").unwrap();
        assert_eq!(
            node.ops.iter().filter(|o| o.op == "origin").count(),
            0,
            "a parameter never inherits from itself: {:?}",
            node.ops
        );
    }

    #[test]
    fn a_repeat_at_the_same_position_collapses_despite_a_later_clock() {
        let _g = setup();
        note_line(4);
        track_name("Q", None);
        on_param_write("Q", "assign", "v");
        let first = lookup_name("Q").unwrap().ops[0].site.time_ms;
        std::thread::sleep(std::time::Duration::from_millis(5));
        on_param_write("Q", "assign", "v");
        let node = lookup_name("Q").unwrap();
        assert_eq!(node.ops.len(), 1, "the clock must not defeat the collapse");
        assert_eq!(node.ops[0].site.time_ms, first, "the first stamp stands");
    }

    #[test]
    fn a_site_carries_the_file_the_line_belongs_to() {
        let _g = setup();
        let saved = crate::ported::utils::scriptfilename_get();
        crate::ported::utils::set_scriptfilename(Some("/tmp/lineage.zsh".to_string()));
        note_line(11);
        track_name("S", None);
        on_param_write("S", "assign", "v");
        crate::ported::utils::set_scriptfilename(saved);
        let node = lookup_name("S").unwrap();
        assert_eq!(node.origin_site.file.as_deref(), Some("/tmp/lineage.zsh"));
        assert_eq!(node.origin_site.line, 11);
        assert_eq!(node.origin_site.func, None, "no function frame is active");
        assert!(node.origin_site.time_ms > 0, "the origin is stamped");
        assert_eq!(node.origin_site.location(), "/tmp/lineage.zsh:11");
        assert!(
            node.origin_site.clock(true).starts_with("20"),
            "clock = {}",
            node.origin_site.clock(true)
        );
        assert!(
            render("S", &node).contains("/tmp/lineage.zsh:11"),
            "the report names the file"
        );
    }

    #[test]
    fn track_all_arms_a_parameter_on_its_first_write() {
        let _g = setup();
        assert!(set_track_all(true));
        note_line(3);
        on_param_write("NEVER_ARMED", "assign", "v");
        let node = lookup_name("NEVER_ARMED").expect("track_all armed it");
        assert_eq!(node.ops.len(), 1);
        assert_eq!(tracked_names(), vec!["NEVER_ARMED".to_string()]);
        set_track_all(false);
        on_param_write("STILL_UNARMED", "assign", "v");
        assert!(
            lookup_name("STILL_UNARMED").is_none(),
            "turning it off stops arming new names"
        );
    }

    /// A `call` op must carry the arguments the call was made with.
    /// Without them two calls to one function are byte-identical on the
    /// chain, so "where did this value come from" cannot distinguish
    /// `deploy staging` from `deploy prod` — which is the question the
    /// engine exists to answer.
    #[test]
    fn call_ops_record_the_arguments_they_were_called_with() {
        let _g = setup();
        assert!(track_func("deploy", None, Some("/tmp/d.zsh"), 1));
        on_func_call("deploy", &["staging".to_string()], None, Some("/tmp/d.zsh"), 1);
        on_func_call("deploy", &["prod".to_string()], None, Some("/tmp/d.zsh"), 2);
        on_func_call("deploy", &[], None, Some("/tmp/d.zsh"), 3);
        let node = lookup_func("deploy").expect("armed");
        let args: Vec<&str> = node.ops.iter().map(|o| o.args[0].as_str()).collect();
        assert_eq!(
            args,
            vec!["deploy(staging)", "deploy(prod)", "deploy()"],
            "each call must be distinguishable by its arguments"
        );
    }

    /// An empty or whitespace-bearing argument is single-quoted, so
    /// `f 'a b'` (one argument) does not render the same as `f a b`
    /// (two), and a long list is capped rather than smearing the table.
    #[test]
    fn call_signature_quotes_ambiguous_args_and_caps_long_lists() {
        let one = call_signature("f", &["a b".to_string()]);
        let two = call_signature("f", &["a".to_string(), "b".to_string()]);
        assert_eq!(one, "f('a b')");
        assert_eq!(two, "f(a b)");
        assert_ne!(one, two, "quoting is what keeps these apart");
        assert_eq!(call_signature("f", &["".to_string()]), "f('')");

        let many: Vec<String> = (0..40).map(|i| format!("arg{i}")).collect();
        let rendered = call_signature("f", &many);
        assert!(rendered.ends_with("…)"), "long list truncates: {rendered}");
        assert!(
            rendered.chars().count() <= 40,
            "must fit render's 40-wide arg column: {} chars",
            rendered.chars().count()
        );
    }

    #[test]
    fn track_all_skips_the_parameters_the_shell_rewrites_itself() {
        let _g = setup();
        assert!(set_track_all(true));
        for volatile in ["LINENO", "RANDOM", "status", "_", "3"] {
            on_param_write(volatile, "assign", "v");
            assert!(
                lookup_name(volatile).is_none(),
                "{volatile} must not arm itself"
            );
        }
        on_param_write("REPLY", "assign", "v");
        assert!(lookup_name("REPLY").is_some(), "ordinary names still arm");
    }

    #[test]
    fn track_all_stops_arming_at_the_ceiling_and_counts_the_rest() {
        let _g = setup();
        assert!(set_track_all(true));
        for i in 0..MAX_AUTO_NAMES + 8 {
            on_param_write(&format!("P{}", i), "assign", "v");
        }
        assert_eq!(tracked_names().len(), MAX_AUTO_NAMES);
        assert_eq!(auto_dropped(), 8, "the overflow is counted, not stored");
    }

    #[test]
    fn a_function_records_its_definition_calls_and_removal() {
        let _g = setup();
        assert!(set_track_all(true));
        on_func_define("build", None, Some("/tmp/lib.zsh"), 12);
        note_line(40);
        on_func_call("build", &[], None, Some("/tmp/lib.zsh"), 12);
        on_func_define("build", None, Some("/tmp/lib.zsh"), 80);
        on_func_unset("build");
        let node = lookup_func("build").expect("the function has a chain");
        assert_eq!(node.origin, "function build");
        assert_eq!(node.origin_site.file.as_deref(), Some("/tmp/lib.zsh"));
        assert_eq!(node.origin_site.line, 12, "origin is the first definition");
        let ops: Vec<&str> = node.ops.iter().map(|o| o.op.as_str()).collect();
        assert_eq!(ops, vec!["call", "redefine", "unfunction"], "{:?}", node.ops);
        assert_eq!(node.ops[0].site.line, 40, "the call op is the caller's site");
        assert_eq!(node.ops[1].site.line, 80, "the redefine op is the new body's");
        assert_eq!(tracked_func_names(), vec!["build".to_string()]);
        assert!(lookup_name("build").is_none(), "the parameter namespace is separate");
    }

    // Arming a function BEFORE it is defined is the normal way to watch one get
    // defined, and it used to produce a lineage that was wrong twice over: an
    // origin of `function NAME` at line 0 -- a definition site that never
    // happened -- and the real first definition logged as a `redefine` against
    // it. `track_name` has always got this right for parameters (an unset one
    // gets no node, so its first assignment supplies the origin); this is the
    // same rule for functions.
    #[test]
    fn arming_a_function_before_it_exists_leaves_the_origin_to_its_definition() {
        let _g = setup();
        // `(None, 0)` is shfunc_def_site's "no such function".
        assert!(track_func("later", None, None, 0));
        assert!(
            lookup_func("later").is_none(),
            "no definition has happened yet, so there is nothing to attribute"
        );
        assert_eq!(
            tracked_func_names(),
            vec!["later".to_string()],
            "but it IS armed"
        );

        on_func_define("later", None, Some("/tmp/lib.zsh"), 7);
        let node = lookup_func("later").expect("the definition creates the chain");
        assert_eq!(node.origin, "function later");
        assert_eq!(
            node.origin_site.line, 7,
            "the origin is where it was defined"
        );
        assert_eq!(node.origin_site.file.as_deref(), Some("/tmp/lib.zsh"));
        assert!(
            node.ops.is_empty(),
            "the first definition is the origin, not a redefine: {:?}",
            node.ops
        );

        // A LATER definition is still a redefine, against the real origin.
        on_func_define("later", None, Some("/tmp/lib.zsh"), 20);
        let node = lookup_func("later").expect("still tracked");
        assert_eq!(node.origin_site.line, 7, "the origin does not move");
        let ops: Vec<&str> = node.ops.iter().map(|o| o.op.as_str()).collect();
        assert_eq!(ops, vec!["redefine"], "{:?}", node.ops);
        assert_eq!(node.ops[0].site.line, 20);
    }

    // The other half of the same rule: a function that already exists when it is
    // armed keeps its definition site, so arming does not erase where it came
    // from.
    #[test]
    fn arming_an_existing_function_seeds_the_origin_from_its_definition_site() {
        let _g = setup();
        assert!(track_func("known", None, Some("/tmp/lib.zsh"), 3));
        let node = lookup_func("known").expect("armed with a known definition site");
        assert_eq!(node.origin_site.line, 3);
        assert_eq!(node.origin_site.file.as_deref(), Some("/tmp/lib.zsh"));
        assert!(node.ops.is_empty());

        // Re-arming does not move the origin or duplicate the node.
        assert!(track_func("known", None, Some("/tmp/lib.zsh"), 3));
        let node = lookup_func("known").expect("still there");
        assert_eq!(node.origin_site.line, 3);
        assert_eq!(tracked_func_names(), vec!["known".to_string()]);
    }

    #[test]
    fn an_unarmed_function_records_nothing_without_track_all() {
        let _g = setup();
        on_func_define("quiet", None, Some("/tmp/lib.zsh"), 1);
        on_func_call("quiet", &[], None, Some("/tmp/lib.zsh"), 1);
        assert!(lookup_func("quiet").is_none());
        assert!(track_func("quiet", None, Some("/tmp/lib.zsh"), 1));
        on_func_call("quiet", &[], None, Some("/tmp/lib.zsh"), 1);
        let node = lookup_func("quiet").expect("armed by name");
        assert_eq!(node.ops.len(), 1, "only the call after arming: {:?}", node.ops);
        assert!(untrack_func("quiet"));
        assert!(!active(), "the last untrack disarms the hot gate");
    }

    #[test]
    fn unset_is_recorded_as_the_final_op() {
        let _g = setup();
        track_name("Z", Some("v"));
        on_param_unset("Z");
        let node = lookup_name("Z").unwrap();
        assert_eq!(node.ops.last().map(|o| o.op.as_str()), Some("unset"));
    }

    #[test]
    fn render_and_json_carry_the_whole_chain() {
        let _g = setup();
        note_line(12);
        track_name("R", None);
        on_cmd_subst("echo hi", "hi");
        on_param_write("R", "assign", "hi");
        let node = lookup_name("R").unwrap();
        let text = render("R", &node);
        assert!(text.contains("origin: cmdsubst"), "text = {}", text);
        assert!(text.contains("line 12"), "text = {}", text);
        let json = render_json("R", &node);
        assert!(json.starts_with("{\"name\":\"R\""), "json = {}", json);
        assert!(json.contains("\"op\":\"assign\""), "json = {}", json);
        assert!(json.ends_with("\"dropped_ops\":0}"), "json = {}", json);
    }

    /// Eight threads hammering the ledger concurrently: no data race, no
    /// deadlock, and the tracked node stays coherent. Ports stryke's
    /// `concurrent_mark_lookup_threads_observe_consistent_state`.
    #[test]
    fn concurrent_hooks_keep_the_ledger_coherent() {
        let _g = setup();
        note_line(1);
        track_name("SHARED", Some("seed"));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                std::thread::spawn(move || {
                    for n in 0..50 {
                        let out = format!("t{}-{}", i, n);
                        on_cmd_subst("worker", &out);
                        let v = Value::str(out.clone());
                        on_param_read("SHARED", &v);
                        on_exec("exec", &["cmd".into(), out]);
                        assert!(lookup_name("SHARED").is_some());
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("thread");
        }
        let node = lookup_name("SHARED").expect("origin still live");
        assert_eq!(node.origin_site.line, 1);
        assert_eq!(node.owner.as_deref(), Some("SHARED"));
    }
}
