//! DAP server for zshrs — `zshrs --dap [HOST:PORT]` (TCP connect-back, or stdio
//! when no address is given).
//!
//! Mirrors strykelang's DAP architecture (`strykelang/dap.rs`):
//! connect TCP → spawn reader thread → wait for `launch` → run the
//! script IN-PROCESS so per-statement breakpoint checks have a live
//! interpreter to pause. The compiler emits `BUILTIN_SET_LINENO` at
//! every top-level statement; that builtin's handler in
//! `fusevm_bridge.rs` calls [`check_line`] which consults the
//! private `DAP_SHARED` static for matching breakpoints and
//! condvar-waits in [`DapShared::pause`].
//!
//! NOT subprocess-based — earlier v1 spawned the script as a child,
//! which made it impossible to honor breakpoints (no IPC channel to
//! pause the child). The new model keeps the script in-process so the
//! cv-wait literally blocks the executor thread until the IDE sends
//! `continue`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread;
use std::time::Duration;

// ── Pause-and-resume shared state ───────────────────────────────────────

/// What the IDE asked the paused executor to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugAction {
    /// Resume normal execution.
    Continue,
    /// Step over the current statement (treat as Continue + pause at
    /// the next `check_line` call). v1 simplification — Continue is
    /// the same as StepOver in absence of frame depth tracking.
    StepOver,
    /// Step in — same simplification as StepOver in v1.
    StepIn,
    /// Step out — same simplification as Continue in v1.
    StepOut,
    /// Client disconnected — bail out of the script.
    Quit,
}

/// Snapshot captured when the executor pauses. Sent back to the IDE
/// as part of the `stopped` event body.
#[derive(Debug, Clone, Default)]
pub struct PauseSnapshot {
    pub reason: String, // "breakpoint" | "step" | "pause" | "entry"
    /// `file` field.
    pub file: String,
    /// `line` field.
    pub line: u32,
}

/// Breakpoint state shared between the reader thread (which updates
/// it on `setBreakpoints`) and the executor thread (which consults
/// it on every `check_line`).
#[derive(Debug, Default)]
pub struct BreakpointState {
    /// Absolute file path → set of 1-based line numbers.
    pub line_breakpoints: HashMap<String, Vec<u32>>,
}

struct DapSharedInner {
    pending_action: Option<DebugAction>,
    is_paused: bool,
    pause_request: bool, // client asked us to pause asap (via `pause`)
    step_mode: bool,     // true = pause at every check_line (StepOver/In)
}

/// Reader thread + executor thread coordinate through this. Owns the
/// TCP writer, the request seq counter, the pause-cv, and the
/// disconnected flag.
pub struct DapShared {
    /// `inner` field.
    inner: Mutex<DapSharedInner>,
    /// `cv` field.
    cv: Condvar,
    /// `seq` field.
    seq: AtomicU64,
    /// `writer` field — the DAP output sink (a TCP socket in connect-back mode,
    /// stdout in stdio mode).
    writer: Mutex<Box<dyn Write + Send>>,
    /// `configuration_done` field.
    pub configuration_done: AtomicBool,
    /// `disconnected` field.
    pub disconnected: AtomicBool,
    /// Absolute path of the launched program — set on `launch`, read
    /// by `check_line` to know which file's breakpoint set to check.
    pub program: Mutex<String>,
}

