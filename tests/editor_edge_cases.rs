//! Edge-case integration tests for the LSP and DAP servers — the bug
//! classes most likely to show up under real IDE traffic but not in the
//! happy-path round-trips covered by `lsp_integration.rs` and
//! `dap_integration.rs`:
//!
//!   * Multi-byte UTF-8 in document bodies (LSP) and DAP request bodies
//!   * stderr from a launched program streams as DAP `output` events
//!     with `category: "stderr"`
//!   * `launch.cwd` is actually applied to the child process
//!   * `launch.args` are passed through to the child
//!   * Repeated `setBreakpoints` for the same source REPLACE the prior set
//!   * Large document (8 KB+) round-trips through LSP without truncation
//!   * `didClose` removes the document; subsequent hover returns null
//!   * `prepareRename` returns the right range
//!
//! These tests are what catch the bug classes that crash production
//! IntelliJ plugin sessions when they go unnoticed (UTF-8 desync, leaked
//! document buffers across files, env-var-state-leak between launches).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

fn zshrs_binary() -> PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_zshrs") {
        return PathBuf::from(p);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join("zshrs")
}

// ── LSP harness (slim) ──────────────────────────────────────────────────

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Lsp {
    fn spawn() -> Self {
        let mut child = Command::new(zshrs_binary())
            .arg("--lsp")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().expect("spawn");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut s = Self { child, stdin, stdout, next_id: 1 };
        let _ = s.request("initialize", json!({ "processId": 1, "capabilities": {} }));
        s.notify("initialized", json!({}));
        s
    }

    fn send(&mut self, msg: &Value) {
        let body = serde_json::to_vec(msg).unwrap();
        write!(self.stdin, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        self.stdin.write_all(&body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let v = self.recv().expect("EOF");
            if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
                if let Some(e) = v.get("error") { panic!("error: {}", e); }
                return v["result"].clone();
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn drain_diagnostics_briefly(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(300);
        while Instant::now() < deadline {
            if self.stdout.buffer().is_empty() {
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            let _ = self.recv();
        }
    }

    fn recv(&mut self) -> Option<Value> {
        let mut len: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).ok()?;
            if n == 0 { return None; }
            if line == "\r\n" || line == "\n" { break; }
            if let Some(r) = line.strip_prefix("Content-Length:") {
                len = r.trim().parse().ok();
            }
        }
        let n = len?;
        let mut buf = vec![0u8; n];
        self.stdout.read_exact(&mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }

    fn shutdown(mut self) {
        let _ = self.request("shutdown", json!({}));
        self.notify("exit", json!({}));
        let _ = self.child.wait();
    }
}

// ── DAP harness (slim) ──────────────────────────────────────────────────

struct Dap {
    _child: Child,
    sock: TcpStream,
    reader: BufReader<TcpStream>,
    seq: i64,
}

impl Dap {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let child = Command::new(zshrs_binary())
            .arg("--dap").arg(format!("127.0.0.1:{}", port))
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
            .spawn().expect("spawn");
        listener.set_nonblocking(false).ok();
        let deadline = Instant::now() + Duration::from_secs(5);
        let sock = loop {
            if Instant::now() > deadline { panic!("connect-back timeout"); }
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let reader = BufReader::new(sock.try_clone().unwrap());
        let mut d = Self { _child: child, sock, reader, seq: 1 };
        let _ = d.request("initialize", json!({}));
        let _ = d.wait_event("initialized", Duration::from_secs(2));
        d
    }

    fn send(&mut self, msg: &Value) {
        let body = serde_json::to_vec(msg).unwrap();
        write!(self.sock, "Content-Length: {}\r\n\r\n", body.len()).unwrap();
        self.sock.write_all(&body).unwrap();
        self.sock.flush().unwrap();
    }

    fn request(&mut self, command: &str, args: Value) -> Value {
        let seq = self.seq;
        self.seq += 1;
        self.send(&json!({ "seq": seq, "type": "request", "command": command, "arguments": args }));
        loop {
            let v = self.recv().expect("EOF");
            if v["type"] == "response" && v["request_seq"].as_i64() == Some(seq) {
                if !v["success"].as_bool().unwrap_or(false) {
                    panic!("DAP error: {}", v);
                }
                return v["body"].clone();
            }
        }
    }

    fn wait_event(&mut self, name: &str, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let v = self.recv()?;
            if v["type"] == "event" && v["event"] == name {
                return Some(v["body"].clone());
            }
        }
        None
    }

    fn wait_output(&mut self, category: &str, needle: &str, timeout: Duration) -> Option<String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            let v = self.recv()?;
            if v["type"] == "event" && v["event"] == "output" {
                let cat = v["body"]["category"].as_str().unwrap_or("");
                let txt = v["body"]["output"].as_str().unwrap_or("");
                if cat == category && txt.contains(needle) {
                    return Some(txt.to_string());
                }
            }
        }
        None
    }

    fn recv(&mut self) -> Option<Value> {
        let mut len: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).ok()?;
            if n == 0 { return None; }
            if line == "\r\n" || line == "\n" { break; }
            if let Some(r) = line.strip_prefix("Content-Length:") {
                len = r.trim().parse().ok();
            }
        }
        let n = len?;
        let mut buf = vec![0u8; n];
        self.reader.read_exact(&mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }

    fn shutdown(mut self) {
        let _ = self.request("disconnect", json!({ "terminateDebuggee": true }));
        let _ = self.sock.shutdown(std::net::Shutdown::Both);
    }
}

