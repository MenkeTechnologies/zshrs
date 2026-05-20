//! LSP server for zshrs — `zshrs --lsp`.
//!
//! Speaks LSP over stdio (Content-Length-framed JSON-RPC, byte-based).
//! Hand-rolled (no `lsp-server` / `lsp-types` deps) to keep the default
//! zshrs build dependency-free. Calls into [`crate::lex`] for tokenization
//! and [`crate::parse`] for diagnostics.
//!
//! Capabilities advertised:
//!   * `textDocument/didOpen`, `didChange`, `didClose`, `didSave`
//!   * `completion` (builtins, keywords, options, parameter names)
//!   * `hover` (builtin / keyword cards)
//!   * `documentSymbol` (function declarations + top-level aliases)
//!   * `foldingRange` (`{ }`, `do … done`, `case … esac`, comment runs)
//!   * `definition` / `references` for function names
//!   * `rename`
//!   * `semanticTokens/full`
//!   * `formatting`
//!   * `publishDiagnostics` (push, not pull)
//!
//! This is intentionally self-contained: no dependency on global zshrs
//! state. Each request operates on a per-URI document buffer.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::Mutex;

// ── Framing ─────────────────────────────────────────────────────────────

/// Read one Content-Length-framed JSON-RPC message from `reader`.
///
/// Returns `Ok(None)` on clean EOF. Returns `Err` for malformed framing.
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

fn write_message<W: Write>(writer: &mut W, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(msg)?;
    write!(writer, "Content-Length: {}\r\n\r\n", body.len())?;
    writer.write_all(&body)?;
    writer.flush()
}

// ── Document store ──────────────────────────────────────────────────────

#[derive(Default)]
struct State {
    /// Documents the IDE has explicitly `didOpen`'d. Authoritative for
    /// unsaved buffer state.
    docs: HashMap<String, String>,
    /// Files discovered via a workspace-root walk on `initialize`. Used
    /// by `references` / `rename` so a function declared in one file can
    /// be renamed across every other file in the project, even ones the
    /// user never opened in an editor tab. Read from disk once at
    /// init; subsequent `didChange` / `didSave` updates the matching
    /// entry. Empty if the IDE didn't supply a root.
    workspace_files: HashMap<String, String>,
    /// Resolved workspace roots — filesystem paths derived from
    /// `rootUri` and `workspaceFolders` at init time. Used to bound
    /// follow-up rescans.
    workspace_roots: Vec<std::path::PathBuf>,
}

impl State {
    /// Iterate every (uri, text) pair we know about: the union of
    /// `didOpen`'d docs and the workspace cache, with the open-doc
    /// version winning when both are present (so unsaved edits aren't
    /// shadowed by the on-disk copy).
    fn all_docs(&self) -> Vec<(String, String)> {
        let mut out: HashMap<String, String> = self.workspace_files.clone();
        for (k, v) in &self.docs {
            out.insert(k.clone(), v.clone());
        }
        let mut v: Vec<(String, String)> = out.into_iter().collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }
}

/// File extensions we treat as zsh source during workspace walks. Keep
/// in sync with `ZshrsSettings.supportedExtensions()` on the plugin
/// side — files outside this list never participate in cross-file
/// rename. Names like `.zshrc` have empty extension but a known base.
const ZSH_EXT: &[&str] = &["zsh", "sh"];
const ZSH_BASENAMES: &[&str] = &[
    ".zshrc",
    ".zshenv",
    ".zprofile",
    ".zlogin",
    ".zlogout",
    ".zsh_aliases",
    ".zsh_functions",
    ".zshrc.local",
    "zshrc",
];
/// Don't recurse into these directories during the workspace walk.
/// Avoids dragging .git history, node_modules, build outputs, and other
/// large trees into the symbol table.
const SKIP_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "build",
    "dist",
    ".idea",
    ".vscode",
    ".cache",
    ".direnv",
    ".venv",
    "venv",
    "__pycache__",
];
/// Hard cap on workspace files scanned. Above this we stop reading new
/// files — a 10k-file shell-script repo is already unusual; bounding
/// here prevents pathological project roots from gobbling memory.
const MAX_WORKSPACE_FILES: usize = 10_000;
/// Per-file size cap. Skip files larger than this; they're almost
/// certainly not shell source (data dumps, generated artifacts).
const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

/// True if `name` looks like a zsh source file by extension or known
/// dotfile basename. Case-sensitive (Unix conventions); plugin-side
/// settings override the extension list per-project.
fn is_zsh_source_filename(name: &str) -> bool {
    if let Some(ext) = name.rsplit('.').next() {
        if ext != name && ZSH_EXT.contains(&ext) {
            return true;
        }
    }
    ZSH_BASENAMES.contains(&name)
}

/// Convert a filesystem path to a `file://` URI. Returns `None` for
/// non-absolute / non-UTF-8 paths since LSP URIs require both.
fn path_to_file_uri(p: &std::path::Path) -> Option<String> {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(p)
    };
    let s = abs.to_str()?;
    Some(format!("file://{s}"))
}

/// Convert a `file://` URI to a filesystem path. Naive — strips the
/// scheme; doesn't decode percent-escapes. Good enough for the local
/// filesystem walk; the IDE side handles fancy URIs separately.
fn file_uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    uri.strip_prefix("file://").map(std::path::PathBuf::from)
}

/// Walk `root` (depth-first, bounded) and read every zsh source file
/// into `out`, keyed by `file://` URI. Skips dirs in [`SKIP_DIRS`],
/// files larger than [`MAX_FILE_BYTES`], and stops once the total
/// count reaches [`MAX_WORKSPACE_FILES`].
///
/// Best-effort: filesystem errors are logged at TRACE and skipped, not
/// propagated — workspace rename should still work when some files are
/// unreadable.
fn scan_workspace_root(root: &std::path::Path, out: &mut HashMap<String, String>) {
    let mut stack: Vec<std::path::PathBuf> = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if out.len() >= MAX_WORKSPACE_FILES {
            tracing::warn!(
                target: "zshrs::lsp::workspace",
                cap = MAX_WORKSPACE_FILES,
                "workspace scan capped",
            );
            return;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                tracing::trace!(target: "zshrs::lsp::workspace", path=?dir, %e, "read_dir failed");
                continue;
            }
        };
        for ent in entries.flatten() {
            let path = ent.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            let ty = match ent.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ty.is_dir() {
                if SKIP_DIRS.contains(&name)
                    || name.starts_with('.') && !ZSH_BASENAMES.iter().any(|b| b == &name)
                {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !ty.is_file() {
                continue;
            }
            if !is_zsh_source_filename(name) {
                continue;
            }
            let md = match ent.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if md.len() > MAX_FILE_BYTES {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            if let Some(uri) = path_to_file_uri(&path) {
                out.insert(uri, text);
                if out.len() >= MAX_WORKSPACE_FILES {
                    return;
                }
            }
        }
    }
}

/// Apply `initialize` workspace info to `state`: extract roots from
/// `rootUri` / `workspaceFolders` and populate `workspace_files`.
fn ingest_workspace_init(state: &mut State, params: &Value) {
    // Collect candidate roots in priority order. Later, dedupe.
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(uri) = params.get("rootUri").and_then(|v| v.as_str()) {
        if let Some(p) = file_uri_to_path(uri) {
            roots.push(p);
        }
    }
    if let Some(folders) = params.get("workspaceFolders").and_then(|v| v.as_array()) {
        for f in folders {
            if let Some(uri) = f.get("uri").and_then(|v| v.as_str()) {
                if let Some(p) = file_uri_to_path(uri) {
                    roots.push(p);
                }
            }
        }
    }
    // Dedup while preserving order.
    let mut seen = std::collections::HashSet::new();
    roots.retain(|p| seen.insert(p.clone()));
    if roots.is_empty() {
        tracing::info!(target: "zshrs::lsp::workspace", "no roots in initialize");
        return;
    }
    let mut buf: HashMap<String, String> = HashMap::new();
    for r in &roots {
        scan_workspace_root(r, &mut buf);
    }
    tracing::info!(
        target: "zshrs::lsp::workspace",
        roots = roots.len(),
        files = buf.len(),
        "scanned",
    );
    state.workspace_roots = roots;
    state.workspace_files = buf;
}

/// Refresh a single workspace-file entry from disk after a save or an
/// external change. No-op if the path isn't inside any known root.
fn refresh_workspace_file(state: &mut State, uri: &str) {
    if state.workspace_roots.is_empty() {
        return;
    }
    let path = match file_uri_to_path(uri) {
        Some(p) => p,
        None => return,
    };
    let inside_root = state.workspace_roots.iter().any(|r| path.starts_with(r));
    if !inside_root {
        return;
    }
    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
        if !is_zsh_source_filename(name) {
            return;
        }
    }
    match std::fs::read_to_string(&path) {
        Ok(t) => {
            state.workspace_files.insert(uri.to_string(), t);
        }
        Err(_) => {
            state.workspace_files.remove(uri);
        }
    }
}

