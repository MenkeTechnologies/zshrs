//! Faithful Rust ports of free functions and file-static globals from
//! `Src/exec.c`. The wordcode-VM dispatch tree (`execlist` / `execpline`
//! / `execcmd` / `execsimple` etc.) that drives execution in C zsh is
//! NOT replicated here — zshrs runs the fusevm bytecode VM instead
//! (see `src/vm_helper.rs` + `src/fusevm_bridge.rs`).
//!
//! What lives here are the parts of `Src/exec.c` that ARE faithful
//! ports and don't depend on the C-side wordcode walker:
//!
//! - **`trap_state` / `trap_return` / `forklevel`** — file-static
//!   integer globals from `Src/exec.c:134 / :155 / :1052`, exposed as
//!   atomics shared between this module, `Src/signals.c`'s port at
//!   `src/ported/signals.rs`, and `Src/params.c`'s port at
//!   `src/ported/params.rs`.
//! - **`gethere`** (`Src/exec.c:4573`) — turn a here-document into a
//!   here-string. Called from the lexer port (`src/ported/lex.rs`).
//! - **`getoutput`** (`Src/exec.c:4712`) — command-substitution body
//!   runner. Called from the parameter-expansion port
//!   (`src/ported/subst.rs`).
//! - **`loadautofn`** + **`getfpfunc`** (`Src/exec.c:5050` / `:5260`)
//!   — `$fpath` walker + autoload file installer. Called from
//!   `bin_autoload` / `bin_functions -c` in `src/ported/builtin.rs`.
//! - **`resolvebuiltin`** (`Src/exec.c:2703`) — module-autoload guard
//!   used by the dispatch walk in `execcmd_exec`.
//! - **`execcmd_exec`** (`Src/exec.c:2900`) — head section only
//!   (locals + precommand-modifier walk through `c:3091`). The rest
//!   of the C function drives the wordcode-walker dispatch and is
//!   replaced by fusevm bytecode in `src/extensions/compile_zsh.rs`;
//!   see the WARNING block inside the function body.

use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::Ordering;

use crate::fusevm_bridge::with_executor;
use crate::ported::builtin::{cd_able_vars, fixdir, BUILTINS, DOPRINTDIR, LASTVAL};
use crate::ported::builtins::rlimits::setlimits;
use crate::ported::compat::zgettime_monotonic_if_available;
use crate::ported::config_h::DEFAULT_PATH;
use crate::ported::context::{zcontext_restore, zcontext_save};
use crate::ported::hashtable::{cmdnam_unhashed, cmdnamtab_lock, dircache_set, hashdir, pathchecked, shfunctab_lock};
use crate::ported::hist::{strinbeg, strinend};
use crate::ported::init::{underscorelen, underscoreused, zunderscore};
use crate::ported::input::{inpop, inpush};
use crate::ported::jobs::{expandjobtab, JOBTAB, THISJOB};
use crate::ported::lex::{hgetc, parsestr, tok, untokenize, ztokens, LEXERR, LEX_LEXSTOP, LEX_LINENO};
use crate::ported::mem::{dupstring, popheap, pushheap};
use crate::ported::options::sticky;
use crate::ported::params::{getsparam, paramtab};
use crate::ported::parse::{ecrawstr, parse_list};
use crate::ported::signals::{queue_signals, unqueue_signals};
use crate::ported::subst::{quotesubst, singsub};
use crate::ported::utils::{errflag, gettempfile, gettempname, movefd, unmeta, unmetafy, write_loop, zclose, zerr, zwarn, ERRFLAG_ERROR};
use crate::ported::ztype_h::{inull, itok};
use crate::ported::zsh_h::{builtin, eprog, hashnode, redir, shfunc, BINF_BUILTIN, BINF_CLEARENV, BINF_COMMAND, BINF_DASH, BINF_EXEC, BINF_PREFIX, ERRFLAG_INT, INP_LINENO, IS_DASH, PM_UNDEFINED, REDIRF_FROM_HEREDOC, REDIR_HEREDOCDASH, WC_TYPESET, wc_code, Z_END, WC_LIST, WC_LIST_TYPE, WC_PIPE, WC_PIPE_END, WC_PIPE_TYPE, WC_REDIR, WC_REDIR_TYPE, WC_REDIR_VARID, WC_SIMPLE, WC_SIMPLE_ARGC, WC_SUBLIST, WC_SUBLIST_END, WC_SUBLIST_FLAGS, WC_SUBLIST_TYPE, Meta, Nularg, Pound, isset, CHASEDOTS, CHASELINKS, Outpar, Inpar, PM_LOADDIR, VERBOSE, Emulation_options, emulation_options, CLOBBER, IS_CLOBBER_REDIR, CLOBBEREMPTY, cmdnam, HASHDIRS, PATHDIRS, PM_READONLY};
use crate::zsh_h::execstack;

/// Port of `int trap_state;` from `Src/exec.c:134`. Tracks whether
/// a trap handler is currently being processed and, paired with
/// `TRAP_RETURN` below, whether a `return` inside the trap should
/// promote to `TRAP_STATE_FORCE_RETURN` to unwind the trap caller.
///
/// Values: `TRAP_STATE_INACTIVE = 0`, `TRAP_STATE_PRIMED = 1`,
/// `TRAP_STATE_FORCE_RETURN = 2` (see `Src/zsh.h`).
pub static TRAP_STATE: std::sync::atomic::AtomicI32 = // c:134 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int trap_return;` from `Src/exec.c:155`. Carries the
/// pending exit status from inside a trap; sentinel `-2` means
/// "running an EXIT/DEBUG-style trap at the current level"
/// (signals.c:1166). Promoted to the user's `return N` value by
/// `bin_return` when POSIX-trap semantics apply (builtin.c:5852).
pub static TRAP_RETURN: std::sync::atomic::AtomicI32 = // c:155 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int forklevel;` from `Src/exec.c:1052`. Records the
/// `locallevel` at the most recent fork point (set at c:1221:
/// `forklevel = locallevel;` inside `entersubsh()`). Used by:
///   - `signals.c:808` SIGPIPE handler — `!forklevel` distinguishes
///     the top-level shell from a forked subshell.
///   - `exec.c:6146` — `if (locallevel > forklevel)` decides whether
///     a function-defined trap should fire on this subshell exit.
///   - `params.c:3724` — WARNCREATEGLOBAL nest-depth check.
///
/// Initialised to 0 (no fork has occurred yet). Set to `locallevel`
/// at every `entersubsh()` entry per c:1221.
pub static FORKLEVEL: std::sync::atomic::AtomicI32 = // c:1052 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

// =============================================================================
// File-static globals from Src/exec.c. Bucket choices per PORT_PLAN.md:
//   - Per-evaluator transient state → thread_local Cell (bucket 1)
//   - Shell-wide shared state       → AtomicI32 / Mutex (bucket 2)
// All names match C exactly. Surrounding doc-comments cite the C
// declaration line.
// =============================================================================

/// Port of `int noerrexit;` from `Src/exec.c:72`. Bit-flags that
/// suppress ERREXIT triggering on the next command(s). Bits:
/// `NOERREXIT_EXIT` (in `if`/`while`/`until` test contexts),
/// `NOERREXIT_RETURN` (after `return`), `NOERREXIT_UNTIL_EXEC`
/// (until next exec'd command). Bucket-1 — per-evaluator (each
/// recursive eval has its own suppression frame).
pub static noerrexit: std::sync::atomic::AtomicI32 = // c:72 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int this_noerrexit;` from `Src/exec.c:109`. When set,
/// suppress ERREXIT for THIS one command only (consumed + cleared
/// before the next command starts). Set by `execcursh` and the
/// `((expr))` arith path so a 0-result doesn't trigger errexit.
pub static this_noerrexit: std::sync::atomic::AtomicI32 = // c:109 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `mod_export int noerrs;` from `Src/exec.c:117`. When
/// non-zero, suppress `zerr()` output (lex error reporting during
/// `parse_string`, `parseopts` etc.). Saved/restored by
/// `execsave`/`execrestore`.
pub static noerrs: std::sync::atomic::AtomicI32 = // c:117 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int nohistsave;` from `Src/exec.c:122`. When non-zero,
/// `addhistnode` no-ops so trap firings / `eval` invocations don't
/// pollute `$HISTCMD`. Tracked alongside `noerrs` in the trap path.
pub static nohistsave: std::sync::atomic::AtomicI32 = // c:122 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int subsh;` from `Src/exec.c:160`. Subshell depth — bumped
/// every time `entersubsh` forks a sub-shell, used by signal handling
/// (different SIGINT semantics in subshells) and by `${$$}` (`$$`
/// stays at the top-level pid).
pub static subsh: std::sync::atomic::AtomicI32 = // c:160 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `mod_export volatile int retflag;` from `Src/exec.c:165`.
/// Set by `bin_return` to unwind the function-call stack. Cleared
/// by `runshfunc` on entry, checked by `execlist`'s main loop.
pub static retflag: std::sync::atomic::AtomicI32 = // c:165 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `pid_t cmdoutpid;` from `Src/exec.c:215`. Pid of the most
/// recent `$(cmd)` command-substitution child. Used by exit-status
/// propagation: `cmdoutval` carries the exit; `cmdoutpid` carries
/// the pid `waitpid`-d for it.
pub static cmdoutpid: std::sync::atomic::AtomicI32 = // c:215 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `mod_export pid_t procsubstpid;` from `Src/exec.c:220`.
/// Pid of the most recent process-substitution child (`<(cmd)` /
/// `>(cmd)`). Tracked separately from `cmdoutpid` because procsubst
/// jobs aren't wait-collected by the parent until the fd is closed.
pub static procsubstpid: std::sync::atomic::AtomicI32 = // c:220 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int cmdoutval;` from `Src/exec.c:225`. Exit status of
/// the most recent `$(cmd)`. Drives `$?` when a varspc-only command
/// runs alongside a substitution.
pub static cmdoutval: std::sync::atomic::AtomicI32 = // c:225 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int use_cmdoutval;` from `Src/exec.c:234`. When set,
/// `lastval` is updated from `cmdoutval` after the command
/// (i.e. the command had substitutions whose exit status matters).
pub static use_cmdoutval: std::sync::atomic::AtomicI32 = // c:234 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `mod_export int sfcontext;` from `Src/exec.c:239`. Source
/// context — one of `SFC_NONE`, `SFC_DIRECT` (user typed it),
/// `SFC_SIGNAL` (trap firing), `SFC_HOOK` (precmd/preexec etc.),
/// `SFC_WIDGET` (ZLE widget), `SFC_COMPLETE` (completion fn),
/// `SFC_CFUNC` (compsys fn), `SFC_SUBST` ($(...) cmd-subst),
/// `SFC_EVAL` (eval body). Read by `zerr()` / `funcstack` building.
pub static sfcontext: std::sync::atomic::AtomicI32 = // c:239 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int list_pipe = 0;` from `Src/exec.c:457`. Set when the
/// currently-executing pipeline is the long-running pipe-into-loop
/// shape (`cat foo | while read a; do ... done`) — drives the
/// super/sub-job tracking documented in the famous `Allen Edeln…`
/// comment block above this declaration in C.
pub static list_pipe: std::sync::atomic::AtomicI32 = // c:457 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int simple_pline = 0;` from `Src/exec.c:457`. Set during
/// dispatch of a "simple" pipeline (single-stage / no shell-construct
/// tail) so the `list_pipe` machinery short-circuits.
pub static simple_pline: std::sync::atomic::AtomicI32 = // c:457 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static pid_t list_pipe_pid;` from `Src/exec.c:459`.
/// PID of the sub-shell created to host the loop-after-pipe pattern;
/// passed up the recursive `execlist` stack so the cat-job's super-
/// job entry can record it.
pub static list_pipe_pid: std::sync::atomic::AtomicI32 = // c:459 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int nowait;` from `Src/exec.c:461`. When set,
/// `execpline` doesn't wait for the pipeline; used during the
/// list_pipe sub-shell fork bookkeeping.
pub static nowait: std::sync::atomic::AtomicI32 = // c:461 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `int pline_level = 0;` from `Src/exec.c:461`. Recursive
/// pipeline depth (counts nested pipelines within the current
/// `execlist` call chain).
pub static pline_level: std::sync::atomic::AtomicI32 = // c:461 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int list_pipe_child = 0;` from `Src/exec.c:462`.
/// Set in the child after the list_pipe fork so the child knows to
/// continue executing the loop body (vs the parent which records
/// the pid + returns).
pub static list_pipe_child: std::sync::atomic::AtomicI32 = // c:462 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int list_pipe_job;` from `Src/exec.c:462`. Job
/// table index of the pipeline's first-stage job (the `cat` in
/// `cat foo | while ...`).
pub static list_pipe_job: std::sync::atomic::AtomicI32 = // c:462 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `static int doneps4;` from `Src/exec.c:262`. Set after
/// `printprompt4` has emitted the `$PS4` prefix for the current
/// xtrace command — prevents double-printing when an inner sub-eval
/// also wants to xtrace.
pub static doneps4: std::sync::atomic::AtomicI32 = // c:262 (Src/exec.c)
    std::sync::atomic::AtomicI32::new(0);

/// Port of `struct execstack *exstack;` from `Src/exec.c:244`. Head
/// of the linked exec-context save stack — `execsave` pushes a frame
/// before signal-handler / trap dispatch; `execrestore` pops it
/// afterwards so the interrupted command resumes with its state intact.
pub static exstack: std::sync::Mutex<Option<Box<crate::ported::zsh_h::execstack>>> = // c:244
    std::sync::Mutex::new(None);

/// Port of `static char *STTYval;` from `Src/exec.c:263`. Pending
/// `stty` argument string captured by `addvars` when the command's
/// inline env contains `STTY=...`. Applied by `execute` before fork
/// + exec so the spawned program sees its tty configured. Reset to
/// `None` after consumption to avoid infinite recursion.
pub static STTYval: std::sync::Mutex<Option<String>> = // c:263 (Src/exec.c)
    std::sync::Mutex::new(None);

