// Daemon accept loop + per-connection handler.
//
// Per docs/DAEMON.md "Daemon = sole writer" + "90/10 work split":
//   - tokio UnixListener on ~/.zshrs/daemon.sock
//   - one async task per connected client; reads frames, dispatches to ops::dispatch,
//     writes responses + async events back through a per-session mpsc::UnboundedSender
//   - graceful shutdown via state.shutdown signal (set by `daemon stop` op or SIGTERM)
//
// For v1 foundation, this only handles the handshake + the basic ops listed in
// `ops::dispatch`. Pub/sub routing, history.db writes, fpath rebuilds etc. arrive in
// later iterations.

use std::sync::Arc;

use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use super::ipc::{self, ErrPayload, Frame, Welcome, PROTOCOL_VERSION};
use super::ops;
use super::paths::CachePaths;
use super::state::DaemonState;
use super::{DaemonError, Result};

/// Run the daemon's accept loop until shutdown. Caller passes a fully-populated
/// `CachePaths`; we set up the listener, handle SIGTERM/SIGINT, and dispatch each
/// connection to a tokio task.
pub async fn serve(paths: CachePaths) -> Result<()> {
    // Cleanup stale socket from a previous (now-defunct) daemon. Acquire of the
    // pidlock above already guarantees we're the only daemon, so unlinking here
    // is safe.
    if paths.socket.exists() {
        let _ = std::fs::remove_file(&paths.socket);
    }

    let listener = UnixListener::bind(&paths.socket)?;
    super::paths::ensure_file_600(&paths.socket)?;

    tracing::info!(socket = %paths.socket.display(), "listening");

    let state = DaemonState::new(paths.clone())?;

    // First-pass diagnostics — TRACE-gated so they don't flood at INFO.
    // Each db size is what's on disk RIGHT NOW; useful for spotting
    // catalog/history bloat without poking sqlite by hand.
    let catalog_bytes = std::fs::metadata(&paths.catalog_db)
        .map(|m| m.len())
        .unwrap_or(0);
    let history_bytes = std::fs::metadata(&paths.history_db)
        .map(|m| m.len())
        .unwrap_or(0);
    let cache_bytes = std::fs::metadata(&paths.cache_db)
        .map(|m| m.len())
        .unwrap_or(0);
    let shard_count = super::shard::list_shards(&paths)
        .map(|v| v.len())
        .unwrap_or(0);
    tracing::trace!(
        catalog_db_bytes = catalog_bytes,
        history_db_bytes = history_bytes,
        cache_db_bytes = cache_bytes,
        shard_count,
        "server: state opened"
    );

    // Spawn the fsnotify watcher task. No paths are registered initially;
    // they're added by the walk-lifecycle evaluator + `fpath_changed` op.
    if let Err(e) = state.fs_watcher.start(Arc::clone(&state)) {
        tracing::warn!(?e, "fsnotify watcher failed to start; running degraded");
    }

    // Spawn the periodic housekeeping ticker (tmp sweep, log size monitor,
    // catalog vacuum, zask timeouts). One minute cadence, weak-ref to state.
    super::ticker::spawn(Arc::clone(&state));
    tracing::trace!("server: ticker spawned");

    // Spawn the schedule tick driver (daemon.schedule.* ops). Wakes once
    // a second, dispatches `job_submit` for any due cron / one-shot rows.
    super::schedule::spawn_tick(Arc::clone(&state));
    tracing::trace!("server: schedule tick spawned");

    // HTTP listener (off by default; opt-in via [http].listen in
    // ~/.zshrs/daemon.toml). Surfaces the same op set as the
    // unix-socket IPC path so curl/httpie/any HTTP client can talk
    // to the daemon. See daemon/http.rs + docs/DAEMON_AS_SERVICE.md.
    let http_cfg = match super::paths::load_http_config() {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(?e, "load_http_config failed; http listener disabled");
            super::http::HttpConfig::default()
        }
    };
    if let Err(e) = super::http::serve_http(http_cfg, Arc::clone(&state)).await {
        tracing::warn!(?e, "http listener init failed; continuing without http");
    }

    let shutdown = tokio::sync::Notify::new();
    let shutdown = Arc::new(shutdown);

    // Watch for SIGTERM / SIGINT.
    let shutdown_signals = Arc::clone(&shutdown);
    tokio::spawn(async move {
        let mut sigterm =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(?e, "failed to install SIGTERM handler");
                    return;
                }
            };
        let mut sigint =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt()) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(?e, "failed to install SIGINT handler");
                    return;
                }
            };

        tokio::select! {
            _ = sigterm.recv() => tracing::info!("received SIGTERM"),
            _ = sigint.recv() => tracing::info!("received SIGINT"),
        }

        shutdown_signals.notify_waiters();
    });

    let accept_state = Arc::clone(&state);
    let accept_shutdown = Arc::clone(&shutdown);
    let accept_loop = async move {
        loop {
            let (stream, _addr) = match listener.accept().await {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!(?e, "accept failed");
                    continue;
                }
            };

            let state = Arc::clone(&accept_state);
            let shutdown = Arc::clone(&accept_shutdown);
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, state, shutdown).await {
                    match e {
                        DaemonError::Io(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => {
                            // Normal client disconnect.
                        }
                        other => {
                            tracing::warn!(?other, "connection ended with error");
                        }
                    }
                }
            });
        }
    };

    tokio::select! {
        _ = accept_loop => {},
        _ = shutdown.notified() => {
            tracing::info!("shutdown notified, draining");
        }
    }

    // Best-effort socket cleanup so a respawn doesn't trip on EADDRINUSE.
    let _ = std::fs::remove_file(&paths.socket);

    Ok(())
}