// ── LSP edge cases ──────────────────────────────────────────────────────

#[test]
fn lsp_handles_multibyte_utf8_in_document_body() {
    let mut lsp = Lsp::spawn();
    // Heredoc body with em-dashes, arrows, box-drawing — all multi-byte
    let text = "# café — résumé naïve\necho \"→ τ ≈ 2π ─┴─\"\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///utf8.zsh", "languageId": "zshrs",
            "version": 1, "text": text,
        }
    }));
    lsp.drain_diagnostics_briefly();
    // documentSymbol must succeed without framing desync
    let r = lsp.request("textDocument/documentSymbol", json!({
        "textDocument": { "uri": "file:///utf8.zsh" }
    }));
    assert!(r.is_array(), "documentSymbol failed on UTF-8 body: {}", r);
    lsp.shutdown();
}

#[test]
fn lsp_large_document_round_trips() {
    let mut lsp = Lsp::spawn();
    // 200 function declarations → ≥ 6 KB document
    let mut text = String::new();
    for i in 0..200 {
        text.push_str(&format!("function fn_{i} {{ local x_{i}=1; echo $x_{i}; }}\n"));
    }
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///big.zsh", "languageId": "zshrs",
            "version": 1, "text": text.clone(),
        }
    }));
    lsp.drain_diagnostics_briefly();
    let r = lsp.request("textDocument/documentSymbol", json!({
        "textDocument": { "uri": "file:///big.zsh" }
    }));
    let arr = r.as_array().expect("array");
    // Should pick up at least the 200 function names
    let fn_count = arr.iter().filter(|s| {
        s["name"].as_str().map(|n| n.starts_with("fn_")).unwrap_or(false)
    }).count();
    assert_eq!(fn_count, 200, "expected 200 functions, got {}", fn_count);
    lsp.shutdown();
}

#[test]
fn lsp_didclose_removes_document() {
    let mut lsp = Lsp::spawn();
    let uri = "file:///x.zsh";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": { "uri": uri, "languageId": "zshrs", "version": 1, "text": "cd /tmp\n" }
    }));
    lsp.drain_diagnostics_briefly();
    // Before close, hover works
    let pre = lsp.request("textDocument/hover", json!({
        "textDocument": { "uri": uri }, "position": { "line": 0, "character": 1 },
    }));
    assert!(pre["contents"].is_object(), "pre-close hover failed: {}", pre);
    // Close
    lsp.notify("textDocument/didClose", json!({ "textDocument": { "uri": uri } }));
    // After close, hover returns null (no doc in state)
    let post = lsp.request("textDocument/hover", json!({
        "textDocument": { "uri": uri }, "position": { "line": 0, "character": 1 },
    }));
    assert!(post.is_null(), "post-close hover did not return null: {}", post);
    lsp.shutdown();
}

