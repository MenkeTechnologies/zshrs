//! HTTP listener for the daemon — exposes the same op surface as the
//! Unix-socket IPC path so any HTTP client (curl, httpie, fetch from a
//! browser, vim+system, etc.) can call the daemon directly.
//!
//! Architecture: see docs/DAEMON_AS_SERVICE.md. Routes are intentionally
//! a 1:1 mirror of the IPC `ops::dispatch` surface — no separate
//! request/response shapes — so the public API is identical regardless
//! of transport.
//!
//! Wire format:
//!
//!     POST /op/<NAME>
//!     Authorization: Bearer <token>      (optional; required if any
//!                                         tokens are configured)
//!     Content-Type: application/json
//!     <args JSON object>
//!
//!     200 OK   { "ok": true,  ...result }
//!     4xx/5xx  { "ok": false, "code": "...", "msg": "..." }
//!
//!     GET /health     → { "ok": true, "version": ..., "uptime_ms": ... }
//!     GET /ops        → { "ok": true, "ops": [...] } (op names enumerated)
//!     GET /openapi    → JSON schema (TODO)
//!
//! Auth model:
//!   - No tokens configured + binding to a loopback address: open access
//!     (same trust model as the unix socket on a single-user box).
//!   - No tokens configured + binding to a non-loopback address: REJECTED
//!     at startup. Refusing to listen on the network without auth is a
//!     hard safety floor — no surprise public exposure.
//!   - Tokens configured: every request must carry a matching Bearer
//!     token in the Authorization header.
//!
//! Dependencies: axum / tower / hyper, all under the tokio team. See
//! daemon/Cargo.toml for the durability rationale.

use std::collections::HashSet;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::{Stream, StreamExt};

use super::ipc::Frame;
use super::state::DaemonState;
use super::Result;

/// Per-listener config materialised from `~/.config/zshrs/daemon.toml`
/// `[http]` section by `paths::load_http_config()`.
#[derive(Debug, Clone, Default)]
pub struct HttpConfig {
    /// Address to bind, e.g. "127.0.0.1:7733". Empty / None disables the
    /// listener entirely (the default).
    pub listen: Option<String>,
    /// Set of valid bearer tokens. Empty means "no auth required" (only
    /// allowed when binding to a loopback address — see auth model above).
    pub tokens: HashSet<String>,
}

#[derive(Clone)]
struct AppState {
    daemon: Arc<DaemonState>,
    tokens: Arc<HashSet<String>>,
    started_at: std::time::Instant,
}

/// Spawn the HTTP listener as a background tokio task. Returns
/// immediately; errors during accept are logged via tracing.
///
/// `cfg.listen` of `None` is a silent no-op so callers can unconditionally
/// invoke this and let the config decide whether the listener exists.
pub async fn serve_http(cfg: HttpConfig, daemon: Arc<DaemonState>) -> Result<()> {
    let Some(listen) = cfg.listen.clone() else {
        tracing::info!("http listener disabled (no [http].listen in daemon.toml)");
        return Ok(());
    };
    let addr: SocketAddr = listen
        .parse()
        .map_err(|e| super::DaemonError::other(format!("[http].listen parse: {e}")))?;

    if !addr.ip().is_loopback() && cfg.tokens.is_empty() {
        return Err(super::DaemonError::other(format!(
            "refusing to bind http listener on non-loopback {addr} without [http.tokens] — \
             configure at least one bearer token first"
        )));
    }

    let token_count = cfg.tokens.len();
    let app_state = AppState {
        daemon,
        tokens: Arc::new(cfg.tokens),
        started_at: std::time::Instant::now(),
    };

    let app = Router::new()
        .route("/health", get(handler_health))
        .route("/ops", get(handler_ops))
        .route("/op/:name", post(handler_op))
        // Server-Sent Events streams for push-style ops. Each connection
        // registers a synthetic session on the daemon's broadcast bus,
        // filters frames by `event` kind, and forwards them as
        // `text/event-stream` records. See docs/DAEMON_AS_SERVICE.md
        // §"WATCH" + §"EVENT".
        .route("/stream/watch", get(handler_stream_watch))
        .route("/stream/events", get(handler_stream_events))
        .with_state(app_state);

    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
        super::DaemonError::other(format!("[http] tcp bind {addr}: {e}"))
    })?;
    tracing::info!(%addr, tokens = token_count, "http listener up");

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(?e, "http listener exited");
        }
    });
    Ok(())
}

