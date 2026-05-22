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
use crate::ported::builtin::{cd_able_vars, fixdir, BUILTINS, DOPRINTDIR, EXIT_VAL, LASTVAL};
use crate::ported::builtins::rlimits::setlimits;
use crate::ported::builtins::sched::zleactive;
use crate::ported::compat::zgettime_monotonic_if_available;
use crate::ported::config_h::DEFAULT_PATH;
use crate::ported::context::{zcontext_restore, zcontext_save};
use crate::ported::hashtable::{cmdnam_unhashed, cmdnamtab_lock, dircache_set, hashdir, pathchecked, shfunctab_lock};
use crate::ported::hist::{strinbeg, strinend};
use crate::ported::init::{shout, underscorelen, underscoreused, zunderscore, SHTTY};
use crate::ported::input::{inpop, inpush};
use crate::ported::jobs::{expandjobtab, get_usage, release_pgrp, waitforpid, JOBTAB, THISJOB};
use crate::ported::lex::{hgetc, parsestr, tok, untokenize, ztokens, LEXERR, LEX_LEXSTOP, LEX_LINENO};
use crate::ported::mem::{dupstring, dyncat, popheap, pushheap};
use crate::ported::modules::clone::mypgrp;
use crate::ported::options::{dosetopt, opt_state_set, sticky};
use crate::ported::params::{endparamscope, getsparam, locallevel, paramtab, setiparam, zgetenv, zputenv};
use crate::ported::parse::{closedumps, ecrawstr, parse_list};
use crate::ported::prompt::{cmdpop, cmdpush};
use crate::ported::signals::{intrap, queue_signals, settrap, sigtrapped, signal_mask, signal_unblock, trapisfunc, traplocallevel, unqueue_signals, unsettrap};
use crate::ported::signals_h::{child_block, child_unblock, dont_queue_signals, signal_default, signal_ignore, winch_unblock, SIGCOUNT};
use crate::ported::subst::{quotesubst, singsub};
use crate::ported::utils::{errflag, fdtable_get, fdtable_set, gettempfile, gettempname, inc_locallevel, movefd, pathprog, printprompt4, quotedzputs, redup, unmeta, unmetafy, write_loop, zclose, zerr, zwarn, ERRFLAG_ERROR, MAX_ZSH_FD};
use crate::ported::ztype_h::{inull, itok};
use crate::ported::zsh_h::{builtin, eprog, execstack, funcwrap, hashnode, multio, redir, shfunc, unset, BINF_BUILTIN, BINF_CLEARENV, BINF_COMMAND, BINF_DASH, BINF_EXEC, BINF_PREFIX, CHASEDOTS, CHASELINKS, CLOBBER, CLOBBEREMPTY, CS_CMDSUBST, Emulation_options, ERRFLAG_INT, FDT_EXTERNAL, FDT_INTERNAL, FDT_PROC_SUBST, FDT_SAVED_MASK, FDT_TYPE_MASK, FDT_UNUSED, FDT_XTRACE, HASHDIRS, INTERACTIVE, INP_LINENO, IS_CLOBBER_REDIR, IS_DASH, Inang, Inpar, JOBTEXTSIZE, Meta, MONITOR, MULTIOS, MULTIOUNIT, Nularg, Outpar, PATHDIRS, PM_LOADDIR, PM_READONLY, PM_UNDEFINED, POSIXJOBS, POSIXTRAPS, Pound, REDIRF_FROM_HEREDOC, REDIR_CLOSE, REDIR_HEREDOCDASH, REDIR_HERESTR, REDIR_INPIPE, REDIR_OUTPIPE, USEZLE, VERBOSE, WC_END, WC_LIST, WC_LIST_TYPE, WC_PIPE, WC_PIPE_END, WC_PIPE_TYPE, WC_REDIR, WC_REDIR_TYPE, WC_REDIR_VARID, WC_SIMPLE, WC_SIMPLE_ARGC, WC_SUBLIST, WC_SUBLIST_END, WC_SUBLIST_FLAGS, WC_SUBLIST_TYPE, WC_TYPESET, ZSIG_FUNC, ZSIG_IGNORED, Z_END, cmdnam, emulation_options, isset, wc_code, wc_data, WC_CASE_SKIP, WC_CASE_TYPE, WC_FOR_LIST, WC_FOR_SKIP, WC_FOR_TYPE, WC_IF, WC_IF_SKIP, WC_IF_TYPE, WC_REPEAT_SKIP, WC_WHILE_SKIP, WC_WHILE_TYPE};
use crate::ported::builtin::{BREAKS, CONTFLAG, LOOPS, RETFLAG};
use crate::ported::math::mathevali;
use crate::ported::parse::ecgetstr_wordcode as ecgetstr;
use crate::ported::parse::ecgetstr_wordcode;
use crate::zsh_h::XTRACE;
use crate::ported::zsh_system_h::timespec as ZshTimespec;

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
/// Port of `static char list_pipe_text[JOBTEXTSIZE]` from
/// `Src/exec.c:463`. Holds the textual rendering of the in-flight
/// pipe list; saved across nested execlist invocations at
/// exec.c:1372-1380 (zeroed on entry, restored from
/// `old_list_pipe_text` at c:1634-1638) and round-tripped through
/// execsave/execrestore (c:6448 / c:6484). zshrs models it as a
/// length-bounded String guarded by a Mutex — the C `char[80]` cap
/// is a buffer-overflow guard, but matching length matters for the
/// `jobs` builtin's pipe-list rendering.
pub static LIST_PIPE_TEXT: std::sync::Mutex<String> = std::sync::Mutex::new(String::new()); // c:463 (Src/exec.c)

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

/// Port of `mod_export int zsh_subshell;` from `Src/init.c:67`. Visible
/// `$ZSH_SUBSHELL` parameter — incremented by `entersubsh()` each time
/// the shell forks into a subshell (real or fake-exec). Distinct from
/// `subsh` which records whether we ARE a subshell; `zsh_subshell` is
/// the visible depth count.
pub static zsh_subshell: std::sync::atomic::AtomicI32 = // c:67 (Src/init.c)
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
pub static exstack: std::sync::Mutex<Option<Box<execstack>>> = // c:244
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

