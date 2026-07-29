// jobs.rs — zshrs-daemon job supervisor (the world-first session-persistent
// job runner).
//
// One Supervisor instance lives for daemon lifetime. Clients submit a command
// via the `job_submit` op; the daemon spawns it as a child of the daemon
// process (so it survives the originating shell's exit, in `nohup`-style),
// captures stdout/stderr to per-job files in ~/.zshrs/jobs/, and
// publishes `job:{id}.{stdout,stderr,complete}` pubsub events so subscribers
// (`zjob output --follow`) get streaming live output.
//
// State is mirrored to a `jobs` table in catalog.db so daemon restarts don't
// lose history. Output files persist on disk regardless of daemon state.
//
// Replaces: nohup, disown, setsid, pueue, screen-as-job-runner.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Weak};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::sync::{mpsc, oneshot};

use super::ipc::Frame;
use super::paths::CachePaths;
use super::state::DaemonState;
use super::Result;

/// Per-job pty state. Present only for jobs spawned with `pty=true`.
/// Master fd is held inside a Mutex so the reader thread + the
/// `job_input` writer + the `job_resize` ioctl all serialize through
/// it. Writer is held cloned via `try_clone_to_owned` from the same
/// underlying device — pty masters can be safely shared across
/// readers/writers (kernel serializes byte ordering per side of the
/// stream).
pub struct PtyHandle {
    /// Master fd. Held in Option so close-on-job-exit can replace
    /// with None to short-circuit late-firing input/resize ops.
    pub master: Mutex<Option<OwnedFd>>,
    /// Last-known winsize, returned in job_status so the client can
    /// initialize its terminal before the first resize event.
    pub winsize: Mutex<(u16, u16)>,
}

impl PtyHandle {
    fn new(master: OwnedFd, rows: u16, cols: u16) -> Arc<Self> {
        Arc::new(Self {
            master: Mutex::new(Some(master)),
            winsize: Mutex::new((rows, cols)),
        })
    }

    /// Write `bytes` to the master fd. Returns the byte count written
    /// (or an io::Error). No-op (returns 0) when master is closed.
    pub fn write(&self, bytes: &[u8]) -> std::io::Result<usize> {
        let g = self.master.lock();
        let fd = match g.as_ref() {
            Some(f) => f,
            None => return Ok(0),
        };
        let raw = fd.as_raw_fd();
        let n = unsafe { libc::write(raw, bytes.as_ptr() as *const _, bytes.len()) };
        if n < 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(n as usize)
        }
    }

    /// Propagate a terminal resize to the child. The kernel notifies
    /// the foreground process group with SIGWINCH so apps like vim /
    /// less re-render at the right size.
    pub fn resize(&self, rows: u16, cols: u16) -> std::io::Result<()> {
        let g = self.master.lock();
        let fd = match g.as_ref() {
            Some(f) => f,
            None => return Ok(()),
        };
        let ws = libc::winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let raw = fd.as_raw_fd();
        let rc = unsafe { libc::ioctl(raw, libc::TIOCSWINSZ, &ws) };
        if rc < 0 {
            return Err(std::io::Error::last_os_error());
        }
        *self.winsize.lock() = (rows, cols);
        Ok(())
    }

    /// Drop the master fd. Called on job exit so file descriptors
    /// don't leak when the supervisor outlives the child by an
    /// unbounded amount of time.
    fn close_master(&self) {
        let _ = self.master.lock().take();
    }
}

/// Public-facing job state, serialized to catalog + IPC responses.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    /// `Running` variant.
    Running,
    /// `Exited` variant.
    Exited(i32),
    /// `Killed` variant.
    Killed(i32), // signal number
    /// `Failed` variant.
    Failed(String),
}

impl JobState {
    /// `label` — see implementation.
    pub fn label(&self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Exited(_) => "exited",
            JobState::Killed(_) => "killed",
            JobState::Failed(_) => "failed",
        }
    }
    /// `exit_code` — see implementation.
    pub fn exit_code(&self) -> Option<i32> {
        match self {
            JobState::Exited(c) => Some(*c),
            JobState::Killed(s) => Some(128 + s),
            _ => None,
        }
    }
    /// `is_terminal` — see implementation.
    pub fn is_terminal(&self) -> bool {
        !matches!(self, JobState::Running)
    }
}

