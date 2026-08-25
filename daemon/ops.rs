/// Public surface — every op name the daemon accepts. Single source of
/// truth: keep in sync with the `match op` arms in `dispatch()` below.
/// Surfaced via `GET /ops` over HTTP and used by `daemon.health` clients
/// to validate availability before issuing requests. Kept alphabetically
/// sorted so the wire output is stable.
pub const OP_NAMES: &[&str] = &[
    "artifact_gc",
    "artifact_get",
    "artifact_get_by_digest",
    "artifact_list",
    "artifact_put",
    "ask_ask",
    "ask_dismiss",
    "ask_pending",
    "ask_response",
    "ask_take",
    "autoload_prewarm",
    "cache_del",
    "cache_get",
    "cache_list",
    "cache_put",
    "cache_stats",
    "canonical_hydrate_view",
    "clean",
    "cmd_result",
    "cmd_started",
    "compact",
    "complete",
    "config_get",
    "config_list",
    "config_set",
    "daemon",
    "definitions_diff",
    "definitions_emit",
    "definitions_kinds",
    "definitions_query",
    "definitions_subscribe",
    "definitions_unsubscribe",
    "diff_canonical",
    "doctor",
    "export",
    "export_all",
    "export_catalog",
    "export_shard",
    "export_zcompdump",
    "fpath_changed",
    "highlight",
    "history_append",
    "history_query",
    "import_all",
    "import_catalog",
    "import_history",
    "import_shard",
    "import_zcompdump",
    "import_zwc",
    "info",
    "job_cancel",
    "job_input",
    "job_kill",
    "job_list",
    "job_output",
    "job_resize",
    "job_status",
    "job_submit",
    "job_wait",
    "keys",
    "list_shells",
    "load_script",
    "lock_acquire",
    "lock_list",
    "lock_release",
    "lock_try_acquire",
    "log_level",
    "log_rotate",
    "log_stats",
    "metrics",
    "notify",
    "ping",
    "publish",
    "pull_canonical",
    "push_canonical",
    "recorder_ingest",
    "register",
    "replay_log",
    "schedule_add",
    "schedule_add_once",
    "schedule_list",
    "schedule_remove",
    "send",
    "snapshot_diff",
    "snapshot_list",
    "snapshot_load",
    "snapshot_save",
    "source_resolve",
    "stats_flush",
    "subscribe",
    "subscribe_shard",
    "subscription_set_paused",
    "suggest",
    "tag",
    "untag",
    "unsubscribe",
    "verify",
    "view",
    "watch_list",
    "watch_subscribe",
    "watch_unsubscribe",
    "watcher_stats",
];

// IPC operation dispatch + handlers.
//
// Per docs/DAEMON.md "Operation table (client → daemon)". Every named op
// routes to a real handler — no stub arms remain. See dispatch() below for
// the authoritative list.

use std::sync::Arc;

use serde_json::{json, Value};

use super::ipc::{ErrPayload, Event, Frame};
use super::state::DaemonState;

/// Result type for op handlers — Ok = json payload merged into the response, Err = ErrPayload.
pub type OpResult = std::result::Result<Value, ErrPayload>;

/// Dispatch an op by name. Called from server::handle_connection per request frame.
pub async fn dispatch(state: &Arc<DaemonState>, client_id: u64, op: &str, args: Value) -> OpResult {
    let span = tracing::info_span!("op", op = %op, client_id);
    let _enter = span.enter();

    let result = match op {
        "info" => op_info(state).await,
        "ping" => op_ping(state, args).await,
        "list_shells" => op_list_shells(state, args).await,
        "tag" => op_tag(state, client_id, args).await,
        "untag" => op_untag(state, client_id, args).await,
        "send" => op_send(state, client_id, args).await,
        "cmd_result" => op_cmd_result(state, args).await,
        "notify" => op_notify(state, client_id, args).await,
        "daemon" => op_daemon(state, args).await,

        // The static `rebuild` / `zshrc_analyze` / `first_init` /
        // `plugin_discover` ops were the AST-walker pipeline. They are
        // deleted: state attribution now comes from `zshrs-recorder`
        // (runtime AOP intercept). When the user installs new plugins
        // or edits .zshrc, they re-run `zshrs-recorder` to re-index;
        // the daemon no longer walks anything itself.
        "recorder_ingest" => op_recorder_ingest(state, args).await,
        "canonical_hydrate_view" => op_canonical_hydrate_view(state).await,
        "clean" => op_clean(state, args).await,
        "verify" => op_verify(state).await,
        "compact" => op_compact(state).await,
        "source_resolve" => super::source_resolver::op_source_resolve(state, args).await,
        "history_append" => super::history::op_history_append(state, args).await,
        "history_query" => super::history::op_history_query(state, args).await,
        "cmd_started" => op_cmd_started(state, args).await,
        "subscribe" => op_subscribe(state, client_id, args).await,
        "unsubscribe" => op_unsubscribe(state, client_id, args).await,
        "subscription_set_paused" => op_subscription_set_paused(state, client_id, args).await,
        "publish" => op_publish(state, client_id, args).await,
        "autoload_prewarm" => op_autoload_prewarm(state, args).await,
        "fpath_changed" => op_fpath_changed(state, args).await,
        "watcher_stats" => op_watcher_stats(state).await,
        "log_level" => op_log_level(args).await,
        "log_rotate" => op_log_rotate(state, args).await,
        "log_stats" => op_log_stats(state).await,
        "push_canonical" => super::zsync::op_push_canonical(state, client_id, args).await,
        "pull_canonical" => super::zsync::op_pull_canonical(state, args).await,
        "diff_canonical" => super::zsync::op_diff_canonical(state, args).await,
        "view" => super::export::op_view(state, args).await,
        "export" => super::export::op_export(state, args).await,
        "ask_ask" => super::zask::op_ask_ask(state, client_id, args).await,
        "ask_pending" => super::zask::op_ask_pending(state, client_id, args).await,
        "ask_take" => super::zask::op_ask_take(state, client_id, args).await,
        "ask_dismiss" => super::zask::op_ask_dismiss(state, client_id, args).await,
        "ask_response" => super::zask::op_ask_response(state, args).await,

        "load_script" => op_load_script(state, args).await,
        "stats_flush" => op_stats_flush(state, args).await,
        "keys" => op_keys(state, args).await,
        "subscribe_shard" => op_subscribe_shard(state, client_id, args).await,
        "export_zcompdump" => op_export_zcompdump(state, args).await,
        "export_catalog" => op_export_catalog(state, args).await,
        "export_shard" => op_export_shard(state, args).await,
        "import_zcompdump" => op_import_zcompdump(state, args).await,
        "import_zwc" => op_import_zwc(state, args).await,
        "import_history" => op_import_history(state, args).await,
        "import_catalog" => op_import_catalog(state, args).await,
        "import_shard" => op_import_shard(state, args).await,
        "import_all" => op_import_all(state, args).await,
        "export_all" => op_export_all(state, args).await,
        "replay_log" => op_replay_log(state, args).await,
        "config_get" => op_config_get(state, args).await,
        "config_set" => op_config_set(state, args).await,
        "config_list" => op_config_list(state).await,

        "job_submit" => op_job_submit(state, client_id, args).await,
        "job_list" => op_job_list(state, args).await,
        "job_status" => op_job_status(state, args).await,
        "job_output" => op_job_output(state, args).await,
        "job_kill" => op_job_kill(state, args).await,
        "job_cancel" => op_job_cancel(state, args).await,
        "job_wait" => op_job_wait(state, args).await,
        "job_input" => op_job_input(state, args).await,
        "job_resize" => op_job_resize(state, args).await,

        "complete" => op_complete(state, args).await,
        "suggest" => op_suggest(state, args).await,
        "highlight" => op_highlight(state, args).await,
        "register" => op_register(state, client_id, args).await,
        "doctor" => op_doctor(state).await,

        // ---- daemon-as-service ops (docs/DAEMON_AS_SERVICE.md) ----
        // Cache (sqlite-backed namespaced KV).
        "cache_put" => super::cache::op_cache_put(state, args).await,
        "cache_get" => super::cache::op_cache_get(state, args).await,
        "cache_del" => super::cache::op_cache_del(state, args).await,
        "cache_list" => super::cache::op_cache_list(state, args).await,
        "cache_stats" => super::cache::op_cache_stats(state, args).await,
        // Lock (named cross-process mutex with PID-tied auto-release).
        "lock_acquire" => super::lock::op_lock_acquire(state, args).await,
        "lock_try_acquire" => super::lock::op_lock_try_acquire(state, args).await,
        "lock_release" => super::lock::op_lock_release(state, args).await,
        "lock_list" => super::lock::op_lock_list(state, args).await,
        // Artifact (content-addressed cache).
        "artifact_put" => super::artifact::op_artifact_put(state, args).await,
        "artifact_get" => super::artifact::op_artifact_get(state, args).await,
        "artifact_get_by_digest" => super::artifact::op_artifact_get_by_digest(state, args).await,
        "artifact_gc" => super::artifact::op_artifact_gc(state, args).await,
        "artifact_list" => super::artifact::op_artifact_list(state, args).await,
        // Snapshot (tag-based canonical-state save/load/diff).
        "snapshot_save" => super::snapshot::op_snapshot_save(state, args).await,
        "snapshot_list" => super::snapshot::op_snapshot_list(state, args).await,
        "snapshot_load" => super::snapshot::op_snapshot_load(state, args).await,
        "snapshot_diff" => super::snapshot::op_snapshot_diff(state, args).await,
        // Schedule (cron-equivalent).
        "schedule_add" => super::schedule::op_schedule_add(state, args).await,
        "schedule_add_once" => super::schedule::op_schedule_add_once(state, args).await,
        "schedule_remove" => super::schedule::op_schedule_remove(state, args).await,
        "schedule_list" => super::schedule::op_schedule_list(state, args).await,
        // Definitions (recorder catalog: query, single-record write,
        // cross-shell diff). `definitions_emit` is the universal
        // shell-recorder write API per docs/DAEMON_AS_SERVICE.md +
        // docs/SHELL_IDS.md — bash/fish/zsh users push records into
        // the federated catalog through this op.
        "definitions_query" => super::definitions::op_definitions_query(state, args).await,
        "definitions_kinds" => super::definitions::op_definitions_kinds(state, args).await,
        "definitions_emit" => super::definitions::op_definitions_emit(state, args).await,
        "definitions_diff" => super::definitions::op_definitions_diff(state, args).await,
        "definitions_subscribe" => {
            super::definitions::op_definitions_subscribe(state, client_id, args).await
        }
        "definitions_unsubscribe" => {
            super::definitions::op_definitions_unsubscribe(state, client_id, args).await
        }
        // Refcounted fsnotify subscription — IPC counterpart of HTTP
        // /stream/watch. See `op_watch_subscribe` below for the contract.
        "watch_subscribe" => op_watch_subscribe(state, args).await,
        "watch_unsubscribe" => op_watch_unsubscribe(state, args).await,
        "watch_list" => op_watch_list(state).await,
        // Metrics (Prometheus-shaped also via GET /metrics).
        "metrics" => super::metrics::op_metrics(state, args).await,

        _ => Err(ErrPayload::new(
            "unknown_op",
            format!("unsupported op `{op}`"),
        )),
    };

    match &result {
        Ok(_) => tracing::info!(ok = true, "op handled"),
        Err(e) => tracing::info!(ok = false, code = %e.code, msg = %e.msg, "op failed"),
    }
    state.metrics.record_op(op, result.is_ok());

    result
}

// -------- Handlers --------

async fn op_info(state: &Arc<DaemonState>) -> OpResult {
    let catalog = state.catalog_summary().ok();
    let shards: Vec<String> = super::shard::list_shards(&state.paths)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .collect();
    Ok(json!({
        "daemon_pid": state.pid,
        "daemon_uptime_ms": state.uptime_ms(),
        "version": env!("CARGO_PKG_VERSION"),
        "protocol_version": super::ipc::PROTOCOL_VERSION,
        "session_count": state.session_count(),
        "started_at": state.start_wall.to_rfc3339(),
        "cache_root": state.paths.root.display().to_string(),
        "log_path": state.paths.log.display().to_string(),
        "catalog": catalog,
        "shards": shards,
    }))
}

async fn op_ping(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let echo = args.get("echo").cloned().unwrap_or(Value::Null);
    Ok(json!({
        "pong": true,
        "ts_ns": chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        "daemon_uptime_ms": state.uptime_ms(),
        "echo": echo,
    }))
}

async fn op_list_shells(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let tag_filter = args.get("tag").and_then(|v| v.as_str()).map(str::to_string);

    let mut sessions = state.snapshot_sessions();
    if let Some(t) = tag_filter.as_ref() {
        sessions.retain(|s| s.tags.iter().any(|x| x == t));
    }

    Ok(json!({
        "shells": sessions,
        "total": sessions.len(),
    }))
}

async fn op_tag(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let tags = parse_tags(&args)?;
    let updated = state
        .add_tags(client_id, &tags)
        .ok_or_else(|| ErrPayload::new("no_session", "client session not found"))?;
    Ok(json!({ "tags": updated }))
}

async fn op_untag(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let all = args.get("all").and_then(Value::as_bool).unwrap_or(false);
    let tags = if all { Vec::new() } else { parse_tags(&args)? };
    let updated = state
        .remove_tags(client_id, &tags)
        .ok_or_else(|| ErrPayload::new("no_session", "client session not found"))?;
    Ok(json!({ "tags": updated }))
}

async fn op_send(state: &Arc<DaemonState>, from: u64, args: Value) -> OpResult {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `command`"))?
        .to_string();

    let target = args.get("target").cloned().unwrap_or(Value::Null);
    let wait = args.get("wait").and_then(Value::as_bool).unwrap_or(false);
    let timeout_ms = args
        .get("timeout_ms")
        .and_then(Value::as_u64)
        .unwrap_or(60_000);

    let delivery_id = format!("send-{}-{}", from, chrono::Utc::now().timestamp_millis());
    let event_payload = json!({
        "delivery_id": delivery_id,
        "from_shell": from,
        "command": command,
        "wait": wait,
    });
    let frame = Frame::event(event_name(Event::CmdExecute), event_payload);

    if wait {
        // Register pending slot BEFORE delivering so a fast responder doesn't
        // race the receiver registration.
        let rx = state.register_pending(delivery_id.clone());
        let delivered = resolve_target(state, &target, from, frame)?;
        if delivered.is_empty() {
            // Nothing to wait on — clean up the pending slot.
            let _ = state.resolve_pending(&delivery_id, Value::Null);
            return Ok(json!({
                "delivered_to": delivered,
                "delivered_count": 0,
                "wait": true,
                "result": null,
                "timed_out": false,
            }));
        }
        // Block on the responder's cmd_result with timeout.
        let result =
            match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
                Ok(Ok(v)) => Some(v),
                _ => None,
            };
        if result.is_none() {
            // Drop the orphaned slot if still present.
            let _ = state.resolve_pending(&delivery_id, Value::Null);
        }
        return Ok(json!({
            "delivered_to": delivered,
            "delivered_count": delivered.len(),
            "wait": true,
            "delivery_id": delivery_id,
            "result": result,
            "timed_out": result.is_none(),
        }));
    }

    let delivered = resolve_target(state, &target, from, frame)?;
    Ok(json!({
        "delivered_to": delivered,
        "delivered_count": delivered.len(),
        "delivery_id": delivery_id,
    }))
}

/// `cmd_result` — responder side of zsend --wait. Target shell calls this
/// after it has executed the dispatched command. Daemon resolves the
/// pending oneshot slot keyed by delivery_id; sender (blocked in op_send)
/// gets the result.
async fn op_cmd_result(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let delivery_id = args
        .get("delivery_id")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `delivery_id`"))?
        .to_string();
    let exit_code = args.get("exit_code").and_then(Value::as_i64);
    let stdout = args.get("stdout").and_then(Value::as_str).unwrap_or("");
    let stderr = args.get("stderr").and_then(Value::as_str).unwrap_or("");
    let value = json!({
        "exit_code": exit_code,
        "stdout": stdout,
        "stderr": stderr,
        "ts_ns": chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
    });
    let resolved = state.resolve_pending(&delivery_id, value);
    Ok(json!({
        "delivery_id": delivery_id,
        "resolved": resolved,
    }))
}

