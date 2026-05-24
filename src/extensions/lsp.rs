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
            Some("textDocument/codeAction") => Some(reply(id, code_actions(&state, &params))),
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
                "codeActionProvider": {
                    "codeActionKinds": [
                        "refactor.extract",
                    ],
                },
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
        // Token-level scan. Pairings tracked on `stack`:
        //   '('  — single paren            ')'
        //   '{'  — single brace            '}'
        //   '['  — single bracket          ']'
        //   'A'  — arithmetic `((`         `))`
        //   'D'  — conditional `[[`        `]]`
        let mut i = 0usize;
        let bytes = line.as_bytes();
        while i < bytes.len() {
            let c = bytes[i] as char;
            match c {
                '(' => {
                    // `((` opens an arithmetic expression — paired with `))`,
                    // not two single parens.
                    if bytes.get(i + 1) == Some(&b'(') {
                        stack.push(('A', line_no, i));
                        i += 2;
                        continue;
                    }
                    stack.push(('(', line_no, i));
                }
                ')' => {
                    // `))` closes arithmetic.
                    if bytes.get(i + 1) == Some(&b')')
                        && stack.last().map(|x| x.0) == Some('A')
                    {
                        stack.pop();
                        i += 2;
                        continue;
                    }
                    if stack.last().map(|x| x.0) == Some('(') {
                        stack.pop();
                    } else {
                        // Bare `)` inside an open `case ... esac` is a
                        // pattern-arm terminator, not a paren mismatch.
                        let in_case =
                            block_stack.iter().any(|(kw, _, _)| *kw == "case");
                        if !in_case {
                            diags.push(diagnostic(
                                line_no,
                                i,
                                1,
                                "unmatched `)`",
                                1,
                            ));
                        }
                    }
                }
                '{' => stack.push(('{', line_no, i)),
                '}' => {
                    if stack.last().map(|x| x.0) == Some('{') {
                        stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, i, 1, "unmatched `}`", 1));
                    }
                }
                '[' => {
                    // `[[` opens a zsh conditional expression — paired
                    // with `]]`, not two single brackets.
                    if bytes.get(i + 1) == Some(&b'[') {
                        stack.push(('D', line_no, i));
                        i += 2;
                        continue;
                    }
                    stack.push(('[', line_no, i));
                }
                ']' => {
                    if bytes.get(i + 1) == Some(&b']')
                        && stack.last().map(|x| x.0) == Some('D')
                    {
                        stack.pop();
                        i += 2;
                        continue;
                    }
                    if stack.last().map(|x| x.0) == Some('[') {
                        stack.pop();
                    } else {
                        diags.push(diagnostic(line_no, i, 1, "unmatched `]`", 1));
                    }
                }
                '\\' => {
                    // Backslash escapes the next char outside of strings —
                    // skip it so `\#`, `\$`, `\(`, `\)`, etc. don't mis-trip.
                    i += 2;
                    continue;
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
                '#' => {
                    // `#` starts a line comment only when preceded by
                    // whitespace, `;`, `&`, `|`, `(`, or BOL — otherwise
                    // it's part of `$#` (argc), `${#var}` (length), or
                    // similar parameter expansion and must not terminate
                    // the scan.
                    let prev = if i == 0 {
                        None
                    } else {
                        Some(bytes[i - 1] as char)
                    };
                    let is_comment_start = match prev {
                        None => true,
                        Some(p) => {
                            p.is_whitespace()
                                || p == ';'
                                || p == '&'
                                || p == '|'
                                || p == '('
                        }
                    };
                    if is_comment_start {
                        break;
                    }
                }
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
    let line_text = text.lines().nth(line_no).unwrap_or("");
    // Use the same identifier-span rule as `word_at` so the gate sees
    // exactly the same byte range the doc card would render — keeps the
    // gate honest when the cursor lands on the trailing edge of a word.
    let (word_start, word_end) = word_span_at(line_text, col).unwrap_or((col, col));
    let gate = classify_hover_position(line_text, word_start, word_end);
    if gate != HoverGate::Code {
        tracing::debug!(
            target: "zshrs::lsp::hover",
            line = line_no, col, %word,
            gated = ?gate,
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

/// Why hover was suppressed at a given cursor position. Returned by
/// [`classify_hover_position`] so the hover handler can log the exact
/// reason — turns "why didn't the doc card pop?" into a one-line tail
/// of `zshrs.log` instead of an LSP-protocol re-derivation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HoverGate {
    /// Normal code position — let the builtin / keyword / option lookup run.
    Code,
    /// Inside a `#` line comment or `#!` shebang.
    Comment,
    /// Inside a string literal (`"..."`, `'...'`, or backtick). Cursor
    /// on a word that happens to spell a builtin must NOT pop the
    /// builtin doc — `"cd to dir"` in a string is the literal text, not
    /// the `cd` builtin. Note: zsh `"${var}"` interpolation IS code,
    /// and `${cd}` should still hover on `cd` — see
    /// [`position_inside_string_literal`] for the interpolation logic.
    StringLiteral,
}

/// True when the identifier at `[start, end)` falls inside a single-
/// line string literal (`"..."`, `'...'`, or backtick) AND outside any
/// `${EXPR}` parameter expansion. Walks the line from byte 0 tracking
/// string-quote state and interpolation depth so the interior of
/// `"path = ${HOME}/x"` is treated as Code (hover should fire on HOME),
/// while bare `"cd"` keeps the StringLiteral classification (hover
/// should NOT fire on the literal text).
///
/// zsh-specific notes vs the stryke port:
///   - The interpolation opener is `${...}` (parameter expansion), not
///     stryke's `#{...}`. We track `$` immediately followed by `{` to
///     enter interpolation; nested `{`/`}` adjusts depth.
///   - zsh single-quoted strings don't expand at all, so the `${`
///     opener is only honored inside `"..."` and `` `...` ``.
///   - Backslash escapes are honored inside `"..."` and backticks; not
///     in `'...'` (where `\` is literal).
fn position_inside_string_literal(line_text: &str, start: usize, end: usize) -> bool {
    let bytes = line_text.as_bytes();
    // `$NAME` inside a double-quoted / backtick string is code (a
    // parameter reference, expanded at runtime), not opaque text —
    // hovering should pop the doc for the variable. Same rule as the
    // semantic-tokens kind-aware mask. Two cases:
    //   1. `word_span_at` included the `$` sigil (span starts with `$`).
    //   2. Span is identifier-only (e.g. cursor mid-name) and the
    //      byte immediately before is `$`.
    let cap = end.min(bytes.len());
    if start < cap && bytes[start] == b'$'
        && bytes[start + 1..cap]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        return false;
    }
    if start > 0 && start < cap && bytes[start - 1] == b'$' {
        let span_ok = bytes[start..cap]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_');
        if span_ok {
            return false;
        }
    }
    let limit = start.min(bytes.len());
    let mut i = 0;
    let mut in_str: Option<u8> = None;
    let mut interp_depth: i32 = 0;
    while i < limit {
        let c = bytes[i];
        // Inside `${...}` interpolation — track nested braces and exit
        // when depth returns to 0.
        if interp_depth > 0 {
            match c {
                b'{' => interp_depth += 1,
                b'}' => interp_depth -= 1,
                _ => {}
            }
            i += 1;
            continue;
        }
        if let Some(q) = in_str {
            if (q == b'"' || q == b'`') && c == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            // `${` opens a code-context interpolation inside `"..."`
            // and `` `...` ``. Single-quoted strings don't expand.
            if (q == b'"' || q == b'`')
                && c == b'$'
                && i + 1 < bytes.len()
                && bytes[i + 1] == b'{'
            {
                interp_depth = 1;
                i += 2;
                continue;
            }
            if c == q {
                in_str = None;
            }
            i += 1;
            continue;
        }
        match c {
            b'#' => return false,
            b'"' | b'\'' | b'`' => in_str = Some(c),
            _ => {}
        }
        i += 1;
    }
    // Cursor inside `${...}` — that's real code, not string text.
    if interp_depth > 0 {
        return false;
    }
    if in_str.is_none() {
        return false;
    }
    // Inside an open quote at `start`. The identifier is in-string
    // unless the closing quote sits BEFORE `end` AND we walk back to
    // out-of-string before `end`. Cheap approximation: the identifier
    // is fully inside the string when the same quote doesn't reappear
    // in `[start, end)`.
    let q = in_str.unwrap();
    let mut j = start;
    while j < end.min(bytes.len()) {
        if (q == b'"' || q == b'`') && bytes[j] == b'\\' && j + 1 < bytes.len() {
            j += 2;
            continue;
        }
        if bytes[j] == q {
            return false;
        }
        j += 1;
    }
    true
}

/// Classify the identifier at `[start, end)` of `line_text` for hover
/// suppression. Exposed so [`hover`] can log the decision and tests can
/// pin every case without faking up a whole document.
pub(crate) fn classify_hover_position(line_text: &str, start: usize, end: usize) -> HoverGate {
    if line_starts_comment_before(line_text, start) {
        return HoverGate::Comment;
    }
    if position_inside_string_literal(line_text, start, end) {
        return HoverGate::StringLiteral;
    }
    HoverGate::Code
}

/// Return the byte span `[start, end)` of the identifier touching `col`
/// on `line_text`. Mirrors the walk in [`word_at`] but returns the span
/// instead of the slice — needed by [`classify_hover_position`] so the
/// gate sees the same range the doc card would render.
fn word_span_at(line_text: &str, col: usize) -> Option<(usize, usize)> {
    let bytes = line_text.as_bytes();
    if col > bytes.len() {
        return None;
    }
    // Phase 1: strict identifier walk (same as `word_at`).
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
    // Phase 2: extend through `-IDENT` segments for zsh function/command
    // names. Skipped when this is a parameter expansion (`$var` or
    // `${var…}`) since variable names forbid `-` per `iident`.
    let is_dollar_var = bytes[start] == b'$';
    let in_braced = start > 0 && bytes[start - 1] == b'{';
    if !is_dollar_var && !in_braced {
        while end < bytes.len() && bytes[end] == b'-' {
            let mut p = end + 1;
            while p < bytes.len() {
                let c = bytes[p] as char;
                if c == '_' || c.is_alphanumeric() {
                    p += 1;
                } else {
                    break;
                }
            }
            if p > end + 1 {
                end = p;
            } else {
                break;
            }
        }
        while start > 1 && bytes[start - 1] == b'-' {
            let mut p = start - 1;
            while p > 0 {
                let c = bytes[p - 1] as char;
                if c == '_' || c.is_alphanumeric() {
                    p -= 1;
                } else {
                    break;
                }
            }
            if p < start - 1 {
                start = p;
            } else {
                break;
            }
        }
    }
    Some((start, end))
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
/// True if byte position `end` on `line` is inside a string literal
/// (`"..."`, `'...'`, `` `...` ``) OR if a `#` line-comment has started
/// before `end`. Used by `references` / `rename` to suppress textual
/// matches that occur inside string content or comment text — those
/// are not real code references and should not surface in Find Usages.
pub(crate) fn line_position_inside_string_or_comment(line: &str, end: usize) -> bool {
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
        } else if c == b'#' {
            return true;
        } else if c == b'"' {
            in_dq = true;
        } else if c == b'\'' {
            in_sq = true;
        } else if c == b'`' {
            in_bt = true;
        }
        i += 1;
    }
    in_dq || in_sq || in_bt
}

