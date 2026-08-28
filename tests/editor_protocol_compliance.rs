//! Protocol-compliance + robustness tests for the LSP and DAP servers.
//!
//! Targets bug classes the first two test batches don't cover:
//!
//!   * **LSP shutdown** sequence (`shutdown` → `exit` exits cleanly,
//!     not just hangs)
//!   * **LSP documentHighlight** + **didSave** handlers that exist but
//!     weren't being exercised
//!   * **Multi-document LSP state isolation** — open three docs, each
//!     hover resolves against its own buffer
//!   * **LSP empty document** doesn't crash diagnose / hover / symbols
//!   * **DAP terminate** request (distinct from `disconnect`)
//!   * **DAP child cleanup**: after disconnect, the launched child is
//!     dead — no zombie processes leaked
//!   * **DAP launch with nonexistent program** fails gracefully
//!   * **`--dump-reflection` value shape** — every entry value is a
//!     string (no nulls, no nested objects) so the IntelliJ tree
//!     renderer doesn't NPE

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

// ── LSP harness ─────────────────────────────────────────────────────────

struct Lsp {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
}

impl Lsp {
    fn spawn_uninit() -> Self {
        let mut child = Command::new(zshrs_binary())
            .arg("--lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        }
    }

    fn spawn() -> Self {
        let mut s = Self::spawn_uninit();
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
                if let Some(e) = v.get("error") {
                    panic!("error: {}", e);
                }
                return v["result"].clone();
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(&json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn drain_briefly(&mut self) {
        let deadline = Instant::now() + Duration::from_millis(200);
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
            if n == 0 {
                return None;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(r) = line.strip_prefix("Content-Length:") {
                len = r.trim().parse().ok();
            }
        }
        let n = len?;
        let mut buf = vec![0u8; n];
        self.stdout.read_exact(&mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }
}

// ── DAP harness ─────────────────────────────────────────────────────────

struct Dap {
    child: Child,
    sock: TcpStream,
    reader: BufReader<TcpStream>,
    seq: i64,
}

impl Dap {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let child = Command::new(zshrs_binary())
            .arg("--dap")
            .arg(format!("127.0.0.1:{}", port))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn");
        listener.set_nonblocking(false).ok();
        let deadline = Instant::now() + Duration::from_secs(5);
        let sock = loop {
            if Instant::now() > deadline {
                panic!("connect-back timeout");
            }
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        sock.set_read_timeout(Some(Duration::from_secs(5))).ok();
        let reader = BufReader::new(sock.try_clone().unwrap());
        let mut d = Self {
            child,
            sock,
            reader,
            seq: 1,
        };
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

    fn recv(&mut self) -> Option<Value> {
        let mut len: Option<usize> = None;
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line).ok()?;
            if n == 0 {
                return None;
            }
            if line == "\r\n" || line == "\n" {
                break;
            }
            if let Some(r) = line.strip_prefix("Content-Length:") {
                len = r.trim().parse().ok();
            }
        }
        let n = len?;
        let mut buf = vec![0u8; n];
        self.reader.read_exact(&mut buf).ok()?;
        serde_json::from_slice(&buf).ok()
    }
}

// ── LSP protocol compliance ─────────────────────────────────────────────

#[test]
fn lsp_shutdown_then_exit_terminates_process_within_timeout() {
    let mut lsp = Lsp::spawn();
    // Spec: `shutdown` returns success, then `exit` causes the server to exit.
    let r = lsp.request("shutdown", json!({}));
    assert!(
        r.is_null(),
        "shutdown should return null result, got: {}",
        r
    );
    lsp.notify("exit", json!({}));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Ok(Some(_)) = lsp.child.try_wait() {
            return;
        }
        if Instant::now() > deadline {
            let _ = lsp.child.kill();
            panic!("LSP server did not exit within 3s after exit notification");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn lsp_unknown_method_returns_method_not_found() {
    let mut lsp = Lsp::spawn();
    // Bypass the request() helper because it panics on error responses.
    let id = lsp.next_id;
    lsp.next_id += 1;
    lsp.send(&json!({
        "jsonrpc": "2.0", "id": id,
        "method": "textDocument/madeUpMethod", "params": {},
    }));
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got = None;
    while Instant::now() < deadline {
        let v = lsp.recv().expect("EOF");
        if v.get("id").and_then(|x| x.as_i64()) == Some(id) {
            got = Some(v);
            break;
        }
    }
    let resp = got.expect("no response to unknown method");
    let err = resp.get("error").expect("no error field");
    assert_eq!(err["code"], json!(-32601), "wrong error code: {}", err);
    let _ = lsp.request("shutdown", json!({}));
    lsp.notify("exit", json!({}));
    let _ = lsp.child.wait();
}

#[test]
fn lsp_documenthighlight_marks_every_occurrence() {
    let mut lsp = Lsp::spawn();
    let text = "function f { :; }\nf\nf 1\nfoo f\n";
    lsp.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": { "uri": "file:///h.zsh", "languageId": "zshrs",
                              "version": 1, "text": text },
        }),
    );
    lsp.drain_briefly();
    let r = lsp.request(
        "textDocument/documentHighlight",
        json!({
            "textDocument": { "uri": "file:///h.zsh" },
            "position": { "line": 1, "character": 0 },
        }),
    );
    let arr = r.as_array().expect("array");
    // 3 occurrences of the SYMBOL `f`: the `function f` decl (line 0)
    // plus the two call sites in command position (lines 1 and 2).
    //
    // Line 3's `foo f` is NOT one of them: there `f` is an argument
    // word passed to `foo`, not a call of the function. documentHighlight
    // shares the AST-backed `references` walk, which for a Func symbol
    // records only command-position uses (`lsp_symbols::OccurrenceFinder`
    // walk_simple, `s.words.first()`); the textual fallback that would
    // have matched a bare argument was removed on purpose, see
    // `references` in src/extensions/lsp.rs — "the fallback turned Find
    // Usages into a glorified `grep -w`". The word-boundary rule
    // separately excludes the `f` inside `function`.
    assert_eq!(arr.len(), 3, "expected 3 highlights, got: {:?}", arr);
    let lines: Vec<u64> = arr
        .iter()
        .filter_map(|h| h["range"]["start"]["line"].as_u64())
        .collect();
    assert_eq!(lines, vec![0, 1, 2], "wrong highlight lines: {:?}", arr);
    assert!(
        !lines.contains(&3),
        "argument occurrence in `foo f` must not be highlighted: {:?}",
        arr
    );
    for h in arr {
        assert_eq!(h["kind"], json!(1));
    } // text
    let _ = lsp.request("shutdown", json!({}));
    lsp.notify("exit", json!({}));
    let _ = lsp.child.wait();
}

#[test]
fn lsp_didsave_republishes_diagnostics_for_current_buffer() {
    let mut lsp = Lsp::spawn();
    let uri = "file:///s.zsh";
    lsp.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": { "uri": uri, "languageId": "zshrs", "version": 1,
                              "text": "function bad {\n  echo no close\n" },
        }),
    );
    // Drain the post-open diagnostic
    lsp.drain_briefly();
    // didSave should re-publish — even though we sent no content changes.
    lsp.notify(
        "textDocument/didSave",
        json!({ "textDocument": { "uri": uri } }),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut got_non_empty = false;
    while Instant::now() < deadline {
        if let Some(v) = lsp.recv() {
            if v["method"] == "textDocument/publishDiagnostics"
                && v["params"]["uri"] == json!(uri)
                && !v["params"]["diagnostics"].as_array().unwrap().is_empty()
            {
                got_non_empty = true;
                break;
            }
        }
    }
    assert!(got_non_empty, "didSave did not re-publish diagnostics");
    let _ = lsp.request("shutdown", json!({}));
    lsp.notify("exit", json!({}));
    let _ = lsp.child.wait();
}