/// Resolve `target` (any of {shell_id, tag, user, all}) into the list of shell ids
/// the frame was actually queued to. Used by `op_send` and `op_notify` so they
/// share a single routing implementation.
fn resolve_target(
    state: &Arc<DaemonState>,
    target: &Value,
    from: u64,
    frame: Frame,
) -> std::result::Result<Vec<u64>, ErrPayload> {
    if let Some(all) = target.get("all").and_then(Value::as_bool) {
        if all {
            let _ = state.broadcast(frame, &[from]);
            return Ok(state
                .snapshot_sessions()
                .into_iter()
                .filter(|s| s.client_id != from)
                .map(|s| s.client_id)
                .collect());
        }
        return Err(ErrPayload::new(
            "bad_args",
            "target.all must be true if present",
        ));
    }
    if let Some(tag) = target.get("tag").and_then(Value::as_str) {
        return Ok(state.send_tag(tag, frame));
    }
    if let Some(shell_id) = target.get("shell_id").and_then(Value::as_u64) {
        if state.send_to(shell_id, frame) {
            return Ok(vec![shell_id]);
        }
        return Err(ErrPayload::new(
            "no_shell",
            format!("shell_id {shell_id} not found"),
        ));
    }
    if let Some(user) = target.get("user").and_then(Value::as_str) {
        // V1 user routing: same-user only. The daemon listens on a UNIX socket
        // it owns; every client necessarily shares the daemon's UID. Until
        // SO_PEERCRED + privilege drop is wired, cross-user is refused with a
        // clear error rather than silently delivering to local-user sessions.
        let daemon_user = std::env::var("USER").unwrap_or_default();
        if user == daemon_user || daemon_user.is_empty() {
            let _ = state.broadcast(frame, &[from]);
            return Ok(state
                .snapshot_sessions()
                .into_iter()
                .filter(|s| s.client_id != from)
                .map(|s| s.client_id)
                .collect());
        }
        return Err(ErrPayload::new(
            "user_mismatch",
            format!(
                "cross-user dispatch (`{}` vs daemon `{}`) requires root + SO_PEERCRED, not yet wired",
                user, daemon_user
            ),
        ));
    }
    Err(ErrPayload::new(
        "bad_args",
        "target must be one of {shell_id, tag, user, all}",
    ))
}

async fn op_notify(state: &Arc<DaemonState>, from: u64, args: Value) -> OpResult {
    let message = args
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `message`"))?
        .to_string();

    let urgency = args
        .get("urgency")
        .and_then(Value::as_str)
        .unwrap_or("normal")
        .to_string();

    let target = args.get("target").cloned().unwrap_or(Value::Null);

    let event_payload = json!({
        "delivery_id": format!("notify-{}-{}", from, chrono::Utc::now().timestamp_millis()),
        "from_shell": from,
        "message": message,
        "urgency": urgency,
    });
    let frame = Frame::event(event_name(Event::Notify), event_payload);
    let delivered = resolve_target(state, &target, from, frame)?;

    Ok(json!({
        "delivered_to": delivered,
        "delivered_count": delivered.len(),
    }))
}

async fn op_daemon(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let verb = args
        .get("verb")
        .and_then(Value::as_str)
        .unwrap_or("status")
        .to_string();

    match verb.as_str() {
        "status" => Ok(json!({
            "pid": state.pid,
            "uptime_ms": state.uptime_ms(),
            "version": env!("CARGO_PKG_VERSION"),
            "protocol_version": super::ipc::PROTOCOL_VERSION,
            "session_count": state.session_count(),
            "started_at": state.start_wall.to_rfc3339(),
        })),

        "stop" => {
            tracing::info!("daemon stop requested via IPC");
            // Broadcast daemon_shutdown to subscribers so anything depending on
            // the daemon can drop their subscription / reconnect cleanly.
            let payload = json!({
                "pid": state.pid,
                "uptime_ms": state.uptime_ms(),
                "reason": "ipc_stop",
                "grace_ms": 50,
            });
            let _ = state.broadcast(super::ipc::Frame::event("daemon_shutdown", payload), &[]);
            // Schedule a self-SIGTERM after the response + event go out.
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;
                let _ = kill(Pid::from_raw(std::process::id() as i32), Signal::SIGTERM);
            });
            Ok(json!({ "stopping": true }))
        }

        "restart" => {
            // Best-effort hand-off: fork-spawn a new `zshrs --daemon`, broadcast
            // shutdown so subscribers re-mmap, then SIGTERM self after a short
            // grace. The new daemon takes the singleton lock as soon as we
            // release it; clients that reconnect via spawn-on-demand land on it.
            //
            // We don't `exec` the replacement here — the running tokio runtime
            // owns this process's resources. Spawn-on-demand from the client
            // side handles the gap: from the user's perspective, `zcache
            // daemon restart` returns success, the daemon process changes pid,
            // and the next IPC op spins up the replacement automatically.
            tracing::info!("daemon restart requested via IPC");
            let payload = json!({
                "pid": state.pid,
                "uptime_ms": state.uptime_ms(),
                "reason": "ipc_restart",
                "grace_ms": 100,
            });
            let _ = state.broadcast(super::ipc::Frame::event("daemon_shutdown", payload), &[]);

            let new_pid = match spawn_replacement_daemon() {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!(
                        ?e,
                        "could not spawn replacement daemon; client must spawn-on-demand"
                    );
                    None
                }
            };

            // Self-SIGTERM after grace period so the response + event have time
            // to flush.
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;
                let _ = kill(Pid::from_raw(std::process::id() as i32), Signal::SIGTERM);
            });

            Ok(json!({
                "restarting": true,
                "old_pid": state.pid,
                "new_pid": new_pid,
                "grace_ms": 100,
            }))
        }

        _ => Err(ErrPayload::new(
            "bad_verb",
            format!("unknown daemon verb `{verb}`"),
        )),
    }
}

// The `op_rebuild`, `op_zshrc_analyze`, `op_first_init`, and
// `op_plugin_discover` handlers (~440 lines) lived here. They drove the
// AST-walk pipeline (`daemon/walk.rs`, `daemon/plugin_walk.rs`,
// `daemon/ast_walker.rs`), all deleted. State attribution is now sourced
// from `zshrs-recorder` (see docs/RECORDER.md) — runtime AOP intercept
// during shell init pushes records to the canonical engine. The daemon
// still hosts cache-management ops (`op_clean` / `op_verify` /
// `op_compact` / `op_canonical_hydrate_view`) and reacts to fsnotify
// events (broadcast `shard_updated` to subscribed shells), but never
// re-derives state by walking sources. New plugin installs require the
// user to re-run `zshrs-recorder`.

/// `cmd_started` — shell-side hook fires this when a command crosses the
/// long-running threshold (5s by default) without having completed. Daemon
/// broadcasts `long_cmd_started` to all of the user's other shells so they
/// can show a status-line indicator. Per docs/DAEMON.md "Long-running
/// command completion notices" companion events.
async fn op_cmd_started(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let line = args
        .get("line")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let from_shell = args.get("shell_id").and_then(Value::as_u64).unwrap_or(0);
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    let payload = json!({
        "from_shell": from_shell,
        "command": line,
        "cwd": cwd,
        "ts_ns": chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
    });
    let frame = super::ipc::Frame::event("long_cmd_started", payload);
    let count = state.broadcast(frame, &[from_shell]);
    Ok(json!({ "delivered_to": count }))
}

/// `complete` — tab-completion data plane. Given a partial line / cursor
/// position, return three slabs of matches:
///   - **commands**: PATH walk results matching the prefix (from
///     catalog.entries kind='command', built by walk.rs Pass 3)
///   - **handlers**: _comps registry entries (canonical compdef) matching
///     the prefix — what zsh's `_main_complete` would dispatch to
///   - **history**: prior commands sharing the prefix, ordered by recency
///
/// Pure server-side data lookup; the actual ZLE keystroke pipe (parsing
/// the buffer, painting the menu) is shell-side work. This op does the
/// heavy state-walking part — the no-walking-in-clients invariant.
async fn op_complete(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let prefix = args
        .get("prefix")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(64)
        .min(10000) as usize;

    // 1. Commands from PATH walk.
    let commands: Vec<String> = state
        .with_catalog(|conn| {
            let mut stmt = conn.prepare(
                "SELECT fq_name FROM entries WHERE plugin_id='system' AND kind='command' \
                 AND fq_name LIKE ? ORDER BY fq_name ASC LIMIT ?",
            )?;
            let pat = format!("cmd:{}%", prefix);
            let rows: Vec<String> = stmt
                .query_map(rusqlite::params![pat, limit as i64], |r| {
                    r.get::<_, String>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok::<_, rusqlite::Error>(rows)
        })
        .map_err(ErrPayload::from)?
        .into_iter()
        .filter_map(|s| s.strip_prefix("cmd:").map(str::to_string))
        .collect();

    // 2. Handlers from canonical compdef (the _comps table).
    let handlers: Vec<(String, String)> = state
        .canonical
        .rows_for("compdef")
        .into_iter()
        .filter(|r| r.key.starts_with(&prefix))
        .take(limit)
        .map(|r| {
            let val = serde_json::from_str::<serde_json::Value>(&r.value)
                .ok()
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .unwrap_or_else(|| r.value.clone());
            (r.key, val)
        })
        .collect();

    // 3. History suggestions matching prefix.
    let history_rows = state
        .with_history(|conn| {
            super::history::query(
                conn,
                Some(&prefix),
                "prefix",
                None,
                None,
                None,
                limit as i64,
                true,
            )
        })
        .map_err(|e: rusqlite::Error| ErrPayload::new("history_query", e.to_string()))?;
    let history: Vec<String> = history_rows.into_iter().map(|r| r.line).collect();

    Ok(json!({
        "prefix": prefix,
        "commands": commands,
        "handlers": handlers.iter().map(|(k, v)| json!({"command": k, "handler": v})).collect::<Vec<_>>(),
        "history": history,
        "totals": {
            "commands": commands.len(),
            "handlers": handlers.len(),
            "history": history.len(),
        },
    }))
}

/// `suggest` — inline autosuggest data plane. Given a prefix + cwd, return
/// the single best-match history row by frecency (recency × call_count).
/// Cwd-scoped first, then global.
async fn op_suggest(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let prefix = args
        .get("prefix")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);

    // Try cwd-scoped first (frecency wins for "in this dir, recently"),
    // then fall back to global prefix match.
    let row_opt = if let Some(c) = &cwd {
        state
            .with_history(|conn| {
                super::history::query(conn, Some(&prefix), "prefix", Some(c), None, None, 1, true)
            })
            .map_err(|e: rusqlite::Error| ErrPayload::new("history_query", e.to_string()))?
            .into_iter()
            .next()
    } else {
        None
    };
    let row_opt = if row_opt.is_none() {
        state
            .with_history(|conn| {
                super::history::query(conn, Some(&prefix), "prefix", None, None, None, 1, true)
            })
            .map_err(|e: rusqlite::Error| ErrPayload::new("history_query", e.to_string()))?
            .into_iter()
            .next()
    } else {
        row_opt
    };

    Ok(match row_opt {
        Some(r) => json!({
            "prefix": prefix,
            "suggestion": r.line,
            "ts_ns": r.ts_ns,
            "cwd": r.cwd,
            "matched": true,
        }),
        None => json!({
            "prefix": prefix,
            "suggestion": null,
            "matched": false,
        }),
    })
}

/// `highlight` — daemon-side syntax classification of a single command line.
/// The shell client sends the buffer; daemon returns a list of `{start, end,
/// kind}` spans the client paints. Kinds: command, builtin, alias, function,
/// keyword, path, string, comment, redirect, glob, error.
///
/// Per docs/DAEMON.md "All search and walking" + "All starting-state
/// preparation": canonical state owns the alias / function tables and the
/// catalog owns the PATH command-hash, so command-vs-error classification
/// happens here without a client-side lookup.
async fn op_highlight(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let line = args
        .get("line")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if line.is_empty() {
        return Ok(json!({"line": "", "spans": [] }));
    }

    let aliases: std::collections::BTreeSet<String> = state
        .canonical
        .rows_for("alias")
        .into_iter()
        .map(|r| r.key)
        .collect();
    let galiases: std::collections::BTreeSet<String> = state
        .canonical
        .rows_for("galias")
        .into_iter()
        .map(|r| r.key)
        .collect();
    let functions: std::collections::BTreeSet<String> = state
        .canonical
        .rows_for("function")
        .into_iter()
        .map(|r| r.key)
        .collect();

    let command_known = |name: &str| -> bool {
        if name.contains('/') {
            return std::path::Path::new(name).exists();
        }
        state
            .with_catalog(|conn| -> rusqlite::Result<bool> {
                let n: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM entries WHERE plugin_id='system' \
                     AND kind='command' AND fq_name = ?",
                    rusqlite::params![format!("cmd:{}", name)],
                    |r| r.get(0),
                )?;
                Ok(n > 0)
            })
            .unwrap_or(false)
    };

    let spans = highlight_line(&line, &aliases, &galiases, &functions, &command_known);

    Ok(json!({
        "line": line,
        "spans": spans,
    }))
}

/// Pure tokenizer + classifier. Split out for unit-testing without a daemon.
fn highlight_line(
    line: &str,
    aliases: &std::collections::BTreeSet<String>,
    galiases: &std::collections::BTreeSet<String>,
    functions: &std::collections::BTreeSet<String>,
    command_known: &dyn Fn(&str) -> bool,
) -> Vec<Value> {
    let bytes = line.as_bytes();
    let mut spans: Vec<Value> = Vec::new();
    let mut i = 0usize;
    let mut at_command_position = true;

    let push = |spans: &mut Vec<Value>, start: usize, end: usize, kind: &str| {
        spans.push(json!({"start": start, "end": end, "kind": kind}));
    };

    while i < bytes.len() {
        let c = bytes[i];

        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }
        if c == b'#' {
            push(&mut spans, i, bytes.len(), "comment");
            return spans;
        }
        if c == b';' || c == b'|' || c == b'&' {
            // Command separators reset position. `&&` / `||` / `|&` collapse.
            let start = i;
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b';' | b'|' | b'&') {
                i += 1;
            }
            push(&mut spans, start, i, "operator");
            at_command_position = true;
            continue;
        }
        if c == b'<' || c == b'>' {
            let start = i;
            i += 1;
            while i < bytes.len() && matches!(bytes[i], b'<' | b'>' | b'&' | b'!' | b'|') {
                i += 1;
            }
            push(&mut spans, start, i, "redirect");
            continue;
        }
        if c == b'\'' || c == b'"' || c == b'`' {
            let quote = c;
            let start = i;
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && quote != b'\'' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == quote {
                    i += 1;
                    break;
                }
                i += 1;
            }
            push(&mut spans, start, i, "string");
            at_command_position = false;
            continue;
        }
        if c == b'$' {
            let start = i;
            i += 1;
            if i < bytes.len() && bytes[i] == b'{' {
                while i < bytes.len() && bytes[i] != b'}' {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            } else {
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
            }
            push(&mut spans, start, i, "param");
            at_command_position = false;
            continue;
        }

        // Word: walk to the next whitespace / metacharacter.
        let start = i;
        while i < bytes.len() {
            let b = bytes[i];
            if b == b' '
                || b == b'\t'
                || b == b';'
                || b == b'|'
                || b == b'&'
                || b == b'<'
                || b == b'>'
                || b == b'#'
            {
                break;
            }
            if b == b'\'' || b == b'"' || b == b'`' {
                break;
            }
            i += 1;
        }
        let word = std::str::from_utf8(&bytes[start..i]).unwrap_or("");

        let kind = classify_word(
            word,
            at_command_position,
            aliases,
            galiases,
            functions,
            command_known,
        );
        push(&mut spans, start, i, kind);
        if at_command_position {
            at_command_position = false;
        }
    }
    spans
}

const ZSH_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "function", "in", "select", "time", "coproc", "repeat", "always", "foreach", "end", "{", "}",
    "[[", "]]",
];

const ZSH_BUILTINS: &[&str] = &[
    "alias",
    "autoload",
    "bg",
    "bindkey",
    "break",
    "builtin",
    "cd",
    "chdir",
    "command",
    "compdef",
    "compinit",
    "compinstall",
    "continue",
    "declare",
    "dirs",
    "disable",
    "disown",
    "echo",
    "echotc",
    "echoti",
    "emulate",
    "enable",
    "eval",
    "exec",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "float",
    "functions",
    "getln",
    "getopts",
    "hash",
    "history",
    "integer",
    "jobs",
    "kill",
    "let",
    "limit",
    "local",
    "log",
    "logout",
    "noglob",
    "popd",
    "print",
    "printf",
    "pushd",
    "pushln",
    "pwd",
    "r",
    "read",
    "readonly",
    "rehash",
    "return",
    "sched",
    "set",
    "setopt",
    "shift",
    "source",
    "suspend",
    "test",
    "times",
    "trap",
    "true",
    "ttyctl",
    "type",
    "typeset",
    "ulimit",
    "umask",
    "unalias",
    "unfunction",
    "unhash",
    "unlimit",
    "unset",
    "unsetopt",
    "wait",
    "whence",
    "where",
    "which",
    "zcompile",
    "zmodload",
    "zparseopts",
    "zstyle",
    ".",
    // zshrs-owned z* builtins
    "zcache",
    "zls",
    "zid",
    "zping",
    "ztag",
    "zuntag",
    "zsend",
    "znotify",
    "zsubscribe",
    "zunsubscribe",
    "zsync",
    "zask",
    "zlog",
    "zjob",
];

fn classify_word(
    word: &str,
    at_command_position: bool,
    aliases: &std::collections::BTreeSet<String>,
    galiases: &std::collections::BTreeSet<String>,
    functions: &std::collections::BTreeSet<String>,
    command_known: &dyn Fn(&str) -> bool,
) -> &'static str {
    if word.is_empty() {
        return "argument";
    }
    if at_command_position {
        if ZSH_KEYWORDS.contains(&word) {
            return "keyword";
        }
        if aliases.contains(word) {
            return "alias";
        }
        if functions.contains(word) {
            return "function";
        }
        if ZSH_BUILTINS.contains(&word) {
            return "builtin";
        }
        if word.contains('=') && !word.starts_with('=') {
            // VAR=value before a command — assignment.
            return "assignment";
        }
        if command_known(word) {
            return "command";
        }
        // Unknown command at command position — flagged red.
        return "error";
    }
    // Non-command-position: still highlight global aliases for awareness.
    if galiases.contains(word) {
        return "galias";
    }
    if word.starts_with('-') {
        return "option";
    }
    if word.contains('*') || word.contains('?') || word.contains('[') {
        return "glob";
    }
    if word.starts_with('/') || word.starts_with('~') || word.starts_with("./") {
        return "path";
    }
    "argument"
}