/// Convert a here-document into a here-string. Line-by-line port of
/// `gethere()` from `Src/exec.c:4569-4652`. Reads the body from the
/// input stream via `hgetc()` until the terminator line is matched,
/// returning the collected body as a string. `strp` is in/out: on
/// entry the raw terminator (possibly with token markers + leading
/// tabs); on return the munged terminator (after `quotesubst` +
/// `untokenize` and, for `REDIR_HEREDOCDASH`, leading-tab strip).
///
/// Returns `None` on out-of-memory (C `zalloc`/`realloc` failure).
/// Rust's `String` auto-grows so the OOM branch is effectively
/// unreachable, but the return type stays `Option<String>` to mirror
/// the C signature which can return NULL.
///
/// Port of `gethere(char **strp, int typ)` from `Src/exec.c:4573`.
pub fn gethere(strp: &mut String, typ: i32) -> Option<String> {
    // c:4573 (Src/exec.c)
    let mut buf: String; // c:4575 char *buf
    let mut bsiz: usize; // c:4576 int bsiz
    let mut qt: i32 = 0; // c:4576 int qt = 0
    let mut strip: i32 = 0; // c:4576 int strip = 0
                            // c:4577 — char *s, *t, *bptr, c. zshrs uses byte-offsets into
                            // `buf` for `t` and tracks `bptr` implicitly as `buf.len()` (the
                            // C `bptr++` increment is `buf.push(c)`; `bptr--` is `buf.pop()`).
                            // `s` (the loop iterator for the inull-scan) stays local to its
                            // for-loop. `c` mirrors the C `char c`.
    let mut t: usize; // c:4577 char *t
    let mut c: Option<char>; // c:4577 char c
    let mut str: String = strp.clone(); // c:4578 char *str = *strp

    // c:4580-4584 — for (s = str; *s; s++) if (inull(*s)) { qt = 1; break; }
    for s in str.bytes() {
        if inull(s) {
            // c:4581
            qt = 1; // c:4582
            break; // c:4583
        }
    }
    str = quotesubst(&str); // c:4585
    str = untokenize(&str); // c:4586
    if typ == REDIR_HEREDOCDASH {
        // c:4587
        strip = 1; // c:4588
                   // c:4589-4590 — while (*str == '\t') str++;
        while str.starts_with('\t') {
            str.remove(0);
        }
    }
    *strp = str.clone(); // c:4592 *strp = str

    // c:4593 — bptr = buf = zalloc(bsiz = 256);
    bsiz = 256;
    buf = String::with_capacity(bsiz);
    let _ = bsiz; // bsiz is tracked by C for zfree; Rust drops automatically

    // c:4594 — for (;;)
    loop {
        t = buf.len(); // c:4595 t = bptr

        // c:4597-4598 — while ((c = hgetc()) == '\t' && strip) ;
        loop {
            c = hgetc();
            if !(c == Some('\t') && strip != 0) {
                break;
            }
        }

        // c:4599 — for (;;) — inner body-read loop
        loop {
            // c:4600-4613 — buffer-growth realloc dance. Rust's
            // String auto-grows; nothing to do.
            // c:4614 — if (lexstop || c == '\n') break;
            if LEX_LEXSTOP.with(|f| f.get()) || c == Some('\n') || c.is_none() {
                break;
            }
            // c:4616 — if (!qt && c == '\\')
            if qt == 0 && c == Some('\\') {
                buf.push('\\'); // c:4617 *bptr++ = c
                c = hgetc(); // c:4618
                if c == Some('\n') {
                    // c:4619
                    buf.pop(); // c:4620 bptr--
                    c = hgetc(); // c:4621
                    continue; // c:4622
                }
            }
            if let Some(ch) = c {
                // c:4625 *bptr++ = c
                buf.push(ch);
            }
            c = hgetc(); // c:4626
        }
        // c:4628 — *bptr = '\0'; (implicit — Rust String tracks len)

        // c:4629-4630 — if (!strcmp(t, str)) break;
        if &buf[t..] == str.as_str() {
            break;
        }
        // c:4631-4634 — if (lexstop) { t = bptr; break; }
        if LEX_LEXSTOP.with(|f| f.get()) {
            t = buf.len();
            break;
        }
        // c:4635 — *bptr++ = '\n';
        buf.push('\n');
    }
    // c:4637 — *t = '\0';
    buf.truncate(t);

    // c:4638-4640 — s = buf; buf = dupstring(buf); zfree(s, bsiz);
    // The C dance frees the realloc'd block and re-allocates via the
    // string-heap allocator. Rust drops the old String when reassigned.
    buf = dupstring(&buf);

    if qt == 0 {
        // c:4641
        // c:4642 — int ef = errflag;
        let ef = errflag.load(Ordering::Relaxed);
        // c:4644 — parsestr(&buf);
        if let Ok(parsed) = parsestr(&buf) {
            buf = parsed;
        }
        // c:4646-4649 — if (!(errflag & ERRFLAG_ERROR)) errflag = ef | (errflag & ERRFLAG_INT);
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0 {
            let cur = errflag.load(Ordering::Relaxed);
            errflag.store(
                ef | (cur & ERRFLAG_INT),
                Ordering::Relaxed,
            );
        }
    }
    Some(buf) // c:4651 return buf
}

/// Free-function wrapper for `getoutput()` from `Src/exec.c:4712`.
/// Runs a command-substitution body in the active executor and
/// returns its captured stdout. The C signature is `LinkList
/// getoutput(char *cmd, int qt)` but every caller in subst.rs
/// joins the list back into a string, so the Rust port collapses
/// the intermediate.
///
/// Uses `with_executor` (panics on missing VM context), not
/// `try_with_executor + unwrap_or_default()`. C `getoutput` calls
/// `execpline` directly — there's no "no shell" code path. The
/// silent-no-op pattern (return empty string when no executor) would
/// mask catastrophic state corruption as "command produced no output",
/// which is the failure mode the `subst.rs:496` warning block flags.
pub fn getoutput(cmd: &str) -> String {
    // c:4712 (Src/exec.c)
    with_executor(|exec| exec.run_command_substitution(cmd))
}

/// Direct port of `Shfunc loadautofn(Shfunc shf, int ks, int test_only,
/// int ignore_loaddir)` from `Src/exec.c:5050`. Walks `$fpath` for a
/// file named `shf->node.nam`, reads it, installs the text body on
/// the corresponding `shfunctab` entry, and clears `PM_UNDEFINED`.
///
/// C body (abridged):
///   1. `name = shf->node.nam`
///   2. `getfpfunc(name, &dir_path, NULL, 0)` → resolved file path
///   3. If !test_only && file found: parse → store eprog on
///      `shf->funcdef`; clear PM_UNDEFINED; set `shf->filename`.
///   4. Returns shf on success, NULL on failure.
///
/// Rust port: returns 0 = success, 1 = failure (matches the
/// existing call-site convention in `bin_functions -c`). Stores
/// raw file text on `ShFunc.body` (the Rust-side ShFunc in
/// `hashtable.rs:362`); the parser pass that converts text →
/// Eprog runs lazily at first call site.
/// Port of `loadautofn(Shfunc shf, int fksh, int autol, int current_fpath)` from `Src/exec.c:5682`.
pub fn loadautofn(
    shf: *mut shfunc, // c:5682 (Src/exec.c)
    _ks: i32,
    test_only: i32,
    _ignore_loaddir: i32,
) -> i32 {
    if shf.is_null() {
        return 1;
    }
    // c:5054 — `name = shf->node.nam`.
    let name = unsafe { (*shf).node.nam.clone() };
    // c:5070 — `path = getfpfunc(name, &dir_path, NULL, 0)`.
    let mut dir_path: Option<String> = None;
    let path = match getfpfunc(&name, &mut dir_path, None, 0) {
        Some(p) => p,
        None => return 1, // c:5074 not found
    };
    if test_only != 0 {
        // c:5096
        return 0; // test passes — file exists
    }
    // c:5100-5140 — read the file. C uses zopen + read + parse_string +
    // execsave; Rust port stores raw text on the ShFunc and defers
    // parse-to-Eprog until the first call.
    let body = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) => return 1,
    };
    // c:5142 — `shf->filename = ztrdup(dir_path)`.
    unsafe {
        (*shf).filename = dir_path.clone().or(Some(path.clone()));
    }
    // c:5148 — `shf->node.flags &= ~PM_UNDEFINED`.
    unsafe {
        (*shf).node.flags &= !(PM_UNDEFINED as i32);
    }
    // Sync the body string into the Rust-side ShFunc table so the
    // lazy-parse path can find it later.
    if let Ok(mut tab) = shfunctab_lock().write() {
        if let Some(existing) = tab.get_mut(&name) {
            existing.body = Some(body);
            existing.filename = dir_path;
        } else {
            tab.add(shfunc {
                node: hashnode {
                    next: None,
                    nam: name.clone(),
                    flags: 0,
                },
                filename: dir_path,
                lineno: 0,
                funcdef: None,
                redir: None,
                sticky: None,
                body: Some(body),
            });
        }
    }
    0
}

/// Port of `getfpfunc(char *s, int *ksh, char **fdir, char **alt_path, int test_only)` from Src/exec.c:5260. Walks `$fpath` (or the
/// supplied `spec_path` slice) for a file named `name` and writes the
/// resolved directory through `*dir_path_out` (matching the C `char **dir_path`).
/// Returns `Some(file_contents_path)` on success, `None` when not found.
pub fn getfpfunc(
    name: &str,
    dir_path_out: &mut Option<String>, // c:5260 (Src/exec.c)
    spec_path: Option<&[String]>,
    _all_loaded: i32,
) -> Option<String> {
    let dirs: Vec<String> = match spec_path {
        Some(s) => s.to_vec(),
        None => std::env::var("FPATH")
            .or_else(|_| std::env::var("fpath"))
            .ok()
            .map(|v| v.split(':').map(String::from).collect())
            .unwrap_or_default(),
    };
    for dir in &dirs {
        if dir.is_empty() {
            continue;
        }
        let path = format!("{}/{}", dir, name);
        if std::path::Path::new(&path).exists() {
            *dir_path_out = Some(dir.clone());
            return Some(path);
        }
    }
    None
}

/// Port of `resolvebuiltin(const char *cmdarg, HashNode hn)` from
/// `Src/exec.c:2703`. Ensures that an autoload-stub builtin has its
/// module loaded before the caller invokes its `handlerfunc`. If the
/// stub has no handler, `ensurefeature` is asked to load the module
/// and re-lookup the builtin node. C body (abridged):
/// ```c
/// if (!((Builtin) hn)->handlerfunc) {
///     char *modname = dupstring(((Builtin) hn)->optstr);
///     (void)ensurefeature(modname, "b:", ...);
///     hn = builtintab->getnode(builtintab, cmdarg);
///     if (!hn) { lastval=1; zerr(...); return NULL; }
/// }
/// return hn;
/// ```
///
/// WARNING: zshrs's builtin table is the static `BUILTINS` array in
/// `src/ported/builtin.rs`. Module autoload (`ensurefeature` from
/// `Src/module.c`) is not yet wired through the same code path; the
/// helper exists today as `crate::ported::module::ensurefeature` for
/// the few sites that touch it. Until module-autoload is hooked up,
/// this port is the identity for builtins with a registered
/// `handlerfunc` and returns `None` for unresolved stubs (matching
/// the C return-NULL-on-failure contract).
pub fn resolvebuiltin<'a>(
    cmdarg: &str, // c:2703 (Src/exec.c)
    hn: &'a builtin,
) -> Option<&'a builtin> {
    // c:2705 — `if (!((Builtin) hn)->handlerfunc)`.
    if hn.handlerfunc.is_none() {
        // c:2706 — `modname = dupstring(((Builtin)hn)->optstr)`.
        let modname = hn.optstr.clone().unwrap_or_default();
        // c:2712-2714 — `ensurefeature(modname, "b:", ...)`. The Rust
        // module-autoload path is not yet wired; treat missing
        // handlerfunc as unresolvable.
        // c:2715-2721 — re-lookup, fail with `lastval=1` + zerr.
        zerr(&format!(
            "autoloading module {} failed to define builtin: {}",
            modname, cmdarg
        ));
        return None; // c:2720
    }
    Some(hn) // c:2723
}