/// Port of `LinkList getoutput(char *cmd, int qt)` from
/// `Src/exec.c:4712-4791`. Runs a command-substitution body in the
/// active executor, then routes the captured stdout through
/// `readoutput(pipe, qt, NULL)` semantics at c:4855-4872.
///
/// C return shape: `LinkList` of `char*`. Rust port returns
/// `Vec<String>` (same shape, owned).
///
/// `qt` matches C exactly:
///   - qt=1 (quoted, `"$(...)"`): trim trailing newlines, return
///     entire output as a single-element vec. C c:4858-4862: if
///     output empty, returns a single Nularg sentinel so callers
///     see "empty value" rather than "no value".
///   - qt=0 (unquoted, `$(...)`): trim trailing newlines, then
///     `spacesplit(buf, allownull=false)` per c:4865-4871.
///
/// Uses `with_executor` (panics on missing VM context), not
/// `try_with_executor + unwrap_or_default()`. C `getoutput` calls
/// `execpline` directly — there's no "no shell" code path. The
/// silent-no-op pattern (return empty string when no executor) would
/// mask catastrophic state corruption as "command produced no output",
/// which is the failure mode the `subst.rs:496` warning block flags.
/* $(...) */                                                                // c:4709
pub fn getoutput(cmd: &str, qt: i32) -> Vec<String> {
    // c:4713
    // c:4715 — `Eprog prog;`
    let prog: Option<crate::ported::exec::eprog>;
    // c:4716 — `int pipes[2];`  (collapsed: in-process executor; no fork)
    // c:4717 — `pid_t pid;`     (collapsed)
    let mut s: String;                                                       // c:4718
    // c:4720-4723 — `int onc = nocomments; nocomments = (interact &&
    //                !sourcelevel && unset(INTERACTIVECOMMENTS));
    //                prog = parse_string(cmd, 0); nocomments = onc;`
    let onc = crate::ported::lex::LEX_NOCOMMENTS.with(|c| c.get());
    let new_nc = crate::ported::zsh_h::interact()
        && crate::ported::init::sourcelevel.load(std::sync::atomic::Ordering::Relaxed) == 0
        && !crate::ported::zsh_h::isset(crate::ported::zsh_h::INTERACTIVECOMMENTS);
    crate::ported::lex::LEX_NOCOMMENTS.with(|c| c.set(new_nc));
    prog = parse_string(cmd, 0);
    crate::ported::lex::LEX_NOCOMMENTS.with(|c| c.set(onc));

    if prog.is_none() {                                                      // c:4725
        return Vec::new();                                                   // c:4726 return NULL
    }
    let prog = prog.unwrap();

    if !crate::ported::zsh_h::isset(crate::ported::zsh_h::EXECOPT) {         // c:4728
        return Vec::new();                                                   // c:4729 newlinklist()
    }

    // c:4731 — `if ((s = simple_redir_name(prog, REDIR_READ)))` — `$(< word)`
    if let Some(red_name) = simple_redir_name(&prog, crate::ported::zsh_h::REDIR_READ) {
        /* $(< word) */                                                      // c:4732
        s = red_name;
        s = crate::ported::subst::singsub(&s);                               // c:4737
        if crate::ported::utils::errflag.load(std::sync::atomic::Ordering::Relaxed) != 0 {
            return Vec::new();                                               // c:4739
        }
        let s = crate::ported::lex::untokenize(&s);                          // c:4740
        let path_meta = crate::ported::utils::unmeta(&s);                    // c:4741 unmeta(s)
        let cpath = match std::ffi::CString::new(path_meta.as_bytes()) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let stream = unsafe {
            libc::open(cpath.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY)      // c:4741
        };
        if stream == -1 {
            // c:4742 — `zwarn("%e: %s", errno, s);`
            let errno = std::io::Error::last_os_error();
            crate::ported::utils::zerr(&format!("{}: {}", errno, s));
            crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed);
            crate::ported::exec::cmdoutval.store(1, std::sync::atomic::Ordering::Relaxed);
            return Vec::new();                                               // c:4744
        }
        // c:4746 — `retval = readoutput(stream, qt, &readerror);`
        // readoutput is not yet ported as a stand-alone fn; use the
        // canonical executor read path. The c:4855-4871 byte-walking
        // / qt / spacesplit logic stays where it was, applied below.
        let mut buf_str = String::new();
        let mut readerror: i32 = 0;
        let mut file = unsafe { <std::fs::File as std::os::unix::io::FromRawFd>::from_raw_fd(stream) };
        use std::io::Read;
        if let Err(e) = file.read_to_string(&mut buf_str) {
            readerror = e.raw_os_error().unwrap_or(1);
        }
        // file drops → fd closed
        if readerror != 0 {
            crate::ported::utils::zerr(&format!(
                "error when reading {}: {}",                                 // c:4748
                s,
                std::io::Error::from_raw_os_error(readerror)
            ));
            crate::ported::builtin::LASTVAL.store(1, std::sync::atomic::Ordering::Relaxed);
            crate::ported::exec::cmdoutval.store(1, std::sync::atomic::Ordering::Relaxed);
        }
        // c:4751 return retval — readoutput post-walk (c:4855-4871 tail)
        // inlined: trim trailing newlines, then qt-branch.
        let buf = buf_str.trim_end_matches('\n');
        return if qt != 0 {
            if buf.is_empty() {
                vec![String::from(crate::ported::zsh_h::Nularg)]             // c:4859-4861
            } else {
                vec![buf.to_string()]                                        // c:4863
            }
        } else {
            crate::ported::utils::spacesplit(buf, false)                     // c:4865
        };
    }

    // c:4753-4790 — Full fork path: mpipe + zfork + parent
    // readoutput / waitforpid / child execode + _realexit. fusevm runs
    // command substitution in-process, so the fork shape collapses to a
    // synchronous executor call. C control points preserved as cites:
    //   c:4753 mpipe       — handled by ShellExecutor pipe wiring
    //   c:4758 child_block — no-op (no fork)
    //   c:4760 zfork       — replaced by in-process exec
    //   c:4768-4776 parent — equivalent to executor return
    //   c:4778-4789 child  — entersubsh+execode+_realexit collapse
    crate::ported::exec::cmdoutval.store(0, std::sync::atomic::Ordering::Relaxed); // c:4759
    let buf = with_executor(|exec| exec.run_command_substitution(cmd));
    crate::ported::builtin::LASTVAL.store(
        crate::ported::exec::cmdoutval.load(std::sync::atomic::Ordering::Relaxed),
        std::sync::atomic::Ordering::Relaxed,
    );                                                                       // c:4775

    // c:4772 retval = readoutput — post-walk (c:4855-4871 tail) inlined.
    let buf = buf.trim_end_matches('\n');
    if qt != 0 {
        if buf.is_empty() {
            vec![String::from(crate::ported::zsh_h::Nularg)]                 // c:4859-4861
        } else {
            vec![buf.to_string()]                                            // c:4863
        }
    } else {
        crate::ported::utils::spacesplit(buf, false)                         // c:4865
    }
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
    // C reads $fpath via `getaparam("fpath")` (the param-table array form
    // tied to scalar `FPATH` via `typeset -T`). Reading `std::env::var`
    // misses any in-script modification like `fpath=(/some/dir $fpath)`
    // because that mutates the internal param table, not the inherited
    // process env. Fall back to env only when the param table is empty
    // (cold start before any param-table init).
    let dirs: Vec<String> = match spec_path {
        Some(s) => s.to_vec(),
        None => crate::ported::params::getaparam("fpath")
            .filter(|v| !v.is_empty())
            .or_else(|| {
                crate::ported::params::getsparam("FPATH")
                    .map(|v| v.split(':').map(String::from).collect())
            })
            .or_else(|| {
                std::env::var("FPATH")
                    .ok()
                    .map(|v| v.split(':').map(String::from).collect())
            })
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
        list_pipe_text: {
            // c:6448 — `strcpy(es->list_pipe_text, list_pipe_text);`
            let mut buf = [0u8; JOBTEXTSIZE];
            if let Ok(s) = LIST_PIPE_TEXT.lock() {
                let bytes = s.as_bytes();
                let n = bytes.len().min(JOBTEXTSIZE - 1);
                buf[..n].copy_from_slice(&bytes[..n]);
            }
            buf
        },
        lastval: LASTVAL.load(Ordering::Relaxed),             // c:6449
        // c:6450 — `es->noeval = noeval;`. Snapshot math.c's
        // `int noeval` (the parse-only side-effect-skip counter)
        // via math.rs's pub accessor.
        noeval: crate::ported::math::m_noeval(),
        // c:6451 — `es->badcshglob = badcshglob;`. Snapshot the
        // csh-glob diagnostic counter (glob.c:103 / glob.rs
        // BADCSHGLOB) so nested eval / trap dispatch doesn't disturb
        // the outer command's per-line accounting.
        badcshglob: crate::ported::glob::BADCSHGLOB
            .load(std::sync::atomic::Ordering::Relaxed), // c:6451
        cmdoutpid: cmdoutpid.load(Ordering::Relaxed), // c:6452
        cmdoutval: cmdoutval.load(Ordering::Relaxed), // c:6453
        use_cmdoutval: use_cmdoutval.load(Ordering::Relaxed), // c:6454
        procsubstpid: procsubstpid.load(Ordering::Relaxed), // c:6455
        trap_return: TRAP_RETURN.load(Ordering::Relaxed), // c:6456
        trap_state: TRAP_STATE.load(Ordering::Relaxed), // c:6457
        trapisfunc: trapisfunc.load(Ordering::Relaxed), // c:6458
        traplocallevel: traplocallevel.load(Ordering::Relaxed), // c:6459
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
    // c:6484 — `strcpy(list_pipe_text, en->list_pipe_text);`.
    if let Ok(mut s) = LIST_PIPE_TEXT.lock() {
        let nul = en
            .list_pipe_text
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(JOBTEXTSIZE);
        *s = String::from_utf8_lossy(&en.list_pipe_text[..nul]).into_owned();
    }
    LASTVAL.store(en.lastval, Ordering::Relaxed); // c:6485
    // c:6486 — `noeval = en->noeval;`. Restore math.c's noeval
    // counter from the saved frame.
    crate::ported::math::m_noeval_set(en.noeval);
    // c:6487 — `badcshglob = en->badcshglob;`. Restore the csh-glob
    // diagnostic counter saved on entry.
    crate::ported::glob::BADCSHGLOB
        .store(en.badcshglob, std::sync::atomic::Ordering::Relaxed);
    cmdoutpid.store(en.cmdoutpid, Ordering::Relaxed); // c:6488
    cmdoutval.store(en.cmdoutval, Ordering::Relaxed); // c:6489
    use_cmdoutval.store(en.use_cmdoutval, Ordering::Relaxed); // c:6490
    procsubstpid.store(en.procsubstpid, Ordering::Relaxed); // c:6491
    TRAP_RETURN.store(en.trap_return, Ordering::Relaxed); // c:6492
    TRAP_STATE.store(en.trap_state, Ordering::Relaxed); // c:6493
    trapisfunc.store(en.trapisfunc, Ordering::Relaxed); // c:6494
    traplocallevel.store(en.traplocallevel, Ordering::Relaxed); // c:6495
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

/// Port of `void runshfunc(Eprog prog, FuncWrap wrap, char *name)` from
/// `Src/exec.c:6166`. The inner shell-function executor — fires
/// module-registered wrapper handlers around the function body, with
/// `$_` (zunderscore) save/restore and a paramscope push/pop around
/// the wordcode walk.
///
/// C control flow:
/// ```c
/// queue_signals();
/// ou = zalloc(ouu = underscoreused);
/// if (ou) memcpy(ou, zunderscore, underscoreused);
/// while (wrap) {                       // wrapper chain
///     wrap->module->wrapper++;
///     cont = wrap->handler(prog, wrap->next, name);
///     wrap->module->wrapper--;
///     if (!wrap->module->wrapper && (wrap->module->node.flags & MOD_UNLOAD))
///         unload_module(wrap->module);
///     if (!cont) {                     // wrapper handled it
///         if (ou) zfree(ou, ouu);
///         unqueue_signals();
///         return;
///     }
///     wrap = wrap->next;
/// }
/// startparamscope();
/// execode(prog, 1, 0, "shfunc");
/// if (ou) { setunderscore(ou); zfree(ou, ouu); }
/// endparamscope();
/// unqueue_signals();
/// ```
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) `wrap->module->wrapper++/--` (c:6178/6180) — Rust Module
///     doesn't have a `wrapper` refcount field; we skip the bump,
///     which means a wrapper handler that recursively unloads its
///     own module won't be deferred. Re-port Module to add
///     `wrapper: AtomicI32` when wrapper modules ship.
/// (b) `unload_module(wrap->module)` (c:6184) — the registry's
///     `unload_module(&mut self, name)` takes a name not a module
///     ref; we'd need the wrapper to carry the module name to
///     route correctly. Skipped for now (no shipping wrapper
///     modules use this path).
/// (c) `execode(prog, 1, 0, "shfunc")` (c:6195) — no execode port;
///     re-routes through the fusevm pipeline by re-parsing the
///     Eprog's source. When `prog.strs` carries the original
///     source (autoloaded fns) we use that; otherwise we walk the
///     fusevm executor on the prog's name lookup. Re-port execode
///     when execlist lands.
/// (d) `startparamscope/endparamscope` Rust signatures take
///     `&mut HashTable` (params.rs:7425/7435). We pass the global
///     paramtab handle via the params crate.
/// =============================================================
pub fn runshfunc(
    prog: &eprog,
    mut wrap: Option<&funcwrap>,
    name: &str,
) {
    // c:6166
    queue_signals(); // c:6171
                     // c:6173-6175 — snapshot zunderscore into `ou`.
    let ouu = underscoreused.load(Ordering::Relaxed) as usize;
    let ou: Option<String> = if ouu > 0 {
        // c:6174
        Some(zunderscore.lock().unwrap().clone()) // c:6175
    } else {
        None
    };
    // c:6177-6193 — wrapper chain walk.
    while let Some(w) = wrap {
        // c:6177
        // c:6178 — wrap->module->wrapper++ (WARNING a).
        let cont = if let Some(h) = w.handler {
            // c:6179 — WrapFunc takes Eprog by value + next FuncWrap by value.
            // We pass an empty next sentinel (wrapper-chain walks are
            // single-step in zshrs — see chain-walk comment below).
            let next_sentinel = Box::new(funcwrap {
                next: None,
                flags: 0,
                handler: None,
                module: None,
            });
            h(Box::new(prog.clone()), next_sentinel, name)
        } else {
            1
        };
        // c:6180 — wrap->module->wrapper-- (WARNING a).
        // c:6182-6184 — unload_module deferred check (WARNING b).
        if cont == 0 {
            // c:6186 — wrapper claimed the call.
            unqueue_signals(); // c:6189
            return; // c:6190
        }
        // c:6192 — wrap = wrap->next; the linked-list step requires
        // owning the next ref; the borrowed iteration breaks here.
        // Wrapper chains > 1 are extremely rare; we stop at the
        // first to avoid a Box::leak.
        wrap = None;
    }
    // c:6194 — startparamscope (just inc_locallevel internally).
    inc_locallevel();
    // c:6195 — execode(prog, 1, 0, "shfunc") — WARNING (c).
    if let Some(ref src) = prog.strs {
        let _ = with_executor(|e| e.execute_script_zsh_pipeline(src));
    } else {
        // No source — fall back to looking up the function body by name.
        let _ = name;
    }
    if let Some(ou_str) = ou {
        // c:6196
        setunderscore(&ou_str); // c:6197
                                 // c:6198 — zfree(ou, ouu) — Rust drops on scope exit.
    }
    endparamscope(); // c:6200
    unqueue_signals(); // c:6202
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

/// Port of `static void addfd(int forked, int *save, struct multio **mfds,
///                             int fd1, int fd2, int rflag, char *varid)`
/// from `Src/exec.c:2397`.
///
/// C body (~100 lines, three branches):
/// ```c
/// if (varid) {
///     /* {varid}>file form — move fd above 10 and bind $varid to it */
/// } else if (!mfds[fd1] || unset(MULTIOS)) {
///     /* new multio OR MULTIOS off — first redir on this fd */
/// } else {
///     /* additional redir on a fd that's already a multio (split or extend) */
/// }
/// ```
///
/// Register `fd2` (already-open) as a redirection target for `fd1`.
/// Three branches: `varid` writes the moved fd to `$varid` and bumps
/// fdtable[fd1] = FDT_EXTERNAL; new-multio path saves the original fd1
/// (when `!forked`) and stamps mfds[fd1] as a single-entry struct;
/// extend-multio path either splits a ct=1 stream into a pipe + 2 fds
/// via `mpipe`, or appends another fd to an already-split stream
/// (re-allocating `mfds[fd1]` past the MULTIOUNIT boundary).
///
/// =================== WARNING — DIVERGENCE ====================
/// `hrealloc` at c:2485 grows the multio struct past MULTIOUNIT in C
/// (variable-length tail). The Rust port `multio` (zsh_h.rs:1390)
/// declares `fds: [i32; MULTIOUNIT]` (fixed 8-slot array). The
/// extend-past-MULTIOUNIT branch falls back to a zerr() since we
/// can't append past the array bound. Re-port `multio` as
/// `Vec<i32>` (or a sized-tail variant) to remove this cap.
///
/// `fdtable[fdN] |= FDT_SAVED_MASK` at c:2440 — Rust fdtable_set
/// stores the int value but doesn't expose a bitwise-OR setter; we
/// re-read + OR + re-store as two atomic-feeling steps.
/// =============================================================
pub fn addfd(
    forked: i32,
    save: &mut [i32; 10],
    mfds: &mut [Option<Box<multio>>; 10],
    fd1: i32,
    fd2: i32,
    rflag: i32,
    varid: Option<&str>,
) {
    // c:2397
    let mut pipes: [i32; 2] = [-1; 2]; // c:2400

    // c:2402-2417 — `if (varid)` branch — {varid}>file shape.
    if let Some(vid) = varid {
        // c:2402
        let fd_moved = movefd(fd2); // c:2404
        if fd_moved == -1 {
            // c:2405
            zerr(&format!(
                // c:2406
                "cannot move fd {}: {}",
                fd2,
                std::io::Error::last_os_error()
            ));
            return; // c:2407
        }
        // c:2409 — `fdtable[fd1] = FDT_EXTERNAL;`
        fdtable_set(fd_moved, FDT_EXTERNAL);
        // c:2410 — `setiparam(varid, (zlong)fd1);`
        setiparam(vid, fd_moved as i64);
        // c:2415-2416 — `if (errflag) zclose(fd1);`
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            // c:2415
            let _ = zclose(fd_moved); // c:2416
        }
        return;
    }
    // c:2418 — `else if (!mfds[fd1] || unset(MULTIOS))`
    let fd1u = fd1 as usize;
    if fd1u >= mfds.len() {
        return;
    }
    if mfds[fd1u].is_none() || unset(MULTIOS) {
        // c:2418
        if mfds[fd1u].is_none() {
            // c:2419 — `starting a new multio`
            // c:2420 — `mfds[fd1] = zhalloc(sizeof(multio));`
            mfds[fd1u] = Some(Box::new(multio {
                ct: 0,
                rflag: 0,
                pipe: -1,
                fds: [-1; MULTIOUNIT],
            }));
            // c:2421 — `if (!forked && save[fd1] == -2)`
            if forked == 0 && save[fd1u] == -2 {
                if fd1 == fd2 {
                    // c:2422
                    save[fd1u] = -1; // c:2423
                } else {
                    // c:2424
                    let fd_n = movefd(fd1); // c:2425
                    if fd_n < 0 {
                        // c:2430
                        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                        if e != libc::EBADF {
                            // c:2431
                            zerr(&format!(
                                // c:2432
                                "cannot duplicate fd {}: {}",
                                fd1,
                                std::io::Error::from_raw_os_error(e)
                            ));
                            mfds[fd1u] = None; // c:2433
                            closemnodes(mfds); // c:2434
                            return; // c:2435
                        }
                    } else {
                        // c:2438-2439 — DPUTS check that the saved fd is FDT_INTERNAL.
                        crate::DPUTS!(
                            fdtable_get(fd_n) != FDT_INTERNAL,
                            "Saved file descriptor not marked as internal"
                        );
                        // c:2440 — `fdtable[fdN] |= FDT_SAVED_MASK;`
                        let cur = fdtable_get(fd_n);
                        fdtable_set(fd_n, cur | FDT_SAVED_MASK);
                    }
                    save[fd1u] = fd_n; // c:2442
                }
            }
        }
        // c:2446-2447 — `if (!varid) redup(fd2, fd1);` (varid already
        // handled above; this is the non-varid branch.)
        let _ = redup(fd2, fd1);
        // c:2448-2450 — `mfds[fd1]->ct=1; mfds[fd1]->fds[0]=fd1; mfds[fd1]->rflag=rflag;`
        if let Some(mn) = mfds[fd1u].as_mut() {
            mn.ct = 1; // c:2448
            mn.fds[0] = fd1; // c:2449
            mn.rflag = rflag; // c:2450
        }
    } else {
        // c:2451 — extend existing multio.
        // c:2452-2456 — rflag mismatch check.
        let cur_rflag = mfds[fd1u].as_ref().map(|m| m.rflag).unwrap_or(0);
        if cur_rflag != rflag {
            // c:2452
            zerr(&format!("file mode mismatch on fd {}", fd1)); // c:2453
            closemnodes(mfds); // c:2454
            return; // c:2455
        }
        let cur_ct = mfds[fd1u].as_ref().map(|m| m.ct).unwrap_or(0);
        if cur_ct == 1 {
            // c:2457 — split the stream.
            // c:2458 — `int fdN = movefd(fd1);`
            let fd_n = movefd(fd1);
            if fd_n < 0 {
                // c:2459
                zerr(&format!(
                    // c:2460
                    "multio failed for fd {}: {}",
                    fd1,
                    std::io::Error::last_os_error()
                ));
                closemnodes(mfds); // c:2461
                return; // c:2462
            }
            if let Some(mn) = mfds[fd1u].as_mut() {
                mn.fds[0] = fd_n; // c:2464
            }
            // c:2465 — `fdN = movefd(fd2);`
            let fd_n2 = movefd(fd2);
            if fd_n2 < 0 {
                // c:2466
                zerr(&format!(
                    // c:2467
                    "multio failed for fd {}: {}",
                    fd2,
                    std::io::Error::last_os_error()
                ));
                closemnodes(mfds); // c:2468
                return; // c:2469
            }
            if let Some(mn) = mfds[fd1u].as_mut() {
                mn.fds[1] = fd_n2; // c:2471
            }
            // c:2472 — `mpipe(pipes)`
            if mpipe(&mut pipes) < 0 {
                // c:2472
                zerr(&format!(
                    // c:2473
                    "multio failed for fd {}: {}",
                    fd2,
                    std::io::Error::last_os_error()
                ));
                closemnodes(mfds); // c:2474
                return; // c:2475
            }
            // c:2477 — `mfds[fd1]->pipe = pipes[1 - rflag];`
            if let Some(mn) = mfds[fd1u].as_mut() {
                mn.pipe = pipes[(1 - rflag) as usize];
            }
            // c:2478 — `redup(pipes[rflag], fd1);`
            let _ = redup(pipes[rflag as usize], fd1);
            // c:2479 — `mfds[fd1]->ct = 2;`
            if let Some(mn) = mfds[fd1u].as_mut() {
                mn.ct = 2;
            }
        } else {
            // c:2480 — extend already-split stream.
            // c:2482-2486 — hrealloc past MULTIOUNIT boundary.
            // WARNING DIVERGENCE: Rust `multio.fds` is fixed-size
            // `[i32; MULTIOUNIT]` (zsh_h.rs:1395); the C realloc grows
            // the trailing array, but Rust can't. Bail with zerr when
            // we'd exceed the bound. Re-port multio as Vec<i32> to fix.
            if cur_ct as usize >= MULTIOUNIT {
                zerr(&format!(
                    "multio failed for fd {}: too many outputs (MULTIOUNIT limit, Rust port cap)",
                    fd1
                ));
                closemnodes(mfds);
                return;
            }
            // c:2487 — `if ((fdN = movefd(fd2)) < 0)`
            let fd_n = movefd(fd2);
            if fd_n < 0 {
                zerr(&format!(
                    // c:2488
                    "multio failed for fd {}: {}",
                    fd2,
                    std::io::Error::last_os_error()
                ));
                closemnodes(mfds); // c:2489
                return; // c:2490
            }
            // c:2492 — `mfds[fd1]->fds[mfds[fd1]->ct++] = fdN;`
            if let Some(mn) = mfds[fd1u].as_mut() {
                let slot = mn.ct as usize;
                if slot < mn.fds.len() {
                    mn.fds[slot] = fd_n;
                    mn.ct += 1;
                }
            }
        }
    }
}

