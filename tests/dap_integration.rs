//! End-to-end integration tests for `zshrs --dap HOST:PORT`.
//!
//! Opens a TCP listener, spawns `zshrs --dap 127.0.0.1:<port>`, accepts
//! the connect-back, and round-trips real DAP messages to prove every
//! advertised request is wired and that the `launch` path actually
//! spawns the program and streams its stdout as `output` events.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

// ── Harness ──────────────────────────────────────────────────────────────

fn zshrs_binary() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

struct DapHandle {
    _child: Child,
    sock: TcpStream,
    reader: BufReader<TcpStream>,
    seq: i64,
}

impl DapHandle {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let child = Command::new(zshrs_binary())
            .arg("--dap")
            .arg(format!("127.0.0.1:{}", port))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn zshrs --dap");
        listener.set_nonblocking(false).ok();
        let deadline = Instant::now() + Duration::from_secs(5);
        let sock = loop {
            if Instant::now() > deadline {
                panic!("zshrs --dap did not connect back within 5s");
            }
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let reader = BufReader::new(sock.try_clone().expect("clone"));
        Self { _child: child, sock, reader, seq: 1 }
    }

    fn send(&mut self, msg: &Value) {
        let body = serde_json::to_vec(msg).unwrap();
        write!(self.sock, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        self.sock.write_all(&body).unwrap();
        self.sock.flush().unwrap();
    }

    fn request(&mut self, command: &str, arguments: Value) -> Value {
        let seq = self.seq;
        self.seq += 1;
        let msg = json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
        self.send(&msg);
        self.recv_response(seq)
    }

    fn recv_response(&mut self, request_seq: i64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                panic!("timeout waiting for response to seq {}", request_seq);
            }
            let v = self.recv_one().expect("server EOF before response");
            let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
            if ty == "response" && v.get("request_seq").and_then(|x| x.as_i64()) == Some(request_seq) {
                let success = v.get("success").and_then(|x| x.as_bool()).unwrap_or(false);
                if !success {
                    panic!("DAP response not success for seq {}: {}", request_seq, v);
                }
                return v["body"].clone();
            }
            // ignore events / other responses
        }
    }

    /// Drain messages until an event of `name` arrives or timeout expires.
    fn wait_event(&mut self, name: &str, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let v = self.recv_one()?;
            let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
            let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
            if ty == "event" && ev == name {
                return Some(v["body"].clone());
            }
        }
        None
    }

    /// Drain messages until an `output` event whose text contains `needle`
    /// arrives, or timeout expires.
    fn wait_output_containing(&mut self, needle: &str, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let v = self.recv_one()?;
            let ty = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
            let ev = v.get("event").and_then(|x| x.as_str()).unwrap_or("");
            if ty == "event" && ev == "output" {
                let txt = v["body"]["output"].as_str().unwrap_or("");
                if txt.contains(needle) {
                    return Some(txt.to_string());
                }
            }
        }
        None
    }

    fn recv_one(&mut self) -> Option<Value> {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).ok()?;
            if n == 0 { return None; }
            if line == "\r\n" || line == "\n" { break; }
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                content_length = rest.trim().parse().ok();
            }
        }
        let len = content_length?;
        let mut buf = vec![0u8; len];
        self.reader.read_exact(&mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }

    fn shutdown(mut self) {
        let _ = self.request("disconnect", json!({ "terminateDebuggee": true }));
        // Best-effort close
        let _ = self.sock.shutdown(std::net::Shutdown::Both);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[test]
fn initialize_advertises_capabilities_and_emits_initialized_event() {
    let mut dap = DapHandle::spawn();
    let body = dap.request("initialize", json!({
        "clientID": "test", "adapterID": "zshrs",
        "linesStartAt1": true, "columnsStartAt1": true,
    }));
    assert_eq!(body["supportsConfigurationDoneRequest"], json!(true));
    assert_eq!(body["supportsEvaluateForHovers"], json!(true));
    assert_eq!(body["supportsTerminateRequest"], json!(true));
    let init = dap.wait_event("initialized", Duration::from_secs(2));
    assert!(init.is_some(), "no `initialized` event emitted");
    dap.shutdown();
}

#[test]
fn set_breakpoints_acks_with_verified_true() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let body = dap.request("setBreakpoints", json!({
        "source": { "path": "/tmp/whatever.zsh" },
        "breakpoints": [{ "line": 7 }, { "line": 12 }],
    }));
    let arr = body["breakpoints"].as_array().expect("bp array");
    assert_eq!(arr.len(), 2);
    assert_eq!(arr[0]["verified"], json!(true));
    assert_eq!(arr[0]["line"], json!(7));
    assert_eq!(arr[1]["line"], json!(12));
    dap.shutdown();
}

