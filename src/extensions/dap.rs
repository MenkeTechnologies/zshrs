//! DAP server for zshrs — `zshrs --dap HOST:PORT`.
//!
//! Connects to the IntelliJ-side DAP client over TCP, speaks
//! Content-Length-framed JSON-RPC (same byte-based framing as LSP). For
//! v1 we run the program in a child `zshrs` subprocess so the IntelliJ
//! plugin gets working: launch, breakpoints (collected), continue/next/
//! stepIn/stepOut/pause (skeleton), threads, stackTrace, scopes,
//! variables, evaluate, output streaming, terminated.
//!
//! Deeper integration with the in-process interpreter (real per-line
//! pausing, scope walk-back into the live `Param` table) is wired
//! through hooks that drop in when called by `bins/zshrs.rs`.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

// ── Public entry point ──────────────────────────────────────────────────

/// Connect to the IntelliJ-side DAP client at `addr` (e.g.
/// `127.0.0.1:55123`) and serve the DAP protocol until the client
/// disconnects.
///
/// Called from `bins/zshrs.rs` when `--dap HOST:PORT` is detected.
pub fn run_dap(addr: &str) -> i32 {
    let stream = match TcpStream::connect(addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zshrs: --dap: connect {} failed: {}", addr, e);
            return 1;
        }
    };
    if let Err(e) = stream.set_nodelay(true) {
        eprintln!("zshrs: --dap: TCP_NODELAY: {}", e);
    }
    let stream2 = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zshrs: --dap: clone socket: {}", e);
            return 1;
        }
    };
    let mut server = DapServer::new(stream, stream2);
    let _ = server.serve();
    0
}

// ── Server state ─────────────────────────────────────────────────────────

struct DapServer {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    seq: AtomicUsize,
    program: String,
    args: Vec<String>,
    cwd: Option<String>,
    breakpoints: HashMap<String, Vec<u64>>, // file → line numbers
    child: Option<Arc<Mutex<Child>>>,
    next_var_ref: AtomicUsize,
    var_refs: Mutex<HashMap<u64, Vec<DapVar>>>,
}

#[derive(Clone, Debug)]
struct DapVar {
    name: String,
    value: String,
    ty: String,
    var_ref: u64,
}

impl DapServer {
    fn new(reader: TcpStream, writer: TcpStream) -> Self {
        Self {
            reader: BufReader::new(reader),
            writer,
            seq: AtomicUsize::new(1),
            program: String::new(),
            args: Vec::new(),
            cwd: None,
            breakpoints: HashMap::new(),
            child: None,
            next_var_ref: AtomicUsize::new(1000),
            var_refs: Mutex::new(HashMap::new()),
        }
    }