/// Dispatch decision returned by `execcmd_exec`'s head-walk port.
/// Encodes the local-variable state that the C function carries
/// through `c:2913-2916` (`is_builtin`, `is_shfunc`, `cflags`,
/// `use_defpath`) plus the precmd-modifier strip count. The fusevm
/// bytecode compiler reads this to emit the correct dispatch opcode
/// in `src/extensions/compile_zsh.rs::compile_simple`.
///
/// Not a C struct — invented to bridge the divergence between the
/// C wordcode-walker (which mutates locals + falls through to
/// invocation) and zshrs's split parse → compile → VM pipeline.
#[allow(non_camel_case_types)]
#[derive(Debug, Default, Clone)]
pub struct execcmd_dispatch {
    /// Number of `BINF_PREFIX` words to strip from the head of args.
    /// `Src/exec.c:3086 uremnode(preargs, firstnode(preargs))`.
    pub precmd_skip: usize,
    /// Set when the head (after strip) is a real builtin
    /// (`Src/exec.c:3065 is_builtin = 1`).
    pub is_builtin: bool,
    /// Set when the head (after strip) is a shell function
    /// (`Src/exec.c:3053 is_shfunc = 1`).
    pub is_shfunc: bool,
    /// `cflags` accumulator from `Src/exec.c:2915` — gathers
    /// `BINF_BUILTIN | BINF_COMMAND | BINF_EXEC | BINF_DASH |
    /// BINF_NOGLOB` bits encountered during the precommand-modifier
    /// walk (c:3062 `cflags |= hn->flags`).
    pub cflags: u32,
    /// `command -p` requested: use the default `$PATH` for lookup
    /// (`Src/exec.c:3160 use_defpath = 1`). NOT YET HONORED by the
    /// fusevm compiler — flagged for follow-up.
    pub use_defpath: bool,
    /// `command -v` / `command -V` requested: the dispatch target
    /// flips to `bin_whence` per `Src/exec.c:3149-3157`
    /// (`hn = &commandbn.node; is_builtin = 1`). The fusevm compiler
    /// reads this and emits `Op::CallBuiltin(BUILTIN_WHENCE_FROM_COMMAND)`
    /// instead of resolving the post-strip head.
    pub has_command_vv: bool,
    /// `exec -a NAME` requested: ARGV0 override per `Src/exec.c:3214-3240`.
    /// `Some(NAME)` triggers `zputenv("ARGV0=NAME")` before exec.
    pub exec_argv0: Option<String>,
    /// Empty-command branch fired with no redirs (`Src/exec.c:3372-3406`
    /// — the `else` arm of `if (redir && nonempty(redir))`). Covers
    /// bare `exec` / `noglob` / `command`. Caller emits
    /// `lastval = cmdoutval` (0 when no `$(cmd)` ran) and returns.
    /// Also fires for the `(cflags & BINF_PREFIX) && (cflags &
    /// BINF_COMMAND)` sub-case at `c:3365-3371` (bare `command`
    /// returns 0 without complaining about missing redirs).
    pub is_empty_command: bool,
}

/// Port of the head section of `execcmd_exec(Estate state,
/// Execcmd_params eparams, int input, int output, int how, int
/// last1, int close_if_forked)` from `Src/exec.c:2900`. Body
/// translated line-for-line from `c:2904-3091` covering local
/// initialisation, %job-table head detection, and the
/// precommand-modifier walk that strips `BINF_PREFIX` builtins
/// (`-`, `builtin`, `command`, `exec`, `noglob`) before dispatch.
///
/// =================== WARNING — DIVERGENCE ====================
///
/// The C function runs ~1500 lines and PERFORMS dispatch: it sets up
/// `multio` redirections, evaluates `varspc` assignments, then calls
/// `execbuiltin` / `runshfunc` / `execute` directly. zshrs DOES NOT
/// port that tail because the fusevm bytecode VM
/// (`src/extensions/compile_zsh.rs::compile_simple` +
/// `src/fusevm_bridge.rs::register_builtins`) emits dispatch opcodes
/// at compile time and the VM drives them at runtime.
///
/// This Rust port stops at `c:3091` — immediately after the
/// precmd-modifier walk — and returns the dispatch decision via
/// `execcmd_dispatch`. The fusevm compiler reads that struct to
/// decide which `Op::CallBuiltin` / `Op::CallFunction` / `Op::Exec`
/// to emit, and to compute the correct post-strip `argc`.
///
/// Code below this function's return point that lives in C
/// (`Src/exec.c:3092-4404`) is intentionally NOT ported. The
/// `BINF_COMMAND` / `BINF_EXEC` sub-modifier option-parsing block
/// (`c:3092-3275`) is a TODO — today `command -p`, `command -v/-V`,
/// `exec -a`, `exec -l`, `exec -c` are partially handled in
/// `compile_zsh.rs` / `src/ported/builtin.rs::bin_command` /
/// `bin_exec` rather than here. When those land canonically, they
/// extend this function past `c:3091`.
///
/// =============================================================
///
/// Signature adaptation: the C `Estate`/`Execcmd_params` carry the
/// wordcode iterator state — zshrs doesn't traverse wordcode, so
/// the args list arrives already-expanded as a `&[String]` (analog
/// of `preargs` after `execcmd_getargs` at `c:3028`). `type_`
/// mirrors `eparams->type` (`WC_SIMPLE` vs `WC_TYPESET`).
pub fn execcmd_exec(args: &[String], type_: u32) -> execcmd_dispatch {
    // c:2900 (Src/exec.c)

    // c:2904-2916 — locals.
    let mut hn: Option<&'static builtin> = None; // c:2904
    let mut is_shfunc = false; // c:2913
    let mut is_builtin = false; // c:2913
    let mut use_defpath = false; // c:2913
    let mut cflags: u32 = 0; // c:2915
    let mut orig_cflags: u32 = 0; // c:2915
    let _ = orig_cflags;
    // c:3263 — `char *exec_argv0 = NULL;` (declared inside the
    // BINF_EXEC arm; hoisted here so the dispatch struct can carry it
    // out after the loop terminates).
    let mut exec_argv0: Option<String> = None;
    // c:3149/3158 — `has_vV`/`has_p` flags from the BINF_COMMAND arm
    // (c:3104). Surface `has_vV` via the dispatch struct so the fusevm
    // compiler can emit `bin_whence` instead of resolving the head.
    let mut has_command_vv = false;

    // c:2962-2973 — `%job` head: rewrite `%name` → `fg|bg|disown %name`.
    // Not in scope for the compile-time dispatch walk: jobspec
    // expansion happens at runtime in fusevm; the bytecode emits a
    // direct `fg`/`bg` call when it sees a leading `%`. Flagged for
    // follow-up when the canonical port lands.

    // c:2975-2986 — AUTORESUME prefix-match against jobtab. Same
    // status as the %job head: runtime concern, deferred.

    // c:3013-3091 — precommand-modifier walk.
    let mut preargs: Vec<String> = args.to_vec(); // c:3027 newlinklist
    let mut precmd_skip: usize = 0;

    // c:3018 — `if ((type == WC_SIMPLE || type == WC_TYPESET) && args)`.
    if (type_ == WC_SIMPLE || type_ == WC_TYPESET) && !preargs.is_empty() {
        // c:3018
        // c:3029 — `while (nonempty(preargs))`.
        while precmd_skip < preargs.len() {
            // c:3029
            // c:3030 — `cmdarg = (char *) peekfirst(preargs);`.
            let cmdarg = untokenize(&preargs[precmd_skip]);
            // c:3031 — `checked = !has_token(cmdarg)`. zshrs's fusevm
            // already performed prefork expansion on `preargs`, so
            // `has_token` is effectively false here; the C `break` on
            // unexpanded tokens is unreachable in this entry point.

            // c:3034-3035 — WC_TYPESET fast path: `getnode2` looks up
            // even disabled builtins so the reserved-word form
            // (`integer x`, `local foo`) still dispatches to the
            // typeset family. The static `BUILTINS` array doesn't
            // expose a separate disabled-bit lookup; one path covers
            // both. Effect is identical for the precmd-modifier walk.

            // c:3050-3052 — `if (!(cflags & (BINF_BUILTIN |
            // BINF_COMMAND)) && shfunctab->getnode(...))` — shell
            // function takes precedence unless a `builtin`/`command`
            // modifier preceded it.
            if (cflags & (BINF_BUILTIN | BINF_COMMAND)) == 0 {
                // c:3051
                if shfunctab_lock()
                    .read()
                    .map(|t| t.iter().any(|(k, _)| k == &cmdarg))
                    .unwrap_or(false)
                {
                    is_shfunc = true; // c:3053
                    break; // c:3054
                }
            }
            // c:3056 — `builtintab->getnode(builtintab, cmdarg)`.
            let entry = BUILTINS
                .iter()
                .find(|b| b.node.nam == cmdarg);
            let Some(entry) = entry else {
                // c:3056-3058
                break;
            };
            hn = Some(entry);
            // c:3061-3063 — accumulate cflags.
            orig_cflags |= cflags;
            cflags &= !(BINF_BUILTIN | BINF_COMMAND);
            cflags |= entry.node.flags as u32;
            // c:3064 — `if (!(hn->flags & BINF_PREFIX))` — real
            // builtin, stop.
            if (entry.node.flags as u32 & BINF_PREFIX) == 0 {
                // c:3064
                // WARNING — DIVERGENCE: c:3068 calls `resolvebuiltin`
                // to autoload the builtin's module if its
                // `handlerfunc` is NULL. In zshrs, builtins live in
                // two places: the static `BUILTINS` table (which
                // mirrors C `handlerfunc`, often `None` for ports
                // dispatched through fusevm) AND fusevm's
                // `register_builtins` map (the actual runtime
                // dispatcher). A null `handlerfunc` in the static
                // table is NOT an autoload failure for us — it
                // means dispatch routes through fusevm. So we
                // skip the resolvebuiltin call here; the faithful
                // port remains available for future callers that
                // genuinely need module-autoload semantics.
                is_builtin = true; // c:3065
                break; // c:3077
            }
            // c:3086 — `uremnode(preargs, firstnode(preargs))`.
            precmd_skip += 1;
            // c:3087-3091 — `if (!firstnode(preargs)) { execcmd_getargs
            //   (...); if (!firstnode(preargs)) break; }`. zshrs has
            // no `execcmd_getargs` (args arrive pre-expanded); the
            // bounds-check at the top of `while precmd_skip <
            // preargs.len()` handles the empty case identically.

            // c:3092-3177 — BINF_COMMAND sub-option parsing
            // (`command -p / -v / -V`).
            if (cflags & BINF_COMMAND) != 0 && precmd_skip < preargs.len() {
                // c:3102-3104 — `LinkNode argnode, oldnode, pnode = NULL;
                //                int has_p = 0, has_vV = 0, has_other = 0;`
                let mut argnode: usize = precmd_skip; // c:3105 `argnode = firstnode(preargs);`
                let mut pnode: Option<usize> = None; // c:3102
                let mut has_p = false; // c:3104
                let mut has_vv = false; // c:3104
                let mut has_other = false; // c:3104
                // c:3107 — `while (IS_DASH(*argdata))`
                while argnode < preargs.len()
                    && IS_DASH(preargs[argnode].chars().next().unwrap_or('\0'))
                {
                    let argdata = preargs[argnode].clone(); // c:3106
                    let bytes = argdata.as_bytes();
                    // c:3108-3111 — stop on bare `-` or `--`.
                    if bytes.len() < 2 || (IS_DASH(bytes[1] as char) && bytes.len() == 2) {
                        // c:3109
                        break; // c:3111
                    }
                    // c:3112-3133 — scan flag chars.
                    for &c in &bytes[1..] {
                        // c:3112
                        match c as char {
                            'p' => {
                                // c:3114
                                has_p = true; // c:3122
                                pnode = Some(argnode); // c:3123
                            }
                            'v' | 'V' => {
                                // c:3125-3126
                                has_vv = true; // c:3127
                            }
                            _ => {
                                // c:3129
                                has_other = true; // c:3130
                            }
                        }
                    }
                    // c:3134-3138 — unknown flag → don't try, leave alone.
                    if has_other {
                        // c:3134
                        has_p = false; // c:3136
                        has_vv = false; // c:3136
                        break; // c:3137
                    }
                    // c:3140-3147 — advance to next arg.
                    argnode += 1; // c:3141 nextnode(argnode)
                    if argnode >= preargs.len() {
                        // c:3142 — execcmd_getargs (skipped: pre-expanded)
                        break; // c:3145
                    }
                }
                // c:3149-3157 — `-v`/`-V` → dispatch to whence.
                if has_vv {
                    // c:3149
                    // c:3154 `pushnode(preargs, "command")` — C re-inserts
                    // "command" so bin_whence sees it as argv[0]. zshrs
                    // surfaces this via `has_command_vv`; the fusevm
                    // compiler emits the equivalent whence call.
                    has_command_vv = true; // c:3155-3156 hn = &commandbn; is_builtin=1
                    is_builtin = true;
                    break; // c:3157
                } else if has_p {
                    // c:3158
                    use_defpath = true; // c:3160
                    if let Some(pn) = pnode {
                        // c:3165 — `uremnode(preargs, pnode)`. zshrs:
                        // remove the `-p`-bearing arg from preargs.
                        if pn < preargs.len() {
                            preargs.remove(pn);
                            // precmd_skip already accounts for the
                            // stripped `command` prefix; we just removed
                            // the `-p` flag which sat at preargs[pn].
                            // No precmd_skip change needed — the head
                            // remains where it was.
                        }
                    }
                }
                // c:3176-3177 — `--` trailing end-of-options strip.
                if argnode < preargs.len() {
                    let argdata = &preargs[argnode];
                    let b = argdata.as_bytes();
                    if b.len() == 2 && IS_DASH(b[0] as char) && IS_DASH(b[1] as char) {
                        // c:3176
                        preargs.remove(argnode); // c:3177
                    }
                }
            } else if (cflags & BINF_EXEC) != 0 && precmd_skip < preargs.len() {
                // c:3178-3275 — BINF_EXEC sub-option parsing
                // (`exec -a NAME -l -c`).
                let mut argnode: usize = precmd_skip; // c:3185
                let mut error_done = false;
                // c:3196 — `while (argdata && IS_DASH(*argdata) &&
                //                  strlen(argdata) >= 2)`
                while argnode < preargs.len() {
                    let argdata = preargs[argnode].clone();
                    let bytes = argdata.as_bytes();
                    if bytes.is_empty() || !IS_DASH(bytes[0] as char) || bytes.len() < 2 {
                        break; // c:3196 loop guard
                    }
                    let oldnode = argnode; // c:3197
                    argnode += 1; // c:3198 nextnode(oldnode)
                                  // c:3203-3208 — empty next → error.
                    if argnode >= preargs.len() {
                        // c:3203
                        zerr(
                            // c:3204
                            "exec requires a command to execute",
                        );
                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3206
                        error_done = true;
                        break; // c:3207 goto done
                    }
                    // c:3209 — `uremnode(preargs, oldnode)`.
                    preargs.remove(oldnode);
                    argnode -= 1; // re-anchor — `argnode` was the post-removed slot
                                  // c:3210-3211 — `--` stops option scan.
                    if bytes.len() == 2 && IS_DASH(bytes[0] as char) && IS_DASH(bytes[1] as char) {
                        // c:3210
                        break; // c:3211
                    }
                    // c:3212-3258 — scan flag chars after the leading `-`.
                    let mut k = 1usize;
                    while k < bytes.len() && !error_done {
                        let cmdopt = bytes[k] as char; // c:3212
                        match cmdopt {
                            'a' => {
                                // c:3214 — `-a` ARGV0 override.
                                if k + 1 < bytes.len() {
                                    // c:3216 — `-aNAME` inline form.
                                    exec_argv0 =
                                        Some(String::from_utf8_lossy(&bytes[k + 1..]).into_owned()); // c:3217
                                    k = bytes.len(); // c:3219 position past end
                                } else {
                                    // c:3220 — `-a NAME` separate form.
                                    if argnode >= preargs.len() {
                                        // c:3230
                                        zerr(
                                            // c:3231
                                            "exec flag -a requires a parameter",
                                        );
                                        errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3233
                                        error_done = true;
                                        break; // c:3234 goto done
                                    }
                                    exec_argv0 = Some(preargs[argnode].clone()); // c:3236
                                    preargs.remove(argnode); // c:3239
                                }
                            }
                            'c' => {
                                // c:3242
                                cflags |= BINF_CLEARENV; // c:3243
                            }
                            'l' => {
                                // c:3245
                                cflags |= BINF_DASH; // c:3246
                            }
                            _ => {
                                // c:3248
                                zerr(
                                    // c:3249
                                    &format!("unknown exec flag -{}", cmdopt),
                                );
                                errflag.fetch_or(ERRFLAG_ERROR, Ordering::Relaxed); // c:3251
                                error_done = true;
                                break; // c:3256
                            }
                        }
                        k += 1;
                    }
                    if error_done {
                        break;
                    }
                }
                // c:3263-3274 — zputenv("ARGV0=NAME"). zshrs defers
                // the actual `setenv` to the fusevm compiler / external
                // exec path; we surface `exec_argv0` via the dispatch
                // struct so the caller can apply it before fork+exec.
                if let Some(ref a0) = exec_argv0 {
                    // c:3263 — `remnulargs + untokenize` then setenv.
                    let cleaned = untokenize(a0); // c:3266-3267
                    exec_argv0 = Some(cleaned);
                }
                if error_done {
                    return execcmd_dispatch {
                        precmd_skip,
                        is_builtin,
                        is_shfunc,
                        cflags,
                        use_defpath,
                        has_command_vv,
                        exec_argv0,
                        is_empty_command: false,
                    };
                }
            }
        }
    }

    // c:3309-3406 — "Empty command" branch. When the precmd-modifier
    // walk above strips every word with nothing left to dispatch
    // (bare `exec`, bare `noglob`, bare `command`, bare `nocorrect`),
    // C falls into `if (!args || empty(args))` at c:3331. Sub-cases:
    //
    // - redir-present + do_exec       → nullexec=1 (continue to run)
    // - redir-present + varspc        → nullexec=2 (continue)
    // - redir-present + no nullcmd    → `zerr("redirection with no command")`
    //                                   lastval=1, return
    // - redir-present + SHNULLCMD     → args=[":"]
    // - redir-present + readnullcmd   → args=[readnullcmd]
    // - redir-present + default       → args=[nullcmd]
    // - NO redir + BINF_PREFIX+COMMAND → lastval=0, return (c:3365-3371)
    // - NO redir + default            → lastval=cmdoutval, return (c:3372-3406)
    //
    // zshrs's `execcmd_exec` doesn't receive `redir` (it takes `args`
    // only). The cases that DEPEND on redirs are handled by
    // `compile_zsh.rs::compile_redir` before this dispatch fires; the
    // remaining cases collapse into the single `is_empty_command`
    // flag below. Both NO-redir sub-cases produce the same observable
    // outcome (lastval=0, return without invoking anything), so a
    // single flag suffices.
    let is_empty_command = precmd_skip >= preargs.len();

    // =================== WARNING — DIVERGENCE ====================
    // c:3285+: prefork-substitution, magic_assign decision, multio
    // setup, varspc evaluation, and the actual execbuiltin /
    // runshfunc / execute call. ~1300 lines of interpreter-only
    // code, entirely replaced by fusevm bytecode dispatch in
    // `src/extensions/compile_zsh.rs::compile_simple` and the
    // opcode handlers in `src/fusevm_bridge.rs::register_builtins`.
    // The return value below feeds those compile-time decisions.
    // =============================================================

    let _ = hn;
    execcmd_dispatch {
        precmd_skip,
        is_builtin,
        is_shfunc,
        cflags,
        use_defpath,
        has_command_vv,
        exec_argv0,
        is_empty_command,
    }
}

