// zjob — client-facing builtin for the daemon's session-persistent job supervisor.
//
// Per docs/DAEMON.md "z* builtin family" + memory cache_architecture_rkyv.md:
// `zjob` is one of three stacked world-firsts on the daemon:
//   (1) shell with dedicated daemon
//   (2) native session-persistent job supervisor   ← this
//   (3) native cross-shell pub/sub + dispatch
//
// Replaces nohup/disown/setsid/pueue/screen-as-job-runner. Jobs survive shell
// exit; output is captured to ~/.zshrs/jobs/{id}.{out,err}; status is
// persisted in catalog.db so even daemon restarts don't lose history.
//
// CLI shape:
//   zjob submit <cmd> [<args>...] [--cwd DIR] [--tag T...] [--env K=V...]
//   zjob list   [--state running|exited|killed|failed] [--tag T] [--limit N]
//   zjob status <id>
//   zjob output <id> [--follow] [--stderr] [--lines N]
//   zjob kill   <id> [--signal NAME]
//   zjob wait   <id> [--timeout SECS]

use serde_json::{json, Value};

use super::client::Client;
use super::paths::CachePaths;
use super::DaemonError;

fn err_exit(msg: &str) -> i32 {
    eprintln!("zshrs: zjob: {}", msg);
    1
}

fn print_pretty(v: &Value) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{}", s),
        Err(_) => println!("{}", v),
    }
}

fn connect() -> Result<Client, ()> {
    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("zshrs: zjob: daemon: {}", e);
            return Err(());
        }
    };
    if let Err(e) = paths.ensure_dirs() {
        eprintln!("zshrs: zjob: daemon: {}", e);
        return Err(());
    }
    Client::connect(&paths).map_err(|e| {
        eprintln!("zshrs: zjob: daemon: {}", e);
    })
}

pub fn zjob(args: &[String]) -> i32 {
    let verb = args.get(1).map(|s| s.as_str()).unwrap_or("");
    let rest: &[String] = if args.len() > 2 { &args[2..] } else { &[] };
    match verb {
        "submit" => submit(rest),
        "list" | "ls" => list(rest),
        "status" => status(rest),
        "output" | "out" => output(rest),
        "kill" => kill(rest),
        "cancel" => cancel(rest),
        "attach" => attach(rest),
        "wait" => wait_for(rest),
        "" | "-h" | "--help" => {
            println!(
                "usage: zjob submit <cmd> [<args>...] [--cwd DIR] [--tag T...] [--env K=V...] [--pty]"
            );
            println!(
                "       zjob list   [--state running|exited|killed|failed] [--tag T] [--limit N]"
            );
            println!("       zjob status <id>");
            println!("       zjob output <id> [--follow] [--stderr] [--lines N]");
            println!("       zjob attach <id>                              # follow stdout+stderr until exit");
            println!("       zjob kill   <id> [--signal NAME]              # immediate signal, configurable");
            println!("       zjob cancel <id> [--grace SECS]                # SIGTERM, wait grace, SIGKILL");
            println!("       zjob wait   <id> [--timeout SECS]");
            0
        }
        other => err_exit(&format!("unknown verb `{}`", other)),
    }
}

fn cancel(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("cancel: missing or non-integer <id>"),
    };
    let mut grace_ms: u64 = 5_000;
    if let Some(idx) = args.iter().position(|a| a == "--grace") {
        match args.get(idx + 1).and_then(|s| s.parse::<u64>().ok()) {
            Some(secs) => grace_ms = secs * 1000,
            None => return err_exit("cancel: --grace requires integer seconds"),
        }
    }
    let payload = json!({ "id": id, "grace_ms": grace_ms });
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    if let Err(e) =
        client.set_read_timeout(Some(std::time::Duration::from_millis(grace_ms + 5_000)))
    {
        return err_exit(&format!("cancel: {}", e));
    }
    match client.call("job_cancel", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("cancel: {}", e)),
    }
}