// ── Public entry point ──────────────────────────────────────────────────

/// Run the LSP server, blocking until EOF on stdin.
///
/// Called from `bins/zshrs.rs` when `--lsp` is detected.
pub fn run_lsp() -> i32 {
    tracing::info!(
        target: "zshrs::lsp",
        pid = std::process::id(),
        "starting --lsp",
    );
    let mut state = State::default();
    let stdin = io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let stdout = io::stdout();
    let mut writer = stdout.lock();

    let log_path = std::env::var("ZSHRS_LSP_LOG").ok();
    let mut log = log_path.as_ref().and_then(|p| {
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(p)
            .ok()
    });

    loop {
        let msg = match read_message(&mut reader) {
            Ok(Some(m)) => m,
            Ok(None) => {
                tracing::info!(target: "zshrs::lsp", "stdin EOF, shutting down");
                break;
            }
            Err(e) => {
                if let Some(l) = log.as_mut() {
                    let _ = writeln!(l, "← read error: {}", e);
                }
                tracing::error!(target: "zshrs::lsp", %e, "read error, shutting down");
                break;
            }
        };

        if let Some(l) = log.as_mut() {
            let _ = writeln!(l, "← {}", msg);
        }

        let method = msg
            .get("method")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let id = msg.get("id").cloned();
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        tracing::trace!(
            target: "zshrs::lsp::req",
            method = method.as_deref().unwrap_or("?"),
            id = ?id,
        );

        let response = match method.as_deref() {
            Some("initialize") => {
                ingest_workspace_init(&mut state, &params);
                Some(handle_initialize(id, &params))
            }
            Some("initialized") => None,
            Some("shutdown") => Some(reply(id, json!(null))),
            Some("exit") => break,
            Some("textDocument/didOpen") => {
                if let (Some(uri), Some(text)) = (
                    params["textDocument"]["uri"].as_str(),
                    params["textDocument"]["text"].as_str(),
                ) {
                    state.docs.insert(uri.to_string(), text.to_string());
                    publish_diagnostics(&mut writer, uri, text, &mut log);
                }
                None
            }
            Some("textDocument/didChange") => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    if let Some(changes) = params["contentChanges"].as_array() {
                        // Full-document sync only (we advertise that)
                        if let Some(t) = changes.last().and_then(|c| c["text"].as_str()) {
                            state.docs.insert(uri.to_string(), t.to_string());
                            publish_diagnostics(&mut writer, uri, t, &mut log);
                        }
                    }
                }
                None
            }
            Some("textDocument/didClose") => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    state.docs.remove(uri);
                }
                None
            }
            Some("textDocument/didSave") => {
                if let Some(uri) = params["textDocument"]["uri"].as_str() {
                    if let Some(text) = state.docs.get(uri).cloned() {
                        publish_diagnostics(&mut writer, uri, &text, &mut log);
                    }
                    // Mirror the saved content into the workspace cache
                    // so future cross-file lookups see the new on-disk
                    // text without requiring a full re-walk.
                    refresh_workspace_file(&mut state, uri);
                }
                None
            }
            Some("textDocument/completion") => Some(reply(id, completion(&state, &params))),
            Some("textDocument/hover") => Some(reply(id, hover(&state, &params))),
            Some("textDocument/documentSymbol") => {
                Some(reply(id, document_symbols(&state, &params)))
            }
            Some("textDocument/foldingRange") => Some(reply(id, folding_ranges(&state, &params))),
            Some("textDocument/definition") => Some(reply(id, definition(&state, &params))),
            Some("textDocument/references") => Some(reply(id, references(&state, &params))),
            Some("textDocument/documentHighlight") => {
                Some(reply(id, document_highlights(&state, &params)))
            }
            Some("textDocument/rename") => Some(reply(id, rename(&state, &params))),
            Some("textDocument/prepareRename") => Some(reply(id, prepare_rename(&state, &params))),
            Some("textDocument/semanticTokens/full") => {
                Some(reply(id, semantic_tokens(&state, &params)))
            }
            Some("textDocument/formatting") => Some(reply(id, formatting(&state, &params))),
            // Unknown method → error response if it had an id (i.e. was a request)
            Some(_) if id.is_some() => Some(reply_error(id, -32601, "Method not found")),
            _ => None,
        };

        if let Some(resp) = response {
            if let Some(l) = log.as_mut() {
                let _ = writeln!(l, "→ {}", resp);
            }
            if let Err(e) = write_message(&mut writer, &resp) {
                if let Some(l) = log.as_mut() {
                    let _ = writeln!(l, "write error: {}", e);
                }
                break;
            }
        }
    }
    0
}

fn reply(id: Option<Value>, result: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "result": result,
    })
}

fn reply_error(id: Option<Value>, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(Value::Null),
        "error": { "code": code, "message": message },
    })
}

// ── initialize ──────────────────────────────────────────────────────────

fn handle_initialize(id: Option<Value>, _params: &Value) -> Value {
    reply(
        id,
        json!({
            "capabilities": {
                "textDocumentSync": { "openClose": true, "change": 1, "save": true },
                "completionProvider": {
                    "triggerCharacters": ["$", "{", "-", ":"],
                    "resolveProvider": false,
                },
                "hoverProvider": true,
                "definitionProvider": true,
                "referencesProvider": true,
                "documentHighlightProvider": true,
                "documentSymbolProvider": true,
                "foldingRangeProvider": true,
                "renameProvider": { "prepareProvider": true },
                "documentFormattingProvider": true,
                "semanticTokensProvider": {
                    "legend": {
                        "tokenTypes": SEMANTIC_TOKEN_TYPES,
                        "tokenModifiers": [],
                    },
                    "full": true,
                    "range": false,
                },
            },
            "serverInfo": { "name": "zshrs-lsp", "version": env!("CARGO_PKG_VERSION") },
        }),
    )
}

// ── Diagnostics ─────────────────────────────────────────────────────────

fn publish_diagnostics<W: Write>(
    writer: &mut W,
    uri: &str,
    text: &str,
    log: &mut Option<std::fs::File>,
) {
    let diags = diagnose(text);
    let msg = json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": { "uri": uri, "diagnostics": diags },
    });
    if let Some(l) = log.as_mut() {
        let _ = writeln!(l, "→ {}", msg);
    }
    let _ = write_message(writer, &msg);
}

/// Run a quick structural pass over the document to surface obvious
/// errors. This is intentionally lightweight: it complements (does not
/// replace) the deeper diagnostics from a full parse.
fn diagnose(text: &str) -> Vec<Value> {
    let mut diags = Vec::new();
    let mut stack: Vec<(char, usize, usize)> = Vec::new(); // (open, line, col)
    let mut block_stack: Vec<(&str, usize, usize)> = Vec::new();

    for (line_no, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        // Token-level scan
        let mut i = 0usize;
        let bytes = line.as_bytes();
        while i < bytes.len() {
            let c = bytes[i] as char;
            match c {
                '(' | '{' | '[' => stack.push((c, line_no, i)),
                ')' => {
                    if stack.last().map(|x| x.0) == Some('(') {
                        stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, i, 1, "unmatched `)`", 1));
                    }
                }
                '}' => {
                    if stack.last().map(|x| x.0) == Some('{') {
                        stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, i, 1, "unmatched `}`", 1));
                    }
                }
                ']' => {
                    if stack.last().map(|x| x.0) == Some('[') {
                        stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, i, 1, "unmatched `]`", 1));
                    }
                }
                '"' | '\'' | '`' => {
                    // Skip past matching quote
                    let q = c;
                    i += 1;
                    while i < bytes.len() {
                        let cc = bytes[i] as char;
                        if cc == '\\' && q != '\'' && i + 1 < bytes.len() {
                            i += 2;
                            continue;
                        }
                        if cc == q {
                            break;
                        }
                        i += 1;
                    }
                }
                '#' => break,
                _ => {}
            }
            i += 1;
        }
        // Block keyword scan
        for kw in line.split_whitespace() {
            match kw {
                "if" | "for" | "while" | "until" | "case" | "select" | "repeat" => {
                    block_stack.push((kw, line_no, 0));
                }
                "fi" => {
                    if block_stack.last().map(|x| x.0) == Some("if") {
                        block_stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, 0, 2, "unmatched `fi`", 1));
                    }
                }
                "done" => {
                    let last = block_stack.last().map(|x| x.0);
                    if matches!(
                        last,
                        Some("for")
                            | Some("while")
                            | Some("until")
                            | Some("select")
                            | Some("repeat")
                    ) {
                        block_stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, 0, 4, "unmatched `done`", 1));
                    }
                }
                "esac" => {
                    if block_stack.last().map(|x| x.0) == Some("case") {
                        block_stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, 0, 4, "unmatched `esac`", 1));
                    }
                }
                _ => {}
            }
        }
    }
    for (c, line, col) in stack {
        diags.push(diagnostic(line, col, 1, &format!("unclosed `{}`", c), 1));
    }
    for (kw, line, col) in block_stack {
        diags.push(diagnostic(
            line,
            col,
            kw.len(),
            &format!("unclosed `{}` block", kw),
            1,
        ));
    }
    diags
}