    fn serve(&mut self) -> io::Result<()> {
        loop {
            let msg = match read_message(&mut self.reader) {
                Ok(Some(m)) => m,
                Ok(None) => break,
                Err(_) => break,
            };
            let cmd = msg.get("command").and_then(|v| v.as_str()).unwrap_or("");
            let req_seq = msg.get("seq").and_then(|v| v.as_i64()).unwrap_or(0);
            let args = msg.get("arguments").cloned().unwrap_or(Value::Null);

            match cmd {
                "initialize" => {
                    self.respond(req_seq, cmd, true, json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsEvaluateForHovers": true,
                        "supportsFunctionBreakpoints": false,
                        "supportsConditionalBreakpoints": false,
                        "supportsHitConditionalBreakpoints": false,
                        "supportsStepBack": false,
                        "supportsSetVariable": false,
                        "supportsRestartFrame": false,
                        "supportsGotoTargetsRequest": false,
                        "supportsStepInTargetsRequest": false,
                        "supportsCompletionsRequest": false,
                        "supportsModulesRequest": false,
                        "supportsTerminateRequest": true,
                        "supportsExceptionInfoRequest": false,
                    }))?;
                    // Emit `initialized` event so the client can send breakpoints
                    self.event("initialized", json!({}))?;
                }
                "setBreakpoints" => {
                    let path = args["source"]["path"].as_str().unwrap_or("").to_string();
                    let mut lines = Vec::new();
                    let mut verified = Vec::new();
                    if let Some(arr) = args["breakpoints"].as_array() {
                        for b in arr {
                            if let Some(l) = b["line"].as_u64() {
                                lines.push(l);
                                verified.push(json!({ "verified": true, "line": l }));
                            }
                        }
                    }
                    if !path.is_empty() { self.breakpoints.insert(path, lines); }
                    self.respond(req_seq, cmd, true, json!({ "breakpoints": verified }))?;
                }
                "setExceptionBreakpoints" => {
                    self.respond(req_seq, cmd, true, json!({}))?;
                }
                "configurationDone" => {
                    self.respond(req_seq, cmd, true, json!({}))?;
                }
                "launch" => {
                    self.program = args["program"].as_str().unwrap_or("").to_string();
                    self.cwd = args["cwd"].as_str().map(|s| s.to_string());
                    self.args = args["args"].as_array().map(|a|
                        a.iter().filter_map(|v| v.as_str().map(String::from)).collect()
                    ).unwrap_or_default();
                    self.respond(req_seq, cmd, true, json!({}))?;
                    self.event("process", json!({
                        "name": self.program,
                        "systemProcessId": std::process::id(),
                        "isLocalProcess": true,
                        "startMethod": "launch",
                    }))?;
                    self.event("thread", json!({ "reason": "started", "threadId": 1 }))?;
                    self.launch_program()?;
                }
                "threads" => {
                    self.respond(req_seq, cmd, true, json!({
                        "threads": [{ "id": 1, "name": "main" }],
                    }))?;
                }
                "stackTrace" => {
                    let frames = self.snapshot_frames();
                    self.respond(req_seq, cmd, true, json!({
                        "stackFrames": frames,
                        "totalFrames": 1,
                    }))?;
                }
                "scopes" => {
                    // One synthetic scope per frame: "Locals" (ref = 1)
                    self.respond(req_seq, cmd, true, json!({
                        "scopes": [{
                            "name": "Locals",
                            "variablesReference": 1,
                            "expensive": false,
                        }],
                    }))?;
                }
                "variables" => {
                    let r = args["variablesReference"].as_u64().unwrap_or(1);
                    let vars = self.variables_for(r);
                    self.respond(req_seq, cmd, true, json!({ "variables": vars }))?;
                }
                "evaluate" => {
                    let expr = args["expression"].as_str().unwrap_or("");
                    let (result, ty) = self.evaluate_expression(expr);
                    self.respond(req_seq, cmd, true, json!({
                        "result": result,
                        "type": ty,
                        "variablesReference": 0,
                    }))?;
                }
                "continue" | "next" | "stepIn" | "stepOut" => {
                    // v1: we don't pause execution in the child, so these are
                    // best-effort acks. A future revision wires hooks into the
                    // in-process interpreter via DAP_HOOKS below.
                    self.respond(req_seq, cmd, true, json!({ "allThreadsContinued": true }))?;
                }
                "pause" => {
                    self.respond(req_seq, cmd, true, json!({}))?;
                    self.event("stopped", json!({
                        "reason": "pause",
                        "threadId": 1,
                        "allThreadsStopped": true,
                    }))?;
                }
                "disconnect" | "terminate" => {
                    self.respond(req_seq, cmd, true, json!({}))?;
                    if let Some(c) = self.child.take() {
                        let _ = c.lock().map(|mut g| g.kill());
                    }
                    break;
                }
                "source" => {
                    self.respond(req_seq, cmd, true, json!({}))?;
                }
                _ => {
                    self.respond(req_seq, cmd, false, json!({ "error": "unsupported" }))?;
                }
            }
        }
        Ok(())
    }

    fn launch_program(&mut self) -> io::Result<()> {
        if self.program.is_empty() {
            self.event("terminated", json!({}))?;
            return Ok(());
        }
        let exe = std::env::current_exe().unwrap_or_else(|_| "zshrs".into());
        let mut cmd = Command::new(&exe);
        cmd.arg(&self.program);
        for a in &self.args { cmd.arg(a); }
        if let Some(c) = &self.cwd { cmd.current_dir(c); }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn()?;

        // Pipe child stdout / stderr → DAP "output" events
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let writer_out = self.writer.try_clone()?;
        let writer_err = self.writer.try_clone()?;
        if let Some(out) = stdout {
            thread::spawn(move || stream_output(out, "stdout", writer_out));
        }
        if let Some(err) = stderr {
            thread::spawn(move || stream_output(err, "stderr", writer_err));
        }

        let child_arc = Arc::new(Mutex::new(child));
        self.child = Some(child_arc.clone());

        // Watcher thread fires `terminated` when the child exits
        let writer_term = self.writer.try_clone()?;
        let seq_counter = AtomicUsize::new(0);
        thread::spawn(move || {
            loop {
                {
                    let mut guard = match child_arc.lock() {
                        Ok(g) => g,
                        Err(_) => return,
                    };
                    match guard.try_wait() {
                        Ok(Some(_status)) => {
                            let _ = send_event(&writer_term, &seq_counter, "terminated", json!({}));
                            return;
                        }
                        Ok(None) => {}
                        Err(_) => return,
                    }
                }
                thread::sleep(Duration::from_millis(100));
            }
        });
        Ok(())
    }

    fn snapshot_frames(&self) -> Vec<Value> {
        // v1: a single frame for the program file.
        vec![json!({
            "id": 1,
            "name": "main",
            "source": { "path": self.program, "name": file_name(&self.program) },
            "line": 1,
            "column": 1,
        })]
    }

    fn variables_for(&self, _var_ref: u64) -> Vec<Value> {
        // v1: surface a snapshot of the current environment. A future
        // revision exposes shell parameters from the live interpreter via
        // DAP_HOOKS.with_locals(|locals| ...).
        let mut out = Vec::new();
        for (k, v) in std::env::vars() {
            out.push(json!({
                "name": k,
                "value": v,
                "type": "scalar",
                "variablesReference": 0,
            }));
            if out.len() >= 200 { break; }
        }
        out
    }

    fn evaluate_expression(&self, expr: &str) -> (String, String) {
        // Run a fresh `zshrs -c <expr>` and return its stdout. Pass-through
        // for any zsh syntax (`${var}`, `$(cmd)`, `$(( arith ))`).
        let exe = std::env::current_exe().unwrap_or_else(|_| "zshrs".into());
        let mut cmd = Command::new(exe);
        cmd.arg("-c").arg(expr);
        if let Some(c) = &self.cwd { cmd.current_dir(c); }
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

    fn respond(&self, req_seq: i64, command: &str, success: bool, body: Value) -> io::Result<()> {
        let s = self.seq.fetch_add(1, Ordering::SeqCst);
        let msg = json!({
            "seq": s,
            "type": "response",
            "request_seq": req_seq,
            "command": command,
            "success": success,
            "body": body,
        });
        write_message(&self.writer, &msg)
    }

    fn event(&self, event: &str, body: Value) -> io::Result<()> {
        let s = self.seq.fetch_add(1, Ordering::SeqCst);
        let msg = json!({
            "seq": s,
            "type": "event",
            "event": event,
            "body": body,
        });
        write_message(&self.writer, &msg)
    }
}