#[test]
fn lsp_didchange_updates_document_and_redraws_diagnostics() {
    let mut lsp = Lsp::spawn();
    let uri = "file:///mut.zsh";
    // First open: clean
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": { "uri": uri, "languageId": "zshrs", "version": 1, "text": "echo hi\n" }
    }));
    lsp.drain_diagnostics_briefly();
    // Change to a broken file → must publish a non-empty diagnostics list
    lsp.notify("textDocument/didChange", json!({
        "textDocument": { "uri": uri, "version": 2 },
        "contentChanges": [{ "text": "function bad {\n  echo no close\n" }],
    }));
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut saw_non_empty = false;
    while Instant::now() < deadline {
        if let Some(msg) = lsp.recv() {
            if msg["method"] == "textDocument/publishDiagnostics" {
                let arr = msg["params"]["diagnostics"].as_array().unwrap();
                if !arr.is_empty() {
                    saw_non_empty = true;
                    break;
                }
            }
        }
    }
    assert!(saw_non_empty, "no diagnostics published after didChange to broken file");
    lsp.shutdown();
}

#[test]
fn lsp_prepare_rename_returns_word_range() {
    let mut lsp = Lsp::spawn();
    let text = "function greet { echo hi }\ngreet\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": { "uri": "file:///p.zsh", "languageId": "zshrs",
                          "version": 1, "text": text },
    }));
    lsp.drain_diagnostics_briefly();
    let r = lsp.request("textDocument/prepareRename", json!({
        "textDocument": { "uri": "file:///p.zsh" },
        "position": { "line": 1, "character": 2 },
    }));
    assert_eq!(r["start"]["line"], json!(1));
    assert_eq!(r["start"]["character"], json!(0));
    assert_eq!(r["end"]["character"], json!(5)); // "greet" length
    lsp.shutdown();
}

#[test]
fn lsp_hover_on_unknown_word_returns_null() {
    let mut lsp = Lsp::spawn();
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": { "uri": "file:///u.zsh", "languageId": "zshrs",
                          "version": 1, "text": "xyzzy_unknown_name\n" }
    }));
    lsp.drain_diagnostics_briefly();
    let r = lsp.request("textDocument/hover", json!({
        "textDocument": { "uri": "file:///u.zsh" }, "position": { "line": 0, "character": 3 },
    }));
    assert!(r.is_null(), "hover should be null for unknown: {}", r);
    lsp.shutdown();
}

#[test]
fn lsp_comment_run_of_three_or_more_folds() {
    let mut lsp = Lsp::spawn();
    let text = "# one\n# two\n# three\n# four\necho hi\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": { "uri": "file:///c.zsh", "languageId": "zshrs",
                          "version": 1, "text": text }
    }));
    lsp.drain_diagnostics_briefly();
    let r = lsp.request("textDocument/foldingRange", json!({
        "textDocument": { "uri": "file:///c.zsh" }
    }));
    let arr = r.as_array().expect("ranges");
    let comment_fold = arr.iter().find(|x| x["kind"] == json!("comment"));
    assert!(comment_fold.is_some(), "no comment fold: {:?}", arr);
    let f = comment_fold.unwrap();
    assert_eq!(f["startLine"], json!(0));
    assert_eq!(f["endLine"], json!(3));
    lsp.shutdown();
}

// ── DAP edge cases ──────────────────────────────────────────────────────

#[test]
fn dap_evaluate_round_trips_multibyte_utf8() {
    let mut dap = Dap::spawn();
    // Print a string with em-dashes, arrows, box-drawing
    let body = dap.request("evaluate", json!({
        "expression": "print -n 'résumé — τ ≈ 2π ─┴─'",
        "frameId": 1, "context": "watch",
    }));
    let result = body["result"].as_str().expect("result");
    assert_eq!(result, "résumé — τ ≈ 2π ─┴─", "UTF-8 desync in evaluate: {:?}", result);
    dap.shutdown();
}

#[test]
fn dap_evaluate_arithmetic_expansion() {
    let mut dap = Dap::spawn();
    let body = dap.request("evaluate", json!({
        "expression": "print -n $((7 * 6))",
        "frameId": 1, "context": "watch",
    }));
    assert_eq!(body["result"].as_str(), Some("42"));
    dap.shutdown();
}