/// Authorization: Bearer <token> check. Returns the bearer token string
/// if it matches; returns `None` if auth is open (no tokens configured).
/// Returns an error response if a token is required and missing/wrong.
fn authorize(headers: &HeaderMap, tokens: &HashSet<String>) -> std::result::Result<(), StatusCode> {
    if tokens.is_empty() {
        return Ok(());
    }
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let token = header.strip_prefix("Bearer ").unwrap_or("").trim();
    if token.is_empty() || !tokens.contains(token) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

async fn handler_health(State(s): State<AppState>) -> impl IntoResponse {
    let uptime_ms = s.started_at.elapsed().as_millis() as u64;
    Json(json!({
        "ok": true,
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_ms": uptime_ms,
    }))
}

async fn handler_ops(State(_s): State<AppState>) -> impl IntoResponse {
    Json(json!({
        "ok": true,
        "ops": super::ops::OP_NAMES,
    }))
}

/// `GET /stream/watch?path=DIR&recursive=BOOL` — subscribes the
/// caller to fsnotify events. Each event arrives as one SSE record:
///
/// ```
/// event: fs
/// data: {"path":"/path/that/changed", "shard":"...", "trigger_path":"...", ...}
/// ```
///
/// On disconnect: the synthetic session is unregistered and the
/// directory watch is removed. (v1: registration happens once per SSE
/// connection; multiple subscribers to the same dir share one watch.)
#[derive(serde::Deserialize)]
struct WatchQuery {
    path: Option<String>,
    recursive: Option<bool>,
}

async fn handler_stream_watch(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<WatchQuery>,
) -> impl IntoResponse {
    if let Err(code) = authorize(&headers, &s.tokens) {
        return code.into_response();
    }
    if let Some(p) = q.path.as_deref() {
        let wp = super::fsnotify::WatchedPath {
            path: std::path::PathBuf::from(p),
            shard_slug: format!("http-watch-{}", super::shard::hash8(p)),
            source_root: p.to_string(),
            kind: super::fsnotify::WatchKind::Generic,
        };
        if let Err(e) = s.daemon.fs_watcher.watch_path(wp, q.recursive.unwrap_or(false)) {
            tracing::warn!(?e, "stream/watch: registration failed");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "code": "watch_register", "msg": e.to_string() })),
            )
                .into_response();
        }
    }
    let stream = sse_event_stream(&s.daemon, |frame| {
        // Forward only fs-shaped events. The fsnotify side emits
        // `shard_updated` per debounced change; map that to SSE event=fs
        // for clarity.
        if let Frame::Event { event, payload } = frame {
            if event == "shard_updated" {
                return Some(("fs".to_string(), payload.clone()));
            }
        }
        None
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// `GET /stream/events?channel=GLOB` — subscribes to the daemon's
/// pubsub bus. Each `daemon.event.publish` matching the GLOB arrives as:
///
/// ```
/// event: pub
/// data: {"channel":"build", "payload":{...}, "sender":..., "ts_ns":...}
/// ```
///
/// `channel` query param defaults to `*` (everything). v1 filters by
/// substring match server-side; full glob support arrives with the
/// existing pubsub op's filter machinery.
#[derive(serde::Deserialize)]
struct EventsQuery {
    channel: Option<String>,
}

async fn handler_stream_events(
    State(s): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> impl IntoResponse {
    if let Err(code) = authorize(&headers, &s.tokens) {
        return code.into_response();
    }
    // Pubsub patterns are `<scope>.<topic>` (see daemon/pubsub.rs).
    // Default `*.*` = every scope, every topic.
    // Caller-supplied `?channel=PATTERN` is passed through verbatim
    // (so callers can scope to e.g. `shell:5.build` or `*.build_done`).
    let pattern = q.channel.unwrap_or_else(|| "*.*".to_string());

    // Set up an SSE-backed synthetic session, then drive the existing
    // op_subscribe through it so this connection joins the pubsub
    // routing table for the requested topic glob. Without this,
    // op_publish would never select us — `state.publish` only
    // delivers to subscribers whose registered topic-pattern matches.
    let (tx, rx) = mpsc::unbounded_channel::<Frame>();
    let pid = std::process::id() as i32;
    let (client_id, _session_id) = s.daemon.register_session(
        pid,
        Some("http-sse".to_string()),
        None,
        Some("http-sse-events".to_string()),
        tx,
    );
    let sub_args = json!({ "pattern": pattern });
    if let Err(e) = super::ops::dispatch(&s.daemon, client_id, "subscribe", sub_args).await {
        s.daemon.unregister_session(client_id);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "ok": false, "code": e.code, "msg": e.msg })),
        )
            .into_response();
    }

    let state_for_drop = Arc::clone(&s.daemon);
    let stream = UnboundedReceiverStream::new(rx).filter_map(|frame| {
        // Pubsub `op_publish` emits Frame::Event { event: "match",
        // payload: { topic, data, scope, subscription_id } }.
        if let Frame::Event { event, payload } = frame {
            if event == "match" {
                return Some(Ok(Event::default()
                    .event("pub")
                    .data(payload.to_string())));
            }
        }
        None
    });
    let guarded = SseGuardStream {
        inner: Box::pin(stream),
        state: state_for_drop,
        client_id,
    };
    Sse::new(guarded)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// Build an SSE-event stream from the daemon's broadcast bus. Registers