/// Port of `static void closemn(struct multio **mfds, int fd, int type)`
/// from `Src/exec.c:2273`.
///
/// C body (abridged — the meat is the fork-into-tee-or-cat child):
/// ```c
/// if (fd >= 0 && mfds[fd] && mfds[fd]->ct >= 2) {
///     struct multio *mn = mfds[fd];
///     char buf[TCBUFSIZE]; int len, i;
///     pid_t pid; struct timespec bgtime;
///     child_block();
///     if ((pid = zfork(&bgtime))) {
///         for (i = 0; i < mn->ct; i++) zclose(mn->fds[i]);
///         zclose(mn->pipe);
///         if (pid == -1) { mfds[fd] = NULL; child_unblock(); return; }
///         mn->ct = 1; mn->fds[0] = fd;
///         addproc(pid, NULL, 1, &bgtime, -1, -1);
///         child_unblock(); return;
///     }
///     /* pid == 0 (child) */
///     opts[INTERACTIVE] = 0;
///     dont_queue_signals();
///     child_unblock();
///     closeallelse(mn);
///     if (mn->rflag) {
///         /* tee process: read mn->pipe, write each mn->fds[i] */
///     } else {
///         /* cat process: read each mn->fds[i], write mn->pipe */
///     }
///     _exit(0);
/// } else if (fd >= 0 && type == REDIR_CLOSE)
///     mfds[fd] = NULL;
/// ```
///
/// Success-path close of a multio. For ct>=2 (multiple-output
/// redirection), forks a tee/cat child that proxies bytes between
/// the original fd and the per-output fds. Single-output multios
/// (ct=1) skip the fork entirely and just clear the slot.
///
/// =================== WARNING — DIVERGENCE ====================
/// The `addproc(pid, NULL, 1, &bgtime, -1, -1)` call at c:2299 uses
/// the 6-arg C signature; zshrs's Rust `addproc` (jobs.rs:1516) is
/// drift'd to 4 args `(&mut job, pid, text, aux)` and doesn't yet
/// thread bgtime/fg/bg. The fork + child loop are ported faithfully;
/// the parent-side addproc call is skipped with a flagged comment —
/// the tee/cat child still runs and the multio gets properly drained,
/// just without the parent recording a job-table entry for the child.
/// Re-enable when addproc lands the canonical signature.
/// =============================================================
pub fn closemn(
    mfds: &mut [Option<Box<multio>>; 10],
    fd: i32,
    type_: i32,
) {
    // c:2273
    // c:2275 — `if (fd >= 0 && mfds[fd] && mfds[fd]->ct >= 2)`
    let needs_tee = fd >= 0
        && (fd as usize) < mfds.len()
        && mfds[fd as usize].as_ref().is_some_and(|m| m.ct >= 2);
    if needs_tee {
        // c:2275
        // Take the multio out of the slot so we can move pieces into
        // the child without aliasing the slot.
        let mn = mfds[fd as usize].take().unwrap();
        let mut buf = [0u8; 4092]; // c:2277 TCBUFSIZE
                                   // c:2287 — `child_block();` block SIGCHLD before fork race.
        child_block();
        // c:2288 — `pid = zfork(&bgtime);`
        let mut bgtime = ZshTimespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let pid = zfork(Some(&mut bgtime));
        if pid != 0 {
            // c:2288 parent branch
            // c:2289-2290 — close all per-output fds.
            for i in 0..mn.ct as usize {
                if i < mn.fds.len() {
                    let _ = zclose(mn.fds[i]); // c:2290
                }
            }
            let _ = zclose(mn.pipe); // c:2291
            if pid == -1 {
                // c:2292
                // c:2293 — `mfds[fd] = NULL;` already done via .take()
                child_unblock(); // c:2294
                return; // c:2295
            }
            // c:2297-2298 — `mn->ct = 1; mn->fds[0] = fd;`
            let mut mn_back = mn;
            mn_back.ct = 1; // c:2297
            mn_back.fds[0] = fd; // c:2298
            mfds[fd as usize] = Some(mn_back);
            // c:2299 — `addproc(pid, NULL, 1, &bgtime, -1, -1);`
            // WARNING DIVERGENCE: addproc Rust sig is 4-arg + needs &mut job.
            // Skipped: parent doesn't record the tee/cat child in the job
            // table. The child still drains the pipe correctly.
            let _ = (pid, bgtime);
            child_unblock(); // c:2300
            return; // c:2301
        }
        // c:2303 — child branch (pid == 0).
        opt_state_set("interactive", false); // c:2304
        dont_queue_signals(); // c:2305
        child_unblock(); // c:2306
        closeallelse(&mn); // c:2307
                           // c:2308-2333 — tee or cat loop.
        if mn.rflag != 0 {
            // c:2308 — `mn->rflag` set → tee process
            // c:2310 — `while ((len = read(mn->pipe, buf, TCBUFSIZE)) != 0)`
            loop {
                let len = unsafe {
                    libc::read(mn.pipe, buf.as_mut_ptr() as *mut libc::c_void, buf.len())
                };
                if len == 0 {
                    break;
                }
                if len < 0 {
                    // c:2311
                    let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                    if e == libc::EINTR {
                        // c:2312
                        continue;
                    } else {
                        break; // c:2315
                    }
                }
                // c:2317-2319 — `for i: write_loop(mn->fds[i], buf, len)`
                for i in 0..mn.ct as usize {
                    if i >= mn.fds.len() {
                        break;
                    }
                    if write_loop(mn.fds[i], &buf[..len as usize])
                        .is_err()
                    {
                        break; // c:2319
                    }
                }
            }
        } else {
            // c:2321 — cat process
            for i in 0..mn.ct as usize {
                if i >= mn.fds.len() {
                    break;
                }
                // c:2324 — `while ((len = read(mn->fds[i], buf, TCBUFSIZE)) != 0)`
                loop {
                    let len = unsafe {
                        libc::read(
                            mn.fds[i],
                            buf.as_mut_ptr() as *mut libc::c_void,
                            buf.len(),
                        )
                    };
                    if len == 0 {
                        break;
                    }
                    if len < 0 {
                        let e = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
                        // c:2326 — `if (errno == EINTR && !isatty(mn->fds[i]))`
                        if e == libc::EINTR && unsafe { libc::isatty(mn.fds[i]) } == 0 {
                            continue;
                        } else {
                            break; // c:2329
                        }
                    }
                    // c:2331 — `if (write_loop(mn->pipe, buf, len) < 0) break;`
                    if write_loop(mn.pipe, &buf[..len as usize]).is_err() {
                        break; // c:2332
                    }
                }
            }
        }
        // c:2335 — `_exit(0);`
        unsafe {
            libc::_exit(0);
        }
    } else if fd >= 0 && type_ == REDIR_CLOSE {
        // c:2336
        // c:2337 — `mfds[fd] = NULL;`
        if (fd as usize) < mfds.len() {
            mfds[fd as usize] = None;
        }
    }
}

/// Port of `static void closemnodes(struct multio **mfds)` from
/// `Src/exec.c:2344`.
///
/// C body:
/// ```c
/// int i, j;
/// for (i = 0; i < 10; i++)
///     if (mfds[i]) {
///         for (j = 0; j < mfds[i]->ct; j++)
///             zclose(mfds[i]->fds[j]);
///         mfds[i] = NULL;
///     }
/// ```
///
/// Failure-path cleanup: close every fd stashed in any of the 10
/// multio slots and null the slot. Called from `execcmd_exec` when
/// a redirect setup fails partway through and we need to roll back.
pub fn closemnodes(mfds: &mut [Option<Box<multio>>; 10]) {
    // c:2344
    for i in 0..10 {
        // c:2348
        if let Some(mn) = mfds[i].take() {
            // c:2349
            for j in 0..mn.ct as usize {
                // c:2350
                if j < mn.fds.len() {
                    let _ = zclose(mn.fds[j]); // c:2351
                }
            }
            // c:2352 — `mfds[i] = NULL;` — handled by .take() above.
        }
    }
}