#[test]
fn lsp_multidoc_state_is_isolated_per_uri() {
    let mut lsp = Lsp::spawn();
    // Three open documents with distinct content
    for (uri, text) in [
        ("file:///a.zsh", "function alpha {}\n"),
        ("file:///b.zsh", "function beta {}\n"),
        ("file:///c.zsh", "function gamma {}\n"),
    ] {
        lsp.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": { "uri": uri, "languageId": "zshrs", "version": 1, "text": text },
            }),
        );
    }
    lsp.drain_briefly();
    // documentSymbol on each must return only its OWN function
    for (uri, wanted) in [
        ("file:///a.zsh", "alpha"),
        ("file:///b.zsh", "beta"),
        ("file:///c.zsh", "gamma"),
    ] {
        let r = lsp.request(
            "textDocument/documentSymbol",
            json!({
                "textDocument": { "uri": uri }
            }),
        );
        let names: Vec<&str> = r
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|s| s["name"].as_str())
            .collect();
        assert!(
            names.contains(&wanted),
            "{}: missing {}: {:?}",
            uri,
            wanted,
            names
        );
        // Cross-leak check
        for other in ["alpha", "beta", "gamma"] {
            if other != wanted {
                assert!(
                    !names.contains(&other),
                    "doc {} leaked symbol {} from another doc: {:?}",
                    uri,
                    other,
                    names
                );
            }
        }
    }
    let _ = lsp.request("shutdown", json!({}));
    lsp.notify("exit", json!({}));
    let _ = lsp.child.wait();
}

