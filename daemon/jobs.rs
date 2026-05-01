// jobs.rs — zshrs-daemon job supervisor (the world-first session-persistent
// job runner).
//
// One Supervisor instance lives for daemon lifetime. Clients submit a command
// via the `job_submit` op; the daemon spawns it as a child of the daemon
// process (so it survives the originating shell's exit, in `nohup`-style),
// captures stdout/stderr to per-job files in ~/.cache/zshrs/jobs/, and
// publishes `job:{id}.{stdout,stderr,complete}` pubsub events so subscribers
// (`zjob output --follow`) get streaming live output.
//
// State is mirrored to a `jobs` table in catalog.db so daemon restarts don't
// lose history. Output files persist on disk regardless of daemon state.
//
// Replaces: nohup, disown, setsid, pueue, screen-as-job-runner.

use std::collections::HashMap;
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

/// Public-facing job state, serialized to catalog + IPC responses.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Running,
    Exited(i32),
    Killed(i32), // signal number
    Failed(String),
}

impl JobState {
    pub fn label(&self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Exited(_) => "exited",
            JobState::Killed(_) => "killed",
            JobState::Failed(_) => "failed",
        }
    }

    pub fn exit_code(&self) -> Option<i32> {
        match self {
            JobState::Exited(c) => Some(*c),
            JobState::Killed(s) => Some(128 + s),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self, JobState::Running)
    }
}

/// Public snapshot used by `zjob list` / `zjob status`. Doesn't borrow from
/// the live registry, so it's safe to ship over IPC.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub id: u64,
    pub command: Vec<String>,
    pub cwd: Option<String>,
    pub tags: Vec<String>,
    pub state: String,
    pub exit_code: Option<i32>,
    pub pid: Option<i32>,
    pub started_by_shell: u64,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub output_path: String,
    pub error_path: String,
    pub stdout_bytes: u64,
    pub stderr_bytes: u64,
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
}

impl JobMeta {
    fn snapshot(&self) -> JobSnapshot {
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
        }
    }
}

/// Daemon-wide singleton owning the in-memory job registry. Held inside
/// `DaemonState` as `Arc<Supervisor>`.
pub struct Supervisor {
    inner: Mutex<SupervisorInner>,
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
        cmd.args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
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
        for (k, v) in &env {
            cmd.env(k, v);
        }
        if let Some(d) = &cwd {
            cmd.current_dir(d);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| super::DaemonError::other(format!("spawn `{}`: {}", &command[0], e)))?;

        let pid = child.id().map(|p| p as i32);
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
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
        };

        {
            let mut g = self.inner.lock();
            g.jobs.insert(id, meta);
        }

        // Persist initial row.
        if let Some(state) = self.upgrade_state() {
            let _ = self.persist_initial(&state, id, &command, &cwd, &tags, client_id, started_at, &output_path, &error_path, pid);
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
        let mut file = match tokio::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .await
        {
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

    async fn handle_exit(self: Arc<Self>, id: u64, exit: std::io::Result<std::process::ExitStatus>) {
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
        let Some(state) = self.upgrade_state() else { return };
        let scope = format!("job:{}", id);
        let payload = json!({
            "subscription_id": null,
            "scope": scope,
            "topic": topic_kind,
            "data": data,
        });
        let frame = Frame::event("job", payload);
        // Build a synthetic Scope for the broadcaster; jobs aren't shells but
        // the pubsub engine routes by canonical scope strings, so we need a
        // scope object. Use shell_id = 0 as a reserved sentinel for job scope.
        let job_scope = super::pubsub::Scope {
            shell_id: 0,
            tags: std::iter::once(format!("job:{}", id)).collect(),
            user: None,
        };
        // Fan out via state.publish (matches pattern) — subscribers using
        // `tag:job:{id}.{stdout,stderr,complete}` get the event.
        let _ = state.publish(&job_scope, topic_kind, frame);
    }

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
                state_filter.map_or(true, |s| m.state.label() == s)
                    && tag_filter.map_or(true, |t| m.tags.iter().any(|x| x == t))
            })
            .map(JobMeta::snapshot)
            .collect();
        out.sort_by(|a, b| b.id.cmp(&a.id));
        if let Some(n) = limit {
            out.truncate(n as usize);
        }
        out
    }

    pub fn status(&self, id: u64) -> Option<JobSnapshot> {
        let g = self.inner.lock();
        g.jobs.get(&id).map(JobMeta::snapshot)
    }

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