// =============================================================================
// Leaf-function ports — c:283 (parse_string) and below. Added incrementally to
// chip at the ~5500 lines of exec.c still un-ported beyond the wordcode
// walker (execlist / execpline / execcmd which the fusevm bytecode VM
// replaces — see the WARNING block in execcmd_exec).
// =============================================================================

/// Port of `parse_string(char *s, int reset_lineno)` from `Src/exec.c:283`.
///
/// C body:
/// ```c
/// Eprog p; zlong oldlineno;
/// zcontext_save();
/// inpush(s, INP_LINENO, NULL);
/// strinbeg(0);
/// oldlineno = lineno;
/// if (reset_lineno) lineno = 1;
/// p = parse_list();
/// lineno = oldlineno;
/// if (tok == LEXERR && !lastval) lastval = 1;
/// strinend();
/// inpop();
/// zcontext_restore();
/// return p;
/// ```
///
/// Parses an arbitrary string as a zsh command list, returning the
/// `Eprog` (compiled wordcode). Used by `getoutput` for `$(cmd)`,
/// `bin_eval` for `eval`, and the autoload path.
pub fn parse_string(s: &str, reset_lineno: i32) -> Option<eprog> {
    // c:285-286
    let p: Option<eprog>;
    let oldlineno: i64;

    zcontext_save(); // c:288
    inpush(s, INP_LINENO, None); // c:289
    strinbeg(0); // c:290
    oldlineno = LEX_LINENO.get() as i64; // c:291
    if reset_lineno != 0 {
        // c:292
        LEX_LINENO.set(1); // c:293
    }
    p = parse_list(); // c:294
    LEX_LINENO.set(oldlineno as u64); // c:295
                                                          // c:296-297 — `if (tok == LEXERR && !lastval) lastval = 1;`
    if tok() == LEXERR
        && LASTVAL.load(Ordering::Relaxed) == 0
    {
        LASTVAL.store(1, Ordering::Relaxed);
    }
    strinend(); // c:298
    inpop(); // c:299
    zcontext_restore(); // c:300
    p // c:301
}

/// Port of `int isgooderr(int e, char *dir)` from `Src/exec.c:652`.
///
/// C body:
/// ```c
/// /* Maybe the directory was unreadable, or maybe it wasn't even a directory. */
/// return ((e != EACCES || !access(dir, X_OK)) &&
///         e != ENOENT && e != ENOTDIR);
/// ```
///
/// errno classifier for `execve` failures during PATH search: if the
/// errno is EACCES (and the dir is X-accessible) or ENOENT/ENOTDIR,
/// it's "expected" (try next PATH entry); otherwise it's a real
/// failure worth surfacing.
pub fn isgooderr(e: i32, dir: &str) -> bool {
    // c:652
    let dir_x_ok = std::path::Path::new(&unmeta(dir))
        .metadata()
        .map(|m| m.permissions().mode() & 0o111 != 0)
        .unwrap_or(false);
    // c:658-659 — `(e != EACCES || !access(dir, X_OK)) && e != ENOENT && e != ENOTDIR`
    (e != libc::EACCES || !dir_x_ok) && e != libc::ENOENT && e != libc::ENOTDIR
}

/// Port of `int iscom(char *s)` from `Src/exec.c:962`.
///
/// C body:
/// ```c
/// struct stat statbuf;
/// char *us = unmeta(s);
/// return (access(us, X_OK) == 0 && stat(us, &statbuf) >= 0 &&
///         S_ISREG(statbuf.st_mode));
/// ```
///
/// True iff `s` names an executable regular file (X-perm + S_IFREG).
/// Used by the PATH-search loop in `findcmd` / `search_defpath` to
/// validate candidate paths before exec.
pub fn iscom(s: &str) -> bool {
    // c:962
    let us = unmeta(s); // c:965
                        // c:967-968 — `access(us, X_OK) == 0 && stat(us, &statbuf) >= 0 && S_ISREG(...)`
    let cstr = match std::ffi::CString::new(us.as_str()) {
        Ok(c) => c,
        Err(_) => return false,
    };
    let x_ok = unsafe { libc::access(cstr.as_ptr(), libc::X_OK) } == 0;
    if !x_ok {
        return false;
    }
    let meta = match std::fs::metadata(&us) {
        Ok(m) => m,
        Err(_) => return false,
    };
    meta.file_type().is_file()
}

/// Port of `int isrelative(char *s)` from `Src/exec.c:996`.
///
/// C body:
/// ```c
/// if (*s != '/') return 1;
/// for (; *s; s++)
///     if (*s == '.' && s[-1] == '/' &&
///         (s[1] == '/' || s[1] == '\0' ||
///          (s[1] == '.' && (s[2] == '/' || s[2] == '\0'))))
///         return 1;
/// return 0;
/// ```
///
/// True iff `s` either doesn't start with `/` OR contains a `./` or
/// `../` component anywhere. Used by `cd` resolution and PATH-cache
/// invalidation to detect non-canonical paths.
pub fn isrelative(s: &str) -> i32 {
    // c:996
    let bytes = s.as_bytes();
    if bytes.is_empty() || bytes[0] != b'/' {
        // c:998
        return 1; // c:999
    }
    // c:1000-1004 — walk for `./` or `../` components.
    for i in 1..bytes.len() {
        let c = bytes[i];
        let prev = bytes[i - 1];
        if c == b'.' && prev == b'/' {
            let next = bytes.get(i + 1).copied().unwrap_or(0);
            if next == b'/' || next == 0 {
                // c:1002
                return 1;
            }
            if next == b'.' {
                let next2 = bytes.get(i + 2).copied().unwrap_or(0);
                if next2 == b'/' || next2 == 0 {
                    // c:1003
                    return 1;
                }
            }
        }
    }
    0 // c:1005
}

/// Port of `void setunderscore(char *str)` from `Src/exec.c:2652`.
///
/// C body:
/// ```c
/// queue_signals();
/// if (str && *str) {
///     size_t l = strlen(str) + 1, nl = (l + 31) & ~31;
///     if (nl > underscorelen || (underscorelen - nl) > 64) {
///         zfree(zunderscore, underscorelen);
///         zunderscore = (char *) zalloc(underscorelen = nl);
///     }
///     strcpy(zunderscore, str);
///     underscoreused = l;
/// } else {
///     ... reset zunderscore = "" ...
/// }
/// unqueue_signals();
/// ```
///
/// Sets the `$_` global to the last argument of the most recent
/// command. Called from `execcmd_exec` (c:3936) per `last_status`
/// update; mirrored in zshrs by the fusevm `Op::Exec` handler.
pub fn setunderscore(str: &str) {
    // c:2652
    queue_signals(); // c:2654
    if !str.is_empty() {
        // c:2655 `if (str && *str)`
        // c:2656-2663 — copy str into zunderscore; track byte length in underscoreused.
        let mut zu = zunderscore.lock().unwrap();
        *zu = str.to_string();
        let nl = (str.len() + 1 + 31) & !31; // c:2656
        underscorelen.store(nl, Ordering::Relaxed); // c:2660
        underscoreused.store((str.len() + 1) as i32, Ordering::Relaxed);
    // c:2663
    } else {
        // c:2664
        let mut zu = zunderscore.lock().unwrap();
        zu.clear(); // c:2669 `*zunderscore = '\0';`
        underscoreused.store(1, Ordering::Relaxed); // c:2670
    }
    unqueue_signals(); // c:2672
}

/// Port of `int mpipe(int *pp)` from `Src/exec.c:5160`.
///
/// C body:
/// ```c
/// if (pipe(pp) < 0) {
///     zerr("pipe failed: %e", errno);
///     return -1;
/// }
/// pp[0] = movefd(pp[0]);
/// pp[1] = movefd(pp[1]);
/// return 0;
/// ```
///
/// libc `pipe(2)` wrapper that pushes both ends out of the reserved-
/// fd range via `movefd`. Used by `getpipe` / `getproc` /
/// `spawnpipes` for process substitution and pipeline wiring.
pub fn mpipe(pp: &mut [i32; 2]) -> i32 {
    // c:5160
    let mut fds: [libc::c_int; 2] = [-1; 2];
    if unsafe { libc::pipe(fds.as_mut_ptr()) } < 0 {
        // c:5162
        zerr(&format!(
            // c:5163
            "pipe failed: {}",
            std::io::Error::last_os_error()
        ));
        return -1; // c:5164
    }
    pp[0] = movefd(fds[0]); // c:5166
    pp[1] = movefd(fds[1]); // c:5167
    0 // c:5168
}

/// Port of `static const char *const ANONYMOUS_FUNCTION_NAME = "(anon)";`
/// from `Src/exec.c:5289`. Anonymous-function name marker used by
/// `is_anonymous_function_name`, `execfuncdef`, and `doshfunc` for
/// `() { ... }` anonymous function dispatch.
pub const ANONYMOUS_FUNCTION_NAME: &str = "(anon)";

