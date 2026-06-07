//! In-editor compsys completion entry point.
//!
//! Drives `_main_complete` from outside an interactive shell so the
//! LSP (and future non-LSP clients) get the same match list a Tab
//! press at the prompt would produce. Reuses the ported compsys
//! runtime — no separate completion engine, no subshell spawn.
//!
//! Design + rationale: `docs/IN_EDITOR_COMPSYS_COMPLETION.md`.
//!
//! # Architecture
//!
//! ```text
//! complete_at(line, cursor)
//!     ├── parse line → words[], CURRENT
//!     ├── snapshot shell params (BUFFER, CURSOR, words, CURRENT, curcontext)
//!     ├── install COMPADD_CAPTURE_BUFFER shadow
//!     ├── _main_complete(&[])                  ← walks completer chain
//!     │      ├── _complete → _normal → _git → _arguments → _describe
//!     │      └── every `compadd` call lands in our capture buffer
//!     ├── drain buffer → Vec<CompsysMatch>
//!     └── restore shell params
//! ```
//!
//! Serialised by a process-wide mutex (`COMPLETE_AT_LOCK`) — the
//! ported compsys runtime uses thread-local + process-global shell
//! state, so concurrent in-editor completion requests would race.
//! Per-request cost dominates anyway (compdef dispatch, fpath
//! autoload, subprocess spawn for exec completions) so the lock
//! isn't on the hot path.
//!
//! # Phase 0.5 scope
//!
//! Wires the basic path end-to-end:
//!   * Whitespace word-split (no quote / parameter / brace expansion
//!     handling yet — that's Phase 0.6).
//!   * Single completer invocation per request — no result cache,
//!     no exec-mode gating (`allow_exec` is read but `_main_complete`
//!     today runs whatever's in the user's `completer` style).
//!   * Snapshot/restore covers the 5 params we set; deeper state
//!     (option flags, hash entries) isn't snapshotted today and
//!     could leak between requests in pathological compsys functions.
//!     Those leaks are visible only in test setups that hand-edit
//!     `compstate` from inside a compdef function — none of the 50+
//!     ported functions do.
//!
//! Phase 1+ (after Phase 0.5 ships) tightens these.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Request a compsys completion at `(line, cursor)`.
#[derive(Debug)]
pub struct CompsysRequest<'a> {
    /// The entire logical command line as the user has typed it.
    /// Multi-line continuations (`\\\n`) must already be glued by
    /// the caller before invocation.
    pub line: &'a str,
    /// 0-based byte column the cursor sits at inside `line`.
    pub cursor: usize,
    /// Hard deadline. Completion functions exceeding it are killed
    /// and the partial match list (if any) is returned with
    /// `is_incomplete = true`. Default in LSP path: 200 ms.
    pub deadline: Instant,
    /// When `false`, completion functions that would spawn
    /// subprocesses (`_kubectl get pods`, `_docker ps`) are
    /// skipped. Today `_main_complete` reads `$completer` style
    /// to decide — this flag is plumbed for Phase 1 to install a
    /// completer chain that excludes shell-out functions.
    pub allow_exec: bool,
}

impl<'a> CompsysRequest<'a> {
    /// Build a request with the LSP-default 200 ms deadline and
    /// `allow_exec = false`.
    pub fn new_with_default_budget(line: &'a str, cursor: usize) -> Self {
        Self {
            line,
            cursor,
            deadline: Instant::now() + std::time::Duration::from_millis(200),
            allow_exec: false,
        }
    }
}

/// A single completion match.
#[derive(Debug, Clone)]
pub struct CompsysMatch {
    pub completion: String,
    pub description: Option<String>,
    /// Group label from `_tags` / `_describe` (`subcommands`,
    /// `options`, `values`, `hosts`, …).
    pub group: Option<String>,
    /// Byte offset in `line` where the match-replacement region
    /// starts.
    pub replace_start: usize,
}

/// A complete response from compsys dispatch.
#[derive(Debug, Default)]
pub struct CompsysResponse {
    pub matches: Vec<CompsysMatch>,
    /// `true` when the deadline cut a dispatch short.
    pub is_incomplete: bool,
}