fn submit(args: &[String]) -> i32 {
    let mut cwd: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut env: Vec<(String, String)> = Vec::new();
    let mut command: Vec<String> = Vec::new();
    let mut pty = false;

    let mut iter = args.iter();
    let mut in_command = false;
    while let Some(a) = iter.next() {
        if in_command {
            command.push(a.clone());
            continue;
        }
        match a.as_str() {
            "--cwd" => match iter.next() {
                Some(d) => cwd = Some(d.clone()),
                None => return err_exit("submit: --cwd requires a directory"),
            },
            "--tag" => match iter.next() {
                Some(t) => tags.push(t.clone()),
                None => return err_exit("submit: --tag requires a name"),
            },
            "--env" => match iter.next() {
                Some(kv) => match kv.split_once('=') {
                    Some((k, v)) => env.push((k.to_string(), v.to_string())),
                    None => return err_exit("submit: --env requires KEY=VALUE"),
                },
                None => return err_exit("submit: --env requires KEY=VALUE"),
            },
            "--pty" => pty = true,
            "--" => {
                in_command = true;
            }
            other if other.starts_with('-') && command.is_empty() => {
                return err_exit(&format!("submit: unknown flag `{}`", other));
            }
            other => {
                in_command = true;
                command.push(other.to_string());
            }
        }
    }

    if command.is_empty() {
        return err_exit("submit: missing <cmd>");
    }

    let env_obj: serde_json::Map<String, Value> = env
        .into_iter()
        .map(|(k, v)| (k, Value::String(v)))
        .collect();

    let payload = json!({
        "command": command,
        "cwd": cwd,
        "tags": tags,
        "env": env_obj,
        "pty": pty,
    });

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_submit", payload) {
        Ok(v) => {
            // Print just the id for easy capture: `id=$(zjob submit ...)`
            if let Some(id) = v.get("job_id").and_then(Value::as_u64) {
                println!("{}", id);
                0
            } else {
                print_pretty(&v);
                0
            }
        }
        Err(e) => err_exit(&format!("submit: {}", e)),
    }
}

fn list(args: &[String]) -> i32 {
    let mut payload = json!({});
    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--state" => match iter.next() {
                Some(s) => payload["state"] = Value::String(s.clone()),
                None => return err_exit("list: --state requires a value"),
            },
            "--tag" => match iter.next() {
                Some(t) => payload["tag"] = Value::String(t.clone()),
                None => return err_exit("list: --tag requires a name"),
            },
            "--limit" => match iter.next() {
                Some(n) => match n.parse::<u64>() {
                    Ok(v) => payload["limit"] = json!(v),
                    Err(_) => return err_exit("list: --limit requires an integer"),
                },
                None => return err_exit("list: --limit requires a value"),
            },
            other if other.starts_with('-') => {
                return err_exit(&format!("list: unknown flag `{}`", other));
            }
            _ => {}
        }
    }

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_list", payload) {
        Ok(v) => {
            print_jobs_table(&v);
            0
        }
        Err(e) => err_exit(&format!("list: {}", e)),
    }
}

fn print_jobs_table(v: &Value) {
    let arr = match v.get("jobs").and_then(Value::as_array) {
        Some(a) => a,
        None => {
            print_pretty(v);
            return;
        }
    };
    if arr.is_empty() {
        println!("(no jobs)");
        return;
    }
    println!(
        "{:<6} {:<10} {:<8} {:<12} {:<10} {}",
        "ID", "STATE", "EXIT", "STARTED", "TAGS", "COMMAND"
    );
    for j in arr {
        let id = j.get("id").and_then(Value::as_u64).unwrap_or(0);
        let state = j.get("state").and_then(Value::as_str).unwrap_or("?");
        let exit = j
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|c| c.to_string())
            .unwrap_or_else(|| "-".to_string());
        let started = j.get("started_at").and_then(Value::as_str).unwrap_or("-");
        let tags = j
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .unwrap_or_default();
        let tags = if tags.is_empty() {
            "-".to_string()
        } else {
            tags
        };
        let cmd = j
            .get("command")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        println!(
            "{:<6} {:<10} {:<8} {:<12} {:<10} {}",
            id, state, exit, started, tags, cmd
        );
    }
}

fn status(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("status: missing or non-integer <id>"),
    };
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_status", json!({ "id": id })) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("status: {}", e)),
    }
}

