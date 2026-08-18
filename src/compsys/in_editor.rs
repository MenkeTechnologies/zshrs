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
//! # The shell thread
//!
//! Every dispatch runs on ONE dedicated thread ([`shell_thread`]),
//! never on the caller's. Two hard requirements force that:
//!
//!   * `exec::dispatch_function_call` (exec.rs:8088) resolves the VM
//!     through `fusevm_bridge::try_with_executor` / `SESSION_EXECUTOR`
//!     — both THREAD-LOCAL. A completer invoked from a thread with no
//!     executor installed silently returns `None`, so `_git` and every
//!     other shell-defined completer is a no-op.
//!   * The ported compsys runtime keeps state in process-globals that
//!     are not reentrant. One thread = one dispatch at a time, no lock
//!     discipline to get wrong.
//!
//! Thread startup is the compsys bootstrap:
//!
//! ```text
//! ShellExecutor::new()               ← option table, params, env import
//!     └── canonical_apply::apply_all ← ~/.zshrs/images/*-recorder.rkyv
//!             ├── fpath              ← where completer bodies load from
//!             ├── compdef map        ← _comps[git]=_git
//!             ├── autoload stubs     ← PM_UNDEFINED shfunctab entries
//!             └── zstyle / aliases / params / bindkeys
//!     └── exec::install_session_executor
//! ```
//!
//! rkyv shard only — no SQLite anywhere on this path. When no shard
//! exists (`zshrs-recorder` never ran) `apply_all` returns 0 and the
//! thread still serves: matches come from whatever the ported Rust
//! completers produce without user state.
//!
//! # Current scope
//!
//!   * Whitespace word-split (no quote / parameter / brace expansion
//!     handling yet).
//!   * Single completer invocation per request — no result cache.
//!   * Snapshot/restore covers the 5 params we set; deeper state
//!     (option flags, hash entries) isn't snapshotted today and
//!     could leak between requests in pathological compsys functions.
//!     Those leaks are visible only in test setups that hand-edit
//!     `compstate` from inside a compdef function — none of the 50+
//!     ported functions do.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

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
    /// When `false`, completion functions that shell out
    /// (`_call_program` — `git branch`, `docker ps`, `kubectl get
    /// pods`) get no subprocess: the call returns 1 with an empty
    /// `$REPLY`, so the completer falls back to its static specs.
    /// When `true` the subprocess runs but is KILLED at `deadline`
    /// (see `_call_program`'s `in_editor::exec_deadline` check), so
    /// a hung helper can never wedge the editor.
    pub allow_exec: bool,
}

impl<'a> CompsysRequest<'a> {
    /// Build a request with the LSP-default 200 ms deadline.
    ///
    /// `allow_exec = true`: the editor gets the same match list the
    /// prompt does, including exec-backed ones (`git checkout <tab>`
    /// → branch names). The deadline bounds it — every subprocess is
    /// killed when the budget runs out.
    pub fn new_with_default_budget(line: &'a str, cursor: usize) -> Self {
        Self {
            line,
            cursor,
            deadline: Instant::now() + Duration::from_millis(200),
            allow_exec: true,
        }
    }