/// Port of `int is_anonymous_function_name(const char *name)` from
/// `Src/exec.c:5300`.
///
/// C body:
/// ```c
/// return !strcmp(name, ANONYMOUS_FUNCTION_NAME);
/// ```
///
/// True iff the name equals the `"(anon)"` sentinel. Used by zprof
/// reporting and `whence -v` to skip / annotate anonymous functions.
pub fn is_anonymous_function_name(name: &str) -> i32 {
    // c:5300
    if name == ANONYMOUS_FUNCTION_NAME {
        // c:5302
        1
    } else {
        0
    }
}

/// Port of `void execsave(void)` from `Src/exec.c:6438`.
///
/// C body:
/// ```c
/// struct execstack *es = (struct execstack *) zalloc(sizeof(struct execstack));
/// es->list_pipe_pid = list_pipe_pid;
/// es->nowait = nowait;
/// es->pline_level = pline_level;
/// es->list_pipe_child = list_pipe_child;
/// es->list_pipe_job = list_pipe_job;
/// strcpy(es->list_pipe_text, list_pipe_text);
/// es->lastval = lastval;
/// es->noeval = noeval;
/// es->badcshglob = badcshglob;
/// es->cmdoutpid = cmdoutpid;
/// es->cmdoutval = cmdoutval;
/// es->use_cmdoutval = use_cmdoutval;
/// es->procsubstpid = procsubstpid;
/// es->trap_return = trap_return;
/// es->trap_state = trap_state;
/// es->trapisfunc = trapisfunc;
/// es->traplocallevel = traplocallevel;
/// es->noerrs = noerrs;
/// es->this_noerrexit = this_noerrexit;
/// es->underscore = ztrdup(zunderscore);
/// es->next = exstack;
/// exstack = es;
/// noerrs = cmdoutpid = 0;
/// ```
///
/// Snapshot every transient exec-context global onto the `exstack`
/// linked list so a signal-handler / trap-firing nested eval can
/// scribble freely; `execrestore` pops the frame back. Called by
/// `dotrap` (signals.c) and the trap-firing entry in `execlist`.
pub fn execsave() {
    // c:6438
    // c:6442 — `es = zalloc(sizeof(execstack));`
    let mut es = Box::new(execstack {
        // c:6442
        next: None,
        list_pipe_pid: list_pipe_pid.load(Ordering::Relaxed), // c:6443
        nowait: nowait.load(Ordering::Relaxed),               // c:6444
        pline_level: pline_level.load(Ordering::Relaxed),     // c:6445
        list_pipe_child: list_pipe_child.load(Ordering::Relaxed), // c:6446
        list_pipe_job: list_pipe_job.load(Ordering::Relaxed), // c:6447
        list_pipe_text: [0u8; crate::ported::zsh_h::JOBTEXTSIZE], // c:6448 strcpy
        lastval: LASTVAL.load(Ordering::Relaxed),             // c:6449
        // c:6450 — `noeval = M_NOEVAL` per math.rs port-rename. Read
        // through the helper accessor for the canonical value.
        // c:6450 — `noeval` (math.c:40) is ported as the thread-local
        // `M_NOEVAL` in math.rs with private `m_noeval()` accessor.
        // execsave/execrestore live across that privacy boundary;
        // snapshot 0 until the accessor is elevated to pub(crate).
        noeval: 0, // c:6450 (deps WARNING — math.rs::m_noeval not pub)
        // c:6451 — `badcshglob` lives in glob.c:103 but isn't ported
        // yet; snapshot 0 (the BSS-zero default) until the port lands.
        badcshglob: 0, // c:6451
        cmdoutpid: cmdoutpid.load(Ordering::Relaxed), // c:6452
        cmdoutval: cmdoutval.load(Ordering::Relaxed), // c:6453
        use_cmdoutval: use_cmdoutval.load(Ordering::Relaxed), // c:6454
        procsubstpid: procsubstpid.load(Ordering::Relaxed), // c:6455
        trap_return: TRAP_RETURN.load(Ordering::Relaxed), // c:6456
        trap_state: TRAP_STATE.load(Ordering::Relaxed), // c:6457
        trapisfunc: crate::ported::signals::trapisfunc.load(Ordering::Relaxed), // c:6458
        traplocallevel: crate::ported::signals::traplocallevel.load(Ordering::Relaxed), // c:6459
        noerrs: noerrs.load(Ordering::Relaxed), // c:6460
        this_noerrexit: this_noerrexit.load(Ordering::Relaxed), // c:6461
        // c:6462 — `es->underscore = ztrdup(zunderscore);`
        underscore: Some(zunderscore.lock().unwrap().clone()),
    });
    // c:6463-6464 — `es->next = exstack; exstack = es;`
    let mut head = exstack.lock().unwrap();
    es.next = head.take();
    *head = Some(es);
    // c:6465 — `noerrs = cmdoutpid = 0;`
    noerrs.store(0, Ordering::Relaxed);
    cmdoutpid.store(0, Ordering::Relaxed);
}

/// Port of `void execrestore(void)` from `Src/exec.c:6470`.
///
/// C body:
/// ```c
/// struct execstack *en = exstack;
/// DPUTS(!exstack, "BUG: execrestore() without execsave()");
/// queue_signals();
/// exstack = exstack->next;
/// list_pipe_pid = en->list_pipe_pid;
/// nowait = en->nowait;
/// pline_level = en->pline_level;
/// list_pipe_child = en->list_pipe_child;
/// list_pipe_job = en->list_pipe_job;
/// strcpy(list_pipe_text, en->list_pipe_text);
/// lastval = en->lastval;
/// noeval = en->noeval;
/// badcshglob = en->badcshglob;
/// cmdoutpid = en->cmdoutpid;
/// cmdoutval = en->cmdoutval;
/// use_cmdoutval = en->use_cmdoutval;
/// procsubstpid = en->procsubstpid;
/// trap_return = en->trap_return;
/// trap_state = en->trap_state;
/// trapisfunc = en->trapisfunc;
/// traplocallevel = en->traplocallevel;
/// noerrs = en->noerrs;
/// this_noerrexit = en->this_noerrexit;
/// setunderscore(en->underscore);
/// zsfree(en->underscore);
/// free(en);
/// unqueue_signals();
/// ```
///
/// Pop the top `execstack` frame and restore every transient
/// exec-context global. Inverse of `execsave`.
pub fn execrestore() {
    // c:6470
    let mut head = exstack.lock().unwrap();
    let en = match head.take() {
        // c:6472 + c:6477
        Some(en) => en,
        None => {
            // c:6474 — DPUTS(!exstack, "BUG: execrestore() without execsave()")
            crate::DPUTS!(true, "BUG: execrestore() without execsave()");
            return;
        }
    };
    queue_signals(); // c:6476
    *head = en.next; // c:6477
    drop(head); // release lock before scalar restores

    list_pipe_pid.store(en.list_pipe_pid, Ordering::Relaxed); // c:6479
    nowait.store(en.nowait, Ordering::Relaxed); // c:6480
    pline_level.store(en.pline_level, Ordering::Relaxed); // c:6481
    list_pipe_child.store(en.list_pipe_child, Ordering::Relaxed); // c:6482
    list_pipe_job.store(en.list_pipe_job, Ordering::Relaxed); // c:6483
                                                              // c:6484 — list_pipe_text restore (not yet stored as Rust static)
    LASTVAL.store(en.lastval, Ordering::Relaxed); // c:6485
    // c:6486 — `noeval = en->noeval;` — same privacy issue as the
    // execsave side; restore is a no-op for now (deps WARNING).
    let _ = en.noeval;
                                                // c:6487 — badcshglob restore (not yet stored as Rust static)
    cmdoutpid.store(en.cmdoutpid, Ordering::Relaxed); // c:6488
    cmdoutval.store(en.cmdoutval, Ordering::Relaxed); // c:6489
    use_cmdoutval.store(en.use_cmdoutval, Ordering::Relaxed); // c:6490
    procsubstpid.store(en.procsubstpid, Ordering::Relaxed); // c:6491
    TRAP_RETURN.store(en.trap_return, Ordering::Relaxed); // c:6492
    TRAP_STATE.store(en.trap_state, Ordering::Relaxed); // c:6493
    crate::ported::signals::trapisfunc.store(en.trapisfunc, Ordering::Relaxed); // c:6494
    crate::ported::signals::traplocallevel.store(en.traplocallevel, Ordering::Relaxed); // c:6495
    noerrs.store(en.noerrs, Ordering::Relaxed); // c:6496
    this_noerrexit.store(en.this_noerrexit, Ordering::Relaxed); // c:6497
                                                                // c:6498-6499 — `setunderscore(en->underscore); zsfree(en->underscore);`
    if let Some(ref u) = en.underscore {
        setunderscore(u); // c:6498
    }
    // c:6500 — `free(en);` — handled by Box drop when `en` falls out of scope.
    unqueue_signals(); // c:6502
}

/// Port of `void execstring(char *s, int dont_change_job, int exiting,
/// char *context)` from `Src/exec.c:1228`.
///
/// C body:
/// ```c
/// Eprog prog;
/// pushheap();
/// if (isset(VERBOSE)) {
///     zputs(s, stderr);
///     fputc('\n', stderr);
///     fflush(stderr);
/// }
/// if ((prog = parse_string(s, 0)))
///     execode(prog, dont_change_job, exiting, context);
/// popheap();
/// ```
///
/// Public entry — execute an arbitrary string as a zsh command list.
/// Called by `eval`, `.`/`source`, `trap` action firing, autoload
/// body executors, command substitution body runners.
///
/// =================== WARNING — DIVERGENCE ====================
/// The C path is `parse_string` → `execode` → `execlist` (wordcode
/// walker). zshrs replaces `execode/execlist` with the fusevm
/// bytecode VM at `crate::vm_helper::ShellExecutor::execute_script_zsh_pipeline`.
/// Faithful port: VERBOSE banner + pushheap/popheap intact; the
/// parse+execute chain delegates to the fusevm entry. When `execlist`
/// lands as a strict 1:1 port, swap the delegate for the canonical
/// chain.
/// =============================================================
pub fn execstring(s: &str, _dont_change_job: i32, _exiting: i32, _context: &str) {
    // c:1228
    pushheap(); // c:1232
                                    // c:1233-1237 — VERBOSE banner.
    if isset(VERBOSE) {
        // c:1233
        let mut stderr = std::io::stderr().lock();
        use std::io::Write;
        let _ = stderr.write_all(s.as_bytes()); // c:1234 zputs(s, stderr)
        let _ = stderr.write_all(b"\n"); // c:1235
        let _ = stderr.flush(); // c:1236
    }
    // c:1238-1239 — parse + execode. zshrs delegates the parse+VM
    // chain to the fusevm pipeline (see WARNING above).
    let _ = with_executor(|e| e.execute_script_zsh_pipeline(s));
    popheap(); // c:1240
}

/// Port of `Emulation_options sticky_emulation_dup(Emulation_options src,
/// int useheap)` from `Src/exec.c:5501`.
///
/// C body (`useheap` selects between heap-arena and permanent zalloc;
/// Rust collapses both into owned `Box` clones):
/// ```c
/// Emulation_options newsticky = useheap ?
///     hcalloc(sizeof(*src)) : zshcalloc(sizeof(*src));
/// newsticky->emulation = src->emulation;
/// if (src->n_on_opts) {
///     size_t sz = src->n_on_opts * sizeof(*src->on_opts);
///     newsticky->n_on_opts = src->n_on_opts;
///     newsticky->on_opts = useheap ? zhalloc(sz) : zalloc(sz);
///     memcpy(newsticky->on_opts, src->on_opts, sz);
/// }
/// if (src->n_off_opts) {
///     size_t sz = src->n_off_opts * sizeof(*src->off_opts);
///     newsticky->n_off_opts = src->n_off_opts;
///     newsticky->off_opts = useheap ? zhalloc(sz) : zalloc(sz);
///     memcpy(newsticky->off_opts, src->off_opts, sz);
/// }
/// return newsticky;
/// ```
///
/// Deep-clone a sticky emulation struct. Used by `shfunc_set_sticky`
/// at function-def time to snapshot the pending `sticky` global so
/// the function carries its own immutable copy.
pub fn sticky_emulation_dup(
    src: &emulation_options,
    _useheap: i32,
) -> Emulation_options {
    // c:5501
    // c:5503-5505 — `newsticky = hcalloc/zshcalloc; newsticky->emulation = src->emulation;`
    let mut newsticky = Box::new(emulation_options {
        emulation: src.emulation, // c:5505
        n_on_opts: 0,
        n_off_opts: 0,
        on_opts: Vec::new(),
        off_opts: Vec::new(),
    });
    // c:5506-5511 — copy on_opts.
    if src.n_on_opts != 0 {
        // c:5506
        newsticky.n_on_opts = src.n_on_opts; // c:5508
        newsticky.on_opts = src.on_opts.clone(); // c:5510 memcpy
    }
    // c:5512-5517 — copy off_opts.
    if src.n_off_opts != 0 {
        // c:5512
        newsticky.n_off_opts = src.n_off_opts; // c:5514
        newsticky.off_opts = src.off_opts.clone(); // c:5516 memcpy
    }
    newsticky // c:5519
}

/// Port of `void shfunc_set_sticky(Shfunc shf)` from `Src/exec.c:5527`.
///
/// C body:
/// ```c
/// if (sticky)
///     shf->sticky = sticky_emulation_dup(sticky, 0);
/// else
///     shf->sticky = NULL;
/// ```
///
/// Stamp the function with the current pending sticky-emulation
/// snapshot (deep-copy via `sticky_emulation_dup`), or clear it.
pub fn shfunc_set_sticky(shf: &mut shfunc) {
    // c:5527
    let sticky_guard = sticky.lock().unwrap();
    if let Some(ref s) = *sticky_guard {
        // c:5529
        shf.sticky = Some(sticky_emulation_dup(s, 0)); // c:5530
    } else {
        // c:5531
        shf.sticky = None; // c:5532
    }
}