fn output(args: &[String]) -> i32 {
    let mut id_opt: Option<u64> = None;
    let mut follow = false;
    let mut stderr = false;
    let mut lines: Option<u64> = None;

    let mut iter = args.iter();
    while let Some(a) = iter.next() {
        match a.as_str() {
            "--follow" | "-f" => follow = true,
            "--stderr" => stderr = true,
            "--lines" | "-n" => match iter.next().and_then(|s| s.parse::<u64>().ok()) {
                Some(n) => lines = Some(n),
                None => return err_exit("output: --lines requires an integer"),
            },
            other if other.starts_with('-') => {
                return err_exit(&format!("output: unknown flag `{}`", other));
            }
            other => {
                if let Ok(n) = other.parse::<u64>() {
                    id_opt = Some(n);
                } else {
                    return err_exit("output: missing or non-integer <id>");
                }
            }
        }
    }
    let id = match id_opt {
        Some(n) => n,
        None => return err_exit("output: missing <id>"),
    };

    let mut payload = json!({
        "id": id,
        "stderr": stderr,
    });
    if let Some(n) = lines {
        payload["lines"] = json!(n);
    }
    if follow {
        payload["follow"] = json!(true);
    }

    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };

    if !follow {
        return match client.call("job_output", payload) {
            Ok(v) => {
                if let Some(content) = v.get("content").and_then(Value::as_str) {
                    print!("{}", content);
                } else {
                    print_pretty(&v);
                }
                0
            }
            Err(e) => err_exit(&format!("output: {}", e)),
        };
    }

    // Follow mode: stream the existing content first, then subscribe to job
    // output events for new lines until the job completes.
    match client.call("job_output", payload.clone()) {
        Ok(v) => {
            if let Some(content) = v.get("content").and_then(Value::as_str) {
                print!("{}", content);
            }
        }
        Err(e) => return err_exit(&format!("output: {}", e)),
    }

    let pattern = format!("job:{}.{}", id, if stderr { "stderr" } else { "stdout" });
    let complete_pattern = format!("job:{}.complete", id);
    if let Err(e) = client.call("subscribe", json!({ "pattern": pattern })) {
        return err_exit(&format!("output --follow subscribe: {}", e));
    }
    if let Err(e) = client.call("subscribe", json!({ "pattern": complete_pattern })) {
        return err_exit(&format!("output --follow subscribe complete: {}", e));
    }
    if let Err(e) = client.set_read_timeout(None) {
        return err_exit(&format!("output --follow timeout: {}", e));
    }

    use super::ipc::Frame;
    loop {
        match client.next_frame() {
            Ok(Frame::Event { event, payload }) => {
                let topic = payload.get("topic").and_then(Value::as_str).unwrap_or("");
                if topic.ends_with(".complete") || event == "job_complete" {
                    return 0;
                }
                if let Some(line) = payload.get("data").and_then(Value::as_str) {
                    print!("{}", line);
                }
            }
            Ok(_) => continue,
            Err(DaemonError::Io(e)) if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) => {
                return 0;
            }
            Err(e) => return err_exit(&format!("output --follow: {}", e)),
        }
    }
}