    /// Same, with an explicit budget. The LSP uses a wider one for
    /// the first request after startup (cold autoload of a large
    /// completer like `_git` is a 400 KB parse).
    pub fn new_with_budget(line: &'a str, cursor: usize, budget: Duration) -> Self {
        Self {
            line,
            cursor,
            deadline: Instant::now() + budget,
            allow_exec: true,
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

/// Called from the ported `bin_compadd` body — after its flag loop,
/// with the parsed [`Cadata`] and the residual match words — when the
/// in-editor capture shadow is active. Appends one [`CompsysMatch`]
/// per proposed match to [`COMPADD_CAPTURE_BUFFER`] and returns `true`
/// so the caller short-circuits (status 0, "matches added") instead of
/// touching ZLE state. Returns `false` when the buffer is inactive —
/// the normal `addmatches` path then runs untouched.
///
/// Everything flag-shaped is already resolved by the port's C-faithful
/// parse; this only has to read it:
///
///   * `dat.aflags & CAF_ARRAYS` (`-a`) — `words` are ARRAY NAMES and
///     the matches are their elements. `_arguments` takes this path for
///     every literal action list: `'1:cmd:(build test)'` becomes
///     `compadd … -a - ws` with `$ws=(build test)`
///     (`_arguments.rs:1264-1280`).
///   * `dat.aflags & CAF_KEYS` (`-k`) — names of assocs, matches are
///     their KEYS; a plain array under `-k` contributes its elements
///     (`_path_commands`, `_users`, `_tilde`, `_value`).
///   * `dat.disp` (`-d`) — parallel display array: element _i_
///     describes match _i_ (`_describe` pairs `-d _tmpd -a _tmpm`).
///   * `dat.exp` (`-X`) — one explanation for the whole batch, used
///     when the display array has nothing for this index.
///   * `dat.group` (`-J`/`-V`) — group label, which the LSP maps to a
///     completion-item kind.
pub fn try_capture_compadd(dat: &crate::ported::zle::comp_h::Cadata, words: &[String]) -> bool {
    use crate::ported::zle::comp_h::{CAF_ARRAYS, CAF_KEYS};

    let mut guard = match COMPADD_CAPTURE_BUFFER.lock() {
        Ok(g) => g,
        Err(_) => return false,
    };
    let buf = match guard.as_mut() {
        Some(b) => b,
        None => return false,
    };

    // QUERY forms are not proposals and must not be shadowed. `-O
    // name` / `-A name` store the candidates in a parameter, `-D name`
    // narrows one in place, and in all three C adds nothing to the
    // match list — the caller wants the array, not a popup. Let the
    // real `addmatches` run so the side effect happens.
    //
    // `_git` depends on exactly this: `compadd "$expl[@]" -O
    // allmatching -a allcmds` then
    // `len=${#${(O)allmatching//?/.}[1]}` — the longest match — and
    // pads every description line to `len` (`_git:6783-6789`). With the
    // query shadowed, `$allmatching` stayed empty, `len` collapsed, and
    // every `git <tab>` description arrived truncated to four
    // characters ("archive" described as "arch").
    if dat.opar.is_some() || dat.apar.is_some() || !dat.dpar.is_empty() {
        return false;
    }

    let matches: Vec<String> = if (dat.aflags & CAF_ARRAYS) != 0 {
        words
            .iter()
            .flat_map(|name| crate::ported::params::getaparam(name).unwrap_or_default())
            .collect()
    } else if (dat.aflags & CAF_KEYS) != 0 {
        words
            .iter()
            .flat_map(|name| {
                crate::ported::params::gethkparam(name)
                    .or_else(|| crate::ported::params::getaparam(name))
                    .unwrap_or_default()
            })
            .collect()
    } else {
        words.to_vec()
    };

    let descs: Vec<String> = dat
        .disp
        .as_deref()
        .and_then(crate::ported::params::getaparam)
        .unwrap_or_default();

    if let Some(path) = std::env::var_os("ZSHRS_CAPDBG") {
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "capture disp={:?} exp={:?} group={:?} matches={:?} descs={:?}",
                dat.disp,
                dat.exp,
                dat.group,
                matches.iter().take(4).collect::<Vec<_>>(),
                descs.iter().take(4).collect::<Vec<_>>(),
            );
        }
    }
    for (idx, m) in matches.iter().enumerate() {
        buf.push(CompsysMatch {
            completion: m.clone(),
            description: descs.get(idx).cloned().or_else(|| dat.exp.clone()),
            group: dat.group.clone(),
            replace_start: 0,
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

/// One completion request handed to the shell thread. The reply
/// channel is per-request: when a caller gives up on its deadline it
/// drops the receiver, the late `send` fails, and the shell thread
/// moves on to the next job instead of blocking on a dead client.
struct Job {
    line: String,
    cursor: usize,
    allow_exec: bool,
    deadline: Instant,
    reply: SyncSender<CompsysResponse>,
}

/// `true` once the shell thread finished [`bootstrap_shell`]. Until
/// then `complete_at` returns `is_incomplete` immediately rather than
/// blocking the LSP's request loop behind a cold start.
static SHELL_READY: AtomicBool = AtomicBool::new(false);

/// Set while the shell thread is inside a dispatch. Diagnostics only —
/// requests are queued, not rejected, because a rejected keystroke is
/// an empty popup while a queued one is merely late.
static SHELL_BUSY: AtomicBool = AtomicBool::new(false);

/// Last completed dispatch: `(line, cursor, matches, finished_at)`.
///
/// This is what makes a slow completer usable in an editor. A cold
/// `_git` (424 KB of shell to autoload, plus its `git` helper calls)
/// overruns any budget an interactive popup can wait for, so the first
/// request answers `is_incomplete` — the LSP contract for "ask again".
/// The dispatch keeps running on the shell thread and lands here, so
/// the client's next request for the same line is served from memory
/// with no dispatch at all.
///
/// One entry: completions are strictly per-cursor-position, and the
/// only consumer is the request that immediately follows.
static LAST_RESULT: Mutex<Option<(String, usize, Vec<CompsysMatch>, Instant)>> = Mutex::new(None);

/// How long a cached dispatch stays servable. Long enough to cover an
/// editor's re-request round trip, short enough that a completer whose
/// output depends on the working tree (`git branch`) is not answered
/// from a stale run.
const LAST_RESULT_TTL: Duration = Duration::from_secs(3);

/// `true` once a recorder shard was applied (or once we know there is
/// none). Read by [`shard_applied`] for the LSP's diagnostics.
static SHARD_ROWS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Rows applied from the rkyv canonical shard at bootstrap. 0 means
/// no shard on disk (`zshrs-recorder` never ran) — completions still
/// work, they just carry no user state (fpath, compdef, zstyle).
pub fn shard_rows() -> usize {
    SHARD_ROWS.load(Ordering::SeqCst)
}

/// `true` once the shell thread is serving requests.
pub fn is_ready() -> bool {
    SHELL_READY.load(Ordering::SeqCst)
}

/// Start the shell thread if it isn't running. Idempotent, non-
/// blocking: the caller returns immediately and [`is_ready`] flips
/// when the bootstrap lands. The LSP calls this from `initialize` so
/// the first real completion request finds a warm shell.
pub fn bootstrap() {
    let _ = shell_thread();
}

/// The bootstrap the shell thread runs before serving anything.
///
/// Builds a real `ShellExecutor` (option table + params + env
/// import), pours the daemon's canonical rkyv shard into it
/// (`~/.zshrs/images/*-recorder.rkyv` — fpath, compdef map, autoload
/// stubs, zstyle, aliases), then installs it as this thread's session
/// executor so `exec::dispatch_function_call` can reach the VM.
///
/// The executor is leaked deliberately: `install_session_executor`
/// stores a raw pointer that must outlive every dispatch, and the
/// shell thread lives for the whole process.
fn bootstrap_shell() {
    let t0 = Instant::now();
    let executor: &'static mut crate::vm_helper::ShellExecutor =
        Box::leak(Box::new(crate::vm_helper::ShellExecutor::new()));

    // rkyv canonical shard — the ONLY state source here. No SQLite:
    // `canonical_apply` replays compdef rows through the same
    // `compinit::compdef` entry point an interactive `compdef _git
    // git` lands at (canonical_apply.rs:258-270).
    #[cfg(feature = "daemon")]
    {
        let rows = crate::canonical_apply::apply_all(executor);
        SHARD_ROWS.store(rows, Ordering::SeqCst);
        if rows == 0 {
            tracing::warn!(
                target: "zshrs::compsys::in_editor",
                "no canonical rkyv shard applied — run `zshrs-recorder` for user fpath/compdef state",
            );
        }
    }

    crate::ported::exec::install_session_executor(executor);
    SHELL_READY.store(true, Ordering::SeqCst);
    tracing::info!(
        target: "zshrs::compsys::in_editor",
        shard_rows = SHARD_ROWS.load(Ordering::SeqCst),
        elapsed_ms = t0.elapsed().as_millis() as u64,
        "shell thread ready",
    );
}

/// Handle to the shell thread's job queue, spawning it on first use.
///
/// Bound 4: enough that a burst of keystrokes doesn't fail to enqueue
/// while the current dispatch finishes, small enough that a stalled
/// completer can't accumulate a backlog of dead lines.
fn shell_thread() -> &'static SyncSender<Job> {
    static TX: OnceLock<SyncSender<Job>> = OnceLock::new();
    TX.get_or_init(|| {
        let (tx, rx) = sync_channel::<Job>(4);
        std::thread::Builder::new()
            .name("zshrs-compsys-editor".to_string())
            .spawn(move || shell_thread_main(rx))
            .expect("spawn compsys editor shell thread");
        tx
    })
}

thread_local! {
    /// In-editor exec policy for the CURRENT dispatch, or `None` when
    /// this thread isn't serving one (every ordinary shell thread).
    /// `Some((allow, deadline))`: `allow == false` means no subprocess
    /// at all; `true` means spawn but kill at `deadline`.
    ///
    /// Thread-local because the shell thread is the only place an
    /// in-editor dispatch runs — an interactive shell must never see
    /// a completion budget applied to its `_call_program` calls.
    static EXEC_POLICY: std::cell::Cell<Option<(bool, Instant)>> =
        const { std::cell::Cell::new(None) };
}

/// Arm the in-editor exec policy for one dispatch.
fn set_exec_policy(allow_exec: bool, deadline: Instant) {
    EXEC_POLICY.with(|c| c.set(Some((allow_exec, deadline))));
}

/// Disarm it — must run on every exit path out of a dispatch, or the
/// shell thread would carry a stale deadline into the next request.
fn clear_exec_policy() {
    EXEC_POLICY.with(|c| c.set(None));
}

/// The exec policy in force on this thread.
///
/// Returns `None` outside an in-editor dispatch: `_call_program` then
/// behaves exactly as it does in the interactive shell (unbounded
/// `Command::output()`).
pub fn exec_policy() -> Option<(bool, Instant)> {
    EXEC_POLICY.with(|c| c.get())
}

/// Shell-thread body: bootstrap once, then serve jobs forever.
fn shell_thread_main(rx: Receiver<Job>) {
    bootstrap_shell();
    for job in rx {
        // A job whose deadline already passed while it sat in the
        // queue is a stale line — the client stopped waiting.
        if Instant::now() >= job.deadline {
            continue;
        }
        SHELL_BUSY.store(true, Ordering::SeqCst);
        let resp = dispatch_on_shell_thread(&job);
        SHELL_BUSY.store(false, Ordering::SeqCst);
        // Publish before replying: the client that gave up on this
        // dispatch is about to ask again, and that retry must hit.
        if !resp.matches.is_empty() {
            if let Ok(mut g) = LAST_RESULT.lock() {
                *g = Some((
                    job.line.clone(),
                    job.cursor,
                    resp.matches.clone(),
                    Instant::now(),
                ));
            }
        }
        // Late reply → receiver dropped → ignore and take the next job.
        let _ = job.reply.send(resp);
    }
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
    let tx = shell_thread();
    // Cold start: the shell thread is still applying the shard.
    // Answer now with "incomplete" so the editor's request loop never
    // blocks on a bootstrap; the client re-requests and gets matches.
    if !is_ready() {
        return CompsysResponse {
            matches: Vec::new(),
            is_incomplete: true,
        };
    }
    // A dispatch that overran an earlier request's budget finished in
    // the background; if it was for THIS line, serve it now.
    if let Ok(mut g) = LAST_RESULT.lock() {
        let hit = g
            .as_ref()
            .filter(|(line, cursor, _, at)| {
                line == req.line && *cursor == req.cursor && at.elapsed() < LAST_RESULT_TTL
            })
            .map(|(_, _, matches, _)| matches.clone());
        if let Some(matches) = hit {
            *g = None;
            return CompsysResponse {
                matches,
                is_incomplete: false,
            };
        }
    }
    let (reply_tx, reply_rx) = sync_channel::<CompsysResponse>(1);
    let job = Job {
        line: req.line.to_string(),
        cursor: req.cursor,
        allow_exec: req.allow_exec,
        deadline: req.deadline,
        reply: reply_tx,
    };
    if tx.try_send(job).is_err() {
        return CompsysResponse {
            matches: Vec::new(),
            is_incomplete: true,
        };
    }
    // Wait out the caller's budget plus a small grace so a dispatch
    // that finishes right at the deadline still gets its matches
    // delivered instead of being thrown away.
    let wait = req
        .deadline
        .saturating_duration_since(Instant::now())
        .saturating_add(Duration::from_millis(20));
    match reply_rx.recv_timeout(wait) {
        Ok(resp) => resp,
        Err(_) => CompsysResponse {
            matches: Vec::new(),
            is_incomplete: true,
        },
    }
}

/// The dispatch half of [`complete_at`], running on the shell thread
/// with a session executor installed. Never call this directly — the
/// executor is thread-local, so off-thread callers get zero matches
/// from every shell-defined completer.
fn dispatch_on_shell_thread(job: &Job) -> CompsysResponse {
    let _guard = complete_at_lock().lock().unwrap();
    let req = CompsysRequest {
        line: &job.line,
        cursor: job.cursor,
        deadline: job.deadline,
        allow_exec: job.allow_exec,
    };
    let started = Instant::now();

    // Snapshot ZLE line + cursor state so we restore exactly what
    // was there before. `complete_at` runs on the shell thread where
    // ZLE state is normally idle, but the snapshot still matters
    // for tests + future re-entrancy.
    let saved_zleline = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let saved_zlecs = crate::ported::zle::compcore::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let saved_zlell = crate::ported::zle::compcore::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    let saved_ed_line = crate::ported::zle::zle_main::ZLELINE
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let saved_ed_cs = crate::ported::zle::zle_main::ZLECS.load(std::sync::atomic::Ordering::SeqCst);
    let saved_ed_ll = crate::ported::zle::zle_main::ZLELL.load(std::sync::atomic::Ordering::SeqCst);
    let saved_curcontext = crate::ported::params::getsparam("curcontext");

    // Populate the ZLE line buffer + cursor + length the way the
    // interactive line editor would before firing Tab.
    //
    // The EDITOR buffer (`zle_main::ZLELINE`, a `Vec<char>` that
    // `self-insert` writes) is the authoritative one: `docomplete`
    // copies it into the completion buffer and re-metafies before
    // `get_comp_string` runs (zle_tricky.rs:855-887). Writing only
    // `compcore::ZLELINE` here was overwritten a few statements later,
    // so the whole engine ran against `s=""` — `$words` came back as
    // `[""]` and every completer produced nothing.
    //
    // Editor-side offsets are CHAR indices; `req.cursor` is a byte
    // offset, so convert (identical for ASCII, not for anything else).
    let line_chars: Vec<char> = req.line.chars().collect();
    let cursor_chars = req.line[..req.cursor.min(req.line.len())].chars().count();
    let line_len_chars = line_chars.len();
    if let Ok(mut g) = crate::ported::zle::zle_main::ZLELINE.lock() {
        *g = line_chars;
    }
    crate::ported::zle::zle_main::ZLECS.store(cursor_chars, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.store(line_len_chars, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
    {
        *g = req.line.to_string();
    }
    crate::ported::zle::compcore::ZLECS
        .store(cursor_chars as i32, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::compcore::ZLELL
        .store(line_len_chars as i32, std::sync::atomic::Ordering::SeqCst);
    let _ = crate::ported::params::setsparam("curcontext", ":::");

    // Install the shadow on every `compadd` call. While Some, the
    // builtin routes matches into the buffer + returns 0 without
    // touching the real ZLE match list.
    {
        let mut g = COMPADD_CAPTURE_BUFFER.lock().unwrap();
        *g = Some(Vec::new());
    }

    // Give the dispatch a terminal geometry. There is no tty here, so
    // `$COLUMNS` / `$LINES` read 0 — and completers do arithmetic on
    // them for list layout. `_git` right-pads every description line
    // with `${(r.COLUMNS-4.)…}` (`_git:6788`): at COLUMNS=0 that width
    // is -4, and `git <tab>` came back with every description clipped
    // to four characters ("archive" described as "arch").
    //
    // 80x24 is what zsh itself falls back to with no terminal
    // (`adjustcolumns`, utils.rs:1869 — `tccolumns` else 80), so
    // completers see the geometry they were written against. Wider
    // values are worse, not better: the padding those layout
    // expressions build is proportional to `$COLUMNS`, and at 200 the
    // `${(r.COLUMNS-4.)…}` pass over `_git`'s command table took long
    // enough to look like a hang.
    let saved_columns = crate::ported::params::getiparam("COLUMNS");
    let saved_lines = crate::ported::params::getiparam("LINES");
    let _ = crate::ported::params::setiparam("COLUMNS", 80);
    let _ = crate::ported::params::setiparam("LINES", 24);

    // EVERY in-editor request is a fresh first Tab. There is no menu
    // to step, no previous list to reuse, no ambiguous-completion
    // history — the editor asks for a match list and nothing else.
    //
    // The interactive engine assumes the opposite: `before_complete`
    // (compcore.rs:543) sees `minfo.cur` set + `menucmp` on and
    // short-circuits into `do_menucmp` — "this Tab advances the menu"
    // — returning before `_main_complete` is ever called. Since the
    // previous dispatch on this thread leaves exactly that state
    // behind, a SECOND request for the same line came back with zero
    // matches after 118 µs, having never run a completer.
    //
    // So clear the continuation state up front.
    crate::ported::zle::zle_tricky::MENUCMP.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::compcore::OLDMENUCMP.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::zle_tricky::LASTAMBIG.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::zle_tricky::VALIDLIST.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::zle_tricky::SHOWAGAIN.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::zle_refresh::SHOWINGLIST.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::zle_refresh::LISTSHOWN.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::compcore::hasoldlist.store(0, std::sync::atomic::Ordering::Relaxed);
    crate::ported::zle::compcore::lastpermmnum.store(0, std::sync::atomic::Ordering::Relaxed);
    if let Some(m) = crate::ported::zle::compcore::MINFO.get() {
        if let Ok(mut mi) = m.lock() {
            mi.cur = None;
            mi.group = None;
            mi.cur_idx = 0;
            mi.group_idx = 0;
        }
    }

    // `compfunc` is what makes this the COMPSYS path. `makecomplist`
    // (compcore.rs:1993) reads it to pick the completion widget's
    // shell function; when it is None the C-faithful code takes the
    // compctl branch instead and no compsys completer ever runs, so
    // `git <tab>` came back with nothing.
    //
    // Interactively `completecall` (zle_tricky.rs:117) plants it from
    // the `zle -C` widget's `func` field. We drive `docomplete`
    // directly (no widget dispatch), so plant it here — `_main_complete`
    // is what `compinit`'s standard widgets bind (compinit sh:542).
    let saved_compfunc = {
        let g = crate::ported::zle::compcore::compfunc.get_or_init(|| Mutex::new(None));
        let mut lock = g.lock().unwrap_or_else(|e| e.into_inner());
        let prev = lock.clone();
        *lock = Some("_main_complete".to_string());
        prev
    };

    // Exec budget for `_call_program`: with `allow_exec` the helper
    // runs but gets killed at the deadline; without it, no subprocess
    // is spawned at all.
    set_exec_policy(req.allow_exec, req.deadline);

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
    clear_exec_policy();
    let _ = crate::ported::params::setiparam("COLUMNS", saved_columns);
    let _ = crate::ported::params::setiparam("LINES", saved_lines);
    {
        let g = crate::ported::zle::compcore::compfunc.get_or_init(|| Mutex::new(None));
        let mut lock = g.lock().unwrap_or_else(|e| e.into_inner());
        *lock = saved_compfunc;
    }
    if let Ok(mut g) = crate::ported::zle::compcore::ZLELINE
        .get_or_init(|| std::sync::Mutex::new(String::new()))
        .lock()
    {
        *g = saved_zleline;
    }
    crate::ported::zle::compcore::ZLECS.store(saved_zlecs, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::compcore::ZLELL.store(saved_zlell, std::sync::atomic::Ordering::SeqCst);
    if let Ok(mut g) = crate::ported::zle::zle_main::ZLELINE.lock() {
        *g = saved_ed_line;
    }
    crate::ported::zle::zle_main::ZLECS.store(saved_ed_cs, std::sync::atomic::Ordering::SeqCst);
    crate::ported::zle::zle_main::ZLELL.store(saved_ed_ll, std::sync::atomic::Ordering::SeqCst);
    match saved_curcontext {
        Some(v) => {
            let _ = crate::ported::params::setsparam("curcontext", &v);
        }
        None => {
            let _ = crate::ported::params::unsetparam("curcontext");
        }
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
    fn request_defaults_to_exec_allowed_and_lsp_budget() {
        // Exec is ON by default: an editor completion is expected to
        // match the prompt, and the prompt's `git checkout <tab>` runs
        // `git for-each-ref`. The deadline is what makes that safe —
        // `_call_program` kills the helper when it expires.
        let req = CompsysRequest::new_with_default_budget("ls -", 4);
        assert!(req.allow_exec);
        let remaining = req.deadline.duration_since(Instant::now());
        assert!(remaining.as_millis() <= 200);
        assert!(remaining.as_millis() >= 150);
    }

    #[test]
    fn exec_policy_is_unset_outside_a_dispatch() {
        // `_call_program` branches on this: `None` means "not an
        // in-editor call" and it must keep the interactive unbounded
        // `Command::output()` path. A leaked policy would apply a
        // completion deadline to the user's real shell.
        assert!(exec_policy().is_none());
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
