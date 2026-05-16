//! End-to-end integration tests for `zshrs --lsp`.
//!
//! Spawns the real binary as a subprocess, speaks Content-Length-framed
//! JSON-RPC over its stdio, and asserts that each capability advertised
//! by `initialize` actually produces correct results for representative
//! zsh documents.
//!
//! These tests are the proof that the LSP server hooks together
//! correctly across the framing layer + request dispatch + response
//! shaping. Pure-function correctness is covered by the unit tests
//! inside `src/extensions/lsp.rs`.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
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

struct LspHandle {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl LspHandle {
    fn spawn() -> Self {
        let mut child = Command::new(zshrs_binary())
            .arg("--lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn zshrs --lsp");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self { child, stdin, stdout, next_id: 1 }
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
        let msg = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        self.send(&msg);
        self.recv_response(id)
    }

    fn notify(&mut self, method: &str, params: Value) {
        let msg = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        self.send(&msg);
    }

    /// Read messages until one matching `request_id` arrives, returning its
    /// `result`. Notifications (`publishDiagnostics`, etc.) are discarded.
    fn recv_response(&mut self, request_id: i64) -> Value {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if Instant::now() > deadline {
                panic!("timeout waiting for response to id {}", request_id);
            }
            let v = self.recv_one().expect("server EOF before response");
            if v.get("id").and_then(|x| x.as_i64()) == Some(request_id) {
                if let Some(err) = v.get("error") {
                    panic!("server returned error for id {}: {}", request_id, err);
                }
                return v["result"].clone();
            }
            // ignore — was a server-pushed notification
        }
    }

    /// Drain server messages until either a `publishDiagnostics` notification
    /// arrives or the timeout expires. Returns `Some(params)` on hit.
    fn wait_diagnostics(&mut self, timeout: Duration) -> Option<Value> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            // Use a short per-read timeout-via-poll fallback: just read.
            // The LSP server reliably pushes diagnostics within ms after
            // didOpen, so blocking-read is fine in practice.
            let v = self.recv_one()?;
            if v.get("method").and_then(|x| x.as_str()) == Some("textDocument/publishDiagnostics") {
                return Some(v["params"].clone());
            }
        }
        None
    }

    fn recv_one(&mut self) -> Option<Value> {
        let mut content_length: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).ok()?;
            if n == 0 { return None; }
            if line == "\r\n" || line == "\n" { break; }
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                content_length = rest.trim().parse().ok();
            }
        }
        let len = content_length?;
        let mut buf = vec![0u8; len];
        self.stdout.read_exact(&mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }

    fn shutdown(mut self) {
        let _ = self.request("shutdown", json!({}));
        self.notify("exit", json!({}));
        // Give the server a moment to clean up
        let _ = self.child.wait();
    }
}

// ── Tests ───────────────────────────────────────────────────────────────

#[test]
fn initialize_advertises_expected_capabilities() {
    let mut lsp = LspHandle::spawn();
    let caps = lsp.request("initialize", json!({
        "processId": 1, "capabilities": {}, "rootUri": null,
    }));
    let c = &caps["capabilities"];
    assert!(c["completionProvider"].is_object(), "no completionProvider");
    assert_eq!(c["hoverProvider"], json!(true));
    assert_eq!(c["definitionProvider"], json!(true));
    assert_eq!(c["referencesProvider"], json!(true));
    assert_eq!(c["documentSymbolProvider"], json!(true));
    assert_eq!(c["foldingRangeProvider"], json!(true));
    assert_eq!(c["documentFormattingProvider"], json!(true));
    assert!(c["renameProvider"]["prepareProvider"] == json!(true));
    assert!(c["semanticTokensProvider"].is_object());
    assert_eq!(caps["serverInfo"]["name"], json!("zshrs-lsp"));
    lsp.shutdown();
}

#[test]
fn didopen_publishes_diagnostics_for_unclosed_brace() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///broken.zsh",
            "languageId": "zshrs",
            "version": 1,
            "text": "function f {\n  echo hi\n",
        }
    }));
    let d = lsp.wait_diagnostics(Duration::from_secs(3)).expect("no diagnostics emitted");
    let arr = d["diagnostics"].as_array().expect("diagnostics array");
    assert!(!arr.is_empty(), "expected at least one diagnostic");
    assert!(arr.iter().any(|x| x["message"].as_str().unwrap_or("").contains("unclosed")));
    lsp.shutdown();
}

#[test]
fn didopen_clean_file_publishes_zero_diagnostics() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///clean.zsh",
            "languageId": "zshrs",
            "version": 1,
            "text": "function f { echo hi }\nfor i in 1 2 3; do echo $i; done\n",
        }
    }));
    let d = lsp.wait_diagnostics(Duration::from_secs(3)).expect("no diagnostics emitted");
    let arr = d["diagnostics"].as_array().expect("diagnostics array");
    assert!(arr.is_empty(), "expected zero diagnostics for clean file, got: {:?}", arr);
    lsp.shutdown();
}

#[test]
fn hover_returns_markdown_for_builtin() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    let text = "cd /tmp\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///hover.zsh",
            "languageId": "zshrs",
            "version": 1,
            "text": text,
        }
    }));
    // Drain the post-open diagnostics push
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));
    let r = lsp.request("textDocument/hover", json!({
        "textDocument": { "uri": "file:///hover.zsh" },
        "position": { "line": 0, "character": 1 },
    }));
    assert_eq!(r["contents"]["kind"], json!("markdown"));
    let v = r["contents"]["value"].as_str().expect("value");
    assert!(v.contains("**cd**"));
    assert!(v.contains("working directory"));
    lsp.shutdown();
}