/// `zjob attach <id>` — follow the job's combined stdout+stderr until
/// it exits, then print the final state. Read-only attach: like
/// `tail -f` over both streams interleaved, plus auto-stop when the
/// job terminates.
///
/// Implementation: direct filesystem tail of `~/.zshrs/jobs/{id}.{out,err}`
/// instead of `subscribe job:N.stdout` (which has a race — see
/// `output --follow` above where the subscription registers after
/// the daemon may already have published `job:N.complete`). The
/// daemon writes both files under user-owned 0600 perms, so reading
/// them from the same user's shell is safe.
///
/// ## Bidirectional attach roadmap
///
/// True `screen -r`-style attach — stdin keystrokes from the client
/// flow back to the running job — is intentionally NOT in this
/// function. The path requires:
///
///   1. **Supervisor: pty allocation at submit** (`daemon/jobs.rs`).
///      Today every job spawns with `Stdio::null()` for stdin
///      (jobs.rs:251). Replace with `nix::pty::openpty()` when a
///      new `--pty` submit flag is set. Slave side becomes child's
///      stdin/stdout/stderr via pre_exec dup2; master fd held in
///      JobMeta.
///   2. **Output multiplexing** (`daemon/jobs.rs`).
///      Reader task pumps `master_fd` to both the `.out` file (so
///      non-attached observers can still tail) AND a broadcast
///      channel that attached clients drain.
///   3. **`job_input` op** (`daemon/ops.rs`).
///      Accepts `{id, bytes_b64}`, writes bytes into the master fd.
///      Base64 envelope keeps the JSON IPC framing intact — binary
///      `Frame` variant would be cleaner but touches the protocol
///      fundamentals.
///   4. **`job_resize` op** (`daemon/ops.rs`).
///      Accepts `{id, rows, cols}`, calls TIOCSWINSZ on master.
///      Client pumps SIGWINCH on attach, reads from termios, fires
///      this op.
///   5. **Bidirectional pump in this function**.
///      Detect pty-mode via `job_status.pty == true`. Switch
///      stdin into raw mode (termios cfmakeraw), spawn a stdin
///      reader thread that batches keystrokes and fires job_input,
///      register a subscriber for output frames. SIGWINCH handler
///      fires job_resize. Restore termios on exit.
///
/// Estimated scope: ~400 LOC across 4 files, plus careful testing
/// for echo/cooked/raw mode interactions and process-group
/// signalling. The current read-only attach is the safe slice of
/// that work — it ships value (live tail of long-running jobs)
/// without the protocol surface expansion.
fn attach(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("attach: missing or non-integer <id>"),
    };

    // Single client across the entire attach lifetime — reused for
    // the initial state probe, every status poll, and the final
    // status read. Reconnecting per poll piles up sessions on the
    // daemon side and racks up ~5ms each on the connect handshake.
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };

    // Refuse to attach to an already-finished job — the user almost
    // certainly wants `zjob output <id>` instead, and dumping the
    // entire output here without explicit consent is surprising.
    let snap = match client.call("job_status", json!({"id": id})) {
        Ok(v) => v,
        Err(e) => return err_exit(&format!("attach: {}", e)),
    };
    // `job_status` wraps the snapshot under a `job` key (see
    // op_job_status in daemon/ops.rs). Reach through it for state.
    let job = snap.get("job");
    let state = job
        .and_then(|j| j.get("state"))
        .and_then(Value::as_str)
        .unwrap_or("?");
    if matches!(state, "exited" | "killed" | "failed" | "cancelled") {
        eprintln!(
            "zjob: attach: job {} is {} (use `zjob output {} --lines N` to read its output)",
            id, state, id
        );
        return 1;
    }

    // Pty mode dispatches to the bidirectional pump (raw stdin
    // forwarded as job_input, daemon stdout broadcast decoded back
    // to the user's tty, SIGWINCH propagated as job_resize). Non-pty
    // mode keeps the read-only file-tail path.
    let pty = job
        .and_then(|j| j.get("pty"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if pty {
        return attach_pty(id, client);
    }

    attach_filetail(id, client)
}

fn attach_filetail(id: u64, mut client: Client) -> i32 {
    use std::io::{Read, Write};

    let paths = match CachePaths::resolve() {
        Ok(p) => p,
        Err(e) => return err_exit(&format!("attach: {}", e)),
    };
    let out_path = paths.root.join("jobs").join(format!("{}.out", id));
    let err_path = paths.root.join("jobs").join(format!("{}.err", id));

    // Open the two output files for tailing. Both must exist —
    // supervisor creates them at submit time (jobs.rs:234-243).
    let mut out_file = match std::fs::File::open(&out_path) {
        Ok(f) => f,
        Err(e) => return err_exit(&format!("attach: open {}: {}", out_path.display(), e)),
    };
    let mut err_file = match std::fs::File::open(&err_path) {
        Ok(f) => f,
        Err(e) => return err_exit(&format!("attach: open {}: {}", err_path.display(), e)),
    };

    // Stream from the start — same posture as `tail -f -n +1`. If the
    // job already produced output before we attached, the user sees it.
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    let mut buf = [0u8; 8192];
    let poll_interval = std::time::Duration::from_millis(50);
    let status_check_interval = std::time::Duration::from_millis(250);
    let mut last_status_check = std::time::Instant::now();
    let mut terminal_seen = false;
    let mut final_state: String = "running".to_string();
    let mut final_exit: Option<i64> = None;
    let mut grace_drain_until: Option<std::time::Instant> = None;

    loop {
        // Drain whatever's ready on both files this tick.
        let mut any_bytes = false;
        loop {
            match out_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    any_bytes = true;
                    let _ = stdout.lock().write_all(&buf[..n]);
                }
                Err(_) => break,
            }
        }
        loop {
            match err_file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    any_bytes = true;
                    let _ = stderr.lock().write_all(&buf[..n]);
                }
                Err(_) => break,
            }
        }
        let _ = stdout.lock().flush();
        let _ = stderr.lock().flush();

        // Status check on a slower cadence than the byte-drain so we
        // don't hammer the daemon. After the job goes terminal, give
        // a 500ms grace window to drain any final buffered bytes the
        // OS hasn't flushed to the file yet. Cache the final state
        // here so we don't need a second IPC round-trip after the
        // loop exits.
        if !terminal_seen && last_status_check.elapsed() >= status_check_interval {
            last_status_check = std::time::Instant::now();
            if let Ok(v) = client.call("job_status", json!({"id": id})) {
                let job = v.get("job");
                let st = job
                    .and_then(|j| j.get("state"))
                    .and_then(Value::as_str)
                    .unwrap_or("?");
                if matches!(st, "exited" | "killed" | "failed" | "cancelled") {
                    terminal_seen = true;
                    final_state = st.to_string();
                    final_exit = job
                        .and_then(|j| j.get("exit_code"))
                        .and_then(Value::as_i64);
                    grace_drain_until =
                        Some(std::time::Instant::now() + std::time::Duration::from_millis(500));
                }
            }
        }

        if let Some(deadline) = grace_drain_until {
            if std::time::Instant::now() >= deadline && !any_bytes {
                break;
            }
        }
        if !any_bytes {
            std::thread::sleep(poll_interval);
        }
    }

    let exit_str = final_exit
        .map(|c| c.to_string())
        .unwrap_or_else(|| "-".to_string());
    eprintln!("[zjob {} {}, exit={}]", id, final_state, exit_str);
    final_exit.map(|c| c as i32).unwrap_or(0)
}

