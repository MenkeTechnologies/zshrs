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

        "rebuild" => op_rebuild(state, args).await,
        "clean" => op_clean(state, args).await,
        "verify" => op_verify(state).await,
        "compact" => op_compact(state).await,
        "source_resolve" => super::source_resolver::op_source_resolve(state, args).await,
        "history_append" => super::history::op_history_append(state, args).await,
        "history_query" => super::history::op_history_query(state, args).await,
        "subscribe" => op_subscribe(state, client_id, args).await,
        "unsubscribe" => op_unsubscribe(state, client_id, args).await,
        "publish" => op_publish(state, client_id, args).await,
        "fpath_changed" => op_fpath_changed(state, args).await,
        "watcher_stats" => op_watcher_stats(state).await,
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

        // Stubs — all return unimplemented. Filling in is later-iteration work.
        "stats_flush"
        | "subscribe_shard"
        | "complete"
        | "suggest"
        | "highlight"
        | "keys"
        | "load_script"
        | "export_zcompdump"
        | "export_catalog"
        | "export_shard"
        | "import_zcompdump"
        | "register" => Err(ErrPayload::new(
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

// -------- rebuild / clean / verify / compact --------

async fn op_rebuild(state: &Arc<DaemonState>, args: Value) -> OpResult {
    // For v1: a single 'system' shard is rebuilt from the daemon's process env $PATH +
    // $FPATH. The shard arg is accepted for forward-compat but we always rebuild the
    // system shard right now (per-shard partial rebuild arrives with the .zshrc
    // analysis pass producing distinct shards per source root).
    let _shard_filter = args.get("shard").and_then(Value::as_str);

    let generation = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
    let (image_path, hydrated, stats) = match super::walk::run_full_rebuild(state, generation) {
        Ok(v) => v,
        Err(e) => return Err(ErrPayload::new("rebuild_failed", e.to_string())),
    };

    Ok(json!({
        "rebuilt": ["system"],
        "path": image_path.display().to_string(),
        "generation": generation,
        "entries_hydrated": hydrated,
        "walk_stats": stats,
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
            for entry in std::fs::read_dir(&paths.root)
                .map_err(|e| ErrPayload::new("io", e.to_string()))?
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
                format!("clean target `{}` not supported (try all|shards|index|log)", other),
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

    let tmp_swept = super::shard::sweep_tmp_files(
        &state.paths,
        std::time::Duration::from_secs(60),
    )
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
    let swept = super::shard::sweep_tmp_files(
        &state.paths,
        std::time::Duration::from_secs(60),
    )
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