/// Per-connection task: handshake, then a request/response loop.
async fn handle_connection(
    stream: UnixStream,
    state: Arc<DaemonState>,
    _shutdown: Arc<tokio::sync::Notify>,
) -> Result<()> {
    // Peer-credential check FIRST — before reading any frame. Daemon-owned
    // ~/.zshrs/ is mode 0700, so the socket itself is unreachable from
    // other UIDs unless the directory perms drift. Defense-in-depth: we
    // explicitly verify peer UID and refuse cross-UID connections (unless we
    // ARE root, in which case cross-uid is allowed for fleet-wide
    // coordination — see docs/DAEMON.md "Security model").
    match peer_uid(&stream) {
        Ok(peer) => {
            let our_uid = nix::unistd::Uid::current().as_raw();
            if our_uid != 0 && peer != our_uid {
                tracing::warn!(peer_uid = peer, our_uid, "rejected cross-uid client");
                return Err(DaemonError::other(format!(
                    "peer uid {} != daemon uid {}",
                    peer, our_uid
                )));
            }
        }
        Err(e) => {
            // SO_PEERCRED / getpeereid both supported on every targeted
            // platform. A failure here is a real protocol-level problem —
            // log + close.
            tracing::warn!(?e, "peer-cred lookup failed; refusing connection");
            return Err(DaemonError::other(format!("peer cred: {e}")));
        }
    }

    let (read_half, write_half) = stream.into_split();
    let mut reader = tokio::io::BufReader::new(read_half);
    let mut writer = write_half;

    // ---- Handshake ----
    let first = ipc::read_frame(&mut reader).await?;
    let hello = match first {
        Frame::Hello { hello } => hello,
        _ => {
            let err = Frame::Response {
                id: 0,
                ok: false,
                payload: serde_json::json!({
                    "err": ErrPayload::new("bad_handshake", "expected Hello as first frame")
                }),
            };
            let _ = ipc::write_frame(&mut writer, &err).await;
            return Err(DaemonError::BadHandshake);
        }
    };

    if hello.version != PROTOCOL_VERSION {
        let err = serde_json::json!({
            "welcome": null,
            "err": ErrPayload::new(
                "version_mismatch",
                format!("client v{}, daemon v{}", hello.version, PROTOCOL_VERSION),
            ),
        });
        // Wrap in WelcomeErr variant.
        let frame: Frame = serde_json::from_value(err).map_err(DaemonError::Json)?;
        ipc::write_frame(&mut writer, &frame).await?;
        return Err(DaemonError::ProtocolMismatch {
            client: hello.version,
            daemon: PROTOCOL_VERSION,
        });
    }

    // Outbound channel for this session — tasks use this to push responses + events.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Frame>();

    // Only a Hello carrying `shell_pid` (the shell's `$$`) is a shell: it
    // creates or joins the record keyed by (shell_pid, shell_start_ns).
    // Anything else (the bench, a client that never called
    // `set_shell_identity`) is an ephemeral session, like HTTP.
    let (client_id, session_id) = match hello.shell_pid {
        Some(shell_pid) => state.register_shell_session(
            shell_pid,
            hello.shell_start_ns,
            hello.tty.clone(),
            hello.cwd.clone(),
            hello.argv0.clone(),
            out_tx.clone(),
        ),
        None => state.register_ephemeral_session(
            hello.client_pid,
            hello.tty.clone(),
            hello.argv0.clone(),
            out_tx.clone(),
        ),
    };

    let welcome = Welcome {
        version: PROTOCOL_VERSION,
        client_id,
        shell_id: state.shell_id_of(client_id).unwrap_or(0),
        session_id: session_id.clone(),
        daemon_pid: state.pid,
        daemon_uptime_ms: state.uptime_ms(),
    };

    // Send welcome through the outbound channel for ordering consistency with
    // any subsequent events that might race the welcome.
    if out_tx.send(Frame::welcome(welcome)).is_err() {
        state.unregister_session(client_id);
        return Ok(());
    }

    tracing::info!(
        client_id, pid = hello.client_pid, tty = ?hello.tty, cwd = ?hello.cwd,
        "client registered"
    );

    // Drop the local handshake-helper sender so only the registered (state) and the
    // request-loop's clone remain. Without this, the pump task would never see the
    // channel close because handle_connection would be holding a dangling sender.
    drop(out_tx);

    // ---- Pump task: drains out_rx → writer ----
    let pump = async move {
        while let Some(frame) = out_rx.recv().await {
            if let Err(e) = ipc::write_frame(&mut writer, &frame).await {
                tracing::debug!(?e, "outbound write failed; closing");
                break;
            }
        }
    };

    // ---- Request loop ----
    let req_state = Arc::clone(&state);
    let request_loop = async move {
        loop {
            match ipc::read_frame(&mut reader).await {
                Ok(Frame::Request { id, op, args }) => {
                    let response = ops::dispatch(&req_state, client_id, &op, args).await;
                    let frame = match response {
                        Ok(payload) => Frame::ok_response(id, payload),
                        Err(err) => Frame::err_response(id, err),
                    };
                    // Send response via the per-session channel held by `state`. We
                    // intentionally do NOT keep a local clone in this task — when the
                    // request loop exits and we unregister, the channel closes and the
                    // pump terminates cleanly.
                    if !req_state.send_to(client_id, frame) {
                        break;
                    }
                }
                Ok(other) => {
                    tracing::debug!(?other, "ignoring unexpected post-handshake frame kind");
                }
                Err(DaemonError::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::BrokenPipe
                            | std::io::ErrorKind::ConnectionReset
                    ) =>
                {
                    break;
                }
                Err(e) => {
                    tracing::warn!(?e, "frame read error; closing");
                    break;
                }
            }
        }
        // Unregister immediately on read-side end. This drops the state's outbound
        // channel clone, which lets the pump task drain remaining messages and then
        // exit when the receiver sees the closed channel.
        req_state.unregister_session(client_id);
    };

    tokio::join!(pump, request_loop);

    tracing::info!(client_id, "client unregistered");
    Ok(())
}