/// a synthetic session, hooks an UnboundedReceiver of Frames, and maps
/// each frame through `pick` — `Some((event_name, payload))` becomes
/// one SSE record, `None` is dropped silently. The session
/// auto-deregisters on stream drop (TCP close).
fn sse_event_stream<F>(
    state: &Arc<DaemonState>,
    pick: F,
) -> impl Stream<Item = std::result::Result<Event, Infallible>>
where
    F: Fn(&Frame) -> Option<(String, Value)> + Send + 'static,
{
    let (tx, rx) = mpsc::unbounded_channel::<Frame>();
    let pid = std::process::id() as i32;
    let (client_id, _session_id) = state.register_session(
        pid,
        Some("http-sse".to_string()),
        None,
        Some("http-sse".to_string()),
        tx,
    );
    let state_for_drop = Arc::clone(state);
    let stream = UnboundedReceiverStream::new(rx)
        .filter_map(move |frame| {
            let pair = pick(&frame);
            pair.map(|(event_name, payload)| {
                Ok(Event::default()
                    .event(event_name)
                    .data(payload.to_string()))
            })
        });
    // Wrap in a guard stream so the synthetic session is unregistered
    // when the SSE connection drops.
    SseGuardStream {
        inner: Box::pin(stream),
        state: state_for_drop,
        client_id,
    }
}

struct SseGuardStream<S> {
    inner: std::pin::Pin<Box<S>>,
    state: Arc<DaemonState>,
    client_id: u64,
}

impl<S> Stream for SseGuardStream<S>
where
    S: Stream<Item = std::result::Result<Event, Infallible>> + Send,
{
    type Item = std::result::Result<Event, Infallible>;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

impl<S> Drop for SseGuardStream<S> {
    fn drop(&mut self) {
        self.state.unregister_session(self.client_id);
        tracing::info!(client_id = self.client_id, "sse session unregistered");
    }
}

async fn handler_op(
    State(s): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> impl IntoResponse {
    if let Err(code) = authorize(&headers, &s.tokens) {
        return (
            code,
            Json(json!({ "ok": false, "code": "unauthorized", "msg": "missing or invalid bearer token" })),
        );
    }
    let args = body.map(|Json(v)| v).unwrap_or_else(|| json!({}));

    // Register a short-lived session for this request so session-aware
    // ops (publish/send/tag/notify/...) can find an `origin_scope` to
    // attach the call to. The session is unregistered after dispatch.
    // Outbound channel is /dev/null since the response goes back on
    // the HTTP body, not via the broadcast bus.
    let (tx, _rx) = mpsc::unbounded_channel::<Frame>();
    let pid = std::process::id() as i32;
    let (client_id, _session_id) = s.daemon.register_session(
        pid,
        Some(format!("http")),
        None,
        Some(format!("http-op:{}", name)),
        tx,
    );
    let dispatch_result = super::ops::dispatch(&s.daemon, client_id, &name, args).await;
    s.daemon.unregister_session(client_id);

    match dispatch_result {
        Ok(payload) => {
            // Merge {ok:true} with the op's payload so HTTP shape matches
            // the existing socket response shape.
            let mut out = match payload {
                Value::Object(map) => map,
                other => {
                    let mut m = serde_json::Map::new();
                    m.insert("payload".to_string(), other);
                    m
                }
            };
            out.insert("ok".to_string(), Value::Bool(true));
            (StatusCode::OK, Json(Value::Object(out)))
        }
        Err(err) => {
            let status = match err.code.as_str() {
                "bad_args" | "no_such_file" => StatusCode::BAD_REQUEST,
                "unauthorized" => StatusCode::UNAUTHORIZED,
                "unknown_op" => StatusCode::NOT_FOUND,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (
                status,
                Json(json!({
                    "ok": false,
                    "code": err.code,
                    "msg": err.msg,
                })),
            )
        }
    }
}