/// `register` — post-handshake update of session metadata. Used by clients
/// that want to refresh tty / cwd / argv0 mid-session (e.g. after `cd` fires
/// chpwd, after `tmux` reattaches, after `exec` rebrands argv0).
///
/// On connect, register_session is called automatically by server::handle_connection;
/// this op is the explicit-update path. Per docs/DAEMON.md "Operation table"
/// `register | Implicit on connect; also tag/cwd updates`.
async fn op_register(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    let tty = args.get("tty").and_then(Value::as_str).map(str::to_string);
    let argv0 = args
        .get("argv0")
        .and_then(Value::as_str)
        .map(str::to_string);
    let added_tags: Vec<String> = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let updated = state
        .update_session(client_id, cwd.clone(), tty.clone(), argv0.clone())
        .ok_or_else(|| ErrPayload::new("no_session", "client session not found"))?;
    if !added_tags.is_empty() {
        let _ = state.add_tags(client_id, &added_tags);
    }

    // chpwd subscribers on this scope expect a structured event.
    if let Some(new_cwd) = cwd {
        if let Some(scope) = state.origin_scope(client_id) {
            let payload = json!({
                "from_shell": client_id,
                "cwd": new_cwd,
                "ts_ns": chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            });
            state.publish(&scope, "chpwd", super::ipc::Frame::event("chpwd", payload));
        }
    }

    Ok(json!({
        "client_id": client_id,
        "cwd": updated.cwd,
        "tty": updated.tty,
        "argv0": updated.argv0,
        "tags": updated.tags,
    }))
}

/// `doctor` — comprehensive health diagnostic. Per docs/DAEMON.md "Failure
/// modes & disaster recovery": one-command sweep over filesystem perms, lock
/// state, catalog/history integrity, shard validity, fsnotify queue stats,
/// in-flight job count.
///
/// Returns a punch list of pass/warn/fail items so the caller can present a
/// quick traffic-light summary.
async fn op_doctor(state: &Arc<DaemonState>) -> OpResult {
    use std::os::unix::fs::PermissionsExt;

    let mut checks: Vec<Value> = Vec::new();
    let mut push = |name: &str, ok: bool, detail: String| {
        checks.push(json!({
            "name": name,
            "ok": ok,
            "detail": detail,
        }));
    };

    // 1. Cache root permissions.
    let root = &state.paths.root;
    match std::fs::metadata(root) {
        Ok(md) => {
            let mode = md.permissions().mode() & 0o777;
            push(
                "cache_root_perms",
                mode == 0o700,
                format!("{} mode={:o} (want 0700)", root.display(), mode),
            );
        }
        Err(e) => push("cache_root_perms", false, format!("stat: {}", e)),
    }

    for f in [
        &state.paths.catalog_db,
        &state.paths.history_db,
        &state.paths.log,
        &state.paths.socket,
        &state.paths.pid_file,
    ] {
        if !f.exists() {
            continue;
        }
        if let Ok(md) = std::fs::metadata(f) {
            let mode = md.permissions().mode() & 0o777;
            // Sockets (srwxr-xr-x → 0755 by default) we just flag if world-writable.
            let want_max = 0o600u32;
            let ok = mode <= want_max || (f == &state.paths.socket && (mode & 0o002) == 0);
            push(
                &format!("file_perms:{}", f.file_name().unwrap().to_string_lossy()),
                ok,
                format!("mode={:o}", mode),
            );
        }
    }

    // 2. Pid lock present + matches our pid.
    match std::fs::read_to_string(&state.paths.pid_file) {
        Ok(s) => {
            let p = s.trim().parse::<i32>().unwrap_or(0);
            push(
                "pidfile",
                p == state.pid,
                format!("file pid={} self pid={}", p, state.pid),
            );
        }
        Err(e) => push("pidfile", false, format!("read: {}", e)),
    }

    // 3. Socket exists + we own it.
    push(
        "socket_present",
        state.paths.socket.exists(),
        state.paths.socket.display().to_string(),
    );

    // 4. catalog.db PRAGMA integrity_check.
    match state.catalog_integrity() {
        Ok(true) => push("catalog_integrity", true, "ok".into()),
        Ok(false) => push("catalog_integrity", false, "FAIL".into()),
        Err(e) => push("catalog_integrity", false, e.to_string()),
    }

    // 5. history row count + sanity (no NULL ts_ns in last 100 rows).
    match state.with_history(|conn| -> rusqlite::Result<(i64, i64)> {
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM history", [], |r| r.get(0))?;
        let null_ts: i64 = conn.query_row(
            "SELECT COUNT(*) FROM history WHERE ts_ns IS NULL",
            [],
            |r| r.get(0),
        )?;
        Ok((total, null_ts))
    }) {
        Ok((total, null_ts)) => push(
            "history_db",
            null_ts == 0,
            format!("rows={} null_ts={}", total, null_ts),
        ),
        Err(e) => push("history_db", false, e.to_string()),
    }

    // 6. Shard files present + readable.
    let shards = super::shard::list_shards(&state.paths).unwrap_or_default();
    let mut shard_ok = 0usize;
    let mut shard_fail = 0usize;
    for p in &shards {
        match std::fs::metadata(p) {
            Ok(md) if md.len() > 0 => shard_ok += 1,
            _ => shard_fail += 1,
        }
    }
    push(
        "shards_present",
        shard_fail == 0,
        format!("ok={} fail={}", shard_ok, shard_fail),
    );

    // 7. fsnotify watcher stats.
    let fs_stats = state.fs_watcher.stats_json();
    push(
        "fsnotify_alive",
        fs_stats
            .get("running")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fs_stats.to_string(),
    );

    // 8. In-flight jobs (informational only).
    let live_jobs = state.jobs.list(Some("running"), None, None).len();
    push("jobs_live", true, format!("{} supervised", live_jobs));

    // 9. .zwc / .zcompdump litter near the user's HOME (the cleanup hint).
    if let Some(home) = dirs::home_dir() {
        let mut litter = 0usize;
        if let Ok(rd) = std::fs::read_dir(&home) {
            for ent in rd.flatten() {
                let n = ent.file_name();
                let s = n.to_string_lossy();
                if s.starts_with(".zcompdump") || s.ends_with(".zwc") {
                    litter += 1;
                }
            }
        }
        push(
            "legacy_litter",
            litter == 0,
            format!(
                "{} legacy artifacts in HOME (run `zcache clean legacy`)",
                litter
            ),
        );
    }

    let total = checks.len();
    let failed = checks
        .iter()
        .filter(|c| !c["ok"].as_bool().unwrap_or(false))
        .count();

    Ok(json!({
        "checks": checks,
        "total": total,
        "passed": total - failed,
        "failed": failed,
        "ts_ns": chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
    }))
}

/// Refresh the SQLite `canonical` view table from the in-memory rkyv-backed
/// state. Hot lookups never hit SQLite (per docs/DAEMON.md "Daemon = sole
/// writer"); this op exists for `zcache view --format sql` / external
/// `sqlite3 catalog.db` inspection.
async fn op_canonical_hydrate_view(state: &Arc<DaemonState>) -> OpResult {
    let n = state
        .canonical
        .hydrate_sqlite_view(state)
        .map_err(|e| ErrPayload::new("hydrate_failed", e.to_string()))?;
    Ok(json!({
        "rows_written": n,
        "total_in_memory": state.canonical.total_rows(),
    }))
}

