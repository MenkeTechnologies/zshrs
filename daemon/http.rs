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
//! ```text
//! POST /op/<NAME>
//! Authorization: Bearer <token>      (optional; required if any
//!                                     tokens are configured)
//! Content-Type: application/json
//! <args JSON object>
//!
//! 200 OK   { "ok": true,  ...result }
//! 4xx/5xx  { "ok": false, "code": "...", "msg": "..." }
//!
//! GET /health     -> { "ok": true, "version": ..., "uptime_ms": ... }
//! GET /ops        -> { "ok": true, "ops": [...] } (op names enumerated)
//! GET /openapi    -> OpenAPI 3.1 doc (auto-derived from OP_NAMES;
//!                    alias /openapi.json -- see handler_openapi)
//! ```
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

/// Per-listener config materialised from `~/.zshrs/daemon.toml`
/// `[http]` section by `paths::load_http_config()`.
#[derive(Debug, Clone, Default)]
pub struct HttpConfig {
    /// Address to bind, e.g. "127.0.0.1:7733". Empty / None disables the
    /// listener entirely (the default).
    pub listen: Option<String>,
    /// Bearer-token registry. Empty means "no auth required" (only
    /// allowed when binding to a loopback address — see auth model
    /// above). Each token may carry a scope set; see `daemon::auth`
    /// for the full op→scope table and matcher rules.
    pub tokens: super::auth::TokenRegistry,
}

#[derive(Clone)]
struct AppState {
    daemon: Arc<DaemonState>,
    tokens: super::auth::TokenRegistry,
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
        tokens: cfg.tokens,
        started_at: std::time::Instant::now(),
    };

    let app = Router::new()
        .route("/health", get(handler_health))
        .route("/ops", get(handler_ops))
        .route("/openapi", get(handler_openapi))
        .route("/openapi.json", get(handler_openapi))
        .route("/metrics", get(handler_metrics))
        .route("/op/:name", post(handler_op))
        // Server-Sent Events streams for push-style ops. Each connection
        // registers a synthetic session on the daemon's broadcast bus,
        // filters frames by `event` kind, and forwards them as
        // `text/event-stream` records. See docs/DAEMON_AS_SERVICE.md
        // §"WATCH" + §"EVENT".
        .route("/stream/watch", get(handler_stream_watch))
        .route("/stream/events", get(handler_stream_events))
        .route("/stream/definitions", get(handler_stream_definitions))
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

/// Authorization: Bearer <token> check. Returns:
///   - `Ok(None)`        — registry is empty (no auth required)
///   - `Ok(Some(token))` — bearer matched a configured token
///   - `Err(401)`        — token required and missing or wrong
fn authorize<'a>(
    headers: &HeaderMap,
    registry: &'a super::auth::TokenRegistry,
) -> std::result::Result<Option<&'a super::auth::Token>, StatusCode> {
    if registry.is_empty() {
        return Ok(None);
    }
    let header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let secret = header.strip_prefix("Bearer ").unwrap_or("").trim();
    if secret.is_empty() {
        return Err(StatusCode::UNAUTHORIZED);
    }
    registry.lookup(secret).map(Some).ok_or(StatusCode::UNAUTHORIZED)
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