/// Like [`line_position_inside_string_or_comment`] but ONLY flags
/// positions that zsh would NOT interpolate parameters in:
///   * inside `'...'` single-quoted strings (opaque to expansion)
///   * after a `#` line-comment
///
/// Use this when scanning for variable references — `$VAR` inside
/// `"..."` (and inside backticks) IS a real reference because zsh
/// interpolates parameters in both contexts.
pub(crate) fn line_position_inside_uninterpolating_context(line: &str, end: usize) -> bool {
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
        } else if c == b'#' {
            // `#` only opens a comment when preceded by whitespace /
            // statement boundary; `$#` (argc), `${#var}` (length),
            // etc. don't. We approximate by checking the previous
            // byte — bottoms out as "start of line" allowed.
            let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
            let starts_comment = match prev {
                None => true,
                Some(p) => matches!(p, b' ' | b'\t' | b';' | b'&' | b'|' | b'('),
            };
            if starts_comment {
                return true;
            }
        } else if c == b'"' {
            in_dq = true;
        } else if c == b'\'' {
            in_sq = true;
        } else if c == b'`' {
            in_bt = true;
        }
        i += 1;
    }
    // Only `in_sq` masks — double-quoted and backtick contexts
    // permit `$VAR` interpolation, so we DON'T mask them.
    in_sq
}

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
    // Upstream-yodl-derived tables come first — they carry the real
    // `man zshall` prose. The hand-curated stub tables below still
    // exist as a fallback for any entry the yo parser missed.
    //
    // Source files (regenerate via `scripts/gen_option_docs.py`):
    //   * Doc/Zsh/grammar.yo    → KEYWORD_DOCS    (`lookup_keyword_doc`)
    //   * Doc/Zsh/builtins.yo   → BUILTIN_DOCS    (`lookup_builtin_doc`)
    //   * Doc/Zsh/params.yo     → SPECIAL_VAR_DOCS (`lookup_special_var_doc`)
    //   * Doc/Zsh/options.yo    → OPTION_DOCS     (`lookup_option_doc`)
    if let Some((canon, body)) = crate::zsh_keyword_docs::lookup_keyword_doc(name) {
        return format!("**{}** — _zsh keyword_\n\n{}", canon, body);
    }
    if let Some((canon, body)) = crate::zsh_builtin_docs::lookup_builtin_doc(name) {
        return format!("**{}** — _zsh builtin_\n\n{}", canon, body);
    }
    // Special vars: try both `$VAR` and bare `VAR` forms — the yo
    // source stores names without `$`, but the LSP hover may pass
    // either spelling.
    let bare = name.strip_prefix('$').unwrap_or(name);
    if let Some((canon, body)) = crate::zsh_special_var_docs::lookup_special_var_doc(bare) {
        return format!("**${}** — _special variable_\n\n{}", canon, body);
    }
    if let Some((canon, body)) = crate::zsh_option_docs::lookup_option_doc(name) {
        return format!("**{}** — _zsh option_\n\n{}", canon, body);
    }
    // Hand-curated stub fallback for anything still uncovered.
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
    if let Some(d) = OPTION_DOCS_FALLBACK.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)) {
        return format!("**{}** — _zsh option_\n\n{}", d.0, d.1);
    }
    String::new()
}

/// Hand-curated docs for options that no upstream yodl `item(tt(...))`
/// block documents. The yodl alias-table-driven cascade covers 202/203
/// canonical `ZSH_OPTIONS_SET` entries; this fills the remainder so
/// every option gets real hover text instead of a `see man zshoptions`
/// stub.
const OPTION_DOCS_FALLBACK: &[(&str, &str)] = &[
    (
        "RESTRICTED",
        "Restricted-shell mode (equivalent to invoking zsh as `rzsh` or with `-r`).\
         \n\nDisables: `cd`, modifying `$PATH` / `$ENV` / `$SHELL`, `>` / `>>` redirects,\
         creating functions with the `function` keyword, `exec`-ing commands containing `/`,\
         `kill`-ing by pid, and several `setopt` toggles. Designed for sandboxed login shells\
         where the user must stay inside a curated command set. Once set, cannot be cleared\
         within the running shell.",
    ),
];

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
    let bare = word.strip_prefix('$').unwrap_or(&word);
    // Try AST first — scans every file in the workspace for a
    // `FuncDef` whose name matches, or a top-level assignment if the
    // cursor is on a `$var` reference. Falls through to the textual
    // single-file scan below on parse failure.
    if let Some(v) = definition_via_ast(state, &word, bare) {
        return v;
    }
    // Textual fallback (single file only, function defs only).
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

/// Cross-file AST-backed definition lookup. `word` is the raw cursor
/// word (may have a `$` prefix); `bare` is the same string with any
/// leading `$` stripped.
///
/// Returns:
/// * `Some(Location)` if a single matching decl is found
/// * `Some(Location[])` if multiple decls share the name (zsh allows
///   per-file shadowing; surface all and let the client pick)
/// * `None` if any active-file parse fails or no decl is found —
///   caller falls back to the textual scan.
fn definition_via_ast(state: &State, word: &str, bare: &str) -> Option<Value> {
    use crate::lsp_symbols::{find_ast_occurrences, SymbolKind};
    // `$x` cursor → look up Global decl. Bare cursor → look up Func
    // decl. (Locals don't cross files.) For bare words we also try
    // Global as a fallback (e.g. cursor on `FOO` in `echo FOO=1`).
    let kind = if word.starts_with('$') {
        SymbolKind::Global
    } else {
        SymbolKind::Func
    };
    let mut hits: Vec<Value> = Vec::new();
    for (uri, src) in state.all_docs() {
        let lines = find_ast_occurrences(&src, bare, kind.clone());
        for line in lines {
            // Only count decl-shaped occurrences. For Func kind that's
            // any `FuncDef.names` match (the walker only emits at the
            // FuncDef line for Func, plus call-site lines — discriminate
            // by re-reading the line). For Global it's any line that
            // starts an assignment / `local`/`typeset`.
            if !line_is_decl(&src, line, bare, &kind) {
                continue;
            }
            if let Some((start, end)) = find_first_word_col(&src, line, bare) {
                hits.push(json!({
                    "uri": uri,
                    "range": {
                        "start": { "line": line, "character": start },
                        "end":   { "line": line, "character": end },
                    },
                }));
            }
        }
    }
    if hits.is_empty() {
        // Fallback once for Global → Func or vice versa, in case the
        // cursor's `$` heuristic guessed wrong.
        let alt = if matches!(kind, SymbolKind::Global) {
            SymbolKind::Func
        } else {
            SymbolKind::Global
        };
        for (uri, src) in state.all_docs() {
            let lines = find_ast_occurrences(&src, bare, alt.clone());
            for line in lines {
                if !line_is_decl(&src, line, bare, &alt) {
                    continue;
                }
                if let Some((start, end)) = find_first_word_col(&src, line, bare) {
                    hits.push(json!({
                        "uri": uri,
                        "range": {
                            "start": { "line": line, "character": start },
                            "end":   { "line": line, "character": end },
                        },
                    }));
                }
            }
        }
    }
    match hits.len() {
        0 => None,
        1 => Some(hits.into_iter().next().unwrap()),
        _ => Some(Value::Array(hits)),
    }
}