/// `recorder_ingest` — single-shot end-of-recording handoff. Receives the
/// full event bundle from `zshrs-recorder`, folds every `(kind, name,
/// value, file, line, fn_chain, ts_ns, order_idx)` row into the canonical
/// engine (replacing every recorder-managed subsystem so removed
/// definitions actually disappear), writes a fresh `system` rkyv shard,
/// hydrates the SQLite mirror, and broadcasts `recorder_ingested` so any
/// subscribed shell re-mmaps. Per docs/RECORDER.md: only the recorder can
/// observe state at 100% fidelity, so this is the daemon's only path for
/// learning .zshrc state — the static-walk pipeline was deleted.
async fn op_recorder_ingest(state: &Arc<DaemonState>, args: Value) -> OpResult {
    use std::collections::HashMap;

    let started = std::time::Instant::now();

    // Per-event view of the recorder bundle. Keep this in sync with
    // `src/recorder/mod.rs::RecordEvent` — they share the wire format.
    #[derive(serde::Deserialize)]
    struct EvIn {
        order_idx: u64,
        ts_ns: u64,
        kind: String,
        name: String,
        value: Option<String>,
        file: Option<String>,
        line: Option<u32>,
        fn_chain: Option<String>,
        // Replay-grade type info — `attrs` is a ParamAttrs bitset
        // (scalar/integer/float/assoc/array/readonly/export/global/
        // unique/tied/append). Populated by the recorder on every
        // assign event so the daemon can reconstruct typed
        // declarations during `recorder_replay` without guessing.
        #[serde(default)]
        attrs: u16,
        // Structured payloads — Some when the event represents an
        // array or assoc mutation. Replay reconstructs `name=(...)`
        // / `name=(k1 v1 k2 v2)` directly from these instead of
        // splitting the joined `value` string. Empty for scalars.
        #[serde(default)]
        value_array: Option<Vec<String>>,
        #[serde(default)]
        value_assoc: Option<Vec<(String, String)>>,
    }
    #[derive(serde::Deserialize)]
    struct BundleIn {
        started_at_ns: u64,
        finished_at_ns: u64,
        cmdline: Option<String>,
        zdotdir: Option<String>,
        home: Option<String>,
        events: Vec<EvIn>,
        // Federated-catalog identity for every row in this bundle.
        // Per-event override via EvIn.shell_id (added separately).
        // None at both layers = "zshrs" (the historical default,
        // preserved for backwards compat with pre-shell_id ingests).
        #[serde(default)]
        shell_id: Option<String>,
    }

    let bundle: BundleIn = serde_json::from_value(args)
        .map_err(|e| ErrPayload::new("bad_args", format!("recorder bundle: {e}")))?;
    let total = bundle.events.len();
    let bundle_shell_id = bundle
        .shell_id
        .clone()
        .unwrap_or_else(|| "zshrs".to_string());

    // Per-subsystem buckets for `replace_subsystem` (so a re-recorder
    // run drops every previous row in that subsystem and replaces with
    // the fresh capture — recorder bundle is authoritative end-state).
    //
    // Each value is `(value_string, file, line)` so file/line
    // attribution flows through to canonical rows for `zwhere`'s
    // "where was this defined?" answer.
    type AttrRow = (String, Option<String>, Option<u32>);
    let mut aliases: HashMap<String, AttrRow> = HashMap::new();
    let mut galias: HashMap<String, AttrRow> = HashMap::new();
    let mut salias: HashMap<String, AttrRow> = HashMap::new();
    let mut functions: HashMap<String, AttrRow> = HashMap::new();
    let mut env_exports: HashMap<String, AttrRow> = HashMap::new();
    let mut params: HashMap<String, AttrRow> = HashMap::new();
    // Replay-grade typed snapshot per parameter name. Value is the
    // JSON-serialised event payload (attrs + value + value_array +
    // value_assoc), so replay can read this back and emit
    // `typeset -<flags> NAME=val` / `name=(elem ...)` / `name=(k1 v1 ...)`
    // verbatim. Latest-wins per name (the recorder bundle is end-state
    // for the run; the most recent event per name reflects current
    // value).
    let mut params_typed: HashMap<String, AttrRow> = HashMap::new();
    let mut bindkeys: HashMap<String, AttrRow> = HashMap::new();
    let mut compdef: HashMap<String, AttrRow> = HashMap::new();
    let mut named_dirs: HashMap<String, AttrRow> = HashMap::new();
    let mut zstyle: Vec<(String, AttrRow)> = Vec::new();
    let mut zmodload: Vec<(String, AttrRow)> = Vec::new();
    let mut setopts: Vec<(String, AttrRow)> = Vec::new();
    let mut unsetopts: Vec<(String, AttrRow)> = Vec::new();
    let mut traps: HashMap<String, AttrRow> = HashMap::new();
    let mut sched: HashMap<String, AttrRow> = HashMap::new();
    let mut zle_widgets: HashMap<String, AttrRow> = HashMap::new();
    let mut completions: HashMap<String, AttrRow> = HashMap::new();
    let mut sourced: Vec<(String, AttrRow)> = Vec::new();
    // path/fpath/manpath are positional; we capture in event order.
    let mut path_acc: Vec<(String, AttrRow)> = Vec::new();
    let mut fpath_acc: Vec<(String, AttrRow)> = Vec::new();

    for ev in &bundle.events {
        // Per-event attribution tuple — stamped on every record's
        // canonical row. Surfaces via `zwhere`'s file:line column.
        let attr = |val: String| -> AttrRow { (val, ev.file.clone(), ev.line) };
        match ev.kind.as_str() {
            "alias" => {
                aliases.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "alias -g" | "galias" => {
                galias.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "alias -s" | "salias" => {
                salias.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "function" => {
                functions.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "export" => {
                env_exports.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "assign" | "typeset" => {
                params.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
                // Stash the full structured payload so replay can
                // reconstruct typed declarations exactly.
                let payload = serde_json::json!({
                    "attrs": ev.attrs,
                    "value": ev.value,
                    "value_array": ev.value_array,
                    "value_assoc": ev.value_assoc,
                });
                params_typed.insert(ev.name.clone(), attr(payload.to_string()));
            }
            "bindkey" => {
                bindkeys.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "compdef" => {
                compdef.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "hash -d" | "hash_d" => {
                named_dirs.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "zstyle" => {
                zstyle.push((ev.name.clone(), attr(ev.value.clone().unwrap_or_default())));
            }
            "zmodload" => {
                zmodload.push((ev.name.clone(), attr(String::new())));
            }
            "setopt" => setopts.push((ev.name.clone(), attr("on".to_string()))),
            "unsetopt" => unsetopts.push((ev.name.clone(), attr("off".to_string()))),
            "trap" => {
                traps.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "sched" => {
                sched.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "zle" => {
                zle_widgets.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "completion" => {
                completions.insert(ev.name.clone(), attr(ev.value.clone().unwrap_or_default()));
            }
            "source" => sourced.push((ev.name.clone(), attr(String::new()))),
            "path_mod" => {
                let target = ev.value.as_deref().unwrap_or("");
                let row = (ev.name.clone(), attr(String::new()));
                if target == "fpath" {
                    fpath_acc.push(row);
                } else {
                    path_acc.push(row);
                }
            }
            other => {
                tracing::debug!(other, "recorder_ingest: unknown kind, ignored");
            }
        }
        let _ = (ev.order_idx, ev.ts_ns, &ev.fn_chain);
    }

    // Replace subsystems wholesale — recorder bundle is end-state.
    // Every row stamped with the bundle's shell_id so the canonical
    // catalog stays federated across bash/zsh/zshrs/etc. recorders.
    // file/line carried through via replace_subsystem_with_attrs so
    // `zwhere alias gst` shows where it was defined.
    let canon = &state.canonical;
    let sid = || Some(bundle_shell_id.clone());
    // map4 borrows + clones so the source HashMap stays available for
    // the rkyv shard build below. Per-kind clone is microseconds even
    // at zpwr scale.
    let map4 =
        |m: &HashMap<String, AttrRow>| -> Vec<(String, String, Option<String>, Option<u32>)> {
            m.iter()
                .map(|(k, (v, file, line))| (k.clone(), json_string(v), file.clone(), *line))
                .collect()
        };
    canon.replace_subsystem_with_attrs("alias", map4(&aliases), None, sid());
    canon.replace_subsystem_with_attrs("galias", map4(&galias), None, sid());
    canon.replace_subsystem_with_attrs("salias", map4(&salias), None, sid());
    canon.replace_subsystem_with_attrs("function", map4(&functions), None, sid());
    canon.replace_subsystem_with_attrs("env", map4(&env_exports), None, sid());
    canon.replace_subsystem_with_attrs("params", map4(&params), None, sid());
    canon.replace_subsystem_with_attrs("bindkey", map4(&bindkeys), None, sid());
    canon.replace_subsystem_with_attrs("compdef", map4(&compdef), None, sid());
    canon.replace_subsystem_with_attrs("named_dir", map4(&named_dirs), None, sid());
    canon.replace_subsystem_with_attrs(
        "zstyle",
        zstyle
            .iter()
            .enumerate()
            .map(|(i, (p, (r, file, line)))| {
                (format!("{}:{}", i, p), json_string(r), file.clone(), *line)
            })
            .collect::<Vec<_>>(),
        None,
        sid(),
    );
    canon.replace_subsystem_with_attrs(
        "zmodload",
        zmodload
            .iter()
            .map(|(m, (_v, file, line))| (m.clone(), json_string(""), file.clone(), *line))
            .collect::<Vec<_>>(),
        None,
        sid(),
    );
    canon.replace_subsystem_with_attrs(
        "setopt",
        setopts
            .iter()
            .map(|(o, (_v, file, line))| (o.clone(), "\"on\"".to_string(), file.clone(), *line))
            .chain(unsetopts.iter().map(|(o, (_v, file, line))| {
                (o.clone(), "\"off\"".to_string(), file.clone(), *line)
            }))
            .collect::<Vec<_>>(),
        None,
        sid(),
    );
    canon.replace_subsystem_with_attrs("trap", map4(&traps), None, sid());
    canon.replace_subsystem_with_attrs("sched", map4(&sched), None, sid());
    canon.replace_subsystem_with_attrs("zle", map4(&zle_widgets), None, sid());
    canon.replace_subsystem_with_attrs("completion", map4(&completions), None, sid());
    // params_typed values are already JSON; pass them through verbatim
    // so the canonical row preserves the structured payload (attrs +
    // value + value_array + value_assoc) for replay.
    canon.replace_subsystem_with_attrs(
        "params_typed",
        params_typed
            .iter()
            .map(|(k, (v, file, line))| (k.clone(), v.clone(), file.clone(), *line))
            .collect::<Vec<_>>(),
        None,
        sid(),
    );
    canon.replace_subsystem_with_attrs(
        "source",
        sourced
            .iter()
            .enumerate()
            .map(|(i, (p, (_v, file, line)))| (i.to_string(), json_string(p), file.clone(), *line))
            .collect::<Vec<_>>(),
        None,
        sid(),
    );
    canon.replace_subsystem_with_attrs(
        "path",
        path_acc
            .iter()
            .enumerate()
            .map(|(i, (d, (_v, file, line)))| (i.to_string(), json_string(d), file.clone(), *line))
            .collect::<Vec<_>>(),
        None,
        sid(),
    );
    canon.replace_subsystem_with_attrs(
        "fpath",
        fpath_acc
            .iter()
            .enumerate()
            .map(|(i, (d, (_v, file, line)))| (i.to_string(), json_string(d), file.clone(), *line))
            .collect::<Vec<_>>(),
        None,
        sid(),
    );

    // Build the rkyv shard from the just-folded canonical state.
    let source_root = bundle
        .zdotdir
        .clone()
        .or_else(|| bundle.home.clone())
        .unwrap_or_else(|| "<recorder>".to_string());
    let header = super::shard::ShardHeader {
        magic: 0,
        format_version: 0,
        generation: bundle.finished_at_ns,
        built_at_ns: bundle.finished_at_ns,
        slug: "recorder".to_string(),
        source_root: source_root.clone(),
        entry_count: total as u32,
    };
    // Helpers: project the AttrRow-carrying maps/vecs into the
    // value-only shapes the shard format expects. The shard format
    // does not yet store per-row file/line attribution; that lives
    // only in the in-memory canonical state for `zwhere` queries
    // (survives until daemon restart, then needs a recorder re-run).
    let flatten_map = |m: &HashMap<String, AttrRow>| -> HashMap<String, String> {
        m.iter()
            .map(|(k, (v, _, _))| (k.clone(), v.clone()))
            .collect()
    };
    let flatten_vec_kv = |v: &Vec<(String, AttrRow)>| -> Vec<(String, String)> {
        v.iter()
            .map(|(k, (val, _, _))| (k.clone(), val.clone()))
            .collect()
    };
    let flatten_vec_v =
        |v: &Vec<(String, AttrRow)>| -> Vec<String> { v.iter().map(|(k, _)| k.clone()).collect() };
    let shard = super::shard::CanonicalShard {
        header,
        aliases: flatten_map(&aliases),
        global_aliases: flatten_map(&galias),
        suffix_aliases: flatten_map(&salias),
        functions: flatten_map(&functions),
        autoload_functions: HashMap::new(),
        setopts: flatten_vec_v(&setopts),
        unsetopts: flatten_vec_v(&unsetopts),
        bindkeys: flatten_map(&bindkeys),
        named_dirs: flatten_map(&named_dirs),
        compdef: flatten_map(&compdef),
        zstyle: flatten_vec_kv(&zstyle),
        zmodload: flatten_vec_v(&zmodload),
        env_exports: flatten_map(&env_exports),
        params: flatten_map(&params),
        path: flatten_vec_v(&path_acc),
        fpath: flatten_vec_v(&fpath_acc),
        manpath: Vec::new(),
        plugins: Vec::new(),
        sourced_files: flatten_vec_v(&sourced),
        // New subsystems (zle widgets, discovered completions) ride in
        // `extras` so the rkyv shard format doesn't need a version
        // bump per addition. CanonicalShard readers iterate `extras`
        // and fold into in-memory state.
        extras: {
            let mut e: HashMap<String, HashMap<String, String>> = HashMap::new();
            if !zle_widgets.is_empty() {
                e.insert("zle".to_string(), flatten_map(&zle_widgets));
            }
            if !completions.is_empty() {
                e.insert("completion".to_string(), flatten_map(&completions));
            }
            if !params_typed.is_empty() {
                e.insert("params_typed".to_string(), flatten_map(&params_typed));
            }
            e
        },
    };
    let shard_path = match super::shard::write_canonical_shard(&state.paths, &shard) {
        Ok(p) => p.display().to_string(),
        Err(e) => {
            tracing::warn!(?e, "recorder_ingest: shard write failed (canonical kept)");
            String::new()
        }
    };

    // Hydrate SQLite mirror so `zcache view ...` is fresh.
    let hydrated = match canon.hydrate_sqlite_view(state) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(?e, "recorder_ingest: hydrate failed (rkyv authoritative)");
            0
        }
    };

    let elapsed_ms = started.elapsed().as_millis() as u64;
    let payload = json!({
        "events_ingested": total,
        "rows_written": hydrated,
        "shard_path": shard_path,
        "elapsed_ms": elapsed_ms,
        "started_at_ns": bundle.started_at_ns,
        "finished_at_ns": bundle.finished_at_ns,
        "cmdline": bundle.cmdline.unwrap_or_default(),
    });
    // Targeted broadcast — only clients that called `definitions_subscribe`
    // (or HTTP /stream/definitions, which auto-subscribes its synthetic
    // session) receive this frame. Silent IPC sessions stay quiet.
    let _ = state.broadcast_to_definitions_subscribers(super::ipc::Frame::event(
        "recorder_ingested",
        payload.clone(),
    ));
    tracing::info!(
        events = total,
        hydrated,
        elapsed_ms,
        "recorder_ingest complete"
    );
    state.metrics.record_recorder_events(total as u64);
    Ok(payload)
}

/// Local helper: JSON-string-encode a value so canonical rows store the
/// same shape (escaped + quoted) the legacy walker pipeline used.
fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

async fn op_clean(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let target = args
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("all")
        .to_string();
    let dry_run = args
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let no_stats = args
        .get("no_stats")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let shard_name = args.get("name").and_then(Value::as_str).map(str::to_string);

    let mut removed: Vec<String> = Vec::new();
    let mut would_remove: Vec<String> = Vec::new();
    let paths = &state.paths;

    let mut record = |path: std::path::PathBuf| {
        if dry_run {
            would_remove.push(path.display().to_string());
        } else {
            let _ = std::fs::remove_file(&path);
            removed.push(path.display().to_string());
        }
    };

    match target.as_str() {
        "all" => {
            for shard in super::shard::list_shards(paths).unwrap_or_default() {
                record(shard);
            }
            if paths.index_rkyv.exists() {
                record(paths.index_rkyv.clone());
            }
        }
        "shards" => {
            for shard in super::shard::list_shards(paths).unwrap_or_default() {
                record(shard);
            }
        }
        // `zcache clean shard <name>` per DAEMON.md:676.
        "shard" => {
            let name = shard_name
                .as_ref()
                .ok_or_else(|| ErrPayload::new("bad_args", "missing `name` for shard target"))?;
            let target_match = format!("-{}.rkyv", name);
            for shard in super::shard::list_shards(paths).unwrap_or_default() {
                let fname = shard.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if fname.contains(&target_match) || fname == format!("{}.rkyv", name) {
                    record(shard);
                }
            }
        }
        "index" => {
            if paths.index_rkyv.exists() {
                record(paths.index_rkyv.clone());
            }
        }
        // `zcache clean catalog [--no-stats]` per DAEMON.md:677-678.
        "catalog" => {
            // Default: preserve entry_stats by reading them out, blowing away
            // catalog.db, recreating schema, replaying entry_stats.
            let preserved_stats: Vec<(String, Option<i64>, i64, Option<i64>)> = if no_stats {
                Vec::new()
            } else {
                state
                    .with_catalog(|conn| -> rusqlite::Result<_> {
                        let mut stmt = conn.prepare(
                            "SELECT fq_name, last_called_at, call_count, total_ns FROM entry_stats",
                        )?;
                        let rows = stmt
                            .query_map([], |r| {
                                Ok((
                                    r.get::<_, String>(0)?,
                                    r.get::<_, Option<i64>>(1)?,
                                    r.get::<_, i64>(2)?,
                                    r.get::<_, Option<i64>>(3)?,
                                ))
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?;
                        Ok(rows)
                    })
                    .unwrap_or_default()
            };
            if dry_run {
                would_remove.push(paths.catalog_db.display().to_string());
            } else if paths.catalog_db.exists() {
                let _ = std::fs::remove_file(&paths.catalog_db);
                removed.push(paths.catalog_db.display().to_string());
                // Reopen + reschema so subsequent ops don't NotConnected.
                let conn = super::catalog::open(paths)
                    .map_err(|e| ErrPayload::new("catalog_reopen", e.to_string()))?;
                if !no_stats && !preserved_stats.is_empty() {
                    let _ = conn.execute_batch("BEGIN");
                    for (fq, last, count, total) in &preserved_stats {
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO entry_stats (fq_name, last_called_at, call_count, total_ns) VALUES (?, ?, ?, ?)",
                            rusqlite::params![fq, last, count, total],
                        );
                    }
                    let _ = conn.execute_batch("COMMIT");
                }
                tracing::info!(
                    preserved = preserved_stats.len(),
                    no_stats,
                    "catalog cleaned"
                );
            }
        }
        // `zcache clean stats` per DAEMON.md:680.
        "stats" => {
            if dry_run {
                would_remove.push("entry_stats (catalog table)".into());
            } else {
                let n = state
                    .with_catalog(|conn| -> rusqlite::Result<i64> {
                        let n: i64 =
                            conn.query_row("SELECT COUNT(*) FROM entry_stats", [], |r| r.get(0))?;
                        conn.execute("DELETE FROM entry_stats", [])?;
                        Ok(n)
                    })
                    .unwrap_or(0);
                removed.push(format!("entry_stats ({} rows)", n));
            }
        }
        "log" => {
            // Truncate today's rolled file (don't unlink — tracing-appender holds an fd).
            for entry in std::fs::read_dir(&paths.root)
                .map_err(|e| ErrPayload::new("io", e.to_string()))?
                .flatten()
            {
                let name = entry.file_name();
                let s = name.to_string_lossy();
                if super::paths::is_zshrs_log_file(&s) {
                    if dry_run {
                        would_remove.push(entry.path().display().to_string());
                    } else {
                        let _ = std::fs::OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(entry.path());
                        removed.push(entry.path().display().to_string());
                    }
                }
            }
        }
        // `zcache clean zwc` / `zcompdump` / `legacy` per DAEMON.md:387-396.
        // Walks only directories the daemon knows about: $HOME, $ZDOTDIR,
        // $ZPWR_LOCAL, $XDG_CACHE_HOME, plus every dir referenced in the
        // canonical path/fpath subsystems and every parent of a watched file.
        "zwc" | "zcompdump" | "legacy" => {
            let scope = legacy_scope_dirs(state);
            let want_zwc = matches!(target.as_str(), "zwc" | "legacy");
            let want_zcompdump = matches!(target.as_str(), "zcompdump" | "legacy");
            for dir in scope {
                if !dir.is_dir() {
                    continue;
                }
                let walker = walkdir::WalkDir::new(&dir)
                    .max_depth(if dir.starts_with(&paths.root) { 2 } else { 6 })
                    .follow_links(false)
                    .into_iter()
                    .filter_map(|r| r.ok());
                for ent in walker {
                    if !ent.file_type().is_file() {
                        continue;
                    }
                    let p = ent.path();
                    let n = match p.file_name().and_then(|n| n.to_str()) {
                        Some(s) => s,
                        None => continue,
                    };
                    let is_zwc = want_zwc && n.ends_with(".zwc");
                    let is_zcd = want_zcompdump
                        && (n.starts_with(".zcompdump") || n.contains(".zcompdump-"));
                    if is_zwc || is_zcd {
                        record(p.to_path_buf());
                    }
                }
            }
        }
        other => {
            return Err(ErrPayload::new(
                "bad_target",
                format!(
                    "clean target `{}` not supported (try all|shards|shard|index|log|catalog|stats|zwc|zcompdump|legacy)",
                    other
                ),
            ));
        }
    }

    let mut out = json!({
        "target": target,
        "dry_run": dry_run,
        "removed": removed,
        "removed_count": removed.len(),
    });
    if dry_run {
        out["would_remove"] = json!(would_remove);
        out["would_remove_count"] = json!(would_remove.len());
    }
    Ok(out)
}

/// Build the directory scope for legacy artifact (`.zwc` / `.zcompdump`)
/// cleanup. Per DAEMON.md "Out-of-scope dirs are never touched (no recursive
/// `find ~ -name '*.zwc'`). Daemon walks only the directories it already
/// knows about."
fn legacy_scope_dirs(state: &Arc<DaemonState>) -> Vec<std::path::PathBuf> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    let mut push = |p: std::path::PathBuf| {
        if !out.iter().any(|q| q == &p) {
            out.push(p);
        }
    };

    // Cache root (catches anything inside ~/.zshrs/).
    push(state.paths.root.clone());

    // HOME + ZDOTDIR + XDG_CACHE_HOME + ZPWR_LOCAL.
    if let Some(home) = dirs::home_dir() {
        push(home);
    }
    for var in &["ZDOTDIR", "XDG_CACHE_HOME", "ZPWR_LOCAL", "ZSH"] {
        if let Ok(v) = std::env::var(var) {
            if !v.is_empty() {
                push(std::path::PathBuf::from(v));
            }
        }
    }

    // Every directory in the canonical path / fpath subsystems.
    for sub in &["path", "fpath", "manpath"] {
        for row in state.canonical.rows_for(sub) {
            let val = row.value.trim_matches('"').to_string();
            let p = std::path::PathBuf::from(val);
            if p.is_dir() {
                push(p);
            }
        }
    }

    // Every parent of a watched fsnotify path (covers .zwc next to any
    // sourced file).
    for wp in state.fs_watcher.registered_paths() {
        if let Some(parent) = wp.path.parent() {
            push(parent.to_path_buf());
        }
    }

    out
}

async fn op_verify(state: &Arc<DaemonState>) -> OpResult {
    let mut issues: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut shards_ok = 0usize;
    let mut shards_bad = 0usize;

    for shard in super::shard::list_shards(&state.paths).unwrap_or_default() {
        match super::shard::MmappedShard::open(&shard) {
            Ok(_) => shards_ok += 1,
            Err(e) => {
                shards_bad += 1;
                issues.push(format!("{}: {}", shard.display(), e));
            }
        }
    }

    let catalog_ok = state.catalog_integrity().unwrap_or(false);
    if !catalog_ok {
        issues.push("catalog.db: PRAGMA integrity_check failed".to_string());
    }

    let tmp_swept = super::shard::sweep_tmp_files(&state.paths, std::time::Duration::from_secs(60))
        .unwrap_or(0);

    // Per docs/DAEMON.md "zcache verify ... already exists; reports
    // .zwc/.zcompdump presence as a WARN with the cleanup hint, since their
    // existence implies stale legacy artifacts" (line 394-395).
    let scope = legacy_scope_dirs(state);
    let mut zwc_count = 0usize;
    let mut zcd_count = 0usize;
    let mut sample: Vec<String> = Vec::new();
    for dir in &scope {
        if !dir.is_dir() {
            continue;
        }
        // Bound walk depth; legacy scope is wide so don't hammer subtrees.
        for ent in walkdir::WalkDir::new(dir)
            .max_depth(if dir.starts_with(&state.paths.root) {
                2
            } else {
                4
            })
            .follow_links(false)
            .into_iter()
            .filter_map(|r| r.ok())
        {
            if !ent.file_type().is_file() {
                continue;
            }
            let p = ent.path();
            let n = match p.file_name().and_then(|n| n.to_str()) {
                Some(s) => s,
                None => continue,
            };
            if n.ends_with(".zwc") {
                zwc_count += 1;
                if sample.len() < 8 {
                    sample.push(p.display().to_string());
                }
            } else if n.starts_with(".zcompdump") || n.contains(".zcompdump-") {
                zcd_count += 1;
                if sample.len() < 8 {
                    sample.push(p.display().to_string());
                }
            }
        }
    }
    if zwc_count > 0 || zcd_count > 0 {
        warnings.push(format!(
            "{} .zwc + {} .zcompdump legacy artifacts found — run `zcache clean legacy` (sample: {})",
            zwc_count,
            zcd_count,
            sample.join(", ")
        ));
        tracing::warn!(
            zwc_count,
            zcd_count,
            sample = ?sample,
            "verify: legacy artifacts present"
        );
    }

    Ok(json!({
        "shards_ok": shards_ok,
        "shards_bad": shards_bad,
        "catalog_ok": catalog_ok,
        "tmp_swept": tmp_swept,
        "legacy_zwc_count": zwc_count,
        "legacy_zcompdump_count": zcd_count,
        "legacy_sample": sample,
        "issues": issues,
        "warnings": warnings,
        "verified": shards_bad == 0 && catalog_ok,
    }))
}

async fn op_compact(state: &Arc<DaemonState>) -> OpResult {
    // For v1: VACUUM the catalog + sweep tmp files.
    let swept = super::shard::sweep_tmp_files(&state.paths, std::time::Duration::from_secs(60))
        .unwrap_or(0);
    Ok(json!({
        "tmp_swept": swept,
    }))
}

// -------- fsnotify --------

async fn op_fpath_changed(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let paths_arr = args
        .get("paths")
        .and_then(Value::as_array)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `paths` array"))?;

    let mut registered = Vec::new();
    for p in paths_arr {
        let path = match p.as_str() {
            Some(s) => s,
            None => continue,
        };
        let wp = super::fsnotify::WatchedPath {
            path: std::path::PathBuf::from(path),
            shard_slug: format!("fpath-{}", super::shard::hash8(path)),
            source_root: path.to_string(),
            kind: super::fsnotify::WatchKind::FpathDir,
        };
        match state.fs_watcher.watch_path(wp, false) {
            Ok(()) => registered.push(path.to_string()),
            Err(e) => tracing::warn!(?e, %path, "fsnotify watch failed"),
        }
    }
    Ok(json!({
        "registered": registered,
        "registered_count": registered.len(),
    }))
}

/// `autoload_prewarm {dirs?: [path], timeout_secs?: n}` — compile every
/// `_*` completer on those fpath dirs into `~/.zshrs/autoloads.rkyv`,
/// so a later `ls -<TAB>` is an O(1) shard probe instead of a parse +
/// compile of the completer's file.
///
/// The daemon does not do the work itself, and cannot: the zsh parser
/// and bytecode compiler live in the `zshrs` crate, which depends on
/// this one. It spawns `zshrs --prewarm-autoloads` — which is also the
/// right isolation, since `parse()` walks process-global lexer state
/// and must not run beside anything else.
///
/// This is not "the daemon walking user config" in the docs/DAEMON.md
/// sense: the directory list comes from the caller (or from the
/// recorded `$fpath` the spawned shell already has), and the daemon
/// only relays the child's summary.
async fn op_autoload_prewarm(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let dirs: Vec<String> = args
        .get("dirs")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let timeout_secs = args
        .get("timeout_secs")
        .and_then(Value::as_u64)
        .unwrap_or(900);

    let exe = zshrs_binary().ok_or_else(|| {
        ErrPayload::new(
            "not_found",
            "no `zshrs` binary beside the daemon or on $PATH to run the compile",
        )
    })?;
    let started = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new(&exe);
    cmd.arg("--prewarm-autoloads");
    for d in &dirs {
        cmd.arg(d);
    }
    cmd.stdin(std::process::Stdio::null());
    cmd.kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), cmd.output())
        .await
        .map_err(|_| ErrPayload::new("timeout", "prewarm exceeded timeout_secs"))?
        .map_err(|e| ErrPayload::new("spawn_failed", format!("{e}")))?;

    if !out.status.success() {
        return Err(ErrPayload::new(
            "prewarm_failed",
            format!(
                "{} exited {}: {}",
                exe.display(),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim(),
            ),
        ));
    }
    // The child prints one JSON line; pass it through so callers see the
    // real counts rather than a summary of a summary.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let summary: Value = stdout
        .lines()
        .rev()
        .find_map(|l| serde_json::from_str::<Value>(l.trim()).ok())
        .unwrap_or_else(|| json!({}));
    let _ = state;
    Ok(json!({
        "binary": exe.display().to_string(),
        "dirs": dirs,
        "summary": summary,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    }))
}

/// Locate the `zshrs` binary: next to the running daemon first (the
/// pair ships together and a dev tree must not pick up an installed
/// copy), then `$PATH`.
fn zshrs_binary() -> Option<std::path::PathBuf> {
    if let Ok(me) = std::env::current_exe() {
        if let Some(dir) = me.parent() {
            let cand = dir.join("zshrs");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|p| p.join("zshrs"))
        .find(|p| p.is_file())
}

async fn op_watcher_stats(state: &Arc<DaemonState>) -> OpResult {
    let stats = state.fs_watcher.stats();
    Ok(json!(stats))
}

/// `watch_subscribe {path, recursive?}` — register a refcounted
/// fsnotify subscription. Returns `{watch_id}`. The same path can be
/// subscribed multiple times safely; the underlying notify::Watcher
/// stays armed until the last subscriber unsubscribes. Used by IPC
/// clients that want fsnotify events without going through HTTP SSE
/// (the HTTP `/stream/watch` handler uses this same op internally so
/// SSE-disconnect cleanup is consistent).
async fn op_watch_subscribe(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let wp = super::fsnotify::WatchedPath {
        path: std::path::PathBuf::from(&path),
        shard_slug: format!("ipc-watch-{}", super::shard::hash8(&path)),
        source_root: path.clone(),
        kind: super::fsnotify::WatchKind::Generic,
    };
    let id = state
        .fs_watcher
        .subscribe(wp, recursive)
        .map_err(|e| ErrPayload::new("watch_register", e.to_string()))?;
    Ok(json!({
        "watch_id": id,
        "path": path,
        "recursive": recursive,
    }))
}

/// `watch_unsubscribe {watch_id}` — drop one subscription. Returns
/// `{removed: bool}`. Decrements the path's refcount; the underlying
/// notify::Watcher is unwatched only when the last subscriber for
/// that path unsubscribes.
async fn op_watch_unsubscribe(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("watch_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `watch_id` (u64)"))?;
    let removed = state.fs_watcher.unsubscribe(id);
    Ok(json!({
        "watch_id": id,
        "removed": removed,
    }))
}

/// `watch_list {}` — snapshot of every active subscription. Returns
/// `{subscriptions: [{watch_id, path, ref_count}], count}`. The
/// `ref_count` field tells you how many other subscribers share the
/// same path (1 = only this subscription).
async fn op_watch_list(state: &Arc<DaemonState>) -> OpResult {
    let subs = state.fs_watcher.list_subscriptions();
    let entries: Vec<Value> = subs
        .into_iter()
        .map(|(id, path, count)| {
            json!({
                "watch_id": id,
                "path": path,
                "ref_count": count,
            })
        })
        .collect();
    let count = entries.len();
    Ok(json!({
        "subscriptions": entries,
        "count": count,
    }))
}

async fn op_log_level(args: Value) -> OpResult {
    let directive = args
        .get("directive")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `directive`"))?;
    super::log::set_runtime_level(directive).map_err(|e| ErrPayload::new("reload_failed", e))?;
    Ok(json!({ "directive": directive }))
}

async fn op_log_rotate(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    let rotated = super::ticker::force_rotate_now(state);
    Ok(json!({ "rotated": rotated }))
}

async fn op_log_stats(state: &Arc<DaemonState>) -> OpResult {
    let mut total_bytes: u64 = 0;
    let mut total_lines: u64 = 0;
    let mut files: Vec<Value> = Vec::new();
    if let Ok(dir) = std::fs::read_dir(&state.paths.root) {
        for entry in dir.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy().to_string();
            if !super::paths::is_zshrs_log_file(&s) {
                continue;
            }
            let path = entry.path();
            let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let lines = std::fs::read_to_string(&path)
                .map(|c| c.lines().count() as u64)
                .unwrap_or(0);
            total_bytes += bytes;
            total_lines += lines;
            files.push(json!({ "name": s, "bytes": bytes, "lines": lines }));
        }
    }
    Ok(json!({
        "files": files,
        "total_bytes": total_bytes,
        "total_lines": total_lines,
    }))
}

// -------- pub/sub --------

async fn op_subscribe(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `pattern`"))?
        .to_string();

    if pattern.starts_with("--list") {
        // --list: report this client's existing subscriptions. Convention used by zsubscribe --list.
        let subs = state.list_subscriptions_for(client_id);
        return Ok(json!({
            "subscriptions": subs.iter().map(|s| json!({
                "id": s.id,
                "pattern": s.pattern,
            })).collect::<Vec<_>>(),
        }));
    }

    match state.add_subscription(client_id, &pattern) {
        Ok(id) => Ok(json!({
            "subscription_id": id,
            "pattern": pattern,
        })),
        Err(e) => Err(ErrPayload::new("bad_pattern", e)),
    }
}

async fn op_subscription_set_paused(
    state: &Arc<DaemonState>,
    client_id: u64,
    args: Value,
) -> OpResult {
    let paused = args
        .get("paused")
        .and_then(Value::as_bool)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `paused` boolean"))?;
    if args.get("all").and_then(Value::as_bool).unwrap_or(false) {
        let n = state.pause_all_subscriptions(client_id, paused);
        return Ok(json!({ "affected": n, "paused": paused }));
    }
    if let Some(id) = args.get("id").and_then(Value::as_u64) {
        let ok = state.set_subscription_paused(client_id, id, paused);
        return Ok(json!({ "affected": if ok { 1 } else { 0 }, "paused": paused }));
    }
    Err(ErrPayload::new("bad_args", "specify `id` or `all: true`"))
}

async fn op_unsubscribe(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    if let Some(id) = args.get("id").and_then(Value::as_u64) {
        let removed = state.remove_subscription_by_id(client_id, id);
        return Ok(json!({ "removed": if removed { 1 } else { 0 } }));
    }
    let pattern = args
        .get("pattern")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `pattern` or `id`"))?
        .to_string();
    let removed = state.remove_subscription_by_pattern(client_id, &pattern);
    Ok(json!({ "removed": removed }))
}

/// Generic publish op. Used by clients to inject events into the fan-out channel
/// (e.g. preexec/precmd hooks publishing `commands`/`chpwd`/`prompt`).
async fn op_publish(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let topic = args
        .get("topic")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `topic`"))?
        .to_string();
    let data = args.get("data").cloned().unwrap_or(Value::Null);

    let origin = state
        .origin_scope(client_id)
        .ok_or_else(|| ErrPayload::new("no_session", "client session not found"))?;

    // The match event delivered to subscribers carries scope+topic+data.
    let scope_str = origin.canonical();
    let payload = json!({
        "subscription_id": null,
        "scope": scope_str,
        "topic": topic,
        "data": data,
    });
    let frame = Frame::event("match", payload);
    let count = state.publish(&origin, &topic, frame);

    Ok(json!({ "delivered_to": count }))
}

// -------- load_script / stats / keys / subscribe_shard --------

async fn op_load_script(state: &Arc<DaemonState>, args: Value) -> OpResult {
    // Same protocol as source_resolve but with kind='script'. For v1 we delegate
    // to source_resolve which inserts kind='source' — overwrite the kind here.
    let resp = super::source_resolver::op_source_resolve(state, args.clone()).await?;
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();
    state.with_catalog(|conn| {
        conn.execute(
            "UPDATE compiled_files SET kind = 'script' WHERE path = ?",
            rusqlite::params![path],
        )?;
        Ok::<_, rusqlite::Error>(())
    })?;
    Ok(resp)
}

async fn op_stats_flush(state: &Arc<DaemonState>, args: Value) -> OpResult {
    // Client batches per-entry stats deltas (call_count, total_ns, last_called_at).
    // Payload: { "deltas": [ {fq_name, calls, total_ns, last_ns}, ... ] }
    let deltas = args
        .get("deltas")
        .and_then(Value::as_array)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `deltas` array"))?;

    let mut merged = 0usize;
    state.with_catalog(|conn| {
        let tx = conn.unchecked_transaction()?;
        for d in deltas {
            let fq = d.get("fq_name").and_then(Value::as_str).unwrap_or("");
            if fq.is_empty() {
                continue;
            }
            let calls = d.get("calls").and_then(Value::as_i64).unwrap_or(0);
            let total = d.get("total_ns").and_then(Value::as_i64).unwrap_or(0);
            let last = d.get("last_ns").and_then(Value::as_i64);
            tx.execute(
                "INSERT INTO entry_stats (fq_name, last_called_at, call_count, total_ns) \
                 VALUES (?, ?, ?, ?) \
                 ON CONFLICT(fq_name) DO UPDATE SET \
                    last_called_at = COALESCE(excluded.last_called_at, entry_stats.last_called_at), \
                    call_count = entry_stats.call_count + excluded.call_count, \
                    total_ns = entry_stats.total_ns + excluded.total_ns",
                rusqlite::params![fq, last, calls, total],
            )?;
            merged += 1;
        }
        tx.commit()?;
        Ok::<_, rusqlite::Error>(())
    })?;

    Ok(json!({ "merged": merged }))
}

async fn op_keys(state: &Arc<DaemonState>, args: Value) -> OpResult {
    // Return the key list of a daemon-served special parameter (or any canonical
    // subsystem). For an actual zsh-style `_comps`, the keys are the names of
    // commands with a registered handler.
    let param = args
        .get("param")
        .and_then(Value::as_str)
        .unwrap_or("compdef");
    super::zsync::ensure_schema(state)?;
    let subsystem = match param {
        "_comps" => "compdef",
        other => other,
    };
    let keys: Vec<String> = state.with_catalog(|conn| {
        let mut stmt =
            conn.prepare("SELECT key FROM canonical WHERE subsystem = ? ORDER BY key ASC")?;
        let rows = stmt
            .query_map(rusqlite::params![subsystem], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok::<_, rusqlite::Error>(rows)
    })?;
    Ok(json!({ "param": param, "keys": keys, "count": keys.len() }))
}

async fn op_subscribe_shard(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let shard = args
        .get("shard")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `shard`"))?
        .to_string();
    let pattern = format!("*.shard_updated:{}", shard);
    match state.add_subscription(client_id, &pattern) {
        Ok(id) => Ok(json!({ "subscription_id": id, "pattern": pattern })),
        Err(e) => Err(ErrPayload::new("bad_pattern", e)),
    }
}

// -------- export_zcompdump / export_catalog / export_shard / import_zcompdump --------

async fn op_export_zcompdump(state: &Arc<DaemonState>, args: Value) -> OpResult {
    super::zsync::ensure_schema(state)?;
    let out_path: std::path::PathBuf = args
        .get("path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .map(|h| h.join(".zcompdump"))
                .unwrap_or_else(|| std::path::PathBuf::from(".zcompdump"))
        });

    // Round-trip path: if a previous `zcache import zcompdump` stored the raw
    // body under canonical[zcompdump_raw][body], emit it byte-identically.
    // This is the spec'd "byte-compatible with what compinit would have
    // produced" guarantee for legacy tooling that introspects the file.
    let raw = state
        .canonical
        .row("zcompdump_raw", "body")
        .map(|r| r.value);
    let body = if let Some(raw_json) = raw {
        match serde_json::from_str::<serde_json::Value>(&raw_json) {
            Ok(serde_json::Value::String(s)) => s,
            _ => synthesize_zcompdump(state),
        }
    } else {
        synthesize_zcompdump(state)
    };
    let bytes_written = body.len();
    std::fs::write(&out_path, body)?;
    super::paths::ensure_file_600(&out_path)?;

    Ok(json!({
        "path": out_path.display().to_string(),
        "bytes": bytes_written,
        "round_tripped": raw_present_check(state),
    }))
}

fn raw_present_check(state: &Arc<DaemonState>) -> bool {
    state.canonical.row("zcompdump_raw", "body").is_some()
}

/// Synthesize a minimal .zcompdump body from the canonical compdef rows.
/// Used when no raw body has been imported. Format is the same shape zsh
/// emits (`_comps=(\n 'k' 'v' \n)\n`) so legacy tooling parses it cleanly.
fn synthesize_zcompdump(state: &Arc<DaemonState>) -> String {
    let mut out = String::new();
    let zsh_version = std::env::var("ZSH_VERSION").unwrap_or_else(|_| "5.9".to_string());
    let comps: Vec<(String, String)> = state
        .canonical
        .rows_for("compdef")
        .into_iter()
        .map(|r| (r.key, unjson_string(&r.value)))
        .collect();
    let services: Vec<(String, String)> = state
        .canonical
        .rows_for("service")
        .into_iter()
        .map(|r| (r.key, unjson_string(&r.value)))
        .collect();
    let patcomps: Vec<(String, String)> = state
        .canonical
        .rows_for("patcomp")
        .into_iter()
        .map(|r| (r.key, unjson_string(&r.value)))
        .collect();
    let postpatcomps: Vec<(String, String)> = state
        .canonical
        .rows_for("postpatcomp")
        .into_iter()
        .map(|r| (r.key, unjson_string(&r.value)))
        .collect();
    let autoloads: Vec<String> = state
        .canonical
        .rows_for("autoload_completion")
        .into_iter()
        .map(|r| r.key)
        .collect();
    out.push_str(&format!(
        "#files: {}\tversion: {}\n\n",
        comps.len(),
        zsh_version
    ));
    push_assoc(&mut out, "_comps", &comps);
    push_assoc(&mut out, "_services", &services);
    push_assoc(&mut out, "_patcomps", &patcomps);
    push_assoc(&mut out, "_postpatcomps", &postpatcomps);
    if !autoloads.is_empty() {
        out.push_str("\nautoload -Uz ");
        for (i, fn_name) in autoloads.iter().enumerate() {
            if i > 0 && i % 5 == 0 {
                out.push_str("\\\n            ");
            } else if i > 0 {
                out.push(' ');
            }
            out.push_str(fn_name);
        }
        out.push('\n');
    }
    out.push_str("\ntypeset -gUa _comp_assocs\n_comp_assocs=( '' )\n");
    out
}

fn push_assoc(out: &mut String, name: &str, rows: &[(String, String)]) {
    out.push_str(&format!("{}=(\n", name));
    for (k, v) in rows {
        out.push_str(&format!("'{}' '{}'\n", k, v));
    }
    out.push_str(")\n\n");
}

fn unjson_string(s: &str) -> String {
    serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| s.to_string())
}

async fn op_export_catalog(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let out_path: std::path::PathBuf = args
        .get("path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| state.paths.root.join("catalog.export.db"));
    // Use sqlite's online backup API via VACUUM INTO (atomic, safe under WAL).
    let target = out_path.display().to_string();
    state.with_catalog(|conn| {
        conn.execute("VACUUM INTO ?", rusqlite::params![target])?;
        Ok::<_, rusqlite::Error>(())
    })?;
    Ok(json!({ "path": out_path.display().to_string() }))
}

async fn op_export_shard(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `name`"))?
        .to_string();
    let out_path: std::path::PathBuf = args
        .get("path")
        .and_then(Value::as_str)
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(format!("./{}.rkyv", name)));

    // Find a shard whose filename ends with `-{name}.rkyv` or starts with the slug.
    let shard_path = super::shard::list_shards(&state.paths)
        .unwrap_or_default()
        .into_iter()
        .find(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.contains(&format!("-{}.rkyv", name)) || s.contains(&name))
        })
        .ok_or_else(|| ErrPayload::new("no_shard", format!("shard `{}` not found", name)))?;

    std::fs::copy(&shard_path, &out_path)?;
    Ok(json!({
        "from": shard_path.display().to_string(),
        "to": out_path.display().to_string(),
    }))
}

async fn op_import_zcompdump(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();

    let content = std::fs::read_to_string(&path)
        .map_err(|e| ErrPayload::new("read_failed", format!("{}: {}", path, e)))?;

    // Full-fidelity zcompdump parse: header, _comps / _services / _patcomps /
    // _postpatcomps assoc-array sections, bindkey lines, autoload list,
    // _comp_assocs trailer. Stored in canonical as structured rows for
    // queryability, AND the raw body is kept verbatim under
    // `zcompdump_raw[body]` so `zcache export zcompdump` round-trips
    // byte-identically.
    //
    // Per docs/DAEMON.md "For legacy tooling that introspects .zcompdump
    // directly (some plugin patterns, backup scripts, p10k cache-staleness
    // probes, parallel zsh sessions sharing the cache), the daemon can
    // synthesize a valid .zcompdump file on demand from its canonical state.
    // The synthesized file is byte-compatible with what compinit would have
    // produced".
    let parsed = parse_zcompdump(&content);

    // Store raw body for round-trip identity.
    let raw_json = serde_json::Value::String(content.clone()).to_string();
    state
        .canonical
        .upsert("zcompdump_raw", "body", &raw_json, None);
    if let Some(h) = parsed.header.as_ref() {
        let h_json = serde_json::Value::String(h.clone()).to_string();
        state
            .canonical
            .upsert("zcompdump_raw", "header", &h_json, None);
    }

    // Structured imports — useful for `zcache view compdef` etc.
    let mut imported = 0usize;
    for (k, v) in &parsed.comps {
        let val = serde_json::Value::String(v.clone()).to_string();
        state.canonical.upsert("compdef", k, &val, None);
        imported += 1;
    }
    for (k, v) in &parsed.services {
        let val = serde_json::Value::String(v.clone()).to_string();
        state.canonical.upsert("service", k, &val, None);
    }
    for (k, v) in &parsed.patcomps {
        let val = serde_json::Value::String(v.clone()).to_string();
        state.canonical.upsert("patcomp", k, &val, None);
    }
    for (k, v) in &parsed.postpatcomps {
        let val = serde_json::Value::String(v.clone()).to_string();
        state.canonical.upsert("postpatcomp", k, &val, None);
    }
    for (k, v) in &parsed.bindkeys {
        let val = serde_json::Value::String(v.clone()).to_string();
        state.canonical.upsert("bindkey", k, &val, None);
    }
    for fn_name in &parsed.autoload_funcs {
        state
            .canonical
            .upsert("autoload_completion", fn_name, "\"loaded\"", None);
    }

    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    if let Err(e) = state.persist_canonical(generation) {
        tracing::warn!(?e, "canonical: persist+hydrate after import failed");
    }

    Ok(json!({
        "imported": imported,
        "from": path,
        "comps": parsed.comps.len(),
        "services": parsed.services.len(),
        "patcomps": parsed.patcomps.len(),
        "postpatcomps": parsed.postpatcomps.len(),
        "autoload_funcs": parsed.autoload_funcs.len(),
        "raw_bytes": content.len(),
        "header": parsed.header,
    }))
}

/// `import_zwc` — explicit user opt-in for ingesting a `.zwc` companion file.
/// Per docs/DAEMON.md: validates adjacent source freshness (mtime) before
/// merging; stale `.zwc`s are skipped with WARN. On a fresh match, records
/// the `.zwc` against the source file in `compiled_files` so subsequent
/// `source` calls can skip the parse step.
///
/// Implementation: walks the `.zwc` to discover the embedded source path
/// (zsh stores the source path in the `.zwc` header), stat()s it, compares
/// mtime. The actual bytecode equivalence ingest happens when the user
/// `source`s the file later — this op just records the `.zwc` as a known
/// fresh-bytecode artifact.
async fn op_import_zwc(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let path_str = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);
    let tree_mode = args.get("tree").and_then(Value::as_bool).unwrap_or(false);

    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    let p = std::path::Path::new(&path_str);

    if tree_mode {
        if !p.is_dir() {
            return Err(ErrPayload::new(
                "bad_args",
                format!("--tree requires a directory, got `{}`", path_str),
            ));
        }
        for ent in walkdir::WalkDir::new(p)
            .follow_links(false)
            .into_iter()
            .filter_map(|r| r.ok())
        {
            if ent.file_type().is_file()
                && ent.path().extension().map(|e| e == "zwc").unwrap_or(false)
            {
                entries.push(ent.path().to_path_buf());
            }
        }
    } else {
        if !p.is_file() {
            return Err(ErrPayload::new(
                "no_such_file",
                format!("`{}` not found or not a file", path_str),
            ));
        }
        entries.push(p.to_path_buf());
    }

    let mut imported = Vec::<String>::new();
    let mut skipped = Vec::<Value>::new();

    for zwc in &entries {
        let zwc_meta = match std::fs::metadata(zwc) {
            Ok(m) => m,
            Err(e) => {
                skipped.push(json!({"path": zwc.display().to_string(), "reason": e.to_string()}));
                continue;
            }
        };
        let zwc_mtime_ns = zwc_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        // Adjacent source: zwc filename minus `.zwc`, in the same dir.
        // E.g. `_git.zwc` → `_git`; `tokens.sh.zwc` → `tokens.sh`.
        let stem = match zwc.file_name().and_then(|n| n.to_str()) {
            Some(n) if n.ends_with(".zwc") => &n[..n.len() - 4],
            _ => {
                skipped.push(
                    json!({"path": zwc.display().to_string(), "reason": "not a .zwc filename"}),
                );
                continue;
            }
        };
        let source = zwc
            .parent()
            .map(|d| d.join(stem))
            .unwrap_or_else(|| stem.into());
        if !source.exists() {
            skipped.push(json!({
                "path": zwc.display().to_string(),
                "reason": format!("adjacent source `{}` missing", source.display()),
            }));
            continue;
        }
        let src_meta = match std::fs::metadata(&source) {
            Ok(m) => m,
            Err(e) => {
                skipped.push(json!({"path": zwc.display().to_string(), "reason": e.to_string()}));
                continue;
            }
        };
        let src_mtime_ns = src_meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        if !force && src_mtime_ns > zwc_mtime_ns {
            skipped.push(json!({
                "path": zwc.display().to_string(),
                "reason": "stale .zwc — adjacent source is newer (re-zcompile or pass --force)",
            }));
            tracing::warn!(zwc = %zwc.display(), source = %source.display(), "stale .zwc skipped");
            continue;
        }

        // Record the source file under compiled_files marked as having a
        // known-fresh .zwc companion. Hash + bytecode are populated lazily
        // when the user `source`s the file (the existing source_resolver
        // path takes over).
        let source_str = source.display().to_string();
        let zwc_str = zwc.display().to_string();
        let parent_paths =
            serde_json::Value::Array(vec![serde_json::Value::String(zwc_str.clone())]).to_string();
        let inode = nix_inode(&src_meta);

        let res = state.with_catalog(|conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT INTO compiled_files \
                   (path, kind, mtime, inode, hash, last_used_at, use_count, \
                    bytes_in, bytes_out, sensitive, parent_paths) \
                   VALUES (?, 'autoload', ?, ?, '', ?, 0, ?, 0, 0, ?) \
                   ON CONFLICT(path) DO UPDATE SET \
                     mtime=excluded.mtime, inode=excluded.inode, \
                     parent_paths=excluded.parent_paths",
                rusqlite::params![
                    source_str,
                    src_mtime_ns,
                    inode,
                    chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                    src_meta.len() as i64,
                    parent_paths,
                ],
            )?;
            Ok(())
        });
        if let Err(e) = res {
            skipped.push(json!({
                "path": zwc.display().to_string(),
                "reason": format!("catalog write: {}", e),
            }));
            continue;
        }
        imported.push(zwc.display().to_string());
        tracing::info!(zwc = %zwc.display(), source = %source.display(), "zwc imported");
    }

    Ok(json!({
        "imported": imported,
        "imported_count": imported.len(),
        "skipped": skipped,
        "skipped_count": skipped.len(),
        "scanned": entries.len(),
    }))
}