impl DapShared {
    fn new(writer: Box<dyn Write + Send>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(DapSharedInner {
                pending_action: None,
                is_paused: false,
                pause_request: false,
                step_mode: false,
            }),
            cv: Condvar::new(),
            seq: AtomicU64::new(1),
            writer: Mutex::new(writer),
            configuration_done: AtomicBool::new(false),
            disconnected: AtomicBool::new(false),
            program: Mutex::new(String::new()),
        })
    }

    /// Called by the executor thread when a `check_line` matches a
    /// breakpoint (or step mode is on). Captures the snapshot, emits
    /// a `stopped` event, then condvar-waits for the IDE to send
    /// `continue` / `next` / `stepIn` / `stepOut`.
    pub fn pause(&self, snap: PauseSnapshot) -> DebugAction {
        // Flush stdout/stderr so any `echo` / `print` output the user
        // produced before this breakpoint is visible in the IDE
        // Console BEFORE the suspend UI shows.
        let _ = io::Write::flush(&mut io::stdout());
        let _ = io::Write::flush(&mut io::stderr());
        tracing::info!(
            target: "zshrs::dap::pause",
            reason = %snap.reason,
            file = %snap.file,
            line = snap.line,
            "executor PAUSED (cv-wait)",
        );
        {
            let mut s = self.inner.lock().expect("dap lock");
            s.is_paused = true;
            s.pending_action = None;
            s.pause_request = false;
        }
        let _ = self.emit_event(
            "stopped",
            json!({
                "reason": snap.reason,
                "threadId": 1,
                "allThreadsStopped": true,
                "preserveFocusHint": false,
                "description": snap.reason,
                "text": format!("{}:{}", snap.file, snap.line),
            }),
        );
        let mut guard = self.inner.lock().expect("dap lock");
        while guard.pending_action.is_none() && !self.disconnected.load(Ordering::SeqCst) {
            guard = self.cv.wait(guard).expect("dap cv");
        }
        let action = guard.pending_action.take().unwrap_or(DebugAction::Continue);
        guard.is_paused = false;
        // Step mode persists until the IDE sends a plain `continue`.
        guard.step_mode = matches!(action, DebugAction::StepOver | DebugAction::StepIn);
        tracing::info!(
            target: "zshrs::dap::pause",
            ?action,
            step_mode = guard.step_mode,
            "executor RESUMED",
        );
        action
    }

    fn resume_with(&self, action: DebugAction) {
        let mut g = self.inner.lock().expect("dap lock");
        g.pending_action = Some(action);
        self.cv.notify_all();
    }

    fn request_pause(&self) {
        let mut g = self.inner.lock().expect("dap lock");
        g.pause_request = true;
    }

    fn want_pause(&self) -> bool {
        self.inner.lock().map(|g| g.pause_request).unwrap_or(false)
    }

    fn step_mode(&self) -> bool {
        self.inner.lock().map(|g| g.step_mode).unwrap_or(false)
    }

    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
    }

    fn write_message(&self, body: Value) -> io::Result<()> {
        let s = serde_json::to_string(&body)?;
        let mut w = self.writer.lock().expect("dap writer");
        write!(w, "Content-Length: {}\r\n\r\n{}", s.len(), s)?;
        w.flush()
    }

    fn emit_response(
        &self,
        req_seq: i64,
        command: &str,
        success: bool,
        body: Value,
    ) -> io::Result<()> {
        let seq = self.next_seq();
        let msg = json!({
            "seq": seq,
            "type": "response",
            "request_seq": req_seq,
            "command": command,
            "success": success,
            "body": body,
        });
        tracing::trace!(target: "zshrs::dap::send", seq, %command, "response");
        self.write_message(msg)
    }
    /// `emit_event` — see implementation.
    pub fn emit_event(&self, event: &str, body: Value) -> io::Result<()> {
        let seq = self.next_seq();
        let milestone = matches!(
            event,
            "stopped" | "terminated" | "exited" | "initialized" | "process" | "breakpoint"
        );
        if milestone {
            tracing::info!(target: "zshrs::dap::send", seq, %event, "event (milestone)");
        } else {
            tracing::trace!(target: "zshrs::dap::send", seq, %event, "event");
        }
        let msg = json!({
            "seq": seq,
            "type": "event",
            "event": event,
            "body": body,
        });
        self.write_message(msg)
    }
}

/// Global handle to the live DAP server (set by `run_dap` before the
/// script starts). The `BUILTIN_SET_LINENO` handler in
/// `fusevm_bridge.rs` reads this on every statement; if unset
/// (normal shell mode) the lookup is a single atomic load and falls
/// through to no-op.
static DAP_SHARED: OnceLock<Arc<DapShared>> = OnceLock::new();

/// Per-thread breakpoint snapshot — read in the hot path of every
/// `check_line` call. Cloned from `BreakpointState` after each
/// `setBreakpoints` so the executor doesn't need to lock the global
/// state on every statement.
static DAP_BREAKPOINTS: OnceLock<Arc<Mutex<BreakpointState>>> = OnceLock::new();

