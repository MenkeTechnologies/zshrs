// Daemon accept loop + per-connection handler.
//
// Per docs/DAEMON.md "Daemon = sole writer" + "90/10 work split":
//   - tokio UnixListener on ~/.cache/zshrs/daemon.sock
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

    // Spawn the fsnotify watcher task. No paths are registered initially;
    // they're added by the walk-lifecycle evaluator + `fpath_changed` op.
    if let Err(e) = state.fs_watcher.start(Arc::clone(&state)) {
        tracing::warn!(?e, "fsnotify watcher failed to start; running degraded");
    }

    // Spawn the periodic housekeeping ticker (tmp sweep, log size monitor,
    // catalog vacuum, zask timeouts). One minute cadence, weak-ref to state.
    super::ticker::spawn(Arc::clone(&state));

    // Spawn the schedule tick driver (daemon.schedule.* ops). Wakes once
    // a second, dispatches `job_submit` for any due cron / one-shot rows.
    super::schedule::spawn_tick(Arc::clone(&state));

    // HTTP listener (off by default; opt-in via [http].listen in
    // ~/.config/zshrs/daemon.toml). Surfaces the same op set as the
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
    // ~/.cache/zshrs/ is mode 0700, so the socket itself is unreachable from
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

    let (client_id, session_id) = state.register_session(
        hello.client_pid,
        hello.tty.clone(),
        hello.cwd.clone(),
        hello.argv0.clone(),
        out_tx.clone(),
    );

    let welcome = Welcome {
        version: PROTOCOL_VERSION,
        client_id,
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
        // ~/.cache/zshrs/ being 0700 already gates this.
        Ok(nix::unistd::Uid::current().as_raw())
    }
}