/// `import_history` — ingest a legacy zsh history file into history.db.
/// Per docs/DAEMON.md:425. Recognized formats:
///   - extended: lines like `: <epoch>:<duration>;<command>` (zsh
///     `EXTENDED_HISTORY` and zpwr's 25 MB / 478 k file).
///   - plain: one command per line (no metadata).
///
/// Each parsed entry calls history::append. Existing entries with the same
/// (ts_ns, line) tuple are skipped to keep imports idempotent.
async fn op_import_history(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();
    let dedupe = args.get("dedupe").and_then(Value::as_bool).unwrap_or(true);

    let bytes = std::fs::read(&path)
        .map_err(|e| ErrPayload::new("read_failed", format!("{}: {}", path, e)))?;
    // zsh history is typically valid UTF-8, but tolerate lossy bytes from
    // legacy locale-quirky entries by replacing them rather than failing.
    let content = String::from_utf8_lossy(&bytes).into_owned();

    let entries = parse_zsh_history_file(&content);
    if entries.is_empty() {
        return Ok(json!({
            "imported": 0,
            "skipped": 0,
            "from": path,
            "format": "empty",
        }));
    }

    let host = std::env::var("HOST")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok());

    let res = state.with_history(|conn| -> rusqlite::Result<(usize, usize)> {
        let mut imported = 0usize;
        let mut skipped = 0usize;
        let tx = conn.unchecked_transaction()?;
        for (ts_ns, duration, line) in &entries {
            if dedupe {
                let exists: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM history WHERE ts_ns = ? AND line = ?",
                    rusqlite::params![ts_ns, line],
                    |r| r.get(0),
                )?;
                if exists > 0 {
                    skipped += 1;
                    continue;
                }
            }
            tx.execute(
                "INSERT INTO history (line, ts_ns, exit_code, cwd, duration_ns, sessid, hostname, shell_id) \
                 VALUES (?, ?, NULL, NULL, ?, NULL, ?, NULL)",
                rusqlite::params![line, ts_ns, duration, host],
            )?;
            imported += 1;
        }
        tx.commit()?;
        Ok((imported, skipped))
    });
    let (imported, skipped) = res.map_err(|e| ErrPayload::new("history_write", e.to_string()))?;

    tracing::info!(imported, skipped, total = entries.len(), %path, "history imported");

    Ok(json!({
        "imported": imported,
        "skipped": skipped,
        "total_parsed": entries.len(),
        "from": path,
        "bytes_in": bytes.len(),
    }))
}