/// True when `(line, name)` in `src` is a *declaration* site for the
/// given kind. Used to filter `find_ast_occurrences` results down to
/// just the decls (occurrences emit both decls AND refs).
fn line_is_decl(src: &str, line: u32, name: &str, kind: &crate::lsp_symbols::SymbolKind) -> bool {
    let l = match src.lines().nth(line as usize) {
        Some(l) => l,
        None => return false,
    };
    let t = l.trim_start();
    use crate::lsp_symbols::SymbolKind;
    match kind {
        SymbolKind::Func => {
            t.starts_with(&format!("function {}", name))
                || t.starts_with(&format!("function {} ", name))
                || t.starts_with(&format!("{}()", name))
                || t.starts_with(&format!("{} ()", name))
        }
        SymbolKind::Global | SymbolKind::Local => {
            // Any line that starts an assignment to `name`.
            // Forms: `name=value`, `local name=value`, `typeset name`,
            // `export name=value`, `name+=value`.
            let prefixes = [
                format!("{}=", name),
                format!("{}+=", name),
                format!("local {}", name),
                format!("typeset {}", name),
                format!("declare {}", name),
                format!("private {}", name),
                format!("export {}", name),
                format!("readonly {}", name),
                format!("integer {}", name),
                format!("float {}", name),
            ];
            prefixes.iter().any(|p| t.starts_with(p.as_str()))
        }
    }
}