/// Bidirectional pty attach. Pipeline:
///
///   stdin (raw)  ->  job_input  (base64 frames per ~16ms batch)
///   job:N.stdout  ->  stdout    (base64-decode, write to user's tty)
///   SIGWINCH      ->  job_resize (TIOCSWINSZ on master)
///   Ctrl-]        ->  detach (clean exit, job keeps running)
///
/// Termios is switched to raw mode for the duration so keystrokes
/// reach the daemon byte-for-byte (no line-buffering, no local
/// echo). Restored on exit via Drop guard.
///
/// Output ordering: stdin pump uses non-blocking reads with a 16ms
/// batching window so a flurry of typed bytes ships as one IPC call
/// instead of one-per-key. Output frames arrive on the broadcast
/// channel and are written to stdout immediately (no batching) so
/// the user sees prompts the moment the daemon delivers them.
fn attach_pty(id: u64, mut client: Client) -> i32 {
    use std::io::Write;
    use std::os::fd::AsRawFd;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    // Subscribe to the job's output broadcast BEFORE doing anything
    // else — race-free against output that the child may emit during
    // our termios setup.
    let pattern_stdout = format!("job:{}.stdout", id);
    let pattern_complete = format!("job:{}.complete", id);
    if let Err(e) = client.call("subscribe", json!({"pattern": &pattern_stdout})) {
        return err_exit(&format!("attach: subscribe stdout: {}", e));
    }
    if let Err(e) = client.call("subscribe", json!({"pattern": &pattern_complete})) {
        return err_exit(&format!("attach: subscribe complete: {}", e));
    }
    if let Err(e) = client.set_read_timeout(None) {
        return err_exit(&format!("attach: set_read_timeout: {}", e));
    }

    // Switch our terminal to raw mode + propagate the initial size.
    let stdin_fd = std::io::stdin().as_raw_fd();
    let saved_termios = match unsafe { read_termios(stdin_fd) } {
        Ok(t) => t,
        Err(e) => return err_exit(&format!("attach: read termios: {e}")),
    };
    if let Err(e) = unsafe { set_raw(stdin_fd) } {
        return err_exit(&format!("attach: set raw: {e}"));
    }
    let _termios_guard = TermiosGuard {
        fd: stdin_fd,
        saved: saved_termios,
    };

    let (rows, cols) = current_winsize(stdin_fd);
    if rows > 0 && cols > 0 {
        let mut sizer = match connect() {
            Ok(c) => c,
            Err(()) => {
                eprintln!("zjob: attach: warn: couldn't open size-update client");
                return 1;
            }
        };
        let _ = sizer.call(
            "job_resize",
            json!({"id": id, "rows": rows, "cols": cols}),
        );
    }
    eprintln!("[zjob {} attached — Ctrl-] to detach]", id);

    // Spawn a stdin reader thread. Reads non-blocking via poll(2),
    // batches up to 16ms or 4KB before firing a job_input IPC. Uses
    // its OWN client connection so the main thread's IPC reader
    // (waiting on event frames) isn't multiplexed with writes.
    let detach_signal = Arc::new(AtomicBool::new(false));
    let stdin_thread = {
        let detach_signal = Arc::clone(&detach_signal);
        std::thread::spawn(move || stdin_pump(id, stdin_fd, detach_signal))
    };

    // Main loop: read event frames from the daemon, decode + write
    // to stdout. Exit on `complete` event OR detach signal from
    // stdin pump.
    use super::ipc::Frame;
    use super::DaemonError;
    let stdout = std::io::stdout();
    let mut final_state = "running".to_string();
    let mut final_exit: Option<i64> = None;
    loop {
        if detach_signal.load(Ordering::SeqCst) {
            eprintln!("\r\n[zjob {} detached]", id);
            // Don't wait on the thread — it owns the detach_signal it
            // just set; it'll exit on its own poll iteration.
            break;
        }
        match client.next_frame() {
            Ok(Frame::Event { event, payload }) => {
                let topic = payload.get("topic").and_then(Value::as_str).unwrap_or("");
                if event == "job" && topic == "stdout" {
                    if let Some(b64) = payload
                        .get("data")
                        .and_then(|d| d.get("bytes_b64"))
                        .and_then(Value::as_str)
                    {
                        if let Ok(bytes) = base64_decode_local(b64) {
                            let _ = stdout.lock().write_all(&bytes);
                            let _ = stdout.lock().flush();
                        }
                    }
                } else if event == "job" && topic == "complete" {
                    final_state = payload
                        .get("data")
                        .and_then(|d| d.get("state"))
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string();
                    final_exit = payload
                        .get("data")
                        .and_then(|d| d.get("exit_code"))
                        .and_then(Value::as_i64);
                    break;
                }
            }
            Ok(_) => continue,
            Err(DaemonError::Io(e)) if matches!(e.kind(), std::io::ErrorKind::UnexpectedEof) => {
                break;
            }
            Err(e) => {
                eprintln!("\r\nzjob: attach: read frame: {e}");
                break;
            }
        }
    }

    // Tell the stdin pump to stop. It checks the flag between poll
    // iterations (poll timeout = 100ms so it converges quickly).
    detach_signal.store(true, Ordering::SeqCst);
    let _ = stdin_thread.join();

    let exit_str = final_exit
        .map(|c| c.to_string())
        .unwrap_or_else(|| "-".to_string());
    eprintln!("[zjob {} {}, exit={}]", id, final_state, exit_str);
    final_exit.map(|c| c as i32).unwrap_or(0)
}