#[test]
fn dap_launch_streams_stderr_as_output_with_category_stderr() {
    let mut dap = Dap::spawn();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    std::fs::write(&path, "echo STDERR_MARKER_99 >&2\n").unwrap();
    let _ = dap.request("launch", json!({
        "program": path.to_string_lossy(),
        "args": [],
        "cwd": std::env::temp_dir().to_string_lossy(),
    }));
    let got = dap.wait_output("stderr", "STDERR_MARKER_99", Duration::from_secs(8));
    assert!(got.is_some(), "stderr text never showed up as category=stderr output event");
    dap.shutdown();
}

#[test]
fn dap_launch_applies_cwd_to_child() {
    let mut dap = Dap::spawn();
    let tmpdir = tempfile::tempdir().unwrap();
    let prog = tempfile::NamedTempFile::new().unwrap();
    let prog_path = prog.path().to_path_buf();
    // Print working dir; cwd should be tmpdir
    std::fs::write(&prog_path, "print -r -- CWD_IS=$(pwd)\n").unwrap();
    let _ = dap.request("launch", json!({
        "program": prog_path.to_string_lossy(),
        "args": [],
        "cwd": tmpdir.path().to_string_lossy(),
    }));
    let out = dap.wait_output("stdout", "CWD_IS=", Duration::from_secs(8))
        .expect("no stdout from child");
    // On macOS `/private/tmp/...` resolves to `/tmp/...` and vice versa; do a
    // suffix-match against the realpath of the tmpdir to be robust.
    let want = std::fs::canonicalize(tmpdir.path()).unwrap();
    let want_str = want.to_string_lossy();
    let last_seg = want_str.rsplit('/').next().unwrap_or("");
    assert!(
        out.contains(last_seg) && !last_seg.is_empty(),
        "cwd not applied — got: {:?}, expected suffix: {}", out.trim(), last_seg,
    );
    dap.shutdown();
}

#[test]
fn dap_launch_passes_args_through_to_program() {
    let mut dap = Dap::spawn();
    let prog = tempfile::NamedTempFile::new().unwrap();
    let prog_path = prog.path().to_path_buf();
    // Echo back all positional params
    std::fs::write(&prog_path, "print -r -- ARGS=\"$@\"\n").unwrap();
    let _ = dap.request("launch", json!({
        "program": prog_path.to_string_lossy(),
        "args": ["alpha", "beta", "gamma"],
        "cwd": std::env::temp_dir().to_string_lossy(),
    }));
    let out = dap.wait_output("stdout", "ARGS=", Duration::from_secs(8))
        .expect("no stdout");
    assert!(out.contains("alpha"));
    assert!(out.contains("beta"));
    assert!(out.contains("gamma"));
    dap.shutdown();
}

#[test]
fn dap_repeated_set_breakpoints_replace_prior_set() {
    let mut dap = Dap::spawn();
    // First call: 3 breakpoints
    let r1 = dap.request("setBreakpoints", json!({
        "source": { "path": "/tmp/x.zsh" },
        "breakpoints": [{"line": 1}, {"line": 2}, {"line": 3}],
    }));
    assert_eq!(r1["breakpoints"].as_array().unwrap().len(), 3);
    // Second call for the SAME source: 1 breakpoint — should replace, not append
    let r2 = dap.request("setBreakpoints", json!({
        "source": { "path": "/tmp/x.zsh" },
        "breakpoints": [{"line": 99}],
    }));
    let arr = r2["breakpoints"].as_array().unwrap();
    assert_eq!(arr.len(), 1, "second call should replace, got: {:?}", arr);
    assert_eq!(arr[0]["line"], json!(99));
    dap.shutdown();
}

#[test]
fn dap_set_breakpoints_with_empty_list_is_legal() {
    // The "clear all breakpoints in this file" message from a client. Some
    // clients send this when the user disables every gutter mark in a file;
    // the server must not panic.
    let mut dap = Dap::spawn();
    let body = dap.request("setBreakpoints", json!({
        "source": { "path": "/tmp/y.zsh" },
        "breakpoints": [],
    }));
    assert_eq!(body["breakpoints"].as_array().unwrap().len(), 0);
    dap.shutdown();
}