/// `GET /openapi` (alias `/openapi.json`) — auto-generated OpenAPI 3.1
/// document derived from `super::ops::OP_NAMES`. Each op gets one
/// `POST /op/{name}` entry; the meta endpoints (/health, /ops,
/// /metrics, /openapi, /stream/*) get their own paths.
///
/// We don't have per-op argument schemas (op handlers all accept a
/// generic `serde_json::Value`), so request/response bodies are typed
/// as the open `object`. That keeps the doc spec-compliant + useful
/// for SDK generation, curl examples, and Swagger-UI rendering even
/// without the deeper schemas.
///
/// Always-open (no auth gate) — same posture as `/health` + `/ops`,
/// so external tooling can discover the surface without a token.
async fn handler_openapi(State(s): State<AppState>) -> impl IntoResponse {
    let mut paths = serde_json::Map::new();

    // Meta endpoints first.
    paths.insert(
        "/health".to_string(),
        json!({
            "get": {
                "summary": "Liveness probe",
                "tags": ["meta"],
                "responses": {
                    "200": {
                        "description": "Daemon alive",
                        "content": {"application/json": {"schema": {
                            "type": "object",
                            "properties": {
                                "ok": {"type": "boolean"},
                                "version": {"type": "string"},
                                "uptime_ms": {"type": "integer", "format": "int64"},
                            },
                        }}},
                    }
                },
            }
        }),
    );
    paths.insert(
        "/ops".to_string(),
        json!({
            "get": {
                "summary": "List every op the daemon accepts",
                "tags": ["meta"],
                "responses": {
                    "200": {
                        "description": "Op list",
                        "content": {"application/json": {"schema": {
                            "type": "object",
                            "properties": {
                                "ok": {"type": "boolean"},
                                "ops": {"type": "array", "items": {"type": "string"}},
                            },
                        }}},
                    }
                },
            }
        }),
    );
    paths.insert(
        "/metrics".to_string(),
        json!({
            "get": {
                "summary": "Prometheus 0.0.4 metrics exposition",
                "tags": ["meta"],
                "responses": {
                    "200": {
                        "description": "Plain-text Prometheus exposition",
                        "content": {"text/plain": {"schema": {"type": "string"}}},
                    }
                },
            }
        }),
    );
    paths.insert(
        "/openapi".to_string(),
        json!({
            "get": {
                "summary": "This document",
                "tags": ["meta"],
                "responses": {
                    "200": {
                        "description": "OpenAPI 3.1 schema",
                        "content": {"application/json": {"schema": {"type": "object"}}},
                    }
                },
            }
        }),
    );

    // SSE streams. OpenAPI 3.1 doesn't have a dedicated SSE type, but
    // `text/event-stream` content with a string schema is the
    // pragmatic encoding most tooling understands.
    for (path, summary) in [
        (
            "/stream/watch",
            "fsnotify SSE: subscribe to filesystem-change events for a path",
        ),
        (
            "/stream/events",
            "Pub/sub SSE: subscribe to fanout events by topic pattern",
        ),
        (
            "/stream/definitions",
            "Definitions SSE: stream every recorder_ingest summary as it lands",
        ),
    ] {
        paths.insert(
            path.to_string(),
            json!({
                "get": {
                    "summary": summary,
                    "tags": ["streams"],
                    "responses": {
                        "200": {
                            "description": "Server-Sent Events stream",
                            "content": {"text/event-stream": {"schema": {"type": "string"}}},
                        }
                    },
                }
            }),
        );
    }

    // Per-op endpoints. Every op accepts and returns a generic JSON
    // object; auth requirement (when the bearer-token registry is
    // populated) is encoded once via the global security scheme below.
    for op in super::ops::OP_NAMES {
        let key = format!("/op/{}", op);
        paths.insert(
            key,
            json!({
                "post": {
                    "summary": format!("Invoke op `{}`", op),
                    "tags": ["ops"],
                    "requestBody": {
                        "required": false,
                        "content": {"application/json": {"schema": {"type": "object"}}},
                    },
                    "responses": {
                        "200": {
                            "description": "Op result envelope",
                            "content": {"application/json": {"schema": {"type": "object"}}},
                        },
                        "400": {
                            "description": "Bad arguments",
                            "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ErrPayload"}}},
                        },
                        "401": {"description": "Bearer token required or invalid"},
                        "403": {"description": "Bearer token lacks scope for this op"},
                        "404": {"description": "Unknown op"},
                        "500": {
                            "description": "Op failed",
                            "content": {"application/json": {"schema": {"$ref": "#/components/schemas/ErrPayload"}}},
                        },
                    },
                }
            }),
        );
    }

    // Auth: only declare the security requirement when the token
    // registry is populated. On loopback-only deployments (the
    // default) we leave it off so the OpenAPI doc reflects reality —
    // the daemon won't actually require a token.
    let security_scheme = json!({
        "type": "http",
        "scheme": "bearer",
        "description": "Bearer token configured under [http.tokens] in zshrs-daemon.toml.",
    });
    let mut components = json!({
        "schemas": {
            "ErrPayload": {
                "type": "object",
                "required": ["ok", "code", "message"],
                "properties": {
                    "ok": {"type": "boolean", "enum": [false]},
                    "code": {"type": "string"},
                    "message": {"type": "string"},
                },
            }
        }
    });
    let mut top = json!({
        "openapi": "3.1.0",
        "info": {
            "title": "zshrs-daemon",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "Single-host daemon backing the zshrs shell. Every op accepts JSON in, returns JSON out. See docs/DAEMON.md + docs/DAEMON_AS_SERVICE.md.",
        },
        "paths": paths,
    });
    if !s.tokens.is_empty() {
        components["securitySchemes"] = json!({"bearerAuth": security_scheme});
        top["security"] = json!([{"bearerAuth": []}]);
    }
    top["components"] = components;

    Json(top)
}

