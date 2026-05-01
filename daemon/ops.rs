// IPC operation dispatch + handlers.
//
// Per docs/DAEMON.md "Operation table (client → daemon)":
//
//   Foundation v1 ships these ops:
//     info             — daemon stats, session count, uptime
//     ping             — roundtrip latency probe
//     list_shells      — zls data (every connected session, optional tag/cwd filter)
//     tag / untag      — self-tag this client
//     send             — dispatch payload to one shell, broadcast, or by tag
//     notify           — status-line/OSC-9 message
//     daemon           — daemon control (status, stop, restart-not-yet)
//
//   All other ops (rebuild, clean, verify, history_*, complete, suggest, highlight,
//   load_script, source_resolve, push/pull/diff_canonical, export_*, import_*,
//   subscribe/unsubscribe, ui_*) are stubbed to return `unimplemented` and will be
//   filled in across subsequent iterations.

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

        "rebuild" => op_rebuild(state, args).await,
        "zshrc_analyze" => op_zshrc_analyze(state, args).await,
        "first_init" => op_first_init(state, args).await,
        "plugin_discover" => op_plugin_discover(state, args).await,
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

        "job_submit" => op_job_submit(state, client_id, args).await,
        "job_list" => op_job_list(state, args).await,
        "job_status" => op_job_status(state, args).await,
        "job_output" => op_job_output(state, args).await,
        "job_kill" => op_job_kill(state, args).await,
        "job_cancel" => op_job_cancel(state, args).await,
        "job_wait" => op_job_wait(state, args).await,

        "complete" => op_complete(state, args).await,
        "suggest" => op_suggest(state, args).await,
        "highlight" => op_highlight(state, args).await,
        "register" => op_register(state, client_id, args).await,
        "doctor" => op_doctor(state).await,

        _ => Err(ErrPayload::new(
            "unknown_op",
            format!("unsupported op `{op}`"),
        )),
    };

    match &result {
        Ok(_) => tracing::info!(ok = true, "op handled"),
        Err(e) => tracing::info!(ok = false, code = %e.code, msg = %e.msg, "op failed"),
    }

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
                    tracing::warn!(?e, "could not spawn replacement daemon; client must spawn-on-demand");
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

// -------- rebuild / clean / verify / compact --------

async fn op_rebuild(state: &Arc<DaemonState>, args: Value) -> OpResult {
    // For v1: a single 'system' shard is rebuilt from the daemon's process env $PATH +
    // $FPATH. The shard arg is accepted for forward-compat but we always rebuild the
    // system shard right now (per-shard partial rebuild arrives with the .zshrc
    // analysis pass producing distinct shards per source root).
    let _shard_filter = args.get("shard").and_then(Value::as_str);
    let async_mode = args.get("async").and_then(Value::as_bool).unwrap_or(false);

    if async_mode {
        // Async path: spawn a background task, return job_id immediately.
        // rebuild_complete event fires when the walk finishes.
        let job_id = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
        let bg_state = Arc::clone(state);
        tokio::spawn(async move {
            let start = std::time::Instant::now();
            let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
            match super::walk::run_full_rebuild(&bg_state, generation) {
                Ok((image_path, hydrated, _stats)) => {
                    let event = json!({
                        "job_id": job_id,
                        "shard": "system",
                        "generation": generation,
                        "duration_ms": start.elapsed().as_millis() as u64,
                        "entries_hydrated": hydrated,
                        "image_path": image_path.display().to_string(),
                    });
                    let _ = bg_state
                        .broadcast(super::ipc::Frame::event("rebuild_complete", event), &[]);
                }
                Err(e) => {
                    tracing::warn!(?e, %job_id, "async rebuild failed");
                    let event = json!({
                        "job_id": job_id,
                        "shard": "system",
                        "error": e.to_string(),
                    });
                    let _ = bg_state
                        .broadcast(super::ipc::Frame::event("rebuild_failed", event), &[]);
                }
            }
        });
        return Ok(json!({
            "job_id": job_id,
            "async": true,
            "rebuilt": ["system"],
        }));
    }

    let start = std::time::Instant::now();
    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let (image_path, hydrated, stats) = match super::walk::run_full_rebuild(state, generation) {
        Ok(v) => v,
        Err(e) => return Err(ErrPayload::new("rebuild_failed", e.to_string())),
    };
    let duration_ms = start.elapsed().as_millis() as u64;

    // Broadcast rebuild_complete event so subscribers tracking shard updates
    // can re-mmap. Per docs/DAEMON.md async event types.
    let event = json!({
        "shard": "system",
        "generation": generation,
        "duration_ms": duration_ms,
        "entries_hydrated": hydrated,
        "image_path": image_path.display().to_string(),
    });
    let _ = state.broadcast(super::ipc::Frame::event("rebuild_complete", event), &[]);

    Ok(json!({
        "rebuilt": ["system"],
        "path": image_path.display().to_string(),
        "generation": generation,
        "entries_hydrated": hydrated,
        "walk_stats": stats,
        "duration_ms": duration_ms,
    }))
}