/// Called by `BUILTIN_SET_LINENO` in `fusevm_bridge.rs` for every
/// top-level statement. O(1) when DAP is not active (single atomic
/// load). When active, checks the breakpoint set for the launched
/// program and pauses if the current line matches OR if step mode
/// is on OR if the client asked for a pause.
pub fn check_line(line: u32) {
    let Some(shared) = DAP_SHARED.get() else {
        return;
    };
    if shared.disconnected.load(Ordering::SeqCst) {
        return;
    }
    let program = shared.program.lock().map(|g| g.clone()).unwrap_or_default();
    let reason = if shared.want_pause() {
        "pause"
    } else if shared.step_mode() {
        "step"
    } else {
        // Breakpoint match check
        let bp_arc = match DAP_BREAKPOINTS.get() {
            Some(b) => b,
            None => return,
        };
        let bp = match bp_arc.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let hit = bp
            .line_breakpoints
            .get(&program)
            .map(|lines| lines.contains(&line))
            .unwrap_or(false);
        if !hit {
            return;
        }
        "breakpoint"
    };
    let snap = PauseSnapshot {
        reason: reason.to_string(),
        file: program,
        line,
    };
    let action = shared.pause(snap);
    if matches!(action, DebugAction::Quit) {
        // Terminate the script. We don't have a clean unwind from
        // inside the VM here; signal via errflag so the executor
        // halts at the next safe point.
        crate::ported::utils::errflag
            .fetch_or(crate::ported::zsh_h::ERRFLAG_ERROR, Ordering::Relaxed);
    }
}

// ── Public entry point ──────────────────────────────────────────────────

