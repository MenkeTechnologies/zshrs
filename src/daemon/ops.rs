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
        "notify" => op_notify(state, client_id, args).await,
        "daemon" => op_daemon(state, args).await,

        // Stubs — all return unimplemented. Filling in is later-iteration work.
        "rebuild"
        | "clean"
        | "verify"
        | "compact"
        | "fpath_changed"
        | "stats_flush"
        | "subscribe_shard"
        | "history_append"
        | "history_query"
        | "complete"
        | "suggest"
        | "highlight"
        | "keys"
        | "load_script"
        | "source_resolve"
        | "push_canonical"
        | "pull_canonical"
        | "diff_canonical"
        | "export_zcompdump"
        | "export_catalog"
        | "export_shard"
        | "import_zcompdump"
        | "register"
        | "subscribe"
        | "unsubscribe" => Err(ErrPayload::new(
            "unimplemented",
            format!("op `{op}` not yet implemented in v1 foundation"),
        )),

        _ => Err(ErrPayload::new("unknown_op", format!("unsupported op `{op}`"))),
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

    let event_payload = json!({
        "delivery_id": format!("send-{}-{}", from, chrono::Utc::now().timestamp_millis()),
        "from_shell": from,
        "command": command,
    });
    let frame = Frame::event(event_name(Event::CmdExecute), event_payload);

    let delivered: Vec<u64> = if let Some(all) = target.get("all").and_then(Value::as_bool) {
        if all {
            // Broadcast to every session except originator.
            let _ = state.broadcast(frame, &[from]);
            state
                .snapshot_sessions()
                .into_iter()
                .filter(|s| s.client_id != from)
                .map(|s| s.client_id)
                .collect()
        } else {
            return Err(ErrPayload::new("bad_args", "target.all must be true if present"));
        }
    } else if let Some(tag) = target.get("tag").and_then(Value::as_str) {
        state.send_tag(tag, frame)
    } else if let Some(shell_id) = target.get("shell_id").and_then(Value::as_u64) {
        if state.send_to(shell_id, frame) {
            vec![shell_id]
        } else {
            return Err(ErrPayload::new("no_shell", format!("shell_id {shell_id} not found")));
        }
    } else {
        return Err(ErrPayload::new(
            "bad_args",
            "target must be one of {shell_id, tag, all}",
        ));
    };

    Ok(json!({
        "delivered_to": delivered,
        "delivered_count": delivered.len(),
    }))
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

    let delivered: Vec<u64> = if target.get("all").and_then(Value::as_bool).unwrap_or(false) {
        let _ = state.broadcast(frame, &[from]);
        state
            .snapshot_sessions()
            .into_iter()
            .filter(|s| s.client_id != from)
            .map(|s| s.client_id)
            .collect()
    } else if let Some(tag) = target.get("tag").and_then(Value::as_str) {
        state.send_tag(tag, frame)
    } else if let Some(shell_id) = target.get("shell_id").and_then(Value::as_u64) {
        if state.send_to(shell_id, frame) {
            vec![shell_id]
        } else {
            return Err(ErrPayload::new("no_shell", format!("shell_id {shell_id} not found")));
        }
    } else {
        return Err(ErrPayload::new(
            "bad_args",
            "target must be one of {shell_id, tag, all}",
        ));
    };

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
            // Schedule a self-SIGTERM after the response goes out.
            tokio::spawn(async {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;
                let _ = kill(Pid::from_raw(std::process::id() as i32), Signal::SIGTERM);
            });
            Ok(json!({ "stopping": true }))
        }

        "restart" => Err(ErrPayload::new(
            "unimplemented",
            "daemon restart requires a parent supervisor; use stop+spawn-on-demand",
        )),

        _ => Err(ErrPayload::new("bad_verb", format!("unknown daemon verb `{verb}`"))),
    }
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