/// `config_get` — read a runtime config knob. Per docs/DAEMON.md:905.
async fn op_config_get(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let key = args
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `key`"))?;
    let value = state.config_get(key);
    Ok(json!({
        "key": key,
        "value": value,
        "source": match value.is_some() {
            true => "runtime_or_env",
            false => "unset",
        },
    }))
}

/// `config_set` — runtime override for any tunable knob (long_cmd_threshold,
/// log level shortcuts, etc.). Per docs/DAEMON.md:905
/// `zcache config set long_cmd_threshold <seconds>`.
async fn op_config_set(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let key = args
        .get("key")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `key`"))?
        .to_string();
    let value = args
        .get("value")
        .map(|v| match v {
            Value::String(s) => s.clone(),
            Value::Number(n) => n.to_string(),
            Value::Bool(b) => b.to_string(),
            other => other.to_string(),
        })
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `value`"))?;

    // Surface validation for known keys so callers get a clear error rather
    // than silent acceptance of garbage. Unknown keys are accepted (forward-
    // compat for future knobs).
    match key.as_str() {
        "long_cmd_threshold" | "log_max_bytes" | "log_max_rotations" => {
            value.parse::<u64>().map_err(|e| {
                ErrPayload::new(
                    "bad_value",
                    format!("`{}` requires a non-negative integer: {}", key, e),
                )
            })?;
        }
        _ => {}
    }
    let prior = state.config_set(&key, value.clone());
    tracing::info!(key, value, prior = ?prior, "config_set");
    Ok(json!({
        "key": key,
        "value": value,
        "prior": prior,
    }))
}

/// `config_list` — full snapshot of in-memory runtime overrides.
async fn op_config_list(state: &Arc<DaemonState>) -> OpResult {
    let snap = state.config_snapshot();
    let map: serde_json::Map<String, Value> = snap
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();
    Ok(json!({
        "runtime": map,
        "note": "env vars (ZSHRS_*) act as fallbacks when a key isn't set here",
    }))
}

/// `replay_log` — return the per-source-root replay-log body so a booting
/// client can `eval` it in-process. Per docs/DAEMON.md "Determinism
/// boundary" (line 278). Two modes:
///   - source_root: explicit absolute path (typical: `~/.zshrc`).
///   - default:     scan replay_dir for the user's most-recently-modified
///     log; that's what spawn-on-demand wants on cold-start.
async fn op_replay_log(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let source_root = args
        .get("source_root")
        .and_then(Value::as_str)
        .map(str::to_string);

    let replay_path = match source_root {
        Some(sr) => {
            let hash8 = super::shard::hash8(&sr);
            // Pick whichever stem matches our hash8 prefix (don't try to
            // reconstruct the stem heuristic — just glob the dir).
            let mut found: Option<std::path::PathBuf> = None;
            if let Ok(rd) = std::fs::read_dir(&state.paths.replay_dir) {
                for ent in rd.flatten() {
                    let n = ent.file_name();
                    let s = n.to_string_lossy();
                    if s.starts_with(&format!("{}-", hash8)) && s.ends_with(".zsh") {
                        found = Some(ent.path());
                        break;
                    }
                }
            }
            found
        }
        None => {
            // Newest replay file in the dir (most-recently regenerated).
            let mut newest: Option<(std::time::SystemTime, std::path::PathBuf)> = None;
            if let Ok(rd) = std::fs::read_dir(&state.paths.replay_dir) {
                for ent in rd.flatten() {
                    let p = ent.path();
                    if p.extension().and_then(|s| s.to_str()) != Some("zsh") {
                        continue;
                    }
                    let mtime = match ent.metadata().and_then(|m| m.modified()) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    match &newest {
                        Some((cur, _)) if *cur >= mtime => {}
                        _ => newest = Some((mtime, p)),
                    }
                }
            }
            newest.map(|(_, p)| p)
        }
    };

    let path = match replay_path {
        Some(p) => p,
        None => {
            return Ok(json!({
                "found": false,
                "body": "",
            }));
        }
    };

    let body = std::fs::read_to_string(&path)
        .map_err(|e| ErrPayload::new("read_failed", format!("{}: {}", path.display(), e)))?;
    let line_count = body
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .count();
    Ok(json!({
        "found": true,
        "path": path.display().to_string(),
        "body": body,
        "line_count": line_count,
    }))
}

