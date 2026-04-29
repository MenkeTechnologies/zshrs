// zask — daemon-queued UI primitives.
//
// Per docs/DAEMON.md "`zask` — daemon-queued UI primitives (pull-mode, never
// interferes with prompt)":
//
//   - Daemon never auto-renders. Daemon pushes `ask:pending` (status-line
//     counter increment + optional OSC-9 + log entry), nothing else. The active
//     prompt stays untouched.
//   - User-initiated activation only. Ctrl-X q keybinding (or `zask take`)
//     pulls the next queued request, renders the picker / input / dialog / menu.
//   - Progress is the lone exception: `ask:progress` may update status-line in
//     place because status-line is reserved passive space.
//   - Inbox model: zask pending / take / dismiss / inbox-clear.
//
// V1 scope:
//   - Queue + push/pop/dismiss + ask:pending event are fully implemented.
//   - Actual TUI rendering of picker/input/dialog/menu is delegated to a
//     future iteration that integrates with ZLE; for now `zask take` returns
//     the request payload to the caller (a `zask`-driven TUI helper renders it
//     client-side).

use std::collections::VecDeque;
use std::sync::Arc;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::ipc::{ErrPayload, Frame};
use super::ops::OpResult;
use super::state::DaemonState;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AskRequest {
    pub request_id: String,
    pub from_shell: u64,
    pub target_shell: u64,
    pub kind: AskKind,
    pub payload: Value,
    pub urgency: String,
    pub created_at_ns: i64,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AskKind {
    Picker,
    Input,
    Dialog,
    Menu,
    Progress,
}

impl AskKind {
    fn from_str(s: &str) -> std::result::Result<Self, ErrPayload> {
        match s {
            "picker" => Ok(Self::Picker),
            "input" => Ok(Self::Input),
            "dialog" => Ok(Self::Dialog),
            "menu" => Ok(Self::Menu),
            "progress" => Ok(Self::Progress),
            other => Err(ErrPayload::new(
                "bad_kind",
                format!("unknown ask kind `{}` (try picker|input|dialog|menu|progress)", other),
            )),
        }
    }
    fn as_str(&self) -> &'static str {
        match self {
            Self::Picker => "picker",
            Self::Input => "input",
            Self::Dialog => "dialog",
            Self::Menu => "menu",
            Self::Progress => "progress",
        }
    }
}

/// Per-target inbox of pending UI requests.
#[derive(Default)]
pub struct AskInbox {
    queues: Mutex<std::collections::HashMap<u64, VecDeque<AskRequest>>>,
    next_request_id: Mutex<u64>,
}

impl AskInbox {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn next_id(&self) -> String {
        let mut n = self.next_request_id.lock();
        *n += 1;
        format!("ask-{}", *n)
    }

    pub fn push(&self, req: AskRequest) {
        let mut g = self.queues.lock();
        g.entry(req.target_shell).or_default().push_back(req);
    }

    pub fn pending_count(&self, shell_id: u64) -> usize {
        let g = self.queues.lock();
        g.get(&shell_id).map(|q| q.len()).unwrap_or(0)
    }

    pub fn pending(&self, shell_id: u64) -> Vec<AskRequest> {
        let g = self.queues.lock();
        g.get(&shell_id).map(|q| q.iter().cloned().collect()).unwrap_or_default()
    }

    /// Pop the most-urgent request from the queue (urgency: critical > normal > low).
    /// Returns None if the queue is empty.
    pub fn take(&self, shell_id: u64) -> Option<AskRequest> {
        let mut g = self.queues.lock();
        let q = g.get_mut(&shell_id)?;
        if q.is_empty() {
            return None;
        }
        // Find the index of the most-urgent oldest request.
        let critical_idx = q.iter().position(|r| r.urgency == "critical");
        let idx = critical_idx.unwrap_or(0);
        q.remove(idx)
    }

    pub fn take_specific(&self, shell_id: u64, req_id: &str) -> Option<AskRequest> {
        let mut g = self.queues.lock();
        let q = g.get_mut(&shell_id)?;
        let idx = q.iter().position(|r| r.request_id == req_id)?;
        q.remove(idx)
    }

    pub fn dismiss(&self, shell_id: u64, req_id: &str) -> bool {
        let mut g = self.queues.lock();
        let Some(q) = g.get_mut(&shell_id) else { return false };
        if let Some(idx) = q.iter().position(|r| r.request_id == req_id) {
            q.remove(idx);
            true
        } else {
            false
        }
    }

    pub fn clear(&self, shell_id: u64) -> usize {
        let mut g = self.queues.lock();
        if let Some(q) = g.get_mut(&shell_id) {
            let n = q.len();
            q.clear();
            n
        } else {
            0
        }
    }