fn stream_output<R: Read>(reader: R, category: &str, writer: TcpStream) {
    let mut br = BufReader::new(reader);
    let mut buf = String::new();
    let seq_counter = AtomicUsize::new(0);
    loop {
        buf.clear();
        match br.read_line(&mut buf) {
            Ok(0) => return,
            Ok(_) => {
                let _ = send_event(
                    &writer,
                    &seq_counter,
                    "output",
                    json!({ "category": category, "output": buf }),
                );
            }
            Err(_) => return,
        }
    }
}

fn send_event(writer: &TcpStream, seq: &AtomicUsize, event: &str, body: Value) -> io::Result<()> {
    let s = seq.fetch_add(1, Ordering::SeqCst);
    let msg = json!({
        "seq": s + 1000,
        "type": "event",
        "event": event,
        "body": body,
    });
    write_message(writer, &msg)
}

fn file_name(path: &str) -> String {
    path.rsplit_once('/').map(|x| x.1).unwrap_or(path).to_string()
}

// ── Framing ──────────────────────────────────────────────────────────────

fn read_message<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 { return Ok(None); }
        if line == "\r\n" || line == "\n" { break; }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            content_length = Some(rest.trim().parse().map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "bad Content-Length")
            })?);
        }
    }
    let len = content_length
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    let v: Value = serde_json::from_slice(&buf)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(Some(v))
}

fn write_message<W: Write>(mut writer: W, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

// ── Hooks for deeper interpreter integration ────────────────────────────

/// Set of callbacks the DAP server installs into the live interpreter via
/// [`install_hooks`] so per-statement pausing, breakpoint honouring, and
/// live local-scope inspection can attach without forcing the interpreter
/// to take a hard dep on this module.
///
/// In v1 these are unset; the DAP server falls back to subprocess
/// execution. A future commit wires `bins/zshrs.rs` to call
/// `crate::dap::install_hooks(...)` before entering the main eval loop
/// when `--dap` is on the command line.
#[allow(dead_code)]
pub struct DapHooks {
    pub on_statement: Box<dyn Fn(&str, u32) + Send + Sync>,
    pub locals: Box<dyn Fn() -> Vec<(String, String, String)> + Send + Sync>,
}

#[allow(dead_code)]
pub fn install_hooks(_hooks: DapHooks) {
    // v1: no-op. Future revisions store these in a OnceLock and have
    // exec.rs/eval.rs call them at statement boundaries.
}