/// Port of `static char *search_defpath(char *cmd, char *pbuf, int plen)`
/// from `Src/exec.c:691`.
///
/// Walk DEFAULT_PATH for an executable `<dir>/<cmd>` regular file.
/// Used by `command -p` to bypass the user's `$PATH` and search the
/// system default (`/bin:/usr/bin:...`).
pub fn search_defpath(cmd: &str, plen: usize) -> Option<String> {
    // c:691
    // c:695 — `for (ps = DEFAULT_PATH; ps; ps = pe ? pe+1 : NULL)`.
    for ps in DEFAULT_PATH.split(':') {
        // c:695
        // c:697 — `if (*ps == '/')`.
        if !ps.starts_with('/') {
            continue;
        }
        // c:700-707 — PATH_MAX bounds check on `<dir>` segment.
        if ps.len() >= plen {
            // c:700 / c:704
            continue; // c:701 / c:705
        }
        // c:708 — `*s++ = '/';`. c:709-710 bounds check on `<dir>/<cmd>`.
        let full_len = ps.len() + 1 + cmd.len();
        if full_len >= plen {
            // c:709
            continue; // c:710
        }
        let buf = format!("{}/{}", ps, cmd); // c:711 `strucpy(&s, cmd);`
                                             // c:712 — `if (iscom(pbuf)) return pbuf;`
        if iscom(&buf) {
            // c:712
            return Some(buf); // c:713
        }
    }
    None // c:716
}

/// Port of `static int checkclobberparam(struct redir *f)` from
/// `Src/exec.c:2178`.
///
/// C body:
/// ```c
/// struct value vbuf; Value v;
/// char *s = f->varid; int fd;
/// if (!s) return 1;
/// if (!(v = getvalue(&vbuf, &s, 0))) return 1;
/// if (v->pm->node.flags & PM_READONLY) {
///     zwarn("can't allocate file descriptor to readonly parameter %s",
///           f->varid);
///     errno = 0;
///     return 0;
/// }
/// /* We can't clobber the value in the parameter if it's
///  * already an opened file descriptor */
/// if (!isset(CLOBBER) && (s = getstrvalue(v)) &&
///     (fd = (int)zstrtol(s, &s, 10)) >= 0 && !*s &&
///     fd <= max_zsh_fd && fdtable[fd] == FDT_EXTERNAL) {
///     zwarn("can't clobber parameter %s containing file descriptor %d",
///          f->varid, fd);
///     errno = 0;
///     return 0;
/// }
/// return 1;
/// ```
///
/// Validate that `f->varid` (the `{var}>file` brace-FD form's var
/// name) is writable and (under NOCLOBBER) doesn't currently hold an
/// FDT_EXTERNAL fd number. Returns 1 on OK, 0 on refusal (zwarn
/// already emitted).
///
/// =================== WARNING — DIVERGENCE ====================
/// The NOCLOBBER + FDT_EXTERNAL clause at c:2205-2213 needs
/// `max_zsh_fd` and `fdtable[fd]` — neither global is yet modeled
/// in zshrs (the fdtable port is a no-op shim at utils.rs:1978).
/// That clause is skipped here. Without it, the only refusal path
/// is the PM_READONLY guard at c:2191; the param-fd-already-open
/// case falls through to OK and the upcoming open(2) clobbers it.
/// Re-enable when fdtable lands.
/// =============================================================
pub fn checkclobberparam(f: &redir) -> i32 {
    // c:2178
    // c:2182 — `char *s = f->varid;`
    let s = match &f.varid {
        Some(v) => v.clone(),
        None => return 1, // c:2185-2186 — `if (!s) return 1;`
    };
    // c:2188-2197 — readonly refusal: lookup PM flags directly via
    // paramtab (the C `getvalue` returns a Value wrapping the same
    // pm; we read pm->node.flags here without the wrap).
    let readonly = paramtab()
        .read()
        .ok()
        .and_then(|t| {
            t.get(&s)
                .map(|p| (p.node.flags as u32 & PM_READONLY) != 0)
        })
        .unwrap_or(false);
    if readonly {
        // c:2191
        zwarn(&format!(
            // c:2192
            "can't allocate file descriptor to readonly parameter {}",
            s
        ));
        // c:2195 — `errno = 0;` not flagged as a system error.
        return 0; // c:2196
    }
    // c:2199-2213 — NOCLOBBER + FDT_EXTERNAL refusal. SKIPPED — see
    // WARNING above (fdtable not modeled). When fdtable lands, port:
    //   if !isset(CLOBBER)
    //     && getstrvalue(v) parses as int fd
    //     && fd <= max_zsh_fd
    //     && fdtable[fd] == FDT_EXTERNAL
    //   then zwarn + return 0.
    1 // c:2214
}

/// Port of `static int clobber_open(struct redir *f)` from
/// `Src/exec.c:2221`.
///
/// C body:
/// ```c
/// struct stat buf;
/// int fd, oerrno;
/// char *ufname = unmeta(f->name);
/// /* If clobbering, just open. */
/// if (isset(CLOBBER) || IS_CLOBBER_REDIR(f->type))
///     return open(ufname, O_WRONLY | O_CREAT | O_TRUNC | O_NOCTTY, 0666);
/// /* If not clobbering, attempt to create file exclusively. */
/// if ((fd = open(ufname, O_WRONLY | O_CREAT | O_EXCL | O_NOCTTY, 0666)) >= 0)
///     return fd;
/// /* If that fails, we are still allowed to open non-regular files. */
/// oerrno = errno;
/// if ((fd = open(ufname, O_WRONLY | O_NOCTTY)) != -1) {
///     if (!fstat(fd, &buf)) {
///         if (!S_ISREG(buf.st_mode)) return fd;
///         /* CLOBBER_EMPTY allows re-use of empty regular files. */
///         if (isset(CLOBBEREMPTY) && buf.st_size == 0) return fd;
///     }
///     close(fd);
/// }
/// errno = oerrno;
/// return -1;
/// ```
///
/// Open the redir target for write with the NOCLOBBER rules:
/// - CLOBBER set or `>|` form → just open with O_TRUNC
/// - Otherwise → try O_EXCL first; on EEXIST, only allow non-regular
///   files (FIFOs, devices, sockets) OR empty regular files under
///   CLOBBEREMPTY.
pub fn clobber_open(f: &redir) -> i32 {
    // c:2221
    let ufname_owned = unmeta(f.name.as_deref().unwrap_or("")); // c:2225
    let ufname = match std::ffi::CString::new(ufname_owned.as_str()) {
        Ok(c) => c,
        Err(_) => return -1,
    };
    // c:2228-2230 — clobber path: just open + truncate.
    if isset(CLOBBER)
        || IS_CLOBBER_REDIR(f.typ)
    {
        // c:2228
        // c:2229 — `open(ufname, O_WRONLY|O_CREAT|O_TRUNC|O_NOCTTY, 0666)`
        let fd = unsafe {
            libc::open(
                ufname.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_NOCTTY,
                0o666 as libc::c_uint,
            )
        };
        return fd; // c:2230
    }
    // c:2233-2235 — try O_EXCL create first.
    let fd = unsafe {
        // c:2233
        libc::open(
            ufname.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOCTTY,
            0o666 as libc::c_uint,
        )
    };
    if fd >= 0 {
        return fd; // c:2235
    }
    // c:2240 — `oerrno = errno;` — save for restoration on the recover path.
    let oerrno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
    // c:2241-2260 — recover: open() w/o O_EXCL, accept if non-regular
    // OR (CLOBBEREMPTY && size == 0).
    let fd = unsafe {
        // c:2241
        libc::open(
            ufname.as_ptr(),
            libc::O_WRONLY | libc::O_NOCTTY,
            0o666 as libc::c_uint,
        )
    };
    if fd != -1 {
        let mut buf: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe { libc::fstat(fd, &mut buf) } == 0 {
            // c:2242
            // c:2243-2244 — non-regular file: accept.
            if (buf.st_mode & libc::S_IFMT) != libc::S_IFREG {
                // c:2243
                return fd; // c:2244
            }
            // c:2256-2257 — CLOBBEREMPTY + empty regular: accept.
            if isset(CLOBBEREMPTY) && buf.st_size == 0 {
                // c:2256
                return fd; // c:2257
            }
        }
        unsafe {
            libc::close(fd);
        } // c:2259
    }
    // c:2262 — `errno = oerrno;` — restore the EEXIST so caller diagnoses
    // "file exists" not the noisier "couldn't reopen" trailing errno.
    unsafe {
        *libc::__error() = oerrno; // macOS errno location
    }
    -1 // c:2263
}

/// Port of `char *findcmd(char *arg0, int docopy, int default_path)`
/// from `Src/exec.c:897`. Walk `$PATH` (or DEFAULT_PATH under
/// `default_path=1`) for `arg0`, returning the matching path on
/// success. `_docopy` is the C source's "duplicate the result"
/// flag — Rust ownership covers it without an explicit copy step.
/// `default_path=1` forces `/bin:/usr/bin:...` search (used by
/// `command -p`).
pub fn findcmd(arg0: &str, _docopy: i32, default_path: i32) -> Option<String> {
    // c:897
    // c:903-908 — if (default_path) → search_defpath; return.
    if default_path != 0 {
        return search_defpath(arg0, libc::PATH_MAX as usize);
    }
    // c:912-913 — strlen(arg0) > PATH_MAX → NULL.
    if arg0.len() > libc::PATH_MAX as usize {
        return None;
    }
    // c:914-920 — `/`-bearing arg: accept only if absolute OR (relative
    // + PATHDIRS set and not ./ ../).
    if arg0.contains('/') {
        // c:915 — `RET_IF_COM(arg0)` — accept if it's an existing executable.
        if iscom(arg0) {
            if arg0.starts_with('/') {
                return Some(arg0.to_string()); // c:916
            }
            // c:917-919 — relative + PATHDIRS set → fall through to walk.
            if arg0.starts_with("./")
                || arg0.starts_with("../")
                || !isset(PATHDIRS)
            {
                return None;
            }
            // else fall through to PATH walk.
        } else {
            return None;
        }
    }
    // c:943-951 — walk `path[]` (the shell `$path` array). Read $PATH
    // from paramtab so shell-private edits via `path=(...)` take
    // effect (not OS env only).
    let path = getsparam("PATH")?;
    for dir in path.split(':') {
        if dir.is_empty() {
            continue;
        }
        let candidate = format!("{}/{}", dir, arg0);
        if iscom(&candidate) {
            return Some(candidate);
        }
    }
    None // c:952
}

/// Port of `static void fixfds(int *save)` from `Src/exec.c:4523`.
///
/// C body:
/// ```c
/// int old_errno = errno;
/// int i;
/// for (i = 0; i != 10; i++)
///     if (save[i] != -2)
///         redup(save[i], i);
/// errno = old_errno;
/// ```
///
/// Restore fds 0..9 from the `save[10]` slot array. `-2` sentinel
/// means "no save was made for this fd"; any other value is the
/// stashed fd that gets `dup2`'d back via `redup`. Preserves the
/// caller's errno across the loop so a downstream caller diagnoses
/// the original failure, not a noisy dup2 errno.
pub fn fixfds(save: &[i32; 10]) {
    // c:4523
    let old_errno = std::io::Error::last_os_error().raw_os_error().unwrap_or(0); // c:4525
    for i in 0..10i32 {
        // c:4528 — `for (i = 0; i != 10; i++)`
        if save[i as usize] != -2 {
            // c:4529
            crate::ported::utils::redup(save[i as usize], i); // c:4530
        }
    }
    // c:4531 — `errno = old_errno;`
    #[cfg(target_os = "macos")]
    unsafe {
        *libc::__error() = old_errno;
    }
    #[cfg(target_os = "linux")]
    unsafe {
        *libc::__errno_location() = old_errno;
    }
}

/// Port of `mod_export void closem(int how, int all)` from `Src/exec.c:4546`.
///
/// C body:
/// ```c
/// int i;
/// for (i = 10; i <= max_zsh_fd; i++)
///     if (fdtable[i] != FDT_UNUSED &&
///         (all || (fdtable[i] != FDT_PROC_SUBST &&
///                  fdtable[i] != FDT_EXTERNAL)) &&
///         (how == FDT_UNUSED || (fdtable[i] & FDT_TYPE_MASK) == how)) {
///         if (i == SHTTY) SHTTY = -1;
///         zclose(i);
///     }
/// ```
///
/// Walk fds 10..=MAX_ZSH_FD and close every internal shell fd that
/// matches the criteria. `how == FDT_UNUSED` matches all kinds (no
/// type filter); otherwise only fds whose low-nibble type equals
/// `how` are closed. `all == 0` preserves user-visible fds
/// (FDT_PROC_SUBST, FDT_EXTERNAL) since those need to outlive the
/// shell's internal-fd lifetime. SHTTY clearing prevents a stale
/// reference if we just closed the controlling tty.
pub fn closem(how: i32, all: i32) {
    // c:4546
    use crate::ported::init::SHTTY;
    use crate::ported::utils::{fdtable_get, MAX_ZSH_FD};
    use crate::ported::zsh_h::{FDT_EXTERNAL, FDT_PROC_SUBST, FDT_TYPE_MASK, FDT_UNUSED};
    let max = MAX_ZSH_FD.load(Ordering::Relaxed); // c:4550
    for i in 10i32..=max {
        // c:4550
        let kind = fdtable_get(i); // c:4551 fdtable[i]
        if kind == FDT_UNUSED {
            // c:4551
            continue;
        }
        // c:4557-4558 — `(all || (kind != FDT_PROC_SUBST && kind != FDT_EXTERNAL))`
        if all == 0 && (kind == FDT_PROC_SUBST || kind == FDT_EXTERNAL) {
            continue;
        }
        // c:4559 — `(how == FDT_UNUSED || (fdtable[i] & FDT_TYPE_MASK) == how)`
        if how != FDT_UNUSED && (kind & FDT_TYPE_MASK) != how {
            continue;
        }
        // c:4560-4561 — `if (i == SHTTY) SHTTY = -1;`
        if i == SHTTY.load(Ordering::Relaxed) {
            // c:4560
            SHTTY.store(-1, Ordering::Relaxed); // c:4561
        }
        // c:4562 — `zclose(i);`
        let _ = crate::ported::utils::zclose(i);
    }
}