/// Stdin reader loop running on a dedicated thread. poll(2)'s the
/// stdin fd with a 100ms timeout; on each readable cycle, drains
/// available bytes (batched, non-blocking) and ships them as one
/// `job_input` IPC. Detects Ctrl-] (0x1D) as the detach key and
/// flips the shared atomic so the main loop exits cleanly.
fn stdin_pump(
    id: u64,
    stdin_fd: std::os::fd::RawFd,
    detach_signal: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> i32 {
    use std::sync::atomic::Ordering;

    // Dedicated client — see attach_pty's note on why the input pump
    // doesn't share the main loop's reader connection.
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };

    let mut buf = [0u8; 4096];
    let mut pollfd = libc::pollfd {
        fd: stdin_fd,
        events: libc::POLLIN,
        revents: 0,
    };
    loop {
        if detach_signal.load(Ordering::SeqCst) {
            return 0;
        }
        let rc = unsafe { libc::poll(&mut pollfd, 1, 100) };
        if rc < 0 {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return 1;
        }
        if rc == 0 {
            continue; // timeout — re-check the detach signal
        }
        if pollfd.revents & libc::POLLIN == 0 {
            continue;
        }
        let n = unsafe { libc::read(stdin_fd, buf.as_mut_ptr() as *mut _, buf.len()) };
        if n <= 0 {
            return 0;
        }
        let chunk = &buf[..n as usize];
        // Detach-key sniffer: Ctrl-] = 0x1D. If present, fire the
        // detach signal and don't forward the byte to the job (the
        // user almost never wants to send literal Ctrl-] to a job).
        if chunk.contains(&0x1D) {
            detach_signal.store(true, Ordering::SeqCst);
            return 0;
        }
        let b64 = base64_encode_local(chunk);
        if let Err(e) = client.call(
            "job_input",
            json!({"id": id, "bytes_b64": b64}),
        ) {
            eprintln!("\r\nzjob: attach: job_input: {e}");
            return 1;
        }
    }
}