/// Prometheus 0.0.4 text-format exposition. Always-open (Prometheus
/// scrapers historically don't carry credentials; tunnel through a
/// reverse proxy if the daemon is reachable from outside the host).
async fn handler_metrics(State(s): State<AppState>) -> impl IntoResponse {
    s.daemon.metrics.record_http("/metrics", 200);
    let body = super::metrics::prometheus_text(&s.daemon);
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        body,
    )
}

/// `GET /stream/watch?path=DIR&recursive=BOOL` — subscribes the
/// caller to fsnotify events. Each event arrives as one SSE record:
///
/// ```text
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
    // SSE streams currently bypass scope checks — every authenticated
    // token can subscribe to every stream. Tighten if cross-tenant
    // dashboards become a real use case.
    // Refcounted subscribe so two SSE clients on the same path don't
    // have the first disconnect's drop break the second's stream. The
    // returned watch_id is captured by the SseGuardStream Drop so the
    // subscription is released when the TCP connection closes.
    let watch_id = if let Some(p) = q.path.as_deref() {
        let wp = super::fsnotify::WatchedPath {
            path: std::path::PathBuf::from(p),
            shard_slug: format!("http-watch-{}", super::shard::hash8(p)),
            source_root: p.to_string(),
            kind: super::fsnotify::WatchKind::Generic,
        };
        match s.daemon.fs_watcher.subscribe(wp, q.recursive.unwrap_or(false)) {
            Ok(id) => Some(id),
            Err(e) => {
                tracing::warn!(?e, "stream/watch: registration failed");
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "ok": false, "code": "watch_register", "msg": e.to_string() })),
                )
                    .into_response();
            }
        }
    } else {
        None
    };
    let stream = sse_event_stream_with_watch(&s.daemon, watch_id, |frame| {
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
/// ```text
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
    // SSE streams currently bypass scope checks — every authenticated
    // token can subscribe to every stream. Tighten if cross-tenant
    // dashboards become a real use case.
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
        client_id: Some(client_id),
        watch_id: None,
    };
    Sse::new(guarded)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// `GET /stream/definitions` — pushes one `event: defs` record every
/// time a `recorder_ingest` lands. Payload is the ingest summary
/// (events_ingested, rows_written, elapsed_ms, ...). Lets editor
/// plugins / dashboards refresh their view without polling.
///
/// `recorder_ingest` only broadcasts to sessions that opted in via
/// `definitions_subscribe`, so this handler flips the per-session
/// flag immediately after registering. The session's flag is dropped
/// alongside the session itself in SseGuardStream::Drop on TCP close.
async fn handler_stream_definitions(
    State(s): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = authorize(&headers, &s.tokens) {
        return code.into_response();
    }
    // SSE streams currently bypass scope checks — every authenticated
    // token can subscribe to every stream. Tighten if cross-tenant
    // dashboards become a real use case.
    let stream = sse_event_stream_with_watch(&s.daemon, None, |frame| {
        if let Frame::Event { event, payload } = frame {
            // recorder_ingest emits broadcast Frame::Event { event:
            // "recorder_ingested", payload: { events_ingested, ... } }.
            if event == "recorder_ingested" {
                return Some(("defs".to_string(), payload.clone()));
            }
        }
        None
    });
    // Auto-subscribe the synthetic session created by
    // sse_event_stream_with_watch. The Drop tears the session down so
    // the flag dies with it; no explicit unsubscribe needed.
    if let Some(client_id) = stream.client_id() {
        let _ = s.daemon.set_definitions_subscribed(client_id, true);
    }
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response()
}

/// Build an SSE-event stream from the daemon's broadcast bus. Registers
/// a synthetic session, hooks an UnboundedReceiver of Frames, and maps
/// each frame through `pick` — `Some((event_name, payload))` becomes
/// one SSE record, `None` is dropped silently. On stream drop (TCP
/// close) the session auto-deregisters AND, if `watch_id` is `Some`,
/// the corresponding fsnotify subscription is released too.
fn sse_event_stream_with_watch<F>(
    state: &Arc<DaemonState>,
    watch_id: Option<u64>,
    pick: F,
) -> SseGuardStream<impl Stream<Item = std::result::Result<Event, Infallible>>>
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
    SseGuardStream {
        inner: Box::pin(stream),
        state: state_for_drop,
        client_id: Some(client_id),
        watch_id,
    }
}