/// Serve the DAP protocol until the client disconnects.
///
/// `addr` selects the transport (called from `bins/zshrs.rs` on `--dap`):
///   * `Some("127.0.0.1:55123")` — **TCP connect-back**: dial the IDE's DAP
///     listener (the path JetBrains uses; stdout stays free for the script).
///   * `None` — **stdio**: DAP over stdin/stdout, for clients that spawn the
///     adapter as an executable (e.g. VS Code). Unusable under IntelliJ, whose
///     OSProcessHandler reads stdout and would steal DAP bytes.
///
/// Stryke-mirror architecture:
///   1. Connect the transport (TCP socket or stdin/stdout).
///   2. Spawn reader thread to handle `initialize`, `setBreakpoints`,
///      `launch`, `continue`, etc. Each `launch` request goes through
///      a oneshot channel back to the main thread.
///   3. Block on the channel until `launch` arrives.
///   4. Install DAP_SHARED + DAP_BREAKPOINTS globals (the BUILTIN_SET_LINENO
///      hook in fusevm_bridge.rs consults them on every statement).
///   5. Run the script IN-PROCESS via `ShellExecutor::execute_script_file`.
///      The executor blocks inside `check_line → pause()` whenever a
///      breakpoint hits; the reader thread sends `continue` to resume.
///   6. After execution: emit `exited` + `terminated`, drop globals.
pub fn run_dap(addr: Option<&str>) -> i32 {
    tracing::info!(
        target: "zshrs::dap",
        pid = std::process::id(),
        mode = if addr.is_some() { "tcp" } else { "stdio" },
        "starting --dap",
    );

    // Two transports. Both run the identical DAP server below — only the
    // reader/writer differ.
    let (reader, writer): (Box<dyn Read + Send>, Box<dyn Write + Send>) = match addr {
        // TCP connect-back: dial the IDE's DAP listener (the path JetBrains
        // uses). stdin/stdout are left free so the script's own output reaches
        // the console.
        Some(addr) => {
            let stream = match TcpStream::connect(addr) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(target: "zshrs::dap", %addr, %e, "tcp connect failed");
                    eprintln!("zshrs: --dap: connect {} failed: {}", addr, e);
                    return 1;
                }
            };
            if let Err(e) = stream.set_nodelay(true) {
                tracing::warn!(target: "zshrs::dap", %e, "TCP_NODELAY failed (non-fatal)");
            }
            let reader_stream = match stream.try_clone() {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!(target: "zshrs::dap", %e, "tcp clone failed");
                    eprintln!("zshrs: --dap: clone socket: {}", e);
                    return 1;
                }
            };
            tracing::info!(target: "zshrs::dap", %addr, "tcp connected");
            (Box::new(reader_stream), Box::new(stream))
        }
        // Stdio mode: DAP traffic over stdin/stdout. For clients that spawn the
        // adapter as an executable and talk over its pipes (e.g. VS Code's
        // DebugAdapterExecutable). NOTE: unusable under IntelliJ, whose
        // OSProcessHandler reads stdout and would steal DAP bytes — use TCP there.
        None => {
            tracing::info!(target: "zshrs::dap", "stdio mode");
            (Box::new(io::stdin()), Box::new(io::stdout()))
        }
    };

    let shared = DapShared::new(writer);
    let bp_state = Arc::new(Mutex::new(BreakpointState::default()));

    // Reader thread: parses DAP requests + dispatches. Sends a
    // LaunchParams down the channel when `launch` arrives.
    let (launch_tx, launch_rx) = std::sync::mpsc::channel::<LaunchParams>();
    let shared_reader = shared.clone();
    let bp_reader = bp_state.clone();
    let _reader = thread::spawn(move || {
        let mut br = BufReader::new(reader);
        loop {
            let msg = match read_message(&mut br) {
                Ok(Some(m)) => m,
                Ok(None) => {
                    tracing::info!(target: "zshrs::dap", "client disconnected (EOF)");
                    break;
                }
                Err(e) => {
                    tracing::error!(target: "zshrs::dap", %e, "read error");
                    break;
                }
            };
            handle_request(&shared_reader, &bp_reader, &launch_tx, msg);
            if shared_reader.disconnected.load(Ordering::SeqCst) {
                break;
            }
        }
        // Reader exiting — unblock any cv-wait so the executor can
        // see Quit and bail.
        shared_reader.disconnected.store(true, Ordering::SeqCst);
        shared_reader.resume_with(DebugAction::Quit);
    });

    // Block until the IDE sends `launch`.
    let lp = match launch_rx.recv() {
        Ok(p) => p,
        Err(_) => {
            tracing::warn!(target: "zshrs::dap", "no launch received before disconnect");
            return 1;
        }
    };
    tracing::info!(
        target: "zshrs::dap",
        program = %lp.program,
        cwd = ?lp.cwd,
        args = ?lp.args,
        stop_on_entry = lp.stop_on_entry,
        "launch received",
    );

    // Install global hooks for BUILTIN_SET_LINENO.
    *shared.program.lock().expect("program lock") = lp.program.clone();
    let _ = DAP_SHARED.set(shared.clone());
    let _ = DAP_BREAKPOINTS.set(bp_state.clone());
    if lp.stop_on_entry {
        // Force a pause at the first statement.
        shared.inner.lock().expect("dap lock").step_mode = true;
    }

    // Cosmetic events for IDE UI.
    let _ = shared.emit_event(
        "process",
        json!({
            "name": lp.program,
            "systemProcessId": std::process::id(),
            "isLocalProcess": true,
            "startMethod": "launch",
        }),
    );
    let _ = shared.emit_event("thread", json!({ "reason": "started", "threadId": 1 }));

    if let Some(cwd) = &lp.cwd {
        let _ = std::env::set_current_dir(cwd);
    }

    // Run the script in-process. The BUILTIN_SET_LINENO hook will
    // block on `shared.pause()` whenever a breakpoint matches.
    tracing::info!(target: "zshrs::dap", "entering executor (in-process)");
    let mut exec = crate::vm_helper::ShellExecutor::new();
    let exit_code = match exec.execute_script_file(&lp.program) {
        Ok(status) => status,
        Err(e) => {
            tracing::error!(target: "zshrs::dap", %e, "executor returned error");
            let _ = shared.emit_event(
                "output",
                json!({
                    "category": "stderr",
                    "output": format!("zshrs --dap: {}\n", e),
                }),
            );
            1
        }
    };
    tracing::info!(target: "zshrs::dap", exit_code, "executor exited");

    let _ = shared.emit_event("exited", json!({ "exitCode": exit_code }));
    let _ = shared.emit_event("terminated", json!({}));
    let _ = shared.emit_event("thread", json!({ "reason": "exited", "threadId": 1 }));
    // Brief grace period for the writer to drain before the process exits.
    thread::sleep(Duration::from_millis(50));
    exit_code
}