/// Recover the peer UID of a Unix-domain-socket connection. Used by the
/// connection handler to enforce same-UID-only access on the daemon socket.
///
/// Linux: `SO_PEERCRED` (struct ucred, 12 bytes: pid, uid, gid).
/// macOS / *BSD: `getpeereid(fd, &uid, &gid)`.
///
/// Returns the raw UID. Errors propagate to the caller, which closes the
/// connection on any failure (defense-in-depth).
fn peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();

    #[cfg(target_os = "linux")]
    {
        use std::mem::MaybeUninit;
        #[repr(C)]
        struct UCred {
            pid: i32,
            uid: u32,
            gid: u32,
        }
        let mut cred = MaybeUninit::<UCred>::uninit();
        let mut len = std::mem::size_of::<UCred>() as libc::socklen_t;
        let r = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                cred.as_mut_ptr() as *mut _,
                &mut len,
            )
        };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        let cred = unsafe { cred.assume_init() };
        Ok(cred.uid)
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    {
        let mut uid: libc::uid_t = 0;
        let mut gid: libc::gid_t = 0;
        let r = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        if r != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(uid)
    }

    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    {
        // Unknown platform: fall back to "trust the directory perms" — the
        // ~/.zshrs/ being 0700 already gates this.
        Ok(nix::unistd::Uid::current().as_raw())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Hello;
    use serde_json::json;

    fn fresh() -> (tempfile::TempDir, Arc<DaemonState>) {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = CachePaths::with_root(tmp.path().join("zshrs"));
        paths.ensure_dirs().unwrap();
        let state = DaemonState::new(paths).unwrap();
        (tmp, state)
    }

    /// Open a connection over a socketpair, send a Hello, return the client
    /// end and the Welcome.
    async fn connect(
        state: &Arc<DaemonState>,
        client_pid: i32,
        shell_pid: Option<i32>,
        shell_start_ns: Option<u64>,
    ) -> (UnixStream, Welcome) {
        let (mut client, server) = UnixStream::pair().unwrap();
        let st = Arc::clone(state);
        tokio::spawn(async move {
            let _ = handle_connection(server, st, Arc::new(tokio::sync::Notify::new())).await;
        });
        let hello = Hello {
            version: PROTOCOL_VERSION,
            client_pid,
            tty: None,
            cwd: None,
            argv0: None,
            shell_pid,
            shell_start_ns,
        };
        ipc::write_frame(&mut client, &Frame::hello(hello)).await.unwrap();
        match ipc::read_frame(&mut client).await.unwrap() {
            Frame::Welcome { welcome } => (client, welcome),
            other => panic!("expected Welcome, got {other:?}"),
        }
    }

    async fn call(client: &mut UnixStream, id: u64, op: &str, args: serde_json::Value) -> serde_json::Value {
        ipc::write_frame(client, &Frame::request(id, op, args)).await.unwrap();
        loop {
            match ipc::read_frame(client).await.unwrap() {
                Frame::Response { id: rid, ok, payload } if rid == id => {
                    assert!(ok, "{op}: {payload}");
                    return payload;
                }
                _ => continue,
            }
        }
    }

    /// A builtin run in a forked subshell connects with its own pid but the
    /// shell's `$$` as `shell_pid`: it must land on the same stable shell id,
    /// and a tag set over one connection must show on a later one.
    #[tokio::test]
    async fn shell_registry_welcome_shell_id_follows_shell_pid() {
        let (_tmp, state) = fresh();
        let shell = std::process::id() as i32;

        let (mut c1, w1) = connect(&state, shell, Some(shell), Some(7)).await;
        call(&mut c1, 1, "tag", json!({ "tags": ["build"] })).await;
        drop(c1);

        let (mut c2, w2) = connect(&state, shell + 100_000, Some(shell), Some(7)).await;
        assert_ne!(w1.client_id, w2.client_id);
        assert_eq!(w1.shell_id, w2.shell_id);
        assert_ne!(w1.shell_id, 0);
        let shells = call(&mut c2, 1, "list_shells", json!({ "tag": "build" })).await;
        assert_eq!(shells["total"].as_u64(), Some(1));
        assert_eq!(shells["shells"][0]["shell_id"].as_u64(), Some(w1.shell_id));
        assert_eq!(shells["shells"][0]["pid"].as_i64(), Some(shell as i64));

        // No shell_pid (a non-shell client): an ephemeral session, no record.
        let (_c3, w3) = connect(&state, shell + 200_000, None, None).await;
        assert_eq!(w3.shell_id, 0);
        assert_eq!(state.snapshot_shells().len(), 1);
    }

    /// A Hello with a known pid but a different start time is a new shell on
    /// a recycled pid: new stable id, and the old record's tags and queue go.
    #[tokio::test]
    async fn shell_registry_recycled_pid_replaces_record() {
        let (_tmp, state) = fresh();
        let pid = std::process::id() as i32;

        let (mut old, w_old) = connect(&state, pid, Some(pid), Some(111)).await;
        call(&mut old, 1, "tag", json!({ "tags": ["stale"] })).await;
        call(
            &mut old,
            2,
            "ask_ask",
            json!({ "kind": "input", "target": { "self": true }, "payload": {} }),
        )
        .await;
        assert_eq!(state.ask_inbox.pending_count(w_old.shell_id), 1);
        drop(old);

        // Same pid, same start: the same shell.
        let (_same, w_same) = connect(&state, pid, Some(pid), Some(111)).await;
        assert_eq!(w_same.shell_id, w_old.shell_id);

        // Same pid, new start: a different process.
        let (mut new, w_new) = connect(&state, pid, Some(pid), Some(222)).await;
        assert_ne!(w_new.shell_id, w_old.shell_id);
        assert_eq!(state.ask_inbox.pending_count(w_old.shell_id), 0);
        let stale = call(&mut new, 1, "list_shells", json!({ "tag": "stale" })).await;
        assert_eq!(stale["total"].as_u64(), Some(0));
        let all = call(&mut new, 2, "list_shells", json!({})).await;
        assert_eq!(all["total"].as_u64(), Some(1));
        assert_eq!(all["shells"][0]["shell_id"].as_u64(), Some(w_new.shell_id));
    }

    /// `zask dismiss` (single and --all) pushes `ask:response` with
    /// cancelled=true to the originator's live connection; with none open,
    /// nothing is delivered.
    #[tokio::test]
    async fn shell_registry_dismiss_notifies_originator() {
        let (_tmp, state) = fresh();
        let asker_pid = std::process::id() as i32;
        let mut target_proc = std::process::Command::new("sleep").arg("30").spawn().unwrap();
        let target_pid = target_proc.id() as i32;

        // The asker keeps this connection open (a script blocked on the answer).
        let (mut asker, _) = connect(&state, asker_pid, Some(asker_pid), Some(1)).await;
        let (mut target, w_target) = connect(&state, target_pid, Some(target_pid), Some(2)).await;
        let ask = |id: u64| {
            json!({
                "kind": "dialog",
                "target": { "shell_id": w_target.shell_id },
                "payload": { "message": format!("q{id}") },
            })
        };
        let r1 = call(&mut asker, 1, "ask_ask", ask(1)).await;
        let rid1 = r1["request_id"].as_str().unwrap().to_string();
        call(&mut asker, 2, "ask_ask", ask(2)).await;
        call(&mut asker, 3, "ask_ask", ask(3)).await;

        async fn next_cancel(c: &mut UnixStream) -> serde_json::Value {
            loop {
                if let Frame::Event { event, payload } = ipc::read_frame(c).await.unwrap() {
                    if event == "ask:response" {
                        return payload;
                    }
                }
            }
        }

        // Single dismiss.
        let d = call(
            &mut target,
            1,
            "ask_dismiss",
            json!({ "request_id": rid1, "reason": "busy" }),
        )
        .await;
        assert_eq!(d["dismissed"].as_u64(), Some(1));
        assert_eq!(d["originators_notified"].as_u64(), Some(1));
        let ev = next_cancel(&mut asker).await;
        assert_eq!(ev["request_id"].as_str(), Some(rid1.as_str()));
        assert_eq!(ev["cancelled"].as_bool(), Some(true));
        assert_eq!(ev["reason"].as_str(), Some("busy"));

        // --all: one cancel per remaining request.
        let d = call(&mut target, 2, "ask_dismiss", json!({ "all": true })).await;
        assert_eq!(d["dismissed"].as_u64(), Some(2));
        assert_eq!(d["originators_notified"].as_u64(), Some(2));
        for _ in 0..2 {
            assert_eq!(next_cancel(&mut asker).await["cancelled"].as_bool(), Some(true));
        }

        // Originator with no open connection: queued fine, no delivery.
        call(&mut asker, 4, "ask_ask", ask(4)).await;
        drop(asker);
        // Wait for the server side to notice the close.
        for _ in 0..200 {
            if state.snapshot_shells().iter().all(|s| s.pid != asker_pid || s.connections == 0) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        let d = call(&mut target, 3, "ask_dismiss", json!({ "all": true })).await;
        assert_eq!(d["dismissed"].as_u64(), Some(1));
        assert_eq!(d["originators_notified"].as_u64(), Some(0));

        let _ = target_proc.kill();
        let _ = target_proc.wait();
    }
}