/// Port of `Cmdnam hashcmd(char *arg0, char **pp)` from
/// `Src/exec.c:1010`.
///
/// C body:
/// ```c
/// Cmdnam cn;
/// char *s, buf[PATH_MAX+1];
/// char **pq;
/// if (*arg0 == '/') return NULL;
/// for (; *pp; pp++)
///     if (**pp == '/') {
///         s = buf;
///         struncpy(&s, *pp, PATH_MAX);
///         *s++ = '/';
///         if ((s - buf) + strlen(arg0) >= PATH_MAX) continue;
///         strcpy(s, arg0);
///         if (iscom(buf)) break;
///     }
/// if (!*pp) return NULL;
/// cn = (Cmdnam) zshcalloc(sizeof *cn);
/// cn->node.flags = 0;
/// cn->u.name = pp;
/// cmdnamtab->addnode(cmdnamtab, ztrdup(arg0), cn);
/// if (isset(HASHDIRS)) {
///     for (pq = pathchecked; pq <= pp; pq++) hashdir(pq);
///     pathchecked = pp + 1;
/// }
/// return cn;
/// ```
///
/// Walk `pp[]` (a $path slice starting from `pathchecked`) for the
/// first absolute-PATH entry where `<entry>/<arg0>` is an executable
/// regular file. Inserts the unhashed-cmdnam entry into `cmdnamtab`
/// and (under HASHDIRS) bulk-hashes every PATH dir we walked through
/// so subsequent commands hit the cache.
///
/// Returns the just-inserted `cmdnam` (now in `cmdnamtab`) on success,
/// `None` if `arg0` is absolute or no PATH entry contains it.
pub fn hashcmd(arg0: &str, pp: &[String]) -> Option<cmdnam> {
    // c:1010
    // c:1016 — `if (*arg0 == '/') return NULL;`
    if arg0.starts_with('/') {
        return None; // c:1017
    }
    // c:1018-1028 — walk pp[] for first matching absolute entry.
    let mut found_idx: Option<usize> = None;
    for (i, dir) in pp.iter().enumerate() {
        // c:1018
        if !dir.starts_with('/') {
            // c:1019
            continue;
        }
        // c:1020-1025 — buf = "<dir>/<arg0>"; PATH_MAX bounds check.
        if dir.len() + 1 + arg0.len() >= libc::PATH_MAX as usize {
            // c:1023
            continue; // c:1024
        }
        let buf = format!("{}/{}", dir, arg0); // c:1025
        if iscom(&buf) {
            // c:1026
            found_idx = Some(i);
            break; // c:1027
        }
    }
    // c:1030-1031 — `if (!*pp) return NULL;`
    let pp_idx = match found_idx {
        Some(i) => i,
        None => return None, // c:1031
    };
    // c:1033-1036 — alloc cn, set flags=0, u.name=pp (the matching slice).
    let path_slice: Vec<String> = pp[pp_idx..].to_vec(); // c:1035
    let cn = cmdnam_unhashed(arg0, path_slice); // c:1033-1035
                                                                          // c:1036 — `cmdnamtab->addnode(cmdnamtab, ztrdup(arg0), cn);`
    if let Ok(mut tab) = cmdnamtab_lock().write() {
        tab.add(cn.clone());
    }
    // c:1038-1042 — under HASHDIRS, bulk-hash every dir up to and
    // including the matching one, then bump pathchecked past it.
    if isset(HASHDIRS) {
        // c:1038
        let start = pathchecked.load(Ordering::Relaxed); // c:1039
        for pq in start..=pp_idx {
            // c:1039
            if pq < pp.len() {
                hashdir(&pp[pq], pq); // c:1040
            }
        }
        pathchecked.store(pp_idx + 1, Ordering::Relaxed); // c:1041
    }
    Some(cn) // c:1044
}

/// Port of `static pid_t zfork(struct timespec *ts)` from
/// `Src/exec.c:349`.
///
/// C body:
/// ```c
/// pid_t pid;
/// if (thisjob != -1 && thisjob >= jobtabsize - 1 && !expandjobtab()) {
///     zerr("job table full");
///     return -1;
/// }
/// if (ts) zgettime_monotonic_if_available(ts);
/// queue_signals();
/// pid = fork();
/// unqueue_signals();
/// if (pid == -1) {
///     zerr("fork failed: %e", errno);
///     return -1;
/// }
/// #ifdef HAVE_GETRLIMIT
/// if (!pid) setlimits(NULL);
/// #endif
/// return pid;
/// ```
///
/// fork(2) wrapper with jobtab capacity check + child rlimit
/// re-application. Used by every subshell-spawning path: pipelines,
/// process substitution, async commands, command substitution.
pub fn zfork(ts: Option<&mut crate::ported::zsh_system_h::timespec>) -> libc::pid_t {
    // c:349
    let pid: libc::pid_t;

    // c:356-359 — `if (thisjob != -1 && thisjob >= jobtabsize - 1 && !expandjobtab())`
    let thisjob_lock = THISJOB.get_or_init(|| std::sync::Mutex::new(-1));
    let thisjob = *thisjob_lock.lock().unwrap();
    if thisjob != -1 {
        // c:356
        let needed = (thisjob + 1) as usize;
        let needs_expand = JOBTAB
            .get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock()
            .map(|t| needed >= t.len().saturating_sub(1))
            .unwrap_or(false);
        if needs_expand {
            let mut tab = JOBTAB.get().unwrap().lock().unwrap();
            if !expandjobtab(&mut tab, needed) {
                // c:357
                zerr("job table full"); // c:357
                return -1; // c:358
            }
        }
    }
    // c:360-361 — `if (ts) zgettime_monotonic_if_available(ts);`
    if let Some(ts) = ts {
        zgettime_monotonic_if_available(ts);
    }
    // c:368-370 — `queue_signals(); pid = fork(); unqueue_signals();`
    queue_signals(); // c:368
    pid = unsafe { libc::fork() }; // c:369
    unqueue_signals(); // c:370
                       // c:371-374 — fork failure.
    if pid == -1 {
        // c:371
        zerr(&format!(
            // c:372
            "fork failed: {}",
            std::io::Error::last_os_error()
        ));
        return -1; // c:373
    }
    // c:375-379 — child: re-apply rlimits (HAVE_GETRLIMIT path).
    #[cfg(unix)]
    if pid == 0 {
        // c:376
        let _ = setlimits(""); // c:378
    }
    pid // c:380
}

/// Port of `void loadautofnsetfile(Shfunc shf, char *fdir)` from
/// `Src/exec.c:5657`.
///
/// C body:
/// ```c
/// if (!(shf->node.flags & PM_LOADDIR) ||
///     strcmp(shf->filename, fdir) != 0) {
///     dircache_set(&shf->filename, NULL);
///     if (fdir) {
///         shf->node.flags |= PM_LOADDIR;
///         dircache_set(&shf->filename, fdir);
///     } else {
///         shf->node.flags &= ~PM_LOADDIR;
///         shf->filename = ztrdup(shf->node.nam);
///     }
/// }
/// ```
///
/// Update `shf->filename` to the autoload directory `fdir`. Routes
/// through the refcounted `dircache_set` so identical directory
/// strings are shared across shfunc table entries.
pub fn loadautofnsetfile(shf: &mut shfunc, fdir: Option<&str>) {
    // c:5657
    // c:5664-5665 — `if (!(shf->node.flags & PM_LOADDIR) || strcmp(shf->filename, fdir) != 0)`
    let loaddir = (shf.node.flags as u32 & PM_LOADDIR) != 0;
    let same = match (&shf.filename, fdir) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    };
    if !loaddir || !same {
        // c:5664
        // c:5667 — `dircache_set(&shf->filename, NULL);` — refcount-drop old.
        dircache_set(&mut shf.filename, None);
        if let Some(fdir) = fdir {
            // c:5668
            shf.node.flags |= PM_LOADDIR as i32; // c:5670
            dircache_set(&mut shf.filename, Some(fdir)); // c:5671
        } else {
            // c:5672
            shf.node.flags &= !(PM_LOADDIR as i32); // c:5674
            shf.filename = Some(shf.node.nam.clone()); // c:5675 `ztrdup(shf->node.nam)`
        }
    }
}

/// Port of `int commandnotfound(char *arg0, LinkList args)` from
/// `Src/exec.c:669`.
///
/// C body:
/// ```c
/// Shfunc shf = (Shfunc)
///     shfunctab->getnode(shfunctab, "command_not_found_handler");
/// if (!shf) {
///     lastval = 127;
///     return 1;
/// }
/// pushnode(args, arg0);
/// lastval = doshfunc(shf, args, 1);
/// return 0;
/// ```
///
/// Look up the user-defined `command_not_found_handler` shfunc and
/// invoke it with `arg0` prepended to `args`. Returns 0 if handled,
/// 1 if no handler (so caller emits the standard "command not found"
/// error). Sets `$?` to 127 in the no-handler path.
pub fn commandnotfound(arg0: &str, args: &mut Vec<String>) -> i32 {
    // c:669
    // c:671-672 — `shf = shfunctab->getnode(shfunctab, "command_not_found_handler");`
    let has_handler = shfunctab_lock()
        .read()
        .map(|t| t.get("command_not_found_handler").is_some())
        .unwrap_or(false);
    if !has_handler {
        // c:674
        LASTVAL.store(127, Ordering::Relaxed); // c:675
        return 1; // c:676
    }
    // c:679 — `pushnode(args, arg0);` — prepend arg0.
    args.insert(0, arg0.to_string());
    // c:680 — `lastval = doshfunc(shf, args, 1);`
    // WARNING — DIVERGENCE: `doshfunc` (c:5823) is not yet ported.
    // Route through the executor's function dispatch so the handler
    // actually fires; the C `noreturnval=1` arg is the "don't pop
    // funcstack into $? after return" flag — fusevm's
    // dispatch_function_call already manages funcstack correctly.
    let status = with_executor(|exec| {
        exec.dispatch_function_call("command_not_found_handler", args)
            .unwrap_or(127)
    });
    LASTVAL.store(status, Ordering::Relaxed);
    0 // c:681
}

/// Port of `char *namedpipe(void)` from `Src/exec.c:5001`.
///
/// C body (#ifdef HAVE_FIFOS branch):
/// ```c
/// char *tnam = gettempname(NULL, 1);
/// if (!tnam) {
///     zerr("failed to create named pipe: %e", errno);
///     return NULL;
/// }
/// if (mkfifo(tnam, 0600) < 0) {
///     zerr("failed to create named pipe: %s, %e", tnam, errno);
///     return NULL;
/// }
/// return tnam;
/// ```
///
/// Create a FIFO with a unique name for process substitution. Used by
/// `getproc` (`<(cmd)` / `>(cmd)`) on systems without `/dev/fd`.
pub fn namedpipe() -> Option<String> {
    // c:5001
    let tnam = gettempname(None, true); // c:5003
    let tnam = match tnam {
        Some(t) => t,
        None => {
            // c:5005
            zerr(&format!(
                // c:5006
                "failed to create named pipe: {}",
                std::io::Error::last_os_error()
            ));
            return None; // c:5007
        }
    };
    // c:5010 — `mkfifo(tnam, 0600)`.
    let cstr = match std::ffi::CString::new(tnam.as_str()) {
        Ok(c) => c,
        Err(_) => return None,
    };
    if unsafe { libc::mkfifo(cstr.as_ptr(), 0o600) } < 0 {
        // c:5010
        zerr(&format!(
            // c:5014
            "failed to create named pipe: {}, {}",
            tnam,
            std::io::Error::last_os_error()
        ));
        return None; // c:5015
    }
    Some(tnam) // c:5017
}

/// Port of `Eprog parsecmd(char *cmd, char **eptr)` from `Src/exec.c:4878`.
///
/// C body:
/// ```c
/// char *str;
/// Eprog prog;
/// for (str = cmd + 2; *str && *str != Outpar; str++);
/// if (!*str || cmd[1] != Inpar) {
///     char *errstr = dupstrpfx(cmd, 2);
///     untokenize(errstr);
///     zerr("unterminated `%s...)'", errstr);
///     return NULL;
/// }
/// *str = '\0';
/// if (eptr) *eptr = str+1;
/// if (!(prog = parse_string(cmd + 2, 0))) {
///     zerr("parse error in process substitution");
///     return NULL;
/// }
/// return prog;
/// ```
///
/// Lex a `<(...)`/`>(...)`/`=(...)` body — the leading 2 chars are
/// the marker pair (`Inang+Inpar`, `Outang+Inpar`, `Equals+Inpar`),
/// remainder is the command up to the matching `Outpar`. Returns the
/// parsed Eprog (and writes the post-`)` cursor through `eptr`).
pub fn parsecmd(cmd: &str, eptr: Option<&mut usize>) -> Option<eprog> {
    // c:4878
    let bytes = cmd.as_bytes();
    // c:4883 — `for (str = cmd + 2; *str && *str != Outpar; str++);`
    if bytes.len() < 2 {
        return None;
    }
    let mut str_idx: usize = 2;
    while str_idx < bytes.len() && (bytes[str_idx] as char) != Outpar {
        str_idx += 1;
    }
    // c:4884 — `if (!*str || cmd[1] != Inpar)`.
    if str_idx >= bytes.len() || (bytes[1] as char) != Inpar {
        // c:4884
        let errstr = if bytes.len() >= 2 {
            untokenize(&cmd[..2]) // c:4891-4892
        } else {
            String::new()
        };
        zerr(&format!("unterminated `{}...)'", errstr)); // c:4893
        return None; // c:4894
    }
    // c:4896 — `*str = '\0';` — cmd[str_idx] becomes the terminator.
    // c:4897-4898 — `if (eptr) *eptr = str + 1;`
    if let Some(p) = eptr {
        *p = str_idx + 1;
    }
    // c:4899 — `parse_string(cmd + 2, 0)`.
    let body = &cmd[2..str_idx];
    let prog = parse_string(body, 0);
    if prog.is_none() {
        // c:4899
        zerr("parse error in process substitution"); // c:4900
        return None; // c:4901
    }
    prog // c:4903
}