#[derive(Debug, Clone)]
struct LaunchParams {
    program: String,
    cwd: Option<String>,
    args: Vec<String>,
    stop_on_entry: bool,
}

/// Dispatch a single DAP request. `launch_tx` is used to hand the
/// program info back to the main thread; everything else (breakpoints,
/// continue, etc.) is handled here directly.
fn handle_request(
    shared: &Arc<DapShared>,
    bp_state: &Arc<Mutex<BreakpointState>>,
    launch_tx: &std::sync::mpsc::Sender<LaunchParams>,
    msg: Value,
) {
    let cmd = msg
        .get("command")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let req_seq = msg.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
    let args = msg.get("arguments").cloned().unwrap_or(Value::Null);
    tracing::trace!(
        target: "zshrs::dap::recv",
        seq = req_seq,
        %cmd,
        "request",
    );

    match cmd.as_str() {
        "initialize" => {
            let _ = shared.emit_response(
                req_seq,
                &cmd,
                true,
                json!({
                    "supportsConfigurationDoneRequest": true,
                    "supportsEvaluateForHovers": true,
                    "supportsTerminateRequest": true,
                    "supportsStepBack": false,
                    "supportsSetVariable": false,
                    "supportsConditionalBreakpoints": false,
                    "supportsHitConditionalBreakpoints": false,
                    "supportsFunctionBreakpoints": false,
                    "supportsRestartFrame": false,
                    "supportsGotoTargetsRequest": false,
                    "supportsStepInTargetsRequest": false,
                    "supportsCompletionsRequest": false,
                    "supportsModulesRequest": false,
                    "supportsExceptionInfoRequest": false,
                }),
            );
            let _ = shared.emit_event("initialized", json!({}));
        }
        "setBreakpoints" => {
            let path = args["source"]["path"].as_str().unwrap_or("").to_string();
            // Canonicalize so `check_line` lookup hits regardless of
            // relative vs absolute path differences between IDE +
            // executor.
            let canon_path = std::fs::canonicalize(&path)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| path.clone());
            let mut lines: Vec<u32> = Vec::new();
            let mut verified = Vec::new();
            if let Some(arr) = args["breakpoints"].as_array() {
                for b in arr {
                    if let Some(l) = b["line"].as_u64() {
                        lines.push(l as u32);
                        verified.push(json!({ "verified": true, "line": l }));
                    }
                }
            }
            tracing::info!(
                target: "zshrs::dap::breakpoints",
                path = %canon_path,
                count = lines.len(),
                lines = ?lines,
                "registered",
            );
            if let Ok(mut bp) = bp_state.lock() {
                if !canon_path.is_empty() {
                    bp.line_breakpoints.insert(canon_path, lines);
                }
            }
            let _ = shared.emit_response(req_seq, &cmd, true, json!({ "breakpoints": verified }));
        }
        "setExceptionBreakpoints" => {
            let _ = shared.emit_response(req_seq, &cmd, true, json!({}));
        }
        "configurationDone" => {
            shared.configuration_done.store(true, Ordering::SeqCst);
            let _ = shared.emit_response(req_seq, &cmd, true, json!({}));
        }
        "launch" => {
            let program_raw = args["program"].as_str().unwrap_or("").to_string();
            // Canonicalize so it matches the canonicalized path the
            // setBreakpoints handler stored.
            let program = std::fs::canonicalize(&program_raw)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(program_raw);
            let cwd = args["cwd"].as_str().map(|s| s.to_string());
            let lp_args: Vec<String> = args["args"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let stop_on_entry = args["stopOnEntry"].as_bool().unwrap_or(false);
            let _ = shared.emit_response(req_seq, &cmd, true, json!({}));
            let _ = launch_tx.send(LaunchParams {
                program,
                cwd,
                args: lp_args,
                stop_on_entry,
            });
        }
        "threads" => {
            let _ = shared.emit_response(
                req_seq,
                &cmd,
                true,
                json!({ "threads": [{ "id": 1, "name": "main" }] }),
            );
        }
        "stackTrace" => {
            // v1: synthesize a single frame from the launched program +
            // current $LINENO. Multi-frame call-stack walk-back is
            // future work (needs funcstack reflection).
            let program = shared.program.lock().map(|g| g.clone()).unwrap_or_default();
            let line = current_lineno();
            let frames = vec![json!({
                "id": 1,
                "name": "main",
                "source": { "path": program, "name": file_name(&program) },
                "line": line,
                "column": 1,
            })];
            let _ = shared.emit_response(
                req_seq,
                &cmd,
                true,
                json!({ "stackFrames": frames, "totalFrames": 1 }),
            );
        }
        "scopes" => {
            // Three separate scopes so IntelliJ renders them as
            // collapsible groups in this fixed order — `Locals` (user
            // vars) first, then `Specials` (zsh PM_SPECIAL params),
            // then `Environment` (caps-name env vars). One big scope
            // gets alpha-sorted client-side, burying the user's vars
            // under 300 env entries. The IDE's sort toggle still works
            // WITHIN each scope but the groups themselves are stable.
            let _ = shared.emit_response(
                req_seq,
                &cmd,
                true,
                json!({
                    "scopes": [
                        { "name": "Locals",      "variablesReference": 1, "expensive": false, "presentationHint": "locals" },
                        { "name": "Specials",    "variablesReference": 2, "expensive": false },
                        { "name": "Environment", "variablesReference": 3, "expensive": false },
                    ],
                }),
            );
        }
        "variables" => {
            let r = args["variablesReference"].as_u64().unwrap_or(1);
            let vars = match r {
                1 => snapshot_user_vars(),
                2 => snapshot_special_vars(),
                3 => snapshot_env_vars(),
                _ => snapshot_user_vars(),
            };
            let _ = shared.emit_response(req_seq, &cmd, true, json!({ "variables": vars }));
        }
        "evaluate" => {
            let expr = args["expression"].as_str().unwrap_or("");
            let (result, ty) = evaluate_expression(expr);
            let _ = shared.emit_response(
                req_seq,
                &cmd,
                true,
                json!({
                    "result": result,
                    "type": ty,
                    "variablesReference": 0,
                }),
            );
        }
        "continue" => {
            let _ =
                shared.emit_response(req_seq, &cmd, true, json!({ "allThreadsContinued": true }));
            shared.resume_with(DebugAction::Continue);
        }
        "next" => {
            let _ =
                shared.emit_response(req_seq, &cmd, true, json!({ "allThreadsContinued": true }));
            shared.resume_with(DebugAction::StepOver);
        }
        "stepIn" => {
            let _ =
                shared.emit_response(req_seq, &cmd, true, json!({ "allThreadsContinued": true }));
            shared.resume_with(DebugAction::StepIn);
        }
        "stepOut" => {
            let _ =
                shared.emit_response(req_seq, &cmd, true, json!({ "allThreadsContinued": true }));
            shared.resume_with(DebugAction::StepOut);
        }
        "pause" => {
            let _ = shared.emit_response(req_seq, &cmd, true, json!({}));
            shared.request_pause();
        }
        "disconnect" | "terminate" => {
            let _ = shared.emit_response(req_seq, &cmd, true, json!({}));
            shared.disconnected.store(true, Ordering::SeqCst);
            shared.resume_with(DebugAction::Quit);
        }
        "source" => {
            let _ = shared.emit_response(req_seq, &cmd, true, json!({}));
        }
        _ => {
            tracing::debug!(target: "zshrs::dap", %cmd, "unsupported request");
            let _ = shared.emit_response(req_seq, &cmd, false, json!({ "error": "unsupported" }));
        }
    }
}