/// Public snapshot used by `zjob list` / `zjob status`. Doesn't borrow from
/// the live registry, so it's safe to ship over IPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSnapshot {
    /// `id` field.
    pub id: u64,
    /// `command` field.
    pub command: Vec<String>,
    /// `cwd` field.
    pub cwd: Option<String>,
    /// `tags` field.
    pub tags: Vec<String>,
    /// `state` field.
    pub state: String,
    /// `exit_code` field.
    pub exit_code: Option<i32>,
    /// `pid` field.
    pub pid: Option<i32>,
    /// `started_by_shell` field.
    pub started_by_shell: u64,
    /// `started_at` field.
    pub started_at: String,
    /// `finished_at` field.
    pub finished_at: Option<String>,
    /// `output_path` field.
    pub output_path: String,
    /// `error_path` field.
    pub error_path: String,
    /// `stdout_bytes` field.
    pub stdout_bytes: u64,
    /// `stderr_bytes` field.
    pub stderr_bytes: u64,
    /// True if this job is running under a pty (submitted with `pty=true`).
    /// Drives `zjob attach`'s mode selection — pty jobs use the
    /// bidirectional raw-mode pump; non-pty jobs use the read-only
    /// file-tail follow.
    #[serde(default)]
    pub pty: bool,
    /// Last-known terminal dimensions, when `pty=true`. Lets `zjob
    /// attach` initialize its termios before the first SIGWINCH.
    #[serde(default)]
    pub pty_rows: Option<u16>,
    /// `pty_cols` field.
    #[serde(default)]
    pub pty_cols: Option<u16>,
}

/// Per-job in-memory record. Output paths live on disk; state is mirrored to
/// catalog.db for crash recovery.
struct JobMeta {
    id: u64,
    command: Vec<String>,
    cwd: Option<String>,
    tags: Vec<String>,
    started_by_shell: u64,
    started_at: chrono::DateTime<chrono::Utc>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    state: JobState,
    pid: Option<i32>,
    output_path: PathBuf,
    error_path: PathBuf,
    stdout_bytes: u64,
    stderr_bytes: u64,
    /// Channel to signal job_wait callers when the job hits a terminal state.
    waiters: Vec<oneshot::Sender<JobState>>,
    /// Pty handle, present iff job spawned with `pty=true`. Holds the
    /// master fd; lets job_input + job_resize ops reach the controlling
    /// terminal of the running child.
    pty: Option<Arc<PtyHandle>>,
}

impl JobMeta {
    fn snapshot(&self) -> JobSnapshot {
        let (pty_rows, pty_cols) = match self.pty.as_ref() {
            Some(h) => {
                let (r, c) = *h.winsize.lock();
                (Some(r), Some(c))
            }
            None => (None, None),
        };
        JobSnapshot {
            id: self.id,
            command: self.command.clone(),
            cwd: self.cwd.clone(),
            tags: self.tags.clone(),
            state: self.state.label().to_string(),
            exit_code: self.state.exit_code(),
            pid: self.pid,
            started_by_shell: self.started_by_shell,
            started_at: self.started_at.to_rfc3339(),
            finished_at: self.finished_at.map(|t| t.to_rfc3339()),
            output_path: self.output_path.display().to_string(),
            error_path: self.error_path.display().to_string(),
            stdout_bytes: self.stdout_bytes,
            stderr_bytes: self.stderr_bytes,
            pty: self.pty.is_some(),
            pty_rows,
            pty_cols,
        }
    }
}

/// Daemon-wide singleton owning the in-memory job registry. Held inside
/// `DaemonState` as `Arc<Supervisor>`.
pub struct Supervisor {
    /// `inner` field.
    inner: Mutex<SupervisorInner>,
    /// `paths` field.
    paths: CachePaths,
    /// Weak back-ref so background supervisor tasks can publish events without
    /// keeping DaemonState alive past its natural lifetime.
    state: parking_lot::RwLock<Weak<DaemonState>>,
}

struct SupervisorInner {
    next_id: u64,
    jobs: HashMap<u64, JobMeta>,
}