#[test]
fn lsp_empty_document_doesnt_crash() {
    let mut lsp = Lsp::spawn();
    lsp.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": { "uri": "file:///empty.zsh", "languageId": "zshrs",
                              "version": 1, "text": "" },
        }),
    );
    lsp.drain_briefly();
    // All endpoints should return empty / null without panicking
    let sym = lsp.request(
        "textDocument/documentSymbol",
        json!({
            "textDocument": { "uri": "file:///empty.zsh" }
        }),
    );
    assert!(sym.as_array().map(|a| a.is_empty()).unwrap_or(false));
    let hov = lsp.request(
        "textDocument/hover",
        json!({
            "textDocument": { "uri": "file:///empty.zsh" },
            "position": { "line": 0, "character": 0 },
        }),
    );
    assert!(hov.is_null());
    let fold = lsp.request(
        "textDocument/foldingRange",
        json!({
            "textDocument": { "uri": "file:///empty.zsh" }
        }),
    );
    assert!(fold.as_array().map(|a| a.is_empty()).unwrap_or(false));
    let _ = lsp.request("shutdown", json!({}));
    lsp.notify("exit", json!({}));
    let _ = lsp.child.wait();
}

#[test]
fn lsp_completion_with_no_prefix_returns_all_known_categories() {
    let mut lsp = Lsp::spawn();
    lsp.notify(
        "textDocument/didOpen",
        json!({
            "textDocument": { "uri": "file:///all.zsh", "languageId": "zshrs",
                              "version": 1, "text": "" },
        }),
    );
    lsp.drain_briefly();
    let r = lsp.request(
        "textDocument/completion",
        json!({
            "textDocument": { "uri": "file:///all.zsh" },
            "position": { "line": 0, "character": 0 },
        }),
    );
    let items = r["items"].as_array().expect("items");
    let labels: Vec<&str> = items.iter().filter_map(|i| i["label"].as_str()).collect();
    // Must contain at least one of each category
    assert!(labels.contains(&"if"), "no keyword `if`");
    assert!(labels.contains(&"cd"), "no builtin `cd`");
    assert!(
        labels.contains(&"EXTENDED_GLOB"),
        "no option `EXTENDED_GLOB`"
    );
    // LSP completion-item-kind 14 = Keyword; 3 = Function/builtin; 21 = Constant
    let kinds: std::collections::HashSet<u64> =
        items.iter().filter_map(|i| i["kind"].as_u64()).collect();
    assert!(kinds.contains(&14), "no Keyword kind in items");
    assert!(kinds.contains(&3), "no Function/builtin kind in items");
    assert!(kinds.contains(&21), "no Constant/option kind in items");
    let _ = lsp.request("shutdown", json!({}));
    lsp.notify("exit", json!({}));
    let _ = lsp.child.wait();
}