/// Read the current `$LINENO` via paramtab — same path BUILTIN_SET_LINENO
/// writes to. Default to 1 when unset.
fn current_lineno() -> u32 {
    crate::ported::params::paramtab()
        .read()
        .ok()
        .and_then(|t| t.get("LINENO").map(|pm| pm.u_val as u32))
        .unwrap_or(1)
}

/// Buckets all paramtab entries into (user, specials, env) by zsh
/// `PM_SPECIAL` flag + caps-name + process-env presence. Returned
/// tuples are sorted alpha within each bucket. Used by the three
/// `snapshot_*_vars` helpers so each scope returns just its bucket.
///
/// Splitting into separate scopes (instead of one big list) means
/// IntelliJ renders them as collapsible groups in fixed order, even
/// when the user has "Sort Values Alphabetically" enabled — that
/// toggle only sorts WITHIN a scope, not across scopes.
fn snapshot_bucketed() -> (
    Vec<(String, String)>,
    Vec<(String, String)>,
    Vec<(String, String)>,
) {
    let mut user: Vec<(String, String)> = Vec::new();
    let mut specials: Vec<(String, String)> = Vec::new();
    let mut env: Vec<(String, String)> = Vec::new();

    if let Ok(tab) = crate::ported::params::paramtab().read() {
        for (name, pm) in tab.iter() {
            if matches!(
                name.as_str(),
                "_" | "PIPESTATUS" | "pipestatus" | "ZSH_ARGZERO"
            ) {
                continue;
            }
            let value = if pm.u_val != 0
                && (pm.node.flags & crate::ported::zsh_h::PM_INTEGER as i32) != 0
            {
                pm.u_val.to_string()
            } else if let Some(s) = pm.u_str.as_ref() {
                s.clone()
            } else {
                String::new()
            };
            let is_special = (pm.node.flags & crate::ported::zsh_h::PM_SPECIAL as i32) != 0;
            // Environment FIRST — most users think of PATH/HOME/USER
            // as env vars even though zsh marks them PM_SPECIAL. The
            // process-env presence is what makes them env, not the
            // zsh flag.
            let in_process_env = std::env::var(name).is_ok();
            // Caps-only names not in env → zsh internal specials
            // bucket (CPUTYPE, MACHTYPE, OSTYPE, HOST, etc).
            let is_caps_only = !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit())
                && name
                    .chars()
                    .next()
                    .map(|c| c.is_ascii_uppercase())
                    .unwrap_or(false);
            if in_process_env {
                env.push((name.clone(), value));
            } else if is_special || is_caps_only {
                specials.push((name.clone(), value));
            } else {
                user.push((name.clone(), value));
            }
        }
    }
    if user.is_empty() && specials.is_empty() && env.is_empty() {
        for (k, v) in std::env::vars() {
            env.push((k, v));
        }
    }
    user.sort_by(|a, b| a.0.cmp(&b.0));
    specials.sort_by(|a, b| a.0.cmp(&b.0));
    env.sort_by(|a, b| a.0.cmp(&b.0));
    (user, specials, env)
}