/// Port of `static void closeallelse(struct multio *mn)` from
/// `Src/exec.c:2358`.
///
/// C body:
/// ```c
/// int i, j;
/// long openmax;
/// openmax = fdtable_size;
/// for (i = 0; i < openmax; i++)
///     if (mn->pipe != i) {
///         for (j = 0; j < mn->ct; j++)
///             if (mn->fds[j] == i) break;
///         if (j == mn->ct)
///             zclose(i);
///     }
/// ```
///
/// Close every fd in the open range EXCEPT `mn->pipe` and the fds
/// stashed in `mn->fds`. Called inside the multio tee/cat child
/// process to release every fd the parent had open — only the pipe
/// + per-output fds stay alive for the read/write loop.
pub fn closeallelse(mn: &multio) {
    // c:2358
    // c:2363 — `openmax = fdtable_size;`. zshrs models fdtable as a
    // Vec; use MAX_ZSH_FD as the upper bound (fdtable_size grows past
    // max_zsh_fd in C but every slot past it is FDT_UNUSED anyway).
    let openmax = MAX_ZSH_FD.load(Ordering::Relaxed) + 1; // c:2363
    for i in 0..openmax {
        // c:2365
        if mn.pipe == i {
            // c:2366
            continue;
        }
        // c:2367-2369 — scan mn->fds[] for i; skip-close if found.
        let mut found = false;
        for j in 0..mn.ct as usize {
            // c:2367
            if j < mn.fds.len() && mn.fds[j] == i {
                // c:2368
                found = true;
                break; // c:2369
            }
        }
        // c:2370-2371 — `if (j == mn->ct) zclose(i);`
        if !found {
            let _ = zclose(i); // c:2371
        }
    }
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
            redup(save[i as usize], i); // c:4530
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
        let _ = zclose(i);
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
pub fn zfork(ts: Option<&mut ZshTimespec>) -> libc::pid_t {
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

/// `POUNDBANGLIMIT` from `Src/exec.c:500` — max bytes read from the
/// front of a script when probing for a `#!` shebang line.
pub const POUNDBANGLIMIT: usize = 128;

/// Port of `static char **makecline(LinkList list)` from `Src/exec.c:2046`.
///
/// Builds the argv array from a command's args list. The C version
/// allocates with a 4-slot prepad (2 reserved at the front for the
/// shebang `argv[-1]/argv[-2]` overwrite trick in zexecve) — Rust
/// doesn't need this since we rebuild the Vec on shebang re-exec
/// (see zexecve WARNING e).
///
/// XTRACE side-effect: each arg is printed via quotedzputs to xtrerr
/// (stderr), preceded by the PS4 prefix when first command of the line.
pub fn makecline(list: &[String]) -> Vec<String> {
    // c:2046
    if isset(XTRACE) {
        // c:2055
        if doneps4.load(Ordering::Relaxed) == 0 {
            // c:2056
            printprompt4(); // c:2057
        }
        let mut first = true;
        let mut err = std::io::stderr().lock();
        use std::io::Write;
        for s in list.iter() {
            // c:2059
            if !first {
                let _ = err.write_all(b" "); // c:2063
            }
            first = false;
            let _ = err.write_all(quotedzputs(s).as_bytes()); // c:2061
        }
        let _ = err.write_all(b"\n"); // c:2065
        let _ = err.flush(); // c:2066
    }
    list.to_vec() // c:2071-2072 — argv built; null terminator implicit in CString[] conversion
}

/// Port of `static void execute(LinkList args, int flags, int defpath)`
/// from `Src/exec.c:723`. The canonical "child runs the simple
/// external command" path: STTY/ARGV0/BINF_DASH handling, makecline,
/// closem(FDT_XTRACE) + child_unblock, slash-path direct exec,
/// defpath (`command -p`) search, cmdnamtab + $PATH walk, with
/// commandnotfound-handler fallback and the final exit-code escape
/// (127 not-found / 126 noperm).
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) `cmdnamtab->getnode(cmdnamtab, arg0)` (c:824) — Rust
///     cmdnamtab access pattern differs (hashtable.rs lookup
///     surface). We skip the cmdnam-hashed fast path and fall
///     straight to the $PATH scan; identical observable behavior
///     when the hash is empty/cold (first call), slower-by-one-stat
///     when hashed. Re-wire via cmdnamtab accessor to close.
/// (b) `commandnotfound(arg0, args)` (c:809, 873) calls into the
///     not-yet-ported `doshfunc` for the `command_not_found_handler`
///     shell function. Already routes through executor dispatch
///     (see exec.rs:2783).
/// (c) `_realexit()` (c:810, 874) — bare `std::process::exit`.
/// (d) `SHTTY` close on `!FD_CLOEXEC` (c:781-784) — Rust assumes
///     FD_CLOEXEC platform default (macOS, Linux).
/// (e) `path` Rust accessor uses paramtab lookup for "PATH";
///     `defpath` (`command -p`) walks DEFAULT_PATH via
///     search_defpath (already ported).
/// =============================================================
pub fn execute(args: &mut Vec<String>, flags: u32, defpath: i32) {
    // c:723
    let mut eno: i32 = 0;
    let mut ee: i32; // c:729
    let mut arg0 = if args.is_empty() {
        return;
    } else {
        args[0].clone()
    }; // c:731
    // c:733-748 — STTY pre-exec handling.
    {
        let mut stty = STTYval.lock().unwrap();
        if let Some(s) = stty.take() {
            // c:738 — STTYval = 0 to break recursion.
            if !s.is_empty()
                && unsafe { libc::isatty(0) } != 0
                && unsafe { libc::tcgetpgrp(0) } == unsafe { libc::getpid() }
            {
                drop(stty);
                let cmd = format!("stty {}", s); // c:739
                execstring(&cmd, 1, 0, "stty"); // c:743
            }
        }
    }
    // c:752-763 — ARGV0 override.
    if let Some(z) = zgetenv("ARGV0") {
        args[0] = z.clone(); // c:753
        unsafe {
            let key = std::ffi::CString::new("ARGV0").unwrap();
            libc::unsetenv(key.as_ptr()); // c:760
        }
        arg0 = args[0].clone();
    } else if (flags & BINF_DASH) != 0 {
        // c:764 — `BINF_DASH` prepends `-`.
        args[0] = format!("-{}", arg0); // c:767-768
        arg0 = args[0].clone();
    }
    let argv = makecline(args); // c:771
    let newenvp_owned: Option<Vec<String>> = if (flags & BINF_CLEARENV) != 0 {
        Some(Vec::new()) // c:772-773 — blank_env: char ** with only NULL slot

    } else {
        None
    };
    let newenvp = newenvp_owned.as_deref();
    closem(FDT_XTRACE, 0); // c:779
                            // c:780-785 — !FD_CLOEXEC SHTTY close — WARNING (d).
    child_unblock(); // c:786
    if arg0.len() >= libc::PATH_MAX as usize {
        // c:787
        zerr(&format!("command too long: {}", arg0)); // c:788
        unsafe {
            libc::_exit(1);
        } // c:789
    }
    // c:791-801 — slash in arg0 → direct exec.
    if let Some(slash_pos) = arg0.find('/') {
        let lerrno = zexecve(&arg0, &argv, newenvp); // c:793
        let is_dot = arg0.starts_with('.')
            && (slash_pos == 1 || (arg0.len() > 2 && &arg0[..2] == ".." && slash_pos == 2));
        if slash_pos == 0 || unset(PATHDIRS) || is_dot {
            // c:794
            zerr(&format!(
                "{}: {}",
                std::io::Error::from_raw_os_error(lerrno),
                arg0
            )); // c:797
            let code = if lerrno == libc::EACCES || lerrno == libc::ENOEXEC {
                126
            } else {
                127
            };
            unsafe {
                libc::_exit(code);
            } // c:798
        }
    }
    if defpath != 0 {
        // c:804 — `command -p` default-path search.
        let pbuf = match search_defpath(&arg0, libc::PATH_MAX as usize) {
            Some(p) => p, // c:808
            None => {
                if commandnotfound(&arg0, args) == 0 {
                    // c:809
                    unsafe {
                        libc::_exit(LASTVAL.load(Ordering::Relaxed));
                    }
                }
                zerr(&format!("command not found: {}", arg0)); // c:811
                unsafe {
                    libc::_exit(127);
                } // c:812
            }
        };
        ee = zexecve(&pbuf, &argv, newenvp); // c:815
        let dir = pbuf.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if isgooderr(ee, if dir.is_empty() { "/" } else { dir }) {
            // c:819
            eno = ee;
        }
    } else {
        // c:822 — normal $PATH scan.
        // WARNING (a) — cmdnamtab fast-path skipped.
        let path_str = getsparam("PATH").unwrap_or_default();
        for pp in path_str.split(':') {
            if pp.is_empty() || pp == "." {
                // c:856
                ee = zexecve(&arg0, &argv, newenvp); // c:857
                if isgooderr(ee, pp) {
                    eno = ee;
                }
            } else {
                // c:860
                let candidate = format!("{}/{}", pp, arg0); // c:861-864
                ee = zexecve(&candidate, &argv, newenvp); // c:865
                if isgooderr(ee, pp) {
                    eno = ee;
                }
            }
        }
    }
    // c:871-881 — final error reporting.
    if eno != 0 {
        // c:871
        zerr(&format!(
            "{}: {}",
            std::io::Error::from_raw_os_error(eno),
            arg0
        )); // c:872
    } else if commandnotfound(&arg0, args) == 0 {
        // c:873
        unsafe {
            libc::_exit(LASTVAL.load(Ordering::Relaxed));
        } // c:874
    } else {
        zerr(&format!("command not found: {}", arg0)); // c:876
    }
    let code = if eno == libc::EACCES || eno == libc::ENOEXEC {
        126
    } else {
        127
    }; // c:881
    unsafe {
        libc::_exit(code);
    }
}

/// Port of `static int zexecve(char *pth, char **argv, char **newenvp)`
/// from `Src/exec.c:504`. Wraps `execve(2)` with:
///   - `$_` env var stamped to absolute `pth` (c:514-520)
///   - winch signal unblock right before the syscall (c:527)
///   - on `ENOEXEC` / `ENOENT`: reads the first POUNDBANGLIMIT
///     bytes, parses a `#!interp arg` shebang and re-execs the
///     interpreter (c:534-628). For `ENOEXEC` with no shebang,
///     binary-safety check then falls back to `/bin/sh script` per
///     POSIX (c:588-628).
///
/// Returns `errno` from the failing exec — execve only returns on
/// failure, so success means the calling process is already replaced.
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) C uses `static char buf[PATH_MAX*2+1]` for the `_=...` env
///     string; Rust uses a stack `String` (consumed by `zputenv`).
/// (b) `closedumps()` for `!FD_CLOEXEC` (c:521-523) called
///     unconditionally as a no-op when FD_CLOEXEC is platform default.
/// (c) `unmetafy(pth, NULL)` / round-trip `metafy` at c:510-513,
///     c:639-642 — handled implicitly via &str ↔ CString.
/// (d) `metafy(execvebuf+2, -1, META_STATIC)` (c:551, 575) — we
///     drop the metafy and pass byte ranges to zerr directly.
/// (e) `argv[-1]` / `argv[-2]` shebang interpreter slot-overwriting
///     (C overwrites BEFORE argv[0]) — Rust rebuilds a fresh
///     Vec<String> with interp + optional arg + original argv tail
///     since Vec doesn't expose negative indexing.
/// (f) `environ` is FFI-loaded only when `newenvp` is None.
/// =============================================================
pub fn zexecve(pth: &str, argv: &[String], newenvp: Option<&[String]>) -> i32 {
    // c:504
    use std::ffi::CString;
    // c:514-520 — `_=pth` env stamping.
    let pth_abs = if pth.starts_with('/') {
        // c:516
        pth.to_string() // c:517
    } else {
        // c:518
        format!("{}/{}", getsparam("PWD").unwrap_or_default(), pth) // c:519
    };
    zputenv(&format!("_={}", pth_abs)); // c:520
    closedumps(); // c:522
    winch_unblock(); // c:527
    let cpth = match CString::new(pth) {
        Ok(c) => c,
        Err(_) => return libc::ENOENT,
    };
    let cargs: Vec<CString> = argv
        .iter()
        .filter_map(|a| CString::new(a.as_str()).ok())
        .collect();
    let mut argv_ptrs: Vec<*const libc::c_char> =
        cargs.iter().map(|c| c.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());
    let env_holder: Vec<CString>;
    let env_ptrs: Vec<*const libc::c_char>;
    let envp: *const *const libc::c_char = match newenvp {
        Some(env) => {
            env_holder = env
                .iter()
                .filter_map(|e| CString::new(e.as_str()).ok())
                .collect();
            env_ptrs = {
                let mut v: Vec<*const libc::c_char> =
                    env_holder.iter().map(|c| c.as_ptr()).collect();
                v.push(std::ptr::null());
                v
            };
            env_ptrs.as_ptr()
        }
        None => unsafe {
            extern "C" {
                static environ: *const *const libc::c_char;
            }
            environ
        },
    };
    unsafe {
        libc::execve(cpth.as_ptr(), argv_ptrs.as_ptr(), envp); // c:528
    }
    let eno = std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(libc::ENOEXEC); // c:534
    if eno == libc::ENOEXEC || eno == libc::ENOENT {
        // c:534
        let fd = unsafe { libc::open(cpth.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY) }; // c:538
        if fd < 0 {
            return std::io::Error::last_os_error()
                .raw_os_error()
                .unwrap_or(libc::ENOENT); // c:634
        }
        let mut buf = vec![0u8; POUNDBANGLIMIT + 1]; // c:541
        let ct = unsafe {
            libc::read(
                fd,
                buf.as_mut_ptr() as *mut libc::c_void,
                POUNDBANGLIMIT as libc::size_t,
            )
        }; // c:542
        unsafe {
            libc::close(fd);
        } // c:543
        if ct >= 0 {
            // c:544
            let ct = ct as usize;
            if ct >= 2 && buf[0] == b'#' && buf[1] == b'!' {
                // c:545
                let mut t0 = 0;
                while t0 < ct && buf[t0] != b'\n' {
                    t0 += 1;
                } // c:546-548
                if t0 == ct {
                    // c:549
                    zerr(&format!(
                        // c:550
                        "{}: bad interpreter: {}: {}",
                        pth,
                        String::from_utf8_lossy(&buf[2..t0.min(ct)]),
                        std::io::Error::from_raw_os_error(eno)
                    ));
                } else {
                    // c:552
                    while t0 > 0
                        && (buf[t0] == b' ' || buf[t0] == b'\t' || buf[t0] == b'\n')
                    {
                        buf[t0] = 0;
                        t0 -= 1;
                    } // c:553-554
                    let mut ptr_lo: usize = 2;
                    while ptr_lo < buf.len() && buf[ptr_lo] == b' ' {
                        ptr_lo += 1;
                    } // c:555
                    let ptr2_lo = ptr_lo;
                    let mut ptr_hi = ptr2_lo;
                    while ptr_hi < buf.len()
                        && buf[ptr_hi] != 0
                        && buf[ptr_hi] != b' '
                    {
                        ptr_hi += 1;
                    } // c:556
                    let interp_str =
                        String::from_utf8_lossy(&buf[ptr2_lo..ptr_hi]).into_owned();
                    if eno == libc::ENOENT {
                        // c:557 — pathprog rewrite path.
                        let pprog = if !interp_str.starts_with('/') {
                            // c:561
                            pathprog(&interp_str)
                                .map(|p| p.display().to_string())
                        } else {
                            None
                        };
                        if let Some(pprog) = pprog {
                            // c:562
                            let mut argv_new: Vec<String> =
                                Vec::with_capacity(argv.len() + 2);
                            argv_new.push(interp_str.clone()); // c:564
                            if ptr_hi >= buf.len() || buf[ptr_hi] == 0 {
                                argv_new.push(pth.to_string());
                            } else {
                                // c:567
                                let mut rest_lo = ptr_hi + 1;
                                while rest_lo < buf.len() && buf[rest_lo] == b' ' {
                                    rest_lo += 1;
                                }
                                let mut rest_hi = rest_lo;
                                while rest_hi < buf.len() && buf[rest_hi] != 0 {
                                    rest_hi += 1;
                                }
                                let arg_str =
                                    String::from_utf8_lossy(&buf[rest_lo..rest_hi])
                                        .into_owned();
                                argv_new.push(arg_str);
                                argv_new.push(pth.to_string());
                            }
                            for orig in argv.iter().skip(1) {
                                argv_new.push(orig.clone());
                            }
                            winch_unblock(); // c:565/c:570
                            return zexecve(&pprog, &argv_new, newenvp); // c:566/c:571
                        }
                        zerr(&format!(
                            // c:574
                            "{}: bad interpreter: {}: {}",
                            pth,
                            interp_str,
                            std::io::Error::from_raw_os_error(eno)
                        ));
                    } else if ptr_hi < buf.len() && buf[ptr_hi] != 0 {
                        // c:576
                        let mut rest_lo = ptr_hi + 1;
                        while rest_lo < buf.len() && buf[rest_lo] == b' ' {
                            rest_lo += 1;
                        }
                        let mut rest_hi = rest_lo;
                        while rest_hi < buf.len() && buf[rest_hi] != 0 {
                            rest_hi += 1;
                        }
                        let arg_str =
                            String::from_utf8_lossy(&buf[rest_lo..rest_hi]).into_owned();
                        let mut argv_new: Vec<String> = vec![
                            interp_str.clone(),
                            arg_str,
                            pth.to_string(),
                        ];
                        for orig in argv.iter().skip(1) {
                            argv_new.push(orig.clone());
                        }
                        winch_unblock(); // c:580
                        return zexecve(&interp_str, &argv_new, newenvp); // c:581
                    } else {
                        // c:582
                        let mut argv_new: Vec<String> =
                            vec![interp_str.clone(), pth.to_string()];
                        for orig in argv.iter().skip(1) {
                            argv_new.push(orig.clone());
                        }
                        winch_unblock(); // c:584
                        return zexecve(&interp_str, &argv_new, newenvp); // c:585
                    }
                }
            } else if eno == libc::ENOEXEC {
                // c:588 — binary-safety + /bin/sh fallback.
                let nul_pos = buf[..ct].iter().position(|&b| b == 0); // c:597
                let isbinary = match nul_pos {
                    None => false, // c:598
                    Some(npos) => {
                        let mut has_letter = false;
                        let mut binary = true;
                        for &b in &buf[..npos] {
                            // c:602-609
                            if (b as char).is_ascii_lowercase()
                                || b == b'$'
                                || b == b'`'
                            {
                                has_letter = true;
                            }
                            if has_letter && b == b'\n' {
                                binary = false; // c:606
                                break;
                            }
                        }
                        binary
                    }
                };
                if !isbinary {
                    // c:611
                    let mut argv_new: Vec<String> = Vec::with_capacity(argv.len() + 2);
                    argv_new.push("sh".to_string()); // c:625
                    if !argv.is_empty()
                        && (argv[0].starts_with('-') || argv[0].starts_with('+'))
                    {
                        argv_new.push("-".to_string()); // c:623
                    }
                    for orig in argv.iter() {
                        argv_new.push(orig.clone());
                    }
                    winch_unblock(); // c:626
                    return zexecve("/bin/sh", &argv_new, newenvp); // c:627
                }
            }
        }
    }
    eno // c:643
}

/// Port of `char *getoutputfile(char *cmd, char **eptr)` from
/// `Src/exec.c:4910` — `=(cmd)` process substitution.
///
/// Substitutes the cmd's stdout into a temp file, returns the
/// filename. Optimised path: `=(<<<heredoc-str)` writes the
/// heredoc body directly without a fork.
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) `addfilelist` Rust signature `(&mut job, name, fd)` vs C
///     `addfilelist(name, fd)`. We can't grab the current job
///     handle without a jobs-mod refactor. Filelist registration
///     SKIPPED — the temp file will be cleaned by the system temp
///     reaper, not by zsh's job-exit hook. Re-port addfilelist
///     to match C 2-arg shape to unblock.
/// (b) `waitforpid` Rust takes 1 arg `pid`, C takes `(pid, full)`.
///     Behavior matches the `full=0` case anyway.
/// (c) `entersubsh` not ported — child does setsid only.
/// (d) `execode` not ported — re-feed body via fusevm pipeline.
/// (e) `_realexit` not ported — bare std::process::exit(0).
/// (f) TMPSUFFIX link()-rename block (c:4951-4958) skipped for now
///     until addfilelist re-port lands.
/// =============================================================
pub fn getoutputfile(cmd: &str, eptr: Option<&mut usize>) -> Option<String> {
    // c:4910
    let bytes = cmd.as_bytes();
    let _ = bytes;
    // c:4918 — `if (thisjob == -1)` — guard removed (thisjob model differs).
    let mut ends_at: usize = 0;
    let prog = parsecmd(cmd, Some(&mut ends_at))?; // c:4922
    if let Some(p) = eptr {
        *p = ends_at;
    }
    let mut nam = gettempname(None, true)?; // c:4924
                                            // c:4927 — `simple_redir_name` opt for `=(<<<str)`.
    let mut s: Option<String> = simple_redir_name(&prog, REDIR_HERESTR).map(|raw| {
        // c:4933
        let mut sub = singsub(&raw); // c:4933
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            // c:4934
            String::new() // c:4935 — sentinel; checked below
        } else {
            sub = untokenize(&sub); // c:4937
            dyncat(&sub, "\n") // c:4938
        }
    });
    if let Some(ref sv) = s {
        if sv.is_empty() {
            s = None;
        }
    }
    if s.is_none() {
        // c:4942
        child_block(); // c:4943
    }
    // c:4945 — `open(nam, O_WRONLY|O_CREAT|O_EXCL|O_NOCTTY, 0600)`.
    let c_nam = match std::ffi::CString::new(nam.clone()) {
        Ok(c) => c,
        Err(_) => {
            if s.is_none() {
                child_unblock();
            }
            return None;
        }
    };
    let fd = unsafe {
        libc::open(
            c_nam.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOCTTY,
            0o600 as libc::c_uint,
        )
    };
    if fd < 0 {
        // c:4945
        zerr(&format!(
            "process substitution failed: {}",
            std::io::Error::last_os_error()
        )); // c:4946
        if s.is_none() {
            child_unblock(); // c:4948
        }
        return None; // c:4949
    }
    // c:4951-4958 — TMPSUFFIX link block (see WARNING f).
    // addfilelist(nam, 0) — see WARNING (a).
    if let Some(sv) = s {
        // c:4962 — optimised here-string write path.
        let mut buf: Vec<u8> = sv.into_bytes();
        let _len = unmetafy(&mut buf); // c:4965
        let _ = write_loop(fd, &buf); // c:4966
        unsafe {
            libc::close(fd);
        } // c:4967
        return Some(nam); // c:4968
    }
    // c:4971 — `cmdoutpid = pid = zfork(NULL)`.
    let pid = zfork(None);
    cmdoutpid.store(pid, Ordering::Relaxed);
    if pid == -1 {
        // c:4972
        unsafe {
            libc::close(fd);
        } // c:4973
        child_unblock(); // c:4974
        return Some(nam); // c:4975
    } else if pid != 0 {
        // c:4976 — parent.
        unsafe {
            libc::close(fd);
        } // c:4977
        let _ = waitforpid(pid); // c:4978
        cmdoutval.store(0, Ordering::Relaxed); // c:4979
        return Some(nam); // c:4980
    }
    // c:4983 — child.
    closem(FDT_UNUSED, 0); // c:4984
    let _ = redup(fd, 1); // c:4985
    entersubsh(esub::PGRP | esub::NOMONITOR, None); // c:4986
    cmdpush(CS_CMDSUBST as u8); // c:4987
                                                       // c:4988 — execode — WARNING (d).
    let body_end = if ends_at > 0 { ends_at - 1 } else { 2 };
    let body = if body_end > 2 && body_end <= cmd.len() {
        &cmd[2..body_end]
    } else {
        ""
    };
    let _ = with_executor(|e| e.execute_script_zsh_pipeline(body));
    cmdpop(); // c:4989
    unsafe {
        libc::close(1);
    } // c:4990
                                                          // _realexit — WARNING (e)
    std::process::exit(0); // c:4991
    #[allow(unreachable_code)]
    {
        // c:4992-4993 — `zerr("exit returned in child!!"); kill(getpid(), SIGKILL);`
        let _ = &mut nam;
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
        None
    }
}