    /// Drop every request whose target shell is the given client (called on disconnect).
    pub fn drop_for_shell(&self, shell_id: u64) {
        let mut g = self.queues.lock();
        g.remove(&shell_id);
    }
}

// ---- IPC op handlers ----

pub async fn op_ask_ask(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let kind_s = args
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `kind`"))?;
    let kind = AskKind::from_str(kind_s)?;

    let target = args.get("target").cloned().unwrap_or(Value::Null);
    let target_shell = match target.get("shell_id").and_then(Value::as_u64) {
        Some(id) => id,
        None => match target.get("self").and_then(Value::as_bool) {
            Some(true) => client_id,
            _ => {
                return Err(ErrPayload::new(
                    "bad_args",
                    "target must specify {shell_id} or {self: true}",
                ));
            }
        },
    };

    if state.snapshot_sessions().iter().all(|s| s.client_id != target_shell) {
        return Err(ErrPayload::new(
            "no_shell",
            format!("target shell_id {} not connected", target_shell),
        ));
    }

    let urgency = args
        .get("urgency")
        .and_then(Value::as_str)
        .unwrap_or("normal")
        .to_string();
    let payload = args.get("payload").cloned().unwrap_or(Value::Null);
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(60 * 60 * 1000); // 60 min default per docs/DAEMON.md

    let request_id = state.ask_inbox.next_id();
    let req = AskRequest {
        request_id: request_id.clone(),
        from_shell: client_id,
        target_shell,
        kind: kind.clone(),
        payload: payload.clone(),
        urgency: urgency.clone(),
        created_at_ns: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        timeout_ms,
    };
    state.ask_inbox.push(req.clone());

    // Push ask:pending event to the target shell.
    let pending_count = state.ask_inbox.pending_count(target_shell);
    let evt = json!({
        "request_id": request_id,
        "kind": kind.as_str(),
        "from_shell": client_id,
        "target_shell": target_shell,
        "urgency": urgency,
        "pending_count": pending_count,
    });
    let frame = Frame::event("ask:pending", evt);
    state.send_to(target_shell, frame);

    Ok(json!({
        "request_id": req.request_id,
        "queued_at_target": target_shell,
        "pending_count": pending_count,
    }))
}

pub async fn op_ask_pending(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let target_shell = match args.get("shell_id").and_then(Value::as_u64) {
        Some(id) => id,
        None => client_id,
    };
    let reqs = state.ask_inbox.pending(target_shell);
    Ok(json!({
        "shell_id": target_shell,
        "pending_count": reqs.len(),
        "requests": reqs.iter().map(|r| json!({
            "request_id": r.request_id,
            "kind": r.kind.as_str(),
            "from_shell": r.from_shell,
            "urgency": r.urgency,
            "summary": summary_of(&r.payload),
            "age_ms": age_ms(r.created_at_ns),
        })).collect::<Vec<_>>(),
    }))
}

pub async fn op_ask_take(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let req = if let Some(req_id) = args.get("request_id").and_then(Value::as_str) {
        state.ask_inbox.take_specific(client_id, req_id)
    } else {
        state.ask_inbox.take(client_id)
    };
    match req {
        Some(r) => Ok(json!({
            "request_id": r.request_id,
            "kind": r.kind.as_str(),
            "from_shell": r.from_shell,
            "urgency": r.urgency,
            "payload": r.payload,
            "remaining_pending": state.ask_inbox.pending_count(client_id),
        })),
        None => Ok(json!({
            "request_id": null,
            "remaining_pending": 0,
        })),
    }
}

pub async fn op_ask_dismiss(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    if args.get("all").and_then(Value::as_bool).unwrap_or(false) {
        let n = state.ask_inbox.clear(client_id);
        return Ok(json!({ "dismissed": n }));
    }
    let req_id = args
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `request_id` or `all`"))?;
    let removed = state.ask_inbox.dismiss(client_id, req_id);
    Ok(json!({ "dismissed": if removed { 1 } else { 0 } }))
}

pub async fn op_ask_response(state: &Arc<DaemonState>, args: Value) -> OpResult {
    // The user picked / declined a UI request. Route the response back to the
    // originating shell.
    let req_id = args
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `request_id`"))?;
    let value = args.get("value").cloned().unwrap_or(Value::Null);
    let cancelled = args.get("cancelled").and_then(Value::as_bool).unwrap_or(false);
    let from_shell = args.get("from_shell").and_then(Value::as_u64);

    // For v1 we don't track the originating shell mapping (no full request-table)
    // — the client passes from_shell explicitly so the response routes back.
    if let Some(orig) = from_shell {
        let payload = json!({
            "request_id": req_id,
            "value": value,
            "cancelled": cancelled,
        });
        let frame = Frame::event("ask:response", payload);
        if !state.send_to(orig, frame) {
            return Err(ErrPayload::new(
                "no_originator",
                format!("originating shell {} no longer connected", orig),
            ));
        }
    }
    Ok(json!({ "delivered": from_shell.is_some() }))
}