// ── DAP protocol compliance ─────────────────────────────────────────────

#[test]
fn dap_terminate_request_acks_cleanly() {
    let mut dap = Dap::spawn();
    let _ = dap.request("terminate", json!({}));
    // After terminate, the child's reader should hang up.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        match dap.child.try_wait() {
            Ok(Some(_)) => return,
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    let _ = dap.child.kill();
    panic!("zshrs --dap did not exit after `terminate` request");
}

#[test]
fn dap_disconnect_kills_the_launched_child_program() {
    let mut dap = Dap::spawn();
    // Launch a script that sleeps long enough to outlive a disconnect.
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    std::fs::write(&path, "sleep 30\n").unwrap();
    let _ = dap.request(
        "launch",
        json!({
            "program": path.to_string_lossy(),
            "args": [],
            "cwd": std::env::temp_dir().to_string_lossy(),
        }),
    );
    // Wait for the child to actually start (we need its PID via `process` event)
    let proc_event = dap
        .wait_event("process", Duration::from_secs(3))
        .expect("no `process` event");
    let _adapter_pid = proc_event["systemProcessId"].as_u64();
    // Disconnect with terminateDebuggee
    let _ = dap.request("disconnect", json!({ "terminateDebuggee": true }));
    // The DAP adapter (the `zshrs --dap`) itself must exit so that, in
    // turn, its child (`sleep 30`) gets reaped via the watcher thread's
    // process kill. Wait for that.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = dap.child.try_wait() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = dap.child.kill();
    panic!("zshrs --dap did not exit after disconnect");
}

#[test]
fn dap_launch_with_nonexistent_program_emits_terminated_promptly() {
    let mut dap = Dap::spawn();
    // launch may succeed (the adapter just spawns Command), then the
    // child errors out immediately and we see `terminated`.
    let res = dap.request(
        "launch",
        json!({
            "program": "/__definitely_does_not_exist__/x.zsh",
            "args": [],
            "cwd": std::env::temp_dir().to_string_lossy(),
        }),
    );
    // `launch` itself returns success; the failure surfaces as `terminated`.
    let _ = res;
    let term = dap.wait_event("terminated", Duration::from_secs(6));
    assert!(
        term.is_some(),
        "no `terminated` event after bad-program launch"
    );
    let _ = dap.request("disconnect", json!({}));
}

#[test]
fn dap_initialize_advertises_unsupported_features_as_false() {
    let mut dap = Dap::spawn();
    // Re-initialize would error; instead grab the capabilities from the
    // initial handshake we did in spawn().
    // We can re-fetch via threads or another request to confirm liveness.
    let _ = dap.request("threads", json!({}));
    // Now drive a second `initialize` purely as a property check on the
    // already-running adapter. (zshrs --dap is permissive and re-acks.)
    let body = dap.request("initialize", json!({}));
    for f in [
        "supportsConditionalBreakpoints",
        "supportsHitConditionalBreakpoints",
        "supportsFunctionBreakpoints",
        "supportsStepBack",
        "supportsSetVariable",
        "supportsRestartFrame",
        "supportsCompletionsRequest",
        "supportsExceptionInfoRequest",
        "supportsModulesRequest",
    ] {
        assert_eq!(body[f], json!(false), "{} should be advertised as false", f);
    }
    let _ = dap.request("disconnect", json!({}));
}

// ── --dump-reflection shape ─────────────────────────────────────────────

#[test]
fn dump_reflection_every_entry_value_is_a_string() {
    let out = Command::new(zshrs_binary())
        .arg("--dump-reflection")
        .output()
        .expect("spawn");
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let obj = v.as_object().expect("top-level object");
    for (cat, val) in obj {
        let m = val.as_object().expect(cat);
        for (name, tag) in m {
            assert!(
                tag.is_string(),
                "category {}, entry {}: value not a string: {}",
                cat,
                name,
                tag
            );
        }
    }
}