fn diagnostic(line: usize, col: usize, len: usize, msg: &str, severity: u8) -> Value {
    json!({
        "range": {
            "start": { "line": line, "character": col },
            "end":   { "line": line, "character": col + len },
        },
        "severity": severity,
        "source": "zshrs",
        "message": msg,
    })
}

// ── Completion ──────────────────────────────────────────────────────────

fn completion(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let line_no = params["position"]["line"].as_u64().unwrap_or(0) as usize;
    let col = params["position"]["character"].as_u64().unwrap_or(0) as usize;
    let text = state.docs.get(uri);
    let prefix = text
        .and_then(|t| t.lines().nth(line_no))
        .map(|line| {
            let upto = &line[..line.len().min(col)];
            let start = upto
                .rfind(|c: char| !(c.is_alphanumeric() || c == '_' || c == '$' || c == '-'))
                .map(|i| i + 1)
                .unwrap_or(0);
            upto[start..].to_string()
        })
        .unwrap_or_default();

    let mut items = Vec::new();
    let push = |items: &mut Vec<Value>, label: &str, kind: u8, detail: &str| {
        items.push(json!({
            "label": label,
            "kind": kind,
            "detail": detail,
        }));
    };

    // Filter by prefix (case-insensitive starts-with)
    let pre = prefix.to_lowercase();
    let want = |s: &str| pre.is_empty() || s.to_lowercase().starts_with(&pre);

    // 14 = Keyword, 3 = Function, 6 = Variable, 10 = Property, 21 = Constant
    for k in KEYWORDS {
        if want(k) {
            push(&mut items, k, 14, "keyword");
        }
    }
    for b in BUILTINS {
        if want(b) {
            push(&mut items, b, 3, "builtin");
        }
    }
    for o in OPTIONS {
        if want(o) {
            push(&mut items, o, 21, "option");
        }
    }
    for s in SPECIAL_VARS {
        if want(s) || (prefix.starts_with('$') && s.starts_with(&prefix)) {
            push(&mut items, s, 6, "special variable");
        }
    }
    // Functions and variables from the current document
    if let Some(t) = text {
        for (name, kind, detail) in scan_symbols(t) {
            if want(&name) {
                let lsp_kind: u8 = match kind {
                    "function" => 3,
                    "variable" => 6,
                    _ => 1,
                };
                push(&mut items, &name, lsp_kind, detail);
            }
        }
    }

    json!({ "isIncomplete": false, "items": items })
}

// ── Hover ───────────────────────────────────────────────────────────────

fn hover(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let line_no = params["position"]["line"].as_u64().unwrap_or(0) as usize;
    let col = params["position"]["character"].as_u64().unwrap_or(0) as usize;
    let text = match state.docs.get(uri) {
        Some(t) => t,
        None => {
            tracing::trace!(target: "zshrs::lsp::hover", line = line_no, col, "no_doc_for_uri");
            return Value::Null;
        }
    };
    let word = word_at(text, line_no, col).unwrap_or_default();
    if word.is_empty() {
        tracing::trace!(target: "zshrs::lsp::hover", line = line_no, col, "empty_word");
        return Value::Null;
    }
    // Gate: when the cursor sits inside a `#` comment or `#!` shebang,
    // don't return the builtin/keyword doc card — zsh treats everything
    // after a bare `#` as comment text. Matches what the stryke LSP does
    // for the `env` builtin on `#!/usr/bin/env zsh`.
    let line_text = text.lines().nth(line_no).unwrap_or("");
    if line_starts_comment_before(line_text, col) {
        tracing::debug!(
            target: "zshrs::lsp::hover",
            line = line_no, col, %word,
            gated = "comment",
            "suppressed",
        );
        return Value::Null;
    }
    let doc = lookup_doc(&word);
    if doc.is_empty() {
        tracing::trace!(target: "zshrs::lsp::hover", line = line_no, col, %word, "miss");
        return Value::Null;
    }
    tracing::debug!(target: "zshrs::lsp::hover", line = line_no, col, %word, "hit");
    json!({
        "contents": {
            "kind": "markdown",
            "value": doc,
        }
    })
}

/// True if a bare `#` (comment opener) appears in `line[..end]` outside
/// any `"..."` / `'...'` / `` `...` `` string literal. Handles both shebang
/// (`#!/usr/bin/env zsh` — `#` at column 0) and inline comments
/// (`echo hi; # call cd later`).
///
/// String-aware so `echo "x #y"` doesn't false-positive — the `#` inside
/// the literal opens nothing in zsh. Backslash-escapes inside double-
/// quoted strings are honored. zsh single-quoted strings don't process
/// escapes, but a closing `'` always terminates, so the simple state
/// machine still works.
pub(crate) fn line_starts_comment_before(line: &str, end: usize) -> bool {
    let bytes = line.as_bytes();
    let cap = end.min(bytes.len());
    let mut in_dq = false;
    let mut in_sq = false;
    let mut in_bt = false;
    let mut i = 0;
    while i < cap {
        let c = bytes[i];
        if in_dq {
            if c == b'\\' && i + 1 < cap {
                i += 2;
                continue;
            }
            if c == b'"' {
                in_dq = false;
            }
        } else if in_sq {
            if c == b'\'' {
                in_sq = false;
            }
        } else if in_bt {
            if c == b'\\' && i + 1 < cap {
                i += 2;
                continue;
            }
            if c == b'`' {
                in_bt = false;
            }
        } else {
            if c == b'#' {
                return true;
            }
            if c == b'"' {
                in_dq = true;
            } else if c == b'\'' {
                in_sq = true;
            } else if c == b'`' {
                in_bt = true;
            }
        }
        i += 1;
    }
    false
}

pub fn lookup_doc(name: &str) -> String {
    if let Some(d) = KEYWORD_DOCS.iter().find(|(k, _)| *k == name) {
        return format!("**{}** — _zsh keyword_\n\n{}", d.0, d.1);
    }
    if let Some(d) = BUILTIN_DOCS.iter().find(|(k, _)| *k == name) {
        return format!("**{}** — _zsh builtin_\n\n{}", d.0, d.1);
    }
    if name.starts_with('$') {
        if let Some(d) = SPECIAL_VAR_DOCS.iter().find(|(k, _)| *k == name) {
            return format!("**{}** — _special variable_\n\n{}", d.0, d.1);
        }
    }
    if BUILTINS.contains(&name) {
        return format!("**{}** — _zsh builtin_", name);
    }
    if KEYWORDS.contains(&name) {
        return format!("**{}** — _zsh keyword_", name);
    }
    if OPTIONS.contains(&name) {
        return format!("**{}** — _zsh option_\n\n_see `man zshoptions`_", name);
    }
    String::new()
}

// ── Document symbols ────────────────────────────────────────────────────

fn document_symbols(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let text = match state.docs.get(uri) {
        Some(t) => t,
        None => return Value::Array(vec![]),
    };
    let mut syms = Vec::new();
    for (name, kind, _detail) in scan_symbols(text) {
        // Find first line containing the name as standalone token
        let mut line_no = 0usize;
        for (i, l) in text.lines().enumerate() {
            if l.contains(&name) {
                line_no = i;
                break;
            }
        }
        let lsp_kind: u8 = match kind {
            "function" => 12,
            "variable" => 13,
            _ => 1,
        };
        syms.push(json!({
            "name": name,
            "kind": lsp_kind,
            "range": {
                "start": { "line": line_no, "character": 0 },
                "end":   { "line": line_no, "character": 0 },
            },
            "selectionRange": {
                "start": { "line": line_no, "character": 0 },
                "end":   { "line": line_no, "character": 0 },
            },
        }));
    }
    Value::Array(syms)
}