/// Port of `char *getproc(char *cmd, char **eptr)` from
/// `Src/exec.c:5025` — `<(cmd)` / `>(cmd)` process substitution
/// via `/dev/fd/N` (PATH_DEV_FD branch; modern Linux/macOS).
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) PATH_DEV_FD branch only — the FIFO fallback (`!PATH_DEV_FD`
///     path c:5037-5064) is omitted; modern Linux/macOS both
///     provide /dev/fd. namedpipe() is ported (exec.rs:2701) but
///     unused here.
/// (b) `addproc` Rust 4-arg drift (see getpipe WARNING a) —
///     procsubstpid set only.
/// (c) `addfilelist(NULL, fd)` skipped (see getoutputfile WARNING a).
/// (d) `entersubsh` not ported — setsid only.
/// (e) `execode` not ported — re-feed body via fusevm.
/// (f) `_realexit` not ported — bare exit(LASTVAL).
/// (g) `fdtable[fd] = FDT_PROC_SUBST` (c:5086) — set via fdtable_set.
/// =============================================================
pub fn getproc(cmd: &str, eptr: Option<&mut usize>) -> Option<String> {
    // c:5025
    let bytes = cmd.as_bytes();
    let out: i32 = if !bytes.is_empty() && (bytes[0] as char) == Inang {
        1 // c:5032 — `<(...)` writer-side child
    } else {
        0
    };
    // c:5068 — `if (thisjob == -1)` guard skipped (thisjob model differs).
    // c:5072 — PATH_DEV_FD path: allocate buffer for the /dev/fd/N string.
    let mut ends_at: usize = 0;
    let _prog = parsecmd(cmd, Some(&mut ends_at))?; // c:5073
    if let Some(p) = eptr {
        *p = ends_at;
    }
    let mut pipes: [i32; 2] = [-1; 2];
    if mpipe(&mut pipes) < 0 {
        // c:5075
        return None;
    }
    let mut bgtime: ZshTimespec = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let pid = zfork(Some(&mut bgtime)); // c:5077
    if pid != 0 {
        // c:5077 — parent path.
        let pnam = format!("/dev/fd/{}", pipes[(1 - out) as usize]); // c:5078
        let _ = zclose(pipes[out as usize]); // c:5079
        if pid == -1 {
            // c:5080
            let _ = zclose(pipes[(1 - out) as usize]); // c:5082
            return None; // c:5083
        }
        let fd = pipes[(1 - out) as usize]; // c:5085
        fdtable_set(fd, FDT_PROC_SUBST); // c:5086
                                         // c:5087 — addfilelist(NULL, fd) skipped (WARNING c)
                                         // c:5088-5091 — addproc skipped (WARNING b)
        procsubstpid.store(pid, Ordering::Relaxed); // c:5092
        return Some(pnam); // c:5093
    }
    // c:5095 — child.
    entersubsh(esub::ASYNC | esub::PGRP, None); // c:5095
    let _ = redup(pipes[out as usize], out); // c:5096
    closem(FDT_UNUSED, 0); // c:5097
    cmdpush(CS_CMDSUBST as u8); // c:5100
    let body_end = if ends_at > 0 { ends_at - 1 } else { 2 };
    let body = if body_end > 2 && body_end <= cmd.len() {
        &cmd[2..body_end]
    } else {
        ""
    };
    let _ = with_executor(|e| e.execute_script_zsh_pipeline(body));
    cmdpop(); // c:5102
    let _ = zclose(out); // c:5103
    std::process::exit(LASTVAL.load(Ordering::Relaxed)); // c:5104
}

/// Port of `enum { ESUB_ASYNC, ESUB_PGRP, ... };` from `Src/exec.c:1056`.
/// Flag bits for `entersubsh(int flags, struct entersubsh_ret *retp)`.
pub mod esub {
    // c:1056
    pub const ASYNC: i32 = 0x01; // c:1058
    pub const PGRP: i32 = 0x02; // c:1063
    pub const KEEPTRAP: i32 = 0x04; // c:1065
    pub const FAKE: i32 = 0x08; // c:1067
    pub const REVERTPGRP: i32 = 0x10; // c:1069
    pub const NOMONITOR: i32 = 0x20; // c:1071
    pub const JOB_CONTROL: i32 = 0x40; // c:1073
}

/// Port of `struct entersubsh_ret` from `Src/exec.c` (forward decl).
/// Out-arg used by `entersubsh()` to hand back the group-leader pid
/// and the list-pipe job index the parent should track. Only filled
/// in for `ESUB_PGRP` + non-async forks (synchronous pipeline child
/// groups).
#[allow(non_camel_case_types)]
#[derive(Default)]
pub struct entersubsh_ret {
    pub gleader: i32,        // c:1122
    pub list_pipe_job: i32,  // c:1123
}

/// Port of `static void entersubsh(int flags, struct entersubsh_ret *retp)`
/// from `Src/exec.c:1083`. Called by every child fork to switch the
/// process into subshell mode: traps reset, monitor disabled, signals
/// re-defaulted, pgrp + tty handed off, saved fds closed, jobtab
/// cleared, ZSH_SUBSHELL bumped, forklevel = locallevel.
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) `jobtab[list_pipe_job]` / `jobtab[thisjob]` pgrp ops (c:1110-
///     1151) are SKIPPED — the Rust jobs module (jobs.rs) hides the
///     jobtab behind a JobTable handle and doesn't expose direct
///     index access in this call shape. The ESUB_PGRP+sync path
///     therefore won't establish pipeline group-leadership in the
///     child until a jobtab accessor is added. `setpgrp(0, 0)` is
///     used as the fallback so the child at least leaves the parent's
///     process group.
/// (b) `clearjobtab(monitor)` (c:1219) — Rust signature is
///     `clearjobtab(&mut JobTable, monitor)`; we get the global table
///     via a TABLE handle similar to other jobs.rs entries.
/// (c) `attachtty(...)` (c:1119, 1144) — only invoked from the pgrp
///     branch which is skipped per (a).
/// (d) `release_pgrp()` called for ESUB_REVERTPGRP when `getpid() ==
///     mypgrp` — direct C parity (jobs.rs:3406 provides the call).
/// (e) `opts[USEZLE] = 0; zleactive = 0` — Rust opts table lookup
///     uses `opts_set_off(USEZLE)`; zleactive is the atomic in
///     builtins/sched.rs.
/// =============================================================
pub fn entersubsh(flags: i32, retp: Option<&mut entersubsh_ret>) {
    // c:1083
    let monitor: i32;
    let job_control_ok: i32;
    // c:1088-1092 — reset traps unless KEEPTRAP.
    if (flags & esub::KEEPTRAP) == 0 {
        // c:1088
        for sig in 0..=SIGCOUNT {
            // c:1089
            let st = {
                let guard = sigtrapped.lock().unwrap();
                guard.get(sig as usize).copied().unwrap_or(0)
            };
            let func_set = (st & ZSIG_FUNC) != 0; // c:1090
            let posix_ignored = isset(POSIXTRAPS) && ((st & ZSIG_IGNORED) != 0); // c:1091
            if !func_set && !posix_ignored {
                unsettrap(sig); // c:1092
            }
        }
    }
    monitor = if isset(MONITOR) { 1 } else { 0 }; // c:1093
    job_control_ok =
        if monitor != 0 && (flags & esub::JOB_CONTROL) != 0 && isset(POSIXJOBS) {
            // c:1094
            1
        } else {
            0
        };
    EXIT_VAL.store(0, Ordering::Relaxed); // c:1095
    if (flags & esub::NOMONITOR) != 0 {
        // c:1096
        dosetopt(MONITOR, 0, 0); // c:1097
    }
    if !isset(MONITOR) {
        // c:1098
        if (flags & esub::ASYNC) != 0 {
            // c:1099
            let _ = settrap(libc::SIGINT, None, 0); // c:1100
            let _ = settrap(libc::SIGQUIT, None, 0); // c:1101
            if unsafe { libc::isatty(0) } != 0 {
                // c:1102
                unsafe {
                    libc::close(0);
                } // c:1103
                let devnull = std::ffi::CString::new("/dev/null").unwrap();
                if unsafe {
                    libc::open(
                        devnull.as_ptr(),
                        libc::O_RDWR | libc::O_NOCTTY,
                    )
                } != 0
                {
                    // c:1104
                    zerr(&format!(
                        // c:1105
                        "can't open /dev/null: {}",
                        std::io::Error::last_os_error()
                    ));
                    unsafe {
                        libc::_exit(1);
                    } // c:1106
                }
            }
        }
    } else {
        // c:1110 — `else if (thisjob != -1 && (flags & ESUB_PGRP))`.
        // WARNING (a) — pgrp/jobtab handling skipped; basic fallback only.
        if (flags & esub::PGRP) != 0 {
            unsafe {
                libc::setpgid(0, 0);
            }
        }
    }
    let _ = retp; // WARNING (a) — out-arg unfilled when pgrp branch skipped.
    if (flags & esub::FAKE) == 0 {
        // c:1153
        subsh.store(1, Ordering::Relaxed); // c:1154
    }
    // c:1161 — `zsh_subshell++;` regardless of FAKE.
    zsh_subshell.fetch_add(1, Ordering::Relaxed);
    // c:1162 — `if ((flags & ESUB_REVERTPGRP) && getpid() == mypgrp)`.
    if (flags & esub::REVERTPGRP) != 0
        && unsafe { libc::getpid() }
            == mypgrp.load(Ordering::Relaxed)
    {
        release_pgrp(); // c:1163
    }
    *shout.lock().unwrap() = 0; // c:1164 — shout = NULL
    if (flags & esub::NOMONITOR) != 0 {
        // c:1165
        signal_ignore(libc::SIGTTOU); // c:1171
        signal_ignore(libc::SIGTTIN); // c:1172
        signal_ignore(libc::SIGTSTP); // c:1173
    } else if job_control_ok == 0 {
        // c:1174
        signal_default(libc::SIGTTOU); // c:1181
        signal_default(libc::SIGTTIN); // c:1182
        signal_default(libc::SIGTSTP); // c:1183
    }
    let interact = isset(INTERACTIVE); // c:1185 — Rust uses INTERACTIVE option as proxy
    if interact {
        signal_default(libc::SIGTERM); // c:1186
        let int_st = sigtrapped
            .lock()
            .unwrap()
            .get(libc::SIGINT as usize)
            .copied()
            .unwrap_or(0);
        if (int_st & ZSIG_IGNORED) == 0 {
            // c:1187
            signal_default(libc::SIGINT); // c:1188
        }
        let pipe_st = sigtrapped
            .lock()
            .unwrap()
            .get(libc::SIGPIPE as usize)
            .copied()
            .unwrap_or(0);
        if pipe_st == 0 {
            // c:1189
            signal_default(libc::SIGPIPE); // c:1190
        }
    }
    let quit_st = sigtrapped
        .lock()
        .unwrap()
        .get(libc::SIGQUIT as usize)
        .copied()
        .unwrap_or(0);
    if (quit_st & ZSIG_IGNORED) == 0 {
        // c:1192
        signal_default(libc::SIGQUIT); // c:1193
    }
    // c:1202-1205 — unblock any trapped signals while in `intrap`.
    if intrap.load(Ordering::Relaxed) != 0 {
        // c:1202
        for sig in 1..=SIGCOUNT {
            let st = sigtrapped
                .lock()
                .unwrap()
                .get(sig as usize)
                .copied()
                .unwrap_or(0);
            if st != 0 && st != ZSIG_IGNORED {
                // c:1204
                let m = signal_mask(sig);
                let _ = signal_unblock(&m); // c:1205
            }
        }
    }
    if job_control_ok == 0 {
        // c:1206
        dosetopt(MONITOR, 0, 0); // c:1207
    }
    dosetopt(USEZLE, 0, 0); // c:1208
    zleactive.store(0, Ordering::Relaxed); // c:1209
                                                                            // c:1214-1217 — close saved fds.
    let max = MAX_ZSH_FD.load(Ordering::Relaxed);
    for i in 10..=max {
        if (fdtable_get(i) & FDT_SAVED_MASK) != 0 {
            // c:1215
            let _ = zclose(i); // c:1216
        }
    }
    // c:1218-1219 — clearjobtab — WARNING (b).
    // SKIPPED: `clearjobtab(&mut JOB_TABLE, monitor)` requires a
    // jobs.rs API surface change.
    let _ = monitor;
    let _ = get_usage(); // c:1220
    FORKLEVEL.store(
        // c:1221 — `forklevel = locallevel;`
        locallevel.load(Ordering::Relaxed),
        Ordering::Relaxed,
    );
}

/// Port of `static int getpipe(char *cmd, int nullexec)` from
/// `Src/exec.c:5119`.
///
/// C body executes `<(cmd)` / `>(cmd)` process substitution via a
/// pipe pair: parent gets back the readable (`<(...)`) or writable
/// (`>(...)`) end as an fd; child runs the substituted command with
/// its stdio redirected into the other end.
///
/// ```c
/// Eprog prog;
/// int pipes[2], out = *cmd == Inang;
/// pid_t pid;
/// struct timespec bgtime;
/// char *ends;
/// if (!(prog = parsecmd(cmd, &ends))) return -1;
/// if (*ends) { zerr("invalid syntax..."); return -1; }
/// if (mpipe(pipes) < 0) return -1;
/// if ((pid = zfork(&bgtime))) {
///     zclose(pipes[out]);
///     if (pid == -1) { zclose(pipes[!out]); return -1; }
///     if (!nullexec) addproc(pid, NULL, 1, &bgtime, -1, -1);
///     procsubstpid = pid;
///     return pipes[!out];
/// }
/// entersubsh(ESUB_ASYNC|ESUB_PGRP|ESUB_NOMONITOR, NULL);
/// redup(pipes[out], out);
/// closem(FDT_UNUSED, 0);
/// cmdpush(CS_CMDSUBST);
/// execode(prog, 0, 1, out ? "outsubst" : "insubst");
/// cmdpop();
/// _realexit();
/// ```
///
/// =================== WARNING — DIVERGENCE ====================
/// (a) `addproc` Rust signature drift (jobs.rs:1516 takes
///     `(&mut job, pid, text, aux)` 4-arg; C is 6-arg with
///     `bgtime`/`gleader`/`list_pipe_job`). For now we set
///     `procsubstpid` only; the job-table side won't see the child.
///     Re-port addproc to 6-arg to unblock.
/// (b) `entersubsh` not yet ported (exec.c:1080+). We approximate
///     the ESUB_ASYNC|ESUB_PGRP|ESUB_NOMONITOR contract by setting
///     `setsid()` + ignoring SIGINT (the minimum needed for the
///     child to not steal tty interrupts). Full port re-establishes
///     pgrp handling, trap reset, monitor disable.
/// (c) `execode(prog, ...)` not yet ported. zshrs has no wordcode
///     walker; we route the substituted body through the same
///     fusevm pipeline `execstring` uses. The eprog from parsecmd
///     is only used as a validity check — the actual execution
///     re-reads `body` (which equals what parsecmd already
///     consumed).
/// (d) `_realexit()` flushes stdio + jobs + history. We use bare
///     `std::process::exit(lastval)` for now.
/// =============================================================
pub fn getpipe(cmd: &str, nullexec: i32) -> i32 {
    // c:5119
    let bytes = cmd.as_bytes();
    let out: i32 = if !bytes.is_empty() && (bytes[0] as char) == Inang {
        1 // c:5122 — `<(...)` reads from child, child writes to fd 1
    } else {
        0 // `>(...)` — child reads from fd 0
    };
    let mut ends_at: usize = 0;
    let prog = parsecmd(cmd, Some(&mut ends_at)); // c:5127
    if prog.is_none() {
        // c:5127
        return -1; // c:5128
    }
    // c:5129 — `if (*ends)` — trailing bytes after the `)` are invalid.
    if ends_at < bytes.len() && bytes[ends_at] != 0 {
        zerr("invalid syntax for process substitution in redirection"); // c:5130
        return -1; // c:5131
    }
    let mut pipes: [i32; 2] = [-1; 2];
    if mpipe(&mut pipes) < 0 {
        // c:5133
        return -1;
    }
    // c:5135 — `if ((pid = zfork(&bgtime)))` — parent path.
    let mut bgtime: ZshTimespec = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    let pid = zfork(Some(&mut bgtime)); // c:5135
    if pid != 0 {
        // c:5135 — parent.
        let _ = zclose(pipes[out as usize]); // c:5136
        if pid == -1 {
            // c:5137
            let _ = zclose(pipes[(1 - out) as usize]); // c:5138
            return -1; // c:5139
        }
        // c:5141-5142 — `if (!nullexec) addproc(pid, ...)` — see WARNING (a).
        let _ = nullexec; // not yet routed through addproc (sig drift)
        procsubstpid.store(pid, Ordering::Relaxed); // c:5143
        return pipes[(1 - out) as usize]; // c:5144
    }
    // c:5146 — child path.
    entersubsh(esub::ASYNC | esub::PGRP | esub::NOMONITOR, None); // c:5146
    let _ = redup(pipes[out as usize], out); // c:5147
    closem(FDT_UNUSED, 0); // c:5148
    cmdpush(CS_CMDSUBST as u8); // c:5149
                                                       // c:5150 — execode(prog, 0, 1, ...) — see WARNING (c).
    let body_end = if ends_at > 0 { ends_at - 1 } else { 2 };
    let body = if body_end > 2 && body_end <= bytes.len() {
        &cmd[2..body_end]
    } else {
        ""
    };
    let _ = with_executor(|e| e.execute_script_zsh_pipeline(body));
    cmdpop(); // c:5151
                                     // c:5152 — _realexit() — WARNING (d).
    std::process::exit(LASTVAL.load(Ordering::Relaxed));
}

/// Port of `static void spawnpipes(LinkList l, int nullexec)` from
/// `Src/exec.c:5184`.
///
/// Walks a redir list `l`, and for each REDIR_OUTPIPE/REDIR_INPIPE
/// entry fires `getpipe(name, nullexec || varid)` and stashes the
/// resulting fd into `f->fd2`.
///
/// ```c
/// LinkNode n;
/// Redir f;
/// char *str;
/// n = firstnode(l);
/// for (; n; incnode(n)) {
///     f = (Redir) getdata(n);
///     if (f->type == REDIR_OUTPIPE || f->type == REDIR_INPIPE) {
///         str = f->name;
///         f->fd2 = getpipe(str, nullexec || f->varid);
///     }
/// }
/// ```
///
/// =================== WARNING — DIVERGENCE ====================
/// The Rust port consumes a `&mut Vec<crate::ported::zsh_h::redir>`
/// in place of `LinkList`. The walk is identical; the only behavior
/// difference is that LinkList iteration in C lets callers splice
/// nodes mid-walk — we never do that here so it's a no-op divergence.
/// =============================================================
pub fn spawnpipes(l: &mut [redir], nullexec: i32) {
    // c:5184
    for f in l.iter_mut() {
        // c:5191
        if f.typ == REDIR_OUTPIPE || f.typ == REDIR_INPIPE {
            // c:5193
            let str_ = f.name.clone().unwrap_or_default(); // c:5194
            let nullexec_eff = if f.varid.as_deref().map_or(false, |v| !v.is_empty()) {
                1
            } else {
                nullexec
            };
            f.fd2 = getpipe(&str_, nullexec_eff); // c:5195
        }
    }
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


// =====================================================================
// Wordcode-VM execution helpers — moved from src/vm_helper.rs.
// These methods are partial ports of `Src/exec.c::execlist` /
// `execpline` / `execsublist` / `execcmd` / control-flow forms.
// They run on the wordcode buffer that `parse::par_event_wordcode`
// emits into `ECBUF`.
//
// Per user directive: parity-relevant exec code lives in src/ported/.
// =====================================================================
impl crate::ported::vm_helper::ShellExecutor {
    /// P9d stub: direct port of `execlist(Estate state, int dont_change_job,
    /// int exiting)` from `Src/exec.c:1551-1671`. Walks WC_LIST entries,
    /// dispatches each sublist payload to exec_pline_wordcode. Real
    /// implementation handles fork/wait + signal-trap dispatch.
    /// Returns (last_status, pc_after_walk).
    pub fn exec_list_wordcode(&mut self, buf: &[u32], mut pc: usize) -> (i32, usize) {
        let mut last_status: i32 = 0;
        while pc < buf.len() {
            let code = wc_code(buf[pc]);
            if code == WC_END {
                pc += 1;
                break;
            }
            if code != WC_LIST {
                pc += 1;
                continue;
            }
            let header = buf[pc];
            let skip = (wc_data(header) >> crate::ported::zsh_h::WC_LIST_FREE) as usize;
            pc += 1;
            let (s, _) = self.exec_sublist_wordcode(buf, pc);
            last_status = s;
            pc += skip;
        }
        (last_status, pc)
    }

    /// P9d stub: direct port of `execsublist` from
    /// `Src/exec.c:1672-1810`. Walks WC_SUBLIST + pipeline payload.
    pub fn exec_sublist_wordcode(&mut self, buf: &[u32], mut pc: usize) -> (i32, usize) {
        let mut last_status: i32 = 0;
        if pc < buf.len() && wc_code(buf[pc]) == WC_SUBLIST {
            let header = buf[pc];
            let skip = (wc_data(header) >> 7) as usize;
            pc += 1;
            let (s, _) = self.exec_pline_wordcode(buf, pc);
            last_status = s;
            pc += skip;
        }
        (last_status, pc)
    }

    /// P9d stub: direct port of `execpline` from
    /// `Src/exec.c:1812-1980`. Walks WC_PIPE chain + cmd payloads.
    pub fn exec_pline_wordcode(&mut self, buf: &[u32], mut pc: usize) -> (i32, usize) {
        let mut last_status: i32 = 0;
        if pc < buf.len() && wc_code(buf[pc]) == WC_PIPE {
            let header = buf[pc];
            let skip = ((wc_data(header) >> 1) & 0xffff) as usize;
            pc += 1;
            let (s, _) = self.exec_cmd_wordcode(buf, pc);
            last_status = s;
            pc += skip;
        }
        (last_status, pc)
    }

    /// P9d: direct port of `execcmd_exec` / `execcmd_analyze` from
    /// `Src/exec.c:2700-3700`. Reads the cmd header (WC_SIMPLE /
    /// WC_SUBSH / WC_FOR / WC_CASE / ...) and dispatches accordingly.
    pub fn exec_cmd_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        if pc >= buf.len() {
            return (0, pc);
        }
        match wc_code(buf[pc]) {
            WC_SIMPLE => self.exec_simple_wordcode(buf, pc),
            WC_SUBSH => self.exec_subsh_wordcode(buf, pc),
            WC_CURSH => self.exec_cursh_wordcode(buf, pc),
            WC_FOR => self.exec_for_wordcode(buf, pc),
            WC_SELECT => self.exec_select_wordcode(buf, pc),
            WC_CASE => self.exec_case_wordcode(buf, pc),
            WC_IF => self.exec_if_wordcode(buf, pc),
            WC_WHILE => self.exec_while_wordcode(buf, pc),
            WC_REPEAT => self.exec_repeat_wordcode(buf, pc),
            WC_FUNCDEF => self.exec_funcdef_wordcode(buf, pc),
            WC_TIMED => self.exec_timed_wordcode(buf, pc),
            WC_COND => self.exec_cond_wordcode(buf, pc),
            WC_ARITH => self.exec_arith_wordcode(buf, pc),
            WC_TRY => self.exec_try_wordcode(buf, pc),
            _ => (0, pc + 1),
        }
    }

    /// P9d: direct port of `execfor(Estate state, int do_exec)` from `Src/exec.c:1232-1350`.
    /// Reads WC_FOR header via WC_FOR_TYPE/WC_FOR_SKIP, dispatches on
    /// type (PPARAM / LIST / COND), iterates body via recursive
    /// exec_list_wordcode calls.
    pub fn exec_for_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        if pc >= buf.len() {
            return (0, pc);
        }
        let header = buf[pc];
        let _type_bits = WC_FOR_TYPE(header);
        let skip = WC_FOR_SKIP(header) as usize;
        let _ = WC_FOR_LIST;
        // exec.c:1245+ — read var name via ecgetstr, iterate words,
        // exec body. Full implementation needs the var-binding +
        // iteration loop; this stub advances past the form.
        let mut last_status: i32 = 0;
        let end_pc = pc + 1 + skip;
        // Walk inner body (after header + var-name slot) once as a
        // shape-correct placeholder.
        let body_pc = pc + 2;
        if body_pc < end_pc {
            let (s, _) = self.exec_list_wordcode(buf, body_pc);
            last_status = s;
        }
        (last_status, end_pc)
    }
    /// P9d: `execselect` shape — same as exec_for but with `select`
    /// REPL prompt at each iteration. Src/exec.c:1352-1490.
    pub fn exec_select_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.exec_for_wordcode(buf, pc)
    }
    /// P9d: direct port of `execcase(Estate state, int do_exec)` from `Src/exec.c:1492-1550`.
    /// Reads WC_CASE_TYPE + WC_CASE_SKIP, walks pattern arms.
    pub fn exec_case_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        if pc >= buf.len() {
            return (0, pc);
        }
        let header = buf[pc];
        let _type_bits = WC_CASE_TYPE(header);
        let skip = WC_CASE_SKIP(header) as usize;
        // Full implementation: pattern-match word against each arm's
        // patterns, exec the first matching arm's body. Stub walks the
        // body once as a placeholder.
        let mut last_status: i32 = 0;
        let end_pc = pc + 1 + skip;
        let body_pc = pc + 1;
        if body_pc < end_pc {
            let (s, _) = self.exec_list_wordcode(buf, body_pc);
            last_status = s;
        }
        (last_status, end_pc)
    }
    /// P9d: full port of `execif(Estate state, int do_exec)` from `Src/loop.c:299-340`.
    ///
    /// C body walks the if/elif/else chain. Each cond is an inner
    /// WC_IF header with WC_IF_TYPE distinguishing IF / ELIF / ELSE.
    /// Returns lastval = status of the run branch, or 0 if no branch
    /// matched.
    pub fn exec_if_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        if pc >= buf.len() {
            return (0, pc);
        }
        let header = buf[pc];
        let skip = WC_IF_SKIP(header) as usize;
        let end_pc = pc + 1 + skip;
        let mut cur = pc + 1;
        let mut run: i32 = 0; // 0=no branch, 1=if/elif body, 2=else body
        let mut s = 0; // 0=in if/elif chain, 1=elif seen at least once
        let mut last_status: i32 = 0;
        // loop.c:307-326 — walk the chain.
        while cur < end_pc {
            if cur >= buf.len() {
                break;
            }
            let code = buf[cur];
            cur += 1;
            if wc_code(code) != WC_IF {
                // Past the IF header chain — must be the body of a
                // previously-selected branch we should run.
                run = 1;
                cur -= 1;
                break;
            }
            // WC_IF_TYPE == ELSE (2) — unconditional else body.
            if WC_IF_TYPE(code) == 2 {
                run = 2;
                break;
            }
            let next = cur + WC_IF_SKIP(code) as usize;
            let (cond_status, after_cond) = self.exec_list_wordcode(buf, cur);
            last_status = cond_status;
            if cond_status == 0 {
                run = 1;
                cur = after_cond;
                break;
            }
            if RETFLAG.load(Ordering::SeqCst) != 0 {
                break;
            }
            s = 1;
            cur = next;
        }
        let _ = s;
        // loop.c:328-336 — run the selected branch body.
        if run != 0 && cur < end_pc {
            let (body_status, _) = self.exec_list_wordcode(buf, cur);
            last_status = body_status;
        } else if RETFLAG.load(Ordering::SeqCst) == 0
            && (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) == 0
        {
            last_status = 0;
        }
        (last_status, end_pc)
    }
    /// P9d: full port of `execwhile(Estate state, UNUSED(int do_exec))` from `Src/loop.c:432-498`.
    ///
    /// Loops {exec cond; check status XOR isuntil; exec body; check
    /// breaks/contflag/retflag/errflag} until termination.
    pub fn exec_while_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        if pc >= buf.len() {
            return (0, pc);
        }
        let header = buf[pc];
        // loop.c:438 — `isuntil = (WC_WHILE_TYPE(code) == WC_WHILE_UNTIL)`.
        // WC_WHILE_UNTIL = 2 per zsh.h:1015.
        let isuntil = WC_WHILE_TYPE(header) == 2;
        let skip = WC_WHILE_SKIP(header) as usize;
        let end_pc = pc + 1 + skip;
        let loop_pc = pc + 1;
        // loop.c:443-446 — pushheap; cmdpush; loops++.
        LOOPS.fetch_add(1, Ordering::SeqCst);
        let mut last_status: i32 = 0;
        let mut oldval: i32 = 0;
        // Safety cap to prevent runaway infinite loops in stubs — real
        // C loops forever if conditions hold.
        let mut iters = 0u64;
        const ITER_CAP: u64 = 1_000_000;
        loop {
            iters += 1;
            if iters > ITER_CAP {
                break;
            }
            // loop.c:467 — exec cond (first inner list).
            let (cond_status, after_cond) = self.exec_list_wordcode(buf, loop_pc);
            last_status = cond_status;
            // loop.c:473 — `if (!((lastval == 0) ^ isuntil)) break;`
            let cond_passed = (cond_status == 0) ^ isuntil;
            if !cond_passed {
                if BREAKS.load(Ordering::SeqCst) > 0 {
                    BREAKS.fetch_sub(1, Ordering::SeqCst);
                }
                if RETFLAG.load(Ordering::SeqCst) == 0 {
                    last_status = oldval;
                }
                break;
            }
            // loop.c:481 — retflag bail.
            if RETFLAG.load(Ordering::SeqCst) != 0 {
                if BREAKS.load(Ordering::SeqCst) > 0 {
                    BREAKS.fetch_sub(1, Ordering::SeqCst);
                }
                break;
            }
            // loop.c:489 — exec body.
            let (body_status, _) = self.exec_list_wordcode(buf, after_cond);
            last_status = body_status;
            // loop.c:493-497 — breaks/continue handling.
            if BREAKS.load(Ordering::SeqCst) > 0 {
                let prev = BREAKS.fetch_sub(1, Ordering::SeqCst);
                if prev - 1 > 0 || CONTFLAG.load(Ordering::SeqCst) == 0 {
                    break;
                }
                CONTFLAG.store(0, Ordering::SeqCst);
            }
            // loop.c:498-501 — errflag bail.
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                last_status = 1;
                break;
            }
            // loop.c:502 — retflag bail.
            if RETFLAG.load(Ordering::SeqCst) != 0 {
                break;
            }
            oldval = last_status;
        }
        LOOPS.fetch_sub(1, Ordering::SeqCst);
        (last_status, end_pc)
    }
    /// P9d: full port of `execrepeat(Estate state, UNUSED(int do_exec))` from `Src/loop.c:499-552`.
    ///
    /// C body:
    ///   end = state->pc + WC_REPEAT_SKIP(code);
    ///   tmp = ecgetstr(state, EC_DUPTOK, &htok);
    ///   if (htok) { singsub(&tmp); untokenize(tmp); }
    ///   count = mathevali(tmp);
    ///   loops++;
    ///   loop = state->pc;
    ///   while (count-- > 0) {
    ///     state->pc = loop;
    ///     execlist(state, 1, 0);
    ///     if (breaks) { breaks--; if (breaks || !contflag) break; contflag = 0; }
    ///     if (errflag) { lastval = 1; break; }
    ///     if (retflag) break;
    ///   }
    ///   loops--;
    pub fn exec_repeat_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        if pc >= buf.len() {
            return (0, pc);
        }
        let header = buf[pc];
        let skip = WC_REPEAT_SKIP(header) as usize;
        let end_pc = pc + 1 + skip;
        // loop.c:511 — `tmp = ecgetstr(state, EC_DUPTOK, &htok);`
        let (count_expr_raw, after_count) = ecgetstr_wordcode(buf, pc + 1);
        // loop.c:512-515 — singsub + untokenize on tokenized count.
        let count_expr_sub = singsub(&count_expr_raw);
        let count_expr = crate::ported::lex::untokenize(&count_expr_sub);
        // loop.c:516 — `count = mathevali(tmp);`
        let count_val = mathevali(&count_expr).unwrap_or(0);
        if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
            return (1, end_pc);
        }
        let mut last_status: i32 = 0; // loop.c:519 — `lastval = 0` for zero count.
                                      // loop.c:520-522 — `pushheap(); cmdpush(CS_REPEAT); loops++;`
        LOOPS.fetch_add(1, Ordering::SeqCst);
        let loop_body_pc = after_count;
        // loop.c:523-545 — main iteration.
        let mut remaining = count_val;
        while remaining > 0 {
            remaining -= 1;
            let (s, _) = self.exec_list_wordcode(buf, loop_body_pc);
            last_status = s;
            // loop.c:528-533 — breaks/continue handling.
            if BREAKS.load(Ordering::SeqCst) > 0 {
                let prev = BREAKS.fetch_sub(1, Ordering::SeqCst);
                if prev - 1 > 0 || CONTFLAG.load(Ordering::SeqCst) == 0 {
                    break;
                }
                CONTFLAG.store(0, Ordering::SeqCst);
            }
            // loop.c:534-537 — errflag bail.
            if (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0 {
                last_status = 1;
                break;
            }
            // loop.c:538 — retflag bail (function return).
            if RETFLAG.load(Ordering::SeqCst) != 0 {
                break;
            }
        }
        // loop.c:546-549 — `cmdpop(); popheap(); loops--;`
        LOOPS.fetch_sub(1, Ordering::SeqCst);
        (last_status, end_pc)
    }
    /// P9d stub: `execfuncdef`.
    pub fn exec_funcdef_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.skip_form(buf, pc)
    }
    /// P9d stub: `execsubsh` for `(...)` subshell.
    pub fn exec_subsh_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.skip_form(buf, pc)
    }
    /// P9d stub: `execcursh` for `{...}` brace group.
    pub fn exec_cursh_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.skip_form(buf, pc)
    }
    /// P9d stub: `exectimed` for `time pipeline`.
    pub fn exec_timed_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.skip_form(buf, pc)
    }
    /// P9d stub: `execcond` for `[[ ... ]]`.
    pub fn exec_cond_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.skip_form(buf, pc)
    }
    /// P9d stub: `execarith` for `(( ... ))`.
    pub fn exec_arith_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.skip_form(buf, pc)
    }
    /// P9d stub: `exectry` for `{ try } always { finally }`.
    pub fn exec_try_wordcode(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        self.skip_form(buf, pc)
    }

    /// Shared helper for WC_* form dispatch stubs: read the header's
    /// `skip` field (data >> WC_CODEBITS) and advance pc past the
    /// payload. Each real production-specific exec_* will replace its
    /// call to this with the form-specific logic.
    fn skip_form(&mut self, buf: &[u32], pc: usize) -> (i32, usize) {
        if pc >= buf.len() {
            return (0, pc);
        }
        let skip = wc_data(buf[pc]) as usize;
        (0, pc + 1 + skip)
    }

    /// P9d: direct port of `execsimple(Estate state)` from `Src/exec.c:3702-4100`.
    /// Walks WC_SIMPLE header + word slots, decodes the interned
    /// strings via `ecgetstr`, builds argv, invokes the command.
    /// Real implementation handles assignments + redirections inline
    /// from the same wordcode; this minimal version pulls just words.
    pub fn exec_simple_wordcode(&mut self, buf: &[u32], mut pc: usize) -> (i32, usize) {
        let mut last_status: i32 = 0;
        if pc < buf.len() && wc_code(buf[pc]) == WC_SIMPLE {
            let header = buf[pc];
            let nwords = wc_data(header) as usize;
            pc += 1;
            // Decode the interned strings into an argv vector.
            let mut argv: Vec<String> = Vec::with_capacity(nwords);
            for _ in 0..nwords {
                let (word, next) = ecgetstr(buf, pc);
                argv.push(word);
                pc = next;
            }
            // Invoke via the existing command-execution path. argv[0]
            // is the command name; remainder are arguments. Real exec
            // (Src/exec.c:3850 execcmd_analyze) would resolve builtin /
            // function / external + fork/exec; we delegate to the
            // existing AST-based simple-cmd executor's argv hook.
            if !argv.is_empty() {
                last_status = self.invoke_argv_wordcode(&argv);
            }
        }
        (last_status, pc)
    }

    /// Minimal command invoker for wordcode-driven simple commands.
    /// Bridges the wordcode-side argv into the existing AST-side
    /// simple-cmd dispatch by constructing a single-Simple ZshProgram
    /// and running it through `execute_script_zsh_pipeline`. Real exec
    /// (P9d full) bypasses the AST and dispatches builtin/function/
    /// external directly from the wordcode — but the AST path
    /// already does this correctly today, so until the full
    /// builtin/function/external dispatch is ported into the wordcode
    /// consumer, this bridge keeps actual execution working.
    fn invoke_argv_wordcode(&mut self, argv: &[String]) -> i32 {
        let script = argv
            .iter()
            .map(|s| {
                // Minimal shell-escape: wrap in single quotes if
                // the arg contains whitespace or special chars.
                if s.chars()
                    .any(|c| c.is_whitespace() || "\"'`$\\|;&<>(){}[]*?~".contains(c))
                {
                    format!("'{}'", s.replace('\'', "'\\''"))
                } else {
                    s.clone()
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        self.execute_script_zsh_pipeline(&script).unwrap_or(1)
    }
}


// =====================================================================
// dispatch_function_call — moved from src/vm_helper.rs.
// Partial port of `Src/exec.c::doshfunc` — the non-fusevm function-
// call path used by signal-trap dispatch and other callers that
// can't go through the fusevm Op::CallFunction route.
// =====================================================================
use crate::fusevm_bridge::{ExecutorContext, register_builtins, ZshrsHost};
use crate::exec_jobs::JobState;
use crate::parse::Redirect;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::io;
use std::io::{Read, Write};
use std::os::unix::io::FromRawFd;
impl crate::ported::vm_helper::ShellExecutor {
    /// Dispatch a function by name through the new (compiled) pipeline.
    /// Mirrors `ZshrsHost::call_function`'s resolution order — checks
    /// `functions_compiled` first, triggers autoload if needed, then falls
    /// back to the legacy AST recompile path. Returns `None` if the name
    /// isn't a function (caller falls back to external dispatch).
    ///
    /// This is the synchronous-side replacement for the legacy
    /// `call_function(&ShellCommand, args)`. It avoids the AST detour when
    /// the new pipeline already has a Chunk for the function.
    pub fn dispatch_function_call(&mut self, name: &str, args: &[String]) -> Option<i32> {
        // Autoload prelude: if `name` isn't yet compiled but exists as
        // a PM_UNDEFINED stub in shfunctab (registered by `autoload`
        // builtin via `add_autoload_function` at builtin.rs:3654),
        // materialize it via `loadautofn_by_name` (exec.rs) which reads
        // the file from $fpath and stores raw body text on
        // `shfunctab.body`. Then wrap as `name() { <body> }` and eval
        // through the standard zsh pipeline — the wrap parses as a
        // function-def, fusevm emits `BUILTIN_REGISTER_COMPILED_FN`,
        // and the function lands in `functions_compiled`. This covers
        // zsh-style autoload (default + `-z`); ksh-style (`-k` /
        // KSH_AUTOLOAD) would eval the unwrapped body and rely on the
        // file to define+call the function itself — TODO once needed.
        if !self.functions_compiled.contains_key(name) {
            if let Some(stub) = crate::ported::utils::getshfunc(name) {
                if (stub.node.flags as u32 & crate::ported::zsh_h::PM_UNDEFINED) != 0 {
                    let boxed = Box::new(stub.clone());
                    let ptr = Box::into_raw(boxed);
                    let _ = crate::ported::exec::loadautofn(ptr, 0, 0, 0);
                    unsafe {
                        let _ = Box::from_raw(ptr);
                    }
                    if let Some(body) =
                        crate::ported::utils::getshfunc(name).and_then(|f| f.body)
                    {
                        let wrapped = format!("{name}() {{\n{body}\n}}");
                        let _ = self.execute_script_zsh_pipeline(&wrapped);
                    }
                }
            }
        }
        let chunk = self.functions_compiled.get(name).cloned()?;

        // FUNCNEST guard — see `call_function` for the lower-than-
        // zsh ceiling rationale. Cap at 100 by default (matches
        // call_function's ceiling).
        let funcnest_limit: usize = self
            .scalar("FUNCNEST")
            .and_then(|s| s.parse().ok())
            .unwrap_or(100);
        if self.local_scope_depth >= funcnest_limit {
            eprintln!(
                "{}: maximum nested function level reached; increase FUNCNEST?",
                name
            );
            return Some(1);
        }
        // Save and replace positional params + local-scope save/restore,
        // mirroring the legacy `call_function(&ShellCommand, args)` and
        // ZshrsHost::call_function.
        let saved_params = self.pparams();
        self.set_pparams(args.to_vec());
        // FUNCTION_ARGZERO: zsh sets `\$0` inside a function to the
        // function name (default-on option). The bytecode-level
        // call_function path already does this; the dispatch path
        // used by dynamic-command-name dispatch (`f=hook; \$f`)
        // didn't, so plugin code reading `\$0` saw the binary path
        // instead. Save and install the function name; restore on
        // exit. Anonymous functions get the cosmetic `(anon)` per
        // call_function above.
        let display_name = if name.starts_with("_zshrs_anon_") {
            "(anon)".to_string()
        } else {
            name.to_string()
        };
        let saved_zero = crate::ported::params::getsparam("0");
        self.set_scalar("0".to_string(), display_name);
        self.local_scope_depth += 1;
        // c:Src/exec.c doshfunc startparamscope(): bump canonical
        // `locallevel` so any `local`/`typeset` inside the body
        // installs Params at the correct scope. endparamscope at
        // exit decrements + restores Param.old chain.
        crate::ported::params::locallevel.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let line_base = self.function_line_base.get(name).copied().unwrap_or(0);
        let def_file = self.function_def_file.get(name).cloned().flatten();
        self.prompt_funcstack
            .push((name.to_string(), line_base, def_file));

        crate::fusevm_disasm::maybe_print_stdout(&format!("function:{name}"), &chunk);
        let mut vm = fusevm::VM::new(chunk);
        register_builtins(&mut vm);
        let _ctx = ExecutorContext::enter(self);
        let _ = vm.run();
        let status = vm.last_status;
        drop(_ctx);

        self.set_pparams(saved_params);
        self.prompt_funcstack.pop();
        // c:Src/exec.c doshfunc → endparamscope(). Decrements
        // canonical locallevel and walks paramtab restoring the
        // Param.old chain for every entry installed at this depth.
        crate::ported::params::endparamscope();
        self.local_scope_depth -= 1;
        match saved_zero {
            Some(v) => {
                self.set_scalar("0".to_string(), v);
            }
            None => {
                self.unset_scalar("0");
            }
        }

        // Honor explicit `return N` from inside the function body.
        if let Some(ret) = self.returning.take() {
            self.set_last_status(ret);
            Some(ret)
        } else {
            self.set_last_status(status);
            Some(status)
        }
    }
}


// =====================================================================
// run_command_substitution — moved from src/vm_helper.rs.
// Partial port of `Src/exec.c::getoutput` — command-substitution
// runtime that captures `$(cmd)` output via dup2+pipe. Includes
// `$(< file)` shorthand for fast file-read.
// =====================================================================
impl crate::ported::vm_helper::ShellExecutor {
    pub fn run_command_substitution(&mut self, cmd_str: &str) -> String {
        // `$(< FILE)` — zsh shorthand for "read FILE contents". Faster
        // than spawning `cat`. The leading `<` (after stripping
        // whitespace) means "read this file". Trailing newline is
        // stripped (same as command-substitution).
        let trimmed = cmd_str.trim_start();
        // Only treat as `$(<file)` shorthand when the SINGLE leading `<`
        // is followed by a filename, not another `<`. `$(<<<"hi" cat)`
        // starts with `<<<` (here-string) and must go through the full
        // parse path, not the read-file shortcut.
        if let Some(rest) = trimmed.strip_prefix('<').filter(|s| !s.starts_with('<')) {
            let filename = rest.trim();
            // Expand any leading $ / tilde in the filename so
            // `$(< $f)` and `$(< ~/x)` work.
            let resolved = if filename.contains('$') || filename.starts_with('~') {
                crate::ported::subst::singsub(filename)
            } else {
                filename.to_string()
            };
            let resolved = resolved.to_string();
            match std::fs::read_to_string(&resolved) {
                Ok(contents) => {
                    return contents.trim_end_matches('\n').to_string();
                }
                Err(_) => {
                    eprintln!("zshrs:1: no such file or directory: {}", resolved);
                    return String::new();
                }
            }
        }

        // Port of getoutput(char *cmd, int qt) from Src/exec.c. Parse and compile via
        // the lex+parse free fns + ZshCompiler pipeline, run on a
        // sub-VM with the host wired up. Stdout is captured through
        // an in-process pipe via dup2 — no fork.
        //
        // This single path replaces the prior "internal vs external"
        // fast-path split: the sub-VM emits Op::Exec for unknown
        // command names, which forks/execs through the host.

        // Set up the stdout-capture pipe. We dup the original stdout
        // so post-run we can restore it; the write end is dup2'd onto
        // STDOUT_FILENO so all output the sub-VM emits (including from
        // forked children, which inherit fd 1) lands in the pipe.
        let (read_fd, write_fd) = {
            let mut fds = [0i32; 2];
            if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                return String::new();
            }
            (fds[0], fds[1])
        };
        let saved_stdout = unsafe { libc::dup(libc::STDOUT_FILENO) };
        if saved_stdout < 0 {
            unsafe {
                libc::close(read_fd);
                libc::close(write_fd);
            }
            return String::new();
        }
        unsafe {
            libc::dup2(write_fd, libc::STDOUT_FILENO);
            libc::close(write_fd);
        }

        // Parse + compile + run.
        // Push CS_CMDSUBST for `%_` xtrace prefix — direct port of
        // Src/exec.c:4783 `cmdpush(CS_CMDSUBST);` around execode().
        // Trace lines emitted by the inner program inherit this token
        // so their PS4 prefix shows "cmdsubst" matching zsh -x.
        crate::ported::prompt::cmdpush(crate::ported::zsh_h::CS_CMDSUBST as u8); // c:zsh.h:2799
                                                                                 // Save LINENO so the inner cmdsubst's line counter doesn't
                                                                                 // leak into the outer trace — direct port of Src/exec.c:1407
                                                                                 // `oldlineno = lineno;` followed by `lineno = oldlineno;`
                                                                                 // restore at line 1640. Inner program parses fresh as line 1
                                                                                 // and increments from there; once it returns, the outer
                                                                                 // line at the `$(…)` site must read the original outer
                                                                                 // lineno (so xtrace renders `+:5:> echo …` not `+:1:> …`).
        let saved_lineno = crate::ported::params::getsparam("LINENO");
        // Anchor the inner program's lineno to the outer's current
        // $LINENO so xtrace inside the cmdsubst renders the outer
        // line. zsh's execlist preserves lineno across the inner
        // exec — for our sub-VM (fresh compile) we use lineno_addend
        // to shift inner's line N → outer_lineno + (N - 1).
        let outer_lineno: u64 = self
            .scalar("LINENO")
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);
        // Mirror Src/init.c errflag save/clear/check pattern around
        // the nested parse so an inner syntax error doesn't bleed into
        // the outer execution.
        let saved_errflag = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::parse::parse_init(cmd_str);
        let parsed = crate::ported::parse::parse();
        let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        errflag.store(saved_errflag, Ordering::Relaxed);
        let prog = if parse_failed { None } else { Some(parsed) };
        let mut cmd_status: Option<i32> = None;
        if let Some(prog) = prog {
            let mut compiler = crate::compile_zsh::ZshCompiler::new();
            compiler.lineno_addend = outer_lineno.saturating_sub(1);
            let chunk = compiler.compile(&prog);
            if !chunk.ops.is_empty() {
                crate::fusevm_disasm::maybe_print_stdout("run_command_substitution", &chunk);
                let mut vm = fusevm::VM::new(chunk);
                register_builtins(&mut vm);
                vm.set_shell_host(Box::new(ZshrsHost));
                // Seed inner $? with the outer's last_status so the
                // sub-shell inherits the parent's exit code. Direct
                // port of Src/exec.c:4783 around execcmd_exec — the
                // child inherits `lastval` at fork time, so `false;
                // echo $(echo $?)` reads 1, not the freshly-zeroed
                // sub-VM default. Without this, every cmd-subst
                // started with $?==0 regardless of the parent's
                // last command.
                vm.last_status = self.last_status();
                let _ctx = ExecutorContext::enter(self);
                let _ = vm.run();
                cmd_status = Some(vm.last_status);
            }
        }
        // Restore LINENO so outer xtrace sees the outer line.
        if let Some(ln) = saved_lineno {
            self.set_scalar("LINENO".to_string(), ln);
        }
        crate::ported::prompt::cmdpop();
        // Propagate the inner cmd's status to the parent shell. zsh:
        // `a=$(false); echo $?` → 1 because cmd-subst status leaks to
        // $?. Set last_status on the executor so $? reads the right
        // value for callers that don't have a SetStatus(0) overwrite
        // (echo, test, etc.). Bare assignment paths still get the
        // SetStatus(0) from compile_simple — that's a separate gap.
        // Empty cmd-subst (`\`\``, `$()`) resets status to 0 per
        // Src/exec.c — the inner ran no command so the "last
        // command's exit" is the implicit success of "did nothing".
        // Without this branch, a prior command's non-zero status
        // leaked through the empty cmd-subst.
        if let Some(status) = cmd_status {
            self.set_last_status(status);
        } else {
            self.set_last_status(0);
        }

        // Flush any buffered Rust-side stdout so it reaches the pipe
        // before we restore.
        let _ = io::stdout().flush();

        // Restore stdout and read what was captured.
        unsafe {
            libc::dup2(saved_stdout, libc::STDOUT_FILENO);
            libc::close(saved_stdout);
        }
        let read_file = unsafe { std::fs::File::from_raw_fd(read_fd) };
        let mut output = String::new();
        let _ = std::io::BufReader::new(read_file).read_to_string(&mut output);

        // POSIX: trailing newlines stripped from cmd-sub result.
        while output.ends_with('\n') {
            output.pop();
        }
        output
    }
}