#[test]
fn threads_returns_main_thread() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let body = dap.request("threads", json!({}));
    let arr = body["threads"].as_array().expect("threads");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], json!(1));
    assert_eq!(arr[0]["name"], json!("main"));
    dap.shutdown();
}

#[test]
fn stacktrace_returns_one_frame_with_program_path() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.path().to_path_buf();
    std::fs::write(&path, "echo hello\n").expect("write program");
    let _ = dap.request("launch", json!({
        "program": path.to_string_lossy(),
        "stopOnEntry": false,
        "args": [],
        "cwd": std::env::temp_dir().to_string_lossy(),
    }));

    let body = dap.request("stackTrace", json!({ "threadId": 1, "startFrame": 0, "levels": 100 }));
    let frames = body["stackFrames"].as_array().expect("frames");
    assert_eq!(frames.len(), 1, "expected 1 frame, got: {:?}", frames);
    assert_eq!(frames[0]["source"]["path"], json!(path.to_string_lossy()));
    dap.shutdown();
}

#[test]
fn scopes_returns_locals_with_ref_one() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let body = dap.request("scopes", json!({ "frameId": 1 }));
    let arr = body["scopes"].as_array().expect("scopes");
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], json!("Locals"));
    assert_eq!(arr[0]["variablesReference"], json!(1));
    dap.shutdown();
}

#[test]
fn variables_returns_env_snapshot() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let body = dap.request("variables", json!({ "variablesReference": 1 }));
    let arr = body["variables"].as_array().expect("variables");
    // PATH or HOME should exist on any reasonable test env
    assert!(
        arr.iter().any(|v| {
            let n = v["name"].as_str().unwrap_or("");
            n == "PATH" || n == "HOME" || n == "USER" || n == "USERPROFILE"
        }),
        "no recognizable env var in scope: {:?}",
        arr.iter().take(5).collect::<Vec<_>>(),
    );
    dap.shutdown();
}

#[test]
fn evaluate_runs_inline_zshrs_command() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let body = dap.request("evaluate", json!({
        "expression": "print -n hello",
        "frameId": 1,
        "context": "watch",
    }));
    let result = body["result"].as_str().expect("result");
    assert_eq!(result, "hello", "evaluate result mismatch: {:?}", result);
    assert_eq!(body["variablesReference"], json!(0));
    dap.shutdown();
}

#[test]
fn launch_streams_program_stdout_as_output_events_and_terminates() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));

    // Create a script that prints a unique marker so we can match it.
    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.path().to_path_buf();
    std::fs::write(&path, "print -r -- ZSHRS_DAP_MARKER_42\n").expect("write program");

    let _ = dap.request("launch", json!({
        "program": path.to_string_lossy(),
        "args": [],
        "cwd": std::env::temp_dir().to_string_lossy(),
    }));

    let got = dap.wait_output_containing("ZSHRS_DAP_MARKER_42", Duration::from_secs(8));
    assert!(got.is_some(), "no output event with marker text");

    let term = dap.wait_event("terminated", Duration::from_secs(8));
    assert!(term.is_some(), "no `terminated` event after child exit");
    dap.shutdown();
}

#[test]
fn pause_emits_stopped_event() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let _ = dap.request("pause", json!({ "threadId": 1 }));
    let body = dap.wait_event("stopped", Duration::from_secs(2));
    let body = body.expect("no stopped event");
    assert_eq!(body["reason"], json!("pause"));
    assert_eq!(body["threadId"], json!(1));
    dap.shutdown();
}

#[test]
fn unsupported_command_returns_error_response() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    // Bypass request() helper since it panics on `success: false`.
    let seq = dap.seq;
    dap.seq += 1;
    dap.send(&json!({
        "seq": seq, "type": "request",
        "command": "totallyMadeUpCommand", "arguments": {},
    }));
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got = None;
    while Instant::now() < deadline {
        let v = dap.recv_one().expect("EOF");
        if v["type"] == "response" && v["request_seq"].as_i64() == Some(seq) {
            got = Some(v);
            break;
        }
    }
    let resp = got.expect("no response");
    assert_eq!(resp["success"], json!(false));
    dap.shutdown();
}