fn vars_to_json(vars: &[(String, String)]) -> Vec<Value> {
    vars.iter()
        .take(500)
        .map(|(n, v)| {
            json!({
                "name": n,
                "value": v,
                "type": "scalar",
                "variablesReference": 0,
            })
        })
        .collect()
}

fn snapshot_user_vars() -> Vec<Value> {
    let (user, _, _) = snapshot_bucketed();
    vars_to_json(&user)
}

fn snapshot_special_vars() -> Vec<Value> {
    let (_, specials, _) = snapshot_bucketed();
    vars_to_json(&specials)
}

fn snapshot_env_vars() -> Vec<Value> {
    let (_, _, env) = snapshot_bucketed();
    vars_to_json(&env)
}

fn evaluate_expression(expr: &str) -> (String, String) {
    // v1: run a fresh subshell to evaluate. Doesn't see the paused
    // executor's local scope, but works for `$VAR` / `$(cmd)` / `$((expr))`.
    let exe = std::env::current_exe().unwrap_or_else(|_| "zshrs".into());
    let mut cmd = Command::new(exe);
    cmd.arg("-c").arg(expr);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    match cmd.output() {
        Ok(o) => {
            if o.status.success() {
                let s = String::from_utf8_lossy(&o.stdout).trim_end().to_string();
                (s, "scalar".into())
            } else {
                let s = String::from_utf8_lossy(&o.stderr).trim_end().to_string();
                (s, "error".into())
            }
        }
        Err(e) => (format!("evaluate: {}", e), "error".into()),
    }
}
// ── Framing ──────────────────────────────────────────────────────────────

fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            return Ok(None);
        }
        if line == "\r\n" || line == "\n" {
            break;
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length =
                Some(rest.trim().parse().map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "bad Content-Length")
                })?);
        }
    }
    let len = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let v: Value =
        serde_json::from_slice(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(v))
}

fn write_message<W: Write>(mut writer: W, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

fn file_name(path: &str) -> String {
    path.rsplit_once('/')
        .map(|x| x.1)
        .unwrap_or(path)
        .to_string()
}