impl Supervisor {
    /// `new` — see implementation.
    pub fn new(paths: CachePaths) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(SupervisorInner {
                next_id: 1,
                jobs: HashMap::new(),
            }),
            paths,
            state: parking_lot::RwLock::new(Weak::new()),
        })
    }
    /// `bind_state` — see implementation.
    pub fn bind_state(&self, state: &Arc<DaemonState>) {
        *self.state.write() = Arc::downgrade(state);
    }

    fn upgrade_state(&self) -> Option<Arc<DaemonState>> {
        self.state.read().upgrade()
    }

    fn jobs_dir(&self) -> PathBuf {
        self.paths.root.join("jobs")
    }

    fn ensure_jobs_dir(&self) -> Result<()> {
        let dir = self.jobs_dir();
        std::fs::create_dir_all(&dir)?;
        let mut perms = std::fs::metadata(&dir)?.permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o700);
        let _ = std::fs::set_permissions(&dir, perms);
        Ok(())
    }

    /// Bring up the `jobs` table in catalog.db. Idempotent; safe to call on
    /// every daemon start.
    pub fn ensure_schema(&self, state: &DaemonState) -> Result<()> {
        state.with_catalog(|conn| -> std::result::Result<(), super::DaemonError> {
            conn.execute_batch(
                r#"
                CREATE TABLE IF NOT EXISTS jobs (
                    id              INTEGER PRIMARY KEY,
                    command         TEXT NOT NULL,
                    cwd             TEXT,
                    tags            TEXT,
                    started_by_shell INTEGER NOT NULL,
                    started_at_ns   INTEGER NOT NULL,
                    finished_at_ns  INTEGER,
                    state           TEXT NOT NULL,
                    exit_code       INTEGER,
                    signal          INTEGER,
                    pid             INTEGER,
                    output_path     TEXT NOT NULL,
                    error_path      TEXT NOT NULL,
                    error_msg       TEXT
                );
                CREATE INDEX IF NOT EXISTS jobs_state_idx ON jobs(state);
                CREATE INDEX IF NOT EXISTS jobs_started_idx ON jobs(started_at_ns DESC);
                "#,
            )?;
            Ok(())
        })
    }

    /// Submit a new job. Returns the assigned id once the child is spawned and
    /// supervisor task is running.
    pub fn submit(
        self: &Arc<Self>,
        client_id: u64,
        command: Vec<String>,
        cwd: Option<String>,
        tags: Vec<String>,
        env: HashMap<String, String>,
        pty: bool,
    ) -> Result<u64> {
        if command.is_empty() {
            return Err(super::DaemonError::other("empty command"));
        }
        self.ensure_jobs_dir()?;

        let id = {
            let mut g = self.inner.lock();
            let id = g.next_id;
            g.next_id += 1;
            id
        };

        let output_path = self.jobs_dir().join(format!("{}.out", id));
        let error_path = self.jobs_dir().join(format!("{}.err", id));

        // Open output files now (even before spawn) so the JobMeta record is
        // consistent. tokio::fs::File would be fine but we want sync open here
        // to surface ENOSPC etc. before the child is spawned.
        let stdout_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&output_path)?;
        let stderr_file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&error_path)?;
        let _ = super::paths::ensure_file_600(&output_path);
        let _ = super::paths::ensure_file_600(&error_path);
        drop(stdout_file);
        drop(stderr_file);

        let mut cmd = Command::new(&command[0]);
        cmd.args(&command[1..]).env_clear();

        // Pty branch: openpty, dup slave to child stdin/out/err, hold
        // master in JobMeta. Pipe branch: existing capture-to-disk
        // path, no terminal allocated.
        let pty_handle: Option<Arc<PtyHandle>> = if pty {
            let ws = nix::pty::Winsize {
                ws_row: 24,
                ws_col: 80,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let pair = nix::pty::openpty(Some(&ws), None)
                .map_err(|e| super::DaemonError::other(format!("openpty: {e}")))?;
            let master_owned: OwnedFd = pair.master;
            let slave_owned: OwnedFd = pair.slave;

            // Three Stdio handles for the child — all dup of the same
            // slave fd. The child's pre_exec then makes that fd the
            // controlling terminal for the new session via TIOCSCTTY.
            let slave_in = slave_owned
                .try_clone()
                .map_err(|e| super::DaemonError::other(format!("dup slave: {e}")))?;
            let slave_out = slave_owned
                .try_clone()
                .map_err(|e| super::DaemonError::other(format!("dup slave: {e}")))?;
            let slave_err = slave_owned;

            cmd.stdin(Stdio::from(slave_in))
                .stdout(Stdio::from(slave_out))
                .stderr(Stdio::from(slave_err));

            // Capture master's raw fd so pre_exec can close it in the
            // child (otherwise the master leaks across exec and the
            // EOF semantic on master read breaks — kernel only closes
            // when ALL refs go away).
            let master_raw_for_child = master_owned.as_raw_fd();
            unsafe {
                cmd.pre_exec(move || {
                    // New session, new process group — child becomes
                    // session leader, eligible to acquire a controlling
                    // tty.
                    nix::unistd::setsid().map_err(|e| std::io::Error::other(e.to_string()))?;
                    // Make stdin (the slave fd, dup'd to fd 0 by the
                    // standard library before pre_exec runs) the
                    // controlling tty. zero arg = "I'm willing to steal
                    // it" (only matters if the slave was already someone
                    // else's controlling tty, which it isn't here).
                    if libc::ioctl(0, libc::TIOCSCTTY as _, 0) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    // Close master in the child. Std-lib-spawned children
                    // inherit every parent fd that wasn't FD_CLOEXEC.
                    libc::close(master_raw_for_child);
                    Ok(())
                });
            }

            Some(PtyHandle::new(master_owned, ws.ws_row, ws.ws_col))
        } else {
            cmd.stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            None
        };

        // Preserve a minimal usable env (PATH at least) plus client overrides.
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        if let Ok(home) = std::env::var("HOME") {
            cmd.env("HOME", home);
        }
        if let Ok(lang) = std::env::var("LANG") {
            cmd.env("LANG", lang);
        }
        // Pty-mode child sees TERM=xterm-256color so apps like vim /
        // less / fzf render colors. Client-side `zjob attach` must set
        // its termios to match before the first output frame.
        if pty && !env.contains_key("TERM") {
            cmd.env("TERM", "xterm-256color");
        }
        for (k, v) in &env {
            cmd.env(k, v);
        }
        if let Some(d) = &cwd {
            cmd.current_dir(d);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| super::DaemonError::other(format!("spawn `{}`: {}", command[0], e)))?;

        let pid = child.id().map(|p| p as i32);
        // Pipe-mode jobs hand stdout/stderr to the existing line-buffered
        // drain_stream. Pty-mode jobs have neither (the slave fd is the
        // child's 0/1/2 but the parent only retains the master) — we
        // pump the master fd from a dedicated thread instead.
        let stdout = if pty_handle.is_none() {
            child.stdout.take()
        } else {
            None
        };
        let stderr = if pty_handle.is_none() {
            child.stderr.take()
        } else {
            None
        };
        let started_at = chrono::Utc::now();

        let meta = JobMeta {
            id,
            command: command.clone(),
            cwd: cwd.clone(),
            tags: tags.clone(),
            started_by_shell: client_id,
            started_at,
            finished_at: None,
            state: JobState::Running,
            pid,
            output_path: output_path.clone(),
            error_path: error_path.clone(),
            stdout_bytes: 0,
            stderr_bytes: 0,
            waiters: Vec::new(),
            pty: pty_handle.clone(),
        };

        {
            let mut g = self.inner.lock();
            g.jobs.insert(id, meta);
        }

        // Persist initial row.
        if let Some(state) = self.upgrade_state() {
            let _ = self.persist_initial(
                &state,
                id,
                &command,
                &cwd,
                &tags,
                client_id,
                started_at,
                &output_path,
                &error_path,
                pid,
            );
        }

        // Publish job:{id}.start event.
        self.publish(
            id,
            "start",
            json!({
                "command": command,
                "pid": pid,
                "started_at": started_at.to_rfc3339(),
            }),
        );

        // Spawn output drainers + waiter task.
        if let Some(out) = stdout {
            let supe = Arc::clone(self);
            let path = output_path.clone();
            tokio::spawn(async move {
                supe.drain_stream(id, "stdout", out, path).await;
            });
        }
        if let Some(err) = stderr {
            let supe = Arc::clone(self);
            let path = error_path.clone();
            tokio::spawn(async move {
                supe.drain_stream(id, "stderr", err, path).await;
            });
        }

        // Pty drainer: dedicated OS thread reading raw bytes from the
        // master fd and forwarding them as base64-chunked job:N.stdout
        // events + appending to .out. Thread (not tokio task) because
        // pty master reads are blocking on a char device — putting it
        // on a worker thread keeps the tokio runtime free.
        if let Some(handle) = pty_handle.as_ref() {
            let supe = Arc::clone(self);
            let handle = Arc::clone(handle);
            let path = output_path.clone();
            std::thread::spawn(move || {
                supe.drain_pty(id, handle, path);
            });
        }

        let supe = Arc::clone(self);
        tokio::spawn(async move {
            let exit = child.wait().await;
            supe.handle_exit(id, exit).await;
        });

        Ok(id)
    }

    async fn drain_stream<R: tokio::io::AsyncRead + Unpin>(
        self: Arc<Self>,
        id: u64,
        topic_kind: &'static str,
        reader: R,
        path: PathBuf,
    ) {
        let mut buf_reader = BufReader::new(reader);
        let mut file = match tokio::fs::OpenOptions::new().append(true).open(&path).await {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(?e, %id, kind=topic_kind, "failed to open job output for append");
                return;
            }
        };

        let mut line = String::new();
        loop {
            line.clear();
            match buf_reader.read_line(&mut line).await {
                Ok(0) => break, // EOF
                Ok(n) => {
                    if let Err(e) = file.write_all(line.as_bytes()).await {
                        tracing::warn!(?e, %id, kind=topic_kind, "job output write failed");
                        break;
                    }
                    {
                        let mut g = self.inner.lock();
                        if let Some(m) = g.jobs.get_mut(&id) {
                            if topic_kind == "stdout" {
                                m.stdout_bytes += n as u64;
                            } else {
                                m.stderr_bytes += n as u64;
                            }
                        }
                    }
                    // Publish only when there are subscribers (publish() returns
                    // recipient count; we don't gate here to keep the hot path
                    // simple — if nobody listens, the lookup is cheap).
                    self.publish(
                        id,
                        topic_kind,
                        json!({
                            "data": line,
                            "bytes": n,
                        }),
                    );
                }
                Err(e) => {
                    tracing::warn!(?e, %id, kind=topic_kind, "job output read failed");
                    break;
                }
            }
        }
        let _ = file.flush().await;
    }

    /// Public accessor for the per-job pty handle. Returns None for
    /// non-pty jobs and for terminated jobs whose master fd has
    /// already been dropped via `close_master`. Consumed by the
    /// `job_input` and `job_resize` ops.
    pub fn pty_handle_for(&self, id: u64) -> Option<Arc<PtyHandle>> {
        let g = self.inner.lock();
        g.jobs.get(&id).and_then(|m| m.pty.clone())
    }

    /// Pty drainer: blocking thread that owns the master-fd reader. Pumps
    /// raw bytes (NOT line-buffered — vt100 sequences mid-line need
    /// to flush immediately so terminals like vim render correctly)
    /// to:
    ///   - the .out file on disk (so non-attached observers can tail
    ///     after the fact via `zjob output`)
    ///   - the `job:{id}.stdout` broadcast channel as base64-encoded
    ///     chunks (attached `zjob attach` sessions decode + write to
    ///     the user's terminal)
    ///
    /// Exits when read() returns 0 (EOF — child closed its end after
    /// exit) or any non-EAGAIN error. The master fd ref count drops
    /// in `handle_exit::pty_handle.close_master()`, which races us to
    /// the EOF — either order is correct.
    fn drain_pty(self: Arc<Self>, id: u64, handle: Arc<PtyHandle>, path: PathBuf) {
        // Snapshot the master raw fd while holding the lock briefly,
        // then drop the lock so the writer side (job_input) isn't
        // blocked behind us. Raw fd remains valid for the lifetime
        // of the OwnedFd inside the PtyHandle, which only drops in
        // `close_master` (after the child exits).
        let raw = match handle.master.lock().as_ref() {
            Some(fd) => fd.as_raw_fd(),
            None => return,
        };
        let mut file = match std::fs::OpenOptions::new().append(true).open(&path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(?e, %id, "pty: failed to open .out for append");
                return;
            }
        };
        use std::io::Write;
        let mut buf = [0u8; 4096];
        loop {
            // Borrow check guard: the OwnedFd may have been dropped
            // by handle_exit between iterations. If so, EBADF on read
            // is the kernel's signal to bail.
            let n = unsafe { libc::read(raw, buf.as_mut_ptr() as *mut _, buf.len()) };
            if n == 0 {
                break; // EOF — child closed slave or master was dropped
            }
            if n < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                // EIO is the typical signal once the slave side has
                // closed (Linux quirk for pty masters). Treat as EOF.
                if err.raw_os_error() == Some(libc::EIO) || err.raw_os_error() == Some(libc::EBADF)
                {
                    break;
                }
                tracing::warn!(?err, %id, "pty: master read failed");
                break;
            }
            let chunk = &buf[..n as usize];
            // Tee to disk: append-write, best-effort flush. A failure
            // here doesn't kill the drainer — output to attached
            // clients still flows.
            if let Err(e) = file.write_all(chunk) {
                tracing::warn!(?e, %id, "pty: .out write failed");
            }
            // Broadcast as base64 so the JSON IPC framing carries
            // arbitrary bytes (vt100 escape sequences include 0x1B
            // ESC + arbitrary high-bit bytes that don't survive raw
            // string serialization).
            let b64 = base64_encode(chunk);
            self.publish(
                id,
                "stdout",
                json!({
                    "bytes_b64": b64,
                    "len": chunk.len(),
                }),
            );
        }
        let _ = file.flush();
    }

    async fn handle_exit(
        self: Arc<Self>,
        id: u64,
        exit: std::io::Result<std::process::ExitStatus>,
    ) {
        use std::os::unix::process::ExitStatusExt;

        let final_state = match exit {
            Ok(status) => {
                if let Some(code) = status.code() {
                    JobState::Exited(code)
                } else if let Some(sig) = status.signal() {
                    JobState::Killed(sig)
                } else {
                    JobState::Exited(-1)
                }
            }
            Err(e) => JobState::Failed(e.to_string()),
        };

        let finished_at = chrono::Utc::now();
        let mut waiters: Vec<oneshot::Sender<JobState>> = Vec::new();
        let pty_handle: Option<Arc<PtyHandle>> = {
            let mut g = self.inner.lock();
            g.jobs.get_mut(&id).and_then(|m| m.pty.clone())
        };
        // Drop master fd so the pty drainer thread sees EOF and exits.
        // Also stops late-firing job_input/job_resize ops from racing
        // with the child's exit.
        if let Some(h) = &pty_handle {
            h.close_master();
        }
        let snap = {
            let mut g = self.inner.lock();
            if let Some(m) = g.jobs.get_mut(&id) {
                m.state = final_state.clone();
                m.finished_at = Some(finished_at);
                std::mem::swap(&mut m.waiters, &mut waiters);
                Some(m.snapshot())
            } else {
                None
            }
        };

        for tx in waiters {
            let _ = tx.send(final_state.clone());
        }

        if let (Some(state), Some(snap)) = (self.upgrade_state(), snap) {
            let _ = self.persist_terminal(&state, id, &final_state, finished_at);
            tracing::info!(%id, state = %final_state.label(), "job finished");
            let _ = snap;
        }

        self.publish(
            id,
            "complete",
            json!({
                "state": final_state.label(),
                "exit_code": final_state.exit_code(),
                "finished_at": finished_at.to_rfc3339(),
            }),
        );
    }

    fn publish(&self, id: u64, topic_kind: &str, data: Value) {
        let Some(state) = self.upgrade_state() else {
            return;
        };
        let scope = format!("job:{}", id);
        let payload = json!({
            "subscription_id": null,
            "scope": scope,
            "topic": topic_kind,
            "data": data,
        });
        let frame = Frame::event("job", payload);
        let job_scope = super::pubsub::Scope::for_job(id);
        // Subscribers using `job:{id}.stdout` / `.stderr` / `.complete`
        // (or `job:*.complete`) match via Scope::matches_scope.
        let _ = state.publish(&job_scope, topic_kind, frame);
    }
    /// `list` — see implementation.
    pub fn list(
        &self,
        state_filter: Option<&str>,
        tag_filter: Option<&str>,
        limit: Option<u64>,
    ) -> Vec<JobSnapshot> {
        let g = self.inner.lock();
        let mut out: Vec<JobSnapshot> = g
            .jobs
            .values()
            .filter(|m| {
                state_filter.is_none_or(|s| m.state.label() == s)
                    && tag_filter.is_none_or(|t| m.tags.iter().any(|x| x == t))
            })
            .map(JobMeta::snapshot)
            .collect();
        out.sort_by_key(|b| std::cmp::Reverse(b.id));
        if let Some(n) = limit {
            out.truncate(n as usize);
        }
        out
    }
    /// `status` — see implementation.
    pub fn status(&self, id: u64) -> Option<JobSnapshot> {
        let g = self.inner.lock();
        g.jobs.get(&id).map(JobMeta::snapshot)
    }
    /// `output` — see implementation.
    pub fn output(&self, id: u64, stderr: bool, lines: Option<u64>) -> Result<String> {
        let path = {
            let g = self.inner.lock();
            let m = g
                .jobs
                .get(&id)
                .ok_or_else(|| super::DaemonError::other(format!("job {} not found", id)))?;
            if stderr {
                m.error_path.clone()
            } else {
                m.output_path.clone()
            }
        };
        let content = std::fs::read_to_string(&path)?;
        if let Some(n) = lines {
            let take = content.lines().rev().take(n as usize).collect::<Vec<_>>();
            let mut out = take.into_iter().rev().collect::<Vec<_>>().join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            Ok(out)
        } else {
            Ok(content)
        }
    }
    /// `kill` — see implementation.
    pub fn kill(&self, id: u64, signal: Option<&str>) -> Result<bool> {
        let pid = {
            let g = self.inner.lock();
            let m = g
                .jobs
                .get(&id)
                .ok_or_else(|| super::DaemonError::other(format!("job {} not found", id)))?;
            if m.state.is_terminal() {
                return Ok(false);
            }
            m.pid
                .ok_or_else(|| super::DaemonError::other(format!("job {} has no pid", id)))?
        };
        let sig = parse_signal(signal.unwrap_or("TERM"))?;
        nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), sig)
            .map_err(super::DaemonError::Nix)?;
        Ok(true)
    }

    /// Graceful cancel: SIGTERM, wait for grace period, SIGKILL if still running.
    /// Returns the final JobState. Async because we wait for the terminal-state
    /// channel rather than busy-poll the job map.
    pub async fn cancel(self: &Arc<Self>, id: u64, grace: std::time::Duration) -> Result<JobState> {
        let already = {
            let g = self.inner.lock();
            let m = g
                .jobs
                .get(&id)
                .ok_or_else(|| super::DaemonError::other(format!("job {} not found", id)))?;
            if m.state.is_terminal() {
                Some(m.state.clone())
            } else {
                None
            }
        };
        if let Some(s) = already {
            return Ok(s);
        }

        let rx = self.wait_handle(id)?;
        let _ = self.kill(id, Some("TERM"));
        match tokio::time::timeout(grace, rx).await {
            Ok(Ok(state)) => Ok(state),
            _ => {
                // Still alive after grace period — SIGKILL and wait again.
                let rx2 = self.wait_handle(id)?;
                let _ = self.kill(id, Some("KILL"));
                Ok(rx2
                    .await
                    .unwrap_or(JobState::Failed("KILL didn't reap".into())))
            }
        }
    }

    /// Async wait for a job to enter a terminal state. If the job is already
    /// terminal, returns immediately.
    pub fn wait_handle(&self, id: u64) -> Result<oneshot::Receiver<JobState>> {
        let (tx, rx) = oneshot::channel();
        let mut g = self.inner.lock();
        let m = g
            .jobs
            .get_mut(&id)
            .ok_or_else(|| super::DaemonError::other(format!("job {} not found", id)))?;
        if m.state.is_terminal() {
            let _ = tx.send(m.state.clone());
        } else {
            m.waiters.push(tx);
        }
        Ok(rx)
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_initial(
        &self,
        state: &DaemonState,
        id: u64,
        command: &[String],
        cwd: &Option<String>,
        tags: &[String],
        client_id: u64,
        started_at: chrono::DateTime<chrono::Utc>,
        output_path: &Path,
        error_path: &Path,
        pid: Option<i32>,
    ) -> Result<()> {
        state.with_catalog(|conn| -> std::result::Result<(), super::DaemonError> {
            let cmd_json = serde_json::to_string(command).unwrap_or_default();
            let tags_json = serde_json::to_string(tags).unwrap_or_default();
            let started_ns = started_at.timestamp_nanos_opt().unwrap_or(0);
            conn.execute(
                "INSERT OR REPLACE INTO jobs \
                 (id, command, cwd, tags, started_by_shell, started_at_ns, state, pid, output_path, error_path) \
                 VALUES (?, ?, ?, ?, ?, ?, 'running', ?, ?, ?)",
                rusqlite::params![
                    id as i64,
                    cmd_json,
                    cwd,
                    tags_json,
                    client_id as i64,
                    started_ns,
                    pid,
                    output_path.display().to_string(),
                    error_path.display().to_string(),
                ],
            )?;
            Ok(())
        })
    }

    fn persist_terminal(
        &self,
        state: &DaemonState,
        id: u64,
        final_state: &JobState,
        finished_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        state.with_catalog(|conn| -> std::result::Result<(), super::DaemonError> {
            let finished_ns = finished_at.timestamp_nanos_opt().unwrap_or(0);
            let (label, exit_code, signal, error_msg): (&str, Option<i32>, Option<i32>, Option<String>) =
                match final_state {
                    JobState::Running => ("running", None, None, None),
                    JobState::Exited(c) => ("exited", Some(*c), None, None),
                    JobState::Killed(s) => ("killed", None, Some(*s), None),
                    JobState::Failed(e) => ("failed", None, None, Some(e.clone())),
                };
            conn.execute(
                "UPDATE jobs SET state = ?, exit_code = ?, signal = ?, error_msg = ?, finished_at_ns = ? WHERE id = ?",
                rusqlite::params![label, exit_code, signal, error_msg, finished_ns, id as i64],
            )?;
            Ok(())
        })
    }
}