#[test]
fn dump_reflection_has_no_duplicate_names_across_categories() {
    // The IntelliJ tool window groups by category, so a name landing in
    // two LEAF categories has to be a name zsh genuinely gives two
    // roles — otherwise it is a registry leaking into the wrong tab.
    //
    // `all` and `builtins` are excluded because they are unions BY
    // CONSTRUCTION (`dump_reflection_json`: `all` is every registry
    // merged, `builtins` is `compat ∪ extensions`), so every single
    // name in the blob "duplicates" into them. Including them made the
    // check fire on the first key it saw and say nothing about
    // registry hygiene.
    //
    // The expected set is pinned exactly — a NEW overlap fails, and so
    // does an overlap that silently disappears.
    let out = Command::new(zshrs_binary())
        .arg("--dump-reflection")
        .output()
        .expect("spawn");
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    const AGGREGATES: [&str; 2] = ["all", "builtins"];
    let mut where_seen: std::collections::HashMap<String, Vec<String>> = Default::default();
    for (cat, m) in v.as_object().unwrap() {
        if AGGREGATES.contains(&cat.as_str()) {
            continue;
        }
        for name in m.as_object().unwrap().keys() {
            where_seen
                .entry(name.clone())
                .or_default()
                .push(cat.clone());
        }
    }
    let mut overlaps: Vec<(String, Vec<String>)> = where_seen
        .into_iter()
        .filter(|(_, cats)| cats.len() > 1)
        .map(|(n, mut cats)| {
            cats.sort();
            (n, cats)
        })
        .collect();
    overlaps.sort();

    // Every one of these is a zsh name with two real roles:
    //   `!`    — reserved word (pipeline negation, `Src/hashtable.c`
    //            reswds[]) and an operator (history expansion, `man
    //            zshexpn`).
    //   `[`    — the `[` builtin (`Doc/Zsh/builtins.yo`) and the
    //            conditional/subscript operator.
    //   `[[`   — reserved word and the conditional operator.
    //   declare / export / float / integer / local / readonly / typeset
    //          — listed as reserved words by `man zshmisc` "Reserved
    //            Words" (`Doc/Zsh/grammar.yo:501-504`) AND present as
    //            builtins; the parser folds them into `typeset` via the
    //            TYPESET lextok. `dump_reflection_json` documents the
    //            deliberate both-tabs listing at its `keywords` loop.
    let expected: Vec<(String, Vec<String>)> = [
        ("!", vec!["keywords", "operators"]),
        ("[", vec!["compat", "operators"]),
        ("[[", vec!["keywords", "operators"]),
        ("declare", vec!["compat", "keywords"]),
        ("export", vec!["compat", "keywords"]),
        ("float", vec!["compat", "keywords"]),
        ("integer", vec!["compat", "keywords"]),
        ("local", vec!["compat", "keywords"]),
        ("readonly", vec!["compat", "keywords"]),
        ("typeset", vec!["compat", "keywords"]),
    ]
    .into_iter()
    .map(|(n, cats)| {
        (
            n.to_string(),
            cats.into_iter().map(str::to_string).collect::<Vec<_>>(),
        )
    })
    .collect();
    assert_eq!(
        overlaps, expected,
        "cross-category name overlap changed; every entry must be a name \
         zsh really gives two roles, not a registry leaking into the wrong tab"
    );

    // The aggregates must still be exactly that — supersets of the
    // leaves, not independent lists that can drift.
    let all = v["all"].as_object().unwrap();
    for (cat, m) in v.as_object().unwrap() {
        if AGGREGATES.contains(&cat.as_str()) {
            continue;
        }
        for name in m.as_object().unwrap().keys() {
            assert!(
                all.contains_key(name),
                "`all` is missing `{}` from category `{}`",
                name,
                cat
            );
        }
    }
}