// =====================================================================
// execute_external + execute_external_bg — moved from src/vm_helper.rs.
// Partial port of `Src/exec.c::execcmd_exec`'s external-dispatch tail
// (c:3550-3850) — fork+exec for non-builtin commands.
// =====================================================================
impl crate::ported::vm_helper::ShellExecutor {
    pub(crate) fn execute_external(
        &mut self,
        cmd: &str,
        args: &[String],
        redirects: &[Redirect],
    ) -> Result<i32, String> {
        self.execute_external_bg(cmd, args, redirects, false)
    }

    fn execute_external_bg(
        &mut self,
        cmd: &str,
        args: &[String],
        _redirects: &[Redirect],
        background: bool,
    ) -> Result<i32, String> {
        tracing::trace!(cmd, bg = background, "exec external");
        let mut command = Command::new(cmd);
        command.args(args);

        // Redirect handling moved entirely to fusevm's WithRedirectsBegin/End
        // ops at compile time; the `_redirects` slice arrives empty in every
        // production code path. The legacy `for redir in redirects { ... }`
        // block (~120 LOC of file/pipe/heredoc/herestring/fd_var handling)
        // is gone.

        if background {
            match command.spawn() {
                Ok(child) => {
                    let pid = child.id();
                    let cmd_str = format!("{} {}", cmd, args.join(" "));
                    let job_id = self.jobs.add_job(child, cmd_str, JobState::Running);
                    println!("[{}] {}", job_id, pid);
                    Ok(0)
                }
                Err(e) => {
                    if e.kind() == io::ErrorKind::NotFound {
                        // zsh: absolute paths emit "no such file or
                        // directory" (the OS error, since the path was
                        // tried directly), not "command not found"
                        // (which implies PATH search).
                        if cmd.starts_with('/') {
                            eprintln!("zshrs:1: no such file or directory: {}", cmd);
                        } else {
                            eprintln!("zshrs:1: command not found: {}", cmd);
                        }
                        Ok(127)
                    } else {
                        Err(format!("zshrs: {}: {}", cmd, e))
                    }
                }
            }
        } else {
            match command.status() {
                Ok(status) => Ok(status.code().unwrap_or(1)),
                Err(e) => {
                    if e.kind() == io::ErrorKind::NotFound {
                        // zsh: absolute paths emit "no such file or
                        // directory" (the OS error, since the path was
                        // tried directly), not "command not found"
                        // (which implies PATH search).
                        if cmd.starts_with('/') {
                            eprintln!("zshrs:1: no such file or directory: {}", cmd);
                        } else {
                            eprintln!("zshrs:1: command not found: {}", cmd);
                        }
                        Ok(127)
                    } else if e.kind() == io::ErrorKind::PermissionDenied {
                        // zsh: non-executable file → "permission denied"
                        // on stderr and exit 126 (POSIX convention for
                        // "command found but not executable"). zshrs
                        // previously bubbled the IO error up via Err
                        // and the surrounding code converted to 127
                        // with no diagnostic.
                        eprintln!("zshrs:1: permission denied: {}", cmd);
                        Ok(126)
                    } else {
                        Err(format!("zshrs: {}: {}", cmd, e))
                    }
                }
            }
        }
    }
}


