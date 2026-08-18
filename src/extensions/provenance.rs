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

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

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

/// Longest value prefix stored as a content key. Content keying hashes
/// the whole string, but the *summary* strings kept in the ledger are
/// truncated to this so the ledger stays small next to the values it
/// describes.
const SUMMARY_MAX: usize = 64;

/// Flipped to `true` the first time a name is tracked or a value is
/// marked. Every hook checks it via [`active`] — one inlined relaxed
/// load on the universal path where nobody armed the engine.
static PROV_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Last `$LINENO` seen by [`note_line`]. Updated from the
/// `BUILTIN_SET_LINENO` bytecode handler while armed, so lineage
/// records carry a source line without any hook having to take the
/// parameter-table lock (which the param write hooks run inside).
static CURRENT_LINE: AtomicUsize = AtomicUsize::new(0);

/// One operation in a value's lineage chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvOp {
    /// Operation name — `cmdsubst`, `glob`, `expand`, `assign`, `exec`,
    /// `function`, `unset`.
    pub op: String,
    /// Short summaries of the operands, in argument order.
    pub args: Vec<String>,
    /// `$LINENO` at the time the op ran, or 0 when unknown.
    pub line: usize,
}

/// Lineage record for a single value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvNode {
    /// How the value entered the shell — e.g. `cmdsubst "date +%s"`,
    /// `glob "*.rs"`, `param FOO`.
    pub origin: String,
    /// `$LINENO` at the origin site.
    pub origin_line: usize,
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
    fn origin(origin: impl Into<String>, line: usize) -> Self {
        Self {
            origin: origin.into(),
            origin_line: line,
            ops: Vec::new(),
            owner: None,
            dropped_ops: 0,
        }
    }

    /// Append an op, collapsing an immediate repeat (same op, operands
    /// and line — a value read several times while evaluating one
    /// statement) and counting instead of storing past [`MAX_OPS`].
    fn push_op(&mut self, op: ProvOp) {
        if self.ops.last() == Some(&op) {
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

/// Arm tracking for parameter `name`. Seeds an origin from the
/// parameter's current value so lineage starts at "what it holds now".
/// Returns false when the engine is disabled by config/env.
pub fn track_name(name: &str, current_value: Option<&str>) -> bool {
    if !enabled() {
        return false;
    }
    let line = current_line();
    let mut l = lock();
    l.tracked.insert(name.to_string());
    let mut node = ProvNode::origin(
        match current_value {
            Some(v) => format!("param {} = {}", name, summarize_str(v)),
            None => format!("param {} (unset)", name),
        },
        line,
    );
    node.owner = Some(name.to_string());
    l.name.insert(name.to_string(), node);
    drop(l);
    PROV_ACTIVE.store(true, Ordering::Relaxed);
    true
}

/// Drop tracking and lineage for `name`. Idempotent; returns true when
/// something was actually removed.
pub fn untrack_name(name: &str) -> bool {
    let mut l = lock();
    let had = l.tracked.remove(name) | l.name.remove(name).is_some();
    let empty = l.tracked.is_empty() && l.name.is_empty();
    drop(l);
    if empty {
        PROV_ACTIVE.store(false, Ordering::Relaxed);
    }
    had
}

/// Drop every ledger entry and disarm the engine.
pub fn clear() {
    let mut l = lock();
    *l = Ledger::default();
    drop(l);
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
    let line = current_line();
    let origin = if source.is_empty() {
        "cmdsubst".to_string()
    } else {
        format!("cmdsubst {}", summarize_str(source))
    };
    lock().put_content(out, ProvNode::origin(origin, line));
}

/// A process substitution produced the path `out` for sub-chunk source
/// `source`.
pub fn on_process_subst(source: &str, out: &str) {
    let line = current_line();
    let origin = if source.is_empty() {
        "procsubst".to_string()
    } else {
        format!("procsubst {}", summarize_str(source))
    };
    lock().put_content(out, ProvNode::origin(origin, line));
}

/// A glob expanded to `results`. Skips high-fanout expansions: a
/// thousand-file glob would evict the whole content FIFO for matches
/// nobody is tracking.
pub fn on_glob(pattern: &str, results: &[String]) {
    if results.is_empty() || results.len() > 32 {
        return;
    }
    let line = current_line();
    let mut l = lock();
    for r in results {
        l.put_content(
            r,
            ProvNode::origin(format!("glob {}", summarize_str(pattern)), line),
        );
    }
}

/// A heredoc / herestring body became the next command's stdin.
pub fn on_heredoc(kind: &str, body: &str) {
    let line = current_line();
    lock().put_content(body, ProvNode::origin(kind.to_string(), line));
}

/// Parameter `name` was read by an `ExpandParam` bytecode op, producing
/// `value`. When the parameter is tracked, the value carries the
/// parameter's chain forward — by `Arc` identity for the rest of the
/// chunk, and by content for the host boundaries that only see `String`.
pub fn on_param_read(name: &str, value: &Value) {
    let line = current_line();
    let mut l = lock();
    let Some(mut node) = l.name.get(name).cloned() else {
        return;
    };
    let op = ProvOp {
        op: "expand".to_string(),
        args: vec![format!("${}", name), summarize_value(value)],
        line,
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
    let line = current_line();
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
    let op = ProvOp {
        op: "concat".to_string(),
        args: vec![summarize_value(lhs), summarize_value(rhs)],
        line,
    };
    l.extend_owner(&node, &op);
    node.push_op(op);
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
/// origin instead of a bare "assigned here".
pub fn on_param_write(name: &str, kind: &str, value: &str) {
    let line = current_line();
    let mut l = lock();
    if !l.tracked.contains(name) {
        return;
    }
    let inherited = l.content_node(value);
    let mut node = match inherited {
        Some(mut n) => {
            n.owner = Some(name.to_string());
            n
        }
        None => {
            let mut n = ProvNode::origin(format!("{} {}", kind, summarize_str(value)), line);
            n.owner = Some(name.to_string());
            n
        }
    };
    node.push_op(ProvOp {
        op: kind.to_string(),
        args: vec![name.to_string(), summarize_str(value)],
        line,
    });
    l.name.insert(name.to_string(), node.clone());
    l.put_content(value, node);
}

/// Parameter `name` was unset. Keeps the chain (that is the interesting
/// part) and records the unset as its final op.
pub fn on_param_unset(name: &str) {
    let line = current_line();
    let mut l = lock();
    if let Some(node) = l.name.get_mut(name) {
        node.push_op(ProvOp {
            op: "unset".to_string(),
            args: vec![name.to_string()],
            line,
        });
    }
}

/// A word whose source text is `source` expanded to `result`. This is
/// the derivation link: when the word names a tracked parameter, the
/// produced bytes — which need not equal the parameter's own value, as
/// in `G=${F}x` — inherit that parameter's chain, so the assignment that
/// consumes them can trace back to the original origin.
pub fn on_word_expand(source: &str, result: &str) {
    if result.is_empty() || !source.contains('$') {
        return;
    }
    let line = current_line();
    let mut l = lock();
    // Tracked names are user-armed and few, so a scan per expansion is
    // cheaper than parsing the word's substitution syntax here.
    let named: Vec<String> = l
        .tracked
        .iter()
        .filter(|n| {
            source.contains(&format!("${}", n)) || source.contains(&format!("${{{}", n))
        })
        .cloned()
        .collect();
    for name in named {
        let Some(mut node) = l.name.get(&name).cloned() else {
            continue;
        };
        let op = ProvOp {
            op: "expand".to_string(),
            args: vec![format!("${}", name), summarize_str(result)],
            line,
        };
        if let Some(target) = l.name.get_mut(&name) {
            target.push_op(op.clone());
        }
        node.push_op(op);
        node.owner = Some(name);
        l.put_content(result, node);
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
    let line = current_line();
    let cmd = args[0].clone();
    let mut l = lock();
    for (i, a) in args.iter().enumerate().skip(1) {
        let Some(mut node) = l.content_node(a) else {
            continue;
        };
        let op = ProvOp {
            op: kind.to_string(),
            args: vec![cmd.clone(), format!("argv[{}]", i)],
            line,
        };
        l.extend_owner(&node, &op);
        node.push_op(op);
        l.put_content(a, node);
    }
}

// ── Rendering ───────────────────────────────────────────────────────

/// Human-readable lineage report, as printed by `provenance NAME`.
pub fn render(label: &str, node: &ProvNode) -> String {
    let mut out = format!("{}\n", label);
    out.push_str(&format!(
        "  origin: {} (line {})\n",
        node.origin, node.origin_line
    ));
    if node.ops.is_empty() {
        out.push_str("  ops: (none)\n");
        return out;
    }
    out.push_str("  ops:\n");
    for (i, op) in node.ops.iter().enumerate() {
        out.push_str(&format!(
            "    {:>2}. {:<10} {:<40} line {}\n",
            i + 1,
            op.op,
            op.args.join(" "),
            op.line
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

/// JSON lineage report, as printed by `provenance -j NAME`. Hand-rolled
/// so the engine pulls in no serializer of its own; the shapes here are
/// flat strings and integers.
pub fn render_json(label: &str, node: &ProvNode) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"name\":{:?},\"origin\":{:?},\"origin_line\":{},\"ops\":[",
        label, node.origin, node.origin_line
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
        out.push_str(&format!("],\"line\":{}}}", op.line));
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
        assert_eq!(node.origin_line, 7);
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
        assert_eq!(node.origin_line, 1);
        assert_eq!(node.owner.as_deref(), Some("SHARED"));
    }
}
