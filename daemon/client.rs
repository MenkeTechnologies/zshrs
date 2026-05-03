// Client-side IPC helpers used by z* builtins (which run in the synchronous shell process,
// not the async daemon). All operations: connect → handshake → request/response → close.
//
// **Connect-only model.** This client never spawns the daemon. The
// daemon is a standalone binary (`zshrs-daemon`) started independently
// by the user via systemd, launchd, brew services, or manually.
// Callers must handle "daemon not running" as a normal degraded-mode
// condition — see `Client::is_daemon_alive` for a cheap probe.

use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::Value;

use super::ipc::{self, ErrPayload, Frame, Hello, Welcome, PROTOCOL_VERSION};
use super::paths::CachePaths;
use super::{DaemonError, Result};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// A live connection to the daemon, post-handshake.
pub struct Client {
    stream: UnixStream,
    pub welcome: Welcome,
    next_id: u64,
}

impl Client {
    /// Connect to an already-running daemon and complete the handshake.
    /// Errors with `DaemonError::NotConnected` (or transport error) if
    /// the daemon isn't running. Callers should treat that as the
    /// normal degraded-mode signal.
    pub fn connect(paths: &CachePaths) -> Result<Self> {
        Self::connect_existing(paths)
    }

    /// Same as `connect` — kept as the explicit-no-spawn name so older
    /// call sites keep compiling.
    pub fn connect_existing(paths: &CachePaths) -> Result<Self> {
        let stream = connect_existing(paths)?;
        Self::handshake(stream)
    }

    /// Cheap "is the daemon alive?" probe. Connects to the socket but
    /// does NOT complete the handshake — saves a roundtrip per startup
    /// when the answer is just "is anything listening?".
    pub fn is_daemon_alive(paths: &CachePaths) -> bool {
        if !paths.socket.exists() {
            return false;
        }
        UnixStream::connect(&paths.socket).is_ok()
    }

    fn handshake(mut stream: UnixStream) -> Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;

        let hello = Hello {
            version: PROTOCOL_VERSION,
            client_pid: std::process::id() as i32,
            tty: tty_name(),
            cwd: std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string()),
            argv0: std::env::args().next(),
        };
        ipc::write_frame_sync(&mut stream, &Frame::hello(hello))?;

        let frame = ipc::read_frame_sync(&mut stream)?;
        let welcome = match frame {
            Frame::Welcome { welcome } => welcome,
            Frame::WelcomeErr { err, .. } => {
                return Err(DaemonError::other(format!(
                    "welcome rejected: {} ({})",
                    err.msg, err.code
                )));
            }
            Frame::Response {
                ok: false, payload, ..
            } => {
                let err: ErrPayload =
                    serde_json::from_value(payload.get("err").cloned().unwrap_or(Value::Null))
                        .unwrap_or_else(|_| ErrPayload::new("unknown", "unparseable error"));
                return Err(DaemonError::other(format!(
                    "handshake failed: {} ({})",
                    err.msg, err.code
                )));
            }
            other => {
                return Err(DaemonError::other(format!(
                    "expected Welcome, got {:?}",
                    other
                )));
            }
        };

        Ok(Self {
            stream,
            welcome,
            next_id: 1,
        })
    }

    /// Set the read timeout on the underlying socket. Pass `None` to block indefinitely
    /// (used by streaming consumers like `zsubscribe` and `zjob output --follow`).
    pub fn set_read_timeout(&mut self, dur: Option<Duration>) -> Result<()> {
        self.stream.set_read_timeout(dur)?;
        Ok(())
    }

    /// Read the next frame off the socket (response or async event). Returns
    /// `DaemonError::Timeout(_)` on read-timeout (distinguishable from EOF, which
    /// surfaces as a `std::io::ErrorKind::UnexpectedEof`). Used by streaming subscribers.
    pub fn next_frame(&mut self) -> Result<Frame> {
        match ipc::read_frame_sync(&mut self.stream) {
            Ok(f) => Ok(f),
            Err(DaemonError::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Err(DaemonError::Timeout(Duration::ZERO))
            }
            Err(e) => Err(e),
        }
    }

    /// Mint a request id without sending a frame. Useful when callers want to
    /// correlate a response that they will read via `next_frame()` directly
    /// (the streaming consumers do their own demux).
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Write a request frame on the wire and return its id. Pair with `next_frame`
    /// when the caller wants to interleave events and the response.
    pub fn send_request(&mut self, op: &str, args: Value) -> Result<u64> {
        let id = self.alloc_id();
        ipc::write_frame_sync(&mut self.stream, &Frame::request(id, op, args))?;
        Ok(id)
    }

    /// Send a request and block until the matching response arrives.
    /// Async events received in between are dropped (this client is for one-shot calls
    /// from sync builtins).
    pub fn call(&mut self, op: &str, args: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;

        ipc::write_frame_sync(&mut self.stream, &Frame::request(id, op, args))?;

        loop {
            let frame = ipc::read_frame_sync(&mut self.stream)?;
            match frame {
                Frame::Response {
                    id: rid,
                    ok,
                    payload,
                } if rid == id => {
                    if ok {
                        return Ok(payload);
                    } else {
                        let err: ErrPayload = serde_json::from_value(
                            payload.get("err").cloned().unwrap_or(Value::Null),
                        )
                        .unwrap_or_else(|_| {
                            ErrPayload::new("unknown", "unparseable error payload")
                        });
                        return Err(DaemonError::other(format!("{} ({})", err.msg, err.code)));
                    }
                }
                Frame::Event { .. } => {
                    // Drop events on this sync path. A future async client will demux.
                    continue;
                }
                Frame::Response { id: rid, .. } => {
                    tracing::debug!(expected = id, got = rid, "stale response id, dropping");
                    continue;
                }
                other => {
                    tracing::debug!(?other, "unexpected frame on sync call, dropping");
                    continue;
                }
            }
        }
    }
}

fn connect_existing(paths: &CachePaths) -> Result<UnixStream> {
    if !paths.socket.exists() {
        return Err(DaemonError::NotConnected);
    }
    let stream = UnixStream::connect(&paths.socket)?;
    Ok(stream)
}

fn tty_name() -> Option<String> {
    use std::os::unix::io::AsRawFd;
    let stdin = std::io::stdin();
    let fd = stdin.as_raw_fd();
    if !atty::is(atty::Stream::Stdin) {
        return None;
    }
    // SAFETY: `ttyname_r` is POSIX-standard.
    let mut buf = vec![0i8; 256];
    let res = unsafe { libc::ttyname_r(fd, buf.as_mut_ptr() as *mut _, buf.len()) };
    if res != 0 {
        return None;
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const _) };
    cstr.to_str().ok().map(str::to_string)
}

/// Convenience: connect, run one op, return the response payload.
/// Errors with `DaemonError::NotConnected` if the daemon isn't running.
pub fn call_once(op: &str, args: Value) -> Result<Value> {
    let paths = CachePaths::resolve()?;
    let mut client = Client::connect(&paths)?;
    client.call(op, args)
}

/// Alias for `call_once` — kept for older call sites that explicitly
/// requested no-spawn semantics. The base `call_once` is now also
/// connect-only, so the two are equivalent.
pub fn call_once_no_spawn(op: &str, args: Value) -> Result<Value> {
    call_once(op, args)
}