/// Port of `static int cancd2(char *s)` from `Src/exec.c:6411`.
///
/// C body:
/// ```c
/// struct stat buf;
/// char *us, *us2 = NULL;
/// int ret;
/// if (!isset(CHASEDOTS) && !isset(CHASELINKS)) {
///     if (*s != '/')
///         us = tricat(pwd[1] ? pwd : "", "/", s);
///     else
///         us = ztrdup(s);
///     fixdir(us2 = us);
/// } else
///     us = unmeta(s);
/// ret = !(access(us, X_OK) || stat(us, &buf) || !S_ISDIR(buf.st_mode));
/// if (us2) free(us2);
/// return ret;
/// ```
///
/// True iff `s` is a directory we can `cd` into (X-perm). With
/// `!CHASEDOTS && !CHASELINKS`, lexically canonicalise the path
/// (joining with PWD if relative) so `cd /foo/bar/..` works without
/// resolving the symlink. Otherwise pass `s` through `unmeta` to libc.
pub fn cancd2(s: &str) -> i32 {
    // c:6411
    let us: String;
    // c:6422 — `if (!isset(CHASEDOTS) && !isset(CHASELINKS))`.
    let chasedots = isset(CHASEDOTS); // c:6422
    let chaselinks = isset(CHASELINKS);
    if !chasedots && !chaselinks {
        // c:6422
        // c:6423-6426 — `*s != '/' ? tricat(pwd, "/", s) : ztrdup(s);`
        let pwd_str = getsparam("PWD").unwrap_or_default(); // c:6424 `pwd`
        let mut raw = if !s.starts_with('/') {
            // c:6423
            format!("{}/{}", if pwd_str.len() > 1 { &pwd_str[..] } else { "" }, s)
        } else {
            s.to_string()
        };
        // c:6427 — `fixdir(us2 = us);` — lexical canonicalisation.
        raw = fixdir(&raw);
        us = raw;
    } else {
        // c:6428
        us = unmeta(s); // c:6429
    }
    // c:6430 — `!(access(us, X_OK) || stat(us, &buf) || !S_ISDIR(...))`.
    let cstr = match std::ffi::CString::new(us.as_str()) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    if unsafe { libc::access(cstr.as_ptr(), libc::X_OK) } != 0 {
        return 0;
    }
    let meta = match std::fs::metadata(&us) {
        Ok(m) => m,
        Err(_) => return 0,
    };
    if !meta.file_type().is_dir() {
        return 0;
    }
    1
}

/// Port of `char *cancd(char *s)` from `Src/exec.c:6370`.
///
/// Resolve a `cd` target against `$cdpath` and `cd_able_vars`.
/// Returns the chosen absolute path (heap-dup) if `cancd2` accepts
/// it, else `None`.
///
/// C body uses CDPATH walking + `cd_able_vars()` fallback. Sets
/// `doprintdir = -1` when a non-trivial path is found (so `cd`
/// echoes the resolved path).
pub fn cancd(s: &str) -> Option<String> {
    // c:6370
    // c:6372-6373 — `nocdpath = s[0]=='.' && (s[1]=='/' || !s[1] ||
    //                (s[1]=='.' && (s[2]=='/' || !s[2])))`.
    let bytes = s.as_bytes();
    let nocdpath = bytes.first().copied() == Some(b'.')
        && (bytes.get(1).copied() == Some(b'/')
            || bytes.get(1).is_none()
            || (bytes.get(1).copied() == Some(b'.')
                && (bytes.get(2).copied() == Some(b'/') || bytes.get(2).is_none())));
    // c:6376 — `if (*s != '/')` branch.
    if !s.starts_with('/') {
        // c:6376
        // c:6379-6380 — `if (cancd2(s)) return s;`
        if cancd2(s) != 0 {
            return Some(s.to_string());
        }
        // c:6381-6382 — `if (access(unmeta(s), X_OK) == 0) return NULL;`
        let cstr = std::ffi::CString::new(unmeta(s).as_str()).ok()?;
        if unsafe { libc::access(cstr.as_ptr(), libc::X_OK) } == 0 {
            return None; // c:6382
        }
        // c:6383-6397 — CDPATH walk.
        if !nocdpath {
            let cdpath_str = getsparam("CDPATH").unwrap_or_default();
            for cp in cdpath_str.split(':') {
                // c:6384
                let sbuf = if !cp.is_empty() {
                    format!("{}/{}", cp, s) // c:6386
                } else {
                    s.to_string() // c:6391
                };
                if cancd2(&sbuf) != 0 {
                    // c:6393
                    DOPRINTDIR.store(-1, Ordering::Relaxed); // c:6394
                    return Some(sbuf); // c:6395
                }
            }
        }
        // c:6398-6403 — `cd_able_vars()` fallback.
        if let Some(t) = cd_able_vars(s) {
            // c:6398
            if cancd2(&t) != 0 {
                // c:6399
                DOPRINTDIR.store(-1, Ordering::Relaxed); // c:6400
                return Some(t); // c:6401
            }
        }
        return None; // c:6404
    }
    // c:6406 — absolute path: `return cancd2(s) ? s : NULL;`
    if cancd2(s) != 0 {
        Some(s.to_string())
    } else {
        None
    }
}

/// Port of `char *simple_redir_name(Eprog prog, int redir_type)` from
/// `Src/exec.c:4689`.
///
/// Test if an Eprog encodes a single simple-command consisting of a
/// SINGLE redirection of the requested type with NO command body
/// (the `cat < foo` shape). When true, returns the redir target name
/// (heap-dup) so callers like `$(< file)` short-circuit to a direct
/// `open(2)` instead of fork+pipe+exec.
///
/// C body walks the wordcode at fixed offsets (`pc[0]` = WC_LIST,
/// `pc[1]` = WC_SUBLIST, `pc[2]` = WC_PIPE, `pc[3]` = WC_REDIR,
/// `pc[6]` = WC_SIMPLE with argc=0). zshrs's wordcode buffer is the
/// same shape — this port replicates the same offset reads.
pub fn simple_redir_name(prog: &eprog, redir_type: i32) -> Option<String> {
    // c:4689
    let pc = &prog.prog;
    // c:4694-4702 — guard chain. Walk the wordcode buffer at fixed
    // offsets matching C's `pc[0]..pc[6]` checks.
    if pc.len() < 7 {
        return None;
    }

    if wc_code(pc[0]) != WC_LIST
        || (WC_LIST_TYPE(pc[0]) & Z_END as u32) == 0  // c:4695
        || wc_code(pc[1]) != WC_SUBLIST
        || WC_SUBLIST_FLAGS(pc[1]) != 0  // c:4696
        || WC_SUBLIST_TYPE(pc[1]) != WC_SUBLIST_END  // c:4697
        || wc_code(pc[2]) != WC_PIPE
        || WC_PIPE_TYPE(pc[2]) != WC_PIPE_END  // c:4698
        || wc_code(pc[3]) != WC_REDIR
        || WC_REDIR_TYPE(pc[3]) != redir_type  // c:4699
        || WC_REDIR_VARID(pc[3]) != 0  // c:4700
        || pc[4] != 0  // c:4701
        || wc_code(pc[6]) != WC_SIMPLE
        || WC_SIMPLE_ARGC(pc[6]) != 0
    // c:4702
    {
        return None; // c:4706
    }
    // c:4703 — `return dupstring(ecrawstr(prog, pc + 5, NULL));`
    Some(dupstring(&ecrawstr(prog, 5, None)))
}

/// Port of `int getherestr(struct redir *fn)` from `Src/exec.c:4655`.
///
/// C body:
/// ```c
/// char *s, *t;
/// int fd, len;
/// t = fn->name;
/// singsub(&t);
/// untokenize(t);
/// unmetafy(t, &len);
/// if (!(fn->flags & REDIRF_FROM_HEREDOC))
///     t[len++] = '\n';
/// if ((fd = gettempfile(NULL, 1, &s)) < 0)
///     return -1;
/// write_loop(fd, t, len);
/// close(fd);
/// fd = open(s, O_RDONLY | O_NOCTTY);
/// unlink(s);
/// return fd;
/// ```
///
/// Materialise a `<<<` herestring or unprocessed-here-doc body into a
/// tempfile, then re-open read-only and unlink — gives the consumer a
/// read fd whose backing file is already cleaned up.
pub fn getherestr(fn_: &redir) -> i32 {
    // c:4655
    let mut t: String = fn_.name.clone().unwrap_or_default(); // c:4660
    t = singsub(&t); // c:4661
    t = untokenize(&t); // c:4662
                        // c:4663 — `unmetafy(t, &len);` — strip Meta-escapes.
                        // Reuse the canonical unmetafy port (utils.rs) on a Vec<u8>.
    let mut bytes: Vec<u8> = t.into_bytes();
    let _len = unmetafy(&mut bytes);
    // c:4671-4672 — `if (!(fn->flags & REDIRF_FROM_HEREDOC)) t[len++] = '\n';`
    if (fn_.flags & REDIRF_FROM_HEREDOC) == 0 {
        // c:4671
        bytes.push(b'\n'); // c:4672
    }
    // c:4673-4674 — `if ((fd = gettempfile(NULL, 1, &s)) < 0) return -1;`
    let (fd, s) = match gettempfile(None) {
        Some(p) => p,
        None => return -1, // c:4674
    };
    // c:4675 — `write_loop(fd, t, len);`
    let _ = write_loop(fd, &bytes); // c:4675
                                                          // c:4676 — `close(fd);`
    let _ = zclose(fd); // c:4676
                                              // c:4677 — `fd = open(s, O_RDONLY | O_NOCTTY);`
    let cstr = std::ffi::CString::new(s.as_str()).unwrap_or_default();
    let new_fd = unsafe { libc::open(cstr.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) }; // c:4677
                                                                                       // c:4678 — `unlink(s);`
    unsafe {
        libc::unlink(cstr.as_ptr());
    } // c:4678
    new_fd // c:4679
}

/// Port of `void quote_tokenized_output(char *str, FILE *file)` from
/// `Src/exec.c:2114`.
///
/// C body (abridged):
/// ```c
/// for (; *s; s++) {
///     switch (*s) {
///         case Meta: putc(*++s ^ 32, file); continue;
///         case Nularg: continue;
///         case '\\' '<' '>' '(' '|' ')' '^' '#' '~' '[' ']' '*' '?' '$' ' ':
///             putc('\\', file); break;
///         case '\t': fputs("$'\\t'", file); continue;
///         case '\n': fputs("$'\\n'", file); continue;
///         case '\r': fputs("$'\\r'", file); continue;
///         case '=': if (s == str) putc('\\', file); break;
///         default:
///             if (itok(*s)) { putc(ztokens[*s - Pound], file); continue; }
///     }
///     putc(*s, file);
/// }
/// ```
///
/// Used by `xtrace` (`set -x` printer) and `whence -c` to display a
/// tokenized argv in a form where lexer tokens (`Star`, `Inpar`, …)
/// surface as unescaped chars (`*`, `(`) while literal special chars
/// get backslash-escaped — round-tripping through the shell.
pub fn quote_tokenized_output(str_in: &str, file: &mut impl std::io::Write) -> std::io::Result<()> {
    // c:2114
    let bytes = str_in.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // c:2118 `for (; *s; s++)`
        let c = bytes[i];
        match c {
            x if x == Meta => {
                // c:2120 — `case Meta: putc(*++s ^ 32, file);`
                if i + 1 < bytes.len() {
                    file.write_all(&[bytes[i + 1] ^ 32])?; // c:2121
                    i += 2;
                } else {
                    i += 1;
                }
                continue; // c:2122
            }
            x if x as char == Nularg => {
                // c:2124
                i += 1;
                continue; // c:2126
            }
            b'\\' | b'<' | b'>' | b'(' | b'|' | b')' | b'^' | b'#' | b'~' | b'[' | b']'
            | b'*' | b'?' | b'$' | b' ' => {
                // c:2128-2142
                file.write_all(b"\\")?; // c:2143
            }
            b'\t' => {
                // c:2146
                file.write_all(b"$'\\t'")?; // c:2147
                i += 1;
                continue;
            }
            b'\n' => {
                // c:2150
                file.write_all(b"$'\\n'")?; // c:2151
                i += 1;
                continue;
            }
            b'\r' => {
                // c:2154
                file.write_all(b"$'\\r'")?; // c:2155
                i += 1;
                continue;
            }
            b'=' => {
                // c:2158 — `if (s == str) putc('\\', file);`
                if i == 0 {
                    file.write_all(b"\\")?; // c:2160
                }
            }
            _ => {
                // c:2163 — `if (itok(*s)) putc(ztokens[*s - Pound], file); continue;`
                if itok(c) {
                    // c:2164
                    let pound = Pound as u8;
                    if c >= pound {
                        let idx = (c - pound) as usize;
                        let zt = ztokens.as_bytes();
                        if idx < zt.len() {
                            file.write_all(&[zt[idx]])?; // c:2165 `ztokens[*s - Pound]`
                        }
                    }
                    i += 1;
                    continue;
                }
            }
        }
        file.write_all(&[c])?; // c:2171
        i += 1;
    }
    Ok(())
}