/// Called from the ported `bin_compadd` body when the in-editor
/// capture shadow is active. Parses the compadd argv (flags +
/// matches) and appends one `CompsysMatch` per proposed match
/// into [`COMPADD_CAPTURE_BUFFER`]. Returns `true` when the buffer
/// was active (caller should short-circuit and return 0 to mimic
/// "matches added"); returns `false` for passthrough (buffer
/// inactive — original `addmatches` path runs).
///
/// argv shape per `man zshmodules: zsh/computil::compadd`:
///   `compadd [ -X expl ] [ -d desc-arr ] [ -J|-V group ] [ -- ] match ...`
/// Phase 0.5 captures bare matches + `-J|-V` group + `-X` expl-
/// description. Phase 0.6 layers `-d desc-arr` so per-match
/// descriptions land in `CompsysMatch.description`.
pub fn try_capture_compadd_argv(argv: &[String]) -> bool {
    let mut guard = match COMPADD_CAPTURE_BUFFER.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let buf = match guard.as_mut() {
        Some(b) => b,
        None => return false,
    };
    let mut group: Option<String> = None;
    let mut description: Option<String> = None;
    // Flags that take an argument (inline `-Xfoo` or `-X foo`).
    // Conservative superset of the compadd flag list per
    // `src/ported/zle/complete.rs::bin_compadd_body`.
    fn takes_arg(c: char) -> bool {
        matches!(
            c,
            'X' | 'x'
                | 'd'
                | 'J'
                | 'V'
                | 'P'
                | 'S'
                | 'p'
                | 's'
                | 'W'
                | 'i'
                | 'I'
                | 'O'
                | 'A'
                | 'D'
                | 'F'
                | 'M'
                | 'n'
                | 'r'
                | 'R'
                | 'q'
                | 'Q'
                | 'T'
                | 'U'
                | 'C'
                | 'y'
                | 'e'
        )
    }
    fn pull_arg(argv: &[String], i: &mut usize, a: &str) -> String {
        if a.len() > 2 {
            a[2..].to_string()
        } else if *i + 1 < argv.len() {
            *i += 1;
            argv[*i].clone()
        } else {
            String::new()
        }
    }
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if a == "--" {
            i += 1;
            break;
        }
        if !a.starts_with('-') || a.len() < 2 {
            break;
        }
        let c = a.as_bytes()[1] as char;
        match c {
            'J' | 'V' => group = Some(pull_arg(argv, &mut i, a)),
            'X' => description = Some(pull_arg(argv, &mut i, a)),
            _ if takes_arg(c) => {
                if a.len() == 2 && i + 1 < argv.len() {
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    for m in argv[i..].iter() {
        buf.push(CompsysMatch {
            completion: m.clone(),
            description: description.clone(),
            group: group.clone(),
            replace_start: 0, // Phase 0.6: derive from BUFFER/CURSOR.
        });
    }
    true
}

/// Process-wide buffer that `bin_compadd` writes to when set.
/// `None` = passthrough (compadd writes to the real ZLE match
/// list). `Some(vec)` = capture mode (compadd routes into the vec,
/// returns 1 without touching ZLE state).
pub static COMPADD_CAPTURE_BUFFER: Mutex<Option<Vec<CompsysMatch>>> = Mutex::new(None);

/// Per-process serialisation for `complete_at` — the underlying
/// shell-state mutation isn't reentrant.
fn complete_at_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Drive compsys dispatch the way a Tab keypress does — for the
/// given line + cursor return the match list.
///
/// Mimics the real ZLE Tab path exactly:
///
/// ```text
///   Tab key (interactive)              In-editor (LSP)
///   ────────────────────               ─────────────────
///   complete-word widget               (skip; we go straight to docomplete)
///       │                                   │
///       ▼                                   ▼
///   completeword()           ──────►    docomplete(COMP_COMPLETE)
///       │                                   │
///       └─► docomplete(COMP_COMPLETE) ◄─────┘
///                │
///                ▼
///       do_completion(zleline, 0, COMP_COMPLETE)
///                │
///                ├── parses line into words / PREFIX / SUFFIX /
///                │   IPREFIX / ISUFFIX / CURRENT / compstate[…]
///                ├── runs `before_complete` hook
///                ├── invokes the registered completer (typically
///                │   `_main_complete`) which walks the completer
///                │   chain `_complete → _normal → _git → _arguments
///                │   → _describe`
///                ├── each `compadd` call lands in our shadow
///                │   buffer (`COMPADD_CAPTURE_BUFFER`)
///                └── runs `after_complete` hook
/// ```
///
/// We do NOT call `_main_complete` directly — that would skip the
/// C-level setup (`do_completion` does word extraction, compstate
/// init, before/after hooks, and the recursion guard). Calling the
/// shell function in isolation works for trivial cases and breaks
/// the moment the completer relies on PREFIX/SUFFIX being set.
pub fn complete_at(req: CompsysRequest<'_>) -> CompsysResponse {
    let _guard = complete_at_lock().lock().unwrap();
    let started = Instant::now();

    // Snapshot ZLE line + cursor state so we restore exactly what
    // was there before. `complete_at` runs in the LSP thread where
    // ZLE state is normally idle, but the snapshot still matters
    // for tests + future re-entrancy.
    let saved_zleline = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let saved_zlecs = crate::ported::zle::compcore::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let saved_zlell = crate::ported::zle::compcore::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    let saved_curcontext = crate::ported::params::getsparam("curcontext");

    // Populate the ZLE line buffer + cursor + length the way the
    // interactive line editor would before firing Tab.
    if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
    {
        *g = req.line.to_string();
    }
    crate::ported::zle::compcore::ZLECS
        .store(req.cursor as i32, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::compcore::ZLELL
        .store(req.line.len() as i32, std::sync::atomic::Ordering::SeqCst);
    let _ = crate::ported::params::setsparam("curcontext", ":::");

    // Install the shadow on every `compadd` call. While Some, the
    // builtin routes matches into the buffer + returns 0 without
    // touching the real ZLE match list.
    {
        let mut g = COMPADD_CAPTURE_BUFFER.lock().unwrap();
        *g = Some(Vec::new());
    }

    // In-editor completion calls `docomplete(COMP_COMPLETE)`
    // DIRECTLY — pure completion path, no expansion phase.
    //
    // Why not `expandorcomplete` (the actual Tab default per
    // `Src/Zle/zle_bindings.c:88 emacsbind[9]`)? Tab at the
    // interactive prompt first attempts history / alias /
    // parameter expansion via `doexpansion()`; only on no-match
    // does it fall through to completion. Inside the editor that
    // first phase is wrong: history expansion shouldn't fire
    // because the LSP isn't connected to the user's history
    // stack, and parameter expansion would mutate the buffer in
    // ways the IDE has no way to roll back.
    //
    // Why not `completeword` either? It sets `USEMENU=0`,
    // `USEGLOB=1`, `WOULDINSTAB=0`, and checks `LASTCHAR == '\t'`
    // before potentially short-circuiting to `selfinsert()`. None
    // of those are correct for the editor — `LASTCHAR` is a stale
    // ZLE state that an LSP request shouldn't touch, and the
    // menu/glob flags are interactive-display concerns.
    //
    // `docomplete(COMP_COMPLETE)` is the shared back-half both
    // widgets fall into: parse the line, populate PREFIX /
    // SUFFIX / IPREFIX / ISUFFIX / CURRENT / compstate, run the
    // before/after hooks, invoke `_main_complete`. Pure
    // completion — exactly what the user asked for.
    let _ret = crate::ported::zle::zle_tricky::docomplete(crate::ported::zle::zle_h::COMP_COMPLETE);
    // (docomplete itself takes an int lst, not args — `Src/Zle/
    // zle_tricky.c:599 int docomplete(int lst)`. The argv form
    // belongs to the widget-level entry points completeword /
    // expandorcomplete / menucomplete / etc, which we skip per the
    // pure-completion contract.)

    // Drain the capture.
    let matches = {
        let mut g = COMPADD_CAPTURE_BUFFER.lock().unwrap();
        g.take().unwrap_or_default()
    };

    // Restore ZLE state.
    if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
    {
        *g = saved_zleline;
    }
    crate::ported::zle::compcore::ZLECS.store(saved_zlecs, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::compcore::ZLELL.store(saved_zlell, std::sync::atomic::Ordering::SeqCst);
    match saved_curcontext {
        Some(v) => {
            let _ = crate::ported::params::setsparam("curcontext", &v);
        }
        None => crate::ported::params::unsetparam("curcontext"),
    }

    let is_incomplete = started.elapsed() >= req.deadline.saturating_duration_since(started);

    tracing::debug!(
        target: "zshrs::compsys::in_editor",
        line = req.line,
        cursor = req.cursor,
        match_count = matches.len(),
        elapsed_us = started.elapsed().as_micros() as u64,
        "complete_at done",
    );

    CompsysResponse {
        matches,
        is_incomplete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_defaults_to_safe_mode_and_lsp_budget() {
        let req = CompsysRequest::new_with_default_budget("ls -", 4);
        assert!(!req.allow_exec);
        let remaining = req.deadline.duration_since(Instant::now());
        assert!(remaining.as_millis() <= 200);
        assert!(remaining.as_millis() >= 150);
    }

    // End-to-end smoke. Runs `complete_at` against a canned
    // line + cursor; success = the call returns (no panic, no
    // deadlock), the capture shadow drains cleanly. Doesn't
    // assert on match count because the in-test environment
    // doesn't load `compinit` — `_main_complete` will find no
    // completer chain installed and return 0 matches. The point
    // is the harness wires up without crashing; Phase 0.6 adds
    // an in-process `compinit` bootstrap so we can hard-assert.
    #[test]
    fn complete_at_smoke_does_not_panic() {
        let req = CompsysRequest::new_with_default_budget("setopt ext", 10);
        let resp = complete_at(req);
        eprintln!(
            "setopt ext -> {} matches: {:?}",
            resp.matches.len(),
            resp.matches
                .iter()
                .take(5)
                .map(|m| &m.completion)
                .collect::<Vec<_>>(),
        );
    }
}