/// `zshrc_analyze` — run the analysis pass on `.zshrc` (or any source-style
/// file) plus every file transitively `source`d, capture deterministic state
/// (aliases / functions / setopt / bindkey / hash -d / compdef / zstyle /
/// zmodload / env / params / path+= / fpath+=), and seed the canonical table
/// with the result. After this, `zcache export aliases`, `zcache export path`,
/// etc. emit the discovered values directly. Per docs/DAEMON.md "Starting
/// state served by daemon" + "Walk lifecycle — first init".
async fn op_zshrc_analyze(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let path_str = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `path`"))?;
    let path = std::path::Path::new(path_str);
    if !path.exists() {
        return Err(ErrPayload::new(
            "no_such_file",
            format!("`{}` not found", path.display()),
        ));
    }

    let analysis = super::zshrc_analysis::analyze_with_sources(path)
        .map_err(|e| ErrPayload::new("analyze_failed", e.to_string()))?;

    // Seed the rkyv-backed canonical engine. SQLite is not touched here; the
    // hydrated `canonical` view table is refreshed lazily on `zcache hydrate-view`.
    let mut captured = 0usize;
    let canon = &state.canonical;
    for (k, v) in &analysis.aliases {
        captured += canon.upsert("alias", k, &json_string(v), None);
    }
    for (k, v) in &analysis.global_aliases {
        captured += canon.upsert("galias", k, &json_string(v), None);
    }
    for (k, v) in &analysis.suffix_aliases {
        captured += canon.upsert("salias", k, &json_string(v), None);
    }
    for (k, v) in &analysis.named_dirs {
        captured += canon.upsert("named_dir", k, &json_string(v), None);
    }
    for (k, v) in &analysis.compdef {
        captured += canon.upsert("compdef", k, &json_string(v), None);
    }
    for (k, v) in &analysis.bindkeys {
        captured += canon.upsert("bindkey", k, &json_string(v), None);
    }
    for (k, v) in &analysis.env_exports {
        captured += canon.upsert("env", k, &json_string(v), None);
    }
    for (k, v) in &analysis.params {
        captured += canon.upsert("params", k, &json_string(v), None);
    }
    for opt in &analysis.setopts {
        captured += canon.upsert("setopt", opt, "\"on\"", None);
    }
    for opt in &analysis.unsetopts {
        captured += canon.upsert("setopt", opt, "\"off\"", None);
    }
    for module in &analysis.zmodload {
        captured += canon.upsert("zmodload", module, "\"loaded\"", None);
    }
    for (i, dir) in analysis.path_additions.iter().enumerate() {
        captured += canon.upsert("path", &i.to_string(), &json_string(dir), None);
    }
    for (i, dir) in analysis.fpath_additions.iter().enumerate() {
        captured += canon.upsert("fpath", &i.to_string(), &json_string(dir), None);
    }
    for (i, dir) in analysis.manpath_additions.iter().enumerate() {
        captured += canon.upsert("manpath", &i.to_string(), &json_string(dir), None);
    }
    for (name, body) in &analysis.functions {
        captured += canon.upsert("function", name, &json_string(body), None);
    }
    for (pat, rest) in &analysis.zstyle {
        captured += canon.upsert("zstyle", pat, &json_string(rest), None);
    }

    // Persist the seeded state to the rkyv shard.
    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    if let Err(e) = canon.persist(generation) {
        tracing::warn!(?e, "canonical: persist after analyze failed");
    }

    // Register the .zshrc + every transitively-sourced file with the
    // fsnotify watcher under WatchKind::ZshrcSource. Future modifications
    // trigger re-analysis automatically (handled in fsnotify::reanalyze_zshrc).
    let mut watch_paths = vec![path.display().to_string()];
    for src in &analysis.sourced_files {
        watch_paths.push(src.clone());
    }
    for wp_path_str in watch_paths {
        let wp_path = std::path::PathBuf::from(&wp_path_str);
        if !wp_path.exists() {
            continue;
        }
        let wp = super::fsnotify::WatchedPath {
            path: wp_path,
            shard_slug: format!("zshrc-{}", super::shard::hash8(&wp_path_str)),
            source_root: path.display().to_string(), // re-analysis target
            kind: super::fsnotify::WatchKind::ZshrcSource,
        };
        if let Err(e) = state.fs_watcher.watch_path(wp, false) {
            tracing::warn!(?e, path=%wp_path_str, "fsnotify watch failed for zshrc source");
        }
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let event = serde_json::json!({
        "subsystem": "*",
        "row_count": captured,
        "set_at_ns": now,
        "set_by_shell": 0,
        "source": path.display().to_string(),
    });
    state.broadcast(super::ipc::Frame::event("canonical_changed", event), &[]);

    Ok(json!({
        "captured": captured,
        "files_analyzed": analysis.stats.files_analyzed,
        "lines_total": analysis.stats.lines_total,
        "lines_deterministic": analysis.stats.lines_deterministic,
        "lines_non_deterministic": analysis.stats.lines_non_deterministic,
        "duration_ms": analysis.stats.duration_ms,
        "plugins": analysis.plugin_decls.iter().map(|p| serde_json::json!({
            "manager": p.manager, "name": p.name, "source_path": p.source_path,
        })).collect::<Vec<_>>(),
        "sourced_files": analysis.sourced_files,
        "aliases": analysis.aliases.len(),
        "functions": analysis.functions.len(),
        "setopts": analysis.setopts.len(),
        "env_exports": analysis.env_exports.len(),
        "path_additions": analysis.path_additions.len(),
        "fpath_additions": analysis.fpath_additions.len(),
    }))
}

fn json_string(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

/// `plugin_discover` — walks ~/.zinit/plugins + ~/.zinit/snippets, picks the
/// init script of each, runs analyze on it, captures per-plugin state into
/// a per-plugin rkyv shard, folds the union into the canonical engine.
/// Per docs/DAEMON.md "Plugin discovery happens at the same time as `.zshrc`
/// analysis: daemon walks the user's `.zshrc`, sees zinit/OMZ/source calls,
/// descends into each referenced plugin, parses + bytecode-compiles
/// per-plugin shards (`{hash8}-plugin-{name}.rkyv`), captures every state
/// contribution".
async fn op_plugin_discover(state: &Arc<DaemonState>, _args: Value) -> OpResult {
    let (stats, records) = super::plugin_walk::run_full_discovery(&state.paths, &state.canonical)
        .map_err(|e| ErrPayload::new("plugin_walk_failed", e.to_string()))?;
    Ok(json!({
        "stats": stats,
        "plugins": records,
    }))
}

/// `first_init` — single-shot multi-pass walk lifecycle. Per docs/DAEMON.md
/// "Walk lifecycle — first init" Pass 1-4:
///   Pass 1+2: analyze_with_sources(.zshrc + transitive) → canonical state
///   Pass 3:   walk_paths over the now-resolved $PATH/$FPATH → command_hash
///             + autoload_table
///   Pass 4:   serialize system shard, hydrate catalog.entries, register
///             watches.
///
/// Replaces the manual `zcache rebuild --zshrc <path> && zcache rebuild`
/// dance. Returns combined stats from both passes.
async fn op_first_init(state: &Arc<DaemonState>, args: Value) -> OpResult {
    // ------ Pass 1+2: .zshrc analysis ------
    let analysis_resp = if let Some(path) = args.get("zshrc").and_then(Value::as_str) {
        Some(
            op_zshrc_analyze(state, json!({ "path": path }))
                .await?,
        )
    } else {
        None
    };

    // ------ Pass 3+4: walk + hydrate ------
    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let (image_path, hydrated, walk_stats) = super::walk::run_full_rebuild(state, generation)
        .map_err(|e| ErrPayload::new("rebuild_failed", e.to_string()))?;

    // ------ Pass 5: plugin tree discovery ------
    let (plugin_stats, plugin_records) =
        super::plugin_walk::run_full_discovery(&state.paths, &state.canonical)
            .map_err(|e| ErrPayload::new("plugin_walk_failed", e.to_string()))?;

    Ok(json!({
        "passes": ["analyze", "walk", "hydrate", "plugin_walk"],
        "analysis": analysis_resp,
        "walk": {
            "image_path": image_path.display().to_string(),
            "entries_hydrated": hydrated,
            "stats": walk_stats,
        },
        "plugins": {
            "stats": plugin_stats,
            "count": plugin_records.len(),
        },
        "generation": generation,
    }))
}

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
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
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
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);

    // Try cwd-scoped first (frecency wins for "in this dir, recently"),
    // then fall back to global prefix match.
    let row_opt = if let Some(c) = &cwd {
        state
            .with_history(|conn| {
                super::history::query(
                    conn,
                    Some(&prefix),
                    "prefix",
                    Some(c),
                    None,
                    None,
                    1,
                    true,
                )
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

    let spans = highlight_line(
        &line,
        &aliases,
        &galiases,
        &functions,
        &command_known,
    );

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
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
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
    "alias", "autoload", "bg", "bindkey", "break", "builtin", "cd", "chdir", "command", "compdef",
    "compinit", "compinstall", "continue", "declare", "dirs", "disable", "disown", "echo",
    "echotc", "echoti", "emulate", "enable", "eval", "exec", "exit", "export", "false", "fc",
    "fg", "float", "functions", "getln", "getopts", "hash", "history", "integer", "jobs", "kill",
    "let", "limit", "local", "log", "logout", "noglob", "popd", "print", "printf", "pushd",
    "pushln", "pwd", "r", "read", "readonly", "rehash", "return", "sched", "set", "setopt",
    "shift", "source", "suspend", "test", "times", "trap", "true", "ttyctl", "type", "typeset",
    "ulimit", "umask", "unalias", "unfunction", "unhash", "unlimit", "unset", "unsetopt", "wait",
    "whence", "where", "which", "zcompile", "zmodload", "zparseopts", "zstyle", ".",
    // zshrs-owned z* builtins
    "zcache", "zls", "zid", "zping", "ztag", "zuntag", "zsend", "znotify", "zsubscribe",
    "zunsubscribe", "zsync", "zask", "zlog", "zjob",
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
            format!("{} legacy artifacts in HOME (run `zcache clean legacy`)", litter),
        );
    }

    let total = checks.len();
    let failed = checks.iter().filter(|c| !c["ok"].as_bool().unwrap_or(false)).count();

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

async fn op_clean(state: &Arc<DaemonState>, args: Value) -> OpResult {
    let target = args
        .get("target")
        .and_then(Value::as_str)
        .unwrap_or("all")
        .to_string();

    let mut removed: Vec<String> = Vec::new();
    let paths = &state.paths;

    match target.as_str() {
        "all" => {
            for shard in super::shard::list_shards(paths).unwrap_or_default() {
                let _ = std::fs::remove_file(&shard);
                removed.push(shard.display().to_string());
            }
            if paths.index_rkyv.exists() {
                let _ = std::fs::remove_file(&paths.index_rkyv);
                removed.push(paths.index_rkyv.display().to_string());
            }
        }
        "shards" => {
            for shard in super::shard::list_shards(paths).unwrap_or_default() {
                let _ = std::fs::remove_file(&shard);
                removed.push(shard.display().to_string());
            }
        }
        "index" => {
            if paths.index_rkyv.exists() {
                let _ = std::fs::remove_file(&paths.index_rkyv);
                removed.push(paths.index_rkyv.display().to_string());
            }
        }
        "log" => {
            // Truncate today's rolled file (don't unlink — tracing-appender holds an fd).
            for entry in
                std::fs::read_dir(&paths.root).map_err(|e| ErrPayload::new("io", e.to_string()))?
            {
                if let Ok(entry) = entry {
                    let name = entry.file_name();
                    let s = name.to_string_lossy();
                    if s.starts_with("zshrs.log") {
                        let _ = std::fs::OpenOptions::new()
                            .write(true)
                            .truncate(true)
                            .open(entry.path());
                        removed.push(entry.path().display().to_string());
                    }
                }
            }
        }
        other => {
            return Err(ErrPayload::new(
                "bad_target",
                format!(
                    "clean target `{}` not supported (try all|shards|index|log)",
                    other
                ),
            ));
        }
    }

    Ok(json!({
        "removed": removed,
        "removed_count": removed.len(),
    }))
}

async fn op_verify(state: &Arc<DaemonState>) -> OpResult {
    let mut issues: Vec<String> = Vec::new();
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

    Ok(json!({
        "shards_ok": shards_ok,
        "shards_bad": shards_bad,
        "catalog_ok": catalog_ok,
        "tmp_swept": tmp_swept,
        "issues": issues,
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

async fn op_watcher_stats(state: &Arc<DaemonState>) -> OpResult {
    let stats = state.fs_watcher.stats();
    Ok(json!(stats))
}

async fn op_log_level(args: Value) -> OpResult {
    let directive = args
        .get("directive")
        .and_then(Value::as_str)
        .ok_or_else(|| ErrPayload::new("bad_args", "missing `directive`"))?;
    super::log::set_runtime_level(directive)
        .map_err(|e| ErrPayload::new("reload_failed", e))?;
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
            if !s.starts_with("zshrs.log") {
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
    Err(ErrPayload::new(
        "bad_args",
        "specify `id` or `all: true`",
    ))
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
        conn.execute(&format!("VACUUM INTO ?"), rusqlite::params![target])?;
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
            p.file_name().and_then(|s| s.to_str()).map_or(false, |s| {
                s.contains(&format!("-{}.rkyv", name)) || s.contains(&name)
            })
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
    state.canonical.upsert("zcompdump_raw", "body", &raw_json, None);
    if let Some(h) = parsed.header.as_ref() {
        let h_json = serde_json::Value::String(h.clone()).to_string();
        state.canonical.upsert("zcompdump_raw", "header", &h_json, None);
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
    if let Err(e) = state.canonical.persist(generation) {
        tracing::warn!(?e, "canonical: persist after import failed");
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
                && ent
                    .path()
                    .extension()
                    .map(|e| e == "zwc")
                    .unwrap_or(false)
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
                skipped.push(json!({"path": zwc.display().to_string(), "reason": "not a .zwc filename"}));
                continue;
            }
        };
        let source = zwc.parent().map(|d| d.join(stem)).unwrap_or_else(|| stem.into());
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
        let parent_paths = serde_json::Value::Array(vec![serde_json::Value::String(zwc_str.clone())]).to_string();
        let inode = nix_inode(&src_meta);

        let res = state.with_catalog(|conn| -> rusqlite::Result<()> {
            conn.execute(
                "INSERT INTO compiled_files \
                   (path, kind, mtime, inode, hash, bytecode, last_used_at, use_count, \
                    bytes_in, bytes_out, sensitive, parent_paths) \
                   VALUES (?, 'autoload', ?, ?, '', NULL, ?, 0, ?, 0, 0, ?) \
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
                let (body, more) = if trimmed_end.ends_with('\\') {
                    (&trimmed_end[..trimmed_end.len() - 1], true)
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
    // tracing output to ~/.cache/zshrs/zshrs.log.
    use std::os::unix::process::CommandExt;
    unsafe {
        cmd.pre_exec(|| {
            // Fully detach: setsid + close stdio.
            let _ = nix::unistd::setsid();
            for fd in 0..3 {
                let _ = nix::unistd::close(fd);
            }
            // Re-open as /dev/null so subsequent writes don't EBADF.
            let _ = std::fs::OpenOptions::new()
                .read(true)
                .open("/dev/null");
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
    let cwd = args
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::to_string);
    let tags: Vec<String> = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
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

    match state.jobs.submit(client_id, command, cwd, tags, env) {
        Ok(id) => Ok(json!({ "job_id": id })),
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
    let stderr = args
        .get("stderr")
        .and_then(Value::as_bool)
        .unwrap_or(false);
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
        let spans = highlight_line(
            "cd /tmp",
            &empty_set(),
            &empty_set(),
            &empty_set(),
            &never,
        );
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