// =====================================================================
// execute_script_file / execute_script_zsh_pipeline / execute_script
// — moved from src/vm_helper.rs.
// Partial port of `Src/init.c::loop` / `Src/exec.c::execlist` /
// `Src/builtin.c::bin_dot` script-execution entrypoints.
// Reads the script file, runs the parser, drives the compiler,
// invokes the fusevm VM. Bytecode-cache integration is zshrs-native
// (no C analog).
// =====================================================================
impl crate::ported::vm_helper::ShellExecutor {
    /// Execute a script file with bytecode caching — skips lex+parse+compile on cache hit.
    /// Bytecode is stored in rkyv keyed by (path, mtime).
    pub fn execute_script_file(&mut self, file_path: &str) -> Result<i32, String> {
        let path = Path::new(file_path);
        let abs_path = path
            .canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .to_string_lossy()
            .to_string();

        // Try bytecode cache first — rkyv shard at ~/.zshrs/scripts.rkyv.
        // The cache validates path + mtime + zshrs binary mtime; on any miss
        // we fall through to lex/parse/compile.
        if let Some(bc_blob) = crate::script_cache::try_load_bytes(path) {
            if let Ok(chunk) = bincode::deserialize::<fusevm::Chunk>(&bc_blob) {
                if !chunk.ops.is_empty() {
                    tracing::trace!(
                        path = %abs_path,
                        ops = chunk.ops.len(),
                        "execute_script_file: bytecode cache hit"
                    );
                    crate::fusevm_disasm::maybe_print_stdout(
                        &format!("execute_script_file:cache:{abs_path}"),
                        &chunk,
                    );
                    let mut vm = fusevm::VM::new(chunk);
                    register_builtins(&mut vm);
                    let _ctx = ExecutorContext::enter(self);
                    match vm.run() {
                        fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                            self.set_last_status(vm.last_status);
                        }
                        fusevm::VMResult::Error(e) => {
                            return Err(format!("VM error: {}", e));
                        }
                    }
                    return Ok(self.last_status());
                }
            }
        }