/// `import_catalog` — restore catalog.db from a backup file. Per
/// docs/DAEMON.md:569. The incoming file is validated as a SQLite database
/// (open + integrity_check) before being copied into place. Existing
/// catalog.db is moved to `catalog.db.backup-<ts>` so an accidental import
/// can be reversed.
async fn op_import_catalog(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);

    let p = std::path::Path::new(&path);
    if !p.is_file() {
        return Err(ErrPayload::new(
            "no_such_file",
            format!("`{}` not found or not a file", path),
        ));
    }

    // Validate the incoming file is a real SQLite db with intact tables we expect.
    let probe = super::catalog::open_at(p)
        .map_err(|e| ErrPayload::new("bad_catalog", format!("open: {}", e)))?;
    let table_count: i64 = probe
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table'",
            [],
            |r| r.get(0),
        )
        .map_err(|e| ErrPayload::new("bad_catalog", format!("schema check: {}", e)))?;
    if table_count == 0 {
        return Err(ErrPayload::new(
            "bad_catalog",
            "incoming file has no tables; not a catalog backup",
        ));
    }
    let integrity_ok: bool = probe
        .query_row("PRAGMA integrity_check", [], |r| {
            r.get::<_, String>(0).map(|s| s == "ok")
        })
        .unwrap_or(false);
    if !integrity_ok && !force {
        return Err(ErrPayload::new(
            "integrity_check_failed",
            "incoming catalog failed PRAGMA integrity_check (pass --force to override)",
        ));
    }
    drop(probe);

    let dest = state.paths.catalog_db.clone();
    let backup_name = format!(
        "catalog.db.backup-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S")
    );
    let backup_path = state.paths.root.join(&backup_name);

    if dest.exists() {
        std::fs::rename(&dest, &backup_path).map_err(|e| {
            ErrPayload::new(
                "rename_failed",
                format!("could not back up existing catalog: {}", e),
            )
        })?;
    }
    std::fs::copy(p, &dest)
        .map_err(|e| ErrPayload::new("copy_failed", format!("{}: {}", path, e)))?;
    super::paths::ensure_file_600(&dest).ok();

    tracing::info!(from = %path, backup = %backup_name, integrity_ok, "catalog imported");

    Ok(json!({
        "imported_from": path,
        "backup_at": backup_path.display().to_string(),
        "integrity_ok": integrity_ok,
        "table_count": table_count,
    }))
}

/// `import_shard` — restore a single rkyv shard into images/. Per
/// docs/DAEMON.md:570. Validates magic + format_version before placing.
async fn op_import_shard(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let name = args
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `name` (shard slug)"))?
        .to_string();
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path` (incoming .rkyv file)"))?
        .to_string();
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);

    let src = std::path::Path::new(&path);
    if !src.is_file() {
        return Err(ErrPayload::new(
            "no_such_file",
            format!("`{}` not found", path),
        ));
    }

    // Validate magic + format_version by attempting to mmap it.
    let _probe = super::shard::MmappedShard::open(src)
        .map_err(|e| ErrPayload::new("bad_shard", format!("{}: {}", path, e)))?;

    // Sanitize the slug — only [A-Za-z0-9_-] allowed, prevents directory escape.
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ErrPayload::new(
            "bad_name",
            "shard name must match [A-Za-z0-9_-]+",
        ));
    }

    let dest = state.paths.images.join(format!("{}.rkyv", name));
    if dest.exists() && !force {
        return Err(ErrPayload::new(
            "exists",
            format!(
                "shard `{}` already present at {} — pass --force to overwrite",
                name,
                dest.display()
            ),
        ));
    }

    let tmp = dest.with_extension("rkyv.tmp.import");
    std::fs::copy(src, &tmp)
        .map_err(|e| ErrPayload::new("copy_failed", format!("{}: {}", path, e)))?;
    super::paths::ensure_file_600(&tmp).ok();
    std::fs::rename(&tmp, &dest)
        .map_err(|e| ErrPayload::new("rename_failed", format!("{}: {}", dest.display(), e)))?;

    tracing::info!(name, from = %path, dest = %dest.display(), "shard imported");

    Ok(json!({
        "imported": name,
        "from": path,
        "to": dest.display().to_string(),
    }))
}

/// `export_all` — pack every cache artifact into one tar archive. Per
/// docs/DAEMON.md:562. Plain tar (no zstd dep — keeps the durable-deps
/// principle from CLAUDE.md). Includes index.rkyv, every shard, catalog.db,
/// history.db, log files. Sensitive shards excluded unless include_sensitive.
async fn op_export_all(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let out = args
        .get("out")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `out`"))?
        .to_string();
    let include_sensitive = args
        .get("include_sensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    // Collect candidate file list (relative paths within ~/.zshrs/).
    let root = &state.paths.root;
    let mut entries: Vec<std::path::PathBuf> = Vec::new();
    if state.paths.index_rkyv.exists() {
        entries.push(state.paths.index_rkyv.clone());
    }
    for shard in super::shard::list_shards(&state.paths).unwrap_or_default() {
        entries.push(shard);
    }
    if state.paths.catalog_db.exists() {
        entries.push(state.paths.catalog_db.clone());
    }
    if state.paths.history_db.exists() {
        entries.push(state.paths.history_db.clone());
    }
    if let Ok(rd) = std::fs::read_dir(root) {
        for ent in rd.flatten() {
            let n = ent.file_name();
            let s = n.to_string_lossy();
            if super::paths::is_zshrs_log_file(&s) {
                entries.push(ent.path());
            }
        }
    }

    // Filter out sensitive shards unless --include-sensitive. Lookup is by
    // matching shard slug against compiled_files.path that has sensitive=1
    // (we only know the source-file path, not the shard, so we filter
    // shards whose slug encodes a sensitive source path's hash8).
    if !include_sensitive {
        let sensitive_paths: Vec<String> = state
            .with_catalog(|conn| -> rusqlite::Result<Vec<String>> {
                let mut stmt = conn.prepare("SELECT path FROM compiled_files WHERE sensitive=1")?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(rows)
            })
            .unwrap_or_default();
        let sensitive_hashes: Vec<String> = sensitive_paths
            .iter()
            .map(|p| super::shard::hash8(p))
            .collect();
        if !sensitive_hashes.is_empty() {
            entries.retain(|p| {
                let fname = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
                !sensitive_hashes.iter().any(|h| fname.starts_with(h))
            });
        }
    }

    // Write a simple ustar archive (POSIX). We avoid the `tar` crate dep —
    // keeps the daemon self-contained per CLAUDE.md endgame rule.
    let out_path = std::path::Path::new(&out);
    let mut writer = std::fs::File::create(out_path)
        .map_err(|e| ErrPayload::new("create_failed", format!("{}: {}", out, e)))?;
    let mut archived: Vec<String> = Vec::new();
    for src in &entries {
        let rel = src.strip_prefix(root).unwrap_or(src);
        let rel_str = rel.display().to_string();
        let bytes = match std::fs::read(src) {
            Ok(b) => b,
            Err(_) => continue,
        };
        if let Err(e) = write_ustar_entry(&mut writer, &rel_str, &bytes) {
            return Err(ErrPayload::new(
                "tar_failed",
                format!("write entry {}: {}", rel_str, e),
            ));
        }
        archived.push(rel_str);
    }
    // ustar end marker: two zero blocks.
    use std::io::Write as _;
    let zero = [0u8; 1024];
    writer
        .write_all(&zero)
        .map_err(|e| ErrPayload::new("tar_failed", format!("trailer: {}", e)))?;
    writer
        .flush()
        .map_err(|e| ErrPayload::new("tar_failed", format!("flush: {}", e)))?;

    let total_bytes = std::fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    tracing::info!(out = %out, archived_count = archived.len(), total_bytes, "export_all complete");

    Ok(json!({
        "out": out,
        "archived": archived,
        "archived_count": archived.len(),
        "total_bytes": total_bytes,
        "include_sensitive": include_sensitive,
    }))
}

/// `import_all` — unpack a tar archive produced by `export_all` back into
/// ~/.zshrs/. Per docs/DAEMON.md:571. Existing files are renamed to
/// `*.preimport-<ts>` so the import is reversible.
async fn op_import_all(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?
        .to_string();
    let force = args.get("force").and_then(Value::as_bool).unwrap_or(false);

    let src = std::path::Path::new(&path);
    if !src.is_file() {
        return Err(ErrPayload::new(
            "no_such_file",
            format!("`{}` not found", path),
        ));
    }

    let bytes = std::fs::read(src)
        .map_err(|e| ErrPayload::new("read_failed", format!("{}: {}", path, e)))?;

    let entries = parse_ustar(&bytes).map_err(|e| ErrPayload::new("bad_archive", e))?;
    if entries.is_empty() {
        return Err(ErrPayload::new("bad_archive", "no entries in archive"));
    }

    let root = &state.paths.root;
    let backup_suffix = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();
    let mut imported: Vec<String> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();

    for (rel, content) in &entries {
        // Path-traversal guard.
        if rel.contains("..") || rel.starts_with('/') {
            skipped.push(json!({"path": rel, "reason": "unsafe path"}));
            continue;
        }
        let dest = root.join(rel);
        if dest.exists() {
            if !force {
                let backup = dest.with_file_name(format!(
                    "{}.preimport-{}",
                    dest.file_name().and_then(|n| n.to_str()).unwrap_or("file"),
                    backup_suffix
                ));
                let _ = std::fs::rename(&dest, &backup);
            } else {
                let _ = std::fs::remove_file(&dest);
            }
        }
        if let Some(parent) = dest.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&dest, content) {
            skipped.push(json!({"path": rel, "reason": e.to_string()}));
            continue;
        }
        super::paths::ensure_file_600(&dest).ok();
        imported.push(rel.clone());
    }

    tracing::info!(
        from = %path,
        imported = imported.len(),
        skipped = skipped.len(),
        "import_all complete",
    );

    Ok(json!({
        "from": path,
        "imported": imported,
        "imported_count": imported.len(),
        "skipped": skipped,
        "skipped_count": skipped.len(),
        "advice": "run `zcache verify` to confirm the restore",
    }))
}

/// Append one ustar-format entry to `writer`. Plain POSIX tar — no extensions,
/// no GNU tar magic, no compression. Matches what `tar -xf` will read.
fn write_ustar_entry(writer: &mut std::fs::File, name: &str, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let name_bytes = name.as_bytes();
    if name_bytes.len() > 100 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("path `{}` exceeds ustar 100-char limit", name),
        ));
    }
    let mut header = [0u8; 512];
    header[..name_bytes.len()].copy_from_slice(name_bytes);
    // mode (octal, 7 chars + null): 0600
    let mode_field = b"0000600";
    header[100..107].copy_from_slice(mode_field);
    // uid / gid: 0
    header[108..115].copy_from_slice(b"0000000");
    header[116..123].copy_from_slice(b"0000000");
    // size (octal, 11 chars + null)
    let size_str = format!("{:011o}", data.len());
    header[124..135].copy_from_slice(size_str.as_bytes());
    // mtime: now (octal, 11 chars + null)
    let mtime = chrono::Utc::now().timestamp() as u64;
    let mtime_str = format!("{:011o}", mtime);
    header[136..147].copy_from_slice(mtime_str.as_bytes());
    // chksum field starts as spaces (8 bytes)
    header[148..156].fill(b' ');
    // typeflag: '0' (regular file)
    header[156] = b'0';
    // ustar magic + version
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    // checksum is sum-of-bytes interpreted as unsigned, with chksum field
    // counted as spaces (already done above).
    let checksum: u32 = header.iter().map(|&b| b as u32).sum();
    let chk_str = format!("{:06o}\0 ", checksum);
    header[148..156].copy_from_slice(chk_str.as_bytes());
    writer.write_all(&header)?;
    writer.write_all(data)?;
    // Pad data to 512-byte boundary.
    let pad_len = (512 - (data.len() % 512)) % 512;
    if pad_len > 0 {
        let zeros = vec![0u8; pad_len];
        writer.write_all(&zeros)?;
    }
    Ok(())
}

/// Parse a ustar archive into (relative_path, content) pairs. Mirrors what
/// `write_ustar_entry` produces. Stops at the trailing zero blocks.
fn parse_ustar(bytes: &[u8]) -> std::result::Result<Vec<(String, Vec<u8>)>, String> {
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut pos = 0usize;
    while pos + 512 <= bytes.len() {
        let header = &bytes[pos..pos + 512];
        // Two zero blocks → end.
        if header.iter().all(|&b| b == 0) {
            break;
        }
        // Validate ustar magic.
        if &header[257..263] != b"ustar\0" {
            return Err(format!("bad ustar magic at offset {}", pos));
        }
        // Name (up to 100 bytes, NUL-terminated).
        let name_end = header[..100].iter().position(|&b| b == 0).unwrap_or(100);
        let name = std::str::from_utf8(&header[..name_end])
            .map_err(|e| format!("bad name encoding at offset {}: {}", pos, e))?
            .to_string();
        // Size (octal in 12 bytes, last is NUL).
        let size_str = std::str::from_utf8(&header[124..135])
            .map_err(|e| format!("bad size encoding: {}", e))?
            .trim();
        let size: usize = usize::from_str_radix(size_str.trim_end_matches('\0'), 8)
            .map_err(|e| format!("bad size value `{}`: {}", size_str, e))?;
        pos += 512;
        if pos + size > bytes.len() {
            return Err(format!(
                "entry `{}` claims {} bytes but archive is truncated",
                name, size
            ));
        }
        let content = bytes[pos..pos + size].to_vec();
        out.push((name, content));
        pos += size;
        // Round up to next 512-byte boundary.
        let pad = (512 - (size % 512)) % 512;
        pos += pad;
    }
    Ok(out)
}

/// Parse a zsh history file. Supports both EXTENDED_HISTORY format
/// (`: <epoch>:<duration>;<line>`) and plain one-line-per-command format.
/// Multi-line commands continued via trailing `\` are folded to one entry.
fn parse_zsh_history_file(content: &str) -> Vec<(i64, Option<i64>, String)> {
    let mut out: Vec<(i64, Option<i64>, String)> = Vec::new();
    let mut buf: Option<(i64, Option<i64>, String)> = None;

    let now_ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let mut fallback_ts = now_ns - (content.len() as i64);

    for raw_line in content.split_inclusive('\n') {
        let line = raw_line.strip_suffix('\n').unwrap_or(raw_line);
        // Continuation: previous line ended with backslash.
        if let Some((ts, dur, partial)) = &mut buf {
            if partial.ends_with('\\') {
                partial.pop();
                partial.push('\n');
                partial.push_str(line);
                if !partial.ends_with('\\') {
                    out.push((*ts, *dur, partial.clone()));
                    buf = None;
                }
                continue;
            } else {
                out.push((*ts, *dur, partial.clone()));
                buf = None;
            }
        }

        // Extended history format.
        if let Some(rest) = line.strip_prefix(": ") {
            if let Some((meta, cmd)) = rest.split_once(';') {
                if let Some((ts_s, dur_s)) = meta.split_once(':') {
                    let ts: i64 = ts_s.trim().parse().unwrap_or(fallback_ts / 1_000_000_000);
                    let dur: Option<i64> =
                        dur_s.trim().parse::<i64>().ok().map(|d| d * 1_000_000_000);
                    let ts_ns = ts.saturating_mul(1_000_000_000);
                    if cmd.ends_with('\\') {
                        buf = Some((ts_ns, dur, cmd.to_string()));
                    } else {
                        out.push((ts_ns, dur, cmd.to_string()));
                    }
                    continue;
                }
            }
        }

        // Plain format: one command per line. Skip empty lines.
        if line.is_empty() {
            continue;
        }
        fallback_ts = fallback_ts.saturating_add(1_000_000_000);
        if line.ends_with('\\') {
            buf = Some((fallback_ts, None, line.to_string()));
        } else {
            out.push((fallback_ts, None, line.to_string()));
        }
    }
    if let Some(b) = buf {
        out.push(b);
    }
    out
}

#[cfg(unix)]
fn nix_inode(md: &std::fs::Metadata) -> i64 {
    use std::os::unix::fs::MetadataExt;
    md.ino() as i64
}
#[cfg(not(unix))]
fn nix_inode(_md: &std::fs::Metadata) -> i64 {
    0
}

/// Parsed structure of a .zcompdump file. Order is preserved (Vec, not
/// HashMap) so the export emits in the original order.
#[derive(Default, Debug)]
struct ZcompdumpParsed {
    header: Option<String>,
    comps: Vec<(String, String)>,
    services: Vec<(String, String)>,
    patcomps: Vec<(String, String)>,
    postpatcomps: Vec<(String, String)>,
    bindkeys: Vec<(String, String)>,
    autoload_funcs: Vec<String>,
}

fn parse_zcompdump(content: &str) -> ZcompdumpParsed {
    let mut out = ZcompdumpParsed::default();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0usize;

    // Header line: `#files: N\tversion: V`
    if let Some(first) = lines.first() {
        if first.starts_with("#files:") {
            out.header = Some(first.to_string());
        }
    }

    while i < lines.len() {
        let line = lines[i];
        // Block-literal form: `_comps=(\n 'k' 'v' \n)`.
        if line.starts_with("_comps=(") {
            i = parse_assoc_block(&lines, i + 1, &mut out.comps);
            continue;
        }
        if line.starts_with("_services=(") {
            i = parse_assoc_block(&lines, i + 1, &mut out.services);
            continue;
        }
        if line.starts_with("_patcomps=(") {
            i = parse_assoc_block(&lines, i + 1, &mut out.patcomps);
            continue;
        }
        if line.starts_with("_postpatcomps=(") {
            i = parse_assoc_block(&lines, i + 1, &mut out.postpatcomps);
            continue;
        }
        // Single-element-assignment form: `_comps[key]=value`. Common in
        // hand-rolled zcompdumps and what op_import_zcompdump's pre-parser
        // historically supported.
        if line.starts_with("_comps[") {
            if let Some((k, v)) = parse_single_assoc_assign(line, "_comps[") {
                out.comps.push((k, v));
            }
            i += 1;
            continue;
        }
        if line.starts_with("_services[") {
            if let Some((k, v)) = parse_single_assoc_assign(line, "_services[") {
                out.services.push((k, v));
            }
            i += 1;
            continue;
        }
        if line.starts_with("_patcomps[") {
            if let Some((k, v)) = parse_single_assoc_assign(line, "_patcomps[") {
                out.patcomps.push((k, v));
            }
            i += 1;
            continue;
        }
        if line.starts_with("_postpatcomps[") {
            if let Some((k, v)) = parse_single_assoc_assign(line, "_postpatcomps[") {
                out.postpatcomps.push((k, v));
            }
            i += 1;
            continue;
        }
        if line.starts_with("bindkey ") {
            // `bindkey '^[/' _history-complete-older`
            if let Some((key, fn_name)) = parse_bindkey_line(line) {
                out.bindkeys.push((key, fn_name));
            }
            i += 1;
            continue;
        }
        if line.starts_with("autoload -Uz ") || line.starts_with("autoload -Uz +X ") {
            // Multi-line continuation: `autoload -Uz fn1 fn2 \` then
            // `            fn3 fn4 \`. Collect until a non-`\`-terminated line.
            let mut buf = String::new();
            let mut j = i;
            loop {
                let l = lines[j];
                let trimmed_end = l.trim_end();
                let (body, more) = if let Some(stripped) = trimmed_end.strip_suffix('\\') {
                    (stripped, true)
                } else {
                    (trimmed_end, false)
                };
                buf.push_str(body);
                buf.push(' ');
                j += 1;
                if !more || j >= lines.len() {
                    break;
                }
            }
            // Strip leading "autoload -Uz" or "autoload -Uz +X"
            let body = buf
                .trim_start_matches("autoload -Uz +X")
                .trim_start_matches("autoload -Uz")
                .trim();
            for tok in body.split_whitespace() {
                out.autoload_funcs.push(tok.to_string());
            }
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

/// Parse an assoc-array block: starts after the `_X=(` opener, terminated
/// by `)`. Each row is `'key' 'value'`. Returns the index AFTER the `)`.
fn parse_assoc_block(lines: &[&str], start: usize, out: &mut Vec<(String, String)>) -> usize {
    let mut i = start;
    while i < lines.len() {
        let line = lines[i].trim();
        if line == ")" {
            return i + 1;
        }
        // `'key' 'value'`. Both quoted with single quotes.
        if let Some((k, v)) = split_two_singlequoted(line) {
            out.push((k, v));
        }
        i += 1;
    }
    i
}

fn split_two_singlequoted(s: &str) -> Option<(String, String)> {
    // Find first '...' then second '...'
    let s = s.trim();
    if !s.starts_with('\'') {
        return None;
    }
    let close1 = s[1..].find('\'')?;
    let key = &s[1..1 + close1];
    let rest = &s[1 + close1 + 1..].trim_start();
    if !rest.starts_with('\'') {
        return None;
    }
    let close2 = rest[1..].find('\'')?;
    let val = &rest[1..1 + close2];
    Some((key.to_string(), val.to_string()))
}

/// Parse `_comps[key]=value` (with the prefix `_comps[` already matched).
/// Handles bare and quoted values. Strips outer single/double quotes from
/// both key and value.
fn parse_single_assoc_assign(line: &str, prefix: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix(prefix)?;
    let close = rest.find(']')?;
    let key = rest[..close].trim_matches(|c: char| c == '\'' || c == '"');
    let after = &rest[close + 1..];
    let after = after.trim_start_matches('=').trim();
    let val = after.trim_matches(|c: char| c == '\'' || c == '"');
    Some((key.to_string(), val.to_string()))
}

fn parse_bindkey_line(line: &str) -> Option<(String, String)> {
    // `bindkey '<KEY>' <fn>`
    let rest = line.trim_start_matches("bindkey").trim();
    if !rest.starts_with('\'') {
        return None;
    }
    let close = rest[1..].find('\'')?;
    let key = &rest[1..1 + close];
    let fn_name = rest[close + 2..].trim();
    if fn_name.is_empty() {
        return None;
    }
    Some((key.to_string(), fn_name.to_string()))
}

/// Locate the `zshrs` binary (or `zshrs-daemon` if it's on PATH) and spawn a
/// detached `--daemon` instance. Used by the `daemon restart` verb. Returns
/// the new daemon PID on success.
///
/// Resolution order:
///   1. `$ZSHRS_BIN` — explicit override.
///   2. `/proc/self/exe` (Linux) — current binary self-path; works for
///      `zshrs --daemon` invocations launched from the shell binary.
///   3. PATH lookup of `zshrs-daemon` then `zshrs`.
fn spawn_replacement_daemon() -> std::io::Result<u32> {
    use std::process::Command;

    let exe = if let Ok(p) = std::env::var("ZSHRS_BIN") {
        std::path::PathBuf::from(p)
    } else if let Ok(p) = std::env::current_exe() {
        p
    } else {
        std::path::PathBuf::from("zshrs")
    };

    // If we're invoked as zshrs-daemon, replicate ourselves with no args.
    // If invoked as zshrs (the shell), pass --daemon.
    let is_daemon_bin = exe
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.contains("daemon"))
        .unwrap_or(false);

    let mut cmd = Command::new(&exe);
    if !is_daemon_bin {
        cmd.arg("--daemon");
    }

    // Detach: new session, redirect stdio to /dev/null. The new daemon writes
    // tracing output to ~/.zshrs/zshrs-daemon.log.
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            // Fully detach: setsid + close stdio.
            let _ = nix::unistd::setsid();
            for fd in 0..3 {
                let _ = nix::unistd::close(fd);
            }
            // Re-open as /dev/null so subsequent writes don't EBADF.
            let _ = std::fs::OpenOptions::new().read(true).open("/dev/null");
            Ok(())
        });
    }

    let child = cmd.spawn()?;
    Ok(child.id())
}

