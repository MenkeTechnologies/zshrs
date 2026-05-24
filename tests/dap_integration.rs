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
        Self {
            _child: child,
            sock,
            reader,
            seq: 1,
        }
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
        let msg =
            json!({ "seq": seq, "type": "request", "command": command, "arguments": arguments });
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
            if ty == "response"
                && v.get("request_seq").and_then(|x| x.as_i64()) == Some(request_seq)
            {
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

    fn recv_one(&mut self) -> Option<Value> {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).ok()?;
            if n == 0 {
                return None;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
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
    let body = dap.request(
        "initialize",
        json!({
            "clientID": "test", "adapterID": "zshrs",
            "linesStartAt1": true, "columnsStartAt1": true,
        }),
    );
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
    let body = dap.request(
        "setBreakpoints",
        json!({
            "source": { "path": "/tmp/whatever.zsh" },
            "breakpoints": [{ "line": 7 }, { "line": 12 }],
        }),
    );
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
    // The new in-process DAP canonicalizes the program path on
    // launch (so setBreakpoints lookups hit regardless of relative
    // vs absolute path); the test's source path is already
    // canonical via NamedTempFile so this is a no-op match.
    let canon = std::fs::canonicalize(&path).expect("canonicalize");
    let _ = dap.request(
        "launch",
        json!({
            "program": path.to_string_lossy(),
            "stopOnEntry": false,
            "args": [],
            "cwd": std::env::temp_dir().to_string_lossy(),
        }),
    );

    let body = dap.request(
        "stackTrace",
        json!({ "threadId": 1, "startFrame": 0, "levels": 100 }),
    );
    let frames = body["stackFrames"].as_array().expect("frames");
    assert_eq!(frames.len(), 1, "expected 1 frame, got: {:?}", frames);
    assert_eq!(frames[0]["source"]["path"], json!(canon.to_string_lossy()));
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
fn variables_order_user_vars_first_then_specials_then_env() {
    // Pin the strykelang-style ordering: when paused at a breakpoint,
    // the Variables panel should show user-defined vars at the top
    // (the ones the user is debugging), then zsh specials, then env
    // vars at the bottom. Without this, the panel reads as "300 env
    // vars and you have to scroll" — useless during debugging.
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.path().to_path_buf();
    let canon = std::fs::canonicalize(&path).expect("canonicalize");
    // Declare a user var, then sit on a breakpoint after it.
    std::fs::write(
        &path,
        "my_user_var=42\nanother_user_var=hello\necho done\n",
    )
    .expect("write program");
    let _ = dap.request(
        "setBreakpoints",
        json!({
            "source": { "path": canon.to_string_lossy() },
            "breakpoints": [{ "line": 3 }],
        }),
    );
    let _ = dap.request("configurationDone", json!({}));
    let _ = dap.request(
        "launch",
        json!({
            "program": canon.to_string_lossy(),
            "args": [],
            "cwd": std::env::temp_dir().to_string_lossy(),
        }),
    );
    let _ = dap
        .wait_event("stopped", Duration::from_secs(8))
        .expect("breakpoint never fired");

    let body = dap.request("variables", json!({ "variablesReference": 1 }));
    let arr = body["variables"].as_array().expect("variables");
    let names: Vec<&str> = arr
        .iter()
        .map(|v| v["name"].as_str().unwrap_or(""))
        .collect();
    // Find positions of: user var, special, env var.
    let pos = |target: &str| names.iter().position(|n| *n == target);
    let user_pos = pos("my_user_var").or_else(|| pos("another_user_var"));
    let env_pos = pos("PATH").or_else(|| pos("HOME"));
    assert!(
        user_pos.is_some(),
        "no user var in snapshot — got: {:?}",
        names.iter().take(20).collect::<Vec<_>>(),
    );
    assert!(env_pos.is_some(), "no env var in snapshot");
    assert!(
        user_pos.unwrap() < env_pos.unwrap(),
        "user var at pos {} should appear BEFORE env var at pos {} — got order: {:?}",
        user_pos.unwrap(),
        env_pos.unwrap(),
        names.iter().take(20).collect::<Vec<_>>(),
    );

    let _ = dap.request("continue", json!({ "threadId": 1 }));
    let _ = dap.wait_event("terminated", Duration::from_secs(4));
    dap.shutdown();
}

#[test]
fn evaluate_runs_inline_zshrs_command() {
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let body = dap.request(
        "evaluate",
        json!({
            "expression": "print -n hello",
            "frameId": 1,
            "context": "watch",
        }),
    );
    let result = body["result"].as_str().expect("result");
    assert_eq!(result, "hello", "evaluate result mismatch: {:?}", result);
    assert_eq!(body["variablesReference"], json!(0));
    dap.shutdown();
}

#[test]
fn launch_emits_terminated_event_after_program_finishes() {
    // New in-process DAP runs the script directly via ShellExecutor,
    // not as a subprocess. Stdout / stderr go to the DAP process's own
    // stdio (which IntelliJ's OSProcessHandler captures in the IDE
    // Console) — NOT through DAP `output` events. We just check that
    // a `terminated` event fires after the script finishes.
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.path().to_path_buf();
    std::fs::write(&path, "true\n").expect("write program");

    let _ = dap.request(
        "launch",
        json!({
            "program": path.to_string_lossy(),
            "args": [],
            "cwd": std::env::temp_dir().to_string_lossy(),
        }),
    );

    let term = dap.wait_event("terminated", Duration::from_secs(8));
    assert!(term.is_some(), "no `terminated` event after script finish");
    dap.shutdown();
}

#[test]
fn breakpoint_actually_pauses_and_continue_resumes() {
    // The REAL test of the new DAP architecture. Register a
    // breakpoint at line 2, launch, expect `stopped` reason=breakpoint
    // at that exact line, send `continue`, expect `terminated`.
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));

    let tmp = tempfile::NamedTempFile::new().expect("tempfile");
    let path = tmp.path().to_path_buf();
    let canon = std::fs::canonicalize(&path).expect("canonicalize");
    std::fs::write(
        &path,
        "echo line1\necho line2\necho line3\necho line4\n",
    )
    .expect("write program");

    let _ = dap.request(
        "setBreakpoints",
        json!({
            "source": { "path": canon.to_string_lossy() },
            "breakpoints": [{ "line": 2 }],
        }),
    );
    let _ = dap.request("configurationDone", json!({}));
    let _ = dap.request(
        "launch",
        json!({
            "program": canon.to_string_lossy(),
            "args": [],
            "cwd": std::env::temp_dir().to_string_lossy(),
        }),
    );

    let body = dap
        .wait_event("stopped", Duration::from_secs(8))
        .expect("breakpoint never fired — DAP `stopped` event not received");
    assert_eq!(
        body["reason"],
        json!("breakpoint"),
        "wrong stop reason: {:?}",
        body,
    );
    let text = body["text"].as_str().unwrap_or("");
    assert!(
        text.ends_with(":2"),
        "stopped at wrong line: text={:?}",
        text,
    );

    let _ = dap.request("continue", json!({ "threadId": 1 }));
    let term = dap.wait_event("terminated", Duration::from_secs(8));
    assert!(term.is_some(), "no `terminated` after continue");
    dap.shutdown();
}

#[test]
fn pause_request_succeeds_without_running_program() {
    // Before launch, `pause` just sets the pause-request flag — no
    // `stopped` event fires until the executor reaches `check_line`.
    // The response must still succeed though.
    let mut dap = DapHandle::spawn();
    let _ = dap.request("initialize", json!({}));
    let _ = dap.wait_event("initialized", Duration::from_secs(2));
    let body = dap.request("pause", json!({ "threadId": 1 }));
    // pause response body is empty json — success is what matters.
    assert!(body.is_object());
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