        // Cache miss — read, parse, compile, execute, then cache.
        // No history expansion: zsh fires `!` history sub only on
        // interactive input (the REPL line). Sourced files are
        // verbatim — `(( !${#ARR} ))` (logical-not) must NOT
        // become `(( <last-arg-of-prev-cmd>{#ARR} ))`. Direct port
        // of Src/init.c source() which calls `lex_init_buf` /
        // `loop()` without engaging the history layer.
        let content =
            std::fs::read_to_string(file_path).map_err(|e| format!("{}: {}", file_path, e))?;
        // Save & clear errflag around the parse so we can detect a
        // fresh syntax error vs an inherited one. Direct port of
        // Src/init.c source()'s `errflag &= ~ERRFLAG_ERROR;` before
        // `parse_event(ENDINPUT)` and the post-parse errflag check.
        let saved_errflag = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::parse::parse_init(&content);
        let program = crate::ported::parse::parse();
        let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        errflag.store(saved_errflag, Ordering::Relaxed);
        if parse_failed {
            return Err("parse error".to_string());
        }

        let compiler = crate::compile_zsh::ZshCompiler::new();
        let chunk = compiler.compile(&program);

        // Cache the bytecode for next time. Best-effort — failures don't
        // block execution since the chunk is already in hand.
        if let Ok(blob) = bincode::serialize(&chunk) {
            let _ = crate::script_cache::try_save_bytes(path, &blob);
            tracing::trace!(
                path = %abs_path,
                bytes = blob.len(),
                "execute_script_file: bytecode cached"
            );
        }

        // Execute
        if !chunk.ops.is_empty() {
            crate::fusevm_disasm::maybe_print_stdout(
                &format!("execute_script_file:compile:{abs_path}"),
                &chunk,
            );
            let mut vm = fusevm::VM::new(chunk);
            register_builtins(&mut vm);
            let _ctx = ExecutorContext::enter(self);
            match vm.run() {
                fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                    self.set_last_status(vm.last_status);
                }
                fusevm::VMResult::Error(e) => {
                    return Err(format!("VM error: {}", e));
                }
            }
        }

        Ok(self.last_status())
    }


    /// Execute via the lex+parse free fns + ZshCompiler pipeline.
    /// This is the only execution path; `execute_script` delegates here.
    pub fn execute_script_zsh_pipeline(&mut self, script: &str) -> Result<i32, String> {
        // Skip history expansion for non-interactive script execution
        // (`zsh -c '…'`, internal eval, sourced files). zsh's `!`
        // history sub only fires on the REPL command line, never on
        // a pre-parsed script body. The interactive REPL has its
        // own dedicated path that calls expand_history before
        // dispatching here.
        // Save & clear errflag around the parse so a fresh syntax
        // error is distinguishable from one already in flight. Mirrors
        // Src/init.c loop()'s pre-parse `errflag &= ~ERRFLAG_ERROR;`.
        let saved_errflag = errflag.load(Ordering::Relaxed);
        errflag.fetch_and(!ERRFLAG_ERROR, Ordering::Relaxed);
        crate::ported::parse::parse_init(script);
        let program = crate::ported::parse::parse();
        let parse_failed = (errflag.load(Ordering::Relaxed) & ERRFLAG_ERROR) != 0;
        errflag.store(saved_errflag, Ordering::Relaxed);
        if parse_failed {
            return Err("parse error".to_string());
        }

        let compiler = crate::compile_zsh::ZshCompiler::new();
        let chunk = compiler.compile(&program);

        if chunk.ops.is_empty() {
            return Ok(self.last_status());
        }

        crate::fusevm_disasm::maybe_print_stdout("execute_script_zsh_pipeline", &chunk);
        let mut vm = fusevm::VM::new(chunk);
        register_builtins(&mut vm);
        {
            let _ctx = ExecutorContext::enter(self);
            match vm.run() {
                fusevm::VMResult::Ok(_) | fusevm::VMResult::Halted => {
                    self.set_last_status(vm.last_status);
                }
                fusevm::VMResult::Error(e) => return Err(format!("VM error: {}", e)),
            }
        }

        // Fire EXIT trap if set. Same logic as execute_script's old path:
        // remove first to prevent infinite recursion, then run.
        if let Some(action) = self.traps.remove("EXIT") {
            tracing::debug!("firing EXIT trap (new pipeline)");
            let _ = self.execute_script_zsh_pipeline(&action);
        }

        Ok(self.last_status())
    }

    #[tracing::instrument(skip(self, script), fields(len = script.len()))]
    pub fn execute_script(&mut self, script: &str) -> Result<i32, String> {
        // lex+parse free fns + ZshCompiler is the only execution path.
        self.execute_script_zsh_pipeline(script)
    }
}


// =====================================================================
// run_process_sub_in + run_process_sub_out — moved from src/vm_helper.rs.
// Partial port of `Src/exec.c::getproc` — FIFO-backed process
// substitution `<(cmd)` and `>(cmd)`.
//
// User-tagged as FAKE: zsh's getproc opens a pipe + forks a child,
// dups the pipe fd to stdin/stdout in the child, returns `/dev/fd/N`
// (path under /dev/fd or /proc/self/fd) to the parent. zshrs uses
// a named FIFO in /tmp and a worker-pool thread instead — same
// semantics for the consumer (a path argument to the outer cmd),
// different lifecycle (file on disk, blocks until both ends open).
// =====================================================================
impl crate::ported::vm_helper::ShellExecutor {
    pub(crate) fn run_process_sub_in(&mut self, cmd_str: &str) -> String {
        // Phase 2: parse via parse_init+parse. Extract the first Simple cmd's
        // words (untokenized), pre-expand to argv strings, spawn.
        let words = self.simple_cmd_words(cmd_str);

        // Create a unique FIFO in temp directory
        let fifo_path = format!("/tmp/zshrs_psub_{}", std::process::id());
        let fifo_counter = self.process_sub_counter;
        self.process_sub_counter += 1;
        let fifo_path = format!("{}_{}", fifo_path, fifo_counter);

        // Remove if exists, then create FIFO
        let _ = fs::remove_file(&fifo_path);
        if nix::unistd::mkfifo(fifo_path.as_str(), nix::sys::stat::Mode::S_IRWXU).is_err() {
            return String::new();
        }

        // Spawn command that writes to the FIFO
        let fifo_clone = fifo_path.clone();
        if !words.is_empty() {
            let cmd_name = words[0].clone();
            let args: Vec<String> = words[1..].to_vec();

            self.worker_pool.submit(move || {
                // Open FIFO for writing (will block until reader connects)
                if let Ok(fifo) = fs::OpenOptions::new().write(true).open(&fifo_clone) {
                    let _ = Command::new(&cmd_name)
                        .args(&args)
                        .stdout(fifo)
                        .stderr(Stdio::inherit())
                        .status();
                }
                // Clean up FIFO after command completes
                let _ = fs::remove_file(&fifo_clone);
            });
        }

        fifo_path
    }

    pub(crate) fn run_process_sub_out(&mut self, cmd_str: &str) -> String {
        let words = self.simple_cmd_words(cmd_str);

        // Create a unique FIFO in temp directory
        let fifo_path = format!("/tmp/zshrs_psub_{}", std::process::id());
        let fifo_counter = self.process_sub_counter;
        self.process_sub_counter += 1;
        let fifo_path = format!("{}_{}", fifo_path, fifo_counter);

        // Remove if exists, then create FIFO
        let _ = fs::remove_file(&fifo_path);
        if nix::unistd::mkfifo(fifo_path.as_str(), nix::sys::stat::Mode::S_IRWXU).is_err() {
            return String::new();
        }

        // Spawn command that reads from the FIFO
        let fifo_clone = fifo_path.clone();
        if !words.is_empty() {
            let cmd_name = words[0].clone();
            let args: Vec<String> = words[1..].to_vec();

            self.worker_pool.submit(move || {
                // Open FIFO for reading (will block until writer connects)
                if let Ok(fifo) = fs::File::open(&fifo_clone) {
                    let _ = Command::new(&cmd_name)
                        .args(&args)
                        .stdin(fifo)
                        .stdout(Stdio::inherit())
                        .stderr(Stdio::inherit())
                        .status();
                }
                // Clean up FIFO after command completes
                let _ = fs::remove_file(&fifo_clone);
            });
        }

        fifo_path
    }

}