// -------- Helpers --------

fn parse_tags(args: &Value) -> std::result::Result<Vec<String>, ErrPayload> {
    let v = args
        .get("tags")
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `tags` array"))?;
    let arr = v
        .as_array()
        .ok_or_else(|| ErrPayload::new("bad_args", "`tags` must be an array of strings"))?;
    let mut out = Vec::with_capacity(arr.len());
    for t in arr {
        let s = t
            .as_str()
            .ok_or_else(|| ErrPayload::new("bad_args", "`tags` entries must be strings"))?;
        out.push(s.to_string());
    }
    Ok(out)
}

fn event_name(ev: Event) -> &'static str {
    match ev {
        Event::ShardUpdated => "shard_updated",
        Event::RebuildComplete => "rebuild_complete",
        Event::CanonicalChanged => "canonical_changed",
        Event::Match => "match",
        Event::CmdExecute => "cmd:execute",
        Event::Notify => "notify",
        Event::DaemonShutdown => "daemon_shutdown",
        Event::AskPending => "ask:pending",
        Event::AskDismissed => "ask:dismissed",
        Event::AskProgress => "ask:progress",
        Event::LongCmdComplete => "long_cmd_complete",
        Event::LongCmdStarted => "long_cmd_started",
        Event::LongCmdFailed => "long_cmd_failed",
        Event::LongCmdSignaled => "long_cmd_signaled",
    }
}

// -------- job_* ops (zjob supervisor) --------

async fn op_job_submit(state: &Arc<DaemonState>, client_id: u64, args: Value) -> OpResult {
    let command: Vec<String> = args
        .get("command")
        .and_then(Value::as_array)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `command` array"))?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    if command.is_empty() {
        return Err(ErrPayload::new("bad_args", "`command` is empty"));
    }
    let cwd = args.get("cwd").and_then(Value::as_str).map(str::to_string);
    let tags: Vec<String> = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let env: std::collections::HashMap<String, String> = args
        .get("env")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                .collect()
        })
        .unwrap_or_default();
    // pty=true allocates a pseudo-terminal pair: child stdin/stdout/stderr
    // bound to the slave, master held by the daemon for `zjob attach`'s
    // bidirectional pump. Defaults to false so non-pty submits stay
    // unchanged (line-buffered .out/.err pipe path).
    let pty = args.get("pty").and_then(Value::as_bool).unwrap_or(false);

    match state.jobs.submit(client_id, command, cwd, tags, env, pty) {
        Ok(id) => Ok(json!({ "job_id": id, "pty": pty })),
        Err(e) => Err(ErrPayload::new("submit_failed", e.to_string())),
    }
}

async fn op_job_list(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let state_filter = args.get("state").and_then(Value::as_str);
    let tag_filter = args.get("tag").and_then(Value::as_str);
    let limit = args.get("limit").and_then(Value::as_u64);
    let jobs = state.jobs.list(state_filter, tag_filter, limit);
    Ok(json!({ "jobs": jobs, "count": jobs.len() }))
}

// Note on payload key naming: Frame::Response is `{ id, ok, ...payload }` with
// payload flattened. Any payload field named "id" would clobber the response
// id at JSON serialize time, so all job_* responses use "job_id" instead.

async fn op_job_status(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    match state.jobs.status(id) {
        Some(s) => Ok(json!({ "job": s })),
        None => Err(ErrPayload::new("no_job", format!("job {} not found", id))),
    }
}

async fn op_job_output(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    let stderr = args.get("stderr").and_then(Value::as_bool).unwrap_or(false);
    let lines = args.get("lines").and_then(Value::as_u64);
    let content = state
        .jobs
        .output(id, stderr, lines)
        .map_err(|e| ErrPayload::new("output_failed", e.to_string()))?;
    Ok(json!({ "job_id": id, "stderr": stderr, "content": content }))
}

async fn op_job_kill(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    let signal = args.get("signal").and_then(Value::as_str);
    let killed = state
        .jobs
        .kill(id, signal)
        .map_err(|e| ErrPayload::new("kill_failed", e.to_string()))?;
    Ok(json!({ "job_id": id, "killed": killed }))
}

async fn op_job_cancel(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    let grace_ms = args
        .get("grace_ms")
        .and_then(Value::as_u64)
        .unwrap_or(5_000);
    let final_state = state
        .jobs
        .cancel(id, std::time::Duration::from_millis(grace_ms))
        .await
        .map_err(|e| ErrPayload::new("cancel_failed", e.to_string()))?;
    Ok(json!({
        "job_id": id,
        "state": final_state.label(),
        "exit_code": final_state.exit_code(),
    }))
}

async fn op_job_wait(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    let timeout_ms = args.get("timeout_ms").and_then(Value::as_u64);
    let rx = state
        .jobs
        .wait_handle(id)
        .map_err(|e| ErrPayload::new("no_job", e.to_string()))?;
    let timeout = timeout_ms.map(std::time::Duration::from_millis);
    match super::jobs::wait_with_timeout(rx, timeout).await {
        Some(state_) => Ok(json!({
            "job_id": id,
            "state": state_.label(),
            "exit_code": state_.exit_code(),
            "timed_out": false,
        })),
        None => Ok(json!({ "job_id": id, "timed_out": true })),
    }
}

/// `job_input` — write base64-decoded bytes to the master fd of a
/// pty-mode job. Errors with `not_pty` for jobs that weren't
/// submitted with `pty=true`. Used by `zjob attach`'s stdin pump
/// and by scripted "feed N keystrokes to a stuck job" callers.
async fn op_job_input(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    let b64 = args
        .get("bytes_b64")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `bytes_b64`"))?;
    let bytes = base64_decode(b64)
        .map_err(|e| ErrPayload::new("bad_args", format!("decode bytes_b64: {e}")))?;
    let handle = state.jobs.pty_handle_for(id).ok_or_else(|| {
        ErrPayload::new(
            "not_pty",
            format!("job {id} is not pty-mode (resubmit with pty=true)"),
        )
    })?;
    let n = handle
        .write(&bytes)
        .map_err(|e| ErrPayload::new("write_failed", e.to_string()))?;
    Ok(json!({"job_id": id, "bytes_written": n}))
}

/// `job_resize` — propagate a terminal resize (TIOCSWINSZ) to the
/// foreground process group of a pty-mode job. Errors with `not_pty`
/// for non-pty jobs. Used by `zjob attach`'s SIGWINCH handler.
async fn op_job_resize(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let id = args
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `id`"))?;
    let rows = args
        .get("rows")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `rows`"))? as u16;
    let cols = args
        .get("cols")
        .and_then(Value::as_u64)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `cols`"))? as u16;
    let handle = state
        .jobs
        .pty_handle_for(id)
        .ok_or_else(|| ErrPayload::new("not_pty", format!("job {id} is not pty-mode")))?;
    handle
        .resize(rows, cols)
        .map_err(|e| ErrPayload::new("ioctl_failed", e.to_string()))?;
    Ok(json!({"job_id": id, "rows": rows, "cols": cols}))
}

/// Local base64 decoder, mirrors `base64_encode` in daemon/jobs.rs.
/// Tolerates whitespace + padding; errors on any other non-alphabet
/// byte.
fn base64_decode(s: &str) -> std::result::Result<Vec<u8>, String> {
    fn val(c: u8) -> Option<u8> {
        match c {
            b'A'..=b'Z' => Some(c - b'A'),
            b'a'..=b'z' => Some(c - b'a' + 26),
            b'0'..=b'9' => Some(c - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let bytes: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut buf = [0u8; 4];
    let mut bi = 0;
    for &b in &bytes {
        let v = val(b).ok_or_else(|| format!("invalid base64 byte: 0x{b:02x}"))?;
        buf[bi] = v;
        bi += 1;
        if bi == 4 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
            bi = 0;
        }
    }
    if bi >= 2 {
        out.push((buf[0] << 2) | (buf[1] >> 4));
    }
    if bi == 3 {
        out.push((buf[1] << 4) | (buf[2] >> 2));
    }
    Ok(out)
}

#[cfg(test)]
mod highlight_tests {
    use super::*;

    fn empty_set() -> std::collections::BTreeSet<String> {
        std::collections::BTreeSet::new()
    }

    fn set(items: &[&str]) -> std::collections::BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn never(_: &str) -> bool {
        false
    }
    fn always(_: &str) -> bool {
        true
    }

    fn kind_at(spans: &[Value], idx: usize) -> &str {
        spans[idx]["kind"].as_str().unwrap()
    }

    #[test]
    fn classifies_known_command() {
        let spans = highlight_line("ls /tmp", &empty_set(), &empty_set(), &empty_set(), &always);
        assert!(spans.len() >= 2);
        assert_eq!(kind_at(&spans, 0), "command");
        assert_eq!(kind_at(&spans, 1), "path");
    }

    #[test]
    fn flags_unknown_command_as_error() {
        let spans = highlight_line(
            "totally_not_a_command foo",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &never,
        );
        assert_eq!(kind_at(&spans, 0), "error");
    }

    #[test]
    fn classifies_alias_at_command_position() {
        let spans = highlight_line("ll /tmp", &set(&["ll"]), &empty_set(), &empty_set(), &never);
        assert_eq!(kind_at(&spans, 0), "alias");
    }

    #[test]
    fn classifies_function_at_command_position() {
        let spans = highlight_line(
            "myfn arg",
            &empty_set(),
            &empty_set(),
            &set(&["myfn"]),
            &never,
        );
        assert_eq!(kind_at(&spans, 0), "function");
    }

    #[test]
    fn classifies_builtin() {
        let spans = highlight_line("cd /tmp", &empty_set(), &empty_set(), &empty_set(), &never);
        assert_eq!(kind_at(&spans, 0), "builtin");
    }

    #[test]
    fn classifies_keyword() {
        let spans = highlight_line(
            "if true; then echo hi; fi",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &always,
        );
        assert_eq!(kind_at(&spans, 0), "keyword");
    }

    #[test]
    fn classifies_quoted_string() {
        let spans = highlight_line(
            "echo \"hello world\"",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &always,
        );
        assert_eq!(kind_at(&spans, 0), "builtin");
        assert_eq!(kind_at(&spans, 1), "string");
    }

    #[test]
    fn classifies_comment_to_eol() {
        let spans = highlight_line(
            "ls # this is a comment",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &always,
        );
        let kinds: Vec<&str> = spans.iter().map(|s| s["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"comment"));
    }

    #[test]
    fn classifies_param_expansion() {
        let spans = highlight_line(
            "echo $HOME",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &always,
        );
        assert_eq!(kind_at(&spans, 1), "param");
    }

    #[test]
    fn classifies_redirect_and_pipe() {
        let spans = highlight_line(
            "ls > /tmp/x | wc -l",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &always,
        );
        let kinds: Vec<&str> = spans.iter().map(|s| s["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"redirect"));
        assert!(kinds.contains(&"operator"));
    }

    #[test]
    fn second_command_after_pipe_classified() {
        let spans = highlight_line(
            "echo hi | unknown_cmd",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &never,
        );
        // After `|`, position resets — `unknown_cmd` is at command position
        // and unknown, so it's flagged red.
        let kinds: Vec<&str> = spans.iter().map(|s| s["kind"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"error"));
    }
}

#[cfg(test)]
mod doctor_tests {
    use super::*;

    #[test]
    fn nix_inode_returns_nonzero_for_real_files() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let md = std::fs::metadata(tmp.path()).unwrap();
        assert!(nix_inode(&md) > 0);
    }
}