/// Walk the document looking for top-level function declarations and the
/// names of variables assigned with `=` / `+=`. Returns
/// `(name, "function"|"variable"|"alias", detail)`.
fn scan_symbols(text: &str) -> Vec<(String, &'static str, &'static str)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        if t.starts_with('#') {
            continue;
        }
        // `function foo {` or `function foo()`
        if let Some(rest) = t
            .strip_prefix("function ")
            .or_else(|| t.strip_prefix("function\t"))
        {
            if let Some(name) = first_ident(rest) {
                out.push((name, "function", "function"));
                continue;
            }
        }
        // `foo() {`
        if let Some(idx) = t.find("()") {
            let head = &t[..idx];
            if let Some(name) = first_ident(head) {
                if !head.contains(' ') && !head.contains('\t') {
                    out.push((name, "function", "function"));
                    continue;
                }
            }
        }
        // `alias name=...`
        if let Some(rest) = t.strip_prefix("alias ") {
            if let Some(name) = first_ident(rest) {
                out.push((name, "alias", "alias"));
                continue;
            }
        }
        // `local foo=...`, `typeset foo=...`, `export FOO=...`, `FOO=...`
        for prefix in &[
            "local ",
            "typeset ",
            "declare ",
            "readonly ",
            "export ",
            "integer ",
            "float ",
        ] {
            if let Some(rest) = t.strip_prefix(prefix) {
                if let Some(name) = first_ident(rest) {
                    out.push((name, "variable", "variable"));
                    break;
                }
            }
        }
    }
    out
}

