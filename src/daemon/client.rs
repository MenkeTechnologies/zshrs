// Client-side IPC helpers used by z* builtins (which run in the synchronous shell process,
// not the async daemon). All operations: connect → handshake → request/response → close.
//
// Per docs/DAEMON.md "Daemon lifecycle":
//   - first client checks for daemon.sock; if absent or unresponsive, fork-spawns
//     `zshrs --daemon` then retries connect
//   - clients are read-only data-plane consumers; here we only handle the control-plane
//     IPC (op requests + responses)

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::ipc::{self, ErrPayload, Frame, Hello, Welcome, PROTOCOL_VERSION};
use super::paths::CachePaths;
use super::pidlock;
use super::{DaemonError, Result};

const CONNECT_RETRY_DELAY: Duration = Duration::from_millis(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const SPAWN_GRACE: Duration = Duration::from_millis(500);

/// A live connection to the daemon, post-handshake.
pub struct Client {
    stream: UnixStream,
    pub welcome: Welcome,
    next_id: u64,
}

impl Client {
    /// Connect to the daemon, spawn-on-demand if necessary, complete the handshake.
    pub fn connect(paths: &CachePaths) -> Result<Self> {
        match connect_existing(paths) {
            Ok(stream) => Self::handshake(stream),
            Err(_) => {
                spawn_daemon(paths)?;
                let stream = wait_for_socket(paths, SPAWN_GRACE * 4)?;
                Self::handshake(stream)
            }
        }
    }

    /// Connect to an already-running daemon. No spawn-on-demand.
    pub fn connect_existing(paths: &CachePaths) -> Result<Self> {
        let stream = connect_existing(paths)?;
        Self::handshake(stream)
    }

    fn handshake(mut stream: UnixStream) -> Result<Self> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
        stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;

        let hello = Hello {
            version: PROTOCOL_VERSION,
            client_pid: std::process::id() as i32,
            tty: tty_name(),
            cwd: std::env::current_dir().ok().map(|p| p.display().to_string()),
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
            Frame::Response { ok: false, payload, .. } => {
                let err: ErrPayload = serde_json::from_value(
                    payload.get("err").cloned().unwrap_or(Value::Null),
                )
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
                Frame::Response { id: rid, ok, payload } if rid == id => {
                    if ok {
                        return Ok(payload);
                    } else {
                        let err: ErrPayload = serde_json::from_value(
                            payload.get("err").cloned().unwrap_or(Value::Null),
                        )
                        .unwrap_or_else(|_| {
                            ErrPayload::new("unknown", "unparseable error payload")
                        });
                        return Err(DaemonError::other(format!(
                            "{} ({})",
                            err.msg, err.code
                        )));
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

/// Spawn `zshrs --daemon` as a detached process so we can reconnect once it binds.
fn spawn_daemon(paths: &CachePaths) -> Result<()> {
    // Don't spawn if a live daemon process is already in pidfile (race-narrow case).
    if let Some(pid) = pidlock::read_pid(paths) {
        if pidlock::pid_alive(pid) {
            return Ok(());
        }
    }

    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("--daemon");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::null());
    // Detach: a quick double-fork via the runtime would be cleaner, but for v1 we
    // rely on the daemon process not inheriting the controlling terminal because
    // its fds are all /dev/null and the parent doesn't wait().
    cmd.spawn()?;

    Ok(())
}

fn wait_for_socket(paths: &CachePaths, max_wait: Duration) -> Result<UnixStream> {
    let start = Instant::now();
    while start.elapsed() < max_wait {
        if paths.socket.exists() {
            if let Ok(s) = UnixStream::connect(&paths.socket) {
                return Ok(s);
            }
        }
        std::thread::sleep(CONNECT_RETRY_DELAY);
    }
    Err(DaemonError::Timeout(max_wait))
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
pub fn call_once(op: &str, args: Value) -> Result<Value> {
    let paths = CachePaths::resolve()?;
    let mut client = Client::connect(&paths)?;
    client.call(op, args)
}

/// Convenience: connect to existing daemon only (no spawn), run one op.
pub fn call_once_no_spawn(op: &str, args: Value) -> Result<Value> {
    let paths = CachePaths::resolve()?;
    let mut client = Client::connect_existing(&paths)?;
    client.call(op, args)
}

#[cfg(test)]
fn _path_unused(_: &Path) {} // silence unused import in restricted feature builds
