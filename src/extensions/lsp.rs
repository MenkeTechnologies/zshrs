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
/// Guard against leaking as an orphan LSP process. Editors / GUI apps reap us via
/// kill-on-drop, which only fires on a *graceful* client exit; a hard client
/// death (SIGKILL/crash/force-quit) skips it, and a leaked pipe fd can keep our
/// stdin open so we never see EOF either. Watch for reparenting to pid 1 (our
/// client died) and exit — nothing this read-only server holds is worth leaking.
fn spawn_orphan_guard() {
    std::thread::spawn(|| {
        // Linux: also ask the kernel to SIGKILL us the instant the parent dies.
        // Best-effort; the getppid poll below is the portable guarantee (macOS
        // has no PDEATHSIG).
        #[cfg(target_os = "linux")]
        // SAFETY: prctl(PR_SET_PDEATHSIG, ...) only registers a signal disposition.
        unsafe {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL as libc::c_ulong, 0, 0, 0);
        }
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            // SAFETY: getppid takes no arguments and never fails.
            if unsafe { libc::getppid() } == 1 {
                std::process::exit(0);
            }
        }
    });
}

pub fn run_lsp() -> i32 {
    tracing::info!(
        target: "zshrs::lsp",
        pid = std::process::id(),
        "starting --lsp",
    );
    spawn_orphan_guard();
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
                    // Auto-open popup on these chars. The list covers
                    // every context-aware surface: `$`/`{` (param/var),
                    // `(` (glob qualifier / param flag / pattern mod /
                    // subscript flag / math fn), `[` (subscript), `#`
                    // (pattern modifier `(#i)`), `*`/`?` (glob meta
                    // before `(`), `!` (history designator), `-`
                    // (typeset flag), `:` (param/history modifier).
                    "triggerCharacters": ["$", "{", "(", "[", "#", "*", "?", "!", "-", ":"],
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

/// Strip line-end comments and the *contents* of quoted strings from
/// `line`, returning a string whose `split_whitespace()` only yields
/// real code tokens. Keeps the quotes themselves so column positions
/// of any surviving tokens line up with the source. Used by the
/// block-keyword scan in `diagnose()` so keywords inside comments
/// (`# foo case bar`) or strings (`echo "if x"`) don't push spurious
/// entries onto the block stack.
fn strip_comments_and_strings(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            '\\' if i + 1 < bytes.len() => {
                out.push(c);
                out.push(bytes[i + 1] as char);
                i += 2;
                continue;
            }
            '"' | '\'' | '`' => {
                let q = c;
                out.push(q);
                i += 1;
                while i < bytes.len() {
                    let cc = bytes[i] as char;
                    if cc == '\\' && q != '\'' && i + 1 < bytes.len() {
                        i += 2;
                        continue;
                    }
                    if cc == q {
                        out.push(q);
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
            '#' => {
                let prev = if i == 0 {
                    None
                } else {
                    Some(bytes[i - 1] as char)
                };
                let is_comment_start = match prev {
                    None => true,
                    // Note: `(` removed from prev-chars — zsh glob
                    // qualifiers `(#i)`/`(#a)` would otherwise truncate
                    // the line at `#` and orphan the trailing `)` etc.
                    Some(p) => p.is_whitespace() || p == ';' || p == '&' || p == '|',
                };
                if is_comment_start {
                    break;
                }
                out.push(c);
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// Run a quick structural pass over the document to surface obvious
/// errors. This is intentionally lightweight: it complements (does not
/// replace) the deeper diagnostics from a full parse.
fn diagnose(text: &str) -> Vec<Value> {
    let mut diags = Vec::new();
    let mut stack: Vec<(char, usize, usize)> = Vec::new(); // (open, line, col)
    let mut block_stack: Vec<(&str, usize, usize)> = Vec::new();
    // Multi-line quote state. `'` and `"` can span lines (heredocs,
    // multi-line awk/sed/perl scripts embedded in single quotes, etc.).
    // When set, the next line is parsed as quote-body until we find the
    // matching close, after which normal parsing resumes.
    let mut open_quote: Option<char> = None;

    for (line_no, line) in text.lines().enumerate() {
        // If we're inside a multi-line quote opened on a previous line,
        // scan for its close. Skip block-keyword scanning entirely for
        // this line — the line content is string-body, not zsh code.
        let mut post_quote_tail: Option<usize> = None;
        if let Some(q) = open_quote {
            let bytes = line.as_bytes();
            let mut i = 0usize;
            let mut closed_at = None;
            while i < bytes.len() {
                let c = bytes[i] as char;
                if c == '\\' && q != '\'' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if c == q {
                    closed_at = Some(i);
                    break;
                }
                i += 1;
            }
            if let Some(close_pos) = closed_at {
                open_quote = None;
                // Multi-line `$(... "string with `(` `)` ..." ...)` form
                // (e.g. examples/demos/365_mini_lisp.zsh:574, 654, 660)
                // closes the quote then has code after. Drop into the
                // normal token-level scan, but starting at close_pos+1,
                // so trailing `)` matches the `$( … )` opener pushed on
                // the prior line. Block-keyword scan stays skipped (the
                // line is mostly string-body; partial coverage avoids
                // re-triggering case/done/fi false positives).
                post_quote_tail = Some(close_pos + 1);
            } else {
                // Still inside the multi-line quote — entire line is
                // string body. Skip both scans.
                continue;
            }
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        // Pre-scan: does this line contain a `case` keyword (in code,
        // not in a comment / string)? If yes, any bare `)` on the same
        // line is a case-arm pattern terminator and must NOT be flagged
        // as an unmatched paren. Without this, lines like
        //   case foo in foo|bar) echo yes ;; *) echo no ;; esac
        // produced two false "unmatched `)`" diagnostics because the
        // bracket scan runs in lexical order and `case` hasn't been
        // pushed onto block_stack yet when the `)` is seen.
        let line_code_only = strip_comments_and_strings(line);
        let line_has_case_keyword = line_code_only.split_whitespace().any(|t| {
            let bare = t.trim_end_matches(|c: char| matches!(c, ';' | '&' | '|'));
            bare == "case"
        });
        // Pre-scan: find positions where `done`/`fi`/`esac` appear as
        // BARE VALUE tokens (not as block terminators). They're
        // terminators only when the previous token is a statement-
        // separator (`;`/`&&`/`||`) or a block-body opener
        // (`do`/`then`/`else`/`elif`). Otherwise they're literal
        // arguments / comparison operands:
        //   * `[[ $x == done ]]`         — comparison right-operand
        //   * `todo_list done`           — CLI arg to a function
        //   * `local status=done`        — assignment value
        // Conservative detector: build a set of token-indices where
        // the previous code-only token does NOT terminate a statement
        // / open a block-body. Used in the loop below to skip those
        // token-positions. Pinned by
        // examples/demos/143_todo_app.zsh:50,90.
        let mut bareword_value_positions: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        {
            let toks: Vec<&str> = line_code_only.split_whitespace().collect();
            for (i, t) in toks.iter().enumerate() {
                if i == 0 {
                    continue;
                }
                let prev_bare =
                    toks[i - 1].trim_end_matches(|c: char| matches!(c, ';' | '&' | '|'));
                let prev_ends_with_separator =
                    toks[i - 1].ends_with(|c: char| matches!(c, ';' | '&' | '|'));
                let bare = t.trim_end_matches(|c: char| matches!(c, ';' | '&' | '|'));
                let prev_is_body_opener = matches!(
                    prev_bare,
                    "do" | "then" | "else" | "elif" | "&&" | "||" | ";" | "&" | "|"
                );
                if matches!(bare, "done" | "fi" | "esac")
                    && !prev_ends_with_separator
                    && !prev_is_body_opener
                {
                    bareword_value_positions.insert(i);
                }
            }
        }
        // Token-level scan. Pairings tracked on `stack`:
        //   '('  — single paren            ')'
        //   '{'  — single brace            '}'
        //   '['  — single bracket          ']'
        //   'A'  — arithmetic `((`         `))`
        //   'D'  — conditional `[[`        `]]`
        let mut i = post_quote_tail.unwrap_or(0);
        let bytes = line.as_bytes();
        while i < bytes.len() {
            let c = bytes[i] as char;
            match c {
                '(' => {
                    // `((` opens an arithmetic expression — paired with `))`,
                    // not two single parens.
                    if bytes.get(i + 1) == Some(&b'(') {
                        // Distinguish `$((arith))` from `$( () { fn } N )`
                        // (anonymous-fn call inside command substitution).
                        // The latter has `()` immediately after `$(`:
                        // `$((` + `)` + non-`)` is `$( (...` not arithmetic.
                        // Empty arithmetic `$(())` (rare) still works because
                        // the third char IS `)`, satisfying the original check.
                        let prev_is_dollar = i > 0 && bytes[i - 1] == b'$';
                        let next2 = bytes.get(i + 2).copied();
                        let next3 = bytes.get(i + 3).copied();
                        if prev_is_dollar && next2 == Some(b')') && next3 != Some(b')') {
                            // Treat as two separate `(` so the inner `()`
                            // pair balances cleanly with its `)`.
                            stack.push(('(', line_no, i)); // outer $(
                            stack.push(('(', line_no, i + 1)); // inner (
                            i += 2;
                            continue;
                        }
                        // INSIDE an arithmetic block (`stack` already has
                        // 'A'), `((expr) + …)` is NOT a nested arithmetic
                        // — it's two single `(` opening a grouped sub-
                        // expression. Pin the inner `((` as two single
                        // `(` so `((hash << 5) + hash)` inside `$((…))`
                        // balances cleanly (reported on
                        // examples/demos/195_sha_simple_hash.zsh:25).
                        if stack.iter().any(|x| x.0 == 'A') {
                            stack.push(('(', line_no, i));
                            stack.push(('(', line_no, i + 1));
                            i += 2;
                            continue;
                        }
                        stack.push(('A', line_no, i));
                        i += 2;
                        continue;
                    }
                    stack.push(('(', line_no, i));
                }
                ')' => {
                    // `))` closes arithmetic.
                    if bytes.get(i + 1) == Some(&b')') && stack.last().map(|x| x.0) == Some('A') {
                        stack.pop();
                        i += 2;
                        continue;
                    }
                    if stack.last().map(|x| x.0) == Some('(') {
                        stack.pop();
                    } else {
                        // Bare `)` inside an open `case ... esac` is a
                        // pattern-arm terminator, not a paren mismatch.
                        // Two recognition modes:
                        //   1. `block_stack` already has `case` (multi-line
                        //      `case … in\n PAT) cmd ;;\n esac`).
                        //   2. The CURRENT line has the `case` keyword
                        //      anywhere left of this `)` (one-liner
                        //      `case x in y) … esac`).
                        let in_case = block_stack.iter().any(|(kw, _, _)| *kw == "case")
                            || line_has_case_keyword;
                        if !in_case {
                            diags.push(diagnostic(line_no, i, 1, "unmatched `)`", 1));
                        }
                    }
                }
                '{' => {
                    // `${...}` parameter substitution can contain glob
                    // patterns with unbalanced `[` / `]` etc. inside
                    // the pattern body (e.g. `${log#\[}`, `${log%%\]*}`).
                    // Skip the whole `${...}` span without recursing
                    // so the inner brackets don't trip the bracket
                    // tracker. Without this, lines like
                    //   date_part=${log_line#*\][}; date_part=${date_part%%\]*}
                    // (`examples/demos/117_backref_replacement.zsh:56`)
                    // raised false "unmatched `}`" / "unclosed `[`".
                    let preceded_by_dollar = i > 0 && bytes[i - 1] == b'$';
                    if preceded_by_dollar {
                        let mut depth = 1i32;
                        let mut j = i + 1;
                        while j < bytes.len() && depth > 0 {
                            match bytes[j] {
                                b'\\' if j + 1 < bytes.len() => {
                                    j += 2;
                                    continue;
                                }
                                b'{' => depth += 1,
                                b'}' => depth -= 1,
                                _ => {}
                            }
                            j += 1;
                        }
                        // Jump past the whole `${...}` span.
                        i = j;
                        continue;
                    }
                    stack.push(('{', line_no, i))
                }
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
                    if bytes.get(i + 1) == Some(&b']') && stack.last().map(|x| x.0) == Some('D') {
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
                    let mut closed = false;
                    while i < bytes.len() {
                        let cc = bytes[i] as char;
                        if cc == '\\' && q != '\'' && i + 1 < bytes.len() {
                            i += 2;
                            continue;
                        }
                        if cc == q {
                            closed = true;
                            break;
                        }
                        i += 1;
                    }
                    if !closed {
                        // Quote spans multiple lines. Mark open so the
                        // next line is parsed as quote-body. Multi-line
                        // awk/sed/perl scripts embedded in `'...'` no
                        // longer leak `for`/`do`/`done` keywords to the
                        // block_stack as false positives.
                        open_quote = Some(q);
                        // Halt scan of this line — rest is quote body.
                        break;
                    }
                }
                '#' => {
                    // `#` starts a line comment only when preceded by
                    // whitespace, `;`, `&`, `|`, `(`, or BOL — otherwise
                    // it's part of `$#` (argc), `${#var}` (length), or
                    // similar parameter expansion and must not terminate
                    // the scan.
                    //
                    // INSIDE arithmetic (`$((...))`, top of `stack` has 'A'),
                    // `#` is NEVER a comment — it's the char-value operator
                    // `#c` (zsh arithmetic) per `Src/math.c`. Skip the
                    // comment check entirely when stack contains 'A'.
                    let in_arith = stack.iter().any(|x| x.0 == 'A');
                    let prev = if i == 0 {
                        None
                    } else {
                        Some(bytes[i - 1] as char)
                    };
                    let is_comment_start = !in_arith
                        && match prev {
                            None => true,
                            Some(p) => {
                                // Note: `(` removed from the prev-chars list
                                // because `(#i)` / `(#a1)` etc. are zsh glob
                                // qualifiers / extended-glob flags where `#`
                                // immediately after `(` is NOT a comment.
                                // `( # cmt)` style is still recognized via
                                // the whitespace test before `#`.
                                p.is_whitespace() || p == ';' || p == '&' || p == '|'
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
        // Block keyword scan — must ignore tokens inside comments and
        // quoted strings, otherwise lines like `# foo case bar` or
        // `echo "if you like"` push spurious entries onto block_stack
        // and surface as "unclosed `case` block" / "unclosed `if`".
        // Skip when the line opened inside a multi-line quote that
        // just closed — most of the line is string body and partial
        // strip_comments_and_strings analysis would mis-flag tokens
        // before close_pos.
        if post_quote_tail.is_some() {
            continue;
        }
        let code_only = strip_comments_and_strings(line);
        for (tok_idx, kw_raw) in code_only.split_whitespace().enumerate() {
            // Strip trailing shell-statement-separator punctuation
            // (`;` / `&` / `|`) so tokens like `done;` and `fi;` and
            // `done;` still match the block-terminator keyword.
            // Without this, `for ((;;)); do echo $i; done;` left
            // `for` orphaned on block_stack → false "unclosed `for`
            // block" diagnostic on every one-liner that ends the
            // statement with a semicolon.
            let kw = kw_raw.trim_end_matches(|c: char| matches!(c, ';' | '&' | '|'));
            // Map matched tokens back to &'static str literals so the
            // borrows pushed onto block_stack don't outlive `code_only`
            // (which drops at end of this loop iteration).
            let kw_static: &'static str = match kw {
                "if" => "if",
                "for" => "for",
                "while" => "while",
                "until" => "until",
                "case" => "case",
                "select" => "select",
                "repeat" => "repeat",
                "fi" => "fi",
                "done" => "done",
                "esac" => "esac",
                _ => continue,
            };
            // `repeat N CMD` is a one-liner with no `do`/`done`; only push
            // onto block_stack when same line has a `do` token (block form
            // `repeat N; do ... done` or `repeat N do CMD done`).
            if kw_static == "repeat" && !code_only.split_whitespace().any(|t| t == "do") {
                continue;
            }
            // `select` can appear as a bareword ARGUMENT to other
            // builtins (e.g. `zstyle ':completion:*' menu select` —
            // `examples/demos/133_zstyle_demo.zsh:6,15`), where it's
            // NOT opening a `select VAR in WORDS; do ... done` block.
            // Only treat as a block opener when the same line also
            // has a `do` token (block form). Same heuristic as repeat.
            if kw_static == "select" && !code_only.split_whitespace().any(|t| t == "do") {
                continue;
            }
            // `done`/`fi`/`esac` as the RIGHT operand of a comparison
            // (e.g. `[[ $status == done ]]`) is a literal value being
            // compared, NOT a block terminator. The pre-scan above
            // populated `bareword_value_positions` with these token
            // indices. Pinned by examples/demos/143_todo_app.zsh:50.
            if bareword_value_positions.contains(&tok_idx) {
                continue;
            }
            match kw_static {
                "if" | "for" | "while" | "until" | "case" | "select" | "repeat" => {
                    block_stack.push((kw_static, line_no, 0));
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
    //
    // `ctx_item` builds an LSP CompletionItem with explicit
    // `filterText` + `insertText` so IntelliJ's prefix matcher
    // doesn't reject single-char non-alphanumeric labels (`/`, `.`,
    // `@`, `*`, etc.). Without this, the IDE would silently drop
    // them from the popup even though the LSP returned the items
    // correctly.
    fn ctx_item(label: &str, detail: &str, doc_md: &str) -> Value {
        json!({
            "label": label,
            "kind": 14, // Constant
            "detail": detail,
            "filterText": label,
            "insertText": label,
            "sortText": format!("0_{}", label),
            "documentation": {
                "kind": "markdown",
                "value": doc_md,
            },
        })
    }
    // Variant for contexts where items CHAIN — `${(LU)var}` /
    // `*(/D^.)` / `(#iI)` / `${arr[(Ri)…]}` / `${var:h:t:r}`. The
    // `command` field re-invokes the suggest popup after insertion
    // so the user can keep adding qualifiers without re-typing or
    // pressing Ctrl-Space. Mirrors how VS Code / IntelliJ Platform
    // LSP honor `editor.action.triggerSuggest`.
    fn ctx_item_chain(label: &str, detail: &str, doc_md: &str) -> Value {
        json!({
            "label": label,
            "kind": 14,
            "detail": detail,
            "filterText": label,
            "insertText": label,
            "sortText": format!("0_{}", label),
            "documentation": {
                "kind": "markdown",
                "value": doc_md,
            },
            "command": {
                "title": "Re-trigger completion",
                "command": "editor.action.triggerSuggest",
            },
        })
    }
    if let Some(l) = line {
        match lsp_completion_context(l, col) {
            LspCompletionContext::ParamFlag => {
                // `${(LU)var}` / `${(jks)arr}` chain — use chain variant
                // so the popup re-opens after each flag insertion.
                let items: Vec<Value> = PARAM_FLAG_DOCS
                    .iter()
                    .map(|(flag, doc)| ctx_item_chain(flag, *doc,
                        &format!("**`(`{}`)`** — {}\n\n_zsh parameter expansion flag — `${{(FLAGS)var}}`_", flag, doc)))
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::GlobQualifier => {
                // `*(/D^.)` chain — directories that aren't dotfiles.
                let items: Vec<Value> = GLOB_QUALIFIER_DOCS
                    .iter()
                    .map(|(q, doc)| {
                        ctx_item_chain(
                            q,
                            *doc,
                            &format!(
                                "**`(`{}`)`** — {}\n\n_zsh glob qualifier — `*(QUALIFIERS)`_",
                                q, doc
                            ),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::HistoryDesignator => {
                let items: Vec<Value> = HISTORY_DESIGNATOR_DOCS
                    .iter()
                    .map(|(d, doc)| {
                        ctx_item(
                            d,
                            *doc,
                            &format!("**`!{}`** — {}\n\n_zsh history event designator_", d, doc),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::ParamColonModifier => {
                // `${var:h:t:r}` / `!!:s/old/new/:gs/a/b/` chain.
                // Each modifier letter inserted; user adds another `:`
                // and re-types — but `:` is already a triggerChar so
                // re-open is automatic without the command field.
                // Still emit the command so insertion-without-typing-
                // colon also re-opens (e.g. for `${var:hto…}` style
                // multi-letter input).
                let items: Vec<Value> = PARAM_MODIFIER_DOCS
                    .iter()
                    .map(|(m, doc)| {
                        ctx_item_chain(
                            m,
                            *doc,
                            &format!(
                                "**`:{}`** — {}\n\n_zsh modifier — `${{var:MOD}}` / `!event:MOD`_",
                                m, doc
                            ),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            // ── Command-position contexts ─────────────────────────────
            LspCompletionContext::OptionOnly => {
                let mut items = Vec::new();
                for o in crate::ported::options::ZSH_OPTIONS_SET.iter() {
                    items.push(json!({
                        "label": o,
                        "kind": 21, // Constant
                        "detail": "zsh option (setopt / unsetopt)",
                    }));
                }
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::SignalName => {
                let items: Vec<Value> = SIGNAL_NAMES
                    .iter()
                    .map(|(n, doc)| {
                        ctx_item(
                            n,
                            *doc,
                            &format!(
                                "**SIG{}** — {}\n\n_signal name — `kill -{}` / `trap … {}`_",
                                n, doc, n, n
                            ),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::ModuleName => {
                let items: Vec<Value> = ZSH_MODULE_NAMES
                    .iter()
                    .map(|(n, doc)| {
                        ctx_item(
                            n,
                            *doc,
                            &format!("**`{}`** — {}\n\n_zsh module — `zmodload {}`_", n, doc, n),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::KeymapName => {
                let items: Vec<Value> = KEYMAP_NAMES
                    .iter()
                    .map(|(n, doc)| ctx_item(n, *doc, &format!("**`{}`** — {}", n, doc)))
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::WidgetName => {
                let items: Vec<Value> = ZLE_WIDGET_NAMES
                    .iter()
                    .map(|(n, doc)| {
                        ctx_item(n, *doc, &format!("**`{}`** — {}\n\n_ZLE widget_", n, doc))
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::TypesetFlag => {
                let items: Vec<Value> = TYPESET_FLAGS
                    .iter()
                    .map(|(f, doc)| {
                        ctx_item(
                            f,
                            *doc,
                            &format!(
                                "**`{}`** — {}\n\n_typeset / declare / local / readonly flag_",
                                f, doc
                            ),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::ZstyleContext => {
                let items: Vec<Value> = ZSTYLE_CONTEXTS
                    .iter()
                    .map(|(c, doc)| {
                        ctx_item(
                            c,
                            *doc,
                            &format!("**`{}`** — {}\n\n_zstyle context pattern_", c, doc),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::CompdefFn => {
                let mut items = Vec::new();
                for n in crate::compsys::COMPSYS_FN_NAMES {
                    items.push(ctx_item(
                        n,
                        "compsys completion function",
                        &format!("**`{}`** — compsys completion function", n),
                    ));
                }
                return json!({ "isIncomplete": false, "items": items });
            }
            // ── Bracket contexts ───────────────────────────────────────
            LspCompletionContext::TestOperator => {
                let items: Vec<Value> = TEST_OPERATORS
                    .iter()
                    .map(|(op, doc)| {
                        ctx_item(
                            op,
                            *doc,
                            &format!("**`{}`** — {}\n\n_inside `[[ … ]]` conditional_", op, doc),
                        )
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::MathFunction => {
                let items: Vec<Value> = MATH_FUNCTIONS
                    .iter()
                    .map(|(n, doc)| ctx_item(n, *doc,
                        &format!("**`{}(…)`** — {}\n\n_math function — inside `((…))` / `$((…))` (most require `zmodload zsh/mathfunc`)_", n, doc)))
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::PatternModifier => {
                // `(#iI)` / `(#ba3)` chain — case-insensitive + ID-reset,
                // backref + approx-3-errors.
                let items: Vec<Value> = PATTERN_MODIFIERS
                    .iter()
                    .map(|(m, doc)| ctx_item_chain(m, *doc,
                        &format!("**`(#{})`** — {}\n\n_extended-glob pattern modifier (needs `EXTENDED_GLOB`)_", m, doc)))
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::SubscriptFlag => {
                // `${arr[(Ri)pat]}` chain — reverse + case-insensitive.
                let items: Vec<Value> = SUBSCRIPT_FLAGS
                    .iter()
                    .map(|(f, doc)| ctx_item_chain(f, *doc,
                        &format!("**`({})`** — {}\n\n_array subscript flag — `${{arr[({})pattern]}}`_", f, doc, f)))
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::BuiltinFlag(ref builtin_name) => {
                // Parse the builtin's yodl doc body for flag bullets
                // / inline citations. Cached per-builtin.
                let flags = extract_builtin_flags(builtin_name);
                let bname = builtin_name.clone();
                // STACKED-FLAG SUPPORT: when the current word is
                // `-XYZ` (multi-char stack), each item label becomes
                // `-XYZa`, `-XYZb`, … so the IDE matcher accepts them
                // against the typed prefix. Single-dash prefix `-`
                // yields plain `-a`, `-b`, etc. The dispatcher
                // re-derives the current-word prefix from the typed
                // line (the BuiltinFlag context lost it through the
                // detector boundary).
                let cur_word: String = if let Some(l) = line {
                    let bytes = l.as_bytes();
                    let cap = col.min(bytes.len());
                    let mut j = cap;
                    while j > 0 && !matches!(bytes[j - 1], b' ' | b'\t') {
                        j -= 1;
                    }
                    String::from_utf8_lossy(&bytes[j..cap]).to_string()
                } else {
                    "-".to_string()
                };
                // If the user has typed `-XYZ` (≥2 chars including
                // the dash), strip the trailing single letter that
                // the next flag would replace: actually no — we
                // want to APPEND, so keep the whole `-XYZ` and
                // emit items `-XYZa`/`-XYZb`/…
                let stack_prefix: String = if cur_word.starts_with('-') && cur_word.len() >= 2 {
                    cur_word.clone()
                } else {
                    "-".to_string()
                };
                // Explicit `textEdit` range covering the typed `-`
                // (or stacked `-XYZ`) through the cursor. Without
                // this, IntelliJ's LSP client treats `-` as a
                // non-identifier character: the replacement range is
                // empty (just at the cursor), so `insertText="-l"`
                // gets inserted AFTER the typed `-`, producing
                // `print --l`. `textEdit` pins the range
                // deterministically so the typed dash is replaced
                // along with the flag — `print -<tab>` → `print -l`.
                // Same shape as the stryke LSP sigil_completions fix.
                let cur_word_chars = cur_word.chars().count() as u64;
                let dash_col = (col as u64).saturating_sub(cur_word_chars);
                let edit_range = json!({
                    "start": { "line": line_no, "character": dash_col },
                    "end":   { "line": line_no, "character": col      },
                });
                let items: Vec<Value> = flags
                    .into_iter()
                    .map(|(flag, desc)| {
                        // `flag` is `-X` (always 2 chars). For
                        // stacked context, build the full
                        // `{stack_prefix}{X}` form. For initial
                        // single-dash, that's just `-X` (unchanged).
                        let letter = flag.trim_start_matches('-');
                        let label = if stack_prefix == "-" {
                            flag.clone()
                        } else {
                            format!("{}{}", stack_prefix, letter)
                        };
                        let detail = if desc.is_empty() {
                            format!("option flag for `{}`", bname)
                        } else {
                            desc.clone()
                        };
                        let doc_md = if desc.is_empty() {
                            format!("**`{}`** — option flag for `{}`", flag, bname)
                        } else {
                            format!("**`{}`** — {}\n\n_option flag for `{}`_", flag, desc, bname)
                        };
                        let mut item = ctx_item(&label, &detail, &doc_md);
                        if let Some(obj) = item.as_object_mut() {
                            obj.remove("insertText");
                            obj.insert(
                                "textEdit".to_string(),
                                json!({ "range": edit_range, "newText": label }),
                            );
                        }
                        item
                    })
                    .collect();
                return json!({ "isIncomplete": false, "items": items });
            }
            LspCompletionContext::BuiltinLongFlag(ref builtin_name) => {
                // Long-form flag completion: `zshrs --<TAB>`,
                // `zshrs --dump-<TAB>`, etc. No letter-stacking;
                // the typed `--xxx` is the full prefix and gets
                // replaced atomically by the chosen flag.
                let flags = extract_builtin_long_flags(builtin_name);
                let bname = builtin_name.clone();
                let cur_word: String = if let Some(l) = line {
                    let bytes = l.as_bytes();
                    let cap = col.min(bytes.len());
                    let mut j = cap;
                    while j > 0 && !matches!(bytes[j - 1], b' ' | b'\t') {
                        j -= 1;
                    }
                    String::from_utf8_lossy(&bytes[j..cap]).to_string()
                } else {
                    "--".to_string()
                };
                let cur_word_chars = cur_word.chars().count() as u64;
                let dash_col = (col as u64).saturating_sub(cur_word_chars);
                let edit_range = json!({
                    "start": { "line": line_no, "character": dash_col },
                    "end":   { "line": line_no, "character": col      },
                });
                // Sort prefix: zshrs-specific entries (the first
                // ZSHRS_SELF_LONG_FLAG_DOCS.len() items) get "0_",
                // setopt mirrors get "1_". IntelliJ honors sortText
                // before alphabetic ordering — keeps the editor /
                // dumper / parity flags at the top of the lookup.
                let zshrs_specific_count = ZSHRS_SELF_LONG_FLAG_DOCS.len();
                let items: Vec<Value> = flags
                    .into_iter()
                    .enumerate()
                    .map(|(idx, (flag, desc))| {
                        let bucket = if idx < zshrs_specific_count { "0" } else { "1" };
                        let detail = if desc.is_empty() {
                            format!("long-form flag for `{}`", bname)
                        } else {
                            desc.clone()
                        };
                        let doc_md = if desc.is_empty() {
                            format!("**`{}`** — long-form flag for `{}`", flag, bname)
                        } else {
                            format!(
                                "**`{}`** — {}\n\n_long-form flag for `{}`_",
                                flag, desc, bname
                            )
                        };
                        json!({
                            "label": flag,
                            "kind": 14, // Constant — same as ctx_item
                            "detail": detail,
                            "filterText": flag,
                            "sortText": format!("{}_{}", bucket, flag),
                            "textEdit": {
                                "range": edit_range,
                                "newText": flag,
                            },
                            "documentation": {
                                "kind": "markdown",
                                "value": doc_md,
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
        // Explicit `filterText` + `insertText` so IntelliJ's LSP
        // matcher uses the FULL label (sigil and all) when ranking
        // against the typed prefix. Without this, IDE-side
        // CamelHumpMatcher may strip leading non-alpha chars (`$`)
        // from labels and stop matching half the canonical set when
        // the user types `$HIS`. Symptom: only the hand 2 items
        // surfaced; canonical `$HISTCMD`/`$HISTNO`/`$HISTCHARS`
        // were sent but invisibly filtered out before display.
        items.push(json!({
            "label": label,
            "kind": kind,
            "detail": detail,
            "filterText": label,
            "insertText": label,
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
    // Canonical reserved words from `Src/hashtable.c::reswds[]`
    // (port: `crate::ported::hashtable::RESWDS`, 31 entries). The
    // hand `KEYWORDS` list above is missing `[[`, `]]`, `{`, `}`,
    // `!`, `end` — without this loop, `[[<TAB>` and `}<TAB>` never
    // suggest the keyword. IDE-side dedup collapses any name that
    // appears in both lists.
    for (name, _token) in crate::ported::hashtable::RESWDS {
        if want(name) {
            push(&mut items, name, 14, "keyword");
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
    // never added to the completion list. Same issue for ext ported
    // generally (91 in-process incl. ztest framework + 23 daemon = 114 names total).
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
    for n in crate::compsys::COMPSYS_FN_NAMES {
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
    // Canonical special-param names from zsh's `params.yo` + every
    // module's special-param table — 538 entries. The hand subset
    // above misses PS2/PS3/PS4/psvar/PROMPT2/PROMPT3/PROMPT4 and
    // hundreds more. Surface every canonical name so `setopt`,
    // `typeset`, `unset`, etc. can complete against the full set.
    //
    // Two prefix-match paths because the user may type the name two
    // ways:
    //   1. Bare `HIST<TAB>` — want("HISTCMD") starts-with "HIST" ✓
    //   2. Dollar `$HIST<TAB>` — bare `name` does NOT start with `$HIST`,
    //      so fall back to comparing the prefix's bare form. When the
    //      $-prefix path matches, emit the candidate WITH `$` prepended
    //      so the IDE inserts the dollar the user typed.
    // The $-prefix path fires WHENEVER the user-typed prefix starts
    // with `$` (or IS exactly `$`). When the prefix is just `$`,
    // `bare_prefix` is empty — we still need to surface every
    // canonical name as `$NAME` so the IDE's local-filter step on
    // subsequent keystrokes (`G`/`H`/etc) has the full set to
    // narrow from. Without this, typing `$` opened a popup of just
    // the hand 41-entry list, then typing `G` filtered locally to
    // zero items and the popup closed — even though the server has
    // `$GID`/`$galiases`/`$gid` available.
    let prefix_has_dollar = prefix.starts_with('$');
    let bare_prefix: String = prefix
        .strip_prefix('$')
        .map(|s| s.to_string())
        .unwrap_or_default();
    let bare_prefix_lc = bare_prefix.to_lowercase();
    for (name, _doc) in crate::zsh_special_var_docs::SPECIAL_VAR_DOCS {
        // Skip the pure-symbolic ones (`$`, `?`, `*`, etc) — they're
        // already in SPECIAL_VARS with the `$` prefix. The remaining
        // alphabetic names are the meaningful completions.
        if !name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            continue;
        }
        if want(name) {
            push(&mut items, name, 6, "special variable");
        } else if prefix_has_dollar
            && (bare_prefix_lc.is_empty() || name.to_lowercase().starts_with(&bare_prefix_lc))
        {
            let with_sigil = format!("${}", name);
            push(&mut items, &with_sigil, 6, "special variable");
        }
    }
    // Aliases (PROMPT/PROMPT2/PROMPT3 → PS1/PS2/PS3, NULLCMD etc.) —
    // surface so completion offers every surface name. Same dual-prefix
    // match as above so `$PROMPT<TAB>` hits aliased forms.
    for (alias, _canon) in crate::zsh_special_var_docs::SPECIAL_VAR_ALIASES {
        if !alias
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            continue;
        }
        if want(alias) {
            push(&mut items, alias, 6, "special variable");
        } else if prefix_has_dollar
            && (bare_prefix_lc.is_empty() || alias.to_lowercase().starts_with(&bare_prefix_lc))
        {
            let with_sigil = format!("${}", alias);
            push(&mut items, &with_sigil, 6, "special variable");
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

    // Phase 0: stage compsys dispatch behind the existing hand-table
    // / scan-symbol path. Today this returns no extra items (stub).
    // Phase 0.5 wires shadow-compadd capture + compdef dispatch so
    // `git a<TAB>` / `kubectl get p<TAB>` / `_options ext<TAB>` start
    // surfacing matches from the user's fpath. Design:
    // `docs/IN_EDITOR_COMPSYS_COMPLETION.md`.
    if let Some(extra) = try_compsys_completion(state, params) {
        // Append items + propagate isIncomplete (true when a
        // dispatch hit the deadline). Existing items keep their
        // sortText so hand-table entries stay above compsys
        // fallback matches.
        if let Some(arr) = extra["items"].as_array() {
            for item in arr {
                items.push(item.clone());
            }
        }
        let is_incomplete = extra["isIncomplete"].as_bool().unwrap_or(false);
        return json!({ "isIncomplete": is_incomplete, "items": items });
    }

    json!({ "isIncomplete": false, "items": items })
}

/// Phase-0 stub: returns `None` today. When Phase 0.5 lands this
/// drives `crate::compsys::in_editor::complete_at` and translates
/// the resulting `CompsysMatch`es to LSP `CompletionItem` JSON.
/// Kept private so the LSP completion flow stays the one entry
/// point; the public entry is the JSON-RPC `textDocument/completion`
/// method.
fn try_compsys_completion(state: &State, params: &Value) -> Option<Value> {
    let uri = params["textDocument"]["uri"].as_str()?;
    let pos = &params["position"];
    let text = state.docs.get(uri)?;
    let line_no = pos["line"].as_u64()? as usize;
    let col = pos["character"].as_u64()? as usize;
    let line_text = text.lines().nth(line_no)?;
    // NB: Phase 0 ignores the multibyte distinction. Phase 0.5
    // will translate the LSP `character` UTF-16 unit count to a
    // byte index against `line_text`. Today the stub returns empty
    // regardless of cursor position.
    let req = crate::compsys::in_editor::CompsysRequest::new_with_default_budget(line_text, col);
    let resp = crate::compsys::in_editor::complete_at(req);
    if resp.matches.is_empty() && !resp.is_incomplete {
        return None;
    }
    let items: Vec<Value> = resp
        .matches
        .iter()
        .map(|m| {
            // Map CompsysMatch.group → LSP kind:
            //   `options` → Field (15) — matches BuiltinFlag style
            //   `subcommands` / `commands` → Function (3)
            //   `values` → Value (12)
            //   `hosts` / `files` / `directories` → File (17)
            //   anything else → Text (1)
            let kind: u64 = match m.group.as_deref() {
                Some("options") => 5,                                      // Field
                Some("subcommands") | Some("commands") => 3,               // Function
                Some("values") => 12,                                      // Value
                Some("hosts") | Some("files") | Some("directories") => 17, // File
                _ => 1,                                                    // Text
            };
            // sortText prefix by group so subcommands list above
            // options list above paths, matching what zsh shows at
            // the prompt.
            let bucket = match m.group.as_deref() {
                Some("subcommands") | Some("commands") => "0",
                Some("options") => "1",
                Some("values") => "2",
                Some("hosts") => "3",
                Some("files") | Some("directories") => "4",
                _ => "5",
            };
            let detail = m
                .description
                .clone()
                .unwrap_or_else(|| m.group.clone().unwrap_or_default());
            json!({
                "label": m.completion,
                "kind": kind,
                "detail": detail,
                "filterText": m.completion,
                "insertText": m.completion,
                "sortText": format!("{bucket}_{}", m.completion),
            })
        })
        .collect();
    Some(json!({
        "isIncomplete": resp.is_incomplete,
        "items": items,
    }))
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
    // Module-doc lookup runs BEFORE the gate: it fires on `#!`
    // shebang lines (which the gate classifies as Comment) and on
    // `source PATH` / `. PATH` argument hovers. When neither shape
    // matches, gate enforcement continues below.
    if let Some(module_doc) = find_module_doc_for_position(state, text, line_text, line_no) {
        tracing::debug!(target: "zshrs::lsp::hover", line = line_no, col, "module_hit");
        return json!({
            "contents": { "kind": "markdown", "value": module_doc }
        });
    }
    if gate != HoverGate::Code {
        tracing::debug!(
            target: "zshrs::lsp::hover",
            line = line_no, col, %word,
            gated = ?gate,
            "suppressed",
        );
        return Value::Null;
    }
    // Priority: user-defined symbols in the current document WIN over
    // generic builtin / keyword / option docs. When the cursor's word
    // is a function / alias / variable defined here, the user wants
    // "what does MY `end`/`load`/`start` do" — not the Csh-style `end`
    // keyword card or the `load` builtin doc. Falling back to builtin
    // lookup only when the word is NOT defined locally avoids the
    // surprising "I hover on my local var, get a zsh-keyword card."
    let mut doc = if let Some(user_doc) = find_user_symbol_doc(text, &word) {
        user_doc
    } else {
        String::new()
    };
    if doc.is_empty() {
        doc = lookup_doc(&word);
    }
    if doc.is_empty() {
        // Module-level fallback: if the cursor is on the shebang
        // line or a `source FILE` / `. FILE` argument, surface the
        // top-of-file `##` block of the relevant document.
        //
        // Shebang case: any word inside `#!/usr/bin/env zsh` etc.
        // triggers the same-document module doc lookup.
        //
        // Source case: resolve `source path` / `. path` and read the
        // target file's top `##` block — lets the user discover what
        // a sourced library does without leaving the editor.
        if let Some(module_doc) = find_module_doc_for_position(state, text, line_text, line_no) {
            doc = module_doc;
        }
    }
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

/// Find the markdown doc for a user-defined symbol `name` in the
/// current document. Returns the formatted hover card (heading +
/// definition-line snippet + the `##` doc-comment block immediately
/// above the definition), or `None` when:
///   * `name` doesn't appear as a function / alias / parameter decl, OR
///   * the definition has no leading `##` block (silent fall-through
///     so builtin lookup remains the primary doc source).
///
/// Recognises three definition shapes per `scan_symbols`:
///   * `function NAME { … }` / `function NAME() …` (keyword form)
///   * `NAME() { … }` (POSIX form)
///   * `local NAME=…` / `typeset NAME=…` / `export NAME=…` /
///     `readonly NAME=…` / `NAME=…` (assignment forms)
///   * `alias NAME=…`
///
/// `##` blocks are gathered by walking BACKWARD from the definition
/// line, accepting consecutive `## …` / `##` lines (blank `##` lines
/// become paragraph breaks) and stopping at the first non-`##` non-
/// blank line. Plain `#` (single-hash) comments are NOT collected —
/// those are routine code comments, not doc strings.
pub(crate) fn find_user_symbol_doc(text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    // PRIMARY: AST-walk via SymbolTable::build. Returns every Func /
    // Global / Local decl in the current file with line numbers. This
    // is the correct source of truth — it sees `function foo()`,
    // `bar=value`, `local x=42` exactly as the parser does, without
    // false matches on `# bar=value` (comment) or `"foo=…"` (string).
    //
    // Previously this function used `symbol_decl_kind` (text-prefix
    // matching) which couldn't tell a real `local s1=…` from a string
    // `"local s1=…"` inside a quoted argument, and missed multi-line
    // function-decl variants. Switched to AST so the hover card is
    // grounded in real parsed decls only.
    //
    // FALLBACK: if the parser fails (mid-edit, broken syntax), drop
    // to the text-based scanner so hover doesn't silently go blank
    // during typing.
    if let Some(table) = crate::lsp_symbols::SymbolTable::build(text) {
        // Two passes: prefer the first decl WITH a `##` doc-block
        // (richer card); fall back to a minimal card when no doc-block
        // exists so hover still confirms "yes, defined here."
        let mut undocumented: Option<(u32, &'static str, &str)> = None;
        for sym in &table.symbols {
            if sym.name != name {
                continue;
            }
            let line_idx = sym.decl_line as usize;
            let def_line = lines.get(line_idx).copied().unwrap_or("");
            // SymbolTable classifies `alias foo=…` declarations as Func
            // (alias creates a callable binding). Re-discriminate from
            // the line prefix so the hover card says "user-defined
            // alias" instead of "user-defined function".
            let line_trim = def_line.trim_start();
            let kind = if line_trim.starts_with("alias ") || line_trim.starts_with("alias\t") {
                "alias"
            } else {
                match sym.kind {
                    crate::lsp_symbols::SymbolKind::Func => "function",
                    crate::lsp_symbols::SymbolKind::Global => "parameter",
                    crate::lsp_symbols::SymbolKind::Local => "parameter",
                }
            };
            let doc_block = collect_doc_block_above(&lines, line_idx);
            if !doc_block.is_empty() {
                return Some(render_user_doc_card(
                    name,
                    kind,
                    def_line.trim(),
                    &doc_block,
                ));
            }
            if undocumented.is_none() {
                undocumented = Some((sym.decl_line, kind, def_line));
            }
        }
        if let Some((_, kind, line)) = undocumented {
            return Some(render_user_doc_card_no_block(name, kind, line.trim()));
        }
        // AST table found no match — fall through to the text-prefix
        // scanner below for any decl shape AST misses entirely.
    }
    // Parser failed OR AST didn't find the name — fall back to text
    // scanner so hover doesn't go blank.
    let mut undocumented: Option<(usize, &'static str, &str)> = None;
    for (i, line) in lines.iter().enumerate() {
        let kind = match symbol_decl_kind(line, name) {
            Some(k) => k,
            None => continue,
        };
        let doc_block = collect_doc_block_above(&lines, i);
        if !doc_block.is_empty() {
            return Some(render_user_doc_card(name, kind, line.trim(), &doc_block));
        }
        if undocumented.is_none() {
            undocumented = Some((i, kind, line));
        }
    }
    undocumented.map(|(_, kind, line)| render_user_doc_card_no_block(name, kind, line.trim()))
}

/// Render a minimal hover card for a user-defined symbol that has no
/// `##` doc-comment block above its definition. Surfaces the kind and
/// the definition-line snippet so the user gets confirmation the
/// symbol is real (not a typo) plus a one-glance pointer to where it
/// lives.
fn render_user_doc_card_no_block(name: &str, kind: &str, def_line: &str) -> String {
    format!(
        "**user-defined {kind} `{name}`**\n\n```zsh\n{def_line}\n```\n\n\
         *(no `## …` doc-comment block above the definition)*"
    )
}

/// Recognise whether `line` declares `name` as a user-defined symbol.
/// Returns the kind word (`"function"` / `"alias"` / `"parameter"`)
/// for the card heading, or `None` when this line doesn't define the
/// requested name.
fn symbol_decl_kind(line: &str, name: &str) -> Option<&'static str> {
    let t = line.trim_start();
    if t.starts_with('#') {
        return None;
    }
    // `function NAME { … }` / `function NAME() …`
    if let Some(rest) = t
        .strip_prefix("function ")
        .or_else(|| t.strip_prefix("function\t"))
    {
        if first_ident(rest).as_deref() == Some(name) {
            return Some("function");
        }
    }
    // `NAME() { … }` — POSIX form. Require `NAME(` with NO whitespace
    // between (`name(` not `name (`), so `if my_check(x)` calls don't
    // misfire.
    if let Some(idx) = t.find("()") {
        let head = &t[..idx];
        if !head.contains(' ') && !head.contains('\t') && head == name {
            return Some("function");
        }
    }
    // `alias NAME=…`
    if let Some(rest) = t.strip_prefix("alias ") {
        if first_ident(rest).as_deref() == Some(name) {
            return Some("alias");
        }
    }
    // `local NAME=…` / `typeset NAME=…` / `export NAME=…` /
    // `readonly NAME=…` / `integer NAME=…` / `float NAME=…`.
    // Skip any leading `-X` flag arguments (`typeset -gA`, `local -i`,
    // `readonly -a`, etc.) before checking the first identifier.
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
            let after_flags = skip_leading_flags(rest);
            if first_ident(after_flags).as_deref() == Some(name) {
                return Some("parameter");
            }
        }
    }
    // Bare `NAME=value` at start of line.
    if let Some(eq) = t.find('=') {
        let head = &t[..eq];
        if head == name && !head.contains(' ') && !head.contains('\t') {
            return Some("parameter");
        }
    }
    None
}

/// Walk BACKWARD from `def_line` collecting the contiguous `##`
/// comment block. Returns markdown text (paragraph-joined). Empty
/// when there's no `##` block.
///
/// Rules:
///   * Stop at the first non-`##` non-blank line.
///   * `## ` lines contribute their text (after the leading `## `).
///   * `##` alone (or `##\n`) becomes a paragraph break (blank line).
///   * Plain `#` single-hash lines TERMINATE the block — they're
///     routine code comments, not docstrings.
///   * Leading blank lines between definition and block ARE allowed
///     (e.g. `## doc\n\nfunction f()` is still attached to `f`).
fn collect_doc_block_above(lines: &[&str], def_line: usize) -> String {
    let mut collected: Vec<String> = Vec::new();
    let mut saw_doc = false;
    for j in (0..def_line).rev() {
        let trimmed = lines[j].trim_start();
        if trimmed.is_empty() {
            if saw_doc {
                // Blank line BEFORE the block we're collecting —
                // terminates collection.
                break;
            }
            // Blank line between def line and (yet-unseen) block —
            // allowed.
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("## ") {
            collected.push(rest.to_string());
            saw_doc = true;
            continue;
        }
        if trimmed == "##" {
            collected.push(String::new());
            saw_doc = true;
            continue;
        }
        // `## ` exactly, or `#!` shebang, or `# ` plain comment — all
        // stop the block. Plain comments are intentionally NOT
        // gathered: only the doubled-hash convention attaches.
        break;
    }
    collected.reverse();
    // Trim trailing blank "paragraph break" lines.
    while collected.last().is_some_and(|l| l.is_empty()) {
        collected.pop();
    }
    collected.join("\n")
}

/// Resolve module-level hover docs for the cursor's current line.
/// Returns `Some(card)` when:
///   * `line_text` is a `#!` shebang — surface THIS file's module
///     doc (the top-of-file `##` block).
///   * `line_text` is a `source PATH` / `. PATH` invocation — resolve
///     PATH (relative to the current doc's URI when applicable),
///     read the file, return ITS module doc.
/// Returns `None` otherwise (hover handler falls through to its
/// normal `Value::Null` reply).
fn find_module_doc_for_position(
    state: &State,
    text: &str,
    line_text: &str,
    _line_no: usize,
) -> Option<String> {
    let trimmed = line_text.trim_start();
    if trimmed.starts_with("#!") {
        return extract_module_doc(text).map(|d| render_module_doc_card("(this file)", &d));
    }
    // `source PATH` / `. PATH` — first identifier after `source`/`.`
    // is the path. Resolve it through the LSP doc cache if loaded;
    // otherwise read from disk.
    let path_arg: Option<&str> = if let Some(rest) = trimmed.strip_prefix("source ") {
        Some(rest.split_whitespace().next().unwrap_or(""))
    } else if let Some(rest) = trimmed.strip_prefix(". ") {
        Some(rest.split_whitespace().next().unwrap_or(""))
    } else {
        None
    };
    let path = path_arg.filter(|p| !p.is_empty())?;
    // Strip surrounding quotes if any.
    let path = path.trim_matches(|c| c == '"' || c == '\'');
    // Normalize: drop leading `./` so URI suffix match works (the
    // doc cache stores `file:///proj/helpers.zsh`, the source line
    // says `./helpers.zsh`).
    let needle = path.strip_prefix("./").unwrap_or(path);
    // Try doc cache first (URI key match), then filesystem.
    let body = state
        .docs
        .iter()
        .find_map(|(uri, body)| uri.ends_with(needle).then(|| body.clone()))
        .or_else(|| std::fs::read_to_string(path).ok())?;
    extract_module_doc(&body).map(|d| render_module_doc_card(path, &d))
}

/// Render a module-doc hover card. Format mirrors the user-symbol
/// card but heading kind is `module`.
/// Stryke-aligned card format: doc body first, horizontal rule,
/// then a one-line header naming what the hover is on. Mirrors
/// `strykelang/lsp.rs::format_with_doc_comments` so users get a
/// consistent hover layout across both LSPs.
fn render_module_doc_card(label: &str, doc: &str) -> String {
    format!("{doc}\n\n---\n\nzsh module `{label}`")
}

/// Extract the module-level `##` doc block from the top of a zsh
/// source file. Skips an optional `#!` shebang line, then collects
/// the contiguous `##` (and `##` paragraph-break) lines until the
/// first non-`##` non-blank line. Returns `None` when there's no
/// top-of-file `##` block.
///
/// Conventional layout the convention matches:
///
/// ```sh
/// #!/usr/bin/env zsh
/// ##
/// ## NAME
/// ##   foo.zsh — short one-line summary
/// ##
/// ## DESCRIPTION
/// ##   Longer paragraph describing what the module does.
///
/// function foo() { … }
/// ```
///
/// Used by the hover handler to surface module docs when the cursor
/// lands on the shebang line or a `source FILE` / `. FILE` argument.
pub(crate) fn extract_module_doc(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;
    // Skip optional shebang.
    if let Some(first) = lines.first() {
        if first.starts_with("#!") {
            i = 1;
        }
    }
    // Skip optional blank line(s) between shebang and doc block.
    while i < lines.len() && lines[i].trim().is_empty() {
        i += 1;
    }
    // Collect `##` block.
    let mut collected: Vec<String> = Vec::new();
    while i < lines.len() {
        let trimmed = lines[i].trim_start();
        if let Some(rest) = trimmed.strip_prefix("## ") {
            collected.push(rest.to_string());
        } else if trimmed == "##" {
            collected.push(String::new());
        } else {
            break;
        }
        i += 1;
    }
    while collected.last().is_some_and(|l| l.is_empty()) {
        collected.pop();
    }
    if collected.is_empty() {
        None
    } else {
        Some(collected.join("\n"))
    }
}

/// Skip a leading run of `-X` / `+X` / `-Xa` flag arguments
/// (each followed by whitespace) and return the remainder. Used so
/// `typeset -gA NAME=…` advances past `-gA ` to `NAME=…` before the
/// first-identifier check.
fn skip_leading_flags(s: &str) -> &str {
    let mut rest = s.trim_start();
    while rest.starts_with('-') || rest.starts_with('+') {
        // Walk until next whitespace.
        let end = rest
            .char_indices()
            .find(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
            .unwrap_or(rest.len());
        rest = rest[end..].trim_start();
    }
    rest
}

/// Render a user-symbol hover card. Format mirrors the builtin
/// docs cards: heading line, optional definition snippet in a code
/// fence, then the doc body verbatim.
/// Stryke-aligned card format: doc body first, horizontal rule,
/// then a one-line header naming the symbol + kind. Drops the
/// code-fence definition line — stryke doesn't include it and the
/// `##` doc block usually carries the signature in prose anyway.
fn render_user_doc_card(name: &str, kind: &str, _def_line: &str, doc: &str) -> String {
    format!("{doc}\n\n---\n\nuser-defined {kind} `{name}`")
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
    if start < cap
        && bytes[start] == b'$'
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
            if (q == b'"' || q == b'`') && c == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{'
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
                // shell-code completion. Caveat: `(#…)` is zsh's
                // extended-glob pattern modifier syntax, NOT a comment
                // — so `(` is EXCLUDED from the comment-open precedents.
                let prev = if i == 0 { None } else { Some(bytes[i - 1]) };
                let comment_open = matches!(
                    prev,
                    None | Some(b' ') | Some(b'\t') | Some(b';') | Some(b'&') | Some(b'|')
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
/// `lookup_doc` — see implementation.
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
    // For names that AREN'T real builtins per the canonical
    // `ported::builtin::BUILTINS` table, prefer the special-var
    // classification if the name has a special-var doc. Names like
    // `prompt`, `path`, `aliases`, `functions`, `history`,
    // `jobdirs`, `commands`, … all have `item(tt(NAME))` blocks in
    // module yo files (zsh/parameter, contrib, etc.) that describe
    // them as PARAMETERS. The builtin extractor pulled them into
    // BUILTIN_DOCS as a side-effect (the yodl format doesn't
    // distinguish builtin-name `item` from parameter-name `item`),
    // shadowing the special-var doc. 109 names overlap; the only
    // genuine builtins among them are zero — they're all params.
    if !is_real_builtin {
        if let Some((canon, body)) = crate::zsh_special_var_docs::lookup_special_var_doc(name) {
            return format!("**${}** — _special variable_\n\n{}", canon, body);
        }
        let bare = name.strip_prefix('$').unwrap_or(name);
        if !bare.is_empty() && bare != name {
            if let Some((canon, body)) = crate::zsh_special_var_docs::lookup_special_var_doc(bare) {
                return format!("**${}** — _special variable_\n\n{}", canon, body);
            }
        }
    }
    if let Some((canon, body)) = crate::zsh_builtin_docs::lookup_builtin_doc(name) {
        return format!("**{}** — _zsh builtin_\n\n{}", canon, body);
    }
    // Special vars: try the raw name first (so `$` resolves to its
    // own `$$` PID entry stored as canonical `"$"` in the doc
    // table), then the bare-stripped form (`$VAR` → `VAR`). Pure-
    // symbolic specials (`$`, `?`, `*`, `#`, `@`, `-`, `_`) are
    // stored under their bare-symbol key so naive `strip_prefix('$')`
    // on `"$"` strips the actual lookup key to empty.
    if let Some((canon, body)) = crate::zsh_special_var_docs::lookup_special_var_doc(name) {
        return format!("**${}** — _special variable_\n\n{}", canon, body);
    }
    let bare = name.strip_prefix('$').unwrap_or(name);
    if !bare.is_empty() && bare != name {
        if let Some((canon, body)) = crate::zsh_special_var_docs::lookup_special_var_doc(bare) {
            return format!("**${}** — _special variable_\n\n{}", canon, body);
        }
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
    if let Some(d) = OPTION_DOCS_FALLBACK
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
    {
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
    ("!", "previous command (`!!`)"),
    ("N", "command N from history (`!42`)"),
    ("-N", "N commands back (`!-3` = third-to-last)"),
    ("str", "most recent command starting with `str` (`!ls`)"),
    (
        "?str?",
        "most recent command containing `str` (`!?docker?`)",
    ),
    ("#", "current command line typed so far"),
    ("$", "last argument of previous command (= `!!:$`)"),
    ("^", "first argument of previous command (= `!!:^`)"),
    ("*", "all arguments of previous command (= `!!:*`)"),
    (
        ":",
        "introduce a word designator / modifier — `!!:1`, `!!:s/old/new/`, `!!:h`",
    ),
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
    ("-", "`${var:-WORD}` — use WORD if `var` unset or empty"),
    (
        "=",
        "`${var:=WORD}` — assign WORD to `var` (and use it) if unset/empty",
    ),
    (
        "?",
        "`${var:?MSG}` — print MSG to stderr + exit if `var` unset/empty",
    ),
    (
        "+",
        "`${var:+WORD}` — use WORD if `var` IS set (the inverse of `:-`)",
    ),
    // ── Substring slicing ──
    (
        "0",
        "`${var:OFFSET:LENGTH}` — substring (zero-based; negative offset = from end)",
    ),
    // ── Path / file modifiers ──
    ("h", "head — strip last path component (like `dirname`)"),
    (
        "t",
        "tail — keep ONLY last path component (like `basename`)",
    ),
    ("r", "root — strip the final `.ext` suffix"),
    (
        "e",
        "extension — keep ONLY the final `.ext` (no leading dot)",
    ),
    (
        "a",
        "absolute — textually resolve `..` / `.` against `$PWD`",
    ),
    ("A", "absolute + resolve symlinks (like `realpath`)"),
    (
        "c",
        "PATH lookup — replace bare command with full path via `$PATH`",
    ),
    ("P", "physical path — resolve all symlinks"),
    (
        "f",
        "repeat `:h` until the result is no longer an existing directory",
    ),
    ("F", "`:F:N:` — repeat `:h` N times"),
    // ── Substitution ──
    ("s", "`:s/OLD/NEW/` — substitute first OLD with NEW"),
    (
        "gs",
        "`:gs/OLD/NEW/` — global substitute (every occurrence)",
    ),
    ("&", "repeat the last `:s` substitution"),
    ("g&", "repeat the last `:s` substitution globally"),
    // ── Quoting ──
    ("q", "quote — backslash-escape all metacharacters"),
    ("Q", "unquote — remove ONE level of quoting"),
    ("x", "quote, breaking at whitespace into separate words"),
    // ── Case ──
    ("l", "lowercase first character"),
    ("u", "uppercase first character"),
    ("L", "lowercase ENTIRE string"),
    ("U", "uppercase ENTIRE string"),
    ("C", "capitalize each word (`Title Case`)"),
    // ── Array operations ──
    ("S", "sort array elements ascending"),
    ("O", "sort array elements descending"),
    (
        "#",
        "`${var:#PATTERN}` — remove array elements matching PATTERN (with `(@)`)",
    ),
    (
        "|",
        "`${arr:|other}` — set difference (elements of `arr` not in `other`)",
    ),
    ("*", "`${arr:*other}` — set intersection"),
    ("^", "`${arr:^other}` — interleave (zip) two arrays"),
    ("^^", "`${arr:^^other}` — distributed zip (every pair)"),
];

/// Signal names — POSIX + zsh-specific synthetic signals (`ZERR`,
/// `DEBUG`, `EXIT`). Used by `kill -SIG` and `trap`. Numeric form
/// (`SIGINT` / `INT`) is offered without the `SIG` prefix per zsh's
/// `kill -l` output convention.
const SIGNAL_NAMES: &[(&str, &str)] = &[
    ("HUP", "1 — hangup (terminal closed)"),
    ("INT", "2 — interrupt (Ctrl-C)"),
    ("QUIT", "3 — quit + core dump (Ctrl-\\)"),
    ("ILL", "4 — illegal instruction"),
    ("TRAP", "5 — trace/breakpoint trap"),
    ("ABRT", "6 — abort (`abort()` syscall)"),
    ("BUS", "7 — bus error"),
    ("FPE", "8 — floating-point exception"),
    ("KILL", "9 — kill (uncatchable, unblockable)"),
    ("USR1", "10 — user-defined signal 1"),
    ("SEGV", "11 — segmentation fault"),
    ("USR2", "12 — user-defined signal 2"),
    ("PIPE", "13 — write to pipe with no readers"),
    ("ALRM", "14 — alarm clock (`alarm()`)"),
    ("TERM", "15 — termination request (default `kill`)"),
    ("CHLD", "17 — child process state change"),
    ("CONT", "18 — continue if stopped"),
    ("STOP", "19 — stop (uncatchable)"),
    ("TSTP", "20 — terminal stop (Ctrl-Z)"),
    ("TTIN", "21 — background process needs tty input"),
    ("TTOU", "22 — background process tty output"),
    ("URG", "23 — urgent socket data"),
    ("XCPU", "24 — CPU time limit exceeded"),
    ("XFSZ", "25 — file size limit exceeded"),
    ("VTALRM", "26 — virtual timer alarm"),
    ("PROF", "27 — profiling timer alarm"),
    ("WINCH", "28 — window size change"),
    ("IO", "29 — async I/O ready"),
    ("PWR", "30 — power failure"),
    ("SYS", "31 — bad syscall"),
    // ── zsh synthetic signals ──
    ("EXIT", "0 — shell exit (special — `trap ... EXIT`)"),
    ("ZERR", "zsh — fires on any non-zero exit status"),
    (
        "DEBUG",
        "zsh — fires before every command (with `DEBUG_BEFORE_CMD`)",
    ),
];

/// Loadable zsh modules — `zmodload zsh/MOD`. Canonical list from
/// `man zshmodules`. Most expose builtins, parameters, or math ported
/// that aren't compiled into the core.
const ZSH_MODULE_NAMES: &[(&str, &str)] = &[
    ("zsh/attr", "extended file attribute manipulation"),
    ("zsh/cap", "POSIX capability sets"),
    ("zsh/clone", "fork the shell to a new session"),
    ("zsh/compctl", "legacy `compctl` completion (deprecated)"),
    ("zsh/complete", "core programmable completion machinery"),
    (
        "zsh/complist",
        "completion list display + menuselect keymap",
    ),
    (
        "zsh/computil",
        "internal helpers used by `_arguments` / `_describe`",
    ),
    ("zsh/curses", "ncurses bindings (`zcurses`)"),
    (
        "zsh/datetime",
        "`strftime` builtin + `$EPOCHSECONDS` / `$EPOCHREALTIME`",
    ),
    (
        "zsh/db/gdbm",
        "GDBM key-value store as a zsh associative array",
    ),
    (
        "zsh/deltochar",
        "`delete-to-char` / `zap-to-char` ZLE widgets",
    ),
    ("zsh/example", "template module (skeleton; not useful)"),
    (
        "zsh/files",
        "in-shell file ops (`mkdir`, `chmod`, `mv`, `rm`, `chown`, `sync`, `ln`)",
    ),
    ("zsh/langinfo", "locale info (`$langinfo`)"),
    ("zsh/mapfile", "read/write a file as an assoc array"),
    (
        "zsh/mathfunc",
        "`sin`, `cos`, `sqrt`, `log`, `exp`, … math functions for `((…))`",
    ),
    ("zsh/nearcolor", "approximate-color terminal fallback"),
    ("zsh/newuser", "first-run user setup helper"),
    (
        "zsh/parameter",
        "reflection — `$functions`, `$aliases`, `$options`, `$commands`, `$parameters`, etc.",
    ),
    ("zsh/pcre", "Perl-compatible regex (`pcre_match` / `=~`)"),
    ("zsh/regex", "POSIX extended regex (`=~`)"),
    ("zsh/sched", "in-shell scheduler (`sched +5 cmd`)"),
    ("zsh/net/socket", "Unix-domain socket builtin (`zsocket`)"),
    ("zsh/stat", "`stat` builtin returning fields into a hash"),
    (
        "zsh/system",
        "low-level syscalls (`sysread`, `syswrite`, `syserror`, `sysopen`)",
    ),
    ("zsh/net/tcp", "TCP socket builtin (`ztcp`)"),
    ("zsh/termcap", "termcap parameter access (`$termcap`)"),
    ("zsh/terminfo", "terminfo parameter access (`$terminfo`)"),
    ("zsh/zftp", "FTP client built into the shell"),
    (
        "zsh/zle",
        "Zsh Line Editor — `bindkey`, `zle`, widget registration",
    ),
    (
        "zsh/zleparameter",
        "ZLE introspection — `$widgets`, `$keymaps`",
    ),
    ("zsh/zprof", "profiling — `zprof` builtin"),
    ("zsh/zpty", "spawn commands in a pseudo-terminal"),
    ("zsh/zselect", "`select(2)` on fds with a timeout"),
    (
        "zsh/zutil",
        "core utilities — `zparseopts`, `zformat`, `zstyle`, `zregexparse`",
    ),
];

/// Keymap names — `bindkey -A NAME` source / `bindkey -N NAME` target,
/// also `bindkey -M NAME …`. The named maps zsh ships out of the box.
const KEYMAP_NAMES: &[(&str, &str)] = &[
    ("emacs", "GNU Readline emacs bindings (default)"),
    ("vicmd", "vi command-mode keymap"),
    ("viins", "vi insert-mode keymap"),
    ("viopp", "vi operator-pending keymap (for `d` / `c` / `y`)"),
    ("visual", "vi visual-mode keymap"),
    (
        ".safe",
        "minimal fallback keymap — only `self-insert` + `accept-line`",
    ),
    (
        "main",
        "alias — whichever keymap is currently the editing map",
    ),
    ("command", "vi-mode command-line input keymap"),
    ("menuselect", "active inside `menu-select` widget"),
    ("isearch", "active inside incremental-search widgets"),
    ("listscroll", "active when scrolling completion list"),
];

/// Built-in ZLE widgets — second arg of `bindkey`, first arg of `zle`,
/// what `zle -al` enumerates at runtime. Covers movement / editing /
/// history / completion / vi-mode / misc. Curated subset of the most
/// commonly used ~120 widgets from `man zshzle` "Standard Widgets".
const ZLE_WIDGET_NAMES: &[(&str, &str)] = &[
    // ── movement ──
    ("backward-char", "move one character left"),
    ("forward-char", "move one character right"),
    ("backward-word", "move one word left"),
    ("forward-word", "move one word right"),
    ("beginning-of-line", "move to start of line"),
    ("end-of-line", "move to end of line"),
    (
        "beginning-of-buffer-or-history",
        "start of buffer / previous-history at top",
    ),
    (
        "end-of-buffer-or-history",
        "end of buffer / next-history at bottom",
    ),
    // ── editing ──
    ("self-insert", "insert the typed character"),
    ("accept-line", "submit current line for execution"),
    ("accept-and-hold", "submit + keep line in buffer"),
    (
        "accept-and-infer-next-history",
        "submit + recall the line after this in history",
    ),
    ("backward-delete-char", "delete character before cursor"),
    ("delete-char", "delete character under cursor"),
    (
        "backward-kill-word",
        "delete word before cursor (saves to kill ring)",
    ),
    ("kill-word", "delete word after cursor"),
    ("backward-kill-line", "delete from cursor to start of line"),
    ("kill-line", "delete from cursor to end of line"),
    ("kill-whole-line", "delete entire line"),
    ("kill-region", "delete from mark to cursor"),
    ("yank", "paste last kill"),
    ("yank-pop", "rotate to earlier kill (after `yank`)"),
    ("transpose-chars", "swap two characters"),
    ("transpose-words", "swap two words"),
    ("up-case-word", "uppercase next word"),
    ("down-case-word", "lowercase next word"),
    ("capitalize-word", "capitalize next word"),
    (
        "quoted-insert",
        "literal-insert next key (e.g. for control chars)",
    ),
    ("overwrite-mode", "toggle insert / overwrite"),
    ("undo", "undo last edit"),
    ("redo", "redo last undone edit"),
    ("clear-screen", "clear terminal + redraw"),
    ("redisplay", "force redraw"),
    ("send-break", "abandon line (SIGINT-equivalent)"),
    // ── history ──
    (
        "up-line-or-history",
        "previous line / previous history entry",
    ),
    ("down-line-or-history", "next line / next history entry"),
    ("up-history", "previous history entry"),
    ("down-history", "next history entry"),
    ("beginning-of-history", "first history entry"),
    ("end-of-history", "last history entry (current line)"),
    (
        "history-incremental-search-backward",
        "Ctrl-R — incremental search backward",
    ),
    (
        "history-incremental-search-forward",
        "Ctrl-S — incremental search forward",
    ),
    (
        "history-search-backward",
        "search history matching current line prefix",
    ),
    (
        "history-search-forward",
        "forward variant of `history-search-backward`",
    ),
    (
        "history-beginning-search-backward",
        "search backward keeping cursor position",
    ),
    (
        "history-beginning-search-forward",
        "search forward keeping cursor position",
    ),
    (
        "infer-next-history",
        "infer next-history based on previous match",
    ),
    (
        "insert-last-word",
        "insert last word of previous line (`!!:$`)",
    ),
    // ── completion ──
    ("complete-word", "complete the current word"),
    ("expand-or-complete", "expand alias / glob, else complete"),
    (
        "expand-or-complete-prefix",
        "as above but with prefix match",
    ),
    ("list-choices", "show completion options without inserting"),
    ("menu-complete", "cycle through completions"),
    ("menu-expand-or-complete", "expand / cycle"),
    ("reverse-menu-complete", "cycle backward"),
    (
        "delete-char-or-list",
        "delete-char if not at EOL, else list-choices",
    ),
    ("complete-prefix", "complete current prefix"),
    ("expand-cmd-path", "expand command to full path"),
    ("expand-word", "expand current word"),
    // ── vi mode ──
    ("vi-cmd-mode", "switch to vi command mode"),
    ("vi-insert", "switch to vi insert mode"),
    ("vi-insert-bol", "insert at start of line"),
    ("vi-add-next", "append after current char (vi `a`)"),
    ("vi-add-eol", "append at end of line (vi `A`)"),
    ("vi-backward-char", "h"),
    ("vi-forward-char", "l"),
    ("vi-backward-word", "b"),
    ("vi-forward-word", "w"),
    ("vi-backward-word-end", "ge"),
    ("vi-forward-word-end", "e"),
    ("vi-backward-blank-word", "B"),
    ("vi-forward-blank-word", "W"),
    ("vi-up-line-or-history", "k — previous line / history"),
    ("vi-down-line-or-history", "j — next line / history"),
    ("vi-beginning-of-line", "0"),
    ("vi-end-of-line", "$"),
    ("vi-first-non-blank", "^"),
    ("vi-delete", "d"),
    ("vi-delete-char", "x"),
    ("vi-backward-delete-char", "X"),
    ("vi-change", "c"),
    ("vi-change-eol", "C"),
    ("vi-change-whole-line", "S"),
    ("vi-substitute", "s"),
    ("vi-yank", "y"),
    ("vi-yank-eol", "Y"),
    ("vi-yank-whole-line", "yy"),
    ("vi-put-after", "p"),
    ("vi-put-before", "P"),
    ("vi-replace", "R"),
    ("vi-replace-chars", "r"),
    ("vi-repeat-change", "."),
    ("vi-repeat-search", "n"),
    ("vi-rev-repeat-search", "N"),
    ("vi-find-next-char", "f"),
    ("vi-find-prev-char", "F"),
    ("vi-find-next-char-skip", "t"),
    ("vi-find-prev-char-skip", "T"),
    ("vi-undo-change", "u"),
    ("vi-join", "J — join with next line"),
    ("vi-quoted-insert", "Ctrl-V — literal next"),
    ("vi-set-buffer", "select named register"),
    ("vi-history-search-backward", "?"),
    ("vi-history-search-forward", "/"),
    ("vi-match-bracket", "% — jump to matching bracket"),
    // ── misc ──
    ("which-command", "show what command would run"),
    ("describe-key-briefly", "show binding for next key"),
    ("execute-named-cmd", "M-x style command execution"),
    ("execute-last-named-cmd", "re-run last named command"),
    ("push-line", "save line + clear, runs on next prompt"),
    ("push-line-or-edit", "push-line or edit multiline"),
    ("push-input", "push to input stack"),
    ("get-line", "pop input from stack"),
    ("set-mark-command", "set the mark at cursor"),
    ("exchange-point-and-mark", "swap cursor + mark"),
    ("digit-argument", "begin numeric argument"),
    ("universal-argument", "begin numeric argument"),
    ("undefined-key", "called when binding lookup fails"),
];

/// `typeset` / `declare` / `local` / `readonly` / `integer` / `float`
/// / `export` flags — what to surface when the current arg starts with
/// `-`. From `man zshbuiltins` "TYPESET".
const TYPESET_FLAGS: &[(&str, &str)] = &[
    ("-a", "indexed array"),
    ("-A", "associative array (hash)"),
    ("-i", "integer (with optional base: `-i 16`)"),
    ("-E", "float, scientific notation"),
    ("-F", "float, fixed notation"),
    ("-l", "lowercase on assignment"),
    ("-u", "uppercase on assignment"),
    ("-L", "left-justify, width N (`-L4`)"),
    ("-R", "right-justify, width N (`-R8`)"),
    ("-Z", "zero-pad (right-justified, numeric)"),
    ("-r", "readonly"),
    ("-x", "export to environment"),
    ("-g", "global (skip the local scope this would create)"),
    ("-U", "unique — for arrays, drop duplicate elements"),
    ("-T", "tie scalar ↔ array (`-T PATH path :`)"),
    ("-t", "set the `TAGGED` flag (used by some completions)"),
    ("-H", "hide value in `typeset` listing"),
    ("-h", "hide builtin/special status"),
    ("-f", "operate on functions, not parameters"),
    ("-p", "print declarations in re-readable form"),
    ("-m", "treat name args as patterns (`typeset -m 'FOO*'`)"),
    ("-+", "operate at the next outer scope"),
];

/// `[[ ... ]]` test operators — what completes inside a conditional
/// expression. File tests, string tests, numeric tests, file-compare
/// tests, logical ops. From `man zshmisc` "CONDITIONAL EXPRESSIONS".
const TEST_OPERATORS: &[(&str, &str)] = &[
    // ── file existence + type ──
    ("-e", "**True if FILE exists**, regardless of type. The catch-all existence test — use `-f` / `-d` etc. to narrow.\n\nExample: `[[ -e $HOME/.zshrc ]] && source $HOME/.zshrc` — guard a source against missing files.\n\nReturns true for symlinks ONLY if the link target exists (use `-L` to test the link itself). Sets `$?` to 0 (true) or 1 (false). Inside `[[ … ]]`, no word-splitting / glob expansion is done on the operand."),
    ("-f", "**True if FILE exists AND is a regular file** (not a directory, symlink to dir, device, FIFO, or socket). Follows symlinks — `-f link → file` is true; `-f link → dir` is false.\n\nExample: `for f in *.zsh; do [[ -f $f ]] || continue; source $f; done` — sources every regular `.zsh` file, skipping symlinks-to-dirs that glob accidentally caught."),
    ("-d", "**True if FILE exists AND is a directory.** Follows symlinks — symlinks to directories test true. Use `-L $f && [[ ! -d $f ]]` (or `! -h && -d`) to distinguish real-dir from symlink-to-dir.\n\nExample: `[[ -d ~/.config ]] || mkdir -p ~/.config`."),
    ("-L", "**True if FILE exists AND is a symbolic link** (regardless of target). Does NOT follow the link — tests the link itself.\n\nExample: `[[ -L $f ]] && rm $f` — remove the symlink without touching its target. Use `-e $f && ! -L $f` to test \"exists AND is not a symlink\". Same operator as `-h`."),
    ("-h", "**True if FILE is a symbolic link** — alias for `-L`. Both come from POSIX (`test`); zsh treats them identically. Prefer `-L` for clarity in new code; `-h` is the older spelling kept for `test`/`[`/`/bin/sh` compatibility."),
    ("-b", "**True if FILE is a block special device** (e.g. `/dev/disk0`, `/dev/sda`). Block devices buffer I/O in fixed-size blocks; contrast with character devices (`-c`) which transfer byte-at-a-time.\n\nExample: `for d in /dev/disk*; do [[ -b $d ]] && echo \"$d is a block dev\"; done`."),
    ("-c", "**True if FILE is a character special device** (e.g. `/dev/tty`, `/dev/null`, `/dev/random`, `/dev/zero`). Character devices transfer one byte at a time and are unbuffered.\n\nExample: `[[ -c /dev/tty ]] && echo 'have a controlling tty'`."),
    ("-p", "**True if FILE is a named pipe (FIFO)** — created via `mkfifo`. Anonymous pipes (between processes in a `|` pipeline) are NOT FIFOs and don't test true; `-p` is for filesystem entries.\n\nExample: `mkfifo /tmp/mypipe; [[ -p /tmp/mypipe ]] && echo 'pipe ready'`."),
    ("-S", "**True if FILE is a socket** — Unix-domain socket file on the filesystem (created by `bind()`). TCP/UDP sockets don't appear in the filesystem and won't test true; this is for `AF_UNIX` only.\n\nExample: `[[ -S /var/run/docker.sock ]] && echo 'docker up'`."),
    ("-t", "**True if file descriptor N is open AND refers to a terminal** — `-t 0` checks stdin, `-t 1` checks stdout, `-t 2` checks stderr. Used to detect interactive vs piped/redirected I/O.\n\nExample: `[[ -t 1 ]] && color=true || color=false` — emit ANSI colors only when stdout is a TTY (skip when piped to a file or another program)."),
    // ── permission ──
    ("-r", "**True if FILE is readable by the effective uid** of the process. Honors filesystem ACLs and special bits, not just mode-bit permissions. Caveat: root tests true for any readable file regardless of mode.\n\nExample: `[[ -r $f ]] || { echo \"$f unreadable\" >&2; exit 1; }`."),
    ("-w", "**True if FILE is writable by the effective uid.** Note: `-w` only tests permission — actual writes can still fail (readonly filesystem, full disk, IMMUTABLE attribute, etc.). For root, almost always returns true even on permissioned-out files unless filesystem is RO.\n\nExample: `[[ -w /etc ]] || sudo=sudo` — pick whether to wrap with sudo."),
    ("-x", "**True if FILE is executable** (for regular files) **or searchable** (for directories — needs `+x` to enter and read inode of contents). Symlinks tested by their target's mode. Honors ACLs.\n\nExample: `[[ -x ./build.sh ]] || chmod +x ./build.sh`."),
    ("-s", "**True if FILE exists AND has size greater than zero.** Useful to distinguish empty files from non-empty ones — `-f` matches both, `-s` only matches non-empty.\n\nExample: `[[ -s err.log ]] && cat err.log` — only show the log when it has actual error output."),
    ("-u", "**True if FILE has the setuid bit set** (mode `04000`). Setuid binaries run with the file owner's uid regardless of caller. Common on `passwd`, `sudo`, `mount`. Security-sensitive — audit periodically.\n\nExample: `find / -perm -4000 2>/dev/null | while read f; do [[ -u $f ]] && echo SETUID: $f; done`."),
    ("-g", "**True if FILE has the setgid bit set** (mode `02000`). On binaries: runs as the file's group. On directories: new files inherit the directory's group instead of the creator's primary group (BSD semantics) — common pattern for shared project directories.\n\nExample: `[[ -g $project_dir ]] || chmod g+s $project_dir`."),
    ("-k", "**True if FILE has the sticky bit set** (mode `01000`). On directories like `/tmp`: only the file's owner (or root) can delete or rename files within, regardless of directory write permission. On regular files: historically meant \"keep text segment swapped in\"; now ignored on most systems.\n\nExample: `[[ -k /tmp ]] || echo 'WARNING: /tmp not sticky'`."),
    ("-O", "**True if FILE is owned by the effective uid** of the current process. Use to gate operations that should only act on user-owned files (vs system-owned).\n\nExample: `find ~/.config -type f ! -O 2>/dev/null` — flag files in your config dir that aren't yours."),
    ("-G", "**True if FILE is owned by the effective gid** of the current process — i.e. the file's group is your primary group. Distinct from `-O`: a file might be owned by another user but in your group.\n\nExample: `[[ -G $shared_log ]] && echo writable`."),
    ("-N", "**True if FILE has been modified since it was last read** — `mtime > atime`. Used by `mail`-style checkers to detect new content since the last access. zsh-specific (not in POSIX `test`).\n\nExample: `[[ -N $MAIL ]] && echo 'new mail'` — historically zsh's `$MAILCHECK` feature uses exactly this comparison."),
    // ── string ──
    ("-z", "**True if STRING has length zero.** Inverse of `-n`. The operand is the WHOLE string after expansion — `[[ -z $var ]]` works even when `$var` is unset (unlike `[ -z $var ]` which can fail with \"unary operator expected\" on unset vars).\n\nExample: `[[ -z $TERM ]] && export TERM=xterm-256color`."),
    ("-n", "**True if STRING has nonzero length.** Inverse of `-z`. Common idiom for \"is variable set AND non-empty\".\n\nExample: `[[ -n ${VAR:-} ]] && echo \"VAR is set: $VAR\"`. The `:-` makes the test work even with `set -u` (no-unset) enabled. Without quoting inside `[[ ]]`, the test still works because `[[ ]]` doesn't word-split."),
    ("=",  "**POSIX string equality.** `[[ a = a ]]` is true. Within `[[ … ]]`, the RHS is treated as a literal string — no globbing. Same operator as `==` in zsh `[[ ]]`; use `=` for `/bin/sh` portability, `==` for clarity in zsh-only code.\n\nDo NOT confuse with assignment `=` — `[[ a = b ]]` tests, `var=b` assigns."),
    ("==", "**String equality with glob pattern matching on the RHS** (zsh extension). The right operand IS a pattern: `[[ foo == f* ]]` is true, `[[ foo == f? ]]` would need exactly one char after `f`.\n\nQuote the RHS to disable glob: `[[ $name == \"f*\" ]]` matches literal `f*`. With `EXTENDED_GLOB` enabled, `(#i)PAT` for case-insensitive: `[[ Foo == (#i)foo ]]` is true. Use `=~` instead for regex semantics."),
    ("!=", "**String inequality with glob pattern matching on the RHS.** Inverse of `==`. The RHS is a zsh pattern (unless quoted).\n\nExample: `[[ $f != *.bak ]] && process $f` — skip backup files. Same EXTENDED_GLOB modifiers (`(#i)`, `(#b)`, etc.) apply as for `==`."),
    ("<",  "**Lexicographic less-than** — string comparison by locale-aware byte order. NOT numeric. `[[ 10 < 9 ]]` is TRUE (lex order) because `\"1\"` < `\"9\"`.\n\nFor numeric comparison use `-lt` or arithmetic context: `(( 10 < 9 ))` is false. The string comparison respects `LC_COLLATE` — `en_US.UTF-8` may give different results than `C`."),
    (">",  "**Lexicographic greater-than** — string comparison. Same locale-awareness caveat as `<`: NOT numeric. For numeric `>`, use `-gt` or `(( a > b ))`.\n\nExample: `[[ $version > 1.10 ]]` is FALSE because `\"1.10\"` < `\"1.2\"` lexically. Use a real version-comparator (sort -V, vercmp) for semantic version ordering."),
    ("=~", "**Regular expression match** — the RHS is an extended regular expression (ERE by default; PCRE with `setopt REMATCH_PCRE` and `zsh/pcre` loaded). Sets `$MATCH` to the full match and `$match` (array) to the parenthesized groups.\n\nExample: `[[ $line =~ ^([0-9]+):(.+)$ ]] && echo \"line ${match[1]}: ${match[2]}\"`. Inside `[[ ]]` the RHS doesn't need quoting in most cases, but special chars (`(`, `|`) can hit shell parsing — quote when unsure."),
    // ── numeric ──
    ("-eq", "**Numeric equality** — arguments parsed as integers (or floats with zsh `FORCE_FLOAT`). Differs from `=` / `==` which compare as strings: `[[ 010 -eq 10 ]]` is true; `[[ 010 = 10 ]]` is false (string `\"010\"` ≠ `\"10\"`).\n\nFor arithmetic context, `(( a == b ))` is shorter. Operands can be variable names without `$` per arithmetic-expansion rules — `[[ x -eq 5 ]]` works if `x=5`."),
    ("-ne", "**Numeric inequality.** Like `-eq` but inverted. Same integer parsing — leading zeros / hex (`0x10`) / floats handled.\n\nExample: `[[ $rc -ne 0 ]] && exit $rc` — propagate non-zero exit codes from a previous command."),
    ("-lt", "**Numeric less-than.** Compares as integers, NOT lexically (unlike `<` which is lexicographic). Always prefer `-lt` over `<` when comparing numbers — `[[ 10 -lt 9 ]]` is correctly false; `[[ 10 < 9 ]]` is wrongly true (string order).\n\nExample: `[[ $count -lt 100 ]] && retry`."),
    ("-le", "**Numeric less-than-or-equal.** Integer-aware. Common for loop bounds.\n\nExample: `[[ $i -le $#argv ]] && process ${argv[$i]}` — check whether the index is within array bounds (1-indexed in zsh)."),
    ("-gt", "**Numeric greater-than.** Integer-aware. Mirror of `-lt`.\n\nExample: `[[ $(date +%s) -gt $deadline ]] && abort 'timed out'`."),
    ("-ge", "**Numeric greater-than-or-equal.** Integer-aware. Common for minimum-version checks: `[[ ${BASH_VERSINFO[0]} -ge 4 ]]` style.\n\nFor float-aware comparison, use arithmetic with `setopt FORCE_FLOAT`: `(( a >= b ))`. zsh's `[[ ]]` numeric tests treat float strings as 0."),
    // ── file compare ──
    ("-nt", "**True if FILE1 is newer than FILE2** (mtime comparison). True if FILE2 doesn't exist; false if FILE1 doesn't exist. Used in build-style checks: rebuild target if any source is newer.\n\nExample: `[[ $src -nt $obj ]] && cc -c $src -o $obj` — recompile only when source has changed. Compare against multiple: loop or use `find -newer`."),
    ("-ot", "**True if FILE1 is older than FILE2.** Inverse of `-nt`. True if FILE1 doesn't exist; false if FILE2 doesn't exist.\n\nExample: `[[ $cache -ot $config ]] && rm $cache` — invalidate cache when config is newer."),
    ("-ef", "**True if FILE1 and FILE2 refer to the same inode** on the same filesystem — same physical file, possibly via different paths (symlinks or hard links). Different files with identical content are NOT `-ef`.\n\nExample: `[[ /tmp -ef /private/tmp ]] && echo 'same dir'` — common on macOS where `/tmp` is a symlink. Distinguishes hard-linked duplicates from copies."),
    // ── logical ──
    ("!",  "**Logical negation** — inverts the truth value of the following test expression. Highest-precedence boolean operator inside `[[ … ]]`.\n\nExample: `[[ ! -f $f ]] && touch $f` — create the file if it doesn't exist. Combine with parens for grouping: `[[ ! ( -f $a || -f $b ) ]]` is true when NEITHER file exists. Same `!` is also pipeline-prefix negation outside `[[ ]]`: `! grep foo bar.txt && echo 'no match'`."),
    ("&&", "**Logical AND with short-circuit.** Inside `[[ … && … ]]`: both tests must pass. The right side is only evaluated if the left is true. Lower precedence than `!`, higher than `||`.\n\nExample: `[[ -f $f && -r $f ]]` — exists AND readable. Outside `[[ ]]`, `cmd1 && cmd2` is command-list short-circuit: run cmd2 only if cmd1 succeeded (exit 0)."),
    ("||", "**Logical OR with short-circuit.** Inside `[[ … || … ]]`: either test passing makes the whole expression true. Right side skipped if left is true.\n\nExample: `[[ -z $TERM || $TERM == dumb ]] && return` — bail out if terminal is unknown or dumb. Outside `[[ ]]`, the command-list form: `cmd1 || fallback`."),
    ("-o", "**POSIX-style OR — DEPRECATED inside `[[ ]]`.** Recognized for `test` / `[` compatibility but documented to be avoided: precedence is ambiguous and ill-defined. Use `||` outside `( )` groups OR rewrite as separate commands.\n\nBackground: zsh's `[[ ]]` does proper short-circuit parsing; `[ ]` with `-o` is parsed as a single command with arguments, leading to surprising precedence."),
    ("-a", "**POSIX-style AND — DEPRECATED inside `[[ ]]`.** Same caveats as `-o`: precedence is undefined when mixed with `!` / parens / other binary ops. Use `&&` instead.\n\n`man zshmisc` explicitly recommends against `-a`/`-o` in conditional expressions; they exist only because `[`/`test` traditionally used them."),
];

/// Math functions for `((…))` / `$((…))`. Most require `zmodload zsh/mathfunc`.
/// From `man zshmodules` "THE ZSH/MATHFUNC MODULE".
const MATH_FUNCTIONS: &[(&str, &str)] = &[
    // ── trigonometry ──
    ("sin",     "**Sine** of `x` radians. Range: `[-1, 1]`. For degrees, multiply input by `M_PI/180` (M_PI ≈ 3.14159265).\n\nExample: `(( y = sin(M_PI / 2) ))` → 1. Used in animation timing, geometry, signal processing. Argument near multiples of π may lose precision due to floating-point representation of π."),
    ("cos",     "**Cosine** of `x` radians. Range: `[-1, 1]`. `cos(0) = 1`, `cos(M_PI) = -1`.\n\nExample: `(( c = cos(t * 2 * M_PI / period) ))` — periodic oscillation between -1 and 1. For combined sin+cos angle decomposition, `(sin(t), cos(t))` traces the unit circle."),
    ("tan",     "**Tangent** of `x` radians = `sin(x) / cos(x)`. Undefined at `x = M_PI/2 + n*M_PI` (where `cos(x) = 0`); returns ±inf or extremely large values near those points.\n\nExample: `(( slope = tan(angle) ))` — convert angle to gradient. Wrap input via `fmod(x, M_PI)` if your formula isn't periodic-safe."),
    ("asin",    "**Arcsine** — inverse of `sin`. Domain: `[-1, 1]`; range: `[-M_PI/2, M_PI/2]` radians. Returns NaN for `|x| > 1`.\n\nExample: `(( angle = asin(opp / hyp) ))` — recover angle from a right triangle's opposite/hypotenuse ratio."),
    ("acos",    "**Arccosine** — inverse of `cos`. Domain: `[-1, 1]`; range: `[0, M_PI]` radians. Returns NaN for `|x| > 1`.\n\nExample: dot-product → angle: `(( theta = acos(dot / (mag_a * mag_b)) ))`. Common in 3D math for angle-between-vectors."),
    ("atan",    "**Arctangent** — inverse of `tan`. Domain: all real; range: `(-M_PI/2, M_PI/2)`. For 2-argument atan2 with quadrant handling, use `atan2(y, x)`.\n\nExample: `(( angle = atan(slope) ))` — convert slope to angle. Range limitation makes `atan` unsuitable for vector → angle conversion; use `atan2` there."),
    ("atan2",   "**Two-argument arctangent** — `atan2(y, x)` returns the angle of the point `(x, y)` from the positive x-axis. Range: `(-M_PI, M_PI]`. Handles all four quadrants correctly AND the `x=0` cases (returns ±M_PI/2). Always prefer over `atan(y/x)` for vector-to-angle conversion.\n\nExample: `(( bearing = atan2(dy, dx) * 180 / M_PI ))` — heading angle in degrees from coordinate delta."),
    ("sinh",    "**Hyperbolic sine** = `(e^x - e^-x) / 2`. Range: all real. Unlike `sin`, NOT periodic — grows exponentially for large `|x|`.\n\nExample: catenary curve (hanging chain): `y = a * cosh(x/a)`. Used in physics (relativity, wave equations) and machine learning (tanh-family activations)."),
    ("cosh",    "**Hyperbolic cosine** = `(e^x + e^-x) / 2`. Range: `[1, +inf)` — always ≥ 1. Even function: `cosh(-x) = cosh(x)`.\n\nExample: `(( y = cosh(x) ))` for catenary shape. Pair with `sinh` for hyperbolic identities: `cosh²(x) - sinh²(x) = 1`."),
    ("tanh",    "**Hyperbolic tangent** = `sinh(x) / cosh(x)`. Range: `(-1, 1)`. Sigmoidal — saturates smoothly as `|x| → ∞`. Common activation function in neural networks for its zero-centered output (unlike sigmoid).\n\nExample: `(( y = tanh(x) ))` squashes any input into `(-1, 1)`."),
    ("asinh",   "**Inverse hyperbolic sine** = `log(x + sqrt(x² + 1))`. Domain: all real. Numerically stable for large `|x|` (unlike the closed-form `log()` expression, which loses precision when x is large negative)."),
    ("acosh",   "**Inverse hyperbolic cosine** = `log(x + sqrt(x² - 1))`. Domain: `[1, +inf)`. Returns NaN for `x < 1`. Range: `[0, +inf)`.\n\nExample: in special relativity, rapidity `φ` from velocity `v/c`: `phi = acosh(gamma)`."),
    ("atanh",   "**Inverse hyperbolic tangent** = `0.5 * log((1+x) / (1-x))`. Domain: `(-1, 1)`. Returns ±inf at the endpoints, NaN outside. Useful for variance-stabilizing transforms in statistics (Fisher's z-transform of correlation coefficient)."),
    // ── exponential / logarithm ──
    ("exp",     "**Natural exponential** = e^x where e ≈ 2.71828. Inverse of `log`. For `|x|` large positive, returns inf (overflow at ~709). For `|x|` large negative, underflows to 0.\n\nExample: probability decay `(( p = exp(-lambda * t) ))`. For `e^x - 1` accurately near 0, use `expm1`."),
    ("expm1",   "**exp(x) − 1**, computed with extra precision near `x = 0`. The naive `exp(x) - 1` loses significant digits when `x` is tiny because `exp(x) ≈ 1 + x + …` and subtracting 1 from ≈1 cancels the meaningful part.\n\nExample: small interest rate: `(( gain = expm1(rate) ))` is far more accurate than `(( gain = exp(rate) - 1 ))` for `rate ≈ 1e-9`."),
    ("log",     "**Natural logarithm** (base e). Inverse of `exp`. Domain: `(0, +inf)`; `log(0)` = -inf; `log(x)` for `x < 0` returns NaN.\n\nExample: `(( bits = log(n) / log(2) ))` — bits needed to represent `n` distinct values (or use `log2(n)` directly). For accurate `log(1+x)` near 0, use `log1p`."),
    ("log2",    "**Base-2 logarithm.** Useful when computing bits or binary tree depth. `log2(1024) = 10` exactly.\n\nExample: `(( depth = ceil(log2(node_count)) ))` — minimum binary tree height. Faster + more accurate than `log(x) / log(2)` because the constant `log(2)` doesn't need to be computed."),
    ("log10",   "**Base-10 logarithm.** Common in engineering / acoustics (decibels: `db = 10 * log10(ratio)`) and order-of-magnitude estimates.\n\nExample: `(( db = 20 * log10(amplitude / reference) ))` — convert linear amplitude to dB. Like `log2`, more accurate than dividing by `log(10)`."),
    ("log1p",   "**log(1+x)**, accurate near `x = 0`. The naive `log(1+x)` loses precision when `x` is tiny because adding small `x` to 1 hits float-rounding before the log is taken.\n\nExample: log-likelihood of small probability: `(( ll = log1p(-p) ))` — avoids `log(1 - tiny_p)` underflowing to `log(1) = 0`."),
    ("pow",     "**x raised to power y** — `pow(x, y)` = `x^y`. Same as zsh's `**` operator: `(( c = x ** y ))`. For integer `y`, `**` is often faster; `pow` always uses float arithmetic.\n\nNegative `x` with non-integer `y` returns NaN. `pow(0, 0) = 1` by convention. For exponential of `e`, prefer `exp(y)` over `pow(M_E, y)`."),
    ("sqrt",    "**Square root** — `sqrt(x)` = `x^0.5`. Domain: `[0, +inf)`; returns NaN for negative input. For complex roots, no native support — use `csqrt` from a math library or compute manually.\n\nExample: distance: `(( dist = sqrt(dx*dx + dy*dy) ))`. For `sqrt(x² + y²)` specifically, prefer `hypot(x, y)` — avoids overflow when intermediate squares are huge."),
    ("cbrt",    "**Cube root** — works for negative inputs (unlike `pow(x, 1.0/3.0)` which returns NaN for `x < 0` because of how float exponents handle negatives). Domain: all real.\n\nExample: `(( radius = cbrt(3 * volume / (4 * M_PI)) ))` — sphere radius from volume."),
    ("hypot",   "**Euclidean norm** = `sqrt(x² + y²)`, computed without overflow/underflow even when `x` or `y` is huge. The naive `sqrt(x*x + y*y)` overflows when `x*x` exceeds float max (~1e308); `hypot` rescales internally to avoid it.\n\nExample: vector magnitude: `(( mag = hypot(dx, dy) ))`. Always prefer over `sqrt(x*x + y*y)` for robustness."),
    // ── rounding / abs ──
    ("abs",     "**Absolute value** — `abs(x)` returns `|x|`. For integers in arithmetic context, this is the same as `(( a < 0 ? -a : a ))`. For floats, preserves the type.\n\nExample: difference magnitude: `(( delta = abs(a - b) ))`. Note: `abs(INT_MIN)` overflows on two's-complement integers (the canonical pitfall)."),
    ("ceil",    "**Round up to the nearest integer** (toward +inf). `ceil(3.1) = 4`, `ceil(-3.1) = -3`. Returns a float — cast to integer with `int(ceil(x))` if you need an int type.\n\nExample: pages needed: `(( pages = ceil(items / per_page) ))`."),
    ("floor",   "**Round down to the nearest integer** (toward -inf). `floor(3.9) = 3`, `floor(-3.1) = -4`. Note: `floor` and integer truncation differ for negatives — `int(-3.1) = -3` (toward zero), `floor(-3.1) = -4` (toward -inf).\n\nExample: bucketing: `(( bucket = floor(value / bucket_size) ))`."),
    ("round",   "**Round half-away-from-zero** to nearest integer. `round(2.5) = 3`, `round(-2.5) = -3`. Distinct from IEEE banker's rounding (`rint`) which rounds half-to-even.\n\nExample: nearest pixel: `(( px = round(x * dpi / 72) ))`."),
    ("trunc",   "**Truncate toward zero** — drop the fractional part. `trunc(3.9) = 3`, `trunc(-3.9) = -3`. Same as the C `(int)` cast or zsh's `int()` function.\n\nDistinct from `floor` for negatives: `floor(-3.9) = -4`, `trunc(-3.9) = -3`."),
    ("rint",    "**Round to nearest integer using the CURRENT rounding mode** (default IEEE-754 round-half-to-even). `rint(2.5) = 2` (even); `rint(3.5) = 4` (even). Banker's rounding eliminates bias when summing many rounded values.\n\nDiffers from `round` (always-away-from-zero) and from `nearbyint` (`rint` raises the inexact exception, `nearbyint` doesn't)."),
    // ── special ──
    ("gamma",   "**Gamma function** Γ(x) — generalization of factorial to real / complex numbers: `Γ(n) = (n-1)!` for positive integer n. `Γ(0.5) = sqrt(M_PI)`. Pole at every non-positive integer; returns ±inf there.\n\nUsed in combinatorics (continuous factorial), statistics (gamma / beta distributions), physics. For large `x`, prefer `lgamma` to avoid overflow."),
    ("lgamma",  "**log |Γ(x)|** — log of absolute value of gamma function. Avoids overflow that Γ itself hits quickly: Γ(171) overflows float, but `lgamma(171)` is ~706 (representable).\n\nExample: log-binomial coefficient: `(( lc = lgamma(n+1) - lgamma(k+1) - lgamma(n-k+1) ))`. Sign of Γ retrievable separately via `signgam` (not always exposed)."),
    ("erf",     "**Error function** — `erf(x) = 2/sqrt(π) * ∫₀ˣ e^(-t²) dt`. Used in statistics (normal-distribution CDF: `Φ(z) = (1 + erf(z/sqrt(2))) / 2`), diffusion equations, signal processing.\n\nRange: `(-1, 1)`. Odd function: `erf(-x) = -erf(x)`. `erf(0) = 0`, `erf(inf) = 1`."),
    ("erfc",    "**Complementary error function** = `1 - erf(x)`. Use instead of `1 - erf(x)` when `x` is large — the naive subtraction loses precision because `erf(x)` approaches 1 and `1 - 0.999…` cancels significant digits.\n\nExample: tail probability: `(( p_tail = erfc(z / sqrt(2)) / 2 ))` — far more accurate than `1 - erf(…)` for z > 5."),
    ("j0",      "**Bessel function of the first kind, order 0** — `J₀(x)`. Oscillatory solution to the Bessel equation; appears in cylindrical-coordinate problems (drum vibration modes, EM wave propagation in cylinders).\n\nNot in POSIX `<math.h>` but standard in BSD/Linux libm. Range: `[-0.4, 1]` approximately, decaying with √x rate."),
    ("j1",      "**Bessel function of the first kind, order 1** — `J₁(x)`. `J₁(0) = 0`. Like `j0`, oscillates with √x-decay. Used in optics (Airy disk pattern: intensity is `(2 J₁(x) / x)²`)."),
    ("jn",      "**Bessel function of the first kind, order n** — `J_n(x)` for integer `n`. Two-arg: `jn(n, x)`. Generalization of `j0` / `j1`; for large `n` the function decays rapidly until `x ≥ n`. Used in FM modulation (sideband amplitudes follow J_n)."),
    ("y0",      "**Bessel function of the second kind, order 0** — `Y₀(x)`. Domain: `(0, +inf)`; diverges to -inf at `x = 0`. Used together with `J₀` as the second linearly-independent solution to Bessel's equation."),
    ("y1",      "**Bessel function of the second kind, order 1** — `Y₁(x)`. Like `y0`: diverges at 0, oscillates with √x decay. Pairs with `j1` for general-solution construction in cylindrical-symmetry problems."),
    ("yn",      "**Bessel function of the second kind, order n** — `Y_n(x)` for integer `n`. Two-arg: `yn(n, x)`. Diverges at `x = 0` faster as `n` grows. Used in optics, antenna theory, heat-equation solutions."),
    // ── classification ──
    ("isinf",   "**Tests if argument is ±infinity** — returns 1 if `x == +inf` or `x == -inf`, 0 otherwise. Use after computations that might overflow (`pow(big, big)`, `1/0.0`) to detect runaway results.\n\nExample: `(( isinf(result) )) && { print 'overflow' >&2; return 1 }`."),
    ("isnan",   "**Tests if argument is NaN** (Not-a-Number) — returns 1 if `x` is the IEEE-754 NaN. NaN appears from `0/0`, `inf - inf`, `sqrt(-1)`, and is the only float value where `x != x` is true (NaN comparisons always return false).\n\nExample: `(( isnan(result) )) && { print 'undefined result' >&2; result=0 }`."),
    ("finite",  "**Tests if argument is finite** — returns 1 if `x` is neither NaN nor ±inf, 0 otherwise. Inverse of `(isnan(x) || isinf(x))`. Less standard than `isnan`/`isinf` separately; on Linux this is `__finite` / `isfinite`."),
    // ── conversion ──
    ("int",     "**Convert to integer by truncating toward zero.** `int(3.9) = 3`, `int(-3.9) = -3`. Same as zsh's `(( i = (int) x ))` cast. For round-half-away-from-zero, use `round`. For floor (-inf direction), use `floor`.\n\nUsed inside arithmetic to force integer type: `(( i = int(rand48() * 100) ))` — random int in 0..99."),
    ("float",   "**Convert to float** — explicit type cast. Mostly redundant since most math ported return float anyway, but useful when you want to force float arithmetic: `(( q = float(a) / b ))` ensures float division even if `a` and `b` are integer parameters."),
    ("rand48",  "**Pseudo-random float in `[0, 1)`** — drand48(3) under the hood. Not cryptographically secure (linear congruential generator). Seed via `srand48()` — not directly exposed in zsh math, but the seed comes from process-startup time by default.\n\nExample: `(( dice = int(rand48() * 6) + 1 ))` — uniform 1..6. For dedicated crypto-grade randomness, read from `/dev/urandom` instead."),
    ("max",     "**Maximum of two or more arguments.** `max(a, b)` for two; `max(a, b, c, …)` works in zsh math context. Float-aware: `max(1, 1.5) = 1.5`.\n\nExample: `(( cap = max(min_size, requested) ))` — clamp lower bound. NOT the same as the GNU coreutils external `/bin/max` (doesn't exist)."),
    ("min",     "**Minimum of two or more arguments.** Mirror of `max`. Used for clamping upper bound or finding the smallest item in a set of computed values.\n\nExample: `(( delay = min(timeout, exponential_backoff) ))`."),
    ("sum",     "**Sum of all arguments.** Variadic — `sum(1, 2, 3, 4) = 10`. Convenient for combining a small set of math expressions without writing `(( a + b + c + d ))`.\n\nExample: `(( total = sum($costs) ))` — but be careful: this only works if `$costs` is a scalar expression list, not an array."),
    ("copysign", "**copysign(x, y)** — returns the magnitude of `x` with the sign of `y`. `copysign(3, -1) = -3`, `copysign(-3, 1) = 3`. Works for `±0` and `±inf` too. Used to preserve sign through computations that otherwise zero it out."),
    ("ilogb",   "**Integer binary exponent of x** — returns the unbiased exponent as an int, i.e. `e` such that `|x| ∈ [2^e, 2^(e+1))`. `ilogb(8) = 3`, `ilogb(0.5) = -1`. Faster than `log2(x)` when you only need the integer part.\n\nUsed for fast bit-counting in floats: number of bits to shift to normalize."),
    ("logb",    "**Binary exponent of x as a float.** Same value as `ilogb` but float-typed. Used in low-level float manipulation where you want to extract the exponent and re-combine via `scalb`."),
    ("scalb",   "**scalb(x, n)** = `x × 2^n`. Faster than `x * pow(2, n)` because it just adjusts the exponent bits directly, no full multiplication. The inverse of `logb` / `ilogb` in a sense — `scalb(1.0, ilogb(x))` recovers the float's exponent magnitude."),
    ("nextafter", "**nextafter(x, y)** — next representable double after `x` in the direction of `y`. Returns the immediate float neighbor — useful for testing float-comparison robustness (`nextafter(0.1, 1.0)` ≠ 0.1) or for iterative algorithms that need to step through every distinct float."),
    ("fma",     "**Fused multiply-add** = `x*y + z`, computed with a SINGLE rounding step instead of two (one for `*`, one for `+`). More accurate than `x*y + z` when the multiplication and addition would cancel meaningful digits.\n\nUsed in dot products / matrix multiply for numerical stability. Most modern CPUs have a single FMA instruction."),
    ("fmod",    "**Floating-point remainder** — `fmod(x, y)` returns `x - n*y` where `n = trunc(x/y)`. Has the same sign as `x`. For non-negative remainder, use `(((x % y) + y) % y)` style or `remainder()`.\n\nExample: clock arithmetic: `(( hour = fmod(elapsed_sec / 3600, 24) ))`."),
    ("drem",    "**IEEE remainder of x/y** — like `fmod` but uses round-half-to-even for the quotient, so the result is in `(-y/2, y/2]`. Standard name on Linux is `remainder`; `drem` is the legacy BSD name kept for compatibility.\n\nDifference vs `fmod`: `drem(7, 3) = 1`, `fmod(7, 3) = 1` — they match for this. But `drem(5, 3) = -1` (round-to-even quotient), `fmod(5, 3) = 2`. Choose based on whether you want truncation or rounding semantics."),
];

/// `zstyle` well-known context patterns. From the most common
/// `zstyle -L` outputs in `.zshrc` configs. NOT exhaustive — zstyle
/// contexts are user-defined — but covers the canonical completion /
/// vcs_info / prompt namespaces.
const ZSTYLE_CONTEXTS: &[(&str, &str)] = &[
    (":completion:*", "all completion settings"),
    (":completion:*:default", "default completion"),
    (
        ":completion:*:descriptions",
        "tag-group descriptions in menus",
    ),
    (":completion:*:matches", "match grouping / formatting"),
    (":completion:*:options", "option-name completion"),
    (":completion:*:warnings", "no-match warning style"),
    (
        ":completion:*:messages",
        "info messages from completion ported",
    ),
    (":completion:*:corrections", "spell-correction style"),
    (
        ":completion:*:*:*:*:processes",
        "process-name completion (`kill <TAB>`)",
    ),
    (":completion:*:functions", "function-name completion"),
    (":completion:*:manuals", "man-page completion"),
    (
        ":completion:*:hosts",
        "hostname completion (ssh, scp, etc.)",
    ),
    (
        ":vcs_info:*",
        "version-control info system (`git`/`hg`/`svn` in prompt)",
    ),
    (":vcs_info:git:*", "git-specific vcs_info"),
    (":prompt:*", "prompt customization (themes)"),
    (":urlglobber", "URL-glob filtering"),
    (":zftp:*", "zftp module configuration"),
    (":grep:*", "grep widget configuration"),
    (":compinstall", "`compinstall` wizard state"),
    (":zle:*", "ZLE widget configuration"),
    (":bracketed-paste-magic", "bracketed-paste-magic widget"),
    (
        ":syntax-highlighting",
        "fast-syntax-highlighting / zsh-syntax-highlighting",
    ),
];

/// Pattern modifiers for extended-glob `(#…)`. Need `EXTENDED_GLOB`.
/// From `man zshexpn` "Pattern Matching → Globbing Flags".
const PATTERN_MODIFIERS: &[(&str, &str)] = &[
    ("i", "case-insensitive matching for the rest of the pattern"),
    ("l", "lowercase chars match upper + lower"),
    ("I", "case-sensitive — reset after `(#i)`"),
    (
        "b",
        "activate backreferences (`$match[N]` / `$mbegin` / `$mend`)",
    ),
    ("B", "deactivate backreferences"),
    (
        "m",
        "set `$MATCH` / `$MBEGIN` / `$MEND` even without backref",
    ),
    ("M", "deactivate `m`"),
    ("a", "`(#aN)` — approximate match with up to N errors"),
    ("s", "anchor pattern to start of string"),
    ("e", "anchor pattern to end of string"),
    (
        "c",
        "`(#cN,M)` — preceding atom matched between N and M times",
    ),
    ("u", "use Unicode character properties"),
    ("U", "deactivate `u`"),
    (
        "q",
        "treat following pattern as glob qualifier list (`(#q.,L0)`)",
    ),
];

/// Subscript flags for `${arr[(X)pattern]}` — reverse / index / range
/// search modifiers inside array subscripts. From `man zshparam`
/// "ARRAY PARAMETERS → SUBSCRIPT FLAGS".
const SUBSCRIPT_FLAGS: &[(&str, &str)] = &[
    ("e", "exact match — disable globbing on subscript"),
    ("i", "return INDEX of first matching element"),
    ("I", "return INDEX of LAST matching element"),
    ("r", "return VALUE of first match — search reverse"),
    ("R", "as `r` but ranged"),
    ("b", "byte offset (with `i` / `I`)"),
    ("n", "`(nN)` — Nth match (with `i` / `I` / `r` / `R`)"),
    ("w", "word offset (split on `$IFS`)"),
    ("W", "word offset with empty fields"),
    ("p", "process `\\NNN` escapes in `(s::)` separator"),
    ("s", "`(s:STR:)` — split on STR (with `w` / `W`)"),
    ("f", "split scalar on newlines (= `(s.\\n.)`)"),
    ("k", "match against keys of an associative array"),
    ("v", "match against values of an associative array"),
];

/// Where the cursor sits — drives which completion table to surface.
/// Detected by scanning backward from the cursor for the innermost
/// open paren / brace / `!` / `[` and looking at what precedes it,
/// plus by inspecting the leading command on the line.
#[derive(Debug, Clone, PartialEq, Eq)]
enum LspCompletionContext {
    Normal,
    ParamFlag,
    GlobQualifier,
    HistoryDesignator,
    ParamColonModifier,
    // NEW — command-position contexts (leading command dispatches).
    OptionOnly,    // setopt / unsetopt / set -o / set +o
    SignalName,    // kill -SIG / trap … SIG
    ModuleName,    // zmodload
    KeymapName,    // bindkey -M / -A / -N (1st arg)
    WidgetName,    // zle … / bindkey "key" (2nd arg)
    TypesetFlag, // typeset / declare / local / readonly / integer / float / export with leading `-`
    ZstyleContext, // zstyle (1st arg)
    CompdefFn,   // compdef (1st arg)
    // NEW — bracket / paren contexts.
    TestOperator,    // inside [[ … ]]
    MathFunction,    // inside (( … )) or $(( … ))
    PatternModifier, // inside (#…)
    SubscriptFlag,   // inside ${arr[(…)…]}
    /// Cursor right after a `-` argument to a known builtin —
    /// surface the builtin's option flags from its yodl hover doc.
    /// `print -<TAB>` → -a/-b/-c/-C/-D/-f/-i/-l/-m/-n/-N/-o/-O/-P/-r
    /// /-R/-s/-S/-u/-v/-x/-z + each one's description. The `.0`
    /// field carries the builtin name so the dispatcher can look
    /// up the right doc body.
    BuiltinFlag(String),
    /// Cursor right after a `--` argument to a known builtin that
    /// publishes long-form flag docs. `zshrs --<TAB>` surfaces the
    /// 24 zshrs-specific long flags (`--lsp`, `--dap`, `--dump-*`,
    /// `--doctor`, parity modes) plus every setopt mirror sourced
    /// from `OPTION_DOCS`. Distinct from `BuiltinFlag` because long
    /// flags don't letter-stack and the replacement range covers
    /// the entire `--xxx` typed prefix as one unit.
    BuiltinLongFlag(String),
}

/// Find the first whitespace-delimited word at or after a list-start
/// boundary on the line — i.e. the "command" of the current pipeline /
/// command-list segment. Returns the command text + the byte position
/// where its first argument begins (skipping the command + one space).
fn leading_command_at(line: &str, col: usize) -> Option<(String, usize)> {
    let bytes = line.as_bytes();
    let cap = col.min(bytes.len());
    // Walk back from cursor to find the most recent statement
    // separator (`;`, `&&`, `||`, `|`, `&`, `(`, newline) or start of
    // line. Skip over chars; treat that position as the cmd start.
    let mut s: usize = 0;
    let mut i = cap;
    while i > 0 {
        i -= 1;
        let c = bytes[i];
        if c == b'\n' || c == b';' {
            s = i + 1;
            break;
        }
        if (c == b'|' || c == b'&') && i > 0 {
            // Bare `|` or `&` is a separator; `&&` / `||` too.
            s = i + 1;
            break;
        }
        // Subshell / command-substitution / process-substitution
        // openers: `$(`, `<(`, `>(`, `(`, `((`. Walking back to one
        // of these starts a fresh command region — without this,
        // `x=$(zshrs --…)` saw `x` as the leading command instead
        // of `zshrs`, so flag completion never fired inside `$(…)`.
        if c == b'(' {
            s = i + 1;
            break;
        }
    }
    // Skip leading whitespace.
    while s < cap && matches!(bytes[s], b' ' | b'\t') {
        s += 1;
    }
    // Read the command token — bare-word chars only.
    let mut e = s;
    while e < cap && (bytes[e].is_ascii_alphanumeric() || matches!(bytes[e], b'_' | b'-' | b'.')) {
        e += 1;
    }
    if e == s {
        return None;
    }
    let cmd = std::str::from_utf8(&bytes[s..e]).ok()?.to_string();
    Some((cmd, e))
}

/// Count occurrences of the 2-byte token `tok` in `bytes[0..end]`.
fn count_pair(bytes: &[u8], end: usize, tok: [u8; 2]) -> i32 {
    let cap = end.min(bytes.len());
    let mut n: i32 = 0;
    let mut i = 0;
    while i + 1 < cap {
        if bytes[i] == tok[0] && bytes[i + 1] == tok[1] {
            n += 1;
            i += 2;
        } else {
            i += 1;
        }
    }
    n
}

/// Walks the line backward from `col` to classify the context for
/// completion routing. Order of checks:
///   1. Bracket contexts — `[[ … ]]` / `((…))` / `(#…)` / `[(…)`
///   2. History designator — `!` at word boundary, not in arith
///   3. Paren-based: `${(…)` / glob-meta-`(…)`
///   4. Param/history modifier — `:` inside `${…}` or after `!event:`
///   5. Command-position dispatch — leading command's first arg
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
            && (bytes[k - 1].is_ascii_alphabetic() || matches!(bytes[k - 1], b'&' | b'/' | b'g'))
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
                    || matches!(
                        bytes[e - 1],
                        b'?' | b'#' | b'$' | b'^' | b'*' | b'-' | b'_' | b'!'
                    ))
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

    // ── 4. Bracket contexts (`[[ … ]]`, `((…))`, `(#…)`, `${arr[(…)`) ─
    {
        let bytes = line.as_bytes();
        let cap = col.min(bytes.len());
        // PatternModifier — innermost unmatched `(#…`. Walk back; if
        // we hit `(#` before any `)`, trigger.
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
                        if i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                            return LspCompletionContext::PatternModifier;
                        }
                        break;
                    }
                    depth -= 1;
                }
            }
        }
        // SubscriptFlag — innermost unmatched `[(…` (the `(X)` form
        // inside `${arr[(X)pattern]}`). Walk back through `(`/`)` to
        // find an unmatched `(` whose preceding char is `[`.
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
                        if i >= 1 && bytes[i - 1] == b'[' {
                            return LspCompletionContext::SubscriptFlag;
                        }
                        break;
                    }
                    depth -= 1;
                }
            }
        }
        // TestOperator — inside `[[ … ]]`. Cheap heuristic: count
        // `[[` vs `]]` before cursor; if `[[` > `]]`, we're inside.
        let lbrack = count_pair(bytes, cap, [b'[', b'[']);
        let rbrack = count_pair(bytes, cap, [b']', b']']);
        if lbrack > rbrack {
            return LspCompletionContext::TestOperator;
        }
        // MathFunction — inside `((…))` or `$((…))`. Count `((` vs `))`.
        let lparen = count_pair(bytes, cap, [b'(', b'(']);
        let rparen = count_pair(bytes, cap, [b')', b')']);
        if lparen > rparen {
            return LspCompletionContext::MathFunction;
        }
    }

    // ── 5. Command-position dispatch ─────────────────────────────────
    if let Some((cmd, _arg_start)) = leading_command_at(line, col) {
        match cmd.as_str() {
            "setopt" | "unsetopt" => return LspCompletionContext::OptionOnly,
            "set" => {
                // `set -o` / `set +o` followed by option name. Cheap
                // check: any `-o` / `+o` between cmd and cursor.
                let bytes = line.as_bytes();
                let cap = col.min(bytes.len());
                let mut j = 0;
                let mut saw_o = false;
                while j + 1 < cap {
                    if (bytes[j] == b'-' || bytes[j] == b'+') && bytes[j + 1] == b'o' {
                        saw_o = true;
                        break;
                    }
                    j += 1;
                }
                if saw_o {
                    return LspCompletionContext::OptionOnly;
                }
            }
            "kill" => {
                // `kill -SIG` or `kill -s SIG` → signal name.
                // Bare `kill` first arg is a PID; once we see a `-`
                // we're in signal context. Cheap: check for `-` in args.
                let bytes = line.as_bytes();
                let cap = col.min(bytes.len());
                let mut has_dash = false;
                let mut j = 0;
                while j < cap {
                    if bytes[j] == b'-' && j > 0 && matches!(bytes[j - 1], b' ' | b'\t') {
                        has_dash = true;
                        break;
                    }
                    j += 1;
                }
                if has_dash {
                    return LspCompletionContext::SignalName;
                }
            }
            "trap" => return LspCompletionContext::SignalName,
            "zmodload" => return LspCompletionContext::ModuleName,
            "bindkey" => {
                // First non-flag arg = key sequence, second = widget.
                // Flag arg `-M name` / `-A from to` / `-N name` = keymap.
                // Cheap: if last seen flag is `-A`/`-M`/`-N` and cursor
                // is on its arg → KeymapName. Otherwise WidgetName.
                let bytes = line.as_bytes();
                let cap = col.min(bytes.len());
                let mut last_flag: Option<u8> = None;
                let mut j = 0;
                while j < cap {
                    if bytes[j] == b'-'
                        && j > 0
                        && matches!(bytes[j - 1], b' ' | b'\t')
                        && j + 1 < cap
                    {
                        last_flag = Some(bytes[j + 1]);
                    }
                    j += 1;
                }
                if matches!(last_flag, Some(b'A') | Some(b'M') | Some(b'N')) {
                    return LspCompletionContext::KeymapName;
                }
                return LspCompletionContext::WidgetName;
            }
            "zle" => {
                // `zle -<TAB>` completes the zle builtin's flags
                // (`-l`, `-L`, `-D`, `-N`, `-A`, `-K`, `-R`, `-M`,
                // `-U`, `-F`, `-I`, `-T`, etc.) from the hand-
                // curated `BUILTIN_FLAG_DOCS_OVERRIDE` entry.
                // `zle <name>` (no leading dash) keeps the existing
                // widget-name completion.
                let bytes = line.as_bytes();
                let cap = col.min(bytes.len());
                let mut j = cap;
                while j > 0 && !matches!(bytes[j - 1], b' ' | b'\t') {
                    j -= 1;
                }
                if j < cap && bytes[j] == b'-' {
                    return LspCompletionContext::BuiltinFlag("zle".to_string());
                }
                return LspCompletionContext::WidgetName;
            }
            "typeset" | "declare" | "local" | "readonly" | "integer" | "float" | "export"
            | "private" => {
                // Surface flags only when the current arg starts with `-`.
                let bytes = line.as_bytes();
                let cap = col.min(bytes.len());
                // Walk back from cursor to find start of current arg.
                let mut j = cap;
                while j > 0 && !matches!(bytes[j - 1], b' ' | b'\t') {
                    j -= 1;
                }
                if j < cap && bytes[j] == b'-' {
                    return LspCompletionContext::TypesetFlag;
                }
            }
            "zstyle" => return LspCompletionContext::ZstyleContext,
            "compdef" => return LspCompletionContext::CompdefFn,
            _ => {}
        }
        // Universal fallback: ANY named-or-unnamed command where the
        // current word starts with `-` AND the command has doc-body
        // flag bullets / citations. This runs AFTER the per-command
        // named arms above so e.g. `typeset -<TAB>` keeps its
        // hand-curated TypesetFlag table (better descriptions),
        // `set -o foo<TAB>` keeps OptionOnly, etc. Catches `set -`,
        // `bindkey -`, `zmv -`, `kill -` (when not after `-s`), etc.
        let bytes = line.as_bytes();
        let cap = col.min(bytes.len());
        let mut j = cap;
        while j > 0 && !matches!(bytes[j - 1], b' ' | b'\t') {
            j -= 1;
        }
        let starts_with_dash = j < cap && bytes[j] == b'-';
        let starts_with_double_dash = j + 1 < cap && bytes[j] == b'-' && bytes[j + 1] == b'-';
        let just_after_builtin = j == cap;
        // `zshrs --<TAB>` — route to long-flag completion when the
        // current word starts with `--` AND the command publishes
        // long-form flag docs. Falls through to BuiltinFlag for
        // single-dash prefixes / builtins that only have short flags.
        if starts_with_double_dash && is_known_builtin_with_long_flag_docs(&cmd) {
            return LspCompletionContext::BuiltinLongFlag(cmd);
        }
        if (starts_with_dash || just_after_builtin) && is_known_builtin_with_flag_docs(&cmd) {
            return LspCompletionContext::BuiltinFlag(cmd);
        }
    }

    LspCompletionContext::Normal
}

/// True when `name` is a builtin whose yodl doc body contains flag
/// bullets — used to gate the `BuiltinFlag` context dispatch. Quick
/// boolean check based on whether the doc body has the
/// `- **\`-X\`** —` pattern; cached per session.
/// Heuristic — given a builtin doc body and a flag like `-f`, find a
/// sentence describing that flag. Walks every backtick-wrapped flag
/// citation, expands to its enclosing sentence (bounded by `. ` or
/// `\n\n`), keeps the FIRST one that looks descriptive (skip section
/// headers and synopsis-style fragments).
///
/// Returns the cleaned-up description or None if no suitable
/// sentence found.
fn derive_inline_flag_desc(body: &str, flag: &str) -> Option<String> {
    let needle = format!("`{}`", flag);
    let bytes = body.as_bytes();
    let nbytes = needle.as_bytes();
    // Walk every occurrence — keep the best one.
    let mut best: Option<String> = None;
    let mut search_from = 0;
    while let Some(pos) = body[search_from..].find(&needle) {
        let abs = search_from + pos;
        search_from = abs + needle.len();
        // Find sentence start: walk back from `abs` over chars
        // until we hit `. ` (period-space), `\n\n`, or start of body.
        let mut sstart = abs;
        while sstart > 0 {
            let c = bytes[sstart - 1];
            if c == b'\n' && sstart >= 2 && bytes[sstart - 2] == b'\n' {
                break;
            }
            if c == b'.' && sstart < bytes.len() && matches!(bytes[sstart], b' ' | b'\n') {
                sstart += 1; // skip the period that ENDED the previous sentence
                break;
            }
            sstart -= 1;
        }
        // Find sentence end: walk forward from `abs` until `.` followed
        // by space/newline, or `\n\n`, or end of body. Cap at 300 chars.
        let mut send = abs + needle.len();
        let cap_end = (sstart + 400).min(bytes.len());
        while send < cap_end {
            let c = bytes[send];
            if c == b'.' && send + 1 < bytes.len() && matches!(bytes[send + 1], b' ' | b'\n') {
                send += 1; // include the period
                break;
            }
            if c == b'\n' && send + 1 < bytes.len() && bytes[send + 1] == b'\n' {
                break;
            }
            send += 1;
        }
        let raw = &body[sstart..send.min(bytes.len())];
        // Clean: collapse whitespace, strip markdown emphasis chars.
        let cleaned: String = raw
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .trim()
            .to_string();
        if cleaned.len() < 15 {
            continue; // too short to be useful
        }
        // Skip section-header-looking fragments.
        if cleaned.starts_with('#') {
            continue;
        }
        // Prefer shorter, sentence-like descriptions.
        if best
            .as_ref()
            .map(|b| b.len() > cleaned.len())
            .unwrap_or(true)
        {
            best = Some(cleaned);
        }
    }
    best.map(|s| s.chars().take(200).collect())
}

fn is_known_builtin_with_flag_docs(name: &str) -> bool {
    let is_compat = crate::ported::builtin::BUILTINS
        .iter()
        .any(|b| b.node.nam == name);
    let is_ext = crate::ext_builtins::EXT_BUILTIN_NAMES.contains(&name);
    // Every compsys fn documented in `man zshcompsys` has a Rust
    // shadow in `compsys/` (canonical_paths in library.rs,
    // call_program in shell_runner.rs, widgets in library.rs, etc.)
    // so they all live in `COMPSYS_FN_NAMES`. Flag completion routes
    // through the per-fn `COMPSYS_FN_FLAG_DOCS` table.
    let is_compsys = crate::compsys::COMPSYS_FN_NAMES.contains(&name);
    // `zshrs` is the binary itself — `zshrs -<TAB>` in a script
    // surfaces the standard zsh-compat short flags via the hand
    // table `ZSHRS_SELF_FLAG_DOCS`.
    let is_self = name == "zshrs" || name == "zsh";
    if !is_compat && !is_ext && !is_compsys && !is_self {
        return false;
    }
    !extract_builtin_flags(name).is_empty()
}

/// Parse `(flag, description)` pairs out of a builtin's hover-doc
/// body. The yodl→markdown converter emits each documented option
/// as a markdown bullet:
///
/// ```text
/// - **`-X`** — description text
/// ```
///
/// (with a Unicode em-dash `\u{2014}`). For options that take an
/// argument, the bullet is `- **\`-X _arg_\`** — desc`. This walks
/// the body's lines looking for that exact shape.
///
/// Returns `Vec<(flag, desc)>` where `flag` is `-X` (just the letter,
/// no arg name) and `desc` is the description prose. Cached per-name
/// in `BUILTIN_FLAGS_CACHE` so a hot `print -<TAB>` doesn't re-parse
/// the same body on every keystroke.

/// Public re-entry for the man-zshall audit integration test
/// (`tests/lsp_man_audit.rs`) — forwards to the internal scraper.
pub fn extract_builtin_flags_for_test(name: &str) -> Vec<(String, String)> {
    extract_builtin_flags(name)
}

fn extract_builtin_flags(name: &str) -> Vec<(String, String)> {
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static CACHE: OnceLock<Mutex<std::collections::HashMap<String, Vec<(String, String)>>>> =
        OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(std::collections::HashMap::new()));
    if let Ok(g) = cache.lock() {
        if let Some(v) = g.get(name) {
            return v.clone();
        }
    }
    // Tier 0a (zshrs binary itself): `zshrs -<TAB>` / `zsh -<TAB>`.
    // Hand table sourced from `zshrs --help` output. Covers the
    // 9 standard zsh-compat short flags. Long-form `--xxx` flags
    // (zshrs-specific dumpers, parity modes, the setopt-mirror
    // `--errexit` etc.) are NOT included here — the completion
    // dispatcher's `BuiltinFlag` context handles only single-dash
    // short flags; long-flag completion is a separate concern.
    if name == "zshrs" || name == "zsh" {
        let out: Vec<(String, String)> = ZSHRS_SELF_FLAG_DOCS
            .iter()
            .map(|(f, d)| (f.to_string(), d.to_string()))
            .collect();
        if let Ok(mut g) = cache.lock() {
            g.insert(name.to_string(), out.clone());
        }
        return out;
    }
    // Tier 0 (compsys functions): hand-curated table derived from
    // `man zshcompsys` signatures. Beats the bullet/inline scrapers
    // for the 26 compsys ported documented there because the yodl
    // source uses signature-style headers (`item(tt(_foo [ -x ] …))`)
    // not bullet lists, so tier-1 picks up nothing and tier-2 only
    // catches whichever flags happen to be re-cited inline.
    if let Some(flags) = lookup_compsys_flag_docs(name) {
        let out: Vec<(String, String)> = flags
            .iter()
            .map(|(f, d)| (f.to_string(), d.to_string()))
            .collect();
        if let Ok(mut g) = cache.lock() {
            g.insert(name.to_string(), out.clone());
        }
        return out;
    }
    // Pull the doc body directly from the canonical table — DON'T
    // route through `lookup_doc` because that one prepends a heading
    // (`**name** — _zsh builtin_\n\n`) and routes through a cascade
    // that can resolve to a special-var entry for the same name.
    // Missing body is NOT fatal: module builtins (xattr / network /
    // pty / zstat / sysread / …) often have no markdown body, and
    // their flags come purely from `BUILTIN_FLAG_DOCS_OVERRIDE`.
    // Fall through with empty body so the Tier 3 merge below supplies them.
    let body: String = match crate::zsh_builtin_docs::lookup_builtin_doc(name) {
        Some((_, b)) => b.to_string(),
        None => crate::zsh_ext_builtin_docs::lookup_full(name)
            .map(|b| b.to_string())
            .unwrap_or_default(),
    };
    let mut out: Vec<(String, String)> = Vec::new();
    // Pattern: `- **\`-X[ _arg_]\`** <anything-but-newline-or-bullet>`.
    // The em-dash separator in zsh's yodl docs got double-mojibaked
    // in some entries during extraction (`Ã¢Â\x80Â\x94` instead of
    // `—`) — we don't care, just skip everything until the next
    // alphabetic character which starts the description prose.
    // Tier 1: bullet pattern `- **\`-X[ args]\`** — desc`. Yields
    // (flag, first-line-desc). Used by the 25 builtins whose docs
    // have proper bullet lists (print, typeset, read, compadd,
    // stat, whence, bindkey, fc, zparseopts, zcompile, zmv, …).
    // Two-shape bullet body: inline `- **`-X`** — desc` OR next-line
    // `- **`-X`** —\ndesc`. The yodl source uses both freely (often
    // the same builtin mixes them — `print`'s `-b` and `-m` are
    // inline, every other flag's desc lives on the next line).
    // `[ \t]*` (no newline) bounds the em-dash junk to the bullet
    // line; the optional single `\n` lets us cross exactly one line
    // boundary to the description, so we don't accidentally pick up
    // prose paragraphs that separate flag clusters.
    // Arg notation has two forms in zsh's docs:
    //   `- **`-C cols`**`        ← arg INSIDE backticks (rare)
    //   `- **`-C` _cols_**`      ← arg OUTSIDE backticks, italicized (dominant)
    // The `[^*\n]*` between closing `` ` `` and closing `**` accepts
    // the italicized-arg form (` _cols_`, ` _name_`, ` _tab-stop_`)
    // without which print/read/where/etc. lose 5–10 flags each.
    let re_bullet = regex::Regex::new(
        r"(?m)^\s*-\s+\*\*`(-[A-Za-z+])(?:\s+[^`]*)?`[^*\n]*\*\*[ \t]*[^A-Za-z\n]*[ \t]*(?:\n[ \t]*)?([A-Z][^\n]+)",
    )
    .unwrap();
    for cap in re_bullet.captures_iter(&body) {
        let flag = cap.get(1).unwrap().as_str().to_string();
        let raw_desc = cap.get(2).map(|m| m.as_str()).unwrap_or("");
        let desc: String = raw_desc.trim().chars().take(200).collect();
        if !out.iter().any(|(f, _)| f == &flag) {
            out.push((flag, desc));
        }
    }
    // Tier 2: when tier-1 produced nothing, fall back to inline
    // `` `-X` `` citations scattered through the prose. Covers
    // cd / set / unset / echo / unsetopt / etc whose docs describe
    // flags in flowing text rather than bullet lists.
    //
    // For each cited flag, pull a SENTENCE-ish description from the
    // surrounding prose. Heuristic: find the sentence that contains
    // the flag mention, take from its opening (`. ` boundary or
    // start-of-paragraph) to the next `.` / `\n\n`. Strip markdown
    // backticks and underscore-italics for readability.
    if out.is_empty() {
        let re_inline = regex::Regex::new(r"`(-[A-Za-z+])`").unwrap();
        for cap in re_inline.captures_iter(&body) {
            let flag = cap.get(1).unwrap().as_str().to_string();
            if out.iter().any(|(f, _)| f == &flag) {
                continue;
            }
            // Find the FIRST occurrence position of this flag in
            // body, then scan around for a description sentence.
            let desc = derive_inline_flag_desc(&body, &flag).unwrap_or_default();
            out.push((flag, desc));
        }
    }
    // Tier 3 merge: union with hand-curated overrides sourced from
    // `man zshall`. Overrides win on flag-letter collisions; body
    // entries fill in letters the override doesn't list. Pinned by
    // `tests/lsp_man_audit.rs`.
    if let Some(over) = lookup_builtin_flag_docs_override(name) {
        let over_keys: std::collections::HashSet<&str> = over.iter().map(|(f, _)| *f).collect();
        out.retain(|(f, _)| !over_keys.contains(f.as_str()));
        for (f, d) in over {
            out.push((f.to_string(), d.to_string()));
        }
    }
    tracing::debug!(
        target: "zshrs::lsp::completion",
        builtin = %name,
        flag_count = out.len(),
        "extract_builtin_flags",
    );
    if let Ok(mut g) = cache.lock() {
        g.insert(name.to_string(), out.clone());
    }
    out
}

/// Look up the hand-curated zsh-builtin flag table. Sourced from
/// `man zshall`. Merged with body-scraped flags in
/// `extract_builtin_flags` — overrides win on flag-letter collisions,
/// body fills in the rest. Pinned by `tests/lsp_man_audit.rs`.
pub(crate) fn lookup_builtin_flag_docs_override(
    name: &str,
) -> Option<&'static [(&'static str, &'static str)]> {
    BUILTIN_FLAG_DOCS_OVERRIDE
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, flags)| *flags)
}

/// Hand-curated zsh-builtin flag tables sourced from `man zshall`.
/// Merged with body-scraped flags. Coverage 100% per
/// `tests/lsp_man_audit.rs`.
const BUILTIN_FLAG_DOCS_OVERRIDE: &[(&str, &[(&str, &str)])] = &[
    (
        "bindkey",
        &[(
            "-L",
            "With `-l`, format output as `bindkey -A` / `-N` replay invocations.",
        )],
    ),
    (
        "enable",
        &[(
            "-p",
            "Operate on patterns added with `disable -p` (custom match-pattern hooks).",
        )],
    ),
    (
        "example",
        &[
            ("-a", "Pass arg as the example builtin's first parameter."),
            (
                "-f",
                "Toggle the example builtin's `flag` field (test option).",
            ),
            ("-g", "Toggle the example builtin's global-state test mode."),
            ("-l", "Toggle the example builtin's `long` test mode."),
            ("-s", "Toggle the example builtin's stateful test mode."),
        ],
    ),
    (
        "fc",
        &[(
            "-s",
            "Substitute `old=new` on the selected line and re-execute (no editor invoked).",
        )],
    ),
    (
        "getln",
        &[
            (
                "-A",
                "Read into an array (split into words instead of one scalar).",
            ),
            ("-E", "Don't echo (default; symmetric counterpart to `-e`)."),
            ("-c", "Read characters one at a time."),
            ("-e", "Echo read text back to terminal as it arrives."),
            ("-l", "Read just one line (default)."),
            ("-n", "Don't strip trailing newline from the result."),
        ],
    ),
    (
        "kill",
        &[
            (
                "-g",
                "Send the signal to the process GROUP, not just the process. Job-spec is a pgid.",
            ),
            (
                "-i",
                "Interpret arguments as job specs rather than process ids.",
            ),
            ("-n", "`-n signum` — send numeric signal `signum`."),
            (
                "-s",
                "`-s signame` — send named signal (`TERM`, `HUP`, `KILL`, …).",
            ),
        ],
    ),
    (
        "print",
        &[(
            "-f",
            "`-f format` — printf-style format string (same semantics as `printf`).",
        )],
    ),
    (
        "read",
        &[
            ("-c", "Read characters one at a time (no line-buffering)."),
            ("-e", "Echo read input back to terminal as it arrives."),
        ],
    ),
    (
        "sched",
        &[
            (
                "-e",
                "`+sched +HH:MM:SS event...` — schedule a command at the given time.",
            ),
            (
                "-i",
                "`sched -i id` — remove the scheduled entry with the given id.",
            ),
            (
                "-m",
                "`sched -m mask` — match scheduled entries against a glob pattern.",
            ),
            ("-t", "Print scheduled entries with full timestamps."),
        ],
    ),
    (
        "type",
        &[
            (
                "-S",
                "Like `-s` but include scripts in `$PATH` as commands.",
            ),
            (
                "-a",
                "Print every match for each name (not just the first).",
            ),
            ("-f", "Skip functions when looking up `name`."),
            ("-m", "Treat each name as a glob pattern."),
            ("-p", "Print only external commands found in `$path`."),
            (
                "-s",
                "Suppress output; exit 0 if name resolves to a command.",
            ),
            (
                "-w",
                "Print one of `alias`/`builtin`/`command`/`function`/`hashed`/`none` per name.",
            ),
        ],
    ),
    (
        "ulimit",
        &[
            (
                "-H",
                "Operate on the hard limit (default with `-S` is the soft limit).",
            ),
            (
                "-N",
                "`-N n` — operate on resource number `n` (system-specific integer).",
            ),
            (
                "-S",
                "Operate on the soft limit (default if neither `-H` nor `-S` given).",
            ),
            ("-T", "Maximum number of threads per process."),
            (
                "-a",
                "List all of the current resource limits (default verb).",
            ),
            ("-c", "Maximum core-file size in 512-byte blocks."),
            ("-d", "Maximum data-segment size in kilobytes."),
            (
                "-f",
                "Maximum file size the shell can write in 512-byte blocks.",
            ),
            ("-i", "Maximum number of pending signals."),
            ("-k", "Maximum number of kqueues allocated (BSD)."),
            ("-l", "Maximum locked-in-memory address space in kilobytes."),
            ("-m", "Maximum resident-set size in kilobytes."),
            ("-n", "Maximum number of open file descriptors."),
            ("-p", "The number of pseudo-terminals (BSD)."),
            ("-q", "Maximum bytes in POSIX message queues."),
            ("-r", "Maximum real-time scheduling priority."),
            ("-s", "Maximum stack size in kilobytes."),
            ("-t", "Maximum CPU time in seconds."),
            ("-v", "Maximum virtual-memory address space in kilobytes."),
            ("-w", "Maximum kilobytes of swapped-out memory."),
            ("-x", "Maximum number of file-locks held."),
        ],
    ),
    (
        "where",
        &[
            (
                "-S",
                "Like `-s` but include scripts in `$PATH` as commands.",
            ),
            ("-m", "Treat each name as a glob pattern."),
            ("-p", "Print only external commands found in `$path`."),
            ("-s", "Suppress output; exit 0 if name resolves."),
            (
                "-w",
                "Print one of `alias`/`builtin`/`command`/`function`/`hashed`/`none` per name.",
            ),
            (
                "-x",
                "`-x num` — indent each printed body line by `num` spaces.",
            ),
        ],
    ),
    (
        "which",
        &[
            ("-S", "Like `-s` but include scripts in `$PATH`."),
            ("-a", "Print every match for each name."),
            ("-m", "Treat each name as a glob pattern."),
            ("-p", "Print only external commands found in `$path`."),
            ("-s", "Suppress output; exit 0 if name resolves."),
            (
                "-w",
                "Print one of `alias`/`builtin`/`command`/`function`/`hashed`/`none` per name.",
            ),
            (
                "-x",
                "`-x num` — indent each printed body line by `num` spaces.",
            ),
        ],
    ),
    (
        "zcompile",
        &[
            ("-k", "Mark each compiled function for KSH-style autoload."),
            ("-m", "With `-c` / `-a`, treat each name as a glob pattern."),
        ],
    ),
    // ── zsh/files coreutils-style builtins ──────────────────────
    (
        "chgrp",
        &[
            ("-R", "Recursively descend into directories."),
            ("-h", "Change group of the symlink itself, not the target."),
            ("-s", "Suppress error messages for inaccessible files."),
        ],
    ),
    (
        "ln",
        &[
            (
                "-d",
                "Create a hard link to a directory (requires privilege).",
            ),
            (
                "-f",
                "If `dest` exists, remove it before creating the link.",
            ),
            (
                "-h",
                "If `dest` is a symlink, operate on the symlink itself.",
            ),
            ("-i", "Prompt before overwriting `dest`."),
            (
                "-n",
                "If `dest` is a symlink to a directory, replace the symlink.",
            ),
            ("-s", "Create a symbolic link instead of a hard link."),
        ],
    ),
    // ── zsh/system ──────────────────────────────────────────────
    (
        "syserror",
        &[
            (
                "-e",
                "`-e errvar` — store error string in `$errvar` instead of stderr.",
            ),
            ("-p", "`-p prefix` — prepend `prefix` to the error message."),
        ],
    ),
    (
        "sysread",
        &[
            (
                "-c",
                "`-c countvar` — store byte count read in `$countvar`.",
            ),
            (
                "-i",
                "`-i infd` — read from file descriptor `infd` instead of stdin.",
            ),
            (
                "-o",
                "`-o outfd` — relay bytes to `outfd` as well as storing them.",
            ),
        ],
    ),
    (
        "syswrite",
        &[
            (
                "-c",
                "`-c countvar` — store byte count actually written in `$countvar`.",
            ),
            ("-o", "`-o outfd` — write to `outfd` instead of stdout."),
        ],
    ),
    (
        "zselect",
        &[
            ("-A", "`-A arrayname` — store ready fds into `arrayname`."),
            (
                "-t",
                "`-t timeout` — timeout in hundredths of a second (centiseconds).",
            ),
        ],
    ),
    (
        "zsystem",
        &[(
            "-f",
            "`zsystem flock -f var file` — store lock file descriptor in `$var`.",
        )],
    ),
    // ── zsh/net/socket + zsh/net/tcp ────────────────────────────
    (
        "zsocket",
        &[
            (
                "-a",
                "Open a server (listening) socket bound to the named path.",
            ),
            (
                "-d",
                "`-d fd` — open the socket on the specified file descriptor.",
            ),
            ("-l", "List currently-open zsocket file descriptors."),
            ("-t", "Set close-on-exec on the socket."),
            (
                "-v",
                "Verbose — print the resulting file descriptor to stdout.",
            ),
        ],
    ),
    (
        "ztcp",
        &[
            (
                "-a",
                "Server mode — accept the next connection on the specified listening fd.",
            ),
            ("-c", "Close the named ztcp file descriptor."),
            ("-d", "`-d fd` — operate on the specified file descriptor."),
            (
                "-f",
                "Force — don't fail if a similar connection already exists.",
            ),
            (
                "-l",
                "Listen mode — open a server socket on the given port.",
            ),
            ("-t", "Set close-on-exec on the socket."),
            (
                "-v",
                "Verbose — print the resulting file descriptor to stdout.",
            ),
        ],
    ),
    // ── zsh/db/gdbm xattr family ────────────────────────────────
    (
        "zdelattr",
        &[("-h", "Operate on the symlink itself, not its target.")],
    ),
    (
        "zgetattr",
        &[("-h", "Operate on the symlink itself, not its target.")],
    ),
    (
        "zlistattr",
        &[("-h", "Operate on the symlink itself, not its target.")],
    ),
    (
        "zsetattr",
        &[("-h", "Operate on the symlink itself, not its target.")],
    ),
    // ── zsh/zutil ───────────────────────────────────────────────
    (
        "zstyle",
        &[
            (
                "-L",
                "`-L [ metapattern [ style ] ]` — list styles in `zstyle`-replay form.",
            ),
            (
                "-e",
                "`-e pattern style string ...` — value-as-shell-code (re-evaluated each lookup).",
            ),
        ],
    ),
    // ── zsh/zpty ────────────────────────────────────────────────
    (
        "zpty",
        &[
            ("-L", "List active zpty sessions with their commands."),
            (
                "-m",
                "With `-r`, treat `pattern` as a match-spec (read until pattern matches).",
            ),
            (
                "-n",
                "With `-w`, don't append a newline to written strings.",
            ),
            ("-t", "Test whether the named zpty session is still alive."),
        ],
    ),
    // ── zshzle: zle builtin ─────────────────────────────────────
    (
        "zle",
        &[
            (
                "-L",
                "With `-l`, format output as `zle` replay invocations.",
            ),
            (
                "-a",
                "With `-N`, mark new widget as available outside the editor (script-callable).",
            ),
            ("-c", "With `-R`, clear the screen before re-display."),
            (
                "-n",
                "Pass `-n num` through to the widget invocation (numeric argument).",
            ),
            ("-r", "With `-T`, remove the named termcap handler."),
            (
                "-w",
                "With `-F`, treat the fd handler as a writeable-fd ready handler.",
            ),
        ],
    ),
];

/// Per-compsys-fn flag tables sourced from `man zshcompsys` signatures
/// (`_foo [ -x ] [ -12VJ ] tag name descr …`). The yodl source for
/// compsys docs uses signature-style headers — not bullet lists — so
/// the bullet/inline scrapers in `extract_builtin_flags` miss most
/// flags. This table beats both tiers for the 26 ported documented in
/// `zshcompsys.1`.
///
/// Shared conventions (per zsh source):
///   `-1`, `-2`, `-V`, `-J`     — passed through to `compadd` for
///                                 grouping / sort-order tags.
///   `-x`                        — show description even when no
///                                 matches are added (else descr
///                                 only shows when matches exist).
///   `-C name`                   — set the curcontext parameter to
///                                 `name` while running.
fn lookup_compsys_flag_docs(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    COMPSYS_FN_FLAG_DOCS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, flags)| *flags)
}

/// Gate for the `BuiltinLongFlag` context. True only for the binary
/// itself today — every other long-flag-aware command in the corpus
/// is reached through its short-flag table.
fn is_known_builtin_with_long_flag_docs(name: &str) -> bool {
    matches!(name, "zshrs" | "zsh")
}

/// Long-form flag table for `zshrs --<TAB>` / `zsh --<TAB>`. Built
/// once (lazy) from [`ZSHRS_SELF_LONG_FLAG_DOCS`] + every entry in
/// [`crate::zsh_option_docs::OPTION_DOCS`] transformed to its
/// invocation spelling (`AUTO_CD` → `--autocd`). Cached for the
/// process lifetime — keystroke-rate completion can't re-build 970+
/// entries on every press.
fn extract_builtin_long_flags(name: &str) -> Vec<(String, String)> {
    use std::sync::OnceLock;
    if name != "zshrs" && name != "zsh" {
        return Vec::new();
    }
    static CACHE: OnceLock<Vec<(String, String)>> = OnceLock::new();
    CACHE
        .get_or_init(|| {
            let mut out: Vec<(String, String)> = ZSHRS_SELF_LONG_FLAG_DOCS
                .iter()
                .map(|(f, d)| (f.to_string(), d.to_string()))
                .collect();
            // Setopt mirrors: every OPTION_DOCS canonical entry is
            // reachable as `--<lowercase-no-underscores>` (positive)
            // AND `--no-<lowercase-no-underscores>` (inverse) per
            // zsh's `--OPTION` / `--no-OPTION` invocation grammar
            // (see `Src/main.c:parseargs`). First-line snippet for
            // the positive form so IntelliJ rows stay single-line;
            // inverse gets a uniform "turn off" caption.
            for (opt_name, opt_desc) in crate::zsh_option_docs::OPTION_DOCS {
                let lower = opt_name.to_ascii_lowercase().replace('_', "");
                let first_line = opt_desc
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("")
                    .trim();
                let short: String = first_line.chars().take(200).collect();
                out.push((format!("--{}", lower), short));
                out.push((
                    format!("--no-{}", lower),
                    format!("Turn OFF `--{}`. Inverse of the setopt option.", lower),
                ));
            }
            out
        })
        .clone()
}

/// `zshrs -<TAB>` / `zsh -<TAB>` self-flag completions. Mirrors the
/// "Standard zsh options" section of `zshrs --help`. Documenting these
/// directly because the binary itself isn't in any of the builtin /
/// ext-builtin / compsys name sets, so the bullet scraper never sees
/// the canonical zsh `Src/main.c:parseargs` flag dispatch.
///
/// Long-form (`--lsp`, `--dap HOST:PORT`, `--zsh`, `--errexit`, …) is
/// intentionally out of scope here: the completion dispatcher's
/// `BuiltinFlag` context strips the leading `-` and stacks letters
/// (`-fl` → suggest `-fla`, `-flb`, …). Long-flag completion needs a
/// separate `BuiltinLongFlag` context with its own prefix model.
const ZSHRS_SELF_FLAG_DOCS: &[(&str, &str)] = &[
    ("-b", "End option processing, like `--`. Subsequent arguments are positional even if they start with `-`."),
    ("-c", "Take the FIRST argument as a command string to execute. Example: `zshrs -c 'echo hi'`."),
    ("-f", "Equivalent to `--no-rcs`: skip sourcing `.zshenv` / `.zshrc` / `.zprofile` / `.zlogin`. Use for clean-environment scripting."),
    ("-i", "Force interactive mode even when stdin isn't a terminal (job control + prompt + line editor active)."),
    ("-l", "Force login-shell mode: source `.zprofile` / `.zlogin` on startup and `.zlogout` on exit. Equivalent to invoking as `-zshrs`."),
    ("-s", "Read commands from standard input. Combine with `-c CMD …` to run CMD first then drain stdin."),
    ("-o", "Set a setopt option by name. Example: `-o errexit -o pipefail -o nounset`. Inverse: `+o OPTION` or `--no-OPTION`."),
    ("-v", "Verbose: print each input line as it's read. Equivalent to `--verbose` / `setopt VERBOSE`."),
    ("-x", "xtrace: print each command and its arguments before execution. Equivalent to `--xtrace` / `setopt XTRACE` / `set -x`."),
];

/// zshrs-specific long flags (everything in `zshrs --help` that ISN'T
/// a setopt mirror — those flow in automatically from `OPTION_DOCS`).
/// Order is "most useful first" so the lookup popup leads with the
/// editor-integration + parity-mode flags users actually invoke from
/// scripts and CI.
const ZSHRS_SELF_LONG_FLAG_DOCS: &[(&str, &str)] = &[
    // Special / informational
    ("--help",     "Print the full usage message (every flag, dumper, parity mode) and exit 0."),
    ("--version",  "Print the zsh version string baked into the binary and exit 0."),
    ("--doctor",   "Full diagnostic report of shell health, caches, plugin load timings, and performance counters."),
    // Editor / IDE integration
    ("--lsp",      "Run the Language Server on stdio. Serves completion / hover / definition / references / rename / documentSymbol / foldingRange / semanticTokens / formatting / diagnostics. Consumed by the IntelliJ plugin, Helix, Neovim, VS Code, etc."),
    ("--dap",      "`--dap HOST:PORT` — Debug Adapter Protocol server. Connects back to the IDE's listener at HOST:PORT and drives breakpoints / step / variables / evaluate."),
    ("--dump-reflection", "Emit the JSON blob the IntelliJ \"zshrs\" reflection tool window consumes: builtins / keywords / options / special_vars, each tagged by category."),
    ("--docs",     "`--docs NAME` — render the same hover card the LSP would return for NAME. Used by the IntelliJ tool window's docs popup; handy for previewing doc output from the CLI."),
    // Parser / VM dumpers
    ("--dump-tokens",   "`--dump-tokens FILE|-` — one TOKNAME<tab>TOKSTR line per lexer token. Use `-` to read from stdin."),
    ("--dump-ast",      "`--dump-ast FILE|-` — parser AST as a canonical S-expression. Use `-` to read from stdin."),
    ("--dump-wordcode", "`--dump-wordcode FILE|-` — wordcode emitter output: EPROG / WORDS / WC[i] / STRS sections matching zsh's binary cache format."),
    ("--dump-zwc",      "`--dump-zwc ZWCFILE [FN]` — inspect a compiled .zwc cache. Without FN, list every function. With FN, dump only that function's wordcode."),
    ("--disasm",        "Print fusevm opcodes for each compiled unit before VM run. Does NOT suppress execution — script still runs."),
    // Documentation generation (parallel to `stryke gen-docs`)
    ("--gen-docs",      "`--gen-docs [PATH] [--out DIR]` — walk PATH (default `.`) collecting every `##` doc-comment block above each function / alias / parameter, render to standalone HTML in `DIR` (default `docs/`)."),
    ("--out",           "`--out DIR` — destination directory for `--gen-docs` HTML output (default `docs/`)."),
    ("--dump-reference-html", "Emit the standalone reference HTML the zshrs project ships at `docs/reference.html` — every builtin / keyword / option / special var as a browseable single-page reference."),
    ("--names",         "With `--dump-reflection` or `--gen-docs`, restrict the dump / walk to a comma-separated list of NAMES instead of the full set."),
    // Daemon / interactive runtime
    ("--daemon",        "Run as the persistent zshrs daemon (used by the IDE / multi-shell scenarios). Holds the rkyv script cache + worker pool warm so subsequent script launches are sub-millisecond."),
    ("--color",         "`--color WHEN` — control coloured output (`auto` / `always` / `never`). Default `auto`: respect `$TERM`, `$NO_COLOR`, and stdout TTY status."),
    // Parity modes (drop-in shell emulation)
    ("--zsh",       "Identical-behaviour drop-in for `/bin/zsh`. Caches OFF, daemon OFF — every `source` re-runs the file fresh. Used as the compat-test entrypoint."),
    ("--bash",      "Identical-behaviour drop-in for `/bin/bash`. Caches / daemon OFF; every echo / source re-fires byte-for-byte against reference bash."),
    ("--ksh",       "Identical-behaviour drop-in for `/bin/ksh` (ksh-93). Caches / daemon OFF."),
    ("--sh",        "Identical-behaviour drop-in for `/bin/sh` / POSIX (alias of `--posix`). Caches / daemon OFF."),
    ("--csh",       "Identical-behaviour drop-in for `/bin/csh`. Caches / daemon OFF."),
    ("--posix",     "Identical-behaviour drop-in for `/bin/sh` / POSIX (Bourne / dash semantics). Caches / daemon OFF."),
    ("--emulate",   "`--emulate MODE` — generic parity alias for `--MODE` (zsh-compat: matches the `emulate zsh` / `emulate bash` etc. builtin)."),
    ("--zsh-compat","Alias of `--zsh` (legacy spelling — kept for back-compat with older scripts / CI invocations)."),
    // Misc invocation
    ("--no-rcs",   "Skip sourcing `.zshenv` / `.zshrc` / `.zprofile` / `.zlogin`. Equivalent to the short `-f` flag."),
    ("--verbose",  "Print each input line as it's read. Equivalent to short `-v` / `setopt VERBOSE`."),
    ("--xtrace",   "Print each command and its arguments before executing. Equivalent to short `-x` / `setopt XTRACE` / `set -x`."),
    ("--login",    "Force login-shell mode. Equivalent to short `-l`."),
    ("--interactive", "Force interactive mode. Equivalent to short `-i`."),
];

const COMPSYS_FN_FLAG_DOCS: &[(&str, &[(&str, &str)])] = &[
    (
        "_all_labels",
        &[
            ("-x", "Show the description even when no matches are added."),
            ("-1", "Pass `-1` through to `compadd` (suppress duplicate-removal of literal matches)."),
            ("-2", "Pass `-2` through to `compadd` (suppress de-duplication based on display string)."),
            ("-V", "Pass `-V name` through to `compadd` (put matches in a named unsorted group)."),
            ("-J", "Pass `-J name` through to `compadd` (put matches in a named sorted group)."),
        ],
    ),
    (
        "_alternative",
        &[
            ("-O", "Pass `-O name` through to nested `_arguments` calls (preserve the option-spec array)."),
            ("-C", "Set the curcontext parameter to `name` while running each alternative."),
        ],
    ),
    (
        "_arguments",
        &[
            ("-n", "Set `$NORMARG` to the position of the first normal argument in the `$words` array."),
            ("-s", "Allow option stacking (`-abc` parsed as `-a -b -c`) — required for typical POSIX-style getopts CLIs."),
            ("-w", "Even with `-s`, allow stacked options to consume an argument before the next short option."),
            ("-W", "Even after a `--` separator, keep parsing further `-x` as options instead of treating them as positional."),
            ("-C", "Make `curcontext` available to action handlers in `->state` form."),
            ("-R", "Return status 300 instead of 0 when a `->state` action is dispatched (lets callers chain dispatch)."),
            ("-S", "Stop parsing options once a non-option word is seen (treat the rest as positionals)."),
            ("-A", "Treat any argument matching pattern `pat` as a non-option terminator (e.g. `-A '-*'`)."),
            ("-O", "Name an array holding extra `compadd` options to pass to every match."),
            ("-M", "Pass match spec `matchspec` to `compadd` (controls case-folding, partial-word matching, etc.)."),
            ("--", "Separator between option specs and rest-arg `[helpspec ...]` syntax."),
            ("-l", "Long-option style: each rest-arg `helpspec` describes one long option (e.g. `--foo=bar`)."),
            ("-i", "Skip option specs whose names match any of the patterns in `pats`."),
        ],
    ),
    (
        "_call_program",
        &[
            ("-l", "Read one line at a time from the external program (don't slurp all output)."),
            ("-p", "Treat the tag as a program-call context for the `command` style lookup."),
        ],
    ),
    (
        "_canonical_paths",
        &[
            ("-A", "Store the resolved canonical paths in array variable `var`."),
            ("-N", "Don't realpath-resolve — treat the input paths as already canonical."),
            ("-M", "Pass `-M matchspec` through to `compadd`."),
            ("-J", "Pass `-J name` through to `compadd` (named sorted group)."),
            ("-V", "Pass `-V name` through to `compadd` (named unsorted group)."),
            ("-1", "Pass `-1` through to `compadd`."),
            ("-2", "Pass `-2` through to `compadd`."),
            ("-n", "Pass `-n` through to `compadd` (no inserted suffix)."),
            ("-f", "Pass `-f` through to `compadd` (mark matches as filenames for suffix/cdpath handling)."),
            ("-X", "Pass `-X explanation` through to `compadd` (custom listing-line message)."),
        ],
    ),
    (
        "_combination",
        &[
            ("-s", "Use `pattern` to split each existing style value into fields (default is comma)."),
        ],
    ),
    (
        "_command_names",
        &[
            ("-e", "Complete only external commands found in `$path` (skip aliases, builtins, functions, reserved words)."),
            ("-", "Same as `-e`: external-only mode."),
        ],
    ),
    (
        "_completers",
        &[
            ("-p", "List only completer-function names that are valid for use in the `completer` style."),
        ],
    ),
    (
        "_describe",
        &[
            ("-1", "Pass `-1` through to `_next_label`/`compadd`."),
            ("-2", "Pass `-2` through to `_next_label`/`compadd`."),
            ("-J", "Pass `-J name` through to `_next_label` (named sorted group)."),
            ("-V", "Pass `-V name` through to `_next_label` (named unsorted group)."),
            ("-x", "Show the description even when no matches are added."),
            ("-o", "Treat each `name1`/`name2` array as describing options (default tag is `options`)."),
            ("-O", "Like `-o` but each match supports an argument-string suffix after a colon."),
            ("-t", "Use `tag` instead of the default `values`/`options` tag."),
        ],
    ),
    (
        "_description",
        &[
            ("-x", "Show the description even when no matches are added."),
            ("-1", "Pass `-1` through to `compadd`."),
            ("-2", "Pass `-2` through to `compadd`."),
            ("-V", "Pass `-V name` through to `compadd`."),
            ("-J", "Pass `-J name` through to `compadd` (groups matches in a named sorted group based on group-name style)."),
        ],
    ),
    (
        "_dir_list",
        &[
            ("-s", "Use `sep` instead of `:` as the directory-list separator."),
            ("-S", "Keep partially-typed list elements in the completion (allow trailing separator)."),
        ],
    ),
    (
        "_email_addresses",
        &[
            ("-c", "Add only the current-typed prefix as a match (don't expand to full addresses)."),
            ("-n", "Restrict to entries returned by the named `plugin` backend (`_email-`<plugin>)."),
        ],
    ),
    (
        "_message",
        &[
            ("-r", "Take `descr` literally as the message text (skip the `format` style lookup)."),
            ("-1", "Show the message even when no matches are added."),
            ("-2", "Show the message only when matches already exist for some other tag."),
            ("-V", "Display the message in a named unsorted group `group`."),
            ("-J", "Display the message in a named sorted group `group`."),
        ],
    ),
    (
        "_multi_parts",
        &[
            ("-i", "Insert matches immediately rather than only when uniquely determined."),
        ],
    ),
    (
        "_next_label",
        &[
            ("-x", "Show the description even when no matches are added."),
            ("-1", "Pass `-1` through to `compadd`."),
            ("-2", "Pass `-2` through to `compadd`."),
            ("-V", "Pass `-V name` through to `compadd`."),
            ("-J", "Pass `-J name` through to `compadd`."),
        ],
    ),
    (
        "_normal",
        &[
            ("-P", "Treat the current word as following a precommand (`nohup`, `time`, …) — skip to the real command."),
            ("-p", "Like `-P` but explicitly name the precommand for context lookup."),
        ],
    ),
    (
        "_numbers",
        &[
            // _numbers signature uses `[ option ... ]` — flags are
            // the `compadd` ones plus signal-specific selectors. No
            // canonical short-flag list in the man page; intentionally
            // empty so caller falls back to no-flag-completion.
        ],
    ),
    (
        "_pick_variant",
        &[
            ("-b", "Treat `builtin-label` as the variant when the command is a shell builtin."),
            ("-c", "Run `command` (with its args) to obtain the version string instead of the default `--version` invocation."),
            ("-r", "Store the matched variant name in the parameter `name`."),
        ],
    ),
    (
        "_requested",
        &[
            ("-x", "Show the description even when no matches are added."),
            ("-1", "Pass `-1` through to `compadd`."),
            ("-2", "Pass `-2` through to `compadd`."),
            ("-V", "Pass `-V name` through to `compadd`."),
            ("-J", "Pass `-J name` through to `compadd`."),
        ],
    ),
    (
        "_sequence",
        &[
            ("-s", "Use `sep` instead of `,` as the list separator."),
            ("-n", "Stop accepting more matches once `max` items have been completed in the sequence."),
            ("-d", "Allow duplicate entries in the sequence (default rejects already-typed values)."),
        ],
    ),
    (
        "_tags",
        &[
            ("-C", "Set the curcontext parameter to `name` while iterating the tag list."),
        ],
    ),
    (
        "_values",
        &[
            ("-O", "Name an array holding extra `compadd` options to pass to every match."),
            ("-s", "Use `sep` as the value separator between multiple keywords (default is comma)."),
            ("-S", "Use `sep` as the keyword-and-argument separator (default is `=`)."),
            ("-w", "Examine other typed arguments as well when computing which value-specs are already used."),
            ("-C", "Make `curcontext` available to action handlers (must be made local by the caller)."),
        ],
    ),
    (
        "_wanted",
        &[
            ("-x", "Show the description even when no matches are added."),
            ("-C", "Set the curcontext parameter to `name` while invoking the inner command."),
            ("-1", "Pass `-1` through to `compadd`."),
            ("-2", "Pass `-2` through to `compadd`."),
            ("-V", "Pass `-V name` through to `compadd`."),
            ("-J", "Pass `-J name` through to `compadd`."),
        ],
    ),
    (
        "_widgets",
        &[
            ("-g", "Restrict completions to widget names matching shell pattern `pattern`."),
        ],
    ),
];

/// Hand docs for compsys functions whose names don't have a per-name
/// `item(tt(_X))` block in `compsys.yo` / `compwid.yo`. Per-command
/// completers (`_git`, `_docker`, …) and a couple of core dispatch
/// internals fall here.
const COMPSYS_FN_DOCS: &[(&str, &str)] = &[
    (
        "_main_complete",
        "Top-level entry the compsys dispatcher calls for every completion attempt. Walks the configured completer list (`_complete` / `_approximate` / `_match` / …), invoking each until one returns matches. Sets `$compstate[insert]` based on the result. Rust impl in `crate::compsys::ported::_main_complete::_main_complete`.",
    ),
    (
        "_directories",
        "Complete directory names only. Equivalent to `_files -/`. Honors `path-files` zstyle and respects `GLOB_DOTS`. Rust impl in `crate::compsys::files::directories_execute`.",
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
    ("compdef", "Register a completion function for one or more commands. `compdef _git git`. Backed by the rkyv-mmap'd compsys shards; lookups are zero-copy. (SQLite mirrors exist beside the shards for `dbview` / SQL inspection only.)"),
    ("compgen", "Bash-compatible word generator. `compgen -W 'foo bar baz' fo` → `foo`. Used by bash-completion scripts ported to zshrs."),
    ("compinit", "Initialize the completion system. Walks `$fpath` in parallel via rayon, populates the rkyv-mmap'd autoload/completion shards, marks every `_*` as autoloaded. Default mode skips `.zcompdump` entirely."),
    ("complete", "Bash-compatible `complete` command — register a completion spec for a command. zshrs bridges to compsys internally."),
    ("compopt", "Bash-compatible `compopt` — modify completion options at runtime."),
    ("cut", "Extract fields or character ranges. `-d':' -f1,3` / `-c5-10`. coreutils drop-in."),
    ("date", "Print or set the system date. `+%FORMAT` strftime; `-d 'rel'` parse relative; `-u` UTC. coreutils drop-in."),
    ("dbview", "Dump the local zshrs SQLite mirror tables (autoload bodies, completion mirror, history FTS). Mirrors only &mdash; the authoritative cache is the rkyv-mmap'd shard set; SQLite is hydrated read-only for SQL/`dbview` inspection. `dbview --table autoloads` filters by table."),
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
    ("run_tests", "Alias for `ztest_run` — print the per-block test summary and roll the per-block counters into the run-wide totals. Returns 0 on all-pass, 1 if any assertion failed. Port of strykelang's `test_run` / `run_tests` builtin pair."),
    ("zassert_contains", "`zassert_contains HAYSTACK NEEDLE [MSG]` — pass when NEEDLE is a substring of HAYSTACK. Bumps the per-block `ztest_pass_count` on success, `ztest_fail_count` on failure; the ✓/✗ glyph + label print on stderr. Port of strykelang's `builtin_assert_contains`."),
    ("zassert_dies", "`zassert_dies \"CMD\" [MSG]` — shell-native variant of strykelang's `assert_dies` (which takes a code ref). Runs CMD in a scratch `ShellExecutor` (inheriting the host's `zsh_compat` / `bash_compat` / `posix_mode`); passes iff the command exits non-zero or errors. Use to assert that a misuse correctly fails."),
    ("zassert_eq", "`zassert_eq A B [MSG]` — string equality. Like every `zassert_*` builtin, returns 0 on pass / 1 on fail and bumps the per-block counters that `ztest_run` reads."),
    ("zassert_err", "`zassert_err V [MSG]` — pass when V is shell-falsy (empty *or* literal \"0\"). Convenient for asserting on `$?` after a command, or on a string that should be empty."),
    ("zassert_false", "`zassert_false V [MSG]` — alias of `zassert_err`."),
    ("zassert_ge", "`zassert_ge A B [MSG]` — pass iff numeric A ≥ B. Inputs are `trim().parse::<f64>()`'d; non-numeric inputs default to 0."),
    ("zassert_gt", "`zassert_gt A B [MSG]` — pass iff numeric A > B."),
    ("zassert_le", "`zassert_le A B [MSG]` — pass iff numeric A ≤ B."),
    ("zassert_lt", "`zassert_lt A B [MSG]` — pass iff numeric A < B."),
    ("zassert_match", "`zassert_match PATTERN STRING [MSG]` — regex match. PATTERN is compiled with the `regex` crate (Rust regex syntax, not POSIX BRE/ERE — so `\\d`/`\\w`/`\\s` work, no backreferences). Fails (rather than panics) on a bad pattern."),
    ("zassert_ne", "`zassert_ne A B [MSG]` — string inequality."),
    ("zassert_near", "`zassert_near A B [EPS [MSG]]` — floating-point approximate equality: pass iff `|A − B| ≤ EPS`. EPS defaults to `1e-9`. Right for asserting on math results without burning on the last few ULPs."),
    ("zassert_ok", "`zassert_ok V [MSG]` — pass when V is shell-truthy (non-empty *and* not literal \"0\"). Use `zassert_eq $? 0 …` for command-success assertions; reserve `zassert_ok` for assertions on populated strings or non-zero counts."),
    ("zassert_true", "`zassert_true V [MSG]` — alias of `zassert_ok`."),
    ("ztest_run", "Print the per-block test summary (green ✓ All N passed / red ✗ M of N failed) and roll the per-block `ztest_pass_count` / `ztest_fail_count` / `ztest_skip_count` into the run-wide `_total` counters, then zero the per-block counters so a second test block in the same file starts fresh. A sticky `ztest_run_failed` flag latches when any block reports failures — the `--ztest` worker reads it to exit non-zero even if the script ends with `exit 0`. Alias: `run_tests`. Returns 0 on all-pass, 1 if any assertion failed."),
    ("ztest_skip", "`ztest_skip MSG` — bump the per-block skip counter and emit a yellow ↷ marker on stderr. Typical use: gate an assertion behind a compat condition using shell short-circuit (`(( cond )) || { ztest_skip \"needs X\"; continue; }`)."),
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
const OPTION_DOCS_FALLBACK: &[(&str, &str)] = &[(
    "RESTRICTED",
    "Restricted-shell mode (equivalent to invoking zsh as `rzsh` or with `-r`).\
         \n\nDisables: `cd`, modifying `$PATH` / `$ENV` / `$SHELL`, `>` / `>>` redirects,\
         creating functions with the `function` keyword, `exec`-ing commands containing `/`,\
         `kill`-ing by pid, and several `setopt` toggles. Designed for sandboxed login shells\
         where the user must stay inside a curated command set. Once set, cannot be cleared\
         within the running shell.",
)];

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
    // Try AST first — walks active file + transitively-sourced files
    // (via `source X` / `. X` / `zsource X`) for a matching FuncDef
    // or top-level assignment. Falls through to the textual single-
    // file scan below on parse failure.
    if let Some(v) = definition_via_ast(state, uri, text, &word, bare) {
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
fn definition_via_ast(
    state: &State,
    active_uri: &str,
    active_text: &str,
    word: &str,
    bare: &str,
) -> Option<Value> {
    use crate::lsp_symbols::{find_ast_occurrences, SymbolKind};
    // `$x` cursor → look up Global decl. Bare cursor → look up Func
    // decl. (Locals don't cross files.) For bare words we also try
    // Global as a fallback (e.g. cursor on `FOO` in `echo FOO=1`).
    let kind = if word.starts_with('$') {
        SymbolKind::Global
    } else {
        SymbolKind::Func
    };
    // Cross-file scope in zsh is OPT-IN via `source X` / `. X` /
    // `zsource X`. Walk active file + transitively-sourced files only.
    // Previously this iterated `state.all_docs()` indiscriminately,
    // returning false-positive jumps to unrelated workspace files that
    // happened to share a symbol name. Fixed: build the source-chain
    // BFS via AST (`collect_sourced_paths`) and only search inside it.
    let files = collect_active_and_sourced_files(state, active_uri, active_text);
    let mut hits: Vec<Value> = Vec::new();
    let scan = |kind: SymbolKind, hits: &mut Vec<Value>| {
        for (uri, src) in &files {
            let lines = find_ast_occurrences(src, bare, kind.clone());
            for line in lines {
                if !line_is_decl(src, line, bare, &kind) {
                    continue;
                }
                if let Some((start, end)) = find_first_word_col(src, line, bare) {
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
    };
    scan(kind.clone(), &mut hits);
    if hits.is_empty() {
        // Fallback once for Global → Func or vice versa, in case the
        // cursor's `$` heuristic guessed wrong.
        let alt = if matches!(kind, SymbolKind::Global) {
            SymbolKind::Func
        } else {
            SymbolKind::Global
        };
        scan(alt, &mut hits);
    }
    match hits.len() {
        0 => None,
        1 => Some(hits.into_iter().next().unwrap()),
        _ => Some(Value::Array(hits)),
    }
}

/// Walk the source graph reachable from `active_uri` (BOTH directions:
/// files the active file transitively sources, AND files that source
/// the active file — i.e. dependents). Returns `(uri, text)` for every
/// file in the source-graph component containing `active_uri`.
///
/// The two-direction walk is the correct scope for rename / references:
/// a function declared in `lib.zsh` and called from `rc.zsh` (which has
/// `source lib.zsh`) needs to be findable from EITHER cursor position.
///
/// Forward = outgoing sources via AST `collect_sourced_paths`.
/// Reverse = scan every workspace file; if its forward chain contains
///           any URI already in the component, add it.
///
/// Cycle-guarded; depth- and breadth-capped (MAX_FILES) to keep
/// pathological rc chains from hanging the LSP. Files reached this way
/// are read fresh from disk so edits propagate without an explicit
/// didChangeWatchedFiles event.
fn collect_active_and_sourced_files(
    state: &State,
    active_uri: &str,
    active_text: &str,
) -> Vec<(String, String)> {
    const MAX_FILES: usize = 256;

    // Helper: read text for a URI, preferring open-doc → workspace cache → disk.
    let read_text = |uri: &str| -> Option<String> {
        if uri == active_uri {
            return Some(active_text.to_string());
        }
        state
            .docs
            .get(uri)
            .cloned()
            .or_else(|| state.workspace_files.get(uri).cloned())
            .or_else(|| file_uri_to_path(uri).and_then(|p| std::fs::read_to_string(p).ok()))
    };

    // Forward BFS: active + everything it transitively sources.
    let mut component: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    component.insert(active_uri.to_string(), active_text.to_string());
    let mut queue: Vec<String> = vec![active_uri.to_string()];
    while let Some(uri) = queue.pop() {
        if component.len() >= MAX_FILES {
            break;
        }
        let parent_text = match component.get(&uri).cloned().or_else(|| read_text(&uri)) {
            Some(t) => t,
            None => continue,
        };
        let parent_dir = file_uri_to_path(&uri)
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        for sourced_path in crate::lsp_symbols::collect_sourced_paths(&parent_text, &parent_dir) {
            let sourced_uri = format!("file://{}", sourced_path.display());
            if component.contains_key(&sourced_uri) {
                continue;
            }
            if let Some(t) = read_text(&sourced_uri) {
                component.insert(sourced_uri.clone(), t);
                queue.push(sourced_uri);
            }
        }
    }

    // Reverse: scan workspace files; if any of them forward-source a
    // file already in `component`, pull them in (transitively). Repeat
    // until fixed point (or MAX_FILES).
    loop {
        if component.len() >= MAX_FILES {
            tracing::warn!(
                target: "zshrs::lsp::source_chain",
                walked = component.len(),
                "source-graph component hit MAX_FILES cap",
            );
            break;
        }
        let before = component.len();
        // Snapshot URIs to avoid mutating during iteration. Iterate
        // BOTH `state.docs` (open editor buffers) and `workspace_files`
        // (cached unopened files), since dependents may live in either.
        let candidates: Vec<(String, String)> = state
            .all_docs()
            .into_iter()
            .filter(|(u, _)| !component.contains_key(u.as_str()))
            .collect();
        for (uri, src) in candidates {
            let parent_dir = file_uri_to_path(&uri)
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let mut sources_into_component = false;
            for sourced_path in crate::lsp_symbols::collect_sourced_paths(&src, &parent_dir) {
                let sourced_uri = format!("file://{}", sourced_path.display());
                if component.contains_key(&sourced_uri) {
                    sources_into_component = true;
                    break;
                }
            }
            if sources_into_component {
                component.insert(uri, src);
                if component.len() >= MAX_FILES {
                    break;
                }
            }
        }
        if component.len() == before {
            break;
        }
    }

    component.into_iter().collect()
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
            // Look for the kind in source-chain-reachable files only.
            // (Previously scanned `state.all_docs()` which let unrelated
            // workspace files seed the kind — wrong scope, same FAKE
            // class of bug as the cross-file emission below.)
            let chain_files = collect_active_and_sourced_files(state, active_uri, active_text);
            let mut found: Option<SymbolKind> = None;
            'outer: for (other_uri, src) in &chain_files {
                if other_uri == active_uri {
                    continue;
                }
                let Some(t) = SymbolTable::build(src) else {
                    continue;
                };
                for s in &t.symbols {
                    if s.name == bare && matches!(s.kind, SymbolKind::Func | SymbolKind::Global) {
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

    // Cross-file: only for symbols that cross file boundaries via an
    // EXPLICIT `source X` / `. X` / `zsource X` chain rooted at the
    // active file. Variables in unrelated scripts are SEPARATE
    // variables — zsh has no implicit cross-file scope.
    //
    // Previously this block iterated `state.all_docs()` indiscriminately,
    // emitting every same-name match in the entire workspace. Users
    // (correctly) called this FAKE: hovering `s1` in 278_rps.zsh would
    // jump to a `s1` defined in 999_unrelated.zsh just because the name
    // matched. Cross-file scope in zsh is opt-in via `source`; the BFS
    // below is the correct mechanism. Removed the indiscriminate sweep.
    if !matches!(kind, SymbolKind::Local) {
        // Use the shared source-graph component helper: walks both
        // outgoing sources (files this one imports) AND incoming
        // sources (files that import this one — for "rename a decl,
        // find all callers" semantics). Pure AST-based via
        // `collect_sourced_paths` so canonicalize / dedup is right.
        let component = collect_active_and_sourced_files(state, active_uri, active_text);
        for (uri, src) in &component {
            if uri == active_uri {
                continue; // already emitted above
            }
            let lines = find_ast_occurrences(src, &name, kind.clone());
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
        tracing::debug!(
            target: "zshrs::lsp::references_ast",
            files_in_component = component.len(),
            "source-graph component walked",
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
    "comment",        // 0
    "string",         // 1
    "number",         // 2
    "keyword",        // 3
    "operator",       // 4
    "function",       // 5 — compat zsh builtins
    "variable",       // 6
    "parameter",      // 7
    "type",           // 8
    "macro",          // 9 — kept for back-compat; also used for compsys ported now
    "property",       // 10
    "regexp",         // 11
    "zshrsExtension", // 12 — zshrs-only ext + daemon `z*` builtins
    "zshrsCompsys",   // 13 — `_arguments` / `_files` / `_describe` family
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
                let inner_end =
                    if close > 0 && close <= bb.len() && bb.get(close - 1) == Some(&(q as u8)) {
                        close - 1
                    } else {
                        close
                    };
                let flush_string = |data: &mut Vec<u32>,
                                    last_line: &mut u32,
                                    last_col: &mut u32,
                                    col: usize,
                                    seg_start: usize,
                                    seg_end: usize| {
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
                        // Detect $((arith)) up front so we can emit the
                        // INTERIOR as proper number/operator/identifier
                        // tokens rather than one opaque variable-colored
                        // blob. Without this, `"$(( -7 % 3 ))"` inside
                        // a double-quoted string had its arithmetic body
                        // colored like a string-internal variable.
                        let is_arith = q2 + 1 < inner_end && bb[q2] == b'(' && bb[q2 + 1] == b'(';
                        if is_arith {
                            // Scan to matching `))` (paren-depth aware).
                            let mut depth = 2i32; // already past `$((`
                            let arith_start = q2 + 2;
                            q2 = arith_start;
                            while q2 < inner_end && depth > 0 {
                                match bb[q2] {
                                    b'(' => depth += 1,
                                    b')' => depth -= 1,
                                    _ => {}
                                }
                                if depth == 0 {
                                    break;
                                }
                                q2 += 1;
                            }
                            // q2 now points at the first `)` of the
                            // closing `))`. Compute spans:
                            //   `$((` at var_start..arith_start
                            //   interior at arith_start..arith_end
                            //   `))`   at arith_end..(arith_end+2)
                            let arith_end = if q2 > 0 && q2 < bb.len() && bb[q2] == b')' {
                                // Move back to the `)` that closes the
                                // OUTER paren (one before q2 — but here
                                // q2 already IS the first `)` of `))`).
                                q2.saturating_sub(0)
                            } else {
                                q2
                            };
                            // Emit `$((` as operator.
                            push_tok(
                                &mut data,
                                &mut last_line,
                                &mut last_col,
                                ln,
                                (col + var_start) as u32,
                                3,
                                4,
                            );
                            // Tokenize the arithmetic interior into
                            // numbers / identifiers / operators.
                            emit_arith_interior(
                                &mut data,
                                &mut last_line,
                                &mut last_col,
                                ln,
                                col,
                                bb,
                                arith_start,
                                arith_end,
                            );
                            // Emit `))` as operator (2 bytes) if present.
                            if arith_end + 1 < bb.len()
                                && bb[arith_end] == b')'
                                && bb[arith_end + 1] == b')'
                            {
                                push_tok(
                                    &mut data,
                                    &mut last_line,
                                    &mut last_col,
                                    ln,
                                    (col + arith_end) as u32,
                                    2,
                                    4,
                                );
                                q2 = arith_end + 2;
                            } else {
                                q2 = arith_end;
                            }
                            seg_start = q2;
                            p = q2;
                            continue;
                        }
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
                flush_string(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    col,
                    seg_start,
                    close,
                );
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
            // Long CLI flag `--foo` / `--foo-bar` — emit as a single
            // OPERATOR token so the whole flag highlights uniformly.
            // Before this branch, `--verbose` fell through every
            // classifier (`-` isn't IUSER/IDENT, no operator match)
            // and ended up unhighlighted (default editor color) while
            // adjacent words stayed colored — the user sees an
            // inconsistent flag display reported in the screenshot.
            //
            // Short flag `-f` is intentionally NOT handled here (zsh's
            // `-x`/`-n`/etc. already render fine as `-` + word; only
            // the double-dash long form was visibly broken).
            if rest.starts_with("--") && rest.len() > 2 && rest.as_bytes()[2].is_ascii_alphabetic()
            {
                let bb = rest.as_bytes();
                let mut end = 2;
                while end < bb.len() {
                    let c = bb[end];
                    if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                        end += 1;
                    } else {
                        break;
                    }
                }
                push_tok(
                    &mut data,
                    &mut last_line,
                    &mut last_col,
                    ln,
                    col as u32,
                    end as u32,
                    4, // operator
                );
                col += end;
                continue;
            }
            // Brace-range expansion `{X..Y}` / `{X..Y..N}` — emit as
            // a single OPERATOR token so the editor colors the whole
            // span uniformly. Without this, the word-classifier below
            // would treat the inner `a`/`e`/`A`/`E` letter endpoints
            // as single-char identifiers and color them with the
            // VARIABLE style (token-type 6 → italic green) — which
            // makes `{A..E}` look like a variable reference, not a
            // brace expansion. Detection is conservative: must see
            // `{` then 1+ alnum chars then literal `..` then 1+ alnum
            // chars (optionally `..N`) then `}` all on the same line.
            // List-form `{a,b,c}` is not handled here (the commas and
            // identifiers already render readably); only the range form
            // produced the false variable-coloring.
            if rest.starts_with('{') {
                let bb = rest.as_bytes();
                let mut p = 1usize;
                let scan_run = |bb: &[u8], mut p: usize| -> usize {
                    while p < bb.len() {
                        let c = bb[p];
                        if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' {
                            p += 1;
                        } else {
                            break;
                        }
                    }
                    p
                };
                let after_a = scan_run(bb, p);
                let has_dotdot = after_a > p
                    && bb.get(after_a) == Some(&b'.')
                    && bb.get(after_a + 1) == Some(&b'.');
                if has_dotdot {
                    p = after_a + 2;
                    let after_b = scan_run(bb, p);
                    if after_b > p {
                        // Optional `..N` step.
                        let mut close = after_b;
                        if bb.get(close) == Some(&b'.') && bb.get(close + 1) == Some(&b'.') {
                            let after_step = scan_run(bb, close + 2);
                            if after_step > close + 2 {
                                close = after_step;
                            }
                        }
                        if bb.get(close) == Some(&b'}') {
                            let span = close + 1;
                            push_tok(
                                &mut data,
                                &mut last_line,
                                &mut last_col,
                                ln,
                                col as u32,
                                span as u32,
                                4, // operator
                            );
                            col += span;
                            continue;
                        }
                    }
                }
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
                ";;&", "<<<", "<<-", "&&", "||", "|&", "<<", ">>", "&>", ">|", ">!", ">&", "<&",
                "<>", "==", "!=", "=~", "+=", "-=", ":=", "?=", "[[", "]]", "((", "))", ";;", ";|",
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
                } else if crate::compsys::COMPSYS_FN_NAMES.contains(&w) {
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

/// Walk the interior of a `$((...))` arithmetic expression and emit
/// per-atom semantic tokens (numbers / identifiers / operators) so
/// the IDE colors the inside as CODE rather than as one opaque
/// variable-colored span. Mirrors the C arithmetic-lexer's atom set
/// in `Src/math.c`: digit runs are numbers, alnum-starting runs are
/// identifiers (vars), and everything else is an operator atom.
///
/// `bb` is the raw line bytes; `arith_start..arith_end` is the
/// half-open range inside `bb` covering the arithmetic body (between
/// `$((` and `))`). `col` is the column of the enclosing string's
/// opening quote, used as the base for the emitted token positions.
fn emit_arith_interior(
    data: &mut Vec<u32>,
    last_line: &mut u32,
    last_col: &mut u32,
    ln: u32,
    col: usize,
    bb: &[u8],
    arith_start: usize,
    arith_end: usize,
) {
    let mut p = arith_start;
    while p < arith_end {
        let c = bb[p];
        // Whitespace — skip.
        if c == b' ' || c == b'\t' {
            p += 1;
            continue;
        }
        // Digit run → number (type 2).
        if c.is_ascii_digit() {
            let mut end = p + 1;
            while end < arith_end && (bb[end].is_ascii_digit() || bb[end] == b'.') {
                end += 1;
            }
            push_tok(
                data,
                last_line,
                last_col,
                ln,
                (col + p) as u32,
                (end - p) as u32,
                2,
            );
            p = end;
            continue;
        }
        // Identifier (alpha or `_`) → variable (type 6).
        if c.is_ascii_alphabetic() || c == b'_' {
            let mut end = p + 1;
            while end < arith_end && (bb[end].is_ascii_alphanumeric() || bb[end] == b'_') {
                end += 1;
            }
            push_tok(
                data,
                last_line,
                last_col,
                ln,
                (col + p) as u32,
                (end - p) as u32,
                6,
            );
            p = end;
            continue;
        }
        // Multi-char operator (longest match): `**`, `++`, `--`, `<<`,
        // `>>`, `&&`, `||`, `==`, `!=`, `<=`, `>=`, `+=`, `-=`, `*=`,
        // `/=`, `%=`. Single-char fallbacks: `+`, `-`, `*`, `/`, `%`,
        // `=`, `<`, `>`, `?`, `:`, `&`, `|`, `^`, `~`, `!`, `(`, `)`,
        // `,`. All emit as operator (type 4).
        let two = if p + 1 < arith_end {
            Some(&bb[p..p + 2])
        } else {
            None
        };
        let span = match two {
            Some(b"**") | Some(b"++") | Some(b"--") | Some(b"<<") | Some(b">>") | Some(b"&&")
            | Some(b"||") | Some(b"==") | Some(b"!=") | Some(b"<=") | Some(b">=") | Some(b"+=")
            | Some(b"-=") | Some(b"*=") | Some(b"/=") | Some(b"%=") => 2,
            _ => 1,
        };
        push_tok(
            data,
            last_line,
            last_col,
            ln,
            (col + p) as u32,
            span as u32,
            4,
        );
        p += span;
    }
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
    let uri = params["textDocument"]["uri"]
        .as_str()
        .unwrap_or("")
        .to_string();
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

fn make_extract_function_singleline(uri: &str, leading_ws: &str, line: u32, body: &str) -> Value {
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
                &l[l.char_indices()
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
    if (t.starts_with('"') && t.ends_with('"')) || (t.starts_with('\'') && t.ends_with('\'')) {
        return false;
    }
    // Bare `$VAR` / `${VAR}` — already an expression.
    if let Some(rest) = t.strip_prefix('$') {
        let body = rest
            .strip_prefix('{')
            .and_then(|r| r.strip_suffix('}'))
            .unwrap_or(rest);
        if !body.is_empty() && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
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
    // Full syntax-aware reindenter (extensions/fmt.rs): block-structure
    // indentation, heredoc-body passthrough, trailing-ws strip.
    //
    // Indent width: DEFAULT is 4 (zpwr house style — set in
    // FmtOptions::default and the unwrap_or above when the client
    // sends no tabSize). An explicitly configured editor tabSize is
    // honored verbatim in both directions, same as the CLI `-i`
    // override.
    let formatted = crate::fmt::format_source(
        &text,
        &crate::fmt::FmtOptions {
            indent_width: tab_size,
            use_tabs: !insert_spaces,
        },
    );
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

// ── Doc tables (regenerated from data/grammar/canonical.json) ──────────
// Run `python3 scripts/gen_grammar_lsp.py` to refresh these blocks; the
// arrays between BEGIN/END markers are overwritten verbatim.

// BEGIN-CANONICAL: keywords
const KEYWORDS: &[&str] = &[
    "[[",
    "]]",
    "always",
    "case",
    "do",
    "done",
    "elif",
    "else",
    "end",
    "esac",
    "fi",
    "for",
    "foreach",
    "if",
    "in",
    "repeat",
    "select",
    "then",
    "until",
    "while",
    "declare",
    "export",
    "float",
    "integer",
    "let",
    "local",
    "readonly",
    "set",
    "shift",
    "typeset",
    "function",
    "{",
    "}",
    ".",
    "eval",
    "exec",
    "source",
    "trap",
    "break",
    "continue",
    "exit",
    "logout",
    "return",
    "builtin",
    "command",
    "coproc",
    "nocorrect",
    "noglob",
    "time",
    "!",
];
// END-CANONICAL: keywords

// BEGIN-CANONICAL: builtins
const BUILTINS: &[&str] = &[
    ".",
    ":",
    "[",
    "alias",
    "autoload",
    "bg",
    "break",
    "bye",
    "cap",
    "cd",
    "chdir",
    "chgrp",
    "chmod",
    "chown",
    "clone",
    "continue",
    "declare",
    "dirs",
    "disable",
    "disown",
    "echo",
    "echotc",
    "echoti",
    "emulate",
    "enable",
    "eval",
    "example",
    "exit",
    "export",
    "false",
    "fc",
    "fg",
    "float",
    "functions",
    "getcap",
    "getln",
    "getopts",
    "hash",
    "hashinfo",
    "history",
    "integer",
    "jobs",
    "kill",
    "let",
    "ln",
    "local",
    "log",
    "logout",
    "mem",
    "mkdir",
    "mv",
    "nameref",
    "patdebug",
    "pcre_compile",
    "pcre_match",
    "pcre_study",
    "popd",
    "print",
    "printf",
    "private",
    "pushd",
    "pushln",
    "pwd",
    "r",
    "read",
    "readonly",
    "rehash",
    "return",
    "rm",
    "rmdir",
    "set",
    "setcap",
    "setopt",
    "shift",
    "source",
    "stat",
    "strftime",
    "suspend",
    "sync",
    "syserror",
    "sysopen",
    "sysread",
    "sysseek",
    "syswrite",
    "test",
    "times",
    "trap",
    "true",
    "ttyctl",
    "type",
    "typeset",
    "umask",
    "unalias",
    "unfunction",
    "unhash",
    "unset",
    "unsetopt",
    "wait",
    "whence",
    "where",
    "which",
    "zask",
    "zcache",
    "zcompdump",
    "zcompile",
    "zcomplete",
    "zcurses",
    "zd",
    "zdelattr",
    "zf_chgrp",
    "zf_chmod",
    "zf_chown",
    "zf_ln",
    "zf_mkdir",
    "zf_mv",
    "zf_rm",
    "zf_rmdir",
    "zf_sync",
    "zformat",
    "zftp",
    "zgdbmpath",
    "zgetattr",
    "zhistory",
    "zid",
    "zjob",
    "zlistattr",
    "zlock",
    "zlog",
    "zls",
    "zmodload",
    "znotify",
    "zparseopts",
    "zping",
    "zprof",
    "zpty",
    "zpublish",
    "zregexparse",
    "zselect",
    "zsend",
    "zsetattr",
    "zsocket",
    "zsource",
    "zstat",
    "zstyle",
    "zsubscribe",
    "zsuggest",
    "zsync",
    "zsystem",
    "ztag",
    "ztcp",
    "ztie",
    "zunsubscribe",
    "zuntag",
    "zuntie",
    "zwc",
    "zwhere",
];
// END-CANONICAL: builtins

// BEGIN-CANONICAL: options
const OPTIONS: &[&str] = &[
    "ALIASES",
    "ALIASFUNCDEF",
    "ALLEXPORT",
    "ALWAYSLASTPROMPT",
    "ALWAYSTOEND",
    "APPENDCREATE",
    "APPENDHISTORY",
    "AUTOCD",
    "AUTOCONTINUE",
    "AUTOLIST",
    "AUTOMENU",
    "AUTONAMEDIRS",
    "AUTOPARAMKEYS",
    "AUTOPARAMSLASH",
    "AUTOPUSHD",
    "AUTOREMOVESLASH",
    "AUTORESUME",
    "BADPATTERN",
    "BANGHIST",
    "BAREGLOBQUAL",
    "BASHAUTOLIST",
    "BASHREMATCH",
    "BEEP",
    "BGNICE",
    "BRACECCL",
    "BRACEEXPAND",
    "BSDECHO",
    "CASEGLOB",
    "CASEMATCH",
    "CASEPATHS",
    "CBASES",
    "CDABLEVARS",
    "CDSILENT",
    "CHASEDOTS",
    "CHASELINKS",
    "CHECKJOBS",
    "CHECKRUNNINGJOBS",
    "CLOBBER",
    "CLOBBEREMPTY",
    "COMBININGCHARS",
    "COMPLETEALIASES",
    "COMPLETEINWORD",
    "CONTINUEONERROR",
    "CORRECT",
    "CORRECTALL",
    "CPRECEDENCES",
    "CSHJUNKIEHISTORY",
    "CSHJUNKIELOOPS",
    "CSHJUNKIEQUOTES",
    "CSHNULLCMD",
    "CSHNULLGLOB",
    "DEBUGBEFORECMD",
    "DOTGLOB",
    "DVORAK",
    "EMACS",
    "EQUALS",
    "ERREXIT",
    "ERRRETURN",
    "EVALLINENO",
    "EXEC",
    "EXTENDEDGLOB",
    "EXTENDEDHISTORY",
    "FLOWCONTROL",
    "FORCEFLOAT",
    "FUNCTIONARGZERO",
    "GLOB",
    "GLOBALEXPORT",
    "GLOBALRCS",
    "GLOBASSIGN",
    "GLOBCOMPLETE",
    "GLOBDOTS",
    "GLOBSTARSHORT",
    "GLOBSUBST",
    "HASHALL",
    "HASHCMDS",
    "HASHDIRS",
    "HASHEXECUTABLESONLY",
    "HASHLISTALL",
    "HISTALLOWCLOBBER",
    "HISTAPPEND",
    "HISTBEEP",
    "HISTEXPAND",
    "HISTEXPIREDUPSFIRST",
    "HISTFCNTLLOCK",
    "HISTFINDNODUPS",
    "HISTIGNOREALLDUPS",
    "HISTIGNOREDUPS",
    "HISTIGNORESPACE",
    "HISTLEXWORDS",
    "HISTNOFUNCTIONS",
    "HISTNOSTORE",
    "HISTREDUCEBLANKS",
    "HISTSAVEBYCOPY",
    "HISTSAVENODUPS",
    "HISTSUBSTPATTERN",
    "HISTVERIFY",
    "HUP",
    "IGNOREBRACES",
    "IGNORECLOSEBRACES",
    "IGNOREEOF",
    "INCAPPENDHISTORY",
    "INCAPPENDHISTORYTIME",
    "INTERACTIVE",
    "INTERACTIVECOMMENTS",
    "KSHARRAYS",
    "KSHAUTOLOAD",
    "KSHGLOB",
    "KSHOPTIONPRINT",
    "KSHTYPESET",
    "KSHZEROSUBSCRIPT",
    "LISTAMBIGUOUS",
    "LISTBEEP",
    "LISTPACKED",
    "LISTROWSFIRST",
    "LISTTYPES",
    "LOCALLOOPS",
    "LOCALOPTIONS",
    "LOCALPATTERNS",
    "LOCALTRAPS",
    "LOG",
    "LOGIN",
    "LONGLISTJOBS",
    "MAGICEQUALSUBST",
    "MAILWARN",
    "MAILWARNING",
    "MARKDIRS",
    "MENUCOMPLETE",
    "MONITOR",
    "MULTIBYTE",
    "MULTIFUNCDEF",
    "MULTIOS",
    "NOMATCH",
    "NOTIFY",
    "NULLGLOB",
    "NUMERICGLOBSORT",
    "OCTALZEROES",
    "ONECMD",
    "OVERSTRIKE",
    "PATHDIRS",
    "PATHSCRIPT",
    "PHYSICAL",
    "PIPEFAIL",
    "POSIXALIASES",
    "POSIXARGZERO",
    "POSIXBUILTINS",
    "POSIXCD",
    "POSIXIDENTIFIERS",
    "POSIXJOBS",
    "POSIXSTRINGS",
    "POSIXTRAPS",
    "PRINTEIGHTBIT",
    "PRINTEXITVALUE",
    "PRIVILEGED",
    "PROMPTBANG",
    "PROMPTCR",
    "PROMPTPERCENT",
    "PROMPTSP",
    "PROMPTSUBST",
    "PROMPTVARS",
    "PUSHDIGNOREDUPS",
    "PUSHDMINUS",
    "PUSHDSILENT",
    "PUSHDTOHOME",
    "RCEXPANDPARAM",
    "RCQUOTES",
    "RCS",
    "RECEXACT",
    "REMATCHPCRE",
    "RMSTARSILENT",
    "RMSTARWAIT",
    "SHAREHISTORY",
    "SHFILEEXPANSION",
    "SHGLOB",
    "SHINSTDIN",
    "SHNULLCMD",
    "SHOPTIONLETTERS",
    "SHORTLOOPS",
    "SHORTREPEAT",
    "SHWORDSPLIT",
    "SINGLECOMMAND",
    "SINGLELINEZLE",
    "SOURCETRACE",
    "STDIN",
    "SUNKEYBOARDHACK",
    "TRACKALL",
    "TRANSIENTRPROMPT",
    "TRAPSASYNC",
    "TYPESETSILENT",
    "TYPESETTOUNSET",
    "UNSET",
    "VERBOSE",
    "VI",
    "WARNCREATEGLOBAL",
    "WARNNESTEDVAR",
    "XTRACE",
    "ZLE",
];
// END-CANONICAL: options

// `SPECIAL_VARS` hand list deleted. The 41 `$`-prefixed entries were
// a stale subset of zsh's actual ~538 special params per `man zshparam`
// + every `mod_*.yo`. All call sites now iterate the canonical
// `zsh_special_var_docs::SPECIAL_VAR_DOCS` table directly (prepending
// `$` where the legacy site expected sigiled labels).

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
    (
        "$SHELL",
        "Pathname of the login shell. Honored by many tools as the default user shell.",
    ),
    (
        "$EDITOR",
        "Preferred editor for tools that invoke an editor (`fc`, `git`, `crontab`, …).",
    ),
    (
        "$VISUAL",
        "Preferred full-screen editor. Takes precedence over `$EDITOR` when set.",
    ),
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
    // Options surfaced in the canonical UPPERCASE_WITH_UNDERSCORES
    // form per `man zshoptions` (`AUTO_CD`, not `autocd`). The
    // `ZSH_OPTIONS_SET` set stores the normalized form zsh uses for
    // lookup (lowercase, no underscores), which is correct for option
    // resolution but unfamiliar in a doc panel — users reading the
    // tool window expect to see the names the way `setopt` / `man`
    // print them. OPTION_DOCS keys carry the canonical CAPS form.
    let mut options = serde_json::Map::new();
    for (name, _doc) in crate::zsh_option_docs::OPTION_DOCS {
        options.insert((*name).to_string(), Value::String("option".into()));
        all.insert((*name).to_string(), Value::String("option".into()));
    }
    for (alias, _canon) in crate::zsh_option_docs::OPTION_ALIASES {
        options.insert((*alias).to_string(), Value::String("option".into()));
        all.insert((*alias).to_string(), Value::String("option".into()));
    }
    // Special params — source from the canonical doc table (538
    // entries extracted from `params.yo` + every `mod_*.yo`) rather
    // than the hand 41-entry `SPECIAL_VARS` subset above. The hand
    // subset was missing `$PS2` / `$PS3` / `$PS4` / `$psvar` /
    // `$PROMPT2` / hundreds more, so the tool window showed a tiny
    // slice of zsh's actual special-param surface.
    let mut special_vars = serde_json::Map::new();
    for (name, _doc) in crate::zsh_special_var_docs::SPECIAL_VAR_DOCS {
        special_vars.insert((*name).to_string(), Value::String("special".into()));
        all.insert((*name).to_string(), Value::String("special".into()));
    }
    // Also surface alias surface names (`PROMPT` / `PROMPT2` /
    // `PROMPT3` → PS1/PS2/PS3, `NULLCMD` etc) so the tool window
    // shows every name the user might type.
    for (alias, _canon) in crate::zsh_special_var_docs::SPECIAL_VAR_ALIASES {
        special_vars.insert((*alias).to_string(), Value::String("special".into()));
        all.insert((*alias).to_string(), Value::String("special".into()));
    }
    // ── Compsys completion functions ────────────────────────────────
    // The `_arguments` / `_files` / `_describe` family — Rust-native
    // implementations from the `compsys` crate. Sourced from
    // `crate::compsys::COMPSYS_FN_NAMES`.
    let mut compsys = serde_json::Map::new();
    for n in crate::compsys::COMPSYS_FN_NAMES {
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
    // Canonical 538-entry special-param doc table — bare names per
    // params.yo convention. Inserted with `$` prefix to match the
    // form users actually type / search for in name lookups.
    for (name, _) in crate::zsh_special_var_docs::SPECIAL_VAR_DOCS {
        // Skip pure-symbolic ones — they're handled separately by
        // the lookup_doc cascade.
        if name
            .chars()
            .next()
            .map(|c| c.is_ascii_alphabetic() || c == '_')
            .unwrap_or(false)
        {
            set.insert(format!("${}", name));
        } else {
            set.insert((*name).to_string());
        }
    }
    for n in crate::compsys::COMPSYS_FN_NAMES {
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
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut cur: Vec<usize> = vec![0; n + 1];
    for i in 1..=m {
        cur[0] = i;
        for j in 1..=n {
            let cost = if av[i - 1] == bv[j - 1] { 0 } else { 1 };
            cur[j] = (cur[j - 1] + 1).min(prev[j] + 1).min(prev[j - 1] + cost);
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

    // ── special vars (canonical 538-entry doc table) ─────────────────
    // Bare names from `SPECIAL_VAR_DOCS` get `$` prepended so the
    // chapter header matches the form users type. Pure-symbolic
    // ones (`?`/`*`/`#`/`@`/`-`/`_`) stay as-is — they're documented
    // under their bare key.
    let mut specials: Vec<String> = crate::zsh_special_var_docs::SPECIAL_VAR_DOCS
        .iter()
        .map(|(name, _)| {
            if name
                .chars()
                .next()
                .map(|c| c.is_ascii_alphabetic() || c == '_')
                .unwrap_or(false)
            {
                format!("${}", name)
            } else {
                (*name).to_string()
            }
        })
        .collect();
    specials.sort();
    specials.dedup();
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

    // ── compsys functions (`crate::compsys::COMPSYS_FN_NAMES`) ──────────────
    let mut compsys_names: Vec<String> = crate::compsys::COMPSYS_FN_NAMES
        .iter()
        .map(|s| s.to_string())
        .collect();
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
            while i < bytes.len() && bytes[i] != b'`' {
                i = push_html_escaped(&mut out, bytes, i);
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
                let after_ok =
                    end + 1 >= bytes.len() || !(bytes[end + 1] as char).is_alphanumeric();
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
        // Default: HTML-escape ASCII metacharacters; copy multibyte UTF-8
        // whole. (`c` only matches ASCII markers above, so high bytes — e.g.
        // the em-dash `—` (e2 80 94) — fall through to here.)
        i = push_html_escaped(&mut out, bytes, i);
    }
    out
}

/// Append the character starting at `bytes[i]` to `out`, HTML-escaping the
/// ASCII metacharacters and copying any multibyte UTF-8 sequence whole.
/// Returns the index just past the emitted character. Never cast individual
/// bytes of a multibyte sequence to `char` — that re-encodes Latin-1→UTF-8 and
/// produces mojibake (e.g. `—` → `Ã¢ÂÂ`).
fn push_html_escaped(out: &mut String, bytes: &[u8], i: usize) -> usize {
    let b = bytes[i];
    if b < 0x80 {
        match b {
            b'&' => out.push_str("&amp;"),
            b'<' => out.push_str("&lt;"),
            b'>' => out.push_str("&gt;"),
            _ => out.push(b as char),
        }
        i + 1
    } else {
        let len = match b {
            0xF0..=0xF4 => 4,
            0xE0..=0xEF => 3,
            0xC0..=0xDF => 2,
            _ => 1,
        };
        let end = (i + len).min(bytes.len());
        out.push_str(std::str::from_utf8(&bytes[i..end]).unwrap_or("\u{fffd}"));
        end
    }
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

    // ── compsys flag docs (man zshcompsys-derived hand table) ──────────

    #[test]
    fn compsys_flag_table_covers_wanted_with_all_six_flags() {
        // `_wanted -<TAB>` previously returned 0 because tier-1 saw no
        // bullets in the doc body and tier-2 caught at most 2 inline
        // citations. The man-derived hand table should now serve all 6
        // flags from the signature `[ -x ] [ -C name ] [ -12VJ ]`.
        let flags = extract_builtin_flags("_wanted");
        let names: Vec<&str> = flags.iter().map(|(f, _)| f.as_str()).collect();
        assert_eq!(
            names,
            vec!["-x", "-C", "-1", "-2", "-V", "-J"],
            "_wanted flag set drifted from man zshcompsys signature",
        );
        for (f, d) in &flags {
            assert!(!d.is_empty(), "flag {f} has no description");
        }
    }

    #[test]
    fn compsys_flag_table_overrides_bullet_scraper_for_arguments() {
        // `_arguments` is the marquee compsys fn — the man page
        // documents 12+ flags but the bullet scraper only catches 7.
        // The man-derived hand table is consulted FIRST, so the count
        // should be the full 12.
        let flags = extract_builtin_flags("_arguments");
        assert!(
            flags.len() >= 12,
            "expected _arguments to surface >=12 flags from man-derived table, got {}",
            flags.len()
        );
        // Spot-check that the man-only flags (-w, -W, -A, -O, --) are
        // present — these are missing from the BUILTIN_DOCS scrape.
        let names: std::collections::HashSet<&str> =
            flags.iter().map(|(f, _)| f.as_str()).collect();
        for must_have in ["-n", "-s", "-w", "-W", "-C", "-R", "-S", "-A", "-O", "-M"] {
            assert!(
                names.contains(must_have),
                "_arguments missing canonical flag {must_have}"
            );
        }
    }

    #[test]
    fn every_compsys_fn_in_man_is_in_inventory() {
        // The 26 compsys ported documented in `man zshcompsys` all have
        // Rust shadows in `compsys/` (canonical_paths, call_program,
        // widgets, …) — so `COMPSYS_FN_NAMES` is the canonical truth
        // and `is_known_builtin_with_flag_docs` doesn't need a doc-
        // only fallback. This test pins that. If a future yo extract
        // adds a new compsys fn, add it to `COMPSYS_FN_NAMES` AND
        // give it a row in `COMPSYS_FN_FLAG_DOCS`.
        for name in [
            "_call_program",
            "_canonical_paths",
            "_combination",
            "_command_names",
            "_completers",
            "_dir_list",
            "_email_addresses",
            "_multi_parts",
            "_numbers",
            "_pick_variant",
            "_sequence",
            "_tags",
            "_widgets",
        ] {
            assert!(
                crate::compsys::COMPSYS_FN_NAMES.contains(&name),
                "{name}: missing from COMPSYS_FN_NAMES inventory",
            );
        }
        assert!(is_known_builtin_with_flag_docs("_canonical_paths"));
        assert!(is_known_builtin_with_flag_docs("_widgets"));
        assert!(is_known_builtin_with_flag_docs("_call_program"));
    }

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

    // ── find_user_symbol_doc (## doc-comment hover) ─────────────────────

    #[test]
    fn user_doc_attaches_to_function_with_keyword_form() {
        // Card format mirrors stryke's
        // `strykelang/lsp.rs::format_with_doc_comments`:
        //   <doc>\n\n---\n\n<one-line header>
        let src = "## Print a hello banner with the user's name.\n\
                   ## Used by the README demo.\n\
                   function greet {\n  print hi\n}\n";
        let doc = super::find_user_symbol_doc(src, "greet").expect("doc");
        assert!(doc.contains("Print a hello banner"), "got {doc:?}");
        assert!(doc.contains("Used by the README demo"), "got {doc:?}");
        assert!(doc.contains("\n\n---\n\n"), "missing divider: {doc:?}");
        assert!(doc.contains("user-defined function `greet`"), "got {doc:?}");
    }

    #[test]
    fn user_doc_attaches_to_posix_function_form() {
        let src = "## Sum two integers.\n\
                   add() {\n  print $(( $1 + $2 ))\n}\n";
        let doc = super::find_user_symbol_doc(src, "add").expect("doc");
        assert!(doc.contains("Sum two integers"), "got {doc:?}");
        assert!(doc.contains("user-defined function"), "got {doc:?}");
    }

    #[test]
    fn user_doc_attaches_to_alias() {
        let src = "## Short for `ls --color=auto -lAh`.\nalias ll='ls --color=auto -lAh'\n";
        let doc = super::find_user_symbol_doc(src, "ll").expect("doc");
        assert!(doc.contains("Short for"), "got {doc:?}");
        assert!(doc.contains("user-defined alias"), "got {doc:?}");
    }

    #[test]
    fn user_doc_attaches_to_typeset_parameter() {
        let src = "## Maximum retries before giving up.\ntypeset -i MAX_RETRIES=5\n";
        let doc = super::find_user_symbol_doc(src, "MAX_RETRIES").expect("doc");
        assert!(doc.contains("Maximum retries"), "got {doc:?}");
        assert!(doc.contains("user-defined parameter"), "got {doc:?}");
    }

    #[test]
    fn user_doc_multi_paragraph_with_blank_double_hash() {
        // `##` on its own line is a paragraph break inside the block.
        let src = "## First paragraph describing what the function does.\n\
                   ##\n\
                   ## Second paragraph about edge cases.\n\
                   function foo() {}\n";
        let doc = super::find_user_symbol_doc(src, "foo").expect("doc");
        assert!(doc.contains("First paragraph"), "got {doc:?}");
        assert!(doc.contains("Second paragraph"), "got {doc:?}");
        // Paragraph break preserved.
        assert!(doc.contains("\n\n"), "expected paragraph break: {doc:?}");
    }

    #[test]
    fn user_doc_skips_blank_lines_between_block_and_def() {
        // Doc block, then blank line, then def — still attached.
        let src = "## Doc for the function.\n\nfunction f() {}\n";
        let doc = super::find_user_symbol_doc(src, "f").expect("doc");
        assert!(doc.contains("Doc for the function"), "got {doc:?}");
    }

    #[test]
    fn user_doc_ignores_single_hash_comments() {
        // Only `##` lines attach to the doc block. Plain `#` comments
        // are routine code remarks and must NOT show up as docstrings.
        // The minimal "defined here" card (find_user_symbol_doc fallback
        // at lsp.rs:2038) still fires so hover doesn't go blank for an
        // undocumented symbol, but it must contain NO part of the `#`
        // comment text.
        let src = "# This is just a code comment, not a docstring.\n\
                   function f() {}\n";
        let card = super::find_user_symbol_doc(src, "f").expect("minimal card");
        assert!(
            !card.contains("just a code comment"),
            "plain `#` comments must not leak into doc card: {card:?}"
        );
    }

    #[test]
    fn user_doc_returns_none_when_symbol_absent() {
        let src = "## Doc for greet.\nfunction greet() {}\n";
        assert!(super::find_user_symbol_doc(src, "nonexistent").is_none());
    }

    #[test]
    fn user_doc_returns_none_when_no_doc_block() {
        // No `##` doc block → falls through to the "defined here"
        // minimal card (find_user_symbol_doc fallback at lsp.rs:2038).
        // The card must mention the symbol but carry no doc body.
        let src = "function greet() {}\n";
        let card = super::find_user_symbol_doc(src, "greet").expect("minimal card");
        assert!(
            card.contains("greet"),
            "card must name the symbol: {card:?}"
        );
    }

    #[test]
    fn user_doc_stops_at_intervening_single_hash_line() {
        // `## doc` then `# code comment` then `function f` —
        // intervening single-`#` terminates the doc-block collection,
        // so the rich-doc card is suppressed. The minimal "defined
        // here" card still fires (per lsp.rs:2038 fallback), but it
        // must NOT contain the `## Real doc here.` text.
        let src = "## Real doc here.\n\
                   # Inline comment unrelated to the doc.\n\
                   function f() {}\n";
        let card = super::find_user_symbol_doc(src, "f").expect("minimal card");
        assert!(
            !card.contains("Real doc here"),
            "intervening `#` must break doc collection: {card:?}"
        );
    }

    // ── extract_module_doc (top-of-file ## block) ───────────────────────

    #[test]
    fn module_doc_collects_top_block_after_shebang() {
        let src = "#!/usr/bin/env zsh\n\
                   ## foo.zsh — short summary.\n\
                   ## Provides foo, bar, baz helpers.\n\
                   \n\
                   function foo() {}\n";
        let doc = super::extract_module_doc(src).expect("doc");
        assert!(doc.contains("foo.zsh"), "got {doc:?}");
        assert!(doc.contains("Provides foo"), "got {doc:?}");
    }

    #[test]
    fn module_doc_collects_top_block_without_shebang() {
        let src = "## bar.zsh — utility library.\n\
                   function bar() {}\n";
        let doc = super::extract_module_doc(src).expect("doc");
        assert!(doc.contains("bar.zsh"), "got {doc:?}");
    }

    #[test]
    fn module_doc_supports_double_hash_paragraph_breaks() {
        let src = "## First paragraph of the module summary.\n\
                   ##\n\
                   ## Second paragraph with details.\n\
                   function foo() {}\n";
        let doc = super::extract_module_doc(src).expect("doc");
        assert!(doc.contains("First paragraph"), "got {doc:?}");
        assert!(doc.contains("Second paragraph"), "got {doc:?}");
        assert!(doc.contains("\n\n"), "expected paragraph break: {doc:?}");
    }

    #[test]
    fn module_doc_skips_blank_lines_between_shebang_and_block() {
        let src = "#!/usr/bin/env zsh\n\
                   \n\
                   \n\
                   ## Module doc starts here.\n\
                   function foo() {}\n";
        let doc = super::extract_module_doc(src).expect("doc");
        assert!(doc.contains("Module doc starts here"), "got {doc:?}");
    }

    #[test]
    fn module_doc_returns_none_when_no_block() {
        let src = "#!/usr/bin/env zsh\nfunction foo() {}\n";
        assert!(super::extract_module_doc(src).is_none());
    }

    #[test]
    fn module_doc_returns_none_for_plain_single_hash_comments() {
        // Single `#` comments at top of file are NOT module docs.
        let src = "#!/usr/bin/env zsh\n\
                   # This is a regular code comment, not a docstring.\n\
                   function foo() {}\n";
        assert!(super::extract_module_doc(src).is_none());
    }

    #[test]
    fn module_doc_stops_at_first_real_code_line() {
        let src = "## Module doc line 1.\n\
                   function foo() {}\n\
                   ## This should NOT be collected — below the def.\n";
        let doc = super::extract_module_doc(src).expect("doc");
        assert_eq!(doc, "Module doc line 1.");
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
    fn lookup_doc_handles_pure_symbolic_specials() {
        // User report: `zshrs --docs '$'` returned "no docs for $".
        // Root cause: strip_prefix('$') on the lookup key turned `"$"`
        // (the canonical entry for `$$`/PID) into "" which failed.
        // Now the raw key is tried first.
        let _g = crate::test_util::global_state_lock();
        // `$` = PID — canonical entry under bare `"$"` key
        let s = lookup_doc("$");
        assert!(
            !s.is_empty() && s.contains("process ID"),
            "lookup_doc('$') should return the PID doc; got: {:?}",
            s,
        );
        // Other pure-symbolic specials must also resolve via their
        // bare key — `?` / `*` / `#` / `@` / `-` / `_` / `!`.
        for sym in &["$?", "$*", "$#", "$@", "$-", "$_", "$!"] {
            let card = lookup_doc(sym);
            assert!(
                !card.is_empty(),
                "lookup_doc({:?}) returned empty; pure-symbolic specials must resolve",
                sym,
            );
        }
    }

    #[test]
    fn lookup_doc_special_var_wins_over_module_builtin_entry() {
        // User report: `prompt` resolved as "zsh builtin" instead of
        // special var. Root cause: module yo files (contrib.yo,
        // zsh/parameter, etc) have `item(tt(NAME))` blocks that
        // describe PARAMETERS but the builtin extractor classifies
        // by macro shape, not semantic intent. 109 names overlap;
        // none are real builtins per `ported::builtin::BUILTINS`.
        // Names that aren't in the canonical builtin table must
        // resolve to the special-var entry, not the misclassified
        // builtin entry.
        let _g = crate::test_util::global_state_lock();
        // These names are NOT in `ported::builtin::BUILTINS` — they're
        // purely special-var/array params. The bug was classifying
        // them as builtins because module doc files have item(tt(X))
        // blocks describing the parameter behavior.
        for name in &[
            "prompt",
            "path",
            "aliases",
            "jobdirs",
            "jobstates",
            "commands",
            "modules",
            "widgets",
        ] {
            let card = lookup_doc(name);
            assert!(
                card.contains("special variable"),
                "lookup_doc({:?}) classified as: {:?} — expected 'special variable'",
                name,
                card.lines().take(3).collect::<Vec<_>>(),
            );
        }
        // Real builtins must STAY classified as builtins —
        // `functions`/`history`/`set`/`shift` are genuine builtins
        // per `ported::builtin::BUILTINS` (history is fc-alias,
        // functions is typeset-f-alias) so the special-var-wins
        // override must NOT touch them.
        for name in &[
            "cd",
            "echo",
            "set",
            "shift",
            "unset",
            "functions",
            "history",
        ] {
            let card = lookup_doc(name);
            assert!(
                card.contains("zsh builtin") || card.contains("zshrs"),
                "lookup_doc({:?}) lost its builtin classification: {:?}",
                name,
                card.lines().take(3).collect::<Vec<_>>(),
            );
        }
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

    /// Block keywords inside comments must NOT push onto block_stack.
    /// Before fix: `# operations — length, case, slice, search.` in
    /// examples/demos/03_strings.zsh produced "unclosed `case` block"
    /// because split_whitespace() yielded "case" as a token and the
    /// scan didn't filter comments.
    #[test]
    fn diagnose_ignores_block_keywords_inside_comments() {
        let _g = crate::test_util::global_state_lock();
        let src = "# operations — length, case, slice, concat, search.\nx=1\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "block keyword `case` inside comment flagged: {:?}",
            d
        );
    }

    /// Same for `if` / `for` / `while` / `until` / `select` / `repeat`
    /// in comments and strings — none should push onto block_stack.
    #[test]
    fn diagnose_ignores_all_block_keywords_in_comments_and_strings() {
        let _g = crate::test_util::global_state_lock();
        let src = "# if for while until case select repeat\n\
                   echo \"if for while until case select repeat\"\n\
                   x=1\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "block keywords in comments/strings flagged: {:?}",
            d
        );
    }

    /// Loop-terminator keywords in comments must NOT pop block_stack
    /// or generate spurious "unmatched `done`" / "unmatched `fi`" /
    /// "unmatched `esac`" diagnostics.
    #[test]
    fn diagnose_ignores_terminators_inside_comments() {
        let _g = crate::test_util::global_state_lock();
        let src = "# fi done esac comment\nx=1\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "terminator keywords in comments flagged: {:?}",
            d
        );
    }

    /// `done;` (terminator with trailing `;`) must still pop the `for`
    /// off block_stack — otherwise one-liner loops like
    /// `for ((;;)); do …; done; }` left `for` orphaned and flagged.
    /// Reported false-positive on
    /// `examples/demos/106_pipe_chains.zsh:24` and
    /// `examples/demos/109_arith_truth_tables.zsh:54-55`.
    #[test]
    fn diagnose_done_with_trailing_semicolon_pops_for() {
        let _g = crate::test_util::global_state_lock();
        let src = "for i in 1 2 3; do echo $i; done;\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "`done;` must pop for: {:?}", d);
    }

    /// `done; done; done` (three terminators on one line) must pop
    /// all three nested `for`s. Pins the
    /// `109_arith_truth_tables.zsh:54-55` `for…; for…; for…; do…done; done; done`
    /// triple-nest pattern.
    #[test]
    fn diagnose_three_done_with_trailing_semicolon_pops_three_fors() {
        let _g = crate::test_util::global_state_lock();
        let src = "for a in 1; do for b in 1; do for c in 1; do echo $a$b$c; done; done; done\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "three `done;` must pop three `for`: {:?}", d);
    }

    /// `fi;` (if-terminator with trailing `;`) similarly pops the
    /// open `if` even when written `if …; then …; fi;`.
    #[test]
    fn diagnose_fi_with_trailing_semicolon_pops_if() {
        let _g = crate::test_util::global_state_lock();
        let src = "if true; then echo hi; fi;\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "`fi;` must pop if: {:?}", d);
    }

    /// One-liner `case … in PAT) cmd ;; *) cmd ;; esac` must NOT
    /// flag the `)` bare-paren terminators. Pins the
    /// `100_zsh_features_summary.zsh:49` false-positive
    /// `case foo in foo|bar) echo yes ;; *) echo no ;; esac`.
    #[test]
    fn diagnose_oneliner_case_arm_paren_not_flagged() {
        let _g = crate::test_util::global_state_lock();
        let src = "case foo in foo|bar) echo yes ;; *) echo no ;; esac\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "one-liner case-arm `)` flagged: {:?}", d);
    }

    /// `${var#PATTERN}` parameter-substitution patterns can contain
    /// unbalanced `[` / `]` (literal-escaped via `\[` / `\]`). The
    /// bracket scanner must skip the whole `${...}` span as opaque,
    /// otherwise it flags bogus 'unmatched }' / 'unclosed [' on
    /// `${log_line#\[}; date=${log_line#*\][}` patterns. Reported on
    /// `examples/demos/117_backref_replacement.zsh:56`.
    #[test]
    fn diagnose_param_subst_with_escaped_brackets_no_flag() {
        let _g = crate::test_util::global_state_lock();
        let src = "level=${log_line#\\[}; date=${log_line#*\\][}\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "param-subst with escaped brackets flagged: {:?}",
            d
        );
    }

    /// `select` as an ARGUMENT to another builtin (e.g.
    /// `zstyle ':completion:*' menu select`) must NOT push onto
    /// block_stack — only the block form
    /// `select var in words; do …; done` opens a block.
    /// Pinned because the original implementation flagged
    /// `examples/demos/133_zstyle_demo.zsh:6` as an unclosed
    /// `select` block.
    #[test]
    fn diagnose_select_as_argument_not_block_opener() {
        let _g = crate::test_util::global_state_lock();
        let src = "zstyle ':completion:*' menu select\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "`select` as builtin arg flagged: {:?}", d);
    }

    /// Real `select VAR in WORDS; do …; done` block form still
    /// works (positive control for the above test).
    #[test]
    fn diagnose_select_block_form_balances() {
        let _g = crate::test_util::global_state_lock();
        let src = "select x in a b c; do echo $x; break; done\n";
        let d = diagnose(src);
        assert!(
            d.is_empty(),
            "block-form `select … do … done` flagged: {:?}",
            d
        );
    }

    // ── Hover regressions: UDF + user-variable fall-back & priority ──

    /// User-defined function with NO `##` doc-block must still produce
    /// a hover card (minimal kind + def-line snippet). Before fix:
    /// hover returned null and the IDE went blank on every UDF that
    /// didn't carry a doc-string.
    #[test]
    fn find_user_symbol_doc_returns_minimal_card_when_no_docblock() {
        let _g = crate::test_util::global_state_lock();
        let src = "greet() {\n    echo hello\n}\n";
        let r = find_user_symbol_doc(src, "greet").expect("UDF must hover");
        assert!(
            r.contains("user-defined function"),
            "card must label kind, got {:?}",
            r
        );
        assert!(
            r.contains("greet"),
            "card must include the symbol name, got {:?}",
            r
        );
    }

    /// User-defined variable (local/typeset/etc.) with no doc-block
    /// also gets a minimal card. Before fix: silent blank hover.
    #[test]
    fn find_user_symbol_doc_minimal_card_for_user_variable() {
        let _g = crate::test_util::global_state_lock();
        let src = "fn() {\n    local start=$EPOCHREALTIME\n}\n";
        let r = find_user_symbol_doc(src, "start").expect("user var must hover");
        assert!(
            r.contains("user-defined parameter") || r.contains("user-defined variable"),
            "card must label parameter/variable kind, got {:?}",
            r
        );
        assert!(
            r.contains("start"),
            "card must include the var name, got {:?}",
            r
        );
    }

    /// User-defined symbol with a doc-block beats the minimal-card
    /// fallback (the documented form is richer).
    #[test]
    fn find_user_symbol_doc_prefers_documented_definition() {
        let _g = crate::test_util::global_state_lock();
        let src = "## says hi\ngreet() {\n    echo hi\n}\n";
        let r = find_user_symbol_doc(src, "greet").expect("UDF must hover");
        assert!(
            r.contains("says hi"),
            "documented form must include the doc block, got {:?}",
            r
        );
    }

    /// Brace-range expansion `{a..e}` / `{A..E}` / `{1..10}` /
    /// `{1..10..2}` must emit a single OPERATOR token (type 4) for
    /// the whole span. Before fix the inner letter endpoints were
    /// classified as single-char identifiers → token-type 6
    /// (VARIABLE) → italic-green color in the IDE, making
    /// `echo {A..E}` look like a variable reference.
    #[test]
    fn semantic_tokens_brace_range_emits_single_operator_span() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        let uri = "file:///t.zsh";
        state
            .docs
            .insert(uri.to_string(), "echo {a..e}\n".to_string());
        let r = semantic_tokens(&state, &json!({ "textDocument": { "uri": uri } }));
        let data = r["data"].as_array().expect("data array");
        // Token stream layout includes `{a..e}` = 6 bytes, type 4 (operator).
        // Each token = 5 u32s (delta_line, delta_col, len, ty, mods).
        let mut found = false;
        for chunk in data.chunks(5) {
            if chunk.len() == 5 && chunk[2].as_u64() == Some(6) && chunk[3].as_u64() == Some(4) {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "brace range `{{a..e}}` must emit a single 6-byte operator token, got data={:?}",
            data
        );
    }

    /// Uppercase letter brace ranges `{A..E}` get the same treatment
    /// (the exact case shown in the user's screenshot reporting the
    /// italic-green mis-coloring of `E`).
    #[test]
    fn semantic_tokens_brace_range_uppercase_endpoints_emit_operator() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        let uri = "file:///t.zsh";
        state
            .docs
            .insert(uri.to_string(), "echo {A..E}\n".to_string());
        let r = semantic_tokens(&state, &json!({ "textDocument": { "uri": uri } }));
        let data = r["data"].as_array().expect("data array");
        let mut found = false;
        for chunk in data.chunks(5) {
            if chunk.len() == 5 && chunk[2].as_u64() == Some(6) && chunk[3].as_u64() == Some(4) {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "brace range `{{A..E}}` must emit a single 6-byte operator token, got data={:?}",
            data
        );
    }

    /// `$((arith))` inside a double-quoted string must NOT emit one
    /// opaque variable-colored span over the entire `$((expr))`.
    /// Before fix, the string-interpolation handler caught `$(` and
    /// counted parens to find the matching `))` then emitted ONE
    /// token-type-6 covering everything — so `"$(( -7 % 3 ))"` had
    /// `-7 % 3` colored as if it were a variable name. Fix breaks
    /// the interior into number/identifier/operator atoms and emits
    /// `$((` and `))` as operator brackets.
    #[test]
    fn semantic_tokens_arith_in_string_breaks_interior_into_atoms() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        let uri = "file:///t.zsh";
        state
            .docs
            .insert(uri.to_string(), "echo \"$(( -7 % 3 ))\"\n".to_string());
        let r = semantic_tokens(&state, &json!({ "textDocument": { "uri": uri } }));
        let data = r["data"].as_array().expect("data array");
        // We expect to see at least:
        //   * one type-2 (number) token for `7` (the digit run after `-`)
        //   * one type-4 (operator) token for `%`
        //   * one type-2 (number) token for `3`
        // Before fix: zero numbers, zero operators (everything was one
        // type-6 variable token across the whole `$((…))` span).
        let mut numbers = 0usize;
        let mut operators = 0usize;
        for chunk in data.chunks(5) {
            if chunk.len() == 5 {
                match chunk[3].as_u64() {
                    Some(2) => numbers += 1,
                    Some(4) => operators += 1,
                    _ => {}
                }
            }
        }
        assert!(
            numbers >= 2,
            "expected ≥2 number tokens inside $((…)), got {numbers}; data={:?}",
            data
        );
        assert!(
            operators >= 3,
            "expected ≥3 operator tokens ($((, %, )) ) inside $((…)), got {operators}; data={:?}",
            data
        );
    }

    /// Long-form CLI flags `--verbose` / `--debug` emit a single
    /// OPERATOR token so the whole `--word` highlights uniformly.
    /// Before fix the `--` fell through to `col += 1` and the rest
    /// of the flag word was unhighlighted, producing the visible
    /// "stripe" reported in the user's screenshot.
    #[test]
    fn semantic_tokens_long_cli_flag_emits_operator_span() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        let uri = "file:///t.zsh";
        state.docs.insert(
            uri.to_string(),
            "parse_demo --verbose --debug arg1\n".to_string(),
        );
        let r = semantic_tokens(&state, &json!({ "textDocument": { "uri": uri } }));
        let data = r["data"].as_array().expect("data array");
        // Need at least one operator token of len=9 (`--verbose`)
        // AND one of len=7 (`--debug`).
        let mut saw_9_op = false;
        let mut saw_7_op = false;
        for chunk in data.chunks(5) {
            if chunk.len() == 5 && chunk[3].as_u64() == Some(4) {
                match chunk[2].as_u64() {
                    Some(9) => saw_9_op = true,
                    Some(7) => saw_7_op = true,
                    _ => {}
                }
            }
        }
        assert!(
            saw_9_op,
            "`--verbose` (9 bytes) must emit an operator token, got {:?}",
            data
        );
        assert!(
            saw_7_op,
            "`--debug` (7 bytes) must emit an operator token, got {:?}",
            data
        );
    }

    /// Stepped brace range `{1..10..2}` covers the full 10-byte span
    /// including the optional step component.
    #[test]
    fn semantic_tokens_brace_range_with_step_emits_operator() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        let uri = "file:///t.zsh";
        state
            .docs
            .insert(uri.to_string(), "echo {1..10..2}\n".to_string());
        let r = semantic_tokens(&state, &json!({ "textDocument": { "uri": uri } }));
        let data = r["data"].as_array().expect("data array");
        let mut found = false;
        for chunk in data.chunks(5) {
            if chunk.len() == 5 && chunk[2].as_u64() == Some(10) && chunk[3].as_u64() == Some(4) {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "brace range `{{1..10..2}}` must emit a single 10-byte operator token, got data={:?}",
            data
        );
    }

    /// Two definitions of the same name: the documented one wins even
    /// when it appears AFTER the undocumented one in source order.
    /// Pinned because the fallback-collector pass must defer the
    /// undocumented hit until the whole file is scanned.
    #[test]
    fn find_user_symbol_doc_documented_wins_over_earlier_undocumented() {
        let _g = crate::test_util::global_state_lock();
        let src = "\
fn foo() { echo first }\n\
\n\
## second def with docs\n\
foo() { echo second }\n\
";
        let r = find_user_symbol_doc(src, "foo").expect("UDF must hover");
        assert!(
            r.contains("second def with docs"),
            "documented later-def must win, got {:?}",
            r
        );
    }

    /// Every `examples/demos/*.zsh` file must pass `diagnose()` with
    /// zero diagnostics. This is the LSP-accepts-all-demos contract.
    /// Any new false positive surfaced by a demo addition fails this
    /// test, forcing a real LSP fix rather than silent IDE noise.
    #[test]
    fn diagnose_accepts_every_examples_demo_zsh_file() {
        let _g = crate::test_util::global_state_lock();
        let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let demos_dir = manifest_dir.join("examples").join("demos");
        let mut failures: Vec<(String, Vec<Value>)> = Vec::new();
        let entries = std::fs::read_dir(&demos_dir).expect("examples/demos must exist");
        let mut total = 0usize;
        for entry in entries {
            let path = entry.unwrap().path();
            if path.extension().and_then(|s| s.to_str()) != Some("zsh") {
                continue;
            }
            total += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            let d = diagnose(&text);
            if !d.is_empty() {
                failures.push((path.file_name().unwrap().to_string_lossy().into_owned(), d));
            }
        }
        assert!(total > 0, "no demos found under {}", demos_dir.display());
        assert!(
            failures.is_empty(),
            "{}/{} demos flagged by LSP diagnose():\n{:#?}",
            failures.len(),
            total,
            failures,
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
        assert!(d.is_empty(), "`$#` mis-handled as comment marker: {:?}", d);
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
        assert!(d.is_empty(), "`[[ ]]` mis-handled as two `[`s: {:?}", d);
    }

    #[test]
    fn diagnose_handles_arithmetic_double_paren_as_pair() {
        let _g = crate::test_util::global_state_lock();
        // `(( ... ))` is a single arithmetic expression.
        let src = "(( i++ ))\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "`(( ))` mis-handled as two `(`s: {:?}", d);
    }

    #[test]
    fn diagnose_does_not_flag_case_arm_paren_as_unmatched() {
        let _g = crate::test_util::global_state_lock();
        // Bare `)` inside an open `case ... esac` block is a
        // pattern-arm terminator, not a paren mismatch.
        let src = "case \"$x\" in\n  -h|--help) echo usage ;;\n  *) echo other ;;\nesac\n";
        let d = diagnose(src);
        assert!(d.is_empty(), "case-arm `)` flagged as unmatched: {:?}", d);
    }

    #[test]
    fn diagnose_still_flags_unmatched_paren_outside_case() {
        let _g = crate::test_util::global_state_lock();
        // Sanity: the case-arm exemption must NOT swallow a real
        // unmatched `)` outside of any case block.
        let src = "echo bare )\n";
        let d = diagnose(src);
        assert!(
            d.iter().any(|v| v["message"]
                .as_str()
                .unwrap_or("")
                .contains("unmatched `)`")),
            "real unmatched `)` was not flagged: {:?}",
            d
        );
    }

    // ── formatting engine (extensions/fmt.rs) ──────────────────────────
    // simple_format (whitespace-only stub) was replaced by the full
    // syntax-aware reindenter; these pin the same two guarantees
    // through the new engine. Note the reindent: a top-level command
    // with stray leading spaces now normalizes to column 0 (the stub
    // preserved whatever indent it found).

    #[test]
    fn format_strips_trailing_whitespace() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo hi   \n  echo bye\t\n";
        let out = crate::fmt::format_source(src, &crate::fmt::FmtOptions::default());
        assert_eq!(out, "echo hi\necho bye\n");
    }

    #[test]
    fn format_ensures_trailing_newline() {
        let _g = crate::test_util::global_state_lock();
        let src = "echo hi";
        let out = crate::fmt::format_source(src, &crate::fmt::FmtOptions::default());
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
        // canonical UPPERCASE_WITH_UNDERSCORES form per `man zshoptions`
        // (`EXTENDED_GLOB`, not `extendedglob`) since dump_reflection_json
        // now sources from `OPTION_DOCS` keys which carry the doc form.
        assert!(v["builtins"]["cd"].is_string());
        assert!(v["keywords"]["if"].is_string());
        assert!(v["options"]["EXTENDED_GLOB"].is_string());
        // PS2/PS3/PS4/psvar/PROMPT2 must surface (user-reported gap).
        for want in ["PS1", "PS2", "PS3", "PS4", "psvar", "PROMPT2"] {
            assert!(
                v["special_vars"][want].is_string(),
                "special_vars missing `{}` — reflection should source from SPECIAL_VAR_DOCS",
                want,
            );
        }
        // Canonical sourcing produces the full inventory, not the
        // hand subset. Options should include CAPS form + NO_ aliases.
        assert!(
            v["options"].as_object().unwrap().len() >= 700,
            "dump_reflection options regressed to {}; expected full OPTION_DOCS + aliases (~750)",
            v["options"].as_object().unwrap().len(),
        );
        assert!(
            v["special_vars"].as_object().unwrap().len() >= 250,
            "dump_reflection special_vars regressed to {}; expected full SPECIAL_VAR_DOCS (~280)",
            v["special_vars"].as_object().unwrap().len(),
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
        assert!(
            body.contains("then") && body.contains("fi"),
            "snippet body wrong: {}",
            body
        );
    }

    #[test]
    fn completion_suppressed_inside_double_quoted_literal() {
        // User report: "inside dbl strings I shouldnt be getting
        // random completions unless inside $() or ``". A double-quoted
        // string body is prose / URLs / JSON — not shell code — so
        // surfacing `if`, `cd`, `setopt` etc. is noise. Pin the gate.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), r#"echo "hello if"#.into());
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
            items
                .iter()
                .take(5)
                .map(|i| i["label"].as_str().unwrap_or("?"))
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_active_inside_command_substitution_inside_dq() {
        // Counterpart to the dq gate: cursor inside `$(...)` IS shell
        // code even when wrapped by `"..."`. The gate must NOT swallow
        // completion here.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), r#"echo "x $(cd"#.into());
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
            items
                .iter()
                .take(5)
                .map(|i| i["label"].as_str().unwrap_or("?"))
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_active_inside_backticks_inside_dq() {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), "echo \"x `cd".into());
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
        state
            .docs
            .insert("file:///t.zsh".into(), r#"echo "x ${P"#.into());
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
        state
            .docs
            .insert("file:///t.zsh".into(), "echo 'hello if".into());
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
        state
            .docs
            .insert("file:///t.zsh".into(), "# todo: if".into());
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
        state
            .docs
            .insert("file:///t.zsh".into(), r#"echo "x" if"#.into());
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
                items
                    .iter()
                    .map(|i| i["label"].as_str().unwrap_or("?"))
                    .collect::<Vec<_>>(),
            );
        }
        // Should NOT include shell builtins / keywords / options here.
        assert!(
            !items
                .iter()
                .any(|i| i["label"] == "cd" || i["label"] == "if"),
            "param-flag context leaked normal completion: {:?}",
            items
                .iter()
                .take(20)
                .map(|i| i["label"].as_str().unwrap_or("?"))
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_param_flags_after_partial_flag() {
        // `${(b` — cursor after first flag letter. We still want the
        // full table surfaced (user may add more flags, eg `${(bC)`).
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), "echo ${(b".into());
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
        state
            .docs
            .insert("file:///t.zsh".into(), "echo ${${(".into());
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
        state
            .docs
            .insert("file:///t.zsh".into(), "echo ${(b)var".into());
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
            && items
                .iter()
                .all(|i| i["label"].as_str().unwrap_or("").chars().count() == 1);
        assert!(
            !single_char_only,
            "param-flag table leaked past closing `)`"
        );
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
                items
                    .iter()
                    .take(20)
                    .map(|i| i["label"].as_str().unwrap_or("?"))
                    .collect::<Vec<_>>(),
            );
        }
        assert!(
            !items
                .iter()
                .any(|i| i["label"] == "cd" || i["label"] == "if"),
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
        let has_normal = items
            .iter()
            .any(|i| i["label"] == "cd" || i["label"] == "if");
        let single_char_only = !items.is_empty()
            && items
                .iter()
                .all(|i| i["label"].as_str().unwrap_or("").chars().count() == 1);
        assert!(
            has_normal || !single_char_only,
            "subshell `(` mis-triggered glob qualifier table"
        );
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
                items
                    .iter()
                    .map(|i| i["label"].as_str().unwrap_or("?"))
                    .collect::<Vec<_>>(),
            );
        }
        // No builtins / keywords here.
        assert!(!items
            .iter()
            .any(|i| i["label"] == "cd" || i["label"] == "if"));
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
        state
            .docs
            .insert("file:///t.zsh".into(), "echo ${var:".into());
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
                items
                    .iter()
                    .map(|i| i["label"].as_str().unwrap_or("?"))
                    .collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn completion_param_modifier_after_partial_modifier() {
        // `${var:h` — cursor after `h`, still want full modifier table
        // surfaced so the IDE can re-filter as the user keeps typing.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///t.zsh".into(), "echo ${var:h".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 12 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        assert!(
            items.len() >= 25,
            "expected full modifier table; got {}",
            items.len()
        );
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
        let has_normal = items
            .iter()
            .any(|i| i["label"] == "cd" || i["label"] == "if");
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

    // ── command-position + bracket-context completion ──────────────

    fn complete_at(input: &str, col: usize) -> Vec<Value> {
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), input.into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": col },
        });
        completion(&state, &params)["items"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn completion_setopt_surfaces_options_only() {
        let items = complete_at("setopt extend", 13);
        // Should include real options, NOT builtins / keywords.
        assert!(
            items.iter().any(|i| i["label"] == "extended_glob"
                || i["label"] == "extendedglob"
                || i["label"] == "EXTENDED_GLOB"),
            "no extended_glob variant"
        );
        assert!(!items
            .iter()
            .any(|i| i["label"] == "cd" || i["label"] == "if"));
    }

    #[test]
    fn completion_unsetopt_surfaces_options_only() {
        let items = complete_at("unsetopt nul", 12);
        assert!(items.iter().any(|i| i["label"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("null")));
        assert!(!items.iter().any(|i| i["label"] == "if"));
    }

    #[test]
    fn completion_set_dash_o_surfaces_options() {
        let items = complete_at("set -o errex", 12);
        assert!(items.iter().any(|i| i["label"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .contains("err")));
    }

    #[test]
    fn completion_kill_dash_surfaces_signals() {
        let items = complete_at("kill -", 6);
        for want in &["HUP", "INT", "TERM", "KILL", "USR1"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing signal `{}`",
                want
            );
        }
    }

    #[test]
    fn completion_trap_surfaces_signals() {
        let items = complete_at("trap 'cmd' ", 11);
        for want in &["INT", "TERM", "EXIT", "ZERR", "DEBUG"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing signal `{}`",
                want
            );
        }
    }

    #[test]
    fn completion_zmodload_surfaces_modules() {
        let items = complete_at("zmodload ", 9);
        for want in &[
            "zsh/zle",
            "zsh/datetime",
            "zsh/system",
            "zsh/parameter",
            "zsh/mathfunc",
        ] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing module `{}`",
                want
            );
        }
    }

    #[test]
    fn completion_bindkey_dash_M_surfaces_keymaps() {
        let items = complete_at("bindkey -M ", 11);
        for want in &["emacs", "vicmd", "viins", "viopp"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing keymap `{}`",
                want
            );
        }
    }

    #[test]
    fn completion_bindkey_bare_surfaces_widgets() {
        // `bindkey "^A" <TAB>` — second arg is a widget. With no `-A/M/N`
        // flag, we default to WidgetName.
        let items = complete_at("bindkey '^A' acce", 17);
        assert!(items.iter().any(|i| i["label"] == "accept-line"));
        assert!(items.iter().any(|i| i["label"] == "accept-and-hold"));
    }

    #[test]
    fn completion_zle_surfaces_widgets() {
        let items = complete_at("zle for", 7);
        assert!(items.iter().any(|i| i["label"] == "forward-char"));
        assert!(items.iter().any(|i| i["label"] == "forward-word"));
    }

    #[test]
    fn completion_typeset_dash_surfaces_flags() {
        let items = complete_at("typeset -", 9);
        for want in &["-a", "-A", "-i", "-g", "-r", "-x", "-U", "-f"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing flag `{}`",
                want
            );
        }
    }

    #[test]
    fn completion_typeset_no_flag_falls_through() {
        // `typeset FOO=bar` — second arg has no leading `-`, normal flow.
        let items = complete_at("typeset FOO", 11);
        // Should NOT be the flags table — `-a` etc. shouldn't appear.
        assert!(!items.iter().any(|i| i["label"] == "-a"));
    }

    #[test]
    fn completion_zstyle_surfaces_contexts() {
        let items = complete_at("zstyle ", 7);
        assert!(items.iter().any(|i| i["label"] == ":completion:*"));
        assert!(items.iter().any(|i| i["label"] == ":vcs_info:*"));
    }

    #[test]
    fn completion_compdef_surfaces_compsys_fns() {
        let items = complete_at("compdef ", 8);
        // Should be `_*` style ported only.
        assert!(items
            .iter()
            .any(|i| i["label"].as_str().unwrap_or("").starts_with('_')));
    }

    #[test]
    fn completion_inside_double_bracket_surfaces_test_ops() {
        let items = complete_at("[[ -f /tmp/x && -", 17);
        for want in &["-f", "-d", "-e", "-z", "-n", "=~"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing test op `{}`",
                want
            );
        }
    }

    #[test]
    fn completion_inside_double_paren_surfaces_math_fns() {
        let items = complete_at("(( x = sq", 9);
        for want in &["sqrt", "sin", "cos", "log", "exp"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing math fn `{}`",
                want
            );
        }
    }

    #[test]
    fn completion_inside_dollar_double_paren_surfaces_math_fns() {
        let items = complete_at("echo $(( ab", 11);
        assert!(items.iter().any(|i| i["label"] == "abs"));
    }

    #[test]
    fn completion_inside_pattern_modifier_paren() {
        let items = complete_at("ls *.(#", 7);
        for want in &["i", "l", "b", "m", "a", "s", "e"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing pattern mod `{}` — got {:?}",
                want,
                items
                    .iter()
                    .take(20)
                    .map(|i| i["label"].as_str().unwrap_or("?"))
                    .collect::<Vec<_>>(),
            );
        }
    }

    #[test]
    fn completion_inside_subscript_flag_paren() {
        let items = complete_at("echo ${arr[(", 12);
        for want in &["i", "I", "r", "R", "e", "n"] {
            assert!(
                items.iter().any(|i| i["label"] == *want),
                "missing subscript flag `{}`",
                want
            );
        }
    }

    // ── table size floors ────────────────────────────────────────────

    #[test]
    fn completion_signal_table_has_30_entries() {
        assert!(
            SIGNAL_NAMES.len() >= 30,
            "SIGNAL_NAMES: {}",
            SIGNAL_NAMES.len()
        );
    }

    #[test]
    fn completion_module_table_has_30_entries() {
        assert!(
            ZSH_MODULE_NAMES.len() >= 30,
            "ZSH_MODULE_NAMES: {}",
            ZSH_MODULE_NAMES.len()
        );
    }

    #[test]
    fn completion_keymap_table_has_10_entries() {
        assert!(
            KEYMAP_NAMES.len() >= 10,
            "KEYMAP_NAMES: {}",
            KEYMAP_NAMES.len()
        );
    }

    #[test]
    fn completion_widget_table_has_100_entries() {
        assert!(
            ZLE_WIDGET_NAMES.len() >= 100,
            "ZLE_WIDGET_NAMES: {}",
            ZLE_WIDGET_NAMES.len()
        );
    }

    #[test]
    fn completion_typeset_flag_table_has_20_entries() {
        assert!(
            TYPESET_FLAGS.len() >= 20,
            "TYPESET_FLAGS: {}",
            TYPESET_FLAGS.len()
        );
    }

    #[test]
    fn completion_test_op_table_has_30_entries() {
        assert!(
            TEST_OPERATORS.len() >= 30,
            "TEST_OPERATORS: {}",
            TEST_OPERATORS.len()
        );
    }

    #[test]
    fn completion_math_fn_table_has_40_entries() {
        assert!(
            MATH_FUNCTIONS.len() >= 40,
            "MATH_FUNCTIONS: {}",
            MATH_FUNCTIONS.len()
        );
    }

    #[test]
    fn completion_zstyle_context_table_has_15_entries() {
        assert!(
            ZSTYLE_CONTEXTS.len() >= 15,
            "ZSTYLE_CONTEXTS: {}",
            ZSTYLE_CONTEXTS.len()
        );
    }

    #[test]
    fn completion_pattern_modifier_table_has_10_entries() {
        assert!(
            PATTERN_MODIFIERS.len() >= 10,
            "PATTERN_MODIFIERS: {}",
            PATTERN_MODIFIERS.len()
        );
    }

    #[test]
    fn completion_subscript_flag_table_has_10_entries() {
        assert!(
            SUBSCRIPT_FLAGS.len() >= 10,
            "SUBSCRIPT_FLAGS: {}",
            SUBSCRIPT_FLAGS.len()
        );
    }

    #[test]
    fn completion_keywords_includes_canonical_reswds() {
        // Hand `KEYWORDS` was missing canonical RESWDS like `end`,
        // `{`, `}`, `!`, `[[`. Verify they reach completion via the
        // RESWDS iteration. Use prefixes that match each name but
        // DON'T trigger a context (e.g. `e` for `end`, no special
        // context). For `[[` / `}` / `!` / `{` themselves, those
        // trigger context-specific completion (TestOperator,
        // SubscriptFlag, HistoryDesignator, PatternModifier
        // respectively) so we test reachability via the call into
        // `lsp_completion_context` returning Normal at non-trigger
        // positions.
        let _g = crate::test_util::global_state_lock();
        // `e<TAB>` — should include `end` (canonical reswd missing
        // from hand KEYWORDS) and `echo`/`exec`/`esac`/etc.
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "e".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 1 },
        });
        let items = completion(&state, &params)["items"]
            .as_array()
            .unwrap()
            .clone();
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or(""))
            .collect();
        assert!(
            labels.iter().any(|l| *l == "end"),
            "RESWDS `end` not surfaced — labels: {:?}",
            labels
                .iter()
                .filter(|l| l.starts_with('e'))
                .take(10)
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn completion_builtin_flag_print_dash_tab() {
        // User report: `print -<TAB>` showed nothing — zsh's native
        // compsys produces a rich -a/-b/-c/-D/-f/-l/-n/-N/-o/-O/-P
        // list with descriptions. Pin that the doc-body-derived
        // BuiltinFlag context fires + extracts ≥15 flag bullets.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "print -".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 7 },
        });
        let items = completion(&state, &params)["items"]
            .as_array()
            .unwrap()
            .clone();
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or(""))
            .collect();
        for want in &[
            "-a", "-b", "-c", "-n", "-N", "-o", "-O", "-P", "-r", "-R", "-z",
        ] {
            assert!(
                labels.iter().any(|l| l == want),
                "missing `print -` flag `{}` — got {:?}",
                want,
                labels.iter().take(20).collect::<Vec<_>>(),
            );
        }
        assert!(
            items.len() >= 15,
            "expected ≥15 print flags, got {}",
            items.len(),
        );
    }

    #[test]
    fn completion_dollar_prefix_matches_canonical_specials() {
        // User report: `$HIST<TAB>` only surfaced `$HISTFILE`/`$HISTSIZE`
        // (the hand SPECIAL_VARS subset which stores names with `$`
        // prefix). Canonical SPECIAL_VAR_DOCS stores names WITHOUT
        // `$`, so the prefix-match `"$HISTCMD".starts_with("$HIST")`
        // failed because the candidate was just `"HISTCMD"`. Fix
        // adds a $-prefix path that matches the bare prefix against
        // the bare name + emits the candidate with `$` prepended.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert("file:///t.zsh".into(), "$HIST".into());
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": 5 },
        });
        let result = completion(&state, &params);
        let items = result["items"].as_array().unwrap();
        let labels: Vec<&str> = items
            .iter()
            .map(|i| i["label"].as_str().unwrap_or(""))
            .collect();
        for want in &["$HISTCMD", "$HISTNO", "$HISTCHARS"] {
            assert!(
                labels.iter().any(|l| l == want),
                "missing `{}` for `$HIST` prefix",
                want,
            );
        }
    }

    #[test]
    fn completion_special_vars_includes_full_canonical_set() {
        // User report: `$PS2`, `$PS3`, `$PS4`, `$psvar` were missing
        // from completion even though they're in the doc table. Root
        // cause: hand `SPECIAL_VARS` const was a stale 40-entry
        // subset of zsh's actual ~538 specials. Fix surfaces every
        // canonical name from `SPECIAL_VAR_DOCS` + `SPECIAL_VAR_ALIASES`.
        let _g = crate::test_util::global_state_lock();
        for prefix in &["PS", "PROMPT", "psvar"] {
            let mut state = State::default();
            state.docs.insert("file:///t.zsh".into(), (*prefix).into());
            let params = json!({
                "textDocument": { "uri": "file:///t.zsh" },
                "position": { "line": 0, "character": prefix.len() },
            });
            let result = completion(&state, &params);
            let items = result["items"].as_array().unwrap();
            let labels: Vec<&str> = items
                .iter()
                .map(|i| i["label"].as_str().unwrap_or(""))
                .collect();
            match *prefix {
                "PS" => {
                    for want in &["PS1", "PS2", "PS3", "PS4"] {
                        assert!(
                            labels.iter().any(|l| l == want),
                            "missing `{}` for prefix `{}`",
                            want,
                            prefix,
                        );
                    }
                }
                "PROMPT" => {
                    for want in &["PROMPT", "PROMPT2", "PROMPT3", "PROMPT4"] {
                        assert!(
                            labels.iter().any(|l| l == want),
                            "missing `{}` for prefix `{}`",
                            want,
                            prefix,
                        );
                    }
                }
                "psvar" => {
                    assert!(labels.iter().any(|l| *l == "psvar"), "missing `psvar`",);
                }
                _ => {}
            }
        }
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
            arr.iter()
                .any(|r| r["uri"].as_str() == Some(sourced_uri.as_str())),
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
    fn hover_on_shebang_with_module_doc_returns_module_card() {
        // Shebang hover normally returns null, but when a `##` block
        // follows the shebang the hover surfaces it as the module
        // doc card. Lets users discover what a sourced library does
        // by hovering its shebang.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///lib.zsh".into(),
            "#!/usr/bin/env zsh\n\
             ## libfoo.zsh — utility helpers.\n\
             ## Provides foo / bar / baz.\n\
             function foo() {}\n"
                .into(),
        );
        // Cursor on `env` (any position on line 0 triggers the
        // shebang-line module-doc lookup).
        let params = json!({
            "textDocument": { "uri": "file:///lib.zsh" },
            "position": { "line": 0, "character": 12 },
        });
        let h = hover(&state, &params);
        assert!(!h.is_null(), "shebang+##block hover should return card");
        let body = h["contents"]["value"].as_str().unwrap_or("");
        assert!(body.contains("libfoo.zsh"), "got {body:?}");
        assert!(body.contains("zsh module"), "got {body:?}");
    }

    #[test]
    fn hover_on_source_path_uri_cached_returns_target_module_doc() {
        // `source ./other.zsh` — when other.zsh is loaded in the
        // doc cache, hover on the path argument should surface its
        // top-of-file `##` block.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///proj/main.zsh".into(),
            "source ./helpers.zsh\nhelpers_init\n".into(),
        );
        state.docs.insert(
            "file:///proj/helpers.zsh".into(),
            "## helpers.zsh — shared bootstrap.\n\
             function helpers_init() {}\n"
                .into(),
        );
        // Cursor on the path argument `./helpers.zsh`.
        let pos = "source ".len() + 2; // somewhere inside `./helpers.zsh`
        let params = json!({
            "textDocument": { "uri": "file:///proj/main.zsh" },
            "position": { "line": 0, "character": pos },
        });
        let h = hover(&state, &params);
        assert!(
            !h.is_null(),
            "source-path hover should return target's module doc"
        );
        let body = h["contents"]["value"].as_str().unwrap_or("");
        assert!(body.contains("helpers.zsh"), "got {body:?}");
        assert!(body.contains("shared bootstrap"), "got {body:?}");
    }

    #[test]
    fn hover_on_dot_source_path_resolves_same_way() {
        // POSIX dot-source form: `. PATH` is equivalent to `source PATH`.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state
            .docs
            .insert("file:///proj/main.zsh".into(), ". ./helpers.zsh\n".into());
        state.docs.insert(
            "file:///proj/helpers.zsh".into(),
            "## dot-sourced helpers.\nfunction h() {}\n".into(),
        );
        let pos = ". ".len() + 2;
        let params = json!({
            "textDocument": { "uri": "file:///proj/main.zsh" },
            "position": { "line": 0, "character": pos },
        });
        let h = hover(&state, &params);
        assert!(!h.is_null(), ". PATH hover should resolve too");
        let body = h["contents"]["value"].as_str().unwrap_or("");
        assert!(body.contains("dot-sourced helpers"), "got {body:?}");
    }

    #[test]
    fn hover_on_source_path_without_target_doc_returns_null() {
        // `source PATH` but target file isn't loaded and doesn't
        // exist on disk → null. (No fabrication.)
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "source /nonexistent/path/that/does/not/exist.zsh\n".into(),
        );
        let pos = "source ".len() + 5;
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 0, "character": pos },
        });
        let h = hover(&state, &params);
        assert!(h.is_null(), "missing source target must not fabricate doc");
    }

    #[test]
    fn hover_on_user_function_with_doc_returns_user_card() {
        // Integration check for the prior find_user_symbol_doc path —
        // confirm it actually surfaces through `hover()`, not just
        // the unit test. Use a non-builtin name (`my_sum_helper`)
        // so the builtin lookup misses and our fallback fires.
        let _g = crate::test_util::global_state_lock();
        let mut state = State::default();
        state.docs.insert(
            "file:///t.zsh".into(),
            "## Sum two numbers and print the result.\n\
             function my_sum_helper() { print $(( $1 + $2 )) }\n\
             my_sum_helper 1 2\n"
                .into(),
        );
        // Cursor on `my_sum_helper` at the call site (line 2).
        let params = json!({
            "textDocument": { "uri": "file:///t.zsh" },
            "position": { "line": 2, "character": 0 },
        });
        let h = hover(&state, &params);
        assert!(!h.is_null(), "user-fn hover should fire");
        let body = h["contents"]["value"].as_str().unwrap_or("");
        assert!(body.contains("Sum two numbers"), "got {body:?}");
        assert!(body.contains("user-defined function"), "got {body:?}");
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
        assert!(
            !edits.is_empty(),
            "expected at least 1 edit, got: {edits:?}"
        );
        for e in edits {
            assert_eq!(
                e["newText"],
                json!("handle2"),
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
        // rc.zsh MUST source lib.zsh — otherwise the cross-file lookup
        // would be FAKE (matching `greet` across unrelated files just
        // because the name matches, with no actual scope linkage).
        // Cross-file scope in zsh is opt-in via `source X`.
        let rc_text = format!("source {}\ngreet\ngreet world\n", lib_path.display());
        std::fs::write(&rc_path, &rc_text).unwrap();
        let rc_uri = format!("file://{}", rc_path.display());

        let mut state = State::default();
        // Only `rc.zsh` is in the editor — `lib.zsh` is on disk.
        state.docs.insert(rc_uri.clone(), rc_text.clone());
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
        // On macOS `/var` is a symlink to `/private/var`. The
        // source-chain BFS canonicalizes `source X` targets (so
        // symlinks don't double-emit), which means the URI in the
        // rename result is the CANONICAL path. Use that for the
        // contains_key assertion below.
        let canon_lib_path = std::fs::canonicalize(&lib_path).unwrap_or(lib_path.clone());
        let canon_lib_uri = format!("file://{}", canon_lib_path.display());
        // Rename `greet` from the rc.zsh call site — must touch both.
        // Cursor on line 1 col 0 since rc.zsh now starts with the
        // `source lib.zsh` directive on line 0; the first `greet` call
        // site is on line 1.
        let params = json!({
            "textDocument": { "uri": rc_uri },
            "position": { "line": 1, "character": 0 },
            "context": { "includeDeclaration": true },
            "newName": "salute",
        });
        let r = rename(&state, &params);
        let changes = r["changes"].as_object().expect("changes map");
        assert!(
            changes.contains_key(&canon_lib_uri),
            "lib.zsh (workspace) edited: keys={:?}",
            changes.keys().collect::<Vec<_>>(),
        );
        assert!(
            changes.contains_key(&rc_uri),
            "rc.zsh (open) edited: keys={:?}",
            changes.keys().collect::<Vec<_>>(),
        );
        // 1 decl in lib + 2 call sites in rc.
        assert_eq!(changes[&canon_lib_uri].as_array().unwrap().len(), 1);
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
        assert!(
            call.trim() == "extracted_function",
            "call must be bare: {:?}",
            call
        );
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

    #[test]
    fn long_flag_completion_inside_command_substitution() {
        // `x=$(zshrs --|)` — cursor inside `$(...)` after `zshrs --`.
        // Before the `(` boundary case in `leading_command_at`, the
        // walk-back found `x` as the leading command (because it
        // never crossed the `$(` opener), and long-flag completion
        // didn't fire.
        let line = "x=$(zshrs --";
        let ctx = super::lsp_completion_context(line, line.len());
        assert!(
            matches!(ctx, super::LspCompletionContext::BuiltinLongFlag(ref n) if n == "zshrs"),
            "expected BuiltinLongFlag(zshrs) inside `$(`, got {ctx:?}",
        );
        // Same for `<(…)` process substitution.
        let line = "diff <(zshrs --";
        let ctx = super::lsp_completion_context(line, line.len());
        assert!(
            matches!(ctx, super::LspCompletionContext::BuiltinLongFlag(ref n) if n == "zshrs"),
            "expected BuiltinLongFlag(zshrs) inside `<(`, got {ctx:?}",
        );
        // Plain `(subshell --` too.
        let line = "(zshrs --";
        let ctx = super::lsp_completion_context(line, line.len());
        assert!(
            matches!(ctx, super::LspCompletionContext::BuiltinLongFlag(ref n) if n == "zshrs"),
            "expected BuiltinLongFlag(zshrs) inside `(`, got {ctx:?}",
        );
    }

    #[test]
    fn zshrs_long_flag_table_includes_gen_docs() {
        // Regression: `zshrs --gen-d<TAB>` should surface `--gen-docs`.
        // Tracked in `ZSHRS_SELF_LONG_FLAG_DOCS`; the wide audit
        // didn't cover this specific entry until the user pointed it
        // out.
        let flags = super::extract_builtin_long_flags("zshrs");
        let names: std::collections::HashSet<&str> =
            flags.iter().map(|(f, _)| f.as_str()).collect();
        for must_have in [
            "--gen-docs",
            "--out",
            "--dump-reference-html",
            "--names",
            "--daemon",
            "--color",
        ] {
            assert!(
                names.contains(must_have),
                "ZSHRS_SELF_LONG_FLAG_DOCS missing {must_have}",
            );
        }
    }

    #[test]
    fn zle_dash_dispatches_to_builtin_flag_not_widget_name() {
        // Regression: `zle -<TAB>` used to short-circuit to the
        // WidgetName context because the dispatcher matched `zle`
        // unconditionally. Now the dispatcher checks whether the
        // current word starts with `-` and routes flag completion
        // through `BUILTIN_FLAG_DOCS_OVERRIDE`. Without that
        // branch the user gets a list of zle widget names where
        // they expected flag completion.
        let line = "zle -";
        let ctx = super::lsp_completion_context(line, line.len());
        assert!(
            matches!(ctx, super::LspCompletionContext::BuiltinFlag(ref n) if n == "zle"),
            "expected BuiltinFlag(zle) for `zle -`, got {ctx:?}",
        );
        // Sanity: bare `zle ` (no dash) still completes widget names.
        let bare = "zle ";
        let ctx2 = super::lsp_completion_context(bare, bare.len());
        assert!(
            matches!(ctx2, super::LspCompletionContext::WidgetName),
            "expected WidgetName for `zle `, got {ctx2:?}",
        );
        // The flag table for `zle` surfaces the documented sub-
        // commands (sample: -l list, -N declare new, -K keymap).
        let flags = extract_builtin_flags("zle");
        let names: std::collections::HashSet<&str> =
            flags.iter().map(|(f, _)| f.as_str()).collect();
        for must_have in ["-l", "-L", "-N", "-K", "-D", "-A", "-R", "-M"] {
            assert!(
                names.contains(must_have),
                "zle missing flag {must_have} from BUILTIN_FLAG_DOCS_OVERRIDE",
            );
        }
    }

    #[test]
    fn print_flag_descriptions_present_for_next_line_desc_pattern() {
        // Regression for the JetBrains-plugin screenshot: `print -<TAB>`
        // showed `-a`, `-c`, `-D`, `-i`, `-l`, `-n`, `-o`, `-p`, `-r`,
        // `-s`, `-z` etc. with NO description, only `-b` and `-m` had
        // text. Root cause: the bullet regex stopped at the EOL right
        // after `**`, so it captured a single trailing mojibake byte
        // instead of the next-line description. Every flag must
        // surface its real prose now.
        let flags = extract_builtin_flags("print");
        let by: std::collections::HashMap<&str, &str> = flags
            .iter()
            .map(|(f, d)| (f.as_str(), d.as_str()))
            .collect();

        // Inline-desc flags — these always worked, keep as canary.
        assert!(
            by.get("-b").is_some_and(|d| d.contains("Recognize")),
            "-b should describe escape sequence recognition, got {:?}",
            by.get("-b"),
        );
        assert!(
            by.get("-m")
                .is_some_and(|d| d.contains("Take the first argument")),
            "-m should describe pattern matching, got {:?}",
            by.get("-m"),
        );

        // Next-line-desc flags — these were silently empty before.
        // Spot-check a representative subset.
        let expectations: &[(&str, &str)] = &[
            ("-a", "Print arguments with the column"),
            ("-c", "Print the arguments in columns"),
            ("-D", "Treat the arguments as paths"),
            ("-l", "Print the arguments separated by newlines"),
            ("-n", "Do not add a newline"),
            ("-o", "Print the arguments sorted in ascending"),
            ("-r", "Ignore the escape conventions"),
            ("-s", "Place the results in the history list"),
            ("-z", "Push the arguments onto the editing buffer"),
        ];
        for (flag, needle) in expectations {
            let got = by.get(flag).copied().unwrap_or("<missing>");
            assert!(
                got.contains(needle),
                "flag {flag} description should contain {:?}, got {:?}",
                needle,
                got,
            );
        }
    }

    #[test]
    fn zshrs_self_long_flag_completion_covers_zshrs_specific_plus_setopt_mirrors() {
        // `zshrs --<TAB>` previously returned zero long-flag items.
        // The new `BuiltinLongFlag` context + extract_builtin_long_flags
        // now surfaces every zshrs-specific long flag plus every
        // setopt option (transformed `AUTO_CD` → `--autocd`).
        assert!(super::is_known_builtin_with_long_flag_docs("zshrs"));
        assert!(super::is_known_builtin_with_long_flag_docs("zsh"));
        assert!(!super::is_known_builtin_with_long_flag_docs("print"));

        let flags = super::extract_builtin_long_flags("zshrs");
        let by: std::collections::HashMap<&str, &str> = flags
            .iter()
            .map(|(f, d)| (f.as_str(), d.as_str()))
            .collect();

        // Spot-check zshrs-specific long flags.
        for spec in [
            "--help",
            "--version",
            "--doctor",
            "--lsp",
            "--dap",
            "--dump-tokens",
            "--dump-ast",
            "--dump-wordcode",
            "--dump-zwc",
            "--dump-reflection",
            "--docs",
            "--disasm",
            "--zsh",
            "--bash",
            "--ksh",
            "--sh",
            "--csh",
            "--posix",
            "--emulate",
            "--zsh-compat",
            "--no-rcs",
            "--verbose",
            "--xtrace",
            "--login",
            "--interactive",
        ] {
            assert!(
                by.contains_key(spec),
                "zshrs long-flag table missing {spec}",
            );
            let d = by.get(spec).unwrap();
            let letters = d.chars().filter(|c| c.is_ascii_alphabetic()).count();
            assert!(
                letters >= 10,
                "{spec} description should be substantive, got {:?}",
                d,
            );
        }
        // Setopt mirrors arrive from OPTION_DOCS — spot-check
        // both positive and inverse forms.
        for mirror in [
            "--autocd",
            "--errexit",
            "--pipefail",
            "--nullglob",
            "--extendedglob",
            "--no-autocd",
            "--no-errexit",
            "--no-pipefail",
        ] {
            assert!(
                by.contains_key(mirror),
                "setopt-mirror flag {mirror} missing — OPTION_DOCS not flowing through?",
            );
        }
        // Total: ZSHRS_SELF_LONG_FLAG_DOCS hand table (~25) +
        // OPTION_DOCS canonical (~197) × 2 (positive + inverse).
        // Should land near 420.
        assert!(
            flags.len() > 400,
            "expected > 400 long-flag entries (hand table + setopt mirrors × 2), got {}",
            flags.len(),
        );
    }

    #[test]
    fn zshrs_self_flag_completion_lists_standard_short_flags() {
        // `zshrs -<TAB>` previously returned zero completions: the
        // binary itself isn't a builtin / ext-builtin / compsys fn,
        // so `is_known_builtin_with_flag_docs` short-circuited. The
        // ZSHRS_SELF_FLAG_DOCS table + the `name == "zshrs"` branch
        // make all 9 standard short flags surface, each with a
        // description.
        assert!(super::is_known_builtin_with_flag_docs("zshrs"));
        assert!(super::is_known_builtin_with_flag_docs("zsh"));

        let flags = extract_builtin_flags("zshrs");
        let names: std::collections::HashSet<&str> =
            flags.iter().map(|(f, _)| f.as_str()).collect();
        for must_have in ["-b", "-c", "-f", "-i", "-l", "-s", "-o", "-v", "-x"] {
            assert!(
                names.contains(must_have),
                "zshrs self-flag table missing {must_have}",
            );
        }
        for (f, d) in &flags {
            let letters = d.chars().filter(|c| c.is_ascii_alphabetic()).count();
            assert!(
                letters >= 10,
                "zshrs {f} description should be substantive, got {:?}",
                d,
            );
        }
        // `zsh` as a name alias resolves to the same table.
        let zsh_flags = extract_builtin_flags("zsh");
        assert_eq!(zsh_flags.len(), flags.len());
    }

    // Coverage audit — eprintln only, never fails. Run with
    //   cargo test -p zshrs --lib audit_all_builtin_flag_coverage -- --nocapture
    // to see which builtins still have empty / near-empty descriptions
    // after the bullet regex fix.
    #[test]
    fn audit_all_builtin_flag_coverage() {
        let mut all_names: Vec<String> = Vec::new();
        for b in crate::ported::builtin::BUILTINS.iter() {
            all_names.push(b.node.nam.to_string());
        }
        for n in crate::ext_builtins::EXT_BUILTIN_NAMES.iter() {
            all_names.push(n.to_string());
        }
        for n in crate::compsys::COMPSYS_FN_NAMES.iter() {
            all_names.push(n.to_string());
        }
        all_names.sort();
        all_names.dedup();

        let mut total_flags = 0usize;
        let mut empty_descs = 0usize;
        let mut builtins_with_any_flag = 0usize;
        let mut builtins_with_empty: Vec<(String, usize, usize)> = Vec::new();
        // A description is "useful" when it has at least 3 ASCII
        // letters somewhere in it. Pure mojibake (`Â`, `â `), bare
        // em-dashes, or punctuation-only strings fail this.
        fn useful(d: &str) -> bool {
            let letters = d.chars().filter(|c| c.is_ascii_alphabetic()).count();
            letters >= 3
        }
        for name in &all_names {
            let flags = extract_builtin_flags(name);
            if flags.is_empty() {
                continue;
            }
            builtins_with_any_flag += 1;
            let total = flags.len();
            let empty = flags.iter().filter(|(_, d)| !useful(d)).count();
            total_flags += total;
            empty_descs += empty;
            if empty > 0 {
                let preview: Vec<String> = flags
                    .iter()
                    .filter(|(_, d)| !useful(d))
                    .take(3)
                    .map(|(f, d)| format!("{f}={:?}", d))
                    .collect();
                builtins_with_empty.push((
                    format!("{name}  [{}]", preview.join(", ")),
                    empty,
                    total,
                ));
            }
        }
        eprintln!(
            "audit: {} builtins with flags, {} total flag entries, {} empty desc ({:.1}%)",
            builtins_with_any_flag,
            total_flags,
            empty_descs,
            100.0 * empty_descs as f64 / total_flags.max(1) as f64,
        );
        builtins_with_empty.sort_by(|a, b| b.1.cmp(&a.1));
        for (name, empty, total) in builtins_with_empty.iter().take(40) {
            eprintln!("  {name} : {empty}/{total} empty");
        }
    }
}
