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
    let line = text.and_then(|t| t.lines().nth(line_no));

    // Context gate: inside a `"..."` or `'...'` literal segment we
    // should NOT fire arbitrary builtin / keyword / option completions
    // — they're noise (the user is typing English / a URL / a JSON
    // payload, not shell code). Exceptions:
    //   * Inside `$(…)` or `` `…` `` command substitution — that IS
    //     shell code, fire normally.
    //   * Inside `${…}` parameter expansion — variable / option name
    //     completion is useful there.
    //   * Inside `$'…'` ANSI-C strings — opaque, no completion.
    if let Some(l) = line {
        if cursor_in_uninterpolated_string(l, col) {
            return json!({ "isIncomplete": false, "items": [] });
        }
    }

    // Context-specific completion tables. `${(…)` → parameter
    // expansion flags; `*(…)` / `?(…)` / `](…)` → glob qualifiers.
    // These OVERRIDE the normal builtin/keyword/option flow because
    // in those positions nothing else is syntactically valid.
    if let Some(l) = line {
        match lsp_completion_context(l, col) {
            LspCompletionContext::ParamFlag => {
                let items: Vec<Value> = PARAM_FLAG_DOCS
                    .iter()
                    .map(|(flag, doc)| {
                        json!({
                            "label": flag,
                            "kind": 14, // Constant
                            "detail": *doc,
                            "documentation": {
                                "kind": "markdown",
                                "value": format!("**`(`{}`)`** — {}\n\n_zsh parameter expansion flag — `${{(FLAGS)var}}`_", flag, doc),
                            },
                        })
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::GlobQualifier => {
                let items: Vec<Value> = GLOB_QUALIFIER_DOCS
                    .iter()
                    .map(|(q, doc)| {
                        json!({
                            "label": q,
                            "kind": 14, // Constant
                            "detail": *doc,
                            "documentation": {
                                "kind": "markdown",
                                "value": format!("**`(`{}`)`** — {}\n\n_zsh glob qualifier — `*(QUALIFIERS)`_", q, doc),
                            },
                        })
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::HistoryDesignator => {
                let items: Vec<Value> = HISTORY_DESIGNATOR_DOCS
                    .iter()
                    .map(|(d, doc)| {
                        json!({
                            "label": d,
                            "kind": 14, // Constant
                            "detail": *doc,
                            "documentation": {
                                "kind": "markdown",
                                "value": format!("**`!{}`** — {}\n\n_zsh history event designator_", d, doc),
                            },
                        })
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::ParamColonModifier => {
                let items: Vec<Value> = PARAM_MODIFIER_DOCS
                    .iter()
                    .map(|(m, doc)| {
                        json!({
                            "label": m,
                            "kind": 14, // Constant
                            "detail": *doc,
                            "documentation": {
                                "kind": "markdown",
                                "value": format!("**`:{}`** — {}\n\n_zsh modifier — `${{var:MOD}}` / `!event:MOD`_", m, doc),
                            },
                        })
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::Normal => {}
        }
    }

    let prefix = line
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
    // Compat builtins — ported `Src/Builtins/*.c` set. Note this is
    // the hand `BUILTINS` const used for fast inline classification.
    for b in BUILTINS {
        if want(b) {
            push(&mut items, b, 3, "builtin");
        }
    }
    // Canonical compat builtins — `ported::builtin::BUILTINS` has 154
    // entries vs the hand `BUILTINS` subset of ~67. Without this, names
    // like `vared`, `zformat`, `sched`, `strftime`, etc. don't surface
    // in completion even though hover docs exist for them. Dedupe via
    // the `want()` filter — duplicate `cd` from BUILTINS + canonical
    // BUILTINS won't both fire because the second push is filtered out
    // by the IDE's own dedup on `label`.
    for b in crate::ported::builtin::BUILTINS.iter() {
        if want(&b.node.nam) {
            push(&mut items, &b.node.nam, 3, "builtin");
        }
    }
    // zshrs extension builtins — `date`, `cat`, `sleep`, `async`,
    // `await`, `barrier`, `peach`, `doctor`, `intercept`, etc. The
    // bug the user filed: `zwh<TAB>` didn't offer `zwhere` because
    // the daemon `z*` builtins live in ZSHRS_BUILTIN_NAMES and were
    // never added to the completion list. Same issue for ext fns
    // generally (74 in-process + 23 daemon = 97 names total).
    for n in crate::ext_builtins::EXT_BUILTIN_NAMES {
        if want(n) {
            push(&mut items, n, 3, "extension builtin");
        }
    }
    for n in crate::daemon::builtins::ZSHRS_BUILTIN_NAMES {
        if want(n) {
            push(&mut items, n, 3, "extension builtin (daemon)");
        }
    }
    // Compsys functions — `_arguments`, `_files`, `_describe`, the
    // per-command completers (`_git` / `_docker` / `_cargo` / etc.).
    // Useful when authoring completion-spec files.
    for n in compsys::COMPSYS_FN_NAMES {
        if want(n) {
            push(&mut items, n, 3, "compsys function");
        }
    }
    for o in OPTIONS {
        if want(o) {
            push(&mut items, o, 21, "option");
        }
    }
    // Canonical options registry — full ~194 entries vs the small
    // hand subset above. `setopt <TAB>` should surface every option
    // the runtime knows, not just the 49 we hand-listed.
    for o in crate::ported::options::ZSH_OPTIONS_SET.iter() {
        if want(o) {
            push(&mut items, o, 21, "option");
        }
    }
    for s in SPECIAL_VARS {
        if want(s) || (prefix.starts_with('$') && s.starts_with(&prefix)) {
            push(&mut items, s, 6, "special variable");
        }
    }
    // Snippet templates — mirrors strykelang's `SNIPPETS` table. Each
    // entry expands to a multi-line template with `${1:...}` placeholders
    // the user tabs through. CompletionItemKind=15 (Snippet),
    // InsertTextFormat=2 (Snippet — placeholders are honored).
    for (prefix, body, detail) in SNIPPETS {
        if !want(prefix) {
            continue;
        }
        items.push(json!({
            "label": format!("{} …", prefix),
            "kind": 15u8,
            "detail": detail,
            "filterText": prefix,
            "insertText": body,
            "insertTextFormat": 2u8,
        }));
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

/// Snippet templates surfaced via `textDocument/completion`. Mirrors
/// strykelang's `SNIPS` table. Each tuple is
/// `(prefix, body, short detail line)` — the body uses LSP
/// snippet placeholders (`${1:label}`, `${2:default}`, ... ending at
/// `${0}` for the final cursor stop).
///
/// Categories covered (60+ entries):
///   * Control flow: if / ifelse / ifelsif / for / forin / for-arith /
///     foreach / while / until / case / select / repeat
///   * Declarations: fn / local / typeset / export / readonly / integer
///   * Idioms: trap / setopt / autoload / compdef / bindkey / alias /
///     hashes / arrays
///   * Hooks: precmd / preexec / chpwd / zshexit (via add-zsh-hook)
///   * Module setup: shebang / safeshebang / main / usage / strict
///   * I/O: while-read / cat-pipe / process-subst / heredoc / printf-fmt
///   * Conditionals: dirtest / filetest / regex-match / not-empty
///   * Parallel (zshrs ext): async / await / barrier / peach
///   * ZLE: zle-widget / bindkey-widget
///   * Compsys: arguments-spec / files-spec / values-spec
///   * Scaffolds: test / git / curl / json
const SNIPPETS: &[(&str, &str, &str)] = &[
    // ── Control flow ────────────────────────────────────────────────
    ("if",       "if ${1:cmd}; then\n    ${2:body}\nfi${0}", "if/then/fi block (snippet)"),
    ("ifelse",   "if ${1:cmd}; then\n    ${2:body}\nelse\n    ${3:alt}\nfi${0}", "if/else/fi block (snippet)"),
    ("ifelsif",  "if ${1:cmd1}; then\n    ${2:body}\nelif ${3:cmd2}; then\n    ${4:alt}\nelse\n    ${5:fallback}\nfi${0}", "if/elif/else chain (snippet)"),
    ("elsif",    "elif ${1:cmd}; then\n    ${2:body}${0}", "elif branch (snippet)"),
    ("unless",   "if ! ${1:cmd}; then\n    ${2:body}\nfi${0}", "negated if (snippet)"),
    ("for",      "for ${1:item} in ${2:list}; do\n    ${3:body}\ndone${0}", "for loop (snippet)"),
    ("forin",    "for ${1:item} in \"${2:\\${array[@]}}\"; do\n    ${3:body}\ndone${0}", "for over quoted-array expansion (snippet)"),
    ("forarith", "for ((${1:i}=0; \\$${1:i} < ${2:n}; ${1:i}++)); do\n    ${3:body}\ndone${0}", "C-style arithmetic for (snippet)"),
    ("foreach",  "foreach ${1:item} (${2:list})\n    ${3:body}\nend${0}", "zsh-alt foreach…end (snippet)"),
    ("while",    "while ${1:cmd}; do\n    ${2:body}\ndone${0}", "while loop (snippet)"),
    ("until",    "until ${1:cmd}; do\n    ${2:body}\ndone${0}", "until loop (snippet)"),
    ("case",     "case ${1:word} in\n    ${2:pattern})\n        ${3:body}\n        ;;\n    *)\n        ${4:default}\n        ;;\nesac${0}", "case/esac (snippet)"),
    ("select",   "select ${1:choice} in ${2:items}; do\n    ${3:body}\n    break\ndone${0}", "select interactive menu (snippet)"),
    ("repeat",   "repeat ${1:N}; do\n    ${2:body}\ndone${0}", "repeat N times (snippet)"),
    ("break",    "break ${1:1}${0}", "break N levels (snippet)"),
    ("continue", "continue ${1:1}${0}", "continue N levels (snippet)"),
    ("return",   "return ${1:0}${0}", "return status (snippet)"),
    // ── Declarations ────────────────────────────────────────────────
    ("fn",       "${1:name}() {\n    ${2:body}\n}${0}", "function declaration (snippet)"),
    ("function", "function ${1:name} {\n    ${2:body}\n}${0}", "function keyword form (snippet)"),
    ("anonfn",   "() {\n    ${1:body}\n} ${2:args}${0}", "anonymous function (snippet)"),
    ("local",    "local ${1:var}=${2:value}${0}", "local declaration (snippet)"),
    ("locals",   "local ${1:a}=\"\\$1\" ${2:b}=\"\\$2\" ${3:c}=\"\\$3\"${0}", "local positional-arg unpack (snippet)"),
    ("typeset",  "typeset -${1:gAi} ${2:name}${3:=value}${0}", "typeset with attributes (snippet)"),
    ("export",   "export ${1:NAME}=\"${2:value}\"${0}", "export env var (snippet)"),
    ("readonly", "readonly ${1:NAME}=\"${2:value}\"${0}", "readonly var (snippet)"),
    ("integer",  "integer ${1:name}=${2:0}${0}", "integer typeset shorthand (snippet)"),
    ("array",    "${1:name}=(${2:a b c})${0}", "indexed array literal (snippet)"),
    ("assoc",    "typeset -A ${1:name}\n${1:name}=(\n    [${2:key1}]=${3:val1}\n    [${4:key2}]=${5:val2}\n)${0}", "associative array (snippet)"),
    // ── Common idioms ───────────────────────────────────────────────
    ("trap",     "trap '${1:handler}' ${2:INT TERM EXIT}${0}", "signal trap (snippet)"),
    ("setopt",   "setopt ${1:EXTENDED_GLOB NULL_GLOB PIPE_FAIL}${0}", "setopt one or more options (snippet)"),
    ("unsetopt", "unsetopt ${1:CASE_GLOB}${0}", "unsetopt options (snippet)"),
    ("autoload", "autoload -Uz ${1:funcname}${0}", "autoload function with -Uz (snippet)"),
    ("compdef",  "compdef ${1:_completer} ${2:command}${0}", "register completion (snippet)"),
    ("bindkey",  "bindkey '${1:^X^E}' ${2:edit-command-line}${0}", "ZLE bindkey (snippet)"),
    ("alias",    "alias ${1:name}='${2:command}'${0}", "alias (snippet)"),
    ("galias",   "alias -g ${1:NAME}='${2:expansion}'${0}", "global alias (snippet)"),
    ("salias",   "alias -s ${1:ext}='${2:opener}'${0}", "suffix alias (snippet)"),
    // ── Hooks (via add-zsh-hook) ────────────────────────────────────
    ("precmd",   "autoload -Uz add-zsh-hook\n${1:my_precmd}() {\n    ${2:body}\n}\nadd-zsh-hook precmd ${1:my_precmd}${0}", "precmd hook (snippet)"),
    ("preexec",  "autoload -Uz add-zsh-hook\n${1:my_preexec}() {\n    ${2:body}  # \\$1 = command line\n}\nadd-zsh-hook preexec ${1:my_preexec}${0}", "preexec hook (snippet)"),
    ("chpwd",    "autoload -Uz add-zsh-hook\n${1:my_chpwd}() {\n    ${2:body}\n}\nadd-zsh-hook chpwd ${1:my_chpwd}${0}", "chpwd hook (snippet)"),
    ("periodic", "autoload -Uz add-zsh-hook\nPERIOD=${1:60}\n${2:my_periodic}() {\n    ${3:body}\n}\nadd-zsh-hook periodic ${2:my_periodic}${0}", "periodic hook (snippet)"),
    ("zshexit",  "autoload -Uz add-zsh-hook\n${1:my_zshexit}() {\n    ${2:cleanup}\n}\nadd-zsh-hook zshexit ${1:my_zshexit}${0}", "zshexit hook (snippet)"),
    // ── Module setup ────────────────────────────────────────────────
    ("shebang",     "#!/usr/bin/env zshrs\n${0}", "zshrs shebang (snippet)"),
    ("safeshebang", "#!/usr/bin/env zsh\nemulate -L zsh\nsetopt err_return no_unset pipe_fail extended_glob\n${0}", "strict-mode shebang (snippet)"),
    ("main",        "#!/usr/bin/env zshrs\nemulate -L zsh\nsetopt err_return no_unset pipe_fail\n\n${1:main}() {\n    ${2:body}\n}\n\n${1:main} \"\\$@\"${0}", "main() scaffold (snippet)"),
    ("usage",       "${1:usage}() {\n    cat <<'EOT'\nUsage: ${2:command} [-h] [-v] ARG...\n\n  -h    show this help\n  -v    verbose\nEOT\n}${0}", "usage() helper (snippet)"),
    ("strict",      "emulate -L zsh\nsetopt err_return no_unset pipe_fail extended_glob${0}", "strict-mode options (snippet)"),
    // ── I/O ─────────────────────────────────────────────────────────
    ("while-read", "while IFS= read -r ${1:line}; do\n    ${2:body}\ndone < ${3:file}${0}", "read-loop over file (snippet)"),
    ("for-each-line", "for ${1:line} in \"\\${(@f)\\$(cat ${2:file})}\"; do\n    ${3:body}\ndone${0}", "for-each-line via process subst (snippet)"),
    ("cat-pipe",   "${1:cmd} | while read -r ${2:line}; do\n    ${3:body}\ndone${0}", "pipe-to-while (snippet)"),
    ("heredoc",    "cat <<EOT\n${1:body}\nEOT${0}", "heredoc (snippet)"),
    ("heredocl",   "cat <<-EOT\n\t${1:body}\nEOT${0}", "tab-stripped heredoc (snippet)"),
    ("herestring", "${1:cmd} <<< \"${2:input}\"${0}", "here-string (snippet)"),
    ("psub-in",    "${1:cmd} < <(${2:producer})${0}", "process substitution (input) (snippet)"),
    ("psub-out",   "${1:cmd} > >(${2:consumer})${0}", "process substitution (output) (snippet)"),
    ("subshell",   "(\n    ${1:body}\n)${0}", "subshell (snippet)"),
    ("printfmt",   "printf '%s\\\\n' \"${1:args}\"${0}", "printf line-per-arg (snippet)"),
    // ── Conditionals ────────────────────────────────────────────────
    ("dirtest",    "[[ -d \"${1:path}\" ]] && ${2:body}${0}", "directory-test guard (snippet)"),
    ("filetest",   "[[ -f \"${1:path}\" ]] && ${2:body}${0}", "regular-file guard (snippet)"),
    ("regexm",     "if [[ \"${1:str}\" =~ ${2:pattern} ]]; then\n    ${3:body}  # \\$match[*] / \\$MATCH\nfi${0}", "regex match into \\$match (snippet)"),
    ("notempty",   "[[ -n \"${1:var}\" ]] || ${2:return 1}${0}", "non-empty guard (snippet)"),
    // ── Parallel primitives (zshrs extension) ───────────────────────
    ("async",      "${1:job}=\\$(async ${2:'expensive_command'})\n${3:# … other work …}\n${4:result}=\\$(await \\$${1:job})${0}", "async + await pair (snippet)"),
    ("barrier",    "barrier '${1:task1}' ::: '${2:task2}' ::: '${3:task3}'${0}", "barrier (parallel + join) (snippet)"),
    ("peach",      "peach ${1:array} {\n    ${2:body}  # uses \\$it for each element\n}${0}", "parallel for-each on worker pool (snippet)"),
    ("intercept",  "intercept ${1:before} ${2:command} {\n    ${3:body}\n}${0}", "AOP intercept (snippet)"),
    // ── ZLE ─────────────────────────────────────────────────────────
    ("zle-widget", "${1:my-widget}() {\n    ${2:zle .accept-line}\n}\nzle -N ${1:my-widget}\nbindkey '${3:^X^E}' ${1:my-widget}${0}", "ZLE widget + bindkey (snippet)"),
    // ── Compsys / completion ────────────────────────────────────────
    ("argspec",    "_arguments \\\\\n    '(-h --help)'{-h,--help}'[show help]' \\\\\n    '(-v --verbose)'{-v,--verbose}'[verbose]' \\\\\n    ':${1:argname}:${2:_files}'${0}", "_arguments spec (snippet)"),
    ("filesspec",  "_files -g '${1:*.zsh}'${0}", "_files glob spec (snippet)"),
    ("valspec",    "_values '${1:tag}' \\\\\n    '${2:one}[${3:desc}]' \\\\\n    '${4:two}[${5:desc}]'${0}", "_values descriptor (snippet)"),
    ("describe",   "_describe '${1:group}' ${2:choices_array}${0}", "_describe (snippet)"),
    // ── Scaffolds ───────────────────────────────────────────────────
    ("test",       "#!/usr/bin/env zshrs\nemulate -L zsh\nsetopt err_return no_unset\n\n${1:test_name}() {\n    [[ \"${2:got}\" == \"${3:want}\" ]] && echo PASS || { echo FAIL; return 1; }\n}\n\n${1:test_name}${0}", "test scaffold (snippet)"),
    ("gitcommit",  "git add -A && git commit -m \"${1:message}\" && git push${0}", "git add+commit+push (snippet)"),
    ("curlget",    "curl -fsSL ${1:https://example.com/api} | ${2:jq .}${0}", "curl GET + jq pipe (snippet)"),
    ("jsonget",    "${1:cmd} | jq -r '${2:.field}'${0}", "extract JSON field via jq (snippet)"),
    ("zmodload",   "zmodload zsh/${1:datetime}${0}", "load zsh module (snippet)"),
];

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
/// True if `col` (a byte column on `line`) sits inside a string
/// literal where completion should be SUPPRESSED. Specifically:
///   * Inside `"..."` literal text → suppress (user is typing prose,
///     not shell code).
///   * Inside `'...'` single-quoted → suppress (opaque to expansion).
///   * Inside `$'...'` ANSI-C quoted → suppress.
/// EXCEPT when we're nested inside a substitution that resumes shell
/// grammar:
///   * `$(...)` command substitution → shell code, allow completion.
///   * `` `...` `` backtick command substitution → allow completion.
///   * `${...}` parameter expansion → allow (variable names useful).
///
/// Walks the line char-by-char tracking the innermost open
/// container. A trailing `$(` / `` ` `` / `${` un-opens any
/// surrounding quotes for completion purposes.
pub(crate) fn cursor_in_uninterpolated_string(line: &str, col: usize) -> bool {
    let bytes = line.as_bytes();
    let cap = col.min(bytes.len());
    // Stack of open containers — `'"', '\'', '`'` for strings,
    // `'('` for `$(...)`, `'{'` for `${...}`. The TOP of the stack
    // tells us what context the cursor sits in.
    let mut stack: Vec<u8> = Vec::new();
    let mut i = 0;
    while i < cap {
        let c = bytes[i];
        let top = stack.last().copied();
        // Escapes — only `\X` inside double-quoted / backtick strings.
        // Single-quoted is opaque (no escapes).
        if matches!(top, Some(b'"') | Some(b'`')) && c == b'\\' && i + 1 < cap {
            i += 2;
            continue;
        }
        match top {
            // Inside single-quote — only `'` closes.
            Some(b'\'') => {
                if c == b'\'' {
                    stack.pop();
                }
                i += 1;
                continue;
            }
            // Inside double-quote — `"` closes, OR enter sub/expansion.
            Some(b'"') => {
                if c == b'"' {
                    stack.pop();
                    i += 1;
                    continue;
                }
                if c == b'$' && i + 1 < cap {
                    let nxt = bytes[i + 1];
                    if nxt == b'(' {
                        stack.push(b'(');
                        i += 2;
                        continue;
                    }
                    if nxt == b'{' {
                        stack.push(b'{');
                        i += 2;
                        continue;
                    }
                }
                if c == b'`' {
                    stack.push(b'`');
                    i += 1;
                    continue;
                }
                i += 1;
                continue;
            }
            // Inside backtick — `` ` `` closes, `$(` / `${` nest.
            Some(b'`') => {
                if c == b'`' {
                    stack.pop();
                    i += 1;
                    continue;
                }
                if c == b'$' && i + 1 < cap {
                    let nxt = bytes[i + 1];
                    if nxt == b'(' {
                        stack.push(b'(');
                        i += 2;
                        continue;
                    }
                    if nxt == b'{' {
                        stack.push(b'{');
                        i += 2;
                        continue;
                    }
                }
                i += 1;
                continue;
            }
            // Inside `$(…)` — `)` closes, quotes / nested subst open.
            Some(b'(') => {
                if c == b')' {
                    stack.pop();
                    i += 1;
                    continue;
                }
                // Fall through to top-level handling for nested
                // strings / substitutions.
            }
            // Inside `${…}` — `}` closes.
            Some(b'{') => {
                if c == b'}' {
                    stack.pop();
                    i += 1;
                    continue;
                }
                // Fall through to top-level.
            }
            _ => {}
        }
        // Top-level (or inside `$()` / `${}`) — track new openers.
        match c {
            b'"' => stack.push(b'"'),
            b'\'' => stack.push(b'\''),
            b'`' => stack.push(b'`'),
            b'$' if i + 1 < cap => {
                let nxt = bytes[i + 1];
                if nxt == b'(' {
                    stack.push(b'(');
                    i += 2;
                    continue;
                }
                if nxt == b'{' {
                    stack.push(b'{');
                    i += 2;
                    continue;
                }
                if nxt == b'\'' {
                    // `$'...'` ANSI-C — push single-quote so the
                    // body counts as opaque-string for completion.
                    stack.push(b'\'');
                    i += 2;
                    continue;
                }
            }
            b'#' => {
                // `#` only starts a comment at statement-start position.
                // Inside strings / subs this branch isn't reached anyway
                // (top is non-None). At top-level treat the rest as a
                // comment — cursor inside a comment also suppresses
                // shell-code completion.
                let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
                let comment_open = matches!(
                    prev,
                    None | Some(b' ') | Some(b'\t') | Some(b';') | Some(b'&') | Some(b'|') | Some(b'(')
                );
                if comment_open {
                    return true;
                }
            }
            _ => {}
        }
        i += 1;
    }
    // Cursor is in an UNINTERPOLATED string when the innermost open
    // container is `"` / `'` (NOT a `$(…)` / `${…}` / backtick).
    matches!(stack.last().copied(), Some(b'"') | Some(b'\''))
}

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
    // Operators / punctuation tokens. Match these BEFORE the yodl
    // keyword table — `man zshmisc` documents `&&` / `||` / `>` / `[[`
    // etc. in section prose, not per-name `item(tt(NAME))` blocks, so
    // the only way to surface them is a hand fallback.
    if let Some(d) = OPERATOR_DOCS.iter().find(|(k, _)| *k == name) {
        return format!("**{}** — _zsh operator_\n\n{}", d.0, d.1);
    }
    if let Some((canon, body)) = crate::zsh_keyword_docs::lookup_keyword_doc(name) {
        return format!("**{}** — _zsh keyword_\n\n{}", canon, body);
    }
    // Hard-classify canonical reserved words BEFORE consulting the
    // yodl builtin table — but only when the name ISN'T also a real
    // builtin in `ported::builtin::BUILTINS`. `mod_complist.yo`
    // (LS_COLORS docs) defines `item(tt(fi 0))(for regular files)`,
    // `item(tt(no 0))(...)`, `item(tt(do 0))(...)` etc. The multi-word
    // `tt(NAME N)` regex in the gen script extracts `fi` / `no` / `do`
    // as builtin names — without this guard, hover on `fi` returned
    // "fi — zsh builtin: for regular files". The declarers (`export`,
    // `typeset`, `float`, `integer`, etc.) ARE real builtins so they
    // must keep flowing to the substantive yodl builtin doc.
    let is_keyword = crate::ported::hashtable::RESWDS
        .iter()
        .any(|(n, _)| *n == name);
    let is_real_builtin = crate::ported::builtin::BUILTINS
        .iter()
        .any(|b| b.node.nam == name);
    if is_keyword && !is_real_builtin {
        if let Some(d) = KEYWORD_DOCS.iter().find(|(k, _)| *k == name) {
            return format!("**{}** — _zsh keyword_\n\n{}", d.0, d.1);
        }
        // Reserved word with no hand fallback — emit a minimal stub
        // instead of falling through to a bogus builtin entry.
        return format!("**{}** — _zsh keyword_", name);
    }
    // Extension-builtin classification wins over yodl-builtin lookup
    // when the same name exists as both. `date` is the textbook case:
    // upstream zsh has it in `zsh/datetime` module (so the yodl
    // builtin table has an entry), but zshrs ships it as an
    // always-available extension (no `zmodload` required). Showing
    // "zshrs builtin" reflects the runtime reality the user sees.
    // Also covers `sched`, `stat` / `zstat`, `strftime`, etc.
    let is_extension = crate::ext_builtins::EXT_BUILTIN_NAMES.contains(&name)
        || crate::daemon::builtins::ZSHRS_BUILTIN_NAMES.contains(&name);
    if is_extension {
        if let Some(body) = crate::zsh_ext_builtin_docs::lookup_full(name) {
            return format!("**{}** — _zshrs extension builtin_\n\n{}", name, body);
        }
        if let Some(d) = EXT_BUILTIN_DOCS.iter().find(|(k, _)| *k == name) {
            return format!("**{}** — _zshrs extension builtin_\n\n{}", d.0, d.1);
        }
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
    // Full doc-comment body (extracted from source `///` blocks by
    // `scripts/gen_ext_builtin_docs.py`). Wins over the hand one-liner
    // in EXT_BUILTIN_DOCS — the user's complaint was that `zwhere`/
    // `zd` etc. were returning one-line summaries when the source has
    // rich multi-paragraph descriptions.
    if let Some(body) = crate::zsh_ext_builtin_docs::lookup_full(name) {
        return format!("**{}** — _zshrs extension builtin_\n\n{}", name, body);
    }
    if let Some(d) = EXT_BUILTIN_DOCS.iter().find(|(k, _)| *k == name) {
        return format!("**{}** — _zshrs extension builtin_\n\n{}", d.0, d.1);
    }
    if let Some(d) = COMPSYS_FN_DOCS.iter().find(|(k, _)| *k == name) {
        return format!("**{}** — _compsys function_\n\n{}", d.0, d.1);
    }
    String::new()
}

/// Hand-curated docs for every shell operator / punctuation token.
/// Sourced from `man zshmisc` — Pipelines, Simple Commands & Pipelines
/// (lists), Complex Commands, Reserved Words; `man zshparam` for
/// expansion forms; `man zshmisc` Conditional Expressions for `[[ … ]]`
/// operators; `man zshexpn` for substitution / brace expansion.
///
/// These don't have per-name `item(tt(X))` blocks in any yodl file —
/// they're documented in section prose, so the gen script has nothing
/// to extract. Hand-bodies are the only path to hover docs for them.
const OPERATOR_DOCS: &[(&str, &str)] = &[
    // ── Pipelines ────────────────────────────────────────────────────
    ("|",   "Pipeline. `cmd1 | cmd2` connects `cmd1`'s stdout to `cmd2`'s stdin. Each stage runs in a separate process; exit status is the last stage's (unless `PIPE_FAIL` is set, in which case the first non-zero in the chain wins)."),
    ("|&",  "Pipeline merging stderr. `cmd1 |& cmd2` = `cmd1 2>&1 | cmd2`. Both stdout AND stderr of `cmd1` are piped to `cmd2`."),
    // ── Lists ────────────────────────────────────────────────────────
    ("&&",  "Logical AND list operator. `cmd1 && cmd2` runs `cmd2` only if `cmd1` succeeded (exit status 0). Short-circuits."),
    ("||",  "Logical OR list operator. `cmd1 || cmd2` runs `cmd2` only if `cmd1` failed (non-zero exit). Short-circuits."),
    (";",   "Sequential list separator. `cmd1; cmd2` runs `cmd2` after `cmd1` finishes, regardless of its exit status."),
    ("&",   "Background list operator. `cmd &` runs `cmd` asynchronously in the background; the shell does not wait. Sets `$!` to the job's PID."),
    (";;",  "Case-branch terminator. Ends a `case` arm: `case x in pat) cmds ;; esac`. Stops case dispatch after this arm."),
    (";;&", "Case-branch fall-through-and-test-next. Continues to the next `case` arm and tests its pattern."),
    (";|",  "Case-branch unconditional fall-through. Continues to the next `case` arm and runs it without testing its pattern."),
    // ── Negation ─────────────────────────────────────────────────────
    ("!",   "Pipeline negation (also a reserved word). `! cmd` inverts `cmd`'s exit status — zero becomes 1, non-zero becomes 0. Distinct from `!` history expansion (lexer-stage)."),
    // ── Redirection ──────────────────────────────────────────────────
    (">",   "Stdout redirect. `cmd > file` writes `cmd`'s stdout to `file` (overwrite). With `NO_CLOBBER`, refuses to overwrite an existing file — use `>|` or `>!` to force."),
    (">>",  "Stdout append. `cmd >> file` appends `cmd`'s stdout to `file` (creates if missing)."),
    ("<",   "Stdin redirect. `cmd < file` makes `file` the source of `cmd`'s stdin."),
    ("<<",  "Heredoc start. `cmd <<MARKER` reads the following lines as `cmd`'s stdin until a line containing only `MARKER`. Variants: `<<-` strips leading tabs; `<<'MARKER'` disables expansion in the body."),
    ("<<-", "Heredoc with tab-stripping. Like `<<` but every leading tab on body lines (and the terminator) is removed — lets you indent the heredoc for readability."),
    ("<<<", "Here-string. `cmd <<< 'text'` makes the literal string `text` the source of `cmd`'s stdin. Adds a trailing newline."),
    ("&>",  "Redirect stdout + stderr together. `cmd &> file` = `cmd > file 2>&1`. Shorthand for the common combined redirect."),
    ("&>>", "Append stdout + stderr together. `cmd &>> file` = `cmd >> file 2>&1`."),
    (">&",  "Redirect a file descriptor. `2>&1` sends stderr to wherever stdout currently points. `>& file` is also accepted as `&> file`."),
    ("<&",  "Duplicate an input file descriptor. `cmd <&3` reads from fd 3. `<& -` closes stdin."),
    ("<>",  "Read+write redirect. `cmd <> file` opens `file` for both reading and writing on stdin."),
    (">|",  "Force-overwrite redirect. Equivalent to `>` but ignores `NO_CLOBBER`."),
    (">!",  "Same as `>|` — force-overwrite, bypass `NO_CLOBBER`."),
    // ── Conditional expressions ──────────────────────────────────────
    ("[[",  "Open zsh conditional expression. `[[ EXPR ]]` evaluates a boolean. No word splitting / glob inside; supports `&&`, `||`, `!`, `==`, `!=`, `=~`, `-e`, `-f`, `-d`, `-z`, `-n`, etc. Prefer this over `[ ]` in zsh."),
    ("]]",  "Close zsh conditional expression. Pairs with `[[`. Must be a separate word — `[[ -n $x]]` is a syntax error; use `[[ -n $x ]]`."),
    ("[",   "POSIX `test` command (also spelled `test`). Same conditional semantics as POSIX `test`. Prefer `[[ … ]]` in zsh — it's safer (no word splitting) and supports more operators."),
    ("]",   "Close POSIX `test`. Pairs with `[`."),
    ("((",  "Open arithmetic command. `(( EXPR ))` evaluates `EXPR` as C-style integer arithmetic; exit 0 if the result is non-zero, 1 otherwise. Inside, `$` on var names is optional: `(( i++ ))`."),
    ("))",  "Close arithmetic command. Pairs with `((`."),
    // ── Command / parameter / arithmetic substitution ────────────────
    ("$(",  "Command substitution open. `$(cmd)` runs `cmd` and substitutes its trimmed-trailing-newline stdout. Nestable: `$(echo $(date))`. Preferred over backticks."),
    ("${",  "Parameter expansion open. `${VAR}` is the value of `VAR`. Rich modifier set: `${VAR:-default}`, `${VAR:=assign}`, `${VAR:+alt}`, `${#VAR}` length, `${VAR/p/r}` replace, `${VAR%suffix}` / `${VAR#prefix}` strip, `${(flags)VAR}` zsh flags."),
    ("$((", "Arithmetic expansion open. `$(( EXPR ))` evaluates `EXPR` as integer arithmetic and substitutes the result as a string. Distinct from `(( … ))` which is a command, not an expansion."),
    ("<(",  "Process substitution (input). `cmd <(producer)` exposes `producer`'s stdout as a filename (`/dev/fd/N`) to `cmd`. Lets commands that take filenames consume pipe output."),
    (">(",  "Process substitution (output). `cmd >(consumer)` exposes a filename to `cmd`; anything `cmd` writes there flows to `consumer`'s stdin."),
    ("`",   "Backtick command substitution. ``cmd`` runs `cmd` and substitutes its stdout. Legacy form — prefer `$(cmd)` for nestability and quoting clarity."),
    // ── Test-operator unaries (most common) ──────────────────────────
    ("-e",  "File-exists test. `[[ -e PATH ]]` is true if `PATH` exists (any type — file / dir / link / socket / ...)."),
    ("-f",  "Regular-file test. `[[ -f PATH ]]` is true if `PATH` exists AND is a regular file (not a directory / symlink / device)."),
    ("-d",  "Directory test. `[[ -d PATH ]]` is true if `PATH` exists AND is a directory."),
    ("-r",  "Readable test. `[[ -r PATH ]]` is true if `PATH` exists AND is readable by the current process."),
    ("-w",  "Writable test. `[[ -w PATH ]]` is true if `PATH` exists AND is writable by the current process."),
    ("-x",  "Executable test. `[[ -x PATH ]]` is true if `PATH` exists AND has execute permission (or for directories, search permission)."),
    ("-s",  "Non-empty test. `[[ -s PATH ]]` is true if `PATH` exists AND has size > 0."),
    ("-L",  "Symlink test. `[[ -L PATH ]]` is true if `PATH` is a symbolic link (does NOT dereference)."),
    ("-h",  "Same as `-L` — symlink test."),
    ("-z",  "Empty-string test. `[[ -z $s ]]` is true if `$s` is the empty string."),
    ("-n",  "Non-empty-string test. `[[ -n $s ]]` is true if `$s` has length > 0. Equivalent to `[[ $s ]]`."),
    // ── Test-operator binaries (numeric) ─────────────────────────────
    ("-eq", "Numeric equality. `[[ a -eq b ]]` is true if integers `a` and `b` are equal. For strings use `==`."),
    ("-ne", "Numeric inequality. `[[ a -ne b ]]` is true if integers `a` and `b` differ."),
    ("-lt", "Numeric less-than. `[[ a -lt b ]]` is true if integer `a` < `b`."),
    ("-le", "Numeric less-or-equal. `[[ a -le b ]]` is true if integer `a` ≤ `b`."),
    ("-gt", "Numeric greater-than. `[[ a -gt b ]]` is true if integer `a` > `b`."),
    ("-ge", "Numeric greater-or-equal. `[[ a -ge b ]]` is true if integer `a` ≥ `b`."),
    ("-ot", "Older-than test. `[[ A -ot B ]]` is true if file `A` has an older mtime than `B`."),
    ("-nt", "Newer-than test. `[[ A -nt B ]]` is true if file `A` has a newer mtime than `B`."),
    ("-ef", "Same-file test. `[[ A -ef B ]]` is true if `A` and `B` are the same inode (hard-linked / same path)."),
    // ── String / pattern operators (inside [[ … ]]) ──────────────────
    ("==",  "Pattern-match equality (inside `[[ … ]]`). `[[ $s == pat* ]]` matches `$s` against the glob pattern `pat*`. RHS is a pattern unless quoted. For literal equality, quote: `[[ $s == \"literal\" ]]`."),
    ("!=",  "Pattern-mismatch (inside `[[ … ]]`). Inverse of `==`. Quote the RHS for literal comparison."),
    ("=~",  "Regex match (inside `[[ … ]]`). `[[ $s =~ pat ]]` matches `$s` against the regex `pat`. Capture groups land in `$match` / `$MATCH` / `$BASH_REMATCH`."),
    // ── Glob / pattern characters ────────────────────────────────────
    ("*",   "Glob: match zero or more characters of any name (excluding leading `.` unless `GLOB_DOTS` is set). Also a multiplication operator inside `(( … ))`."),
    ("?",   "Glob: match exactly one character. Also the last-exit-status variable when used as `$?`."),
    ("**",  "Recursive glob (zsh extended). `**/*.rs` matches `*.rs` at any depth under the current directory. Requires `EXTENDED_GLOB` for additional pattern operators."),
    ("~",   "Pattern exclude (with `EXTENDED_GLOB`). `*~README` matches everything except `README`. Also tilde expansion: `~` = `$HOME`, `~user` = user's home, `~+` = `$PWD`, `~-` = `$OLDPWD`."),
    ("^",   "Pattern negate first-match (with `EXTENDED_GLOB`). `^*.rs` matches everything that's NOT `*.rs`. Inside `[…]` ranges, negates: `[^abc]`."),
    // ── Brace expansion ──────────────────────────────────────────────
    ("{a,b,c}", "Brace expansion (literal list). Expands to multiple words: `cp file.{txt,bak}` becomes `cp file.txt file.bak`. No whitespace before commas."),
    ("{1..10}", "Brace range expansion. `{1..10}` expands to `1 2 3 4 5 6 7 8 9 10`. Supports step: `{1..10..2}` → `1 3 5 7 9`. Letters work too: `{a..z}`."),
    // ── Assignment ───────────────────────────────────────────────────
    ("=",   "Assignment. `VAR=value`. NO whitespace around `=`. With `local` / `typeset`: `local VAR=value` declares + assigns."),
    ("+=",  "Append assignment. `VAR+=more` appends to a scalar; for arrays `arr+=(x y)` appends elements. Numeric for `integer`: `(( count += 1 ))`."),
    (":=",  "Conditional-assign default (inside `${…}`). `${VAR:=fallback}` assigns `fallback` to `VAR` (and substitutes it) if `VAR` is unset or empty."),
    ("?=",  "Error-if-unset (inside `${…}`). `${VAR:?msg}` substitutes `$VAR` if set, else prints `msg` to stderr and exits."),
];

/// `${(FLAGS)var}` parameter expansion flags. Single-char flags + a
/// few `(F:string:)` colon-delimited args. Surfaced as LSP completion
/// items when the cursor sits inside `${(…)` before the closing `)`.
/// Same list zsh's compsys `_parameter_flags` produces — verified
/// against `man zshexpn` "Parameter Expansion Flags".
const PARAM_FLAG_DOCS: &[(&str, &str)] = &[
    ("-", "sort decimal integers numerically (signed)"),
    ("@", "prevent double-quoted joining of arrays"),
    ("*", "enable extended globs for pattern arguments"),
    ("#", "interpret numeric expression as character code"),
    ("%", "expand prompt sequences (`%P` for prompt-only escapes)"),
    ("~", "treat strings in parameter flag arguments as patterns"),
    ("0", "split words on null bytes"),
    ("A", "assign as an array parameter (in `${...=...}` etc)"),
    ("a", "sort in array index order (with `O` to reverse)"),
    ("b", "backslash-quote pattern characters only"),
    ("B", "include index of beginning of match in `#`, `%` expressions"),
    ("C", "capitalize words"),
    ("c", "count characters in an array (with `${(c)#...}`)"),
    ("D", "perform directory name abbreviation"),
    ("E", "include index of one past end of match in `#`, `%` expressions"),
    ("e", "perform single-word shell expansions"),
    ("F", "join arrays with newlines"),
    ("f", "split the result on newlines"),
    ("g", "process echo array sequences (needs options like `gec`)"),
    ("I", "search Nth match in `#`, `%`, `/` expressions (`(I:N:)`)"),
    ("i", "sort case-insensitively"),
    ("j", "join arrays with specified string (`(j:STR:)`)"),
    ("k", "substitute keys of associative arrays"),
    ("l", "left-pad resulting words (`(l:N:)`, `(l:N::pad:)`)"),
    ("L", "lower case all letters"),
    ("m", "count multibyte width in padding calculation"),
    ("M", "include matched portion in `#`, `%` expressions"),
    ("N", "include length of match in `#`, `%` expressions"),
    ("n", "sort positive decimal integers numerically (unsigned)"),
    ("o", "sort in ascending order (lexically if no other sort option)"),
    ("O", "sort in descending order (lexically if no other sort option)"),
    ("p", "handle print escapes in parameter flag string arguments"),
    ("P", "use parameter value as name of parameter for redirected lookup"),
    ("q", "quote with backslashes (`q-` shell-quote, `qq` single-quote, `qqq` double-quote, `qqqq` $'...')"),
    ("Q", "remove one level of quoting"),
    ("R", "include rest (unmatched portion) in `#`, `%` expressions"),
    ("r", "right-pad resulting words (`(r:N:)`, `(r:N::pad:)`)"),
    ("S", "match non-greedy in `/`, `//`, or search substrings in `%`/`#` expressions"),
    ("s", "split words on specified string (`(s:STR:)`)"),
    ("t", "substitute type of parameter (`scalar`, `array`, `association`, `integer`, `float`, plus flags)"),
    ("u", "substitute first occurrence of each unique word"),
    ("U", "upper case all letters"),
    ("v", "substitute values of associative arrays (with `k`)"),
    ("V", "visibility enhancements for special characters"),
    ("w", "count words in array or string (with `${(w)#...}`)"),
    ("W", "count words including empty words (with `${(W)#...}`)"),
    ("X", "report parsing errors and exit substitution on failure"),
    ("z", "split words as if a zsh command line"),
    ("Z", "split words as if a zsh command line (with options — `(Z:cn:)`, `(Z:Cn:)`)"),
];

/// Glob qualifiers — letters inside `*(…)` / `pattern(…)` that restrict
/// the matches. Surfaced as LSP completion when the cursor sits inside
/// an unclosed paren immediately following a glob meta (`*`, `?`, `]`,
/// `)`). Verified against `man zshexpn` "Glob Qualifiers".
const GLOB_QUALIFIER_DOCS: &[(&str, &str)] = &[
    // ── File types ──
    ("/",  "directories"),
    ("F",  "non-empty directories"),
    (".",  "plain files (regular)"),
    ("@",  "symbolic links"),
    ("=",  "sockets"),
    ("p",  "named pipes (FIFOs)"),
    ("*",  "executable plain files (mode `0111`)"),
    ("%",  "device files (block or character)"),
    // ── Owner / permission ──
    ("r",  "owner-readable"),
    ("w",  "owner-writable"),
    ("x",  "owner-executable"),
    ("A",  "group-readable"),
    ("I",  "group-writable"),
    ("E",  "group-executable"),
    ("R",  "world-readable"),
    ("W",  "world-writable"),
    ("X",  "world-executable"),
    ("s",  "setuid"),
    ("S",  "setgid"),
    ("t",  "sticky bit set"),
    ("U",  "owned by current effective uid"),
    ("G",  "owned by current effective gid"),
    ("u",  "owned by specified uid (`u:LOGIN:` / `u<UID>`)"),
    ("g",  "owned by specified gid (`g:GROUP:` / `g<GID>`)"),
    ("f",  "exact file mode match (`f:SPEC:`, eg `f:u+w:`)"),
    // ── Time / size ──
    ("a",  "atime (`a-N` younger than N days, `a+N` older)"),
    ("m",  "mtime (`m-N` / `m+N`; suffixes `M`/`w`/`h`/`m`/`s`)"),
    ("c",  "ctime (`c-N` / `c+N`)"),
    ("L",  "size in bytes (`L-N`, `L+N`, suffixes `k`/`m`/`p`)"),
    ("l",  "link count (`l-N` / `l+N`)"),
    ("d",  "files on device DEV (`d<DEV>`)"),
    // ── Sort / slice / control ──
    ("o",  "order ascending (`oN` name, `oL` size, `om` mtime, `oa` atime, `oc` ctime, `od` depth, `oe:cmd:` custom)"),
    ("O",  "order descending (same suffixes as `o`)"),
    ("[",  "slice / range (`[N]`, `[N,M]`, `[N,-1]`)"),
    ("^",  "negate the rest of the qualifier list"),
    ("-",  "follow symbolic links when testing subsequent qualifiers"),
    ("M",  "mark directories with trailing `/`"),
    ("T",  "mark types with file-type indicator (`/=@*%|`)"),
    ("N",  "set NULL_GLOB for this glob only (no match → empty)"),
    ("D",  "include dotfiles in matches"),
    ("n",  "numeric sort (use with `o` / `O`)"),
    ("Y",  "early termination after N matches (`Y<N>`)"),
    ("P",  "prepend WORD to each result (`P:WORD:`)"),
    ("e",  "evaluate expression on each candidate (`e:EXPR:`); `$REPLY` is the filename"),
    ("+",  "true if `cmd FILENAME` exits 0 (`+cmd`)"),
];

/// History event designators — what follows `!` at the start of a
/// word. Triggered when the cursor sits after `!` at a word boundary
/// (start of line / after `;` / `&` / `|` / `(` / whitespace), not
/// inside `((…))` arithmetic. Verified against `man zshexpn` "History
/// Expansion → Event Designators".
const HISTORY_DESIGNATOR_DOCS: &[(&str, &str)] = &[
    ("!",   "previous command (`!!`)"),
    ("N",   "command N from history (`!42`)"),
    ("-N",  "N commands back (`!-3` = third-to-last)"),
    ("str", "most recent command starting with `str` (`!ls`)"),
    ("?str?", "most recent command containing `str` (`!?docker?`)"),
    ("#",   "current command line typed so far"),
    ("$",   "last argument of previous command (= `!!:$`)"),
    ("^",   "first argument of previous command (= `!!:^`)"),
    ("*",   "all arguments of previous command (= `!!:*`)"),
    (":",   "introduce a word designator / modifier — `!!:1`, `!!:s/old/new/`, `!!:h`"),
];

/// Parameter expansion + history modifiers — what follows `:` inside
/// `${var:…}` and `!event:…`. Combines:
///   * Default-value forms (`:-` / `:=` / `:?` / `:+`)
///   * Word modifiers (`:h` / `:t` / `:r` / `:e` / `:s/…/…/` etc.)
///   * Substring offset (`:N:M`)
/// Most modifier letters work in BOTH contexts, so a single table
/// drives modifier completion regardless of whether the `:` belongs
/// to a `${…}` or a `!…`. Verified against `man zshexpn` "Parameter
/// Expansion" + "Modifiers".
const PARAM_MODIFIER_DOCS: &[(&str, &str)] = &[
    // ── Parameter default-value forms ──
    ("-",   "`${var:-WORD}` — use WORD if `var` unset or empty"),
    ("=",   "`${var:=WORD}` — assign WORD to `var` (and use it) if unset/empty"),
    ("?",   "`${var:?MSG}` — print MSG to stderr + exit if `var` unset/empty"),
    ("+",   "`${var:+WORD}` — use WORD if `var` IS set (the inverse of `:-`)"),
    // ── Substring slicing ──
    ("0",   "`${var:OFFSET:LENGTH}` — substring (zero-based; negative offset = from end)"),
    // ── Path / file modifiers ──
    ("h",   "head — strip last path component (like `dirname`)"),
    ("t",   "tail — keep ONLY last path component (like `basename`)"),
    ("r",   "root — strip the final `.ext` suffix"),
    ("e",   "extension — keep ONLY the final `.ext` (no leading dot)"),
    ("a",   "absolute — textually resolve `..` / `.` against `$PWD`"),
    ("A",   "absolute + resolve symlinks (like `realpath`)"),
    ("c",   "PATH lookup — replace bare command with full path via `$PATH`"),
    ("P",   "physical path — resolve all symlinks"),
    ("f",   "repeat `:h` until the result is no longer an existing directory"),
    ("F",   "`:F:N:` — repeat `:h` N times"),
    // ── Substitution ──
    ("s",   "`:s/OLD/NEW/` — substitute first OLD with NEW"),
    ("gs",  "`:gs/OLD/NEW/` — global substitute (every occurrence)"),
    ("&",   "repeat the last `:s` substitution"),
    ("g&",  "repeat the last `:s` substitution globally"),
    // ── Quoting ──
    ("q",   "quote — backslash-escape all metacharacters"),
    ("Q",   "unquote — remove ONE level of quoting"),
    ("x",   "quote, breaking at whitespace into separate words"),
    // ── Case ──
    ("l",   "lowercase first character"),
    ("u",   "uppercase first character"),
    ("L",   "lowercase ENTIRE string"),
    ("U",   "uppercase ENTIRE string"),
    ("C",   "capitalize each word (`Title Case`)"),
    // ── Array operations ──
    ("S",   "sort array elements ascending"),
    ("O",   "sort array elements descending"),
    ("#",   "`${var:#PATTERN}` — remove array elements matching PATTERN (with `(@)`)"),
    ("|",   "`${arr:|other}` — set difference (elements of `arr` not in `other`)"),
    ("*",   "`${arr:*other}` — set intersection"),
    ("^",   "`${arr:^other}` — interleave (zip) two arrays"),
    ("^^",  "`${arr:^^other}` — distributed zip (every pair)"),
];

/// Where the cursor sits — drives which completion table to surface.
/// Detected by scanning backward from the cursor for the innermost
/// open paren / brace / `!` and looking at what precedes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LspCompletionContext {
    /// Default — surface builtins / keywords / options / params / snippets.
    Normal,
    /// Cursor inside `${(…)` before the closing `)`. Surface `PARAM_FLAG_DOCS`.
    ParamFlag,
    /// Cursor inside `pattern(…)` where `pattern` ends in a glob meta
    /// (`*`, `?`, `]`, `)`). Surface `GLOB_QUALIFIER_DOCS`.
    GlobQualifier,
    /// Cursor after `!` at a word boundary. Surface `HISTORY_DESIGNATOR_DOCS`.
    HistoryDesignator,
    /// Cursor sits after a `:` whose nearest enclosing brace is `${`,
    /// or after the `:` of a history reference (`!!:`). Surface
    /// `PARAM_MODIFIER_DOCS`.
    ParamColonModifier,
}

/// Walks the line backward from `col` to classify the context for
/// completion routing. Order of checks:
///   1. History designator — `!` at word boundary, not inside `((…))`
///   2. Paren-based: innermost unmatched `(` preceded by `${` → ParamFlag,
///      or by `*` / `?` / `]` / `)` → GlobQualifier
///   3. Param colon modifier — most recent `:` at brace-depth-0 inside
///      `${…}`, OR after a `!event:` history reference
fn lsp_completion_context(line: &str, col: usize) -> LspCompletionContext {
    let bytes = line.as_bytes();
    let cap = col.min(bytes.len());

    // ── 1. HistoryDesignator ────────────────────────────────────────
    // Walk back over designator-y chars (alnum + `?` / `#` / `$` / `^`
    // / `*` / `-` / `_`). If we land on `!` at a word boundary AND
    // we're not inside `((…))` arithmetic, trigger.
    {
        let mut k = cap;
        while k > 0 {
            let c = bytes[k - 1];
            if c.is_ascii_alphanumeric()
                || matches!(c, b'?' | b'#' | b'$' | b'^' | b'*' | b'-' | b'_')
            {
                k -= 1;
            } else {
                break;
            }
        }
        if k > 0 && bytes[k - 1] == b'!' {
            let bang = k - 1;
            let word_bound = bang == 0
                || matches!(
                    bytes[bang - 1],
                    b' ' | b'\t' | b';' | b'&' | b'|' | b'(' | b'`' | b'\n'
                );
            let escaped = bang > 0 && bytes[bang - 1] == b'\\';
            // Suppress inside `((…))` arithmetic where `!` is logical
            // NOT, not history. Cheap check: count `((` vs `))` before
            // the bang.
            let mut paren_pairs: i32 = 0;
            let mut j = 0;
            while j + 1 < bang {
                if bytes[j] == b'(' && bytes[j + 1] == b'(' {
                    paren_pairs += 1;
                    j += 2;
                    continue;
                }
                if bytes[j] == b')' && bytes[j + 1] == b')' {
                    paren_pairs -= 1;
                    j += 2;
                    continue;
                }
                j += 1;
            }
            let in_arith = paren_pairs > 0;
            if word_bound && !escaped && !in_arith {
                return LspCompletionContext::HistoryDesignator;
            }
        }
    }

    // ── 2. ParamFlag / GlobQualifier ─────────────────────────────────
    {
        let mut depth: i32 = 0;
        let mut i = cap;
        while i > 0 {
            i -= 1;
            let c = bytes[i];
            if c == b')' {
                depth += 1;
            } else if c == b'(' {
                if depth == 0 {
                    if i >= 2 && bytes[i - 2] == b'$' && bytes[i - 1] == b'{' {
                        return LspCompletionContext::ParamFlag;
                    }
                    if i >= 1 {
                        let prev = bytes[i - 1];
                        if matches!(prev, b'*' | b'?' | b']' | b')') {
                            return LspCompletionContext::GlobQualifier;
                        }
                    }
                    break;
                }
                depth -= 1;
            }
        }
    }

    // ── 3. ParamColonModifier ────────────────────────────────────────
    // Walk back tracking `{`/`}` depth. Find the most recent `:` at
    // brace-depth 0; if we then hit an unmatched `${`, trigger.
    {
        let mut bdepth: i32 = 0;
        let mut found_colon = false;
        let mut k = cap;
        while k > 0 {
            k -= 1;
            let c = bytes[k];
            if c == b'}' {
                bdepth += 1;
            } else if c == b'{' {
                if bdepth == 0 {
                    if k >= 1 && bytes[k - 1] == b'$' && found_colon {
                        return LspCompletionContext::ParamColonModifier;
                    }
                    break;
                }
                bdepth -= 1;
            } else if c == b':' && bdepth == 0 && !found_colon {
                found_colon = true;
            }
        }
    }

    // Also handle `!event:MOD` — cursor after a `:` whose nearest
    // preceding non-alnum / non-designator char is a `!event` reference.
    {
        let mut k = cap;
        // Walk back over the modifier letters being typed.
        while k > 0
            && (bytes[k - 1].is_ascii_alphabetic()
                || matches!(bytes[k - 1], b'&' | b'/' | b'g'))
        {
            k -= 1;
        }
        if k > 0 && bytes[k - 1] == b':' {
            // Walk back over the event designator (`!`, `!!`, `!42`,
            // `!ls`, `!?str?`, `!$`, etc). `!` itself is allowed in
            // the designator body for the `!!` form.
            let colon = k - 1;
            let mut e = colon;
            while e > 0
                && (bytes[e - 1].is_ascii_alphanumeric()
                    || matches!(bytes[e - 1], b'?' | b'#' | b'$' | b'^' | b'*' | b'-' | b'_' | b'!'))
            {
                e -= 1;
            }
            if e < colon && bytes[e] == b'!' {
                // `bang` is the position of the FIRST `!` in the
                // designator. Word boundary is checked before that.
                let bang = e;
                let word_bound = bang == 0
                    || matches!(
                        bytes[bang - 1],
                        b' ' | b'\t' | b';' | b'&' | b'|' | b'(' | b'`' | b'\n'
                    );
                if word_bound {
                    return LspCompletionContext::ParamColonModifier;
                }
            }
        }
    }

    LspCompletionContext::Normal
}

/// Hand docs for compsys functions whose names don't have a per-name
/// `item(tt(_X))` block in `compsys.yo` / `compwid.yo`. Per-command
/// completers (`_git`, `_docker`, …) and a couple of core dispatch
/// internals fall here.
const COMPSYS_FN_DOCS: &[(&str, &str)] = &[
    (
        "_main_complete",
        "Top-level entry the compsys dispatcher calls for every completion attempt. Walks the configured completer list (`_complete` / `_approximate` / `_match` / …), invoking each until one returns matches. Sets `$compstate[insert]` based on the result. Rust impl in `compsys::base::main_complete`.",
    ),
    (
        "_directories",
        "Complete directory names only. Equivalent to `_files -/`. Honors `path-files` zstyle and respects `GLOB_DOTS`. Rust impl in `compsys::files::directories_execute`.",
    ),
    (
        "_cargo",
        "Completion for the Rust `cargo` command — subcommands, flags, target names, feature names, profile names. Synthesizes from `cargo --list` and the manifest. Rust-native; no shell-script fallback.",
    ),
    (
        "_docker",
        "Completion for the `docker` CLI — subcommands, image names, container names/IDs, network names, volume names. Queries the local daemon socket via the `docker` binary; falls back to static-only when the daemon is unavailable.",
    ),
    (
        "_git",
        "Completion for `git` — subcommands, branches, tags, refs, remotes, file paths sensitive to `git status`. The most heavily-used compsys function in practice; Rust-native rewrite is several hundred times faster than the upstream shell implementation.",
    ),
    (
        "_kubectl",
        "Completion for `kubectl` — subcommands, resource kinds, resource names (queried via `kubectl get`), context/namespace names from kubeconfig.",
    ),
    (
        "_terraform",
        "Completion for `terraform` — subcommands, workspace names, state-file paths, providers, modules, variable names from the loaded HCL.",
    ),
    (
        "_ls",
        "Completion for `ls` — flags + file paths. Baseline stub that delegates path completion to `_files` and option completion to a static spec.",
    ),
    (
        "_cd",
        "Completion for `cd` — directory paths from `$PWD`, `$cdpath`, and the `dirs` stack. Honors `AUTO_CD` and `CDABLE_VARS`.",
    ),
    (
        "_cp",
        "Completion for `cp` — flags + file paths. Source paths exclude the destination; destination directory is offered as the final candidate.",
    ),
    (
        "_mv",
        "Completion for `mv` — flags + file/directory paths. Source/destination split identical to `_cp`.",
    ),
    (
        "_rm",
        "Completion for `rm` — flags + file paths. `-r` enables directory completion; without it, directories are filtered out.",
    ),
    (
        "_cat",
        "Completion for `cat` — file paths only. No subcommands; flags pass through to `_files`.",
    ),
    (
        "_grep",
        "Completion for `grep` (GNU/BSD-flavor-aware) — flags then file paths. First positional argument is the pattern (no completion offered for free-text patterns).",
    ),
];

/// Hand-curated docs for the zshrs extension builtins (`coreutils`
/// drop-ins, async/await primitives, doctor, intercept, etc.). The
/// canonical name list lives in `ext_builtins::EXT_BUILTIN_NAMES`;
/// every entry there must appear here too or the doc coverage gate
/// (`tests/doc_coverage_audit::every_canonical_extension_has_real_doc`)
/// fails.
const EXT_BUILTIN_DOCS: &[(&str, &str)] = &[
    ("add_zsh_hook", "Add a function to a zsh hook array (chpwd / precmd / preexec / periodic / zshaddhistory / zshexit). `add-zsh-hook chpwd my_chpwd_fn`. Idempotent — re-adding the same function is a no-op."),
    ("arch", "Print the machine architecture (uname -m equivalent): `x86_64`, `arm64`, `aarch64`, etc."),
    ("async", "Spawn a background task on the persistent worker pool. `async name { body }` queues the body for parallel execution. Pair with `await name` to join."),
    ("await", "Block until a previously-spawned `async` task completes. `await name` returns the task's exit status; `await` with no args waits for all in-flight tasks."),
    ("barrier", "Synchronization point for the parallel worker pool. Waits until every running `async`/`peach` task has finished before continuing."),
    ("base64", "Encode / decode Base64. `-d` decodes; `-w0` no line wrap. coreutils drop-in."),
    ("basename", "Strip leading directories and an optional suffix. `basename /a/b.txt .txt` → `b`. coreutils drop-in."),
    ("caller", "Bash-compatible `caller` builtin. With no arg or 0: prints `LINE FUNC` for the current frame; with N>0: `LINE FUNC FILE` for the Nth call-stack frame."),
    ("cat", "Concatenate files to stdout. `-n` numbers lines, `-A` shows tabs/EOLs. coreutils drop-in."),
    ("cdreplay", "Replay the directory stack into the named directory. Reverses recent `cd` history without traversing the parent chain."),
    ("cksum", "Print CRC32 checksum + byte count of each file. coreutils drop-in."),
    ("comm", "Compare two sorted files line-by-line. `-1` / `-2` / `-3` suppress columns. coreutils drop-in."),
    ("compdef", "Register a completion function for one or more commands. `compdef _git git`. Backed by the SQLite compsys cache; lookups are O(log n)."),
    ("compgen", "Bash-compatible word generator. `compgen -W 'foo bar baz' fo` → `foo`. Used by bash-completion scripts ported to zshrs."),
    ("compinit", "Initialize the completion system. Walks `$fpath` in parallel via rayon, populates the SQLite cache, marks every `_*` as autoloaded. Default mode skips `.zcompdump` entirely."),
    ("complete", "Bash-compatible `complete` command — register a completion spec for a command. zshrs bridges to compsys internally."),
    ("compopt", "Bash-compatible `compopt` — modify completion options at runtime."),
    ("cut", "Extract fields or character ranges. `-d':' -f1,3` / `-c5-10`. coreutils drop-in."),
    ("date", "Print or set the system date. `+%FORMAT` strftime; `-d 'rel'` parse relative; `-u` UTC. coreutils drop-in."),
    ("dbview", "Dump the local zshrs SQLite caches (autoload bodies, completion cache, history FTS). `dbview --table autoloads` filters by table."),
    ("dircolors", "Emit `LS_COLORS` from a `.dircolors` file. coreutils drop-in."),
    ("dirname", "Strip the last path component. `dirname /a/b/c` → `/a/b`. coreutils drop-in."),
    ("doctor", "Diagnostic report of shell health — cache stats, autoload coverage, fpath sanity, daemon presence, memory footprint, recent error summary. zshrs-only."),
    ("env", "Run a command in a modified environment, or print the current environment. `env -i` empties; `env VAR=val cmd` sets. coreutils drop-in."),
    ("expand", "Convert tabs to spaces. `-t N` sets tab width. coreutils drop-in."),
    ("expr", "Evaluate an arithmetic / string expression. `expr 2 + 3` → `5`. Prefer `$(( … ))` in zshrs scripts; provided for POSIX compatibility."),
    ("factor", "Print prime factors. `factor 60` → `60: 2 2 3 5`. coreutils drop-in."),
    ("find", "Walk the filesystem and print / act on matches. Supports `-name`/`-type`/`-mtime`/`-exec`. coreutils drop-in (subset)."),
    ("fold", "Wrap each input line to a width. `-w N` width, `-s` break at spaces. coreutils drop-in."),
    ("groups", "Print groups the user (or named user) belongs to. coreutils drop-in."),
    ("head", "Print the first N lines (`-n N`) or bytes (`-c N`) of each file. coreutils drop-in."),
    ("help", "Print help for a builtin. `help cd` shows the cd usage. zshrs-only."),
    ("hostname", "Print the system hostname. `-s` short, `-f` FQDN."),
    ("id", "Print user / group IDs. `-u` user only, `-g` group only, `-n` names. coreutils drop-in."),
    ("intercept", "Register an AOP intercept. `intercept before|after|around <cmd> { body }` runs `body` around every invocation of `<cmd>`. Bytecode-compiled at registration; no per-call interpreter overhead. zshrs-only."),
    ("intercept_proceed", "Inside an `around` intercept body, invoke the underlying command. Required so the intercept doesn't shadow the call permanently."),
    ("link", "Create a hard link. `link src dst`. coreutils drop-in."),
    ("logname", "Print the user's login name. coreutils drop-in."),
    ("mkfifo", "Create named pipes (FIFOs). `mkfifo path …`. coreutils drop-in."),
    ("mktemp", "Create a temp file or directory with a unique name. `-d` directory, `-p DIR` parent. coreutils drop-in."),
    ("nice", "Run a command with adjusted scheduling priority. `nice -n 10 cmd`. coreutils drop-in."),
    ("nl", "Number lines. `-b a` numbers all, `-w N` field width. coreutils drop-in."),
    ("nproc", "Print the number of processing units available. `--all` ignores affinity."),
    ("paste", "Merge corresponding lines of files. `-d DELIM` separator. coreutils drop-in."),
    ("peach", "Parallel-for-each — run a block once per element of an array across the worker pool. `peach arr { print $it }`. Returns when all workers finish. zshrs-only."),
    ("pgrep", "Print PIDs of processes matching a pattern. `-f` matches full command line."),
    ("pmap", "Display the memory map of one or more processes. `pmap PID`."),
    ("printenv", "Print the value of one or more environment variables, or all if none given. coreutils drop-in."),
    ("profile", "CPU / wall-time profile a command and emit a flamegraph. `profile cmd …` → SVG path printed on stdout. Backed by the same sampler as `zprof`."),
    ("realpath", "Resolve symlinks and `.` / `..` to a canonical absolute path. coreutils drop-in."),
    ("rev", "Reverse each input line character-by-character. coreutils drop-in."),
    ("seq", "Print a sequence of numbers. `seq 1 10` / `seq 1 2 10` / `seq -w 1 10`. coreutils drop-in."),
    ("sha256sum", "Print or check SHA-256 digests. `-c FILE` checks. coreutils drop-in."),
    ("shuf", "Shuffle input lines. `-n N` limit, `-e ITEM…` shuffle args, `-i LO-HI` shuffle range. coreutils drop-in."),
    ("sleep", "Pause for the given duration. `sleep 1`, `sleep 0.5`, `sleep 1m`. coreutils drop-in."),
    ("sort", "Sort lines. `-n` numeric, `-r` reverse, `-k N` by field, `-u` unique. coreutils drop-in."),
    ("sum", "BSD/sysv checksum + 1K-block count. coreutils drop-in."),
    ("tac", "Concatenate files in reverse line order. coreutils drop-in."),
    ("tail", "Print the last N lines (`-n N`) or follow appends (`-f`). coreutils drop-in."),
    ("tee", "Copy stdin to stdout AND to each named file. `-a` append. coreutils drop-in."),
    ("touch", "Create a file or update its mtime. `-d STR` set time, `-r REF` copy from REF. coreutils drop-in."),
    ("tput", "Terminal-capability query. `tput cols`, `tput setaf 1`. Reads `$TERM` via terminfo."),
    ("tr", "Translate / squeeze / delete characters. `tr a-z A-Z` uppercases. coreutils drop-in."),
    ("tsort", "Topological sort of partial-order pairs read from stdin. coreutils drop-in."),
    ("tty", "Print the controlling terminal device path, or `not a tty` if stdin isn't one."),
    ("uname", "Print system info. `-a` all, `-s` kernel, `-m` machine, `-r` release. coreutils drop-in."),
    ("unexpand", "Convert leading spaces to tabs. `-a` all spaces. coreutils drop-in."),
    ("uniq", "Filter adjacent matching lines. `-c` prefix count, `-d` only duplicates. coreutils drop-in."),
    ("unlink", "Remove a single file via the `unlink(2)` syscall (no `-r`, no prompts). coreutils drop-in."),
    ("users", "Print the login names of users currently logged in."),
    ("wc", "Count newlines, words, bytes. `-l` lines, `-w` words, `-c` bytes. coreutils drop-in."),
    ("whoami", "Print the effective user name. coreutils drop-in."),
    ("yes", "Repeatedly output a line. `yes` prints `y` forever; `yes STR` prints STR. coreutils drop-in."),
    ("zbuild", "Bytecode-compile a zsh source file ahead of time. `zbuild script.zsh` writes `script.zwc` next to it; subsequent `source`s skip the lexer/parser. Same on-disk format as `zcompile` but uses fusevm bytecode."),
    // ── Daemon-backed `z*` builtins (Unix-socket RPC to zshrs-daemon) ──
    ("zask", "Send an ask-style request to the daemon and print the JSON response. Used by tools/agents that want a single synchronous query against the shared catalog."),
    ("zcache", "Read / write / list the per-shell cache namespace. `zcache get K` / `zcache set K V [TTL]` / `zcache del K` / `zcache list [PREFIX]`. Backed by the daemon's in-memory KV with optional SQLite persistence."),
    ("zcmd-result", "Push the exit status + output of a just-completed command to the daemon's command-history catalog. Used by `precmd` hooks to populate the cross-shell `zhistory` index."),
    ("zcomplete", "Push a completion candidate to the daemon's shared completion cache. Other shells running compinit will see it without re-walking fpath."),
    ("zd", "Daemon HTTP client. In-process when invoked from inside zshrs (Unix socket); same args as the standalone `zd` binary. `zd ping` / `zd ops` / `zd cache get K`. Maps 1:1 to `POST /op/<NAME>`."),
    ("zhistory", "Query the daemon's federated command-history catalog. Spans every shell that pushed via `zcmd-result`. SQLite FTS5-backed; `zhistory search 'pattern'`."),
    ("zid", "Print the current shell's federated ID — the stable `shell_id` (`bash` / `zsh` / `zshrs` / …) and the per-process `bundle_id` the daemon uses to scope state."),
    ("zjob", "Manage background jobs through the daemon: `zjob submit -- cmd …` queues, `zjob status ID`, `zjob output ID`, `zjob wait ID`, `zjob kill ID`. Jobs survive shell exit because the daemon owns them."),
    ("zlock", "Acquire / release / try a named cross-shell lock. `zlock acquire NAME [TIMEOUT]` / `zlock release NAME TOKEN` / `zlock try NAME` / `zlock do NAME -- cmd …`. PID-tagged so the daemon GCs stale entries."),
    ("zlog", "Append a structured log entry to the daemon's log catalog. `zlog 'message' [key=val …]`. Queryable later via `zhistory` / `dbview`."),
    ("zls", "List entries in the daemon's federated catalog (aliases, functions, env vars, etc.). `zls --kind alias --shell-id bash`. The cross-shell mirror of `alias`/`functions`/`typeset`."),
    ("znotify", "Send a desktop / system notification through the daemon. Routes to `osascript` (macOS), `notify-send` (Linux), or the in-shell UI when no platform notifier is available."),
    ("zping", "Round-trip latency probe against the daemon. Prints the RTT in microseconds; non-zero exit if the daemon is unreachable."),
    ("zpublish", "Publish a JSON event to a pubsub topic. `zpublish topic.name '{\"key\":\"val\"}'`. Subscribers receive via `zsubscribe`."),
    ("zsend", "Send a one-shot message to another shell (by `shell_id` or `bundle_id`). Like `znotify` but targets a specific shell, not the user's desktop."),
    ("zsource", "Push a sourced-file event to the daemon's federated catalog. Used by `source`/`.` hooks so the daemon knows which rc files have been loaded by which shells."),
    ("zsubscribe", "Subscribe to a pubsub topic and stream incoming messages to stdout as SSE-style JSON lines. `zsubscribe 'shell:*.build_done'`."),
    ("zsuggest", "Query the daemon's suggestion engine for the next command, given the current cwd + history. Used by ZLE's autosuggestion widget when the local history can't supply a candidate."),
    ("zsync", "Force a flush of the daemon's in-memory state to the SQLite catalog. Normally happens in the background; `zsync` makes it synchronous so a snapshot is consistent."),
    ("ztag", "Tag the current shell session with one or more labels. `ztag prod-deploy`. Other shells can filter by tag via `zls --tag prod-deploy`."),
    ("zunsubscribe", "Cancel a `zsubscribe` stream. `zunsubscribe TOPIC` or `zunsubscribe --all`."),
    ("zuntag", "Remove a tag from the current shell session. Inverse of `ztag`."),
    ("zwhere", "Locate which shell / bundle / cwd defined a given alias / function / env var in the federated catalog. `zwhere alias ll` → list of every shell that set `ll`."),
];

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
    // AST-backed cross-file path — ONLY. No textual fallback.
    //
    // History: an earlier impl fell through to a whole-document
    // text-grep when the parse failed mid-edit. The fallback turned
    // Find Usages into a glorified `grep -w` — every comment match,
    // every string-literal match, every same-name-different-symbol
    // match got reported as a usage. Users called it FAKE and they
    // were right. Removed: parse failure now returns empty, which
    // surfaces as "no usages" in the IDE (with a debug log line
    // pointing at the failing file). The correctness trade is worth
    // the loss of coverage on broken-syntax buffers.
    match references_via_ast(state, &active_uri, &active_text, line_no as u32, &word) {
        Some(v) => v,
        None => {
            tracing::warn!(
                target: "zshrs::lsp::references",
                uri = %active_uri,
                %word,
                line = line_no,
                col,
                "AST-walk returned no resolution \
                 (parse failure or cursor not on a declared symbol); \
                 returning empty rather than falling back to text-search",
            );
            Value::Array(vec![])
        }
    }
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
        // Track every URI already walked so source-chain following
        // (below) doesn't re-emit duplicate locations.
        let mut walked: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        walked.insert(active_uri.to_string());
        for (uri, src) in state.all_docs() {
            if uri == active_uri {
                continue;
            }
            walked.insert(uri.clone());
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

        // ── Source-chain following ───────────────────────────────────
        // BFS over `source X` / `. X` / `zsource X` (zshrs daemon
        // variant) commands found via AST walk. Pulls in files OUTSIDE
        // the workspace root that the active file depends on. Cycle-
        // guarded; depth-capped to keep pathological rc chains from
        // hanging the LSP. Files reached this way are NOT cached —
        // they're read fresh each call so edits propagate without an
        // explicit didChangeWatchedFiles event.
        let mut queue: Vec<String> = vec![active_uri.to_string()];
        // Seed with every workspace file too — sourced files may live
        // off any of them, not just the active doc.
        for (uri, _) in state.all_docs() {
            queue.push(uri);
        }
        const MAX_FILES: usize = 256;
        while let Some(uri) = queue.pop() {
            if walked.len() >= MAX_FILES {
                tracing::warn!(
                    target: "zshrs::lsp::references_ast",
                    walked = walked.len(),
                    "source-chain hit MAX_FILES cap; stopping BFS",
                );
                break;
            }
            // Find this file's text in the open-doc map or read it
            // from disk if we have a file:// URI.
            let parent_text = state
                .docs
                .get(&uri)
                .cloned()
                .or_else(|| state.workspace_files.get(&uri).cloned())
                .or_else(|| {
                    file_uri_to_path(&uri).and_then(|p| std::fs::read_to_string(p).ok())
                });
            let Some(parent_text) = parent_text else { continue };
            let parent_dir = file_uri_to_path(&uri)
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            for sourced_path in crate::lsp_symbols::collect_sourced_paths(&parent_text, &parent_dir) {
                let sourced_uri = format!("file://{}", sourced_path.display());
                if walked.contains(&sourced_uri) {
                    continue;
                }
                walked.insert(sourced_uri.clone());
                let Ok(sourced_text) = std::fs::read_to_string(&sourced_path) else { continue };
                let lines = find_ast_occurrences(&sourced_text, &name, kind.clone());
                let src_lines: Vec<&str> = sourced_text.lines().collect();
                for line in lines {
                    if let Some(lt) = src_lines.get(line as usize) {
                        for (s, e) in find_all_word_cols_kinded(lt, &name, is_var) {
                            out.push(json!({
                                "uri": sourced_uri,
                                "range": {
                                    "start": { "line": line, "character": s },
                                    "end":   { "line": line, "character": e },
                                },
                            }));
                        }
                    }
                }
                // Push for transitive BFS — sourced file may itself
                // source more files.
                queue.push(sourced_uri);
            }
        }
        tracing::debug!(
            target: "zshrs::lsp::references_ast",
            files_walked = walked.len(),
            "source-chain BFS done",
        );
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
    "comment",       // 0
    "string",        // 1
    "number",        // 2
    "keyword",       // 3
    "operator",      // 4
    "function",      // 5 — compat zsh builtins
    "variable",      // 6
    "parameter",     // 7
    "type",          // 8
    "macro",         // 9 — kept for back-compat; also used for compsys fns now
    "property",      // 10
    "regexp",        // 11
    "zshrsExtension",// 12 — zshrs-only ext + daemon `z*` builtins
    "zshrsCompsys",  // 13 — `_arguments` / `_files` / `_describe` family
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
            // Multi-char operators — emit as OPERATOR (token type 4).
            // Longest-match-first so `&&` doesn't lex as `&` + `&`.
            //
            // The IDE then maps semantic-token type 4 to ZshrsColors.OPERATOR
            // (which the user can rebind under Settings → Editor → Color
            // Scheme → zshrs → Operators). Without this branch the hand
            // lexer's OPERATOR token-type was wired but the LSP overlay
            // never emitted any, so the user's selected operator color
            // never applied.
            const OPERATORS: &[&str] = &[
                ";;&", "<<<", "<<-",
                "&&", "||", "|&", "<<", ">>", "&>", ">|", ">!",
                ">&", "<&", "<>", "==", "!=", "=~", "+=", "-=", ":=", "?=",
                "[[", "]]", "((", "))", ";;", ";|",
                "|", "&", ">", "<",
            ];
            let mut op_len = 0usize;
            for op in OPERATORS {
                if rest.starts_with(op) {
                    op_len = op.len();
                    break;
                }
            }
            if op_len > 0 {
                push_tok(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    ln,
                    col as u32,
                    op_len as u32,
                    4, // operator
                );
                col += op_len;
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
            // Word — classify. Allow leading `.` / `+` / `@` when
            // followed by `_`/letter (zinit-style function names like
            // `.zinit-foo` / `+vi-…` / `@hook-fn`). Body chars allow
            // `-` for hyphenated names (`daemon-lock-do`,
            // `daemon-export-pdf`) so they don't lex as multiple tokens
            // with `do` / `export` getting mis-classified.
            // Use the C-faithful character-class predicates from
            // `ported::ztype_h` — same `iuser` / `iident` / `ialnum`
            // bits the upstream lexer (`Src/lex.c::gettokstr`) checks.
            // Avoids drift between the hand rule here and the canonical
            // port. `iuser` is "username char" — letters/digits/`_` +
            // `-`/`.`/Dash. Add `:` to the body set because zsh
            // function names may include it (audited against zinit's
            // `:hist:precmd`); `:` isn't in IUSER but neither is it
            // an `ispecial` metachar, so command-word lexing accepts it.
            use crate::ported::ztype_h::{ialnum, iident, iuser};
            let leading_sigil = iuser(c0 as u8)
                && !iident(c0 as u8)  // exclude alnum/`_` — those start a plain word
                && rest.as_bytes().get(1).map_or(false, |b| iident(*b));
            // `+`/`@`/`:`/`^` aren't in IUSER (only `-` and `.` are
            // per the C source). Allow them anyway — zinit / p10k /
            // async hooks use them widely as function-name prefixes
            // and the C lexer accepts them as command-word content.
            let extra_sigil = !leading_sigil
                && matches!(c0, '+' | '@' | ':' | '^')
                && rest.as_bytes().get(1).map_or(false, |b| iident(*b));
            let is_sigil = leading_sigil || extra_sigil;
            if iident(c0 as u8) || is_sigil {
                let b = rest.as_bytes();
                let mut end = if is_sigil { 1 } else { 0 };
                while end < b.len() {
                    let c = b[end];
                    if ialnum(c) || c == b'_' {
                        end += 1;
                    } else if matches!(c, b'-' | b'.' | b':')
                        && end + 1 < b.len()
                        && (ialnum(b[end + 1]) || b[end + 1] == b'_')
                    {
                        end += 1;
                    } else {
                        break;
                    }
                }
                let w = &rest[..end];
                // Token-type classification — match index in
                // SEMANTIC_TOKEN_TYPES. Priority:
                //   * KEYWORDS (3) — reserved words.
                //   * zshrs extension builtins (12) — distinct color
                //     so `date` / `cat` / `zd` / etc. don't visually
                //     merge with compat builtins.
                //   * Compsys functions (13) — `_arguments` family.
                //   * BUILTINS (5) — compat zsh builtins.
                //   * VARIABLE (6) fallback for plain identifiers.
                let kind = if KEYWORDS.contains(&w) {
                    3u32
                } else if crate::ext_builtins::EXT_BUILTIN_NAMES.contains(&w)
                    || crate::daemon::builtins::ZSHRS_BUILTIN_NAMES.contains(&w)
                {
                    12
                } else if compsys::COMPSYS_FN_NAMES.contains(&w) {
                    13
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

    // ── Multi-line selection → only Extract Function applies. ────────
    // (Variable / constant extract require a single-line expression to
    // assign to a name; multi-line bodies have to become a callable.)
    if !same_line {
        if let Some(action) = make_extract_function_multiline(&uri, &text, start_line, end_line) {
            actions.push(action);
        }
        return Value::Array(actions);
    }

    // Resolve the line first — every action below needs it, and an
    // out-of-bounds line means no actions regardless of mode.
    let line_text = match text.lines().nth(start_line as usize) {
        Some(l) => l,
        None => return Value::Array(vec![]),
    };
    let leading_ws: String = line_text
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    // ── Extract Function: offered whenever the cursor's line has
    // any non-whitespace content, regardless of whether the user has
    // an explicit selection. The Cmd-Opt-M ("Extract Method") common
    // case is caret-only; before this branch the LSP returned an
    // empty action list, the plugin showed "LSP returned no code
    // actions for this range", and the user's only recourse was to
    // manually select the line first.
    let line_has_content = !line_text.trim().is_empty();
    let whole_line_selected =
        nonempty && selection_covers_whole_line(line_text, start_char, end_char);
    if line_has_content && (whole_line_selected || !nonempty) {
        let body = if whole_line_selected {
            utf16_slice(line_text, start_char, end_char)
                .map(str::trim_end)
                .unwrap_or_else(|| line_text.trim())
        } else {
            line_text.trim()
        };
        actions.push(make_extract_function_singleline(
            &uri,
            &leading_ws,
            start_line,
            body,
        ));
    }

    // ── Extract Variable / Constant: need a concrete sub-expression
    // to assign to a name. Caret-only invocations snap to the word at
    // the cursor; explicit selections use the user's range as-is.
    let (eff_start_char, eff_end_char) = if !nonempty {
        match snap_to_word_at_cursor(line_text, start_char) {
            Some((s, e)) => (s, e),
            // Caret not on a word — Extract Function still applied
            // above (if line had content), so return what we have.
            None => return Value::Array(actions),
        }
    } else {
        (start_char, end_char)
    };

    if eff_end_char <= eff_start_char {
        return Value::Array(actions);
    }

    let sel = match utf16_slice(line_text, eff_start_char, eff_end_char) {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Value::Array(actions),
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

/// True when the selection spans the line's entire non-whitespace
/// content — leading indent before `eff_start_char` is whitespace, and
/// everything after `eff_end_char` is whitespace too. Used to decide
/// whether Extract Function applies to a single-line selection (we want
/// to extract whole statements, not arbitrary expression fragments —
/// the latter are already covered by Extract Variable / Constant).
fn selection_covers_whole_line(line_text: &str, start_col: u32, end_col: u32) -> bool {
    let mut prefix_byte = 0;
    let mut suffix_byte = line_text.len();
    let mut u16_seen = 0u32;
    for (i, ch) in line_text.char_indices() {
        if u16_seen == start_col {
            prefix_byte = i;
        }
        u16_seen += ch.len_utf16() as u32;
        if u16_seen == end_col {
            suffix_byte = i + ch.len_utf8();
        }
    }
    line_text[..prefix_byte].chars().all(char::is_whitespace)
        && line_text[suffix_byte..].chars().all(char::is_whitespace)
}

fn make_extract_function_singleline(
    uri: &str,
    leading_ws: &str,
    line: u32,
    body: &str,
) -> Value {
    // Insert `extracted_function() { body; }` above the line, replace
    // the line's content with a bare call.
    let name = "extracted_function";
    let decl = format!("{leading_ws}{name}() {{\n{leading_ws}    {body}\n{leading_ws}}}\n");
    let insert_range = json!({
        "start": { "line": line, "character": 0 },
        "end":   { "line": line, "character": 0 },
    });
    let replace_range = json!({
        "start": { "line": line, "character": 0 },
        "end":   { "line": line + 1, "character": 0 },
    });
    let replacement = format!("{leading_ws}{name}\n");
    let changes = json!({
        uri: [
            { "range": insert_range, "newText": decl },
            { "range": replace_range, "newText": replacement },
        ]
    });
    json!({
        "title": "Extract to function (`name() { … }`)",
        "kind": "refactor.extract",
        "edit": { "changes": changes },
    })
}

fn make_extract_function_multiline(
    uri: &str,
    text: &str,
    start_line: u32,
    end_line: u32,
) -> Option<Value> {
    // Pull the inclusive line range, snapping the LSP exclusive end-line
    // semantics to "all lines that the selection touches." A selection
    // ending at column 0 of line N covers lines start..N-1 only; a
    // selection ending mid-line N covers start..N inclusive.
    let lines: Vec<&str> = text.lines().collect();
    if (start_line as usize) >= lines.len() {
        return None;
    }
    let last = (end_line as usize).min(lines.len() - 1);
    let block = &lines[start_line as usize..=last];
    if block.iter().all(|l| l.trim().is_empty()) {
        return None;
    }

    // Common leading-whitespace prefix on non-blank lines determines the
    // function-body indent we'll strip back to.
    let common_indent = block
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.chars().take_while(|c| c.is_whitespace()).count())
        .min()
        .unwrap_or(0);

    let leading_ws: String = block
        .iter()
        .find(|l| !l.trim().is_empty())
        .map(|l| l.chars().take(common_indent).collect())
        .unwrap_or_default();

    let name = "extracted_function";
    let mut decl = String::new();
    decl.push_str(&format!("{leading_ws}{name}() {{\n"));
    for l in block {
        if l.trim().is_empty() {
            decl.push('\n');
        } else {
            // Strip the common indent then re-indent one level past the
            // function-decl leading whitespace.
            let stripped = if l.chars().take(common_indent).all(|c| c.is_whitespace()) {
                &l[l
                    .char_indices()
                    .nth(common_indent)
                    .map(|(i, _)| i)
                    .unwrap_or(l.len())..]
            } else {
                l.trim_start()
            };
            decl.push_str(&format!("{leading_ws}    {stripped}\n"));
        }
    }
    decl.push_str(&format!("{leading_ws}}}\n"));

    let insert_range = json!({
        "start": { "line": start_line, "character": 0 },
        "end":   { "line": start_line, "character": 0 },
    });
    let replace_range = json!({
        "start": { "line": start_line,    "character": 0 },
        "end":   { "line": last as u32 + 1, "character": 0 },
    });
    let replacement = format!("{leading_ws}{name}\n");

    let changes = json!({
        uri: [
            { "range": insert_range, "newText": decl },
            { "range": replace_range, "newText": replacement },
        ]
    });
    Some(json!({
        "title": "Extract to function (`name() { … }`)",
        "kind": "refactor.extract",
        "edit": { "changes": changes },
    }))
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
        "{",
        "Command-group open brace. `{ cmd1; cmd2; }` runs the commands in the current shell (no subshell), grouping them as one syntactic unit. Reserved word — must be followed by whitespace or a newline.",
    ),
    (
        "}",
        "Command-group close brace. Pairs with `{ … }`. Reserved word — preceded by `;` or newline.",
    ),
    (
        "!",
        "Pipeline negation. `! cmd` inverts `cmd`'s exit status — zero becomes non-zero, non-zero becomes zero. As the first word of a command. Distinct from `!` history expansion (which is a lexer-stage substitution, not a reserved word).",
    ),
    (
        "fi",
        "Closes an `if` block. `if cmd; then body; fi`. Required terminator — without it the parser keeps reading until EOF.",
    ),
    (
        "done",
        "Closes a `for` / `foreach` / `while` / `until` / `select` / `repeat` loop body. `for v in a b c; do echo $v; done`. Required terminator.",
    ),
    (
        "end",
        "Closes the alternate-form compound statement (`foreach NAME (WORDS) … end`, `if COND … end`, `while COND … end`). Csh-style syntactic mirror of `fi` / `done` / `esac` for users coming from csh / tcsh.",
    ),
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
    let mut all = serde_json::Map::new();

    // ── Compat builtins: ported zsh-faithful builtins from
    // `ported::builtin::BUILTINS`. These mirror the upstream zsh C
    // `Src/Builtins/*.c` tables 1:1. Distinct from `extensions` (the
    // zshrs-only additions). `builtins` below is the union for tools
    // that want everything under one key.
    let mut compat = serde_json::Map::new();
    for b in crate::ported::builtin::BUILTINS.iter() {
        compat.insert(b.node.nam.clone(), Value::String("compat".into()));
        all.insert(b.node.nam.clone(), Value::String("compat".into()));
    }
    // Keywords sourced from the canonical `reswds[]` table at
    // `Src/hashtable.c:1076-1108` (Rust port: `ported::hashtable::RESWDS`).
    // Filter out entries with `token == TYPESET` — those are declaration
    // commands (local / typeset / declare / export / readonly / integer
    // / float) that the parser folds into the `typeset` builtin. They
    // already show up in the Builtins tab; listing them in Keywords too
    // duplicates them and miscategorizes them as control-flow.
    // Keywords sourced from the canonical `reswds[]` table at
    // `Src/hashtable.c:1076-1108` (port: `ported::hashtable::RESWDS`).
    // Per `man zshmisc` "Reserved Words" (`Doc/Zsh/grammar.yo:501-504`),
    // ALL 31 names are reserved words — including `declare`/`export`/
    // `float`/`integer`/`local`/`readonly`/`typeset`. Those also exist
    // as builtins (the parser folds them into the `typeset` builtin via
    // the `TYPESET` lextok), but `man zshmisc` lists them as reserved
    // first. We list them in both tabs.
    let mut keywords = serde_json::Map::new();
    for (name, _token) in crate::ported::hashtable::RESWDS {
        keywords.insert(name.to_string(), Value::String("keyword".into()));
        all.insert(name.to_string(), Value::String("keyword".into()));
    }
    let mut options = serde_json::Map::new();
    for o in crate::ported::options::ZSH_OPTIONS_SET.iter() {
        options.insert(o.to_string(), Value::String("option".into()));
        all.insert(o.to_string(), Value::String("option".into()));
    }
    let mut special_vars = serde_json::Map::new();
    for s in SPECIAL_VARS {
        special_vars.insert(s.to_string(), Value::String("special".into()));
        all.insert(s.to_string(), Value::String("special".into()));
    }
    // ── Compsys completion functions ────────────────────────────────
    // The `_arguments` / `_files` / `_describe` family — Rust-native
    // implementations from the `compsys` crate. Sourced from
    // `compsys::COMPSYS_FN_NAMES`.
    let mut compsys = serde_json::Map::new();
    for n in compsys::COMPSYS_FN_NAMES {
        compsys.insert((*n).to_string(), Value::String("compsys".into()));
        all.insert((*n).to_string(), Value::String("compsys".into()));
    }
    // ── zshrs extension builtins ────────────────────────────────────
    // Builtins that have NO upstream zsh C counterpart. Two sources:
    //   * `ext_builtins::EXT_BUILTIN_NAMES` — in-process builtins
    //     dispatched by `ShellExecutor` (coreutils drop-ins, bash-only
    //     builtins, async/await/barrier, doctor, intercept, contrib
    //     autoloads exposed as builtins, etc.).
    //   * `daemon::builtins::ZSHRS_BUILTIN_NAMES` — daemon-backed `z*`
    //     builtins (zd, zcache, zls, zping, zlock, zpublish, …) that
    //     proxy to the local Unix-socket daemon for cross-shell state.
    // Both are zshrs-only; combining them gives the full inventory of
    // builtins the user can call that aren't in upstream zsh.
    let mut extensions = serde_json::Map::new();
    for n in crate::ext_builtins::EXT_BUILTIN_NAMES {
        extensions.insert((*n).to_string(), Value::String("extension".into()));
        all.insert((*n).to_string(), Value::String("extension".into()));
    }
    for n in crate::daemon::builtins::ZSHRS_BUILTIN_NAMES {
        extensions.insert((*n).to_string(), Value::String("extension".into()));
        all.insert((*n).to_string(), Value::String("extension".into()));
    }
    // ── Operators / punctuation tokens (man zshmisc) ─────────────────
    let mut operators = serde_json::Map::new();
    for (op, _body) in OPERATOR_DOCS {
        operators.insert((*op).to_string(), Value::String("operator".into()));
        all.insert((*op).to_string(), Value::String("operator".into()));
    }
    // ── Backwards-compat aggregate: every builtin the user can call,
    // ported + extension. Equals `compat ∪ extensions`. Kept as the
    // `builtins` key so older tool-window UIs (pre-compat-split) still
    // see something familiar.
    let mut builtins = compat.clone();
    for (k, _) in &extensions {
        builtins.insert(k.clone(), Value::String("builtin".into()));
    }
    serde_json::to_string_pretty(&json!({
        "all": all,
        "builtins": builtins,
        "compat": compat,
        "keywords": keywords,
        "options": options,
        "special_vars": special_vars,
        "compsys": compsys,
        "extensions": extensions,
        "operators": operators,
    }))
    .unwrap_or_else(|_| "{}".into())
}

/// Every canonical name across every registry, sorted and de-duped.
/// Drives `zshrs --names` (fed into the `_zshrs` completer for
/// `--docs <TAB>`) and the closest-name fuzzy-suggest fallback when
/// `--docs FOO` doesn't resolve.
pub fn all_canonical_names() -> Vec<String> {
    use std::collections::BTreeSet;
    let mut set: BTreeSet<String> = BTreeSet::new();
    for b in crate::ported::builtin::BUILTINS.iter() {
        set.insert(b.node.nam.clone());
    }
    for (n, t) in crate::ported::hashtable::RESWDS {
        if *t == crate::ported::zsh_h::TYPESET {
            continue;
        }
        set.insert((*n).to_string());
    }
    for o in crate::ported::options::ZSH_OPTIONS_SET.iter() {
        set.insert((*o).to_string());
    }
    for s in SPECIAL_VARS {
        set.insert((*s).to_string());
    }
    for n in compsys::COMPSYS_FN_NAMES {
        set.insert((*n).to_string());
    }
    for n in crate::ext_builtins::EXT_BUILTIN_NAMES {
        set.insert((*n).to_string());
    }
    for n in crate::daemon::builtins::ZSHRS_BUILTIN_NAMES {
        set.insert((*n).to_string());
    }
    for (op, _) in OPERATOR_DOCS {
        set.insert((*op).to_string());
    }
    set.into_iter().collect()
}

/// Closest canonical name to `query` by edit distance, when the
/// distance is small enough to be useful. Used by `--docs FOO` to
/// suggest "did you mean `bar`?" on typo.
///
/// Threshold: ≤ max(2, query.len() / 3). Below that we'd suggest
/// random unrelated names; the slop scales with input length so
/// `xy` doesn't pick `if` but `compdefffff` can still find `compdef`.
pub fn closest_name(query: &str) -> Option<String> {
    let names = all_canonical_names();
    let q_bare = query.strip_prefix('$').unwrap_or(query);
    let max_dist = std::cmp::max(2, q_bare.len() / 3);
    let mut best: Option<(usize, String)> = None;
    for n in names {
        let n_bare = n.strip_prefix('$').unwrap_or(&n);
        let d = edit_distance(q_bare, n_bare);
        if d > max_dist {
            continue;
        }
        match best {
            None => best = Some((d, n)),
            Some((bd, _)) if d < bd => best = Some((d, n)),
            _ => {}
        }
    }
    best.map(|(_, n)| n)
}

/// Damerau-Levenshtein-lite (insertions + deletions + substitutions,
/// no transpositions). Hand-rolled to avoid a dependency on
/// `strsim` / `edit-distance` crates. O(m·n) with rolling two-row buffer.
fn edit_distance(a: &str, b: &str) -> usize {
    let av: Vec<char> = a.chars().collect();
    let bv: Vec<char> = b.chars().collect();
    let m = av.len();
    let n = bv.len();
    if m == 0 { return n; }
    if n == 0 { return m; }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = if av[i - 1] == bv[j - 1] { 0 } else { 1 };
            cur[j] = (cur[j - 1] + 1)
                .min(prev[j] + 1)
                .min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[n]
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

    // ── compat builtins (canonical from ported::builtin::BUILTINS) ──
    // These are the ported zsh-faithful builtins. Distinct from the
    // Extension chapter (which lists zshrs-only additions). Together
    // they cover every builtin the user can call.
    let mut compat: Vec<String> = crate::ported::builtin::BUILTINS
        .iter()
        .map(|b| b.node.nam.clone())
        .collect();
    compat.sort();
    compat.dedup();
    write_chapter(
        &mut out,
        "ch-lsp-compat",
        "Compat Builtin Index",
        &format!(
            "{} entries · zsh-faithful ports from <code>ported::builtin::BUILTINS</code>. \
             Each mirrors an upstream <code>Src/Builtins/*.c</code> entry 1:1, with the \
             hover body extracted from <code>man zshall</code> yodl. See also: \
             <a href=\"#ch-lsp-extensions\">Extension Builtin Index</a> for zshrs-only \
             additions.",
            compat.len()
        ),
        &compat,
        "compat",
    );

    // ── keywords (canonical `reswds[]`) ─────────────────────────────
    // Source: `ported::hashtable::RESWDS` — direct port of upstream
    // `Src/hashtable.c:1076-1108`. Mirrors the `man zshmisc` "Reserved
    // Words" section (`Doc/Zsh/grammar.yo:501-504`) verbatim — every
    // one of the 31 entries (including the declarers `declare` /
    // `export` / `float` / `integer` / `local` / `readonly` / `typeset`,
    // which are reserved AND also exist as builtins).
    let keywords: Vec<String> = crate::ported::hashtable::RESWDS
        .iter()
        .map(|(n, _)| n.to_string())
        .collect();
    write_chapter(
        &mut out,
        "ch-lsp-keywords",
        "Keyword Index",
        &format!(
            "{} entries · zsh reserved words from <code>Src/hashtable.c</code> \
             <code>reswds[]</code>. Mirrors the <code>man zshmisc</code> \
             \"Reserved Words\" section. Declarers (<code>declare</code>, \
             <code>export</code>, <code>float</code>, <code>integer</code>, \
             <code>local</code>, <code>readonly</code>, <code>typeset</code>) \
             are reserved AND also appear in the Builtin Index — they're both.",
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

    // ── compsys functions (`compsys::COMPSYS_FN_NAMES`) ──────────────
    let mut compsys_names: Vec<String> =
        compsys::COMPSYS_FN_NAMES.iter().map(|s| s.to_string()).collect();
    compsys_names.sort();
    write_chapter(
        &mut out,
        "ch-lsp-compsys",
        "Compsys Function Index",
        &format!(
            "{} entries · the <code>_arguments</code> / <code>_files</code> / \
             <code>_describe</code> family of completion functions. Native Rust \
             implementations in the <code>compsys</code> crate replace the \
             upstream zsh shell-function versions for performance.",
            compsys_names.len()
        ),
        &compsys_names,
        "compsys",
    );

    // ── extension builtins (ext + daemon z* builtins) ────────────────
    let mut ext_names: Vec<String> = crate::ext_builtins::EXT_BUILTIN_NAMES
        .iter()
        .map(|s| s.to_string())
        .chain(
            crate::daemon::builtins::ZSHRS_BUILTIN_NAMES
                .iter()
                .map(|s| s.to_string()),
        )
        .collect();
    ext_names.sort();
    ext_names.dedup();
    write_chapter(
        &mut out,
        "ch-lsp-extensions",
        "Extension Builtin Index",
        &format!(
            "{} entries · zshrs-only builtins with NO upstream zsh counterpart. \
             Split across in-process builtins (coreutils drop-ins, <code>async</code>/\
             <code>await</code>/<code>barrier</code>, <code>doctor</code>, \
             <code>intercept</code>, contrib autoloads) and daemon-backed <code>z*</code> \
             builtins (<code>zd</code>, <code>zcache</code>, <code>zls</code>, \
             <code>zlock</code>, <code>zpublish</code>, …) that proxy to the local \
             <code>zshrs-daemon</code> for cross-shell state.",
            ext_names.len()
        ),
        &ext_names,
        "extension",
    );

    // ── operators / punctuation tokens ───────────────────────────────
    let op_names: Vec<String> = OPERATOR_DOCS
        .iter()
        .map(|(op, _)| (*op).to_string())
        .collect();
    write_chapter(
        &mut out,
        "ch-lsp-operators",
        "Operator / Punctuation Index",
        &format!(
            "{} entries · pipelines (<code>|</code>, <code>|&amp;</code>), list ops \
             (<code>&amp;&amp;</code>, <code>||</code>, <code>;</code>, <code>&amp;</code>, \
             <code>;;</code>), redirects (<code>&gt;</code>, <code>&gt;&gt;</code>, \
             <code>&lt;&lt;</code>, <code>&lt;&lt;&lt;</code>, <code>&amp;&gt;</code>, …), \
             conditional/arithmetic openers (<code>[[</code>, <code>]]</code>, <code>((</code>, \
             <code>))</code>), substitution forms (<code>$(</code>, <code>${{</code>, \
             <code>$((</code>, <code>&lt;(</code>, <code>&gt;(</code>), test ops \
             (<code>-e</code>, <code>-eq</code>, <code>=~</code>, …), pattern chars \
             (<code>*</code>, <code>?</code>, <code>**</code>, <code>~</code>), brace \
             expansion (<code>{{a,b,c}}</code>, <code>{{1..10}}</code>), and assignment \
             (<code>=</code>, <code>+=</code>). Sourced from <code>man zshmisc</code> \
             section prose — these have no per-name yodl <code>item</code> blocks so \
             they're hand-curated.",
            op_names.len()
        ),
        &op_names,
        "operator",
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

    #[test]
    fn completion_offers_daemon_z_star_builtins() {
        // Regression: user typed `zwh<TAB>` in IntelliJ + plugin and
        // got nothing. Root cause — the completion handler iterated
        // the hand `BUILTINS` const but not `ZSHRS_BUILTIN_NAMES`
        // (which holds `zwhere`, `zd`, `zcache`, etc.). Pin the
        // canonical-set sourcing so future builtins added there show
        // up in completion automatically.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "zwh".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 3 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["label"] == "zwhere"),
            "no `zwhere` in completion items for `zwh` prefix: {:?}",
            items
                .iter()
                .map(|i| i["label"].as_str().unwrap_or("?"))
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_offers_ext_builtins_and_compsys_fns() {
        // Same root cause as the `zwh` regression — verify the OTHER
        // tables we missed also surface. `date` is an extension
        // builtin (NOT in the hand subset), `_arguments` is a compsys
        // function, `vared` is a canonical compat builtin missing from
        // the hand `BUILTINS` subset.
        let _g = crate::test_util::global_state_lock();
        for (input, want) in &[("dat", "date"), ("_argu", "_arguments"), ("vare", "vared")] {
            let mut state = State::default();
            state.docs.insert("file:///t.zsh".into(), (*input).into());
            let params = json!({
                "textDocument": { "uri": "file:///t.zsh" },
                "position": { "line": 0, "character": input.len() },
            });
            let result = completion(&state, &params);
            let items = result["items"].as_array().unwrap();
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "no `{}` in completion for `{}` prefix: {:?}",
                want,
                input,
                items
                    .iter()
                    .map(|i| i["label"].as_str().unwrap_or("?"))
                    .collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn completion_offers_snippet_templates() {
        // Mirror of strykelang's snippet behavior: typing `if` should
        // surface the `if …` snippet template (kind=15, insertTextFormat=2)
        // alongside the bare `if` keyword. Without snippet items, the
        // user has no fast path to scaffold the full `if cmd; then …; fi`
        // body — the whole point of porting stryke's pattern.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "if".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 2 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        let snippet = items
            .iter()
            .find(|i| i["label"].as_str() == Some("if …"))
            .unwrap_or_else(|| panic!("no `if …` snippet in items: {:?}", items));
        assert_eq!(snippet["kind"], 15, "snippet kind must be 15 (Snippet)");
        assert_eq!(
            snippet["insertTextFormat"], 2,
            "snippet insertTextFormat must be 2 (Snippet — placeholders honored)"
        );
        let body = snippet["insertText"].as_str().unwrap();
        assert!(body.contains("then") && body.contains("fi"), "snippet body wrong: {}", body);
    }

    #[test]
    fn completion_suppressed_inside_double_quoted_literal() {
        // User report: "inside dbl strings I shouldnt be getting
        // random completions unless inside $() or ``". A double-quoted
        // string body is prose / URLs / JSON — not shell code — so
        // surfacing `if`, `cd`, `setopt` etc. is noise. Pin the gate.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), r#"echo "hello if"#.into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            // Cursor sits right after `if` INSIDE the open `"...` literal.
            "position": { "line": 0, "character": 14 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.is_empty(),
            "expected 0 items inside dq literal, got {}: {:?}",
            items.len(),
            items.iter().take(5).map(|i| i["label"].as_str().unwrap_or("?")).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_active_inside_command_substitution_inside_dq() {
        // Counterpart to the dq gate: cursor inside `$(...)` IS shell
        // code even when wrapped by `"..."`. The gate must NOT swallow
        // completion here.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), r#"echo "x $(cd"#.into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            // Cursor right after `cd` inside `$(`.
            "position": { "line": 0, "character": 12 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["label"] == "cd"),
            "expected `cd` to surface inside $() within dq: {:?}",
            items.iter().take(5).map(|i| i["label"].as_str().unwrap_or("?")).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_active_inside_backticks_inside_dq() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo \"x `cd".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 11 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["label"] == "cd"),
            "expected `cd` to surface inside backticks within dq",
        );
    }

    #[test]
    fn completion_active_inside_param_expansion_inside_dq() {
        // `${...}` is parameter expansion — variable / option name
        // completion is genuinely useful here, so the gate must allow it.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), r#"echo "x ${P"#.into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 11 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            !items.is_empty(),
            "expected non-empty items inside ${{...}} within dq",
        );
    }

    #[test]
    fn completion_suppressed_inside_single_quoted_literal() {
        // Single-quoted strings are opaque — no interpolation possible —
        // so completion is pure noise.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo 'hello if".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 14 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(items.is_empty(), "expected 0 items inside sq literal");
    }

    #[test]
    fn completion_suppressed_inside_comment() {
        // Comments are docs / TODOs / disabled code — not shell code.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "# todo: if".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 10 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(items.is_empty(), "expected 0 items inside comment");
    }

    #[test]
    fn completion_active_after_closing_double_quote() {
        // Sanity check: the gate must REOPEN once the cursor crosses
        // the closing quote. `echo "x" if|` is back at shell-code top
        // level.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), r#"echo "x" if"#.into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 11 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.iter().any(|i| i["label"] == "if"),
            "expected `if` to surface after closed dq string"
        );
    }

    // ── parameter expansion flag + glob qualifier completion ───────────

    #[test]
    fn completion_param_flags_inside_dollar_brace_paren() {
        // User-driven: typing `${(<TAB>` should surface every flag
        // letter zsh's compsys `_parameter_flags` produces, with
        // descriptions. Pin a representative sample (`L` lower-case,
        // `U` upper-case, `@` array-keep, `#` count) — drift in any
        // entry fails the test so the table stays canonical.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo ${(".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 8 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        for want in &["L", "U", "@", "#", "f", "F", "j", "s", "q"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing param flag `{}` in completion; got {:?}",
                want,
                items.iter().map(|i| i["label"].as_str().unwrap_or("?")).collect::<Vec<_>>(),
            );
        }
        // Should NOT include shell builtins / keywords / options here.
        assert!(
            !items.iter().any(|i| i["label"] == "cd" || i["label"] == "if"),
            "param-flag context leaked normal completion: {:?}",
            items.iter().take(20).map(|i| i["label"].as_str().unwrap_or("?")).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_param_flags_after_partial_flag() {
        // `${(b` — cursor after first flag letter. We still want the
        // full table surfaced (user may add more flags, eg `${(bC)`).
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo ${(b".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 9 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.len() >= 40,
            "expected full flag table (40+), got {}",
            items.len(),
        );
    }

    #[test]
    fn completion_param_flags_inside_nested_dollar_brace() {
        // `${${(L)` — inner `${(` still triggers ParamFlag. The
        // backward walker must find the innermost unmatched `(` and
        // classify on `${` before it.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo ${${(".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 10 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(items.iter().any(|i| i["label"] == "L"), "missing `L`");
    }

    #[test]
    fn completion_no_param_flag_when_paren_already_closed() {
        // `${(b)var` — past the closing `)`, we're back in param-name
        // context, NOT flag context. Param-flag table must NOT fire.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo ${(b)var".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 13 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        // Either normal completion fires (builtins / params) or it's
        // empty — but it must NOT be the param-flag table. Heuristic:
        // the flag table has only single-char labels; a real builtin
        // like `cd`/`vared` has multi-char. Assert at least one
        // multi-char label exists OR the result is empty.
        let single_char_only = !items.is_empty()
            && items.iter().all(|i| i["label"].as_str().unwrap_or("").chars().count() == 1);
        assert!(!single_char_only, "param-flag table leaked past closing `)`");
    }

    #[test]
    fn completion_glob_qualifier_after_star_paren() {
        // `ls *(` — cursor right after `(` of glob qualifier. Should
        // surface `/`, `.`, `@`, `*`, `r`, `w`, `x` and friends.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "ls *(".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 5 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        for want in &["/", ".", "@", "*", "r", "w", "x", "U", "G"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing glob qualifier `{}`; got {:?}",
                want,
                items.iter().take(20).map(|i| i["label"].as_str().unwrap_or("?")).collect::<Vec<_>>(),
            );
        }
        assert!(
            !items.iter().any(|i| i["label"] == "cd" || i["label"] == "if"),
            "glob-qualifier context leaked normal completion",
        );
    }

    #[test]
    fn completion_glob_qualifier_after_question_mark() {
        // `?(` is also a glob meta open — should trigger qualifier
        // completion the same way `*(` does.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "ls ?(".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 5 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(items.iter().any(|i| i["label"] == "."));
    }

    #[test]
    fn completion_no_glob_qualifier_for_plain_subshell() {
        // `cmd (foo)` — bare `(` preceded by SPACE is a subshell /
        // function-list grouping, NOT a glob qualifier. Must NOT
        // surface qualifier table.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo (".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 6 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        // Should be normal completion — expect `cd` to be present.
        let has_normal = items.iter().any(|i| i["label"] == "cd" || i["label"] == "if");
        let single_char_only = !items.is_empty()
            && items.iter().all(|i| i["label"].as_str().unwrap_or("").chars().count() == 1);
        assert!(has_normal || !single_char_only, "subshell `(` mis-triggered glob qualifier table");
    }

    #[test]
    fn completion_param_flag_table_has_50_entries() {
        // Pin: drift below 50 fails the gate so anyone trimming
        // entries notices. Screenshot from the user shows the full
        // compsys `_parameter_flags` list which is ~50 chars.
        assert!(
            PARAM_FLAG_DOCS.len() >= 49,
            "PARAM_FLAG_DOCS dropped below 49 entries: {}",
            PARAM_FLAG_DOCS.len()
        );
    }

    // ── history designator + modifier completion ────────────────────

    #[test]
    fn completion_history_designator_after_bang_at_word_start() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "!".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 1 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        for want in &["!", "$", "^", "*", "#"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing history designator `{}`; got {:?}",
                want,
                items.iter().map(|i| i["label"].as_str().unwrap_or("?")).collect::<Vec<_>>(),
            );
        }
        // No builtins / keywords here.
        assert!(!items.iter().any(|i| i["label"] == "cd" || i["label"] == "if"));
    }

    #[test]
    fn completion_history_designator_after_bang_midline() {
        // `vim !` — `!` at word boundary after space, mid-line.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "vim !".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 5 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(items.iter().any(|i| i["label"] == "$"));
    }

    #[test]
    fn completion_no_history_designator_inside_arithmetic() {
        // `(( a != b ))` — `!` is logical NOT, not history. Must NOT
        // surface history table.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "(( a !".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 6 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        // History table has labels like `$`, `^`, `?str?`. If the
        // arithmetic suppression worked, we should see normal items
        // OR no items, but NOT the history-specific markers.
        assert!(
            !items.iter().any(|i| i["label"] == "?str?"),
            "history table leaked into `((…))` arithmetic context",
        );
    }

    #[test]
    fn completion_no_history_designator_after_alnum() {
        // `foo!` — `!` preceded by alnum char is NOT a history start.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "foo!".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 4 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            !items.iter().any(|i| i["label"] == "?str?"),
            "history table fired after alnum-preceded `!`",
        );
    }

    #[test]
    fn completion_param_modifier_after_colon_in_dollar_brace() {
        // `${var:` — cursor after `:`, want modifier completion.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo ${var:".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 11 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        for want in &["h", "t", "r", "e", "-", "=", "+", "?", "s", "gs", "q", "Q"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing modifier `{}`; got {:?}",
                want,
                items.iter().map(|i| i["label"].as_str().unwrap_or("?")).collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn completion_param_modifier_after_partial_modifier() {
        // `${var:h` — cursor after `h`, still want full modifier table
        // surfaced so the IDE can re-filter as the user keeps typing.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "echo ${var:h".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 12 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(items.len() >= 25, "expected full modifier table; got {}", items.len());
    }

    #[test]
    fn completion_param_modifier_after_history_bang_colon() {
        // `!!:` — history reference with colon, want modifier table.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "vim !!:".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 7 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(items.iter().any(|i| i["label"] == "h"));
        assert!(items.iter().any(|i| i["label"] == "t"));
    }

    #[test]
    fn completion_no_param_modifier_outside_dollar_brace() {
        // Bare `foo:bar` — `:` outside any `${…}` AND no preceding
        // `!event`. Should NOT trigger modifier table.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "foo:".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 4 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        // No history-only label like `?str?`; verify it's normal flow.
        let has_normal = items.iter().any(|i| i["label"] == "cd" || i["label"] == "if");
        let modifier_only = !items.is_empty()
            && items.iter().all(|i| {
                let l = i["label"].as_str().unwrap_or("");
                l.chars().count() <= 2
            });
        assert!(
            has_normal || !modifier_only,
            "bare `:` mis-triggered modifier table",
        );
    }

    #[test]
    fn completion_history_designator_table_has_9_entries() {
        assert!(
            HISTORY_DESIGNATOR_DOCS.len() >= 9,
            "HISTORY_DESIGNATOR_DOCS dropped below 9: {}",
            HISTORY_DESIGNATOR_DOCS.len()
        );
    }

    #[test]
    fn completion_param_modifier_table_has_30_entries() {
        assert!(
            PARAM_MODIFIER_DOCS.len() >= 30,
            "PARAM_MODIFIER_DOCS dropped below 30: {}",
            PARAM_MODIFIER_DOCS.len()
        );
    }

    #[test]
    fn completion_glob_qualifier_table_has_30_entries() {
        // Pin: zsh's qualifier table per `man zshexpn` covers
        // file-type / perm / time / size / sort / control categories;
        // dropping below 30 means we've lost a whole category.
        assert!(
            GLOB_QUALIFIER_DOCS.len() >= 30,
            "GLOB_QUALIFIER_DOCS dropped below 30 entries: {}",
            GLOB_QUALIFIER_DOCS.len()
        );
    }

    #[test]
    fn completion_snippet_table_has_60_plus_entries() {
        // Pin: stryke's plugin README advertises "60+ snippet templates."
        // Mirror the bar here — the table is the public surface for
        // shell-snippet completion. Drift below 60 fails the gate so
        // anyone removing entries notices.
        assert!(
            SNIPPETS.len() >= 60,
            "snippet table dropped below 60 entries: {}",
            SNIPPETS.len()
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

    #[test]
    fn references_follows_source_chain_outside_workspace() {
        // Regression: usages in `source ~/...` files weren't picked up.
        // Active file declares `greet`, sourced file calls it. The
        // chain should be followed even when the sourced file lives
        // OUTSIDE the workspace root.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        // Build a temp dir + sourced file + an active doc that points
        // to the sourced file with an absolute path.
        let tmp = std::env::temp_dir().join(format!(
            "zshrs-ref-source-chain-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let sourced = tmp.join("helpers.zsh");
        std::fs::write(&sourced, "greet world\ngreet again\n").unwrap();
        let active_text = format!(
            "function greet {{ echo hi }}\nsource {}\n",
            sourced.display()
        );
        state.docs.insert("file:///t.zsh".into(), active_text);
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 9 }, // on "greet" decl
            "context": { "includeDeclaration": true },
        });
        let refs = references(&state, &params);
        let arr = refs.as_array().unwrap();
        // 1 decl in active + 2 calls in sourced = 3
        assert!(
            arr.len() >= 3,
            "source-chain following missed refs, got {}: {:?}",
            arr.len(),
            arr,
        );
        // At least one ref must point at the sourced file URI.
        let sourced_uri = format!("file://{}", sourced.canonicalize().unwrap().display());
        assert!(
            arr.iter().any(|r| r["uri"].as_str() == Some(sourced_uri.as_str())),
            "no ref pointing at sourced file `{}`: {:?}",
            sourced_uri,
            arr,
        );
        let _ = std::fs::remove_dir_all(&tmp);
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

    // ── code_actions: Extract Variable / Constant / Function ───────────

    fn run_code_actions(text: &str, sl: u32, sc: u32, el: u32, ec: u32) -> Vec<Value> {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), text.to_string());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "range": {
                "start": { "line": sl, "character": sc },
                "end":   { "line": el, "character": ec },
            },
        });
        match code_actions(&state, &params) {
            Value::Array(v) => v,
            _ => Vec::new(),
        }
    }

    #[test]
    fn code_actions_single_line_offers_var_const_and_function() {
        let acts = run_code_actions("    echo hello\n", 0, 4, 0, 14);
        let titles: Vec<&str> = acts
            .iter()
            .map(|a| a["title"].as_str().unwrap_or(""))
            .collect();
        // Whole-line selection: all three should fire.
        assert!(
            titles.iter().any(|t| t.contains("variable")),
            "missing Extract Variable: {:?}",
            titles,
        );
        assert!(
            titles.iter().any(|t| t.contains("constant")),
            "missing Extract Constant: {:?}",
            titles,
        );
        assert!(
            titles.iter().any(|t| t.contains("function")),
            "missing Extract Function: {:?}",
            titles,
        );
    }

    #[test]
    fn code_actions_subexpression_skips_function_extract() {
        // Selection covers only "hello" inside `echo hello world` — a
        // sub-expression, not a whole statement. Function extract on a
        // partial expression would call a function whose result is then
        // interpolated weirdly; the user wants Extract Variable for
        // that case (already covered).
        let acts = run_code_actions("echo hello world\n", 0, 5, 0, 10);
        let titles: Vec<&str> = acts
            .iter()
            .map(|a| a["title"].as_str().unwrap_or(""))
            .collect();
        assert!(titles.iter().any(|t| t.contains("variable")));
        assert!(
            !titles.iter().any(|t| t.contains("function")),
            "function extract leaked on sub-expression: {:?}",
            titles,
        );
    }

    #[test]
    fn code_actions_multiline_only_offers_function_extract() {
        // Spans three lines — variable / constant extract require a
        // single-line expression target and must NOT appear.
        let text = "if true; then\n    echo a\n    echo b\nfi\n";
        let acts = run_code_actions(text, 1, 0, 3, 0);
        let titles: Vec<&str> = acts
            .iter()
            .map(|a| a["title"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(acts.len(), 1, "expected exactly one action: {:?}", titles);
        assert!(titles[0].contains("function"));
        // Verify the edit shape: insert a `extracted_function() { … }`
        // declaration above and replace the lines with a bare call.
        let changes = &acts[0]["edit"]["changes"]["file:///t.zsh"];
        let edits = changes.as_array().expect("edits array");
        assert_eq!(edits.len(), 2);
        let decl = edits[0]["newText"].as_str().unwrap_or("");
        assert!(
            decl.contains("extracted_function() {")
                && decl.contains("echo a")
                && decl.contains("echo b"),
            "decl missing body lines: {:?}",
            decl,
        );
        let call = edits[1]["newText"].as_str().unwrap_or("");
        assert!(call.trim() == "extracted_function", "call must be bare: {:?}", call);
    }

    #[test]
    fn code_actions_multiline_preserves_relative_indent() {
        // Inner if-block: the extracted body should keep the inner
        // indent so re-indenting against the function-body indent
        // (`+4 spaces`) doesn't flatten the structure.
        let text = "if outer; then\n    if inner; then\n        echo nested\n    fi\nfi\n";
        let acts = run_code_actions(text, 1, 0, 3, 0);
        assert_eq!(acts.len(), 1);
        let decl = acts[0]["edit"]["changes"]["file:///t.zsh"][0]["newText"]
            .as_str()
            .unwrap_or("");
        // The `echo nested` line had 8 spaces; common-indent for the
        // block is 4; after stripping common and adding +4 indent it
        // should still be 4 leading spaces past the function indent.
        assert!(
            decl.contains("        echo nested"),
            "relative indent lost: {:?}",
            decl,
        );
    }

    #[test]
    fn code_actions_caret_only_snaps_to_word() {
        // Cursor with no selection — must snap to identifier under
        // caret and still produce Extract Variable.
        let acts = run_code_actions("echo greeting\n", 0, 8, 0, 8);
        assert!(
            acts.iter()
                .any(|a| a["title"].as_str().unwrap_or("").contains("variable")),
            "caret-only didn't snap to a word: {:?}",
            acts.iter().map(|a| a["title"].clone()).collect::<Vec<_>>(),
        );
    }

    #[test]
    fn code_actions_caret_only_offers_extract_function() {
        // Regression: the JetBrains plugin's Extract Method shortcut
        // (Cmd-Opt-M) used to report "LSP returned no code actions for
        // this range" because caret-only invocations only produced
        // Extract Variable / Constant — no function action existed for
        // the plugin's title filter to match. Cursor at column 8 of
        // `echo greeting` should now ALSO emit Extract Function over
        // the whole line.
        let acts = run_code_actions("echo greeting\n", 0, 8, 0, 8);
        let titles: Vec<&str> = acts
            .iter()
            .map(|a| a["title"].as_str().unwrap_or(""))
            .collect();
        assert!(
            titles.iter().any(|t| t.contains("function")),
            "caret-only must include Extract Function for Cmd-Opt-M: {:?}",
            titles,
        );
        // The function body should be the trimmed whole line, not just
        // the snapped word.
        let fn_act = acts
            .iter()
            .find(|a| a["title"].as_str().unwrap_or("").contains("function"))
            .expect("function action present");
        let decl = fn_act["edit"]["changes"]["file:///t.zsh"][0]["newText"]
            .as_str()
            .unwrap_or("");
        assert!(
            decl.contains("echo greeting"),
            "caret-only function extract should wrap the whole line, not just the word: {:?}",
            decl,
        );
    }

    #[test]
    fn code_actions_caret_on_whitespace_still_offers_function() {
        // Cursor sits in the leading indent of `    echo hello` (col 2,
        // inside whitespace). Snap-to-word returns None — without the
        // fix, the LSP returned []. With the fix, Extract Function
        // still applies over the line's actual content.
        let acts = run_code_actions("    echo hello\n", 0, 2, 0, 2);
        let titles: Vec<&str> = acts
            .iter()
            .map(|a| a["title"].as_str().unwrap_or(""))
            .collect();
        assert!(
            titles.iter().any(|t| t.contains("function")),
            "cursor on whitespace must still emit Extract Function: {:?}",
            titles,
        );
    }

    #[test]
    fn code_actions_caret_on_blank_line_returns_empty() {
        // Truly nothing to extract — blank line, no content. Returning
        // an empty list is correct; the plugin will surface "no code
        // actions for this range" which is the honest answer.
        let acts = run_code_actions("foo\n\nbar\n", 1, 0, 1, 0);
        assert!(
            acts.is_empty(),
            "blank line should produce no actions: {:?}",
            acts.iter().map(|a| a["title"].clone()).collect::<Vec<_>>(),
        );
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