/// Termios accessors. Kept inline (no nix dep) so the call site
/// stays self-contained and readable. SAFETY: stdin_fd must outlive
/// the call (it's process stdin, so always valid).
unsafe fn read_termios(fd: std::os::fd::RawFd) -> std::io::Result<libc::termios> {
    let mut t: libc::termios = std::mem::zeroed();
    if libc::tcgetattr(fd, &mut t) != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(t)
}

unsafe fn set_raw(fd: std::os::fd::RawFd) -> std::io::Result<()> {
    let mut t = read_termios(fd)?;
    libc::cfmakeraw(&mut t);
    if libc::tcsetattr(fd, libc::TCSANOW, &t) != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

fn current_winsize(fd: std::os::fd::RawFd) -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 {
            (ws.ws_row, ws.ws_col)
        } else {
            (24, 80)
        }
    }
}

/// Restores the saved termios on Drop. Without this, a panic / early
/// return inside attach_pty would leave the user's terminal in raw
/// mode — uncomfortable.
struct TermiosGuard {
    fd: std::os::fd::RawFd,
    saved: libc::termios,
}

impl Drop for TermiosGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::tcsetattr(self.fd, libc::TCSANOW, &self.saved);
        }
    }
}

// ---- base64 encode/decode kept local to the builtin so it doesn't
// need a new crate dep. Mirrors daemon/jobs.rs::base64_encode +
// daemon/ops.rs::base64_decode. RFC 4648 alphabet, padded.

const B64_ALPHABET: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode_local(bytes: &[u8]) -> String {
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
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

fn base64_decode_local(s: &str) -> std::result::Result<Vec<u8>, String> {
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

fn kill(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("kill: missing or non-integer <id>"),
    };
    let mut signal: Option<String> = None;
    if let Some(idx) = args.iter().position(|a| a == "--signal") {
        signal = args.get(idx + 1).cloned();
    }
    let payload = match signal {
        Some(s) => json!({ "id": id, "signal": s }),
        None => json!({ "id": id }),
    };
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    match client.call("job_kill", payload) {
        Ok(v) => {
            print_pretty(&v);
            0
        }
        Err(e) => err_exit(&format!("kill: {}", e)),
    }
}

fn wait_for(args: &[String]) -> i32 {
    let id = match args.first().and_then(|s| s.parse::<u64>().ok()) {
        Some(n) => n,
        None => return err_exit("wait: missing or non-integer <id>"),
    };
    let mut payload = json!({ "id": id });
    if let Some(idx) = args.iter().position(|a| a == "--timeout") {
        match args.get(idx + 1).and_then(|s| s.parse::<u64>().ok()) {
            Some(secs) => payload["timeout_ms"] = json!(secs * 1000),
            None => return err_exit("wait: --timeout requires integer seconds"),
        }
    }
    let mut client = match connect() {
        Ok(c) => c,
        Err(()) => return 1,
    };
    // wait can take arbitrarily long — set generous timeout, default 1h cap.
    if let Err(e) = client.set_read_timeout(Some(std::time::Duration::from_secs(3600))) {
        return err_exit(&format!("wait: {}", e));
    }
    match client.call("job_wait", payload) {
        Ok(v) => {
            print_pretty(&v);
            v.get("exit_code")
                .and_then(Value::as_i64)
                .map(|c| c as i32)
                .unwrap_or(0)
        }
        Err(e) => err_exit(&format!("wait: {}", e)),
    }
}