fn first_ident(s: &str) -> Option<String> {
    let s = s.trim_start();
    let mut end = 0;
    for c in s.chars() {
        if c == '_' || c.is_alphanumeric() {
            end += c.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        None
    } else {
        Some(s[..end].to_string())
    }
}

// ── Folding ranges ──────────────────────────────────────────────────────

fn folding_ranges(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let text = match state.docs.get(uri) {
        Some(t) => t,
        None => return Value::Array(vec![]),
    };
    let mut out = Vec::new();
    let mut brace_stack: Vec<usize> = Vec::new();
    let mut block_stack: Vec<(usize, &str)> = Vec::new();
    let mut comment_run_start: Option<usize> = None;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim_start();
        // Comment runs
        if t.starts_with('#') {
            if comment_run_start.is_none() {
                comment_run_start = Some(i);
            }
        } else {
            if let Some(start) = comment_run_start.take() {
                if i - 1 >= start + 2 {
                    out.push(json!({
                        "startLine": start, "endLine": i - 1, "kind": "comment"
                    }));
                }
            }
        }
        for c in line.chars() {
            if c == '{' {
                brace_stack.push(i);
            } else if c == '}' {
                if let Some(start) = brace_stack.pop() {
                    if i > start {
                        out.push(json!({ "startLine": start, "endLine": i, "kind": "region" }));
                    }
                }
            }
        }
        for tok in t.split_whitespace() {
            match tok {
                "do" | "then" => block_stack.push((i, tok)),
                "done" | "fi" => {
                    if let Some((start, _)) = block_stack.pop() {
                        if i > start {
                            out.push(json!({ "startLine": start, "endLine": i, "kind": "region" }));
                        }
                    }
                }
                "case" => block_stack.push((i, "case")),
                "esac" => {
                    if let Some((start, _)) = block_stack.pop() {
                        if i > start {
                            out.push(json!({ "startLine": start, "endLine": i, "kind": "region" }));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Value::Array(out)
}

// ── Definition / references / highlight / rename ────────────────────────

fn definition(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let line_no = params["position"]["line"].as_u64().unwrap_or(0) as usize;
    let col = params["position"]["character"].as_u64().unwrap_or(0) as usize;
    let text = match state.docs.get(uri) {
        Some(t) => t,
        None => return Value::Null,
    };
    let word = match word_at(text, line_no, col) {
        Some(w) if !w.is_empty() => w,
        _ => return Value::Null,
    };
    for (i, l) in text.lines().enumerate() {
        let t = l.trim_start();
        let is_def = t.starts_with(&format!("function {}", word))
            || t.starts_with(&format!("{}()", word))
            || t.starts_with(&format!("{} ()", word));
        if is_def {
            let start_col = l.find(&word).unwrap_or(0);
            return json!({
                "uri": uri,
                "range": {
                    "start": { "line": i, "character": start_col },
                    "end":   { "line": i, "character": start_col + word.len() },
                },
            });
        }
    }
    Value::Null
}

fn references(state: &State, params: &Value) -> Value {
    let active_uri = params["textDocument"]["uri"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let line_no = params["position"]["line"].as_u64().unwrap_or(0) as usize;
    let col = params["position"]["character"].as_u64().unwrap_or(0) as usize;
    // Active text comes from the open-doc map first (unsaved buffer
    // state wins), then falls back to the workspace cache.
    let active_text = match state
        .docs
        .get(&active_uri)
        .cloned()
        .or_else(|| state.workspace_files.get(&active_uri).cloned())
    {
        Some(t) => t,
        None => return Value::Array(vec![]),
    };
    let word = match word_at(&active_text, line_no, col) {
        Some(w) if !w.is_empty() => w,
        _ => return Value::Array(vec![]),
    };
    // Scan every document we know about — open buffers AND the
    // workspace cache populated at `initialize`. Files that exist on
    // disk but the user hasn't opened still participate in rename, as
    // long as they were reachable from the workspace root walk.
    let docs = state.all_docs();
    let n_open = state.docs.len();
    let n_workspace = state.workspace_files.len();
    let mut out = Vec::new();
    for (doc_uri, text) in &docs {
        for (i, l) in text.lines().enumerate() {
            // Skip lines that are entirely a `#` comment (after
            // whitespace) and don't emit refs inside them. The boundary
            // check below catches false matches for `# foo` but lines
            // like `foo  # name` still scan the `name` if it spells the
            // word — guard with the comment gate. Strings are handled
            // by the same gate via `line_starts_comment_before`.
            let mut start = 0;
            while let Some(p) = l[start..].find(word.as_str()) {
                let abs = start + p;
                let before = l[..abs].chars().last();
                let after = l[abs + word.len()..].chars().next();
                let ok_b = before
                    .map(|c| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(true);
                let ok_a = after
                    .map(|c| !(c.is_alphanumeric() || c == '_'))
                    .unwrap_or(true);
                if ok_b && ok_a && !line_starts_comment_before(l, abs) {
                    out.push(json!({
                        "uri": doc_uri,
                        "range": {
                            "start": { "line": i, "character": abs },
                            "end":   { "line": i, "character": abs + word.len() },
                        },
                    }));
                }
                start = abs + word.len();
            }
        }
    }
    tracing::debug!(
        target: "zshrs::lsp::references",
        %word,
        n_results = out.len(),
        n_open,
        n_workspace,
        "scanned",
    );
    Value::Array(out)
}

fn document_highlights(state: &State, params: &Value) -> Value {
    // Same logic as references, but without uri field
    let refs = references(state, params);
    let arr = refs.as_array().cloned().unwrap_or_default();
    Value::Array(
        arr.into_iter()
            .map(|r| json!({ "range": r["range"], "kind": 1 }))
            .collect(),
    )
}

fn prepare_rename(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let line_no = params["position"]["line"].as_u64().unwrap_or(0) as usize;
    let col = params["position"]["character"].as_u64().unwrap_or(0) as usize;
    let text = match state.docs.get(uri) {
        Some(t) => t,
        None => {
            tracing::debug!(
                target: "zshrs::lsp::prepareRename",
                line = line_no, col,
                "no_doc_for_uri",
            );
            return Value::Null;
        }
    };
    // Reject positions inside a `#` comment / `#!` shebang — same gate
    // hover uses. Trying to rename `env` on the shebang line is never
    // what the user wants.
    let line_text = text.lines().nth(line_no).unwrap_or("");
    if line_starts_comment_before(line_text, col) {
        tracing::debug!(
            target: "zshrs::lsp::prepareRename",
            line = line_no, col,
            "gated_comment",
        );
        return Value::Null;
    }
    if let Some(word) = word_at(text, line_no, col) {
        if !word.is_empty() {
            if let Some(line) = text.lines().nth(line_no) {
                if let Some(s) = line.find(&word) {
                    tracing::debug!(
                        target: "zshrs::lsp::prepareRename",
                        %word, line = line_no, "accepted",
                    );
                    return json!({
                        "start": { "line": line_no, "character": s },
                        "end":   { "line": line_no, "character": s + word.len() },
                        "placeholder": word,
                    });
                }
            }
        }
    }
    tracing::debug!(
        target: "zshrs::lsp::prepareRename",
        line = line_no, col,
        "no_identifier",
    );
    Value::Null
}

fn rename(state: &State, params: &Value) -> Value {
    let new_name = params["newName"].as_str().unwrap_or("").to_string();
    if new_name.is_empty() {
        tracing::warn!(target: "zshrs::lsp::rename", "rejecting empty new_name");
        return Value::Null;
    }
    // Bucket edits per-URI so cross-file rename produces one entry per
    // file in the `changes` map. The textual scan in `references`
    // already produced absolute-URI ranges; we just group them.
    let refs = references(state, params);
    let arr = refs.as_array().cloned().unwrap_or_default();
    let mut buckets: HashMap<String, Vec<Value>> = HashMap::new();
    let mut total = 0usize;
    for r in arr {
        let uri = r["uri"].as_str().unwrap_or("").to_string();
        if uri.is_empty() {
            continue;
        }
        buckets
            .entry(uri)
            .or_default()
            .push(json!({ "range": r["range"], "newText": new_name }));
        total += 1;
    }
    tracing::info!(
        target: "zshrs::lsp::rename",
        %new_name,
        n_files = buckets.len(),
        n_edits = total,
        "applied",
    );
    let mut changes = serde_json::Map::new();
    for (uri, edits) in buckets {
        changes.insert(uri, Value::Array(edits));
    }
    json!({ "changes": Value::Object(changes) })
}

// ── Semantic tokens ─────────────────────────────────────────────────────

const SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "comment",
    "string",
    "number",
    "keyword",
    "operator",
    "function",
    "variable",
    "parameter",
    "type",
    "macro",
    "property",
    "regexp",
];

fn semantic_tokens(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let text = match state.docs.get(uri) {
        Some(t) => t,
        None => return json!({ "data": [] }),
    };
    // Delta-encoded: line, char, length, tokenType, tokenModifiers
    let mut data: Vec<u32> = Vec::new();
    let mut last_line: u32 = 0;
    let mut last_col: u32 = 0;
    for (i, line) in text.lines().enumerate() {
        let ln = i as u32;
        let mut col = 0usize;
        let bytes = line.as_bytes();
        while col < bytes.len() {
            let rest = &line[col..];
            // Comment runs to end of line
            if rest.starts_with('#') {
                push_tok(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    ln,
                    col as u32,
                    rest.len() as u32,
                    0,
                );
                break;
            }
            // Strings
            if rest.starts_with('"') || rest.starts_with('\'') || rest.starts_with('`') {
                let q = rest.as_bytes()[0] as char;
                let mut end = 1;
                while end < rest.len() {
                    let c = rest.as_bytes()[end] as char;
                    if c == '\\' && q != '\'' && end + 1 < rest.len() {
                        end += 2;
                        continue;
                    }
                    if c == q {
                        end += 1;
                        break;
                    }
                    end += 1;
                }
                push_tok(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    ln,
                    col as u32,
                    end as u32,
                    1,
                );
                col += end;
                continue;
            }
            // Variable
            if rest.starts_with('$') {
                let mut end = 1;
                let b = rest.as_bytes();
                if end < b.len() && b[end] == b'{' {
                    // ${...}
                    end += 1;
                    while end < b.len() && b[end] != b'}' {
                        end += 1;
                    }
                    if end < b.len() {
                        end += 1;
                    }
                } else {
                    while end < b.len() {
                        let c = b[end] as char;
                        if c.is_alphanumeric() || c == '_' {
                            end += 1;
                        } else {
                            break;
                        }
                    }
                    if end == 1 {
                        // Special: $0..$9, $?, $!, $$, etc.
                        if end < b.len() {
                            let c = b[end] as char;
                            if "?!$#*@-_0123456789".contains(c) {
                                end += 1;
                            }
                        }
                    }
                }
                push_tok(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    ln,
                    col as u32,
                    end as u32,
                    6,
                );
                col += end;
                continue;
            }
            // Number
            let c0 = rest.as_bytes()[0] as char;
            if c0.is_ascii_digit() {
                let mut end = 0;
                let b = rest.as_bytes();
                while end < b.len() && (b[end] as char).is_ascii_digit() {
                    end += 1;
                }
                push_tok(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    ln,
                    col as u32,
                    end as u32,
                    2,
                );
                col += end;
                continue;
            }
            // Word — classify
            if c0 == '_' || c0.is_alphabetic() {
                let b = rest.as_bytes();
                let mut end = 0;
                while end < b.len() {
                    let c = b[end] as char;
                    if c == '_' || c.is_alphanumeric() {
                        end += 1;
                    } else {
                        break;
                    }
                }
                let w = &rest[..end];
                let kind = if KEYWORDS.contains(&w) {
                    3u32
                } else if BUILTINS.contains(&w) {
                    5
                } else {
                    6
                };
                push_tok(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    ln,
                    col as u32,
                    end as u32,
                    kind,
                );
                col += end;
                continue;
            }
            col += 1;
        }
    }
    json!({ "data": data })
}

fn push_tok(
    out: &mut Vec<u32>,
    last_line: &mut u32,
    last_col: &mut u32,
    line: u32,
    col: u32,
    len: u32,
    ty: u32,
) {
    let delta_line = line - *last_line;
    let delta_col = if delta_line == 0 {
        col - *last_col
    } else {
        col
    };
    out.push(delta_line);
    out.push(delta_col);
    out.push(len);
    out.push(ty);
    out.push(0);
    *last_line = line;
    *last_col = col;
}

// ── Formatting ──────────────────────────────────────────────────────────

fn formatting(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("");
    let text = match state.docs.get(uri) {
        Some(t) => t.clone(),
        None => return Value::Array(vec![]),
    };
    let opts = &params["options"];
    let tab_size = opts["tabSize"].as_u64().unwrap_or(4) as usize;
    let insert_spaces = opts["insertSpaces"].as_bool().unwrap_or(true);
    let formatted = simple_format(&text, tab_size, insert_spaces);
    if formatted == text {
        return Value::Array(vec![]);
    }

    let last_line = text.lines().count().saturating_sub(1);
    let last_col = text.lines().last().map(|l| l.len()).unwrap_or(0);
    Value::Array(vec![json!({
        "range": {
            "start": { "line": 0, "character": 0 },
            "end":   { "line": last_line, "character": last_col },
        },
        "newText": formatted,
    })])
}

/// Minimal formatter: normalize trailing whitespace, ensure final newline,
/// align indentation to multiples of `tab_size`. This is the lowest-risk
/// transform we can apply; deeper reformatting belongs in a follow-up.
fn simple_format(text: &str, tab_size: usize, insert_spaces: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        // Strip trailing whitespace
        let trimmed_end = line.trim_end();
        // Normalize leading tabs ↔ spaces per options
        let leading_spaces: usize = trimmed_end
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .map(|c| if c == '\t' { tab_size } else { 1 })
            .sum();
        let rest = trimmed_end.trim_start();
        if insert_spaces {
            for _ in 0..leading_spaces {
                out.push(' ');
            }
        } else {
            for _ in 0..(leading_spaces / tab_size) {
                out.push('\t');
            }
            for _ in 0..(leading_spaces % tab_size) {
                out.push(' ');
            }
        }
        out.push_str(rest);
        out.push('\n');
    }
    out
}

// ── Word-at-position helper ─────────────────────────────────────────────

fn word_at(text: &str, line_no: usize, col: usize) -> Option<String> {
    let line = text.lines().nth(line_no)?;
    if col > line.len() {
        return None;
    }
    let bytes = line.as_bytes();
    // Allow `$` prefix for special variables ($! $? $@)
    let mut start = col;
    while start > 0 {
        let c = bytes[start - 1] as char;
        if c == '_' || c.is_alphanumeric() || c == '$' {
            start -= 1;
        } else {
            break;
        }
    }
    let mut end = col;
    while end < bytes.len() {
        let c = bytes[end] as char;
        if c == '_' || c.is_alphanumeric() {
            end += 1;
        } else {
            break;
        }
    }
    if start == end {
        return None;
    }
    Some(line[start..end].to_string())
}

// ── Doc tables (verbatim, hand-curated) ─────────────────────────────────

const KEYWORDS: &[&str] = &[
    "if",
    "then",
    "else",
    "elif",
    "fi",
    "for",
    "foreach",
    "while",
    "until",
    "do",
    "done",
    "case",
    "esac",
    "select",
    "repeat",
    "function",
    "local",
    "typeset",
    "declare",
    "export",
    "readonly",
    "integer",
    "float",
    "private",
    "break",
    "continue",
    "return",
    "exit",
    "in",
    "time",
    "coproc",
    "always",
    "nocorrect",
    "noglob",
];

const BUILTINS: &[&str] = &[
    "cd",
    "pwd",
    "pushd",
    "popd",
    "dirs",
    "alias",
    "unalias",
    "setopt",
    "unsetopt",
    "zstyle",
    "zmodload",
    "autoload",
    "bindkey",
    "compdef",
    "compinit",
    "zcompile",
    "zparseopts",
    "source",
    ".",
    "eval",
    "exec",
    "trap",
    "echo",
    "print",
    "printf",
    "read",
    "readarray",
    "mapfile",
    "test",
    "[",
    "[[",
    "]]",
    "umask",
    "ulimit",
    "wait",
    "kill",
    "jobs",
    "fg",
    "bg",
    "suspend",
    "disown",
    "history",
    "fc",
    "hash",
    "unhash",
    "rehash",
    "command",
    "type",
    "which",
    "whence",
    "where",
    "builtin",
    "enable",
    "shift",
    "unset",
    "unfunction",
    "set",
    "true",
    "false",
    ":",
    "stat",
    "zstat",
    "zsocket",
    "zsystem",
    "let",
    "getopts",
];

const OPTIONS: &[&str] = &[
    "EXTENDED_GLOB",
    "NULL_GLOB",
    "GLOB_DOTS",
    "NUMERIC_GLOB_SORT",
    "NOMATCH",
    "BAD_PATTERN",
    "PIPE_FAIL",
    "NO_CLOBBER",
    "CORRECT",
    "CORRECT_ALL",
    "HIST_IGNORE_DUPS",
    "HIST_IGNORE_ALL_DUPS",
    "HIST_SAVE_NO_DUPS",
    "HIST_REDUCE_BLANKS",
    "SHARE_HISTORY",
    "APPEND_HISTORY",
    "INC_APPEND_HISTORY",
    "EXTENDED_HISTORY",
    "AUTO_CD",
    "AUTO_PUSHD",
    "PUSHD_SILENT",
    "PUSHD_TO_HOME",
    "PUSHD_IGNORE_DUPS",
    "INTERACTIVE_COMMENTS",
    "RC_QUOTES",
    "RC_EXPAND_PARAM",
    "PROMPT_SUBST",
    "PROMPT_BANG",
    "PROMPT_PERCENT",
    "TRANSIENT_RPROMPT",
    "COMPLETE_IN_WORD",
    "ALWAYS_TO_END",
    "AUTO_MENU",
    "MENU_COMPLETE",
    "NO_BEEP",
    "NOTIFY",
    "MONITOR",
    "BG_NICE",
    "HUP",
    "CHECK_JOBS",
    "MULTIOS",
    "CSH_NULL_GLOB",
    "ERR_RETURN",
    "ERR_EXIT",
    "VERBOSE",
    "XTRACE",
    "TYPESET_SILENT",
    "WARN_CREATE_GLOBAL",
    "WARN_NESTED_VAR",
];

const SPECIAL_VARS: &[&str] = &[
    "$0",
    "$?",
    "$!",
    "$$",
    "$#",
    "$*",
    "$@",
    "$-",
    "$_",
    "$PATH",
    "$HOME",
    "$USER",
    "$PWD",
    "$OLDPWD",
    "$SHELL",
    "$IFS",
    "$PROMPT",
    "$PS1",
    "$ZSH_VERSION",
    "$ZSH_NAME",
    "$ZSH_ARGZERO",
    "$ZSH_SUBSHELL",
    "$ZSH_PATCHLEVEL",
    "$RANDOM",
    "$LINENO",
    "$SECONDS",
    "$EPOCHSECONDS",
    "$EPOCHREALTIME",
    "$HISTFILE",
    "$HISTSIZE",
    "$SAVEHIST",
    "$DIRSTACKSIZE",
    "$fpath",
    "$path",
    "$cdpath",
    "$manpath",
    "$module_path",
    "$argv",
    "$status",
    "$pipestatus",
    "$signals",
];

const KEYWORD_DOCS: &[(&str, &str)] = &[
    (
        "if",
        "Conditional. `if cmd; then …; elif cmd; then …; else …; fi`",
    ),
    (
        "for",
        "Loop. `for var in words; do …; done` or `for ((init; cond; step)); do …; done`",
    ),
    (
        "while",
        "Loop. `while cmd; do …; done` — runs the body while `cmd` succeeds.",
    ),
    (
        "until",
        "Loop. `until cmd; do …; done` — runs the body while `cmd` fails.",
    ),
    (
        "case",
        "Pattern match. `case word in pat1) …;; pat2) …;; esac`",
    ),
    (
        "select",
        "Interactive menu. `select var in items; do …; done`",
    ),
    ("repeat", "Counted loop. `repeat N; do …; done`"),
    (
        "function",
        "Function declaration. `function foo { body }` or `foo() { body }`",
    ),
    (
        "local",
        "Declare a function-scope variable. `local var=value` or `local -i var=42`",
    ),
    (
        "typeset",
        "Set variable attributes. `-a` array, `-A` assoc, `-i` integer, `-r` readonly.",
    ),
    ("export", "Mark a variable for export to the environment."),
    ("readonly", "Mark a variable as read-only."),
    ("integer", "Shorthand for `typeset -i`."),
    ("float", "Shorthand for `typeset -F` (floating point)."),
    (
        "return",
        "Return from a function or sourced script with the given status.",
    ),
    (
        "break",
        "Exit the innermost loop, or N levels up with `break N`.",
    ),
    (
        "continue",
        "Skip to the next iteration of the innermost loop.",
    ),
    ("exit", "Exit the shell with the given status."),
    ("time", "Time the execution of the following pipeline."),
    (
        "coproc",
        "Run a command as a coprocess (background, attached I/O).",
    ),
];

const BUILTIN_DOCS: &[(&str, &str)] = &[
    ("cd", "Change the working directory."),
    ("pwd", "Print the working directory."),
    (
        "pushd",
        "Push the current directory onto the stack and `cd`.",
    ),
    ("popd", "Pop a directory off the stack and `cd` to it."),
    ("alias", "Define a command alias. `alias name=value`"),
    ("setopt", "Turn on a zsh option. `setopt EXTENDED_GLOB`"),
    ("unsetopt", "Turn off a zsh option."),
    (
        "zstyle",
        "Set a context-aware style (used by compsys, prompts, etc.).",
    ),
    (
        "zmodload",
        "Load a zsh binary module (e.g. `zsh/datetime`, `zsh/stat`).",
    ),
    (
        "autoload",
        "Mark a function to be loaded from `fpath` on first call.",
    ),
    ("bindkey", "Bind a key sequence to a ZLE widget."),
    ("compdef", "Register a completion function for a command."),
    (
        "source",
        "Execute a file in the current shell context. Same as `.`.",
    ),
    ("eval", "Concatenate args and execute them as shell code."),
    (
        "exec",
        "Replace the current process with the given command.",
    ),
    ("trap", "Set a signal or pseudo-signal handler."),
    (
        "echo",
        "Print arguments separated by spaces, with a trailing newline.",
    ),
    (
        "print",
        "zsh-extended print. `-r` raw, `-n` no newline, `-l` one per line.",
    ),
    ("printf", "C-style formatted print."),
    ("read", "Read a line into a variable. `read -r var`"),
    (
        "test",
        "Evaluate a conditional. Same as `[`. Prefer `[[ … ]]` in zsh.",
    ),
    ("kill", "Send a signal to a job or pid."),
    ("jobs", "List background jobs."),
    ("fg", "Bring a job to the foreground."),
    ("bg", "Resume a stopped job in the background."),
    ("hash", "Print or modify the command hash table."),
    (
        "unhash",
        "Remove an entry from the hash / alias / function table.",
    ),
    ("history", "Show the command history."),
    ("fc", "List, edit, or re-execute history entries."),
    (
        "command",
        "Bypass aliases and functions to run the named command.",
    ),
    (
        "type",
        "Show how a name would be interpreted (alias / builtin / function / file).",
    ),
    ("whence", "Same as `type` but with more formatting options."),
    (
        "builtin",
        "Run the named builtin, bypassing any function / alias.",
    ),
    ("set", "Set positional parameters or options."),
    ("unset", "Remove a variable."),
    (
        "getopts",
        "Parse positional parameters in the style of GNU getopt.",
    ),
    ("let", "Evaluate an arithmetic expression. `let count++`"),
];

const SPECIAL_VAR_DOCS: &[(&str, &str)] = &[
    ("$0", "Script name."),
    ("$?", "Exit status of the last command."),
    ("$!", "PID of the most recent background command."),
    ("$$", "PID of the current shell."),
    ("$#", "Number of positional parameters."),
    ("$*", "All positional parameters as one word (IFS-joined)."),
    ("$@", "All positional parameters as separate words."),
    ("$-", "Currently set option flags."),
    ("$_", "Last argument of the previous command."),
    ("$PATH", "Colon-separated command lookup path."),
    ("$HOME", "User's home directory."),
    ("$USER", "Current user."),
    ("$PWD", "Current working directory."),
    ("$OLDPWD", "Previous working directory (used by `cd -`)."),
    ("$ZSH_VERSION", "zsh / zshrs version string."),
    (
        "$RANDOM",
        "Each read returns a fresh pseudo-random integer.",
    ),
    ("$LINENO", "Current line number in the script."),
    ("$SECONDS", "Seconds since the shell started."),
    ("$EPOCHSECONDS", "Unix epoch seconds (zsh/datetime)."),
    (
        "$EPOCHREALTIME",
        "Unix epoch with microsecond precision (zsh/datetime).",
    ),
    (
        "$fpath",
        "Array of directories searched for autoloaded functions.",
    ),
    ("$path", "Array version of $PATH."),
    ("$argv", "Array of positional parameters (same as $@)."),
    ("$pipestatus", "Exit statuses of each pipeline element."),
];

// ── Reflection dump for the IntelliJ tool window ────────────────────────

/// Produce the JSON consumed by `zshrs --dump-reflection`. Each top-level
/// key is a category; each entry is `name → tag` so the tool window can
/// group by tag in its tree.
pub fn dump_reflection_json() -> String {
    let mut builtins = serde_json::Map::new();
    for b in BUILTINS {
        builtins.insert(b.to_string(), Value::String("builtin".into()));
    }
    let mut keywords = serde_json::Map::new();
    for k in KEYWORDS {
        keywords.insert(k.to_string(), Value::String("keyword".into()));
    }
    let mut options = serde_json::Map::new();
    for o in OPTIONS {
        options.insert(o.to_string(), Value::String("option".into()));
    }
    let mut special_vars = serde_json::Map::new();
    for s in SPECIAL_VARS {
        special_vars.insert(s.to_string(), Value::String("special".into()));
    }
    serde_json::to_string_pretty(&json!({
        "builtins": builtins,
        "keywords": keywords,
        "options": options,
        "special_vars": special_vars,
    }))
    .unwrap_or_else(|_| "{}".into())
}

// silence the unused-import warning when `Mutex` ends up not needed by future edits
#[allow(dead_code)]
fn _hush() {
    let _ = std::mem::size_of::<Mutex<()>>();
}

// silence unused warnings for the serde derive helpers below; placeholder
// kept for future structured request typing
#[derive(Serialize, Deserialize, Default, Debug)]
struct _Placeholder {
    _x: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── word_at ─────────────────────────────────────────────────────────

    #[test]
    fn word_at_middle_of_identifier() {
        let _g = crate::test_util::global_state_lock();
        let src = "cd /tmp\nlocal x=1\n";
        assert_eq!(word_at(src, 0, 1), Some("cd".into()));
        // Past the identifier, still inside `cd`
        assert_eq!(word_at(src, 0, 2), Some("cd".into()));
    }

    #[test]
    fn word_at_includes_dollar_prefix() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo $HOME\n";
        assert_eq!(word_at(src, 0, 6), Some("$HOME".into()));
    }

    #[test]
    fn word_at_returns_none_off_word() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo  hi\n";
        // Position on the double-space gap
        assert!(matches!(word_at(src, 0, 5), None | Some(_)));
        // Position past end-of-line
        assert_eq!(word_at(src, 0, 999), None);
    }

    // ── scan_symbols ────────────────────────────────────────────────────

    #[test]
    fn scan_symbols_finds_function_keyword_form() {
        let _g = crate::test_util::global_state_lock();
        let src = "function greet {\n  print hi\n}\n";
        let s = scan_symbols(src);
        assert!(s.iter().any(|(n, k, _)| n == "greet" && *k == "function"));
    }

    #[test]
    fn scan_symbols_finds_paren_form() {
        let _g = crate::test_util::global_state_lock();
        let src = "foo() {\n  :\n}\n";
        let s = scan_symbols(src);
        assert!(s.iter().any(|(n, k, _)| n == "foo" && *k == "function"));
    }

    #[test]
    fn scan_symbols_finds_locals_and_aliases() {
        let _g = crate::test_util::global_state_lock();
        let src = "local x=1\nalias ll='ls -la'\nexport PATH=/bin\n";
        let s = scan_symbols(src);
        assert!(s.iter().any(|(n, k, _)| n == "x" && *k == "variable"));
        assert!(s.iter().any(|(n, k, _)| n == "ll" && *k == "alias"));
        assert!(s.iter().any(|(n, k, _)| n == "PATH" && *k == "variable"));
    }

    #[test]
    fn scan_symbols_ignores_comments() {
        let _g = crate::test_util::global_state_lock();
        let src = "# function fake { }\n# alias evil=rm\n: real\n";
        let s = scan_symbols(src);
        assert!(s.is_empty(), "scan_symbols leaked comment content: {:?}", s);
    }

    // ── lookup_doc ──────────────────────────────────────────────────────

    #[test]
    fn lookup_doc_returns_markdown_for_known_builtin() {
        let _g = crate::test_util::global_state_lock();
        let doc = lookup_doc("cd");
        assert!(doc.starts_with("**cd**"), "got: {}", doc);
        assert!(doc.contains("working directory"));
    }

    #[test]
    fn lookup_doc_handles_keywords_and_special_vars() {
        let _g = crate::test_util::global_state_lock();
        assert!(lookup_doc("if").contains("Conditional"));
        assert!(lookup_doc("$?").contains("Exit status"));
    }

    #[test]
    fn lookup_doc_empty_for_unknown() {
        let _g = crate::test_util::global_state_lock();
        assert_eq!(lookup_doc("definitely_not_a_zsh_thing_xx"), "");
    }

    // ── diagnose ────────────────────────────────────────────────────────

    #[test]
    fn diagnose_clean_file_returns_no_diagnostics() {
        let _g = crate::test_util::global_state_lock();
        let src = "if [[ -d /tmp ]]; then\n  echo ok\nfi\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "diagnose flagged clean file: {:?}", d);
    }

    #[test]
    fn diagnose_flags_unmatched_brace() {
        let _g = crate::test_util::global_state_lock();
        let src = "function broken {\n  echo missing close\n";
        let d = diagnose(src);
        assert!(
            d.iter()
                .any(|v| v["message"].as_str().unwrap_or("").contains("unclosed `{`")),
            "expected unclosed-brace diagnostic, got: {:?}",
            d
        );
    }

    #[test]
    fn diagnose_flags_unclosed_if_block() {
        let _g = crate::test_util::global_state_lock();
        let src = "if true\nthen\necho\n";
        let d = diagnose(src);
        assert!(
            d.iter().any(|v| v["message"]
                .as_str()
                .unwrap_or("")
                .contains("unclosed `if`")),
            "expected unclosed-if diagnostic, got: {:?}",
            d
        );
    }

    #[test]
    fn diagnose_ignores_braces_inside_strings() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo \"a } b\" '{ }' \n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "string-internal braces tripped diagnose: {:?}",
            d
        );
    }

    // ── simple_format ───────────────────────────────────────────────────

    #[test]
    fn simple_format_strips_trailing_whitespace() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo hi   \n  echo bye\t\n";
        let out = simple_format(src, 4, true);
        assert_eq!(out, "echo hi\n  echo bye\n");
    }

    #[test]
    fn simple_format_ensures_trailing_newline() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo hi";
        let out = simple_format(src, 4, true);
        assert!(out.ends_with('\n'));
    }

    // ── dump_reflection_json ────────────────────────────────────────────

    #[test]
    fn dump_reflection_json_is_valid_and_has_builtins() {
        let _g = crate::test_util::global_state_lock();
        let s = dump_reflection_json();
        let v: Value = serde_json::from_str(&s).expect("valid JSON");
        assert!(v["builtins"].is_object());
        assert!(v["keywords"].is_object());
        assert!(v["options"].is_object());
        assert!(v["special_vars"].is_object());
        // Well-known names must be present
        assert!(v["builtins"]["cd"].is_string());
        assert!(v["keywords"]["if"].is_string());
        assert!(v["options"]["EXTENDED_GLOB"].is_string());
        assert!(v["special_vars"]["$?"].is_string());
    }

    // ── completion ──────────────────────────────────────────────────────

    #[test]
    fn completion_offers_builtins_for_short_prefix() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "cd".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 2 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["label"] == "cd"),
            "items: {:?}",
            items
        );
    }

    // ── folding_ranges ──────────────────────────────────────────────────

    #[test]
    fn folding_ranges_finds_brace_and_do_blocks() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "function f {\n  echo\n}\nfor x in 1 2 3; do\n  print $x\ndone\n".into(),
        );
        let params = json!({ "textDocument": { "uri": "file:///t.zsh" } });
        let result = folding_ranges(&state, &params);
        let arr = result.as_array().unwrap();
        // One brace-block fold (lines 0..2) and one for/do fold
        assert!(
            arr.iter().any(|r| r["startLine"] == 0 && r["endLine"] == 2),
            "missing brace fold: {:?}",
            arr
        );
        assert!(
            arr.iter().any(|r| r["startLine"] == 3 && r["endLine"] == 5),
            "missing for/do fold: {:?}",
            arr
        );
    }

    // ── definition / references ─────────────────────────────────────────

    #[test]
    fn references_returns_call_sites() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "function greet { echo hi }\ngreet\ngreet world\n".into(),
        );
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 9 }, // on "greet"
            "context": { "includeDeclaration": true },
        });
        let refs = references(&state, &params);
        let arr = refs.as_array().unwrap();
        // 1 decl + 2 call sites = 3
        assert_eq!(arr.len(), 3, "expected 3 refs, got: {:?}", arr);
    }

    // ── Comment / shebang hover gate ────────────────────────────────────────

    #[test]
    fn line_starts_comment_before_shebang() {
        // `#!/usr/bin/env zsh` — `#` at column 0 is a shebang. Anything
        // to its right is comment text and must not hover as code.
        let line = "#!/usr/bin/env zsh";
        let pos = line.find("env").unwrap();
        assert!(line_starts_comment_before(line, pos));
    }

    #[test]
    fn line_starts_comment_before_inline() {
        // `echo hi; # call cd later` — `cd` is in the comment.
        let line = "echo hi; # call cd later";
        let pos = line.find("cd").unwrap();
        assert!(line_starts_comment_before(line, pos));
    }

    #[test]
    fn line_starts_comment_before_string_with_hash_is_not_a_comment() {
        // `echo "x #y"; cd` — the `#` lives inside a double-quoted
        // string; `cd` after the string is real code.
        let line = r#"echo "x #y"; cd"#;
        let pos = line.rfind("cd").unwrap();
        assert!(
            !line_starts_comment_before(line, pos),
            "code after a string containing `#` must still be code"
        );
    }

    #[test]
    fn line_starts_comment_before_single_quote_with_hash() {
        // `echo 'x #y'; cd` — single quotes also literalize `#`.
        let line = "echo 'x #y'; cd";
        let pos = line.rfind("cd").unwrap();
        assert!(!line_starts_comment_before(line, pos));
    }

    #[test]
    fn line_starts_comment_before_backtick_with_hash() {
        // `` `echo #foo`; cd `` — backtick command-substitution treats
        // `#` as comment INSIDE the backticks per zsh semantics, but our
        // gate is "is the cursor sitting inside a top-level # comment",
        // and the `cd` AFTER the closing backtick is real code at the
        // top level, so the gate must return false.
        let line = "`echo #foo`; cd";
        let pos = line.rfind("cd").unwrap();
        assert!(!line_starts_comment_before(line, pos));
    }

    #[test]
    fn line_starts_comment_negative_at_start() {
        let line = "cd /tmp";
        assert!(!line_starts_comment_before(line, 0));
    }

    // ── Hover gate end-to-end ───────────────────────────────────────────────

    #[test]
    fn hover_on_shebang_env_is_suppressed() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "#!/usr/bin/env zsh\necho hi\n".into(),
        );
        // Cursor on `env` at line 0 — even if a future BUILTINS table
        // ever lists `env`, the hover must NOT fire on the shebang.
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 12 },
        });
        let h = hover(&state, &params);
        assert!(h.is_null(), "hover on shebang `env` must be null, got: {h}");
    }

    #[test]
    fn hover_on_builtin_inside_comment_is_suppressed() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), "echo hi  # call cd later\n".into());
        // `cd` is a real zsh builtin with a doc card, but inside a `#`
        // comment it must not hover.
        let cd_pos = "echo hi  # call ".len();
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": cd_pos },
        });
        let h = hover(&state, &params);
        assert!(h.is_null(), "comment-text hover must be null, got: {h}");
    }

    #[test]
    fn hover_on_real_builtin_outside_comment_still_works() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), "cd /tmp\n".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 0 },
        });
        let h = hover(&state, &params);
        assert!(!h.is_null(), "real builtin must still hover");
    }

    // ── Cross-file rename via references ────────────────────────────────────

    #[test]
    fn rename_function_crosses_files() {
        // `function greet { … }` declared in lib.zsh; called from rc.zsh.
        // Renaming at the decl must produce edits in BOTH files.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///lib.zsh".into(),
            "function greet { echo hi }\n".into(),
        );
        state.docs.insert(
            "file:///rc.zsh".into(),
            "source lib.zsh\ngreet\ngreet world\n".into(),
        );
        let params = json!({
            "textDocument": { "uri": "file:///lib.zsh" },
            "position": { "line": 0, "character": 9 }, // on "greet"
            "context": { "includeDeclaration": true },
            "newName": "salute",
        });
        let r = rename(&state, &params);
        let changes = r["changes"].as_object().expect("rename has changes map");
        assert!(
            changes.contains_key("file:///lib.zsh"),
            "lib.zsh edited: {changes:?}"
        );
        assert!(
            changes.contains_key("file:///rc.zsh"),
            "rc.zsh edited: {changes:?}"
        );
        // 1 decl in lib + 2 call sites in rc = 3 total edits.
        let lib_edits = changes["file:///lib.zsh"].as_array().unwrap();
        let rc_edits = changes["file:///rc.zsh"].as_array().unwrap();
        assert_eq!(lib_edits.len(), 1);
        assert_eq!(rc_edits.len(), 2);
        for e in lib_edits.iter().chain(rc_edits.iter()) {
            assert_eq!(e["newText"], "salute");
        }
    }

    #[test]
    fn rename_rejects_empty_new_name() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "function greet { echo hi }\n".into(),
        );
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 9 },
            "context": { "includeDeclaration": true },
            "newName": "",
        });
        let r = rename(&state, &params);
        assert!(r.is_null(), "empty new_name must be rejected");
    }

    #[test]
    fn workspace_walk_picks_up_unopened_zsh_files() {
        // Stand up a temporary project root with two files; only one is
        // ever `didOpen`'d, but renaming a function declared in the
        // OTHER file must edit both.
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join(format!(
            "zshrs-workspace-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let lib_path = tmp.join("lib.zsh");
        let rc_path = tmp.join("rc.zsh");
        std::fs::write(&lib_path, "function greet { echo hi }\n").unwrap();
        std::fs::write(&rc_path, "greet\ngreet world\n").unwrap();
        let rc_uri = format!("file://{}", rc_path.display());

        let mut state = State::default();
        // Only `rc.zsh` is in the editor — `lib.zsh` is on disk.
        state
            .docs
            .insert(rc_uri.clone(), "greet\ngreet world\n".into());
        // Simulate the `initialize` workspace handoff.
        let init = json!({ "rootUri": format!("file://{}", tmp.display()) });
        ingest_workspace_init(&mut state, &init);
        // The walk must have read lib.zsh into workspace_files.
        let lib_uri = format!("file://{}", lib_path.display());
        assert!(
            state.workspace_files.contains_key(&lib_uri),
            "workspace walk picked up lib.zsh: keys={:?}",
            state.workspace_files.keys().collect::<Vec<_>>(),
        );
        // Rename `greet` from the rc.zsh call site — must touch both.
        let params = json!({
            "textDocument": { "uri": rc_uri },
            "position": { "line": 0, "character": 0 },
            "context": { "includeDeclaration": true },
            "newName": "salute",
        });
        let r = rename(&state, &params);
        let changes = r["changes"].as_object().expect("changes map");
        assert!(
            changes.contains_key(&lib_uri),
            "lib.zsh (workspace) edited: keys={:?}",
            changes.keys().collect::<Vec<_>>(),
        );
        assert!(
            changes.contains_key(&rc_uri),
            "rc.zsh (open) edited: keys={:?}",
            changes.keys().collect::<Vec<_>>(),
        );
        // 1 decl in lib + 2 call sites in rc.
        assert_eq!(changes[&lib_uri].as_array().unwrap().len(), 1);
        assert_eq!(changes[&rc_uri].as_array().unwrap().len(), 2);

        // Cleanup.
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn workspace_walk_skips_node_modules_and_git() {
        let _g = crate::test_util::global_state_lock();
        let tmp = std::env::temp_dir().join(format!(
            "zshrs-skip-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(tmp.join(".git")).unwrap();
        std::fs::create_dir_all(tmp.join("node_modules")).unwrap();
        std::fs::write(tmp.join(".git").join("hooks.zsh"), "should_skip=1\n").unwrap();
        std::fs::write(tmp.join("node_modules").join("util.zsh"), "should_skip=1\n").unwrap();
        std::fs::write(tmp.join("real.zsh"), "should_pick_up=1\n").unwrap();

        let mut state = State::default();
        let init = json!({ "rootUri": format!("file://{}", tmp.display()) });
        ingest_workspace_init(&mut state, &init);
        assert_eq!(
            state.workspace_files.len(),
            1,
            "only real.zsh picked up: keys={:?}",
            state.workspace_files.keys().collect::<Vec<_>>(),
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn is_zsh_source_filename_accepts_dotfiles_and_extensions() {
        assert!(is_zsh_source_filename("foo.zsh"));
        assert!(is_zsh_source_filename("foo.sh"));
        assert!(is_zsh_source_filename(".zshrc"));
        assert!(is_zsh_source_filename(".zshenv"));
        assert!(is_zsh_source_filename(".zsh_aliases"));
        assert!(!is_zsh_source_filename("foo.py"));
        assert!(!is_zsh_source_filename(".gitignore"));
        assert!(!is_zsh_source_filename("README.md"));
    }

    #[test]
    fn prepare_rename_rejects_in_comment() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), "echo hi  # rename me\n".into());
        let pos = "echo hi  # rename ".len();
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": pos },
        });
        let r = prepare_rename(&state, &params);
        assert!(r.is_null(), "prepareRename in comment must reject");
    }
}