struct SseGuardStream<S> {
    inner: std::pin::Pin<Box<S>>,
    state: Arc<DaemonState>,
    client_id: Option<u64>,
    /// Set when this SSE stream owns a refcounted fsnotify
    /// subscription (currently only `/stream/watch`). The Drop releases
    /// it via `fs_watcher.unsubscribe(id)` so SSE TCP-close cleans up
    /// the watch automatically — no leaked fsnotify registration when
    /// the client disconnects without an explicit `watch_unsubscribe`.
    watch_id: Option<u64>,
}

impl<S> SseGuardStream<S> {
    /// Synthetic-session id this stream is bound to. Lets handlers
    /// (`/stream/definitions`) call `state.set_definitions_subscribed`
    /// on the right session before returning the stream.
    fn client_id(&self) -> Option<u64> {
        self.client_id
    }
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
        if let Some(id) = self.watch_id.take() {
            let removed = self.state.fs_watcher.unsubscribe(id);
            tracing::info!(watch_id = id, removed, "sse fsnotify subscription released");
        }
        if let Some(cid) = self.client_id.take() {
            self.state.unregister_session(cid);
            tracing::info!(client_id = cid, "sse session unregistered");
        }
    }
}

async fn handler_op(
    State(s): State<AppState>,
    Path(name): Path<String>,
    headers: HeaderMap,
    body: Option<Json<Value>>,
) -> impl IntoResponse {
    let token = match authorize(&headers, &s.tokens) {
        Ok(t) => t,
        Err(code) => {
            return (
                code,
                Json(json!({
                    "ok": false,
                    "code": "unauthorized",
                    "msg": "missing or invalid bearer token",
                })),
            );
        }
    };
    // Scope check — only enforced when a token is configured AND the
    // token has a non-empty scope set. Unscoped (legacy) tokens grant
    // full access via Token::allows. Op→required-scope mapping lives
    // in daemon/auth.rs:op_scope; unmapped ops fall back to
    // `meta.admin` (deny-by-default for any new op until added).
    if let Some(t) = token {
        let required = super::auth::op_scope(&name);
        if !t.allows(required) {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "ok": false,
                    "code": "scope_denied",
                    "msg": format!("token `{}` lacks scope `{required}` for op `{name}`", t.label),
                    "required_scope": required,
                    "granted_scopes": t.granted_scopes(),
                })),
            );
        }
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

    let path_label = format!("/op/{name}");
    match dispatch_result {
        Ok(payload) => {
            s.daemon.metrics.record_http(&path_label, 200);
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
                "bad_args" | "bad_cron" | "bad_format" | "bad_pattern" => {
                    StatusCode::BAD_REQUEST
                }
                "unauthorized" => StatusCode::UNAUTHORIZED,
                "no_such_file"
                | "no_such_kind"
                | "no_such_function"
                | "unknown_op" => StatusCode::NOT_FOUND,
                "busy" => StatusCode::CONFLICT,
                "wrong_token" => StatusCode::FORBIDDEN,
                "timeout" => StatusCode::REQUEST_TIMEOUT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            s.daemon.metrics.record_http(&path_label, status.as_u16());
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