#[test]
fn document_symbols_returns_functions() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    let text = "function greet {\n  echo hi\n}\nbar() { echo bye }\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///syms.zsh",
            "languageId": "zshrs",
            "version": 1,
            "text": text,
        }
    }));
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));
    let r = lsp.request("textDocument/documentSymbol", json!({
        "textDocument": { "uri": "file:///syms.zsh" },
    }));
    let arr = r.as_array().expect("symbol array");
    let names: Vec<&str> = arr.iter().filter_map(|s| s["name"].as_str()).collect();
    assert!(names.contains(&"greet"), "names: {:?}", names);
    assert!(names.contains(&"bar"), "names: {:?}", names);
}

#[test]
fn completion_returns_builtins_for_short_prefix() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///comp.zsh",
            "languageId": "zshrs",
            "version": 1,
            "text": "ec",
        }
    }));
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));
    let r = lsp.request("textDocument/completion", json!({
        "textDocument": { "uri": "file:///comp.zsh" },
        "position": { "line": 0, "character": 2 },
    }));
    let items = r["items"].as_array().expect("items");
    let labels: Vec<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();
    // `echo` should be there (prefix `ec`)
    assert!(labels.contains(&"echo"), "labels missing echo: {:?}", labels);
}

#[test]
fn definition_and_references_round_trip() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    let text = "function greet {\n  echo hi\n}\ngreet\ngreet world\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///refs.zsh", "languageId": "zshrs",
            "version": 1, "text": text,
        }
    }));
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));

    // Cursor on "greet" call at line 3
    let pos = json!({ "line": 3, "character": 2 });
    let def = lsp.request("textDocument/definition", json!({
        "textDocument": { "uri": "file:///refs.zsh" }, "position": pos.clone(),
    }));
    assert_eq!(def["range"]["start"]["line"], json!(0), "definition: {:?}", def);

    let refs = lsp.request("textDocument/references", json!({
        "textDocument": { "uri": "file:///refs.zsh" },
        "position": pos,
        "context": { "includeDeclaration": true },
    }));
    let arr = refs.as_array().expect("ref array");
    assert_eq!(arr.len(), 3, "expected decl+2 calls, got: {:?}", arr);
}

#[test]
fn folding_ranges_finds_block() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    let text = "function f {\n  echo a\n  echo b\n}\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///fold.zsh", "languageId": "zshrs",
            "version": 1, "text": text,
        }
    }));
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));
    let r = lsp.request("textDocument/foldingRange", json!({
        "textDocument": { "uri": "file:///fold.zsh" },
    }));
    let arr = r.as_array().expect("ranges");
    assert!(
        arr.iter().any(|x| x["startLine"] == 0 && x["endLine"] == 3),
        "expected (0..3) fold, got: {:?}", arr
    );
}

#[test]
fn formatting_strips_trailing_whitespace() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///fmt.zsh", "languageId": "zshrs",
            "version": 1, "text": "echo hi   \n  echo bye\t\n",
        }
    }));
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));
    let r = lsp.request("textDocument/formatting", json!({
        "textDocument": { "uri": "file:///fmt.zsh" },
        "options": { "tabSize": 4, "insertSpaces": true },
    }));
    let arr = r.as_array().expect("edits");
    assert_eq!(arr.len(), 1, "expected 1 whole-file edit: {:?}", arr);
    let new_text = arr[0]["newText"].as_str().expect("newText");
    assert!(!new_text.contains("   \n"), "trailing spaces not stripped: {:?}", new_text);
    assert!(new_text.ends_with('\n'));
}

#[test]
fn rename_emits_workspace_edits_for_all_occurrences() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    let text = "function greet { echo hi }\ngreet\ngreet x\n";
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///rename.zsh", "languageId": "zshrs",
            "version": 1, "text": text,
        }
    }));
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));
    let r = lsp.request("textDocument/rename", json!({
        "textDocument": { "uri": "file:///rename.zsh" },
        "position": { "line": 0, "character": 9 },
        "newName": "salutate",
    }));
    let changes = r["changes"].as_object().expect("changes");
    let edits = changes["file:///rename.zsh"].as_array().expect("edits");
    assert_eq!(edits.len(), 3, "expected 3 edits, got: {:?}", edits);
    for e in edits {
        assert_eq!(e["newText"], json!("salutate"));
    }
}

#[test]
fn semantic_tokens_emit_delta_encoded_array() {
    let mut lsp = LspHandle::spawn();
    let _ = lsp.request("initialize", json!({ "processId": 1, "capabilities": {} }));
    lsp.notify("initialized", json!({}));
    lsp.notify("textDocument/didOpen", json!({
        "textDocument": {
            "uri": "file:///sem.zsh", "languageId": "zshrs",
            "version": 1, "text": "# c\nif true; then echo $HOME; fi\n",
        }
    }));
    let _ = lsp.wait_diagnostics(Duration::from_millis(500));
    let r = lsp.request("textDocument/semanticTokens/full", json!({
        "textDocument": { "uri": "file:///sem.zsh" },
    }));
    let data = r["data"].as_array().expect("data array");
    // Must be a multiple of 5 (delta-encoded 5-tuples)
    assert!(!data.is_empty(), "no tokens emitted");
    assert_eq!(data.len() % 5, 0, "data not multiple-of-5: len={}", data.len());
}