fn summary_of(payload: &Value) -> String {
    if let Some(s) = payload.get("prompt").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(s) = payload.get("message").and_then(Value::as_str) {
        return s.to_string();
    }
    if let Some(arr) = payload.get("items").and_then(Value::as_array) {
        return format!("{} items", arr.len());
    }
    payload.to_string()
}

fn age_ms(created_ns: i64) -> i64 {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    (now - created_ns) / 1_000_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tokio::sync::mpsc;

    fn fresh() -> (TempDir, Arc<DaemonState>) {
        let tmp = TempDir::new().unwrap();
        let paths = super::super::paths::CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let state = DaemonState::new(paths).unwrap();
        (tmp, state)
    }

    fn add_dummy_session(state: &Arc<DaemonState>) -> u64 {
        let (tx, _rx) = mpsc::unbounded_channel();
        let (id, _) = state.register_session(100, None, None, None, tx);
        id
    }

    #[tokio::test]
    async fn ask_then_pending_then_take() {
        let (_tmp, state) = fresh();
        let target = add_dummy_session(&state);
        let asker = add_dummy_session(&state);

        let r = op_ask_ask(
            &state,
            asker,
            json!({
                "kind": "picker",
                "target": { "shell_id": target },
                "urgency": "normal",
                "payload": { "items": ["a", "b", "c"] }
            }),
        )
        .await
        .unwrap();
        assert!(r["request_id"].as_str().unwrap().starts_with("ask-"));
        assert_eq!(r["queued_at_target"].as_u64(), Some(target));

        let r = op_ask_pending(&state, target, json!({})).await.unwrap();
        assert_eq!(r["pending_count"].as_u64(), Some(1));

        let r = op_ask_take(&state, target, json!({})).await.unwrap();
        assert!(r["request_id"].as_str().unwrap().starts_with("ask-"));
        assert_eq!(r["kind"].as_str(), Some("picker"));
        assert_eq!(r["remaining_pending"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn take_critical_first() {
        let (_tmp, state) = fresh();
        let target = add_dummy_session(&state);
        let asker = add_dummy_session(&state);

        // Push two normal then one critical.
        for kind_payload in [
            ("picker", "normal"),
            ("picker", "normal"),
            ("dialog", "critical"),
        ] {
            op_ask_ask(
                &state,
                asker,
                json!({
                    "kind": kind_payload.0,
                    "target": { "shell_id": target },
                    "urgency": kind_payload.1,
                    "payload": {}
                }),
            )
            .await
            .unwrap();
        }
        let first = op_ask_take(&state, target, json!({})).await.unwrap();
        assert_eq!(first["kind"].as_str(), Some("dialog"));
        assert_eq!(first["urgency"].as_str(), Some("critical"));
    }

    #[tokio::test]
    async fn dismiss_specific() {
        let (_tmp, state) = fresh();
        let target = add_dummy_session(&state);
        let asker = add_dummy_session(&state);

        let r = op_ask_ask(
            &state,
            asker,
            json!({ "kind": "input", "target": {"shell_id": target}, "payload": {}}),
        )
        .await
        .unwrap();
        let req_id = r["request_id"].as_str().unwrap().to_string();

        let r = op_ask_dismiss(&state, target, json!({ "request_id": req_id }))
            .await
            .unwrap();
        assert_eq!(r["dismissed"].as_u64(), Some(1));

        // Second dismiss is a no-op.
        let r = op_ask_dismiss(&state, target, json!({ "request_id": "nothing" }))
            .await
            .unwrap();
        assert_eq!(r["dismissed"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn clear_inbox() {
        let (_tmp, state) = fresh();
        let target = add_dummy_session(&state);
        let asker = add_dummy_session(&state);

        for _ in 0..5 {
            op_ask_ask(
                &state,
                asker,
                json!({ "kind": "menu", "target": {"shell_id": target}, "payload": {}}),
            )
            .await
            .unwrap();
        }
        let r = op_ask_dismiss(&state, target, json!({ "all": true }))
            .await
            .unwrap();
        assert_eq!(r["dismissed"].as_u64(), Some(5));
        assert_eq!(state.ask_inbox.pending_count(target), 0);
    }

    #[tokio::test]
    async fn ask_to_unknown_shell_errors() {
        let (_tmp, state) = fresh();
        let asker = add_dummy_session(&state);
        let r = op_ask_ask(
            &state,
            asker,
            json!({ "kind": "input", "target": {"shell_id": 9999}, "payload": {}}),
        )
        .await;
        assert!(r.is_err());
    }
}