fn parse_signal(name: &str) -> Result<nix::sys::signal::Signal> {
    use nix::sys::signal::Signal;
    let s = name.trim().trim_start_matches("SIG").to_uppercase();
    let sig = match s.as_str() {
        "TERM" => Signal::SIGTERM,
        "KILL" => Signal::SIGKILL,
        "INT" => Signal::SIGINT,
        "QUIT" => Signal::SIGQUIT,
        "HUP" => Signal::SIGHUP,
        "STOP" => Signal::SIGSTOP,
        "CONT" => Signal::SIGCONT,
        "USR1" => Signal::SIGUSR1,
        "USR2" => Signal::SIGUSR2,
        other => {
            return Err(super::DaemonError::other(format!(
                "unknown signal `{}`",
                other
            )))
        }
    };
    Ok(sig)
}

// ---- Wait helper used by the IPC op (handles the timeout overlay) ----

/// Wait for a job to terminate, with optional timeout. Returns the final state
/// or `None` on timeout.
pub async fn wait_with_timeout(
    rx: oneshot::Receiver<JobState>,
    timeout: Option<std::time::Duration>,
) -> Option<JobState> {
    match timeout {
        Some(t) => match tokio::time::timeout(t, rx).await {
            Ok(Ok(state)) => Some(state),
            _ => None,
        },
        None => rx.await.ok(),
    }
}

// Channel placeholder to keep mpsc::Sender symbol referenced (some configs strip
// unused types; we use mpsc indirectly through tokio::sync re-exports above).
#[allow(dead_code)]
fn _keep_mpsc_alive() -> Option<mpsc::Sender<()>> {
    None
}

// Local base64 encoder so we don't pull a new crate just for the pty
// drainer. Same alphabet as RFC 4648; padded with `=`. Used to wrap
// raw pty bytes (vt100 escape sequences) in JSON IPC payloads. Decoder
// lives in daemon/zd_dispatch.rs.
const B64_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        out.push(B64_ALPHABET[(b0 >> 2) as usize] as char);
        out.push(B64_ALPHABET[(((b0 & 0b11) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            out.push(B64_ALPHABET[(((b1 & 0b1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(B64_ALPHABET[(b2 & 0b111111) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}
