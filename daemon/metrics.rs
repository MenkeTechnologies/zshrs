//! `daemon.metrics` op + `GET /metrics` Prometheus exposition.
//!
//! Per docs/DAEMON_AS_SERVICE.md: "metrics map (counters, histograms)".
//! In-process counters; no scrape-side persistence (Prometheus does
//! that). Exposes:
//!
//!   - `daemon_uptime_seconds`            (gauge)
//!   - `daemon_op_total{op="..."}`        (counter, per op name)
//!   - `daemon_op_failures_total{op="..."}` (counter, per op)
//!   - `daemon_http_requests_total{path="...",code="..."}` (counter)
//!   - `daemon_active_sessions`           (gauge)
//!   - `daemon_active_subscriptions`      (gauge)
//!   - `daemon_active_locks`              (gauge)
//!   - `daemon_jobs_total{state="..."}`   (gauge)
//!   - `daemon_canonical_rows`            (gauge)
//!   - `daemon_recorder_events_total`     (counter — sum of all
//!     events_ingested across all
//!     recorder_ingest calls)
//!
//! No external metrics crate (`metrics-rs`, `prometheus-client`) —
//! the surface is small, hand-rolled keeps the dep graph durable.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::ops::OpResult;
use super::state::DaemonState;

/// Per-op counters. `op_total[name]` and `op_failures_total[name]`.
/// Bumped from `ops::dispatch` after every call.
pub struct Metrics {
    pub op_total: Mutex<HashMap<String, u64>>,
    pub op_failures: Mutex<HashMap<String, u64>>,
    pub http_total: Mutex<HashMap<(String, u16), u64>>,
    pub recorder_events_total: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Self::new()
    }
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            op_total: Mutex::new(HashMap::new()),
            op_failures: Mutex::new(HashMap::new()),
            http_total: Mutex::new(HashMap::new()),
            recorder_events_total: AtomicU64::new(0),
        }
    }
    pub fn record_op(&self, op: &str, ok: bool) {
        *self.op_total.lock().entry(op.to_string()).or_insert(0) += 1;
        if !ok {
            *self.op_failures.lock().entry(op.to_string()).or_insert(0) += 1;
        }
    }
    pub fn record_http(&self, path: &str, code: u16) {
        *self
            .http_total
            .lock()
            .entry((path.to_string(), code))
            .or_insert(0) += 1;
    }
    pub fn record_recorder_events(&self, n: u64) {
        self.recorder_events_total
            .fetch_add(n, Ordering::Relaxed);
    }
}

/// Render the metrics surface as Prometheus 0.0.4 text format.
/// Spec: <https://prometheus.io/docs/instrumenting/exposition_formats/>
pub fn prometheus_text(state: &DaemonState) -> String {
    let mut out = String::new();
    let m = &state.metrics;
    let uptime = state.started_at.elapsed().as_secs();
    out.push_str("# HELP daemon_uptime_seconds Seconds since daemon start.\n");
    out.push_str("# TYPE daemon_uptime_seconds gauge\n");
    out.push_str(&format!("daemon_uptime_seconds {uptime}\n"));

    let canon_rows = state.canonical.total_rows();
    out.push_str("# HELP daemon_canonical_rows Total rows in canonical state engine.\n");
    out.push_str("# TYPE daemon_canonical_rows gauge\n");
    out.push_str(&format!("daemon_canonical_rows {canon_rows}\n"));

    let sessions = state.session_count();
    out.push_str("# HELP daemon_active_sessions Currently registered IPC sessions.\n");
    out.push_str("# TYPE daemon_active_sessions gauge\n");
    out.push_str(&format!("daemon_active_sessions {sessions}\n"));

    let subs = state.subscription_count();
    out.push_str("# HELP daemon_active_subscriptions Total pubsub subscriptions across all sessions.\n");
    out.push_str("# TYPE daemon_active_subscriptions gauge\n");
    out.push_str(&format!("daemon_active_subscriptions {subs}\n"));

    let locks = state.locks.lock().len();
    out.push_str("# HELP daemon_active_locks Currently-held named locks.\n");
    out.push_str("# TYPE daemon_active_locks gauge\n");
    out.push_str(&format!("daemon_active_locks {locks}\n"));

    let recorder_events = m.recorder_events_total.load(Ordering::Relaxed);
    out.push_str("# HELP daemon_recorder_events_total Sum of events_ingested across all recorder_ingest calls.\n");
    out.push_str("# TYPE daemon_recorder_events_total counter\n");
    out.push_str(&format!("daemon_recorder_events_total {recorder_events}\n"));

    out.push_str("# HELP daemon_op_total Number of times each op was dispatched.\n");
    out.push_str("# TYPE daemon_op_total counter\n");
    let op_total = m.op_total.lock();
    let mut ops: Vec<(&String, &u64)> = op_total.iter().collect();
    ops.sort_by_key(|(k, _)| k.as_str());
    for (op, count) in ops {
        out.push_str(&format!(
            "daemon_op_total{{op=\"{}\"}} {count}\n",
            escape_label(op)
        ));
    }
    drop(op_total);

    out.push_str("# HELP daemon_op_failures_total Per-op failure count (Err returns).\n");
    out.push_str("# TYPE daemon_op_failures_total counter\n");
    let op_fail = m.op_failures.lock();
    let mut fails: Vec<(&String, &u64)> = op_fail.iter().collect();
    fails.sort_by_key(|(k, _)| k.as_str());
    for (op, count) in fails {
        out.push_str(&format!(
            "daemon_op_failures_total{{op=\"{}\"}} {count}\n",
            escape_label(op)
        ));
    }
    drop(op_fail);

    out.push_str("# HELP daemon_http_requests_total HTTP request counts by path + code.\n");
    out.push_str("# TYPE daemon_http_requests_total counter\n");
    let http = m.http_total.lock();
    let mut http_rows: Vec<(&(String, u16), &u64)> = http.iter().collect();
    http_rows.sort_by(|a, b| (a.0).0.cmp(&(b.0).0).then((a.0).1.cmp(&(b.0).1)));
    for ((path, code), count) in http_rows {
        out.push_str(&format!(
            "daemon_http_requests_total{{path=\"{}\",code=\"{}\"}} {count}\n",
            escape_label(path),
            code
        ));
    }
    out
}

/// Same data, JSON-shaped, for `daemon.metrics` op consumers that
/// don't want to parse Prometheus text format.
pub async fn op_metrics(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    let m = &state.metrics;
    let op_total: HashMap<String, u64> = m.op_total.lock().clone();
    let op_failures: HashMap<String, u64> = m.op_failures.lock().clone();
    let http_total: Vec<Value> = m
        .http_total
        .lock()
        .iter()
        .map(|((path, code), count)| json!({"path": path, "code": code, "count": count}))
        .collect();
    Ok(json!({
        "uptime_seconds": state.started_at.elapsed().as_secs(),
        "canonical_rows": state.canonical.total_rows(),
        "active_sessions": state.session_count(),
        "active_subscriptions": state.subscription_count(),
        "active_locks": state.locks.lock().len(),
        "recorder_events_total": m.recorder_events_total.load(Ordering::Relaxed),
        "op_total": op_total,
        "op_failures_total": op_failures,
        "http_total": http_total,
    }))
}

/// Escape Prometheus label values: \, ", and newline. Per spec.
fn escape_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            other => out.push(other),
        }
    }
    out
}