/// Find the first whole-word occurrence of `name` on `src`'s `line`.
/// Returns `(start_col, end_col)` in UTF-16 code units approximated by
/// char count. Whole-word means surrounded by non-ident, non-`-` chars
/// (matches the boundary used in [`references`]).
fn find_first_word_col(src: &str, line: u32, name: &str) -> Option<(u32, u32)> {
    let l = src.lines().nth(line as usize)?;
    let mut start = 0;
    while let Some(p) = l[start..].find(name) {
        let abs = start + p;
        let before = l[..abs].chars().last();
        let after = l[abs + name.len()..].chars().next();
        let ok_b = before
            .map(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(true);
        let ok_a = after
            .map(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(true);
        if ok_b && ok_a && !line_position_inside_string_or_comment(l, abs) {
            return Some((abs as u32, (abs + name.len()) as u32));
        }
        start = abs + name.len();
    }
    None
}

/// Every whole-word column of `name` on `line`. Used by the AST refs
/// path to compute LSP ranges from AST-tracked (line, name) pairs.
///
/// `is_variable_ref` toggles the mask used to skip false matches
/// inside strings. Variable refs (`$VAR`) interpolate inside `"..."`
/// and backticks, so those contexts are KEPT; only `'...'` and
/// comments mask. Function-name matches (which are literal) always
/// mask quoted regions and comments.
fn find_all_word_cols(line_text: &str, name: &str) -> Vec<(u32, u32)> {
    find_all_word_cols_kinded(line_text, name, false)
}

fn find_all_word_cols_kinded(
    line_text: &str,
    name: &str,
    is_variable_ref: bool,
) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(p) = line_text[start..].find(name) {
        let abs = start + p;
        let before = line_text[..abs].chars().last();
        let after = line_text[abs + name.len()..].chars().next();
        let ok_b = before
            .map(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(true);
        let ok_a = after
            .map(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
            .unwrap_or(true);
        let masked = if is_variable_ref {
            line_position_inside_uninterpolating_context(line_text, abs)
        } else {
            line_position_inside_string_or_comment(line_text, abs)
        };
        if ok_b && ok_a && !masked {
            out.push((abs as u32, (abs + name.len()) as u32));
        }
        start = abs + name.len();
    }
    out
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
    // AST-backed cross-file path. Mirrors strykelang's SymbolTable
    // approach: parse the active file, resolve cursor → SymbolId,
    // then walk every workspace file looking for occurrences matching
    // the symbol's kind. Falls through to the textual scan below on
    // parse failure (e.g. mid-edit syntax error).
    if let Some(v) = references_via_ast(state, &active_uri, &active_text, line_no as u32, &word) {
        return v;
    }
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
                // Whole-word boundary: chars immediately before/after
                // the match must NOT be ident chars OR `-`. Excluding
                // `-` is what stops `daemon-ping` from spuriously
                // matching inside `daemon-ping-x` after the word-at
                // logic learned to include `-` in zsh function names.
                let ok_b = before
                    .map(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
                    .unwrap_or(true);
                let ok_a = after
                    .map(|c| !(c.is_alphanumeric() || c == '_' || c == '-'))
                    .unwrap_or(true);
                // Skip matches inside string literals OR after a `#`
                // comment-opener — string content and comment text are
                // not real code references and would surface as false
                // positives in Find Usages.
                if ok_b && ok_a && !line_position_inside_string_or_comment(l, abs) {
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

/// AST-backed cross-file find-references. Returns `None` if any of:
/// * the active file fails to parse
/// * the cursor doesn't resolve to a known symbol in that file
///
/// in which case the caller falls back to the textual scan. On
/// success returns the full LSP `Location[]` JSON array.
///
/// Algorithm (matches strykelang's SymbolTable approach):
/// 1. Build [`SymbolTable`] for the active file, resolve cursor
///    `(line, word)` → SymbolId → (name, kind).
/// 2. Active file: emit every line that the SymbolTable already
///    recorded as a decl or ref of that id.
/// 3. Other workspace files: kind-gated walk via
///    [`find_ast_occurrences`].  Locals don't cross files.
/// 4. Re-scan each (line, name) to compute the column range — the
///    AST loses column info, so this is the same trick stryke uses.
fn references_via_ast(
    state: &State,
    active_uri: &str,
    active_text: &str,
    cursor_line: u32,
    cursor_word: &str,
) -> Option<Value> {
    use crate::lsp_symbols::{find_ast_occurrences, SymbolKind, SymbolTable};

    // `$var` cursor → strip the `$` so the symbol-name match works.
    let bare = cursor_word.strip_prefix('$').unwrap_or(cursor_word);

    let active_table = SymbolTable::build(active_text)?;
    // Resolve cursor → (name, kind). If the active file declares the
    // symbol, use that. Otherwise look across the workspace for any
    // file that declares it (typical for `function daemon-ping` in
    // lib.zsh called from main.zsh — main.zsh has no decl).
    let (name, kind) = match active_table
        .symbol_at(cursor_line, bare)
        .and_then(|id| active_table.symbols.iter().find(|s| s.id == id))
    {
        Some(sym) => (sym.name.clone(), sym.kind.clone()),
        None => {
            let mut found: Option<SymbolKind> = None;
            'outer: for (other_uri, src) in state.all_docs() {
                if other_uri == active_uri {
                    continue;
                }
                let Some(t) = SymbolTable::build(&src) else {
                    continue;
                };
                for s in &t.symbols {
                    if s.name == bare
                        && matches!(s.kind, SymbolKind::Func | SymbolKind::Global)
                    {
                        found = Some(s.kind.clone());
                        break 'outer;
                    }
                }
            }
            let default_kind = if cursor_word.starts_with('$') {
                SymbolKind::Global
            } else {
                SymbolKind::Func
            };
            (bare.to_string(), found.unwrap_or(default_kind))
        }
    };

    let mut out: Vec<Value> = Vec::new();

    // Variables interpolate inside `"..."` and backticks; functions
    // don't. Pick the mask via `is_var` so `$VAR` refs inside
    // double-quoted strings (the common case for command flags, URLs,
    // messages) are surfaced as real references.
    let is_var = matches!(kind, SymbolKind::Global | SymbolKind::Local);

    // Active file occurrences. Prefer SymbolTable-resolved sites when
    // the symbol is declared here (gives us decl + same-file refs at
    // once); otherwise fall back to the AST occurrence walker.
    let active_lines: Vec<&str> = active_text.lines().collect();
    if let Some(id) = active_table.symbol_at(cursor_line, &name) {
        for (line, n) in active_table.occurrences(id) {
            if let Some(lt) = active_lines.get(line as usize) {
                for (s, e) in find_all_word_cols_kinded(lt, &n, is_var) {
                    out.push(json!({
                        "uri": active_uri,
                        "range": {
                            "start": { "line": line, "character": s },
                            "end":   { "line": line, "character": e },
                        },
                    }));
                }
            }
        }
    } else {
        let lines = find_ast_occurrences(active_text, &name, kind.clone());
        for line in lines {
            if let Some(lt) = active_lines.get(line as usize) {
                for (s, e) in find_all_word_cols_kinded(lt, &name, is_var) {
                    out.push(json!({
                        "uri": active_uri,
                        "range": {
                            "start": { "line": line, "character": s },
                            "end":   { "line": line, "character": e },
                        },
                    }));
                }
            }
        }
    }

    // Cross-file: only for symbols that cross file boundaries.
    if !matches!(kind, SymbolKind::Local) {
        for (uri, src) in state.all_docs() {
            if uri == active_uri {
                continue;
            }
            let lines = find_ast_occurrences(&src, &name, kind.clone());
            let src_lines: Vec<&str> = src.lines().collect();
            for line in lines {
                if let Some(lt) = src_lines.get(line as usize) {
                    for (s, e) in find_all_word_cols_kinded(lt, &name, is_var) {
                        out.push(json!({
                            "uri": uri,
                            "range": {
                                "start": { "line": line, "character": s },
                                "end":   { "line": line, "character": e },
                            },
                        }));
                    }
                }
            }
        }
    }

    tracing::debug!(
        target: "zshrs::lsp::references_ast",
        %name,
        ?kind,
        n_results = out.len(),
        "AST-resolved",
    );
    Some(Value::Array(out))
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
    let new_name_raw = params["newName"].as_str().unwrap_or("").to_string();
    if new_name_raw.is_empty() {
        tracing::warn!(target: "zshrs::lsp::rename", "rejecting empty new_name");
        return Value::Null;
    }
    // Defensive: strip any `::`-qualifier the client may have included in
    // newName. Earlier versions of the IntelliJ plugin (and other LSP
    // frontends — Helix, neovim, etc.) prefilled the Rename dialog with
    // a qualified form like `Demo::handle`; the user edited just the
    // suffix to `handle2`, but the dialog returned the WHOLE prefilled
    // string with the new suffix (`Demo::handle2`), and the server then
    // spliced that into every match site as the bare replacement —
    // producing nonsense like `Demo::Demo::handle2`. The rename target
    // is resolved from the cursor POSITION, not the dialog text; the new
    // name only needs to carry the new bare segment. Stripping here is
    // safe defense-in-depth across clients, and a no-op for callers who
    // already send bare. Note: zsh doesn't natively use `::` in function
    // names but compsys/autoload code and perl-style user conventions
    // do, so the same prefill bug surfaces in zsh codebases too.
    let new_name = match new_name_raw.rfind("::") {
        Some(idx) => {
            let bare = new_name_raw[idx + 2..].to_string();
            tracing::warn!(
                target: "zshrs::lsp::rename",
                %new_name_raw, %bare,
                "stripping `::` qualifier from new_name",
            );
            bare
        }
        None => new_name_raw,
    };
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
            // Strings.
            //
            // Single-quoted `'...'` is opaque to parameter expansion —
            // emit one big string token.
            //
            // Double-quoted `"..."` and backtick `` `...` `` interpolate
            // `$var` / `${var}` / `$(cmd)`. Walk the contents and emit
            // alternating string / variable sub-tokens so the editor can
            // colorize `$var` distinctly from the surrounding string.
            // Mirrors the strykelang plugin's behavior — the user's
            // mental model is "the dollar sigil glows, even inside a
            // string."
            if rest.starts_with('"') || rest.starts_with('\'') || rest.starts_with('`') {
                let q = rest.as_bytes()[0] as char;
                let bb = rest.as_bytes();
                // Locate the closing quote first; same logic as before
                // so the overall span doesn't change.
                let mut close = 1;
                while close < bb.len() {
                    let c = bb[close] as char;
                    if c == '\\' && q != '\'' && close + 1 < bb.len() {
                        close += 2;
                        continue;
                    }
                    if c == q {
                        close += 1;
                        break;
                    }
                    close += 1;
                }
                // Single-quoted: one opaque token.
                if q == '\'' {
                    push_tok(
                        &mut data,
                        &mut last_line,
                        &mut last_col,
                        ln,
                        col as u32,
                        close as u32,
                        1,
                    );
                    col += close;
                    continue;
                }
                // Interpolating string: emit segments.
                // `seg_start` is offset within `rest` of the current
                // un-emitted string segment (starts at 1 to include the
                // opening quote in the first string segment).
                let mut seg_start = 0usize;
                let mut p = 1usize;
                // Inner-end excludes the closing quote so we don't try
                // to interpolate past it; if the string was unterminated
                // (`close == bb.len()` and last char is NOT `q`), keep
                // going to end-of-line.
                let inner_end = if close > 0 && close <= bb.len() && bb.get(close - 1) == Some(&(q as u8)) {
                    close - 1
                } else {
                    close
                };
                let flush_string =
                    |data: &mut Vec<u32>, last_line: &mut u32, last_col: &mut u32,
                     col: usize, seg_start: usize, seg_end: usize| {
                        if seg_end > seg_start {
                            push_tok(
                                data,
                                last_line,
                                last_col,
                                ln,
                                (col + seg_start) as u32,
                                (seg_end - seg_start) as u32,
                                1, // string
                            );
                        }
                    };
                while p < inner_end {
                    let c = bb[p] as char;
                    // Skip escape sequences `\X` (backslash applies in
                    // double-quoted strings only when followed by `$`,
                    // `\``, `"`, `\\`, newline — but for highlighting
                    // we just skip 2 bytes to avoid `\$` triggering an
                    // interpolation marker).
                    if c == '\\' && q != '\'' && p + 1 < inner_end {
                        p += 2;
                        continue;
                    }
                    if c == '$' {
                        // Flush the string segment up to here.
                        flush_string(&mut data, &mut last_line, &mut last_col, col, seg_start, p);
                        // Scan the `$var` / `${var}` / `$(cmd)` / `$((expr))`
                        // expansion. Keep it simple: match the existing
                        // `Variable` arm below for plain `$var` and
                        // `${...}`; for `$(...)` / `$((...))` skip past
                        // the matching close paren counting depth.
                        let var_start = p;
                        let mut q2 = p + 1;
                        if q2 < inner_end && bb[q2] == b'{' {
                            // ${...} — find matching close brace, allowing
                            // one level of nested braces (e.g. `${(@)arr}`,
                            // `${x:-${y}}`).
                            let mut depth = 1i32;
                            q2 += 1;
                            while q2 < inner_end && depth > 0 {
                                match bb[q2] {
                                    b'{' => depth += 1,
                                    b'}' => depth -= 1,
                                    _ => {}
                                }
                                q2 += 1;
                            }
                        } else if q2 < inner_end && bb[q2] == b'(' {
                            // $(...) or $((...)) — count parens.
                            let mut depth = 1i32;
                            q2 += 1;
                            while q2 < inner_end && depth > 0 {
                                match bb[q2] {
                                    b'(' => depth += 1,
                                    b')' => depth -= 1,
                                    _ => {}
                                }
                                q2 += 1;
                            }
                        } else {
                            // Bare `$var` — alphanum / `_` body.
                            while q2 < inner_end {
                                let cc = bb[q2] as char;
                                if cc.is_alphanumeric() || cc == '_' {
                                    q2 += 1;
                                } else {
                                    break;
                                }
                            }
                            // Single-char specials: $0..$9, $?, $!, $$,
                            // $#, $*, $@, $-, $_.
                            if q2 == p + 1 && q2 < inner_end {
                                let cc = bb[q2] as char;
                                if "?!$#*@-_0123456789".contains(cc) {
                                    q2 += 1;
                                }
                            }
                        }
                        if q2 > var_start + 1 {
                            // Emit as `variable` (token type 6).
                            push_tok(
                                &mut data,
                                &mut last_line,
                                &mut last_col,
                                ln,
                                (col + var_start) as u32,
                                (q2 - var_start) as u32,
                                6,
                            );
                            seg_start = q2;
                            p = q2;
                            continue;
                        }
                        // Lone `$` (no name follows) — let it stay in
                        // the string segment.
                        p += 1;
                        continue;
                    }
                    p += 1;
                }
                // Trailing string segment (includes the closing quote).
                flush_string(&mut data, &mut last_line, &mut last_col, col, seg_start, close);
                col += close;
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

// ── Code actions: Extract Variable / Constant / Parameter ──────────────
//
// Ported from `strykelang/strykelang/lsp_extras.rs::compute_code_actions`.
// Adaptations for zsh syntax:
//   - declaration:  `local NAME=value`           (no sigil, no `my`)
//   - constant:     `readonly NAME=value`        (no `frozen`)
//   - var reference: `$NAME` / `${NAME}`         (caller adds the `$`)
//   - param:        zsh `name() { … }` has no `(param)` list, so
//                   Extract Parameter prepends `local NAME=$1` and
//                   shifts all body references by one positional index.
//                   v1 is simpler: we just append `local NAME=$N` at
//                   the top of the body with `N = positional count + 1`.

fn code_actions(state: &State, params: &Value) -> Value {
    let uri = params["textDocument"]["uri"].as_str().unwrap_or("").to_string();
    let text = match state.docs.get(&uri).cloned() {
        Some(t) => t,
        None => return Value::Array(vec![]),
    };
    let r = &params["range"];
    let start_line = r["start"]["line"].as_u64().unwrap_or(0) as u32;
    let start_char = r["start"]["character"].as_u64().unwrap_or(0) as u32;
    let end_line = r["end"]["line"].as_u64().unwrap_or(0) as u32;
    let end_char = r["end"]["character"].as_u64().unwrap_or(0) as u32;

    let mut actions: Vec<Value> = Vec::new();
    let same_line = start_line == end_line;
    let nonempty = start_line != end_line || start_char != end_char;

    // Caret-only snap: if range is empty, expand to the word at cursor.
    let (eff_start_char, eff_end_char) = if !nonempty && same_line {
        let line_text = match text.lines().nth(start_line as usize) {
            Some(l) => l,
            None => return Value::Array(vec![]),
        };
        match snap_to_word_at_cursor(line_text, start_char) {
            Some((s, e)) => (s, e),
            None => return Value::Array(vec![]),
        }
    } else if same_line {
        (start_char, end_char)
    } else {
        return Value::Array(vec![]); // multi-line — v1 skips
    };

    if eff_end_char <= eff_start_char {
        return Value::Array(vec![]);
    }

    let line_text = match text.lines().nth(start_line as usize) {
        Some(l) => l,
        None => return Value::Array(vec![]),
    };
    let sel = match utf16_slice(line_text, eff_start_char, eff_end_char) {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Value::Array(vec![]),
    };

    let eff_range = json!({
        "start": { "line": start_line, "character": eff_start_char },
        "end":   { "line": start_line, "character": eff_end_char   },
    });

    // Wrap selection in `"..."` if it sits inside an interpolating
    // string (double-quoted or backtick) AND isn't already a self-
    // contained expression (`$foo` / already-quoted literal).
    let in_string = same_line_inside_interpolating_string(line_text, eff_start_char);
    let rhs = if in_string && needs_string_wrap_for_extraction(sel) {
        format!("\"{}\"", escape_for_double_quoted(sel))
    } else {
        sel.to_string()
    };

    let leading_ws: String = line_text
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    // Extract to Variable: `local EXTRACTED=<rhs>` above, replace with `$EXTRACTED`.
    actions.push(make_extract_action(
        &uri,
        &leading_ws,
        start_line,
        &eff_range,
        &rhs,
        "EXTRACTED",
        "local",
        "Extract to variable (`local NAME=…`)",
    ));
    actions.push(make_extract_action(
        &uri,
        &leading_ws,
        start_line,
        &eff_range,
        &rhs,
        "EXTRACTED",
        "readonly",
        "Extract to constant (`readonly NAME=…`)",
    ));

    Value::Array(actions)
}

fn make_extract_action(
    uri: &str,
    leading_ws: &str,
    line: u32,
    selection_range: &Value,
    rhs: &str,
    name: &str,
    decl_keyword: &str,
    title: &str,
) -> Value {
    let decl_line = format!("{leading_ws}{decl_keyword} {name}={rhs}\n");
    let insert_range = json!({
        "start": { "line": line, "character": 0 },
        "end":   { "line": line, "character": 0 },
    });
    let changes = json!({
        uri: [
            { "range": insert_range, "newText": decl_line },
            { "range": selection_range, "newText": format!("${name}") },
        ]
    });
    json!({
        "title": title,
        "kind": "refactor.extract",
        "edit": { "changes": changes },
    })
}

/// UTF-16 slice of a single line. LSP positions are UTF-16 code units;
/// we convert back to a `&str` byte slice for use as the selection
/// content.
fn utf16_slice(line_text: &str, start: u32, end: u32) -> Option<&str> {
    let mut u16_seen = 0u32;
    let mut s_byte: Option<usize> = None;
    let mut e_byte: Option<usize> = None;
    for (i, ch) in line_text.char_indices() {
        if u16_seen == start {
            s_byte = Some(i);
        }
        u16_seen += ch.len_utf16() as u32;
        if u16_seen == end {
            e_byte = Some(i + ch.len_utf8());
            break;
        }
    }
    let s = s_byte?;
    let e = e_byte.unwrap_or(line_text.len());
    line_text.get(s..e)
}

/// True if the LSP char column `col` (UTF-16) on `line_text` falls
/// inside an unclosed interpolating string (`"..."` or `` `...` ``).
/// Mirrors stryke's `same_line_selection_inside_interpolating_string`.
fn same_line_inside_interpolating_string(line_text: &str, col: u32) -> bool {
    let mut byte_cutoff = line_text.len();
    let mut u16_seen = 0u32;
    for (i, ch) in line_text.char_indices() {
        if u16_seen >= col {
            byte_cutoff = i;
            break;
        }
        u16_seen += ch.len_utf16() as u32;
    }
    let mut in_dq = false;
    let mut in_sq = false;
    let mut in_bt = false;
    let mut chars = line_text[..byte_cutoff].chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                chars.next();
            }
            '"' if !in_sq && !in_bt => in_dq = !in_dq,
            '\'' if !in_dq && !in_bt => in_sq = !in_sq,
            '`' if !in_dq && !in_sq => in_bt = !in_bt,
            _ => {}
        }
    }
    in_dq || in_bt
}

/// True when the extracted text needs to be wrapped in `"..."` for the
/// decl to be a valid expression. False for already-quoted literals
/// and bare sigiled variables.
fn needs_string_wrap_for_extraction(selection: &str) -> bool {
    let t = selection.trim();
    if t.is_empty() {
        return false;
    }
    if (t.starts_with('"') && t.ends_with('"'))
        || (t.starts_with('\'') && t.ends_with('\''))
    {
        return false;
    }
    // Bare `$VAR` / `${VAR}` — already an expression.
    if let Some(rest) = t.strip_prefix('$') {
        let body = rest.strip_prefix('{').and_then(|r| r.strip_suffix('}')).unwrap_or(rest);
        if !body.is_empty()
            && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            return false;
        }
    }
    true
}

fn escape_for_double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

/// Snap a caret-only cursor to a word-boundary span on the line.
/// Returns `(start_utf16, end_utf16)` columns or `None`.
fn snap_to_word_at_cursor(line_text: &str, cursor_col: u32) -> Option<(u32, u32)> {
    let mut byte_cur = line_text.len();
    let mut u16_seen = 0u32;
    for (i, ch) in line_text.char_indices() {
        if u16_seen >= cursor_col {
            byte_cur = i;
            break;
        }
        u16_seen += ch.len_utf16() as u32;
    }
    let is_word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';

    // Inside a string: snap to a $VAR or to a word run.
    if same_line_inside_interpolating_string(line_text, cursor_col) {
        let prev_char = line_text[..byte_cur].chars().next_back();
        let cur_char = line_text[byte_cur..].chars().next();
        if matches!(prev_char, Some('$')) || matches!(cur_char, Some('$')) {
            // Walk back to `$`, then forward over the var name.
            let mut start_byte = byte_cur;
            for (i, c) in line_text[..byte_cur].char_indices().rev() {
                if c == '$' {
                    start_byte = i;
                    break;
                }
                if !is_word_char(c) {
                    break;
                }
                start_byte = i;
            }
            if cur_char == Some('$') {
                start_byte = byte_cur;
            }
            let mut end_byte = start_byte;
            let mut iter = line_text[start_byte..].char_indices();
            if let Some((_, first)) = iter.next() {
                if first == '$' {
                    end_byte = start_byte + first.len_utf8();
                    for (i, c) in iter {
                        if !is_word_char(c) {
                            break;
                        }
                        end_byte = start_byte + i + c.len_utf8();
                    }
                }
            }
            if end_byte > start_byte {
                return Some((
                    byte_to_utf16_col(line_text, start_byte),
                    byte_to_utf16_col(line_text, end_byte),
                ));
            }
        }
        let mut start_byte = byte_cur;
        for (i, c) in line_text[..byte_cur].char_indices().rev() {
            if !is_word_char(c) {
                break;
            }
            start_byte = i;
        }
        let mut end_byte = byte_cur;
        for (i, c) in line_text[byte_cur..].char_indices() {
            if !is_word_char(c) {
                break;
            }
            end_byte = byte_cur + i + c.len_utf8();
        }
        if end_byte > start_byte {
            return Some((
                byte_to_utf16_col(line_text, start_byte),
                byte_to_utf16_col(line_text, end_byte),
            ));
        }
        return None;
    }

    // Outside a string: snap to an identifier, with leading `$`.
    let mut start_byte = byte_cur;
    for (i, c) in line_text[..byte_cur].char_indices().rev() {
        if !is_word_char(c) {
            break;
        }
        start_byte = i;
    }
    let mut end_byte = byte_cur;
    for (i, c) in line_text[byte_cur..].char_indices() {
        if !is_word_char(c) {
            break;
        }
        end_byte = byte_cur + i + c.len_utf8();
    }
    // Include a leading `$` if standalone.
    if start_byte > 0 {
        if let Some((idx, '$')) = line_text[..start_byte].char_indices().next_back() {
            let standalone = match line_text[..idx].chars().next_back() {
                None => true,
                Some(c) => !is_word_char(c),
            };
            if standalone {
                start_byte = idx;
            }
        }
    }
    if end_byte > start_byte {
        Some((
            byte_to_utf16_col(line_text, start_byte),
            byte_to_utf16_col(line_text, end_byte),
        ))
    } else {
        None
    }
}

fn byte_to_utf16_col(line_text: &str, byte_idx: usize) -> u32 {
    line_text[..byte_idx.min(line_text.len())]
        .encode_utf16()
        .count() as u32
}

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
    // Phase 1: strict identifier walk (`[A-Za-z0-9_]`). `$` is allowed
    // on the LEFT only — it's the parameter-expansion prefix marker.
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
    // Phase 2: zsh function/command names allow `-` (e.g. `daemon-ping`,
    // `daemon-job-submit`). Extend the word through `-NAME` segments at
    // both ends, but ONLY when this is not a parameter-expansion (which
    // forbids `-` in identifier chars per `Src/lex.c iident` /
    // `Src/params.c isident`). Discriminator:
    //   * `bytes[start] == '$'`         → bare `$var` parameter
    //   * `bytes[start - 1] == '{'`     → `${var…}` braced expansion;
    //                                     inside `${var-default}` the
    //                                     `-` is the default-value
    //                                     operator, not part of the name
    let is_dollar_var = bytes[start] == b'$';
    let in_braced = start > 0 && bytes[start - 1] == b'{';
    if !is_dollar_var && !in_braced {
        // Extend right through `-IDENT` segments.
        while end < bytes.len() && bytes[end] == b'-' {
            let mut p = end + 1;
            while p < bytes.len() {
                let c = bytes[p] as char;
                if c == '_' || c.is_alphanumeric() {
                    p += 1;
                } else {
                    break;
                }
            }
            if p > end + 1 {
                end = p;
            } else {
                break;
            }
        }
        // Extend left through `IDENT-` segments.
        while start > 1 && bytes[start - 1] == b'-' {
            let mut p = start - 1;
            while p > 0 {
                let c = bytes[p - 1] as char;
                if c == '_' || c.is_alphanumeric() {
                    p -= 1;
                } else {
                    break;
                }
            }
            if p < start - 1 {
                start = p;
            } else {
                break;
            }
        }
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
    // Compound-statement sub-keywords. Upstream zsh documents each
    // compound (`if`, `for`, `case`, …) as one `item(...)` block, so
    // the sub-keywords (`then`/`else`/`elif`/`fi`/`do`/`done`/`in`/
    // `esac`) get no per-keyword `item` and fall through to the hand
    // fallback. Each entry points the reader at the parent compound.
    ("then", "Body separator for `if`/`elif`. `if cmd; then body; fi`"),
    ("else", "Alternative branch for `if`. `if cmd; then a; else b; fi`"),
    ("elif", "Alternative test in an `if` chain. `if a; then …; elif b; then …; fi`"),
    ("do",  "Body-introducer for `for`/`while`/`until`/`select`/`repeat`. `for v in …; do body; done`"),
    ("esac", "Closes a `case` statement. `case word in pat) …;; esac`"),
    ("in",  "Word-list introducer for `for` and `case`. `for v in a b c; do …; done`"),
    (
        "declare",
        "Alias for `typeset`. Set variable attributes. `-a` array, `-A` assoc, `-i` integer, `-r` readonly.",
    ),
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
    // ── Builtins that have no per-name `item(tt(...))(…)` block in any
    // upstream yodl source. Most are simple aliases for documented
    // builtins; a few (`hashinfo`, `mem`, `patdebug`) are debug/internal
    // entry points. The `zf_*` family are zftp companion functions
    // documented as a group in `Functions/Zftp/README` rather than per-name.
    (
        ":",
        "Null command. Returns true. Side-effects of argument expansion still happen.",
    ),
    (
        "[",
        "Alias for `test`. `[ expr ]` — POSIX conditional. Prefer `[[ expr ]]` in zsh.",
    ),
    ("bye", "Alias for `exit`. Exit the shell with the given status."),
    ("chdir", "Alias for `cd`. Change the working directory."),
    (
        "compctl",
        "Old completion control (compctl mechanism). Largely superseded by `compdef` / compsys.",
    ),
    ("declare", "Alias for `typeset`. Set variable attributes."),
    (
        "hashinfo",
        "Print internal hash-table statistics. Debug builtin in `zsh/parameter`-adjacent code.",
    ),
    (
        "mem",
        "Print zsh memory-allocator statistics. Debug builtin compiled only with `--enable-zsh-mem`.",
    ),
    (
        "noglob",
        "Precommand modifier. Disable filename generation for the next command. `noglob ls *.tmp`",
    ),
    (
        "patdebug",
        "Print pattern-matcher internals for a glob/regex. Debug builtin from `zsh/pattern`.",
    ),
    ("r", "Re-execute the previous command. Shorthand for `fc -e -`."),
    (
        "unfunction",
        "Remove a function definition. Equivalent to `unhash -f` / `unset -f name`.",
    ),
    // ── zftp companion functions (zsh/zftp module). Each `zf_X` mirrors
    // the unix command `X` against the connected FTP server.
    ("zf_chgrp", "zftp: change group of remote files. Mirrors `chgrp(1)`."),
    ("zf_chmod", "zftp: change mode of remote files. Mirrors `chmod(1)`."),
    ("zf_chown", "zftp: change owner of remote files. Mirrors `chown(1)`."),
    ("zf_ln",    "zftp: link / rename remote files. Mirrors `ln(1)`."),
    ("zf_mkdir", "zftp: create remote directories. Mirrors `mkdir(1)`."),
    ("zf_mv",    "zftp: move / rename remote files. Mirrors `mv(1)`."),
    ("zf_rm",    "zftp: remove remote files. Mirrors `rm(1)`."),
    ("zf_rmdir", "zftp: remove remote directories. Mirrors `rmdir(1)`."),
    ("zf_sync",  "zftp: flush pending writes on the FTP control channel."),
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
    ("$SHELL", "Pathname of the login shell. Honored by many tools as the default user shell."),
    ("$EDITOR", "Preferred editor for tools that invoke an editor (`fc`, `git`, `crontab`, …)."),
    ("$VISUAL", "Preferred full-screen editor. Takes precedence over `$EDITOR` when set."),
];

// ── Reflection dump for the IntelliJ tool window ────────────────────────

/// Produce the JSON consumed by `zshrs --dump-reflection`. Each top-level
/// key is a category; each entry is `name → tag` so the tool window can
/// group by tag in its tree.
///
/// Sources the canonical registries (`ported::builtin::BUILTINS`,
/// `ported::options::ZSH_OPTIONS_SET`) rather than the hand-curated
/// LSP subsets above. The hand subsets were a 49-option / 67-builtin /
/// 34-keyword / 41-special slice — fine for in-buffer keyword
/// classification but wrong as a tool-window inventory because the
/// IntelliJ panel is meant to mirror everything the runtime actually
/// implements. Sourcing from the canonical sets keeps the panel honest
/// as new ports land (e.g. adding a builtin to `ported::builtin::BUILTINS`
/// makes it show up in the panel without a parallel edit here).
pub fn dump_reflection_json() -> String {
    let mut builtins = serde_json::Map::new();
    for b in crate::ported::builtin::BUILTINS.iter() {
        builtins.insert(b.node.nam.clone(), Value::String("builtin".into()));
    }
    // Keywords stay on the hand list — zsh's "reserved words" set is
    // small, fixed, and grammatical (no canonical Rust registry mirrors
    // it because keywords aren't a runtime table the way builtins are).
    let mut keywords = serde_json::Map::new();
    for k in KEYWORDS {
        keywords.insert(k.to_string(), Value::String("keyword".into()));
    }
    let mut options = serde_json::Map::new();
    for o in crate::ported::options::ZSH_OPTIONS_SET.iter() {
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

/// Render the full LSP knowledge base as the four chapter `<section>`s
/// that `docs/reference.html` splices in between its `<!-- BEGIN/END
/// LSP-REFERENCE -->` markers. One `<article class="doc-entry">` per
/// canonical name across builtins / keywords / options / specials.
///
/// All inputs come from the baked Rust tables — no upstream zsh repo
/// access at runtime. The HTML uses the existing `.doc-entry` /
/// `.chapter-meta` styling already defined in reference.html so no
/// CSS changes are needed.
pub fn dump_reference_html() -> String {
    use std::fmt::Write;

    let mut out = String::new();

    // ── builtins (canonical from ported::builtin::BUILTINS) ──────────
    let mut builtins: Vec<String> = crate::ported::builtin::BUILTINS
        .iter()
        .map(|b| b.node.nam.clone())
        .collect();
    builtins.sort();
    builtins.dedup();
    write_chapter(
        &mut out,
        "ch-lsp-builtins",
        "Builtin Index",
        &format!(
            "{} entries · sourced from <code>ported::builtin::BUILTINS</code>. \
             Each body is the canonical man-zshall yodl text routed through \
             <code>lsp::lookup_doc</code>.",
            builtins.len()
        ),
        &builtins,
        "builtin",
    );

    // ── keywords (LSP hand registry — the grammatical reserved-word set) ──
    let keywords: Vec<String> = KEYWORDS.iter().map(|s| s.to_string()).collect();
    write_chapter(
        &mut out,
        "ch-lsp-keywords",
        "Keyword Index",
        &format!(
            "{} entries · zsh reserved words. Sub-keywords (<code>then</code>, \
             <code>else</code>, <code>do</code>, <code>esac</code>, …) point at \
             the parent compound statement.",
            keywords.len()
        ),
        &keywords,
        "keyword",
    );

    // ── options (canonical ZSH_OPTIONS_SET) ──────────────────────────
    let mut options: Vec<String> = crate::ported::options::ZSH_OPTIONS_SET
        .iter()
        .map(|s| s.to_string())
        .collect();
    options.sort();
    write_chapter(
        &mut out,
        "ch-lsp-options",
        "Option Index",
        &format!(
            "{} entries · the canonical zsh option registry. \
             Set / clear via <code>setopt NAME</code> / <code>unsetopt NAME</code>.",
            options.len()
        ),
        &options,
        "option",
    );

    // ── special vars (LSP hand registry, with `$` prefix kept) ───────
    let specials: Vec<String> = SPECIAL_VARS.iter().map(|s| s.to_string()).collect();
    write_chapter(
        &mut out,
        "ch-lsp-specials",
        "Special Variable Index",
        &format!(
            "{} entries · zsh-defined parameters and well-known env vars. \
             Includes both scalar (<code>$?</code>) and array (<code>$path</code>) forms.",
            specials.len()
        ),
        &specials,
        "special",
    );

    out
}

fn write_chapter(
    out: &mut String,
    id: &str,
    title: &str,
    meta_html: &str,
    names: &[String],
    kind: &str,
) {
    use std::fmt::Write;
    let _ = writeln!(
        out,
        "\n    <!-- ════════════════════════════════════════════════════════════════════ -->\n\
         \n    <section class=\"tutorial-section\" id=\"{id}\">\n\
         \n      <h2>{title}</h2>\n\
         \n      <p class=\"chapter-meta\">{meta_html}</p>",
    );
    for n in names {
        let body = lookup_doc(n);
        // lookup_doc returns `**HEADING** — _kind_\n\nBODY`. Split that
        // apart so the article shows the body without the heading
        // duplication (the article already prints the name in <h3>).
        let body_only = body.split_once("\n\n").map(|(_, b)| b).unwrap_or("");
        let anchor = anchor_for(kind, n);
        let _ = writeln!(
            out,
            "\n      <article class=\"doc-entry\" id=\"{anchor}\">\n\
             \n        <h3><code>{}</code> <a class=\"doc-anchor\" href=\"#{anchor}\">¶</a></h3>\n\
             {}      </article>",
            html_escape(n),
            md_to_html(body_only),
        );
    }
    out.push_str("\n    </section>\n");
}

fn anchor_for(kind: &str, name: &str) -> String {
    // Map every non-alphanumeric char to a stable mnemonic so single-char
    // punctuation builtins (`-`, `:`, `.`, `[`, `]`, `:`) each get a
    // unique anchor instead of all collapsing to `doc-lsp-builtin-`.
    // Preserve case to distinguish `$PATH` from `$path` (zsh ties them
    // via `typeset -T` but they're distinct hover targets).
    let mut slug = String::new();
    for c in name.chars() {
        match c {
            c if c.is_ascii_alphanumeric() => slug.push(c),
            '_' => slug.push('_'),
            '-' => slug.push_str("dash"),
            ':' => slug.push_str("colon"),
            '.' => slug.push_str("dot"),
            '[' => slug.push_str("lbracket"),
            ']' => slug.push_str("rbracket"),
            '(' => slug.push_str("lparen"),
            ')' => slug.push_str("rparen"),
            '{' => slug.push_str("lbrace"),
            '}' => slug.push_str("rbrace"),
            '?' => slug.push_str("qmark"),
            '!' => slug.push_str("bang"),
            '$' => slug.push_str("dollar"),
            '#' => slug.push_str("hash"),
            '*' => slug.push_str("star"),
            '@' => slug.push_str("at"),
            '/' => slug.push_str("slash"),
            '+' => slug.push_str("plus"),
            '=' => slug.push_str("eq"),
            _ => slug.push('-'),
        }
    }
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() {
        format!("doc-lsp-{}-unnamed", kind)
    } else {
        format!("doc-lsp-{}-{}", kind, slug)
    }
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// Convert the markdown subset `lookup_doc` produces into HTML.
///
/// Supported: `**bold**`, `_italic_`, backtick code, blank-line
/// paragraph breaks. Anything else passes through HTML-escaped. The
/// generator in `scripts/gen_option_docs.py` already strips yodl down
/// to this subset, so we don't need a full Markdown parser.
fn md_to_html(s: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for para in s.split("\n\n") {
        let para = para.trim_matches('\n');
        if para.is_empty() {
            continue;
        }
        // Collapse intra-paragraph newlines to spaces so wrapped yodl
        // text reflows cleanly.
        let joined: String = para
            .split('\n')
            .map(str::trim_end)
            .collect::<Vec<_>>()
            .join(" ");
        let _ = writeln!(out, "        <p>{}</p>", inline_md(&joined));
    }
    out
}

fn inline_md(s: &str) -> String {
    // Walk char-by-char tracking three states: code-span (between `…`),
    // bold (between **…**), italic (between _…_). Code wins over the
    // others; bold and italic stay greedy/non-overlapping.
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i] as char;
        // Code span — close at next backtick.
        if c == '`' {
            out.push_str("<code>");
            i += 1;
            while i < bytes.len() && bytes[i] as char != '`' {
                let cc = bytes[i] as char;
                match cc {
                    '&' => out.push_str("&amp;"),
                    '<' => out.push_str("&lt;"),
                    '>' => out.push_str("&gt;"),
                    _ => out.push(cc),
                }
                i += 1;
            }
            out.push_str("</code>");
            if i < bytes.len() {
                i += 1; // consume closing `
            }
            continue;
        }
        // **bold**
        if c == '*' && i + 1 < bytes.len() && bytes[i + 1] as char == '*' {
            if let Some(end) = find_close(bytes, i + 2, b"**") {
                out.push_str("<strong>");
                out.push_str(&inline_md(
                    std::str::from_utf8(&bytes[i + 2..end]).unwrap_or(""),
                ));
                out.push_str("</strong>");
                i = end + 2;
                continue;
            }
        }
        // _italic_ — only when bounded by non-alphanumeric on both sides
        // so `name_with_underscores` doesn't trigger.
        if c == '_'
            && (i == 0 || !(bytes[i - 1] as char).is_alphanumeric())
            && i + 1 < bytes.len()
            && !(bytes[i + 1] as char).is_whitespace()
        {
            if let Some(end) = find_close(bytes, i + 1, b"_") {
                let after_ok = end + 1 >= bytes.len() || !(bytes[end + 1] as char).is_alphanumeric();
                if after_ok {
                    out.push_str("<em>");
                    out.push_str(&inline_md(
                        std::str::from_utf8(&bytes[i + 1..end]).unwrap_or(""),
                    ));
                    out.push_str("</em>");
                    i = end + 1;
                    continue;
                }
            }
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

fn find_close(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    let mut i = start;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            return Some(i);
        }
        i += 1;
    }
    None
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

    // Regression pins (2026-05-23): zsh function/command names allow
    // `-` (e.g. `daemon-ping`, `daemon-job-submit`). Before the fix,
    // `word_at` stopped at `-` and rename/find-refs only matched the
    // segment after the last `-`. Now `-NAME` segments are included
    // for non-`$`-prefixed words, while `$var` / `${var-default}`
    // contexts stay at the strict-identifier boundary.

    #[test]
    fn word_at_extends_through_hyphen_for_function_name() {
        let _g = crate::test_util::global_state_lock();
        let src = "daemon-ping arg\n";
        // Cursor on `daemon` segment.
        assert_eq!(word_at(src, 0, 0), Some("daemon-ping".into()));
        assert_eq!(word_at(src, 0, 3), Some("daemon-ping".into()));
        // Cursor on `ping` segment.
        assert_eq!(word_at(src, 0, 7), Some("daemon-ping".into()));
        assert_eq!(word_at(src, 0, 10), Some("daemon-ping".into()));
    }

    #[test]
    fn word_at_extends_through_multiple_hyphens() {
        let _g = crate::test_util::global_state_lock();
        let src = "daemon-job-submit -- cmd\n";
        assert_eq!(word_at(src, 0, 8), Some("daemon-job-submit".into()));
        assert_eq!(word_at(src, 0, 13), Some("daemon-job-submit".into()));
    }

    #[test]
    fn word_at_dollar_var_does_not_extend_through_hyphen() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo $x-y suffix\n";
        // `$x-y` in shell expands `$x` then literal `-y`. Caret on
        // `x` must return `$x`, NOT `$x-y`.
        assert_eq!(word_at(src, 0, 6), Some("$x".into()));
    }

    #[test]
    fn word_at_braced_var_does_not_extend_through_hyphen() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo ${x-default}\n";
        // `${x-default}` is the `${VAR-WORD}` (default-if-unset)
        // operator. Caret on `x` must return `x`, NOT `x-default`.
        assert_eq!(word_at(src, 0, 7), Some("x".into()));
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
        // Upstream `Doc/Zsh/builtins.yo` `cd` description.
        assert!(
            doc.contains("Change the current directory"),
            "expected upstream cd prose; got: {}",
            doc
        );
    }

    #[test]
    fn lookup_doc_handles_keywords_and_special_vars() {
        let _g = crate::test_util::global_state_lock();
        // Upstream `Doc/Zsh/grammar.yo` `if` description.
        assert!(
            lookup_doc("if").contains("zero exit status"),
            "expected upstream if prose; got: {}",
            lookup_doc("if")
        );
        // Upstream `Doc/Zsh/params.yo` `?` description (stripped of $).
        assert!(
            lookup_doc("$?").contains("exit status"),
            "expected $? doc; got: {}",
            lookup_doc("$?")
        );
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

    // Pins for the four false-positive classes that flagged 197
    // bogus diagnostics on a 575-line daemon helper before fixes:
    //   1. `#` inside `$#` / `${#var}` aborting the line scan.
    //   2. `[[ ... ]]` parsed as two single brackets.
    //   3. `(( ... ))` arithmetic parsed as two single parens.
    //   4. `)` as case-arm pattern terminator flagged as unmatched.

    #[test]
    fn diagnose_does_not_flag_dollar_hash_as_comment() {
        let _g = crate::test_util::global_state_lock();
        // `$#` is the arg-count special variable, not a comment.
        // Pre-fix this terminated the line scan and left `[[`
        // unclosed → cascade of 100+ false positives downstream.
        let src = "[[ $# -gt 0 ]] && echo args\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "`$#` mis-handled as comment marker: {:?}",
            d
        );
    }

    #[test]
    fn diagnose_does_not_flag_param_length_as_comment() {
        let _g = crate::test_util::global_state_lock();
        // `${#var}` is parameter-length expansion.
        let src = "echo ${#args}\nif [[ ${#arr} -gt 0 ]]; then echo nonempty; fi\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "`${{#var}}` mis-handled as comment marker: {:?}",
            d
        );
    }

    #[test]
    fn diagnose_handles_double_bracket_as_pair() {
        let _g = crate::test_util::global_state_lock();
        // `[[ ... ]]` is a single zsh conditional expression — must
        // not be parsed as two `[`/`]` token pairs.
        let src = "[[ -n \"$x\" ]]\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "`[[ ]]` mis-handled as two `[`s: {:?}",
            d
        );
    }

    #[test]
    fn diagnose_handles_arithmetic_double_paren_as_pair() {
        let _g = crate::test_util::global_state_lock();
        // `(( ... ))` is a single arithmetic expression.
        let src = "(( i++ ))\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "`(( ))` mis-handled as two `(`s: {:?}",
            d
        );
    }

    #[test]
    fn diagnose_does_not_flag_case_arm_paren_as_unmatched() {
        let _g = crate::test_util::global_state_lock();
        // Bare `)` inside an open `case ... esac` block is a
        // pattern-arm terminator, not a paren mismatch.
        let src = "case \"$x\" in\n  -h|--help) echo usage ;;\n  *) echo other ;;\nesac\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "case-arm `)` flagged as unmatched: {:?}",
            d
        );
    }

    #[test]
    fn diagnose_still_flags_unmatched_paren_outside_case() {
        let _g = crate::test_util::global_state_lock();
        // Sanity: the case-arm exemption must NOT swallow a real
        // unmatched `)` outside of any case block.
        let src = "echo bare )\n";
        let d = diagnose(src);
        assert!(
            d.iter()
                .any(|v| v["message"]
                    .as_str()
                    .unwrap_or("")
                    .contains("unmatched `)`")),
            "real unmatched `)` was not flagged: {:?}",
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
        // Well-known names must be present. Option names follow the
        // canonical `ZSH_OPTIONS_SET` casing (lowercase, no underscore)
        // since dump_reflection_json now sources from there directly.
        assert!(v["builtins"]["cd"].is_string());
        assert!(v["keywords"]["if"].is_string());
        assert!(v["options"]["extendedglob"].is_string());
        assert!(v["special_vars"]["$?"].is_string());
        // Canonical sourcing produces the full inventory, not the
        // hand subset. The pre-rewire JSON had ~49 options; post-
        // rewire it should be the full ZSH_OPTIONS_SET (~200).
        assert!(
            v["options"].as_object().unwrap().len() > 150,
            "dump_reflection options count regressed to {}; expected canonical full set",
            v["options"].as_object().unwrap().len(),
        );
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

    // ── String-literal hover gate ───────────────────────────────────────

    /// Cursor on `cd` inside `"cd to dir"` — `position_inside_string_literal`
    /// must return true so the gate suppresses the doc card.
    #[test]
    fn position_inside_double_quoted_string_detected() {
        // 0         1
        // 01234567890123456
        // echo "cd to dir"
        let line = "echo \"cd to dir\"";
        let cd_start = line.find("cd").unwrap();
        let cd_end = cd_start + 2;
        assert!(position_inside_string_literal(line, cd_start, cd_end));
    }

    /// Same word but inside `'...'` — single quotes still suppress (no
    /// `${...}` expansion in zsh single-quoted strings, so the gate is
    /// even simpler).
    #[test]
    fn position_inside_single_quoted_string_detected() {
        let line = "echo 'cd to dir'";
        let cd_start = line.find("cd").unwrap();
        let cd_end = cd_start + 2;
        assert!(position_inside_string_literal(line, cd_start, cd_end));
    }

    /// Inside backticks (`` `cmd subst` ``) — also treated as a string
    /// boundary for hover purposes. The interior is technically code,
    /// but we keep the conservative behavior matching stryke until a
    /// real need surfaces.
    #[test]
    fn position_inside_backtick_string_detected() {
        let line = "echo `cd to dir`";
        let cd_start = line.find("cd").unwrap();
        let cd_end = cd_start + 2;
        assert!(position_inside_string_literal(line, cd_start, cd_end));
    }

    /// `"${HOME}"` — cursor on `HOME` is INSIDE the string syntactically
    /// but inside a `${...}` parameter expansion, which is code. The
    /// gate must allow hover.
    #[test]
    fn position_inside_parameter_expansion_is_code() {
        // echo "${HOME}/x"
        let line = "echo \"${HOME}/x\"";
        let home_start = line.find("HOME").unwrap();
        let home_end = home_start + 4;
        assert!(
            !position_inside_string_literal(line, home_start, home_end),
            "`${{HOME}}` inside double-quotes is code, not string text"
        );
    }

    /// Outside any string — bare code, no suppression.
    #[test]
    fn position_outside_string_is_code() {
        let line = "cd /tmp";
        assert!(!position_inside_string_literal(line, 0, 2));
    }

    /// Closing quote before cursor — outside the string again.
    #[test]
    fn position_after_closing_quote_is_code() {
        // echo "foo" cd
        let line = "echo \"foo\" cd";
        let cd_start = line.find(" cd").unwrap() + 1;
        let cd_end = cd_start + 2;
        assert!(!position_inside_string_literal(line, cd_start, cd_end));
    }

    /// Full `classify_hover_position` integration: comment beats string.
    #[test]
    fn classify_comment_outranks_string() {
        // `# echo "cd"` — `cd` is inside a quote, but the whole line
        // is comment-text. The Comment gate fires first.
        let line = "# echo \"cd\"";
        let cd_start = line.find("cd").unwrap();
        let cd_end = cd_start + 2;
        assert_eq!(
            classify_hover_position(line, cd_start, cd_end),
            HoverGate::Comment
        );
    }

    /// Plain string-literal classification.
    #[test]
    fn classify_string_literal() {
        let line = "echo \"cd to dir\"";
        let cd_start = line.find("cd").unwrap();
        let cd_end = cd_start + 2;
        assert_eq!(
            classify_hover_position(line, cd_start, cd_end),
            HoverGate::StringLiteral
        );
    }

    /// Plain code-position classification.
    #[test]
    fn classify_bare_code() {
        let line = "cd /tmp";
        assert_eq!(classify_hover_position(line, 0, 2), HoverGate::Code);
    }

    // ── Rename: `::` qualifier strip ────────────────────────────────────

    /// Regression: client prefilled `Demo::handle`; user edited suffix
    /// to `handle2`; dialog returned `"Demo::handle2"`. The rename
    /// handler must strip the qualifier and emit BARE `handle2` at
    /// every call site — never `Demo::Demo::handle2`.
    #[test]
    fn rename_strips_colon_colon_qualifier() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "function handle { echo hi }\nhandle\nhandle x\n".into(),
        );
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 9 }, // on `handle`
            "newName": "Demo::handle2",
        });
        let r = rename(&state, &params);
        let changes = r["changes"].as_object().expect("changes");
        let edits = changes["file:///t.zsh"].as_array().expect("edits");
        assert!(!edits.is_empty(), "expected at least 1 edit, got: {edits:?}");
        for e in edits {
            assert_eq!(
                e["newText"], json!("handle2"),
                "qualifier must be stripped; got: {e:?}"
            );
        }
    }

    /// Bare new_name without `::` — pass through unchanged (no-op for
    /// callers who already send the right form).
    #[test]
    fn rename_passes_through_bare_new_name() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "function handle { echo hi }\nhandle\n".into(),
        );
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 9 },
            "newName": "handle2",
        });
        let r = rename(&state, &params);
        let edits = r["changes"]["file:///t.zsh"].as_array().expect("edits");
        for e in edits {
            assert_eq!(e["newText"], json!("handle2"));
        }
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
